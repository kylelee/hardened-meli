/*
 * meli - MCP URL extraction from e-mail bodies.
 *
 * Copyright 2026 Kyle Lee
 *
 * This file is part of meli.
 *
 * meli is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * meli is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with meli.  If not, see <http://www.gnu.org/licenses/>.
 */

//! Extract every link from an e-mail.
//!
//! Both `text/plain` and `text/html` parts are scanned, together with the
//! context needed to judge a link: the anchor display text (HTML), the
//! part it came from, and whether it was obfuscated (`hxxp://`,
//! `evil[.]com`, ...).
//!
//! Extraction shares the same `linkify` scanner the TUI pager uses for
//! plain text, so MCP and TUI agree on what a plain-text link is. HTML is
//! scanned for `<a href=...>` anchors plus bare URLs in the rendered text.

use std::borrow::Cow;

use linkify::LinkFinder;
use melib::Attachment;
use melib::email::attachment_types::ContentType;
use std::sync::LazyLock;

use super::scoring::LinkClass;
use regex::Regex;

/// Maximum number of links reported per mail.
pub const MAX_URLS_PER_MAIL: usize = 500;

/// One extracted link.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ExtractedUrl {
    /// Normalized URL (`mailto:` links keep their scheme; scheme-less
    /// `www.` links gain an `http://` prefix).
    pub url: String,
    /// Lowercase scheme, without `:`.
    pub scheme: String,
    /// Lowercase host, when present.
    pub host: Option<String>,
    /// Explicit port, when present.
    pub port: Option<u16>,
    /// Anchor display text for HTML links.
    pub display_text: Option<String>,
    /// Part the link was found in, e.g. `text/html#0`.
    pub part: String,
    /// Coarse classification (web / mail / other).
    #[serde(rename = "class")]
    pub class: LinkClass,
    /// True when the original text was obfuscated (`hxxp`, `evil[.]com`)
    /// and had to be de-obfuscated to produce `url`.
    pub obfuscated: bool,
}

struct UrlAccumulator {
    urls: Vec<ExtractedUrl>,
    seen: Vec<(String, Option<String>)>,
}

impl UrlAccumulator {
    fn new() -> Self {
        Self {
            urls: Vec::new(),
            seen: Vec::new(),
        }
    }

    fn push(&mut self, url: String, display_text: Option<String>, part: &str, obfuscated: bool) {
        if self.urls.len() >= MAX_URLS_PER_MAIL {
            return;
        }
        let key = (url.clone(), display_text.clone());
        if self.seen.contains(&key) {
            return;
        }
        self.seen.push(key);
        let (scheme, host, port) = split_scheme_host_port(&url);
        self.urls.push(ExtractedUrl {
            class: match scheme.as_str() {
                "http" | "https" => LinkClass::Web,
                "mailto" => LinkClass::Mail,
                _ => LinkClass::Other,
            },
            url,
            scheme,
            host,
            port,
            display_text,
            part: part.to_string(),
            obfuscated,
        });
    }
}

fn split_scheme_host_port(url: &str) -> (String, Option<String>, Option<u16>) {
    let (scheme, rest) = match url.find("://") {
        Some(pos) => (url[..pos].to_ascii_lowercase(), &url[pos + 3..]),
        None => match url.find(':') {
            Some(pos) => (url[..pos].to_ascii_lowercase(), &url[pos + 1..]),
            None => ("http".to_string(), url),
        },
    };
    if scheme == "mailto" {
        return (
            scheme,
            Some(rest.split('?').next().unwrap_or(rest).to_string()),
            None,
        );
    }
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let mut authority = &rest[..authority_end];
    authority = authority.rsplit('@').next().unwrap_or(authority);
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => {
            (h, p.parse::<u16>().ok())
        }
        _ => (authority, None),
    };
    (
        scheme,
        (!host.is_empty()).then(|| host.to_ascii_lowercase()),
        port,
    )
}

/// Walk the MIME tree and extract links from every text part.
///
/// Nested `message/rfc822` attachments are not descended into: their body
/// belongs to the attached mail, not this one, and the TUI treats them as
/// separate views too.
pub fn extract_from_attachment(root: &Attachment) -> Vec<ExtractedUrl> {
    let mut acc = UrlAccumulator::new();
    let mut plain_idx = 0usize;
    let mut html_idx = 0usize;
    walk(root, &mut acc, &mut plain_idx, &mut html_idx, 0);
    acc.urls
}

const MAX_WALK_DEPTH: usize = 10;

fn walk(
    part: &Attachment,
    acc: &mut UrlAccumulator,
    plain_idx: &mut usize,
    html_idx: &mut usize,
    depth: usize,
) {
    if depth > MAX_WALK_DEPTH {
        return;
    }
    match &part.content_type {
        ContentType::Text { .. } => {
            let decoded = part.decode(Default::default());
            let text = String::from_utf8_lossy(&decoded);
            if part.content_type.is_text_html() {
                let label = format!("text/html#{}", *html_idx);
                *html_idx += 1;
                scan_html(&text, acc, &label);
            } else {
                let label = format!("text/plain#{}", *plain_idx);
                *plain_idx += 1;
                scan_plain(&text, acc, &label);
            }
        }
        ContentType::Multipart { parts, .. } => {
            for sub in parts {
                walk(sub, acc, plain_idx, html_idx, depth + 1);
            }
        }
        _ => {}
    }
}

/// Push one plain-text URL, normalizing scheme-less `www.` hosts and
/// leaving obfuscated spellings (`hxxp`) to `scan_obfuscated`.
fn push_plain_url(raw: &str, acc: &mut UrlAccumulator, label: &str) {
    let lower = raw.to_ascii_lowercase();
    if lower.starts_with("hxxp") || lower.starts_with("h**p") {
        return;
    }
    let url = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("http://{raw}")
    };
    acc.push(url, None, label, false);
}

/// Scan decoded plain text with the same `linkify` scanner the TUI uses.
fn scan_plain(text: &str, acc: &mut UrlAccumulator, label: &str) {
    let finder = LinkFinder::new();
    for link in finder.links(text) {
        match link.kind() {
            linkify::LinkKind::Url => {
                push_plain_url(link.as_str(), acc, label);
            }
            linkify::LinkKind::Email => {
                acc.push(format!("mailto:{}", link.as_str()), None, label, false);
            }
            _ => {}
        }
    }
    // linkify does not match bare `www.` hosts; scan for them explicitly.
    static WWW_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?i)\bwww\.[^\s<>\"'`,;]+"#).expect("valid www regex"));
    for m in WWW_RE.find_iter(text) {
        acc.push(format!("http://{}", m.as_str()), None, label, false);
    }
    scan_obfuscated(text, acc, label, None);
}

static ANCHOR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)<a\b[^>]*?href\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))[^>]*>(.*?)</a>"#)
        .expect("valid anchor regex")
});
static TAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<[^>]*>").expect("valid tag regex"));

/// Scan HTML for `<a href>` anchors (keeping the display text) and bare
/// URLs in the rendered text.
fn scan_html(html: &str, acc: &mut UrlAccumulator, label: &str) {
    let mut anchor_urls = Vec::new();
    for caps in ANCHOR_RE.captures_iter(html) {
        let href = caps
            .get(1)
            .or_else(|| caps.get(2))
            .or_else(|| caps.get(3))
            .map(|m| decode_entities(m.as_str()));
        let Some(href) = href else { continue };
        let href = href.trim().to_string();
        if href.is_empty() || href.starts_with('#') || href.starts_with("cid:") {
            continue;
        }
        let display = decode_entities(&caps[4]);
        let display = TAG_RE.replace_all(&display, "").trim().to_string();
        let display = if display.is_empty() {
            None
        } else {
            Some(display)
        };
        let normalized = if href.starts_with("mailto:") || href.contains("://") {
            href.clone()
        } else if href.starts_with("//") {
            format!("https:{href}")
        } else {
            format!("http://{href}")
        };
        let obfuscated = is_obfuscated_token(&href);
        let deobfuscated = if obfuscated {
            deobfuscate(&normalized).unwrap_or_else(|| normalized.clone())
        } else {
            normalized.clone()
        };
        anchor_urls.push(deobfuscated.clone());
        acc.push(deobfuscated, display, label, obfuscated);
    }
    // Bare URLs in the rendered text that no anchor already covers.
    let rendered = TAG_RE.replace_all(html, " ").to_string();
    let finder = LinkFinder::new();
    for link in finder.links(&rendered) {
        if link.kind() != &linkify::LinkKind::Url {
            continue;
        }
        let raw = link.as_str();
        let url = if raw.contains("://") {
            raw.to_string()
        } else {
            format!("http://{raw}")
        };
        if anchor_urls.iter().any(|u| u.eq_ignore_ascii_case(&url)) {
            continue;
        }
        push_plain_url(raw, acc, label);
    }
    scan_obfuscated(&rendered, acc, label, Some(&anchor_urls));
}

/// Decode the handful of HTML entities that matter for link text.
pub fn decode_entities(s: &str) -> Cow<'_, str> {
    if !s.contains('&') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];
        let (entity, remainder) = match rest.find(';') {
            Some(end) if end <= 12 => rest.split_at(end + 1),
            _ => {
                out.push('&');
                rest = &rest[1..];
                continue;
            }
        };
        let replaced = match entity {
            "&amp;" => Some('&'),
            "&lt;" => Some('<'),
            "&gt;" => Some('>'),
            "&quot;" => Some('"'),
            "&apos;" => Some('\''),
            "&nbsp;" => Some(' '),
            "&copy;" => Some('©'),
            "&reg;" => Some('®'),
            "&trade;" => Some('™'),
            "&hellip;" => Some('…'),
            "&mdash;" => Some('—'),
            "&ndash;" => Some('–'),
            "&lsquo;" | "&rsquo;" => Some('\''),
            "&ldquo;" | "&rdquo;" => Some('"'),
            _ => entity
                .strip_prefix("&#")
                .and_then(|num| num.strip_suffix(';'))
                .and_then(|num| {
                    if let Some(hex) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
                        u32::from_str_radix(hex, 16).ok()
                    } else {
                        num.parse::<u32>().ok()
                    }
                })
                .and_then(char::from_u32),
        };
        match replaced {
            Some(c) => {
                out.push(c);
                rest = remainder;
            }
            None => {
                out.push_str(entity);
                rest = remainder;
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// Does this token use obfuscation markers?
fn is_obfuscated_token(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    lower.contains("hxxp")
        || lower.contains("h**p")
        || lower.contains("[.]")
        || lower.contains("{.}")
        || lower.contains("(.)")
        || lower.contains("(dot)")
        || lower.contains("[dot]")
}

/// Normalize an obfuscated token to a real URL, when possible.
fn deobfuscate(token: &str) -> Option<String> {
    let mut s = token.to_string();
    s = s.replace([' ', '\t'], "");
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("hxxps") {
        s.replace_range(0..5, "https");
    } else if lower.starts_with("hxxp") {
        s.replace_range(0..4, "http");
    } else if lower.starts_with("h**ps") {
        s.replace_range(0..5, "https");
    } else if lower.starts_with("h**p") {
        s.replace_range(0..4, "http");
    }
    for (marker, dot) in [
        ("[.]", "."),
        ("{.}", "."),
        ("(.)", "."),
        ("(dot)", "."),
        ("[dot]", "."),
    ] {
        s = s.replace(marker, dot);
    }
    let looks_like_url = (s.starts_with("http://") || s.starts_with("https://"))
        && s["http://".len().min(s.len())..].contains('.');
    looks_like_url.then_some(s)
}

/// Scan text for deliberately obfuscated links (`hxxp://evil[.]com`).
fn scan_obfuscated(text: &str, acc: &mut UrlAccumulator, label: &str, already: Option<&[String]>) {
    static WORD_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"[^\s<>"'`]+"#).expect("valid word regex"));
    for word in WORD_RE.find_iter(text) {
        let word = word.as_str();
        if !is_obfuscated_token(word) {
            continue;
        }
        let Some(url) = deobfuscate(word) else {
            continue;
        };
        if let Some(already) = already {
            if already.iter().any(|u| u.eq_ignore_ascii_case(&url)) {
                continue;
            }
        }
        acc.push(url, None, label, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use melib::AttachmentBuilder;

    fn build_mail(bytes: &'static [u8]) -> Attachment {
        AttachmentBuilder::new(bytes).build()
    }

    const PLAIN_MAIL: &[u8] = b"From: a@example.org\n\
To: b@example.org\n\
Subject: links\n\
Content-Type: text/plain; charset=utf-8\n\
\n\
Check https://meli-email.org and www.example.org/docs plus \
manos@example.org and hxxp://evil[.]com/phish\n";

    #[test]
    fn test_extract_plain_text_links() {
        let root = build_mail(PLAIN_MAIL);
        let urls = extract_from_attachment(&root);
        let raw: Vec<&str> = urls.iter().map(|u| u.url.as_str()).collect();
        assert!(raw.contains(&"https://meli-email.org"), "{raw:?}");
        assert!(raw.contains(&"http://www.example.org/docs"), "{raw:?}");
        assert!(raw.contains(&"mailto:manos@example.org"), "{raw:?}");
        assert!(raw.contains(&"http://evil.com/phish"), "{raw:?}");
        let obfuscated = urls.iter().find(|u| u.obfuscated).expect("one obfuscated");
        assert_eq!(obfuscated.url, "http://evil.com/phish");
        assert_eq!(obfuscated.part, "text/plain#0");
    }

    const HTML_MAIL: &[u8] = b"From: phishing@example.org\n\
To: victim@example.org\n\
Subject: urgent\n\
Content-Type: text/html; charset=utf-8\n\
\n\
<html><body>\n\
<p>Dear customer, <a href=\"http://192.168.13.37:8080/verify\">https://secure.bank.example/login</a></p>\n\
<p>Also read <a href=\"https://meli-email.org/blog.html\">our blog &amp; news</a></p>\n\
<p>Bare link: https://example.org/plain</p>\n\
</body></html>\n";

    #[test]
    fn test_extract_html_anchors_with_display_text() {
        let root = build_mail(HTML_MAIL);
        let urls = extract_from_attachment(&root);
        let mismatch = urls
            .iter()
            .find(|u| u.url == "http://192.168.13.37:8080/verify")
            .expect("href extracted");
        assert_eq!(
            mismatch.display_text.as_deref(),
            Some("https://secure.bank.example/login")
        );
        assert_eq!(mismatch.part, "text/html#0");
        let blog = urls
            .iter()
            .find(|u| u.url == "https://meli-email.org/blog.html")
            .expect("blog");
        assert_eq!(blog.display_text.as_deref(), Some("our blog & news"));
        // Bare URL in rendered html text.
        assert!(urls.iter().any(|u| u.url == "https://example.org/plain"));
    }

    const ALTERNATIVE_MAIL: &[u8] = b"From: news@example.org\n\
To: reader@example.org\n\
Subject: issue 215\n\
MIME-Version: 1.0\n\
Content-Type: multipart/alternative; boundary=\"XYZ\"\n\
\n\
--XYZ\n\
Content-Type: text/plain; charset=utf-8\n\
\n\
Unsubscribe at https://click.list-manage.com/unsubscribe?u=1\n\
--XYZ\n\
Content-Type: text/html; charset=utf-8\n\
\n\
<a href=\"https://click.list-manage.com/unsubscribe?u=1\">unsubscribe</a>\n\
--XYZ--\n";

    #[test]
    fn test_extract_multipart_alternative_parts_labelled() {
        let root = build_mail(ALTERNATIVE_MAIL);
        let urls = extract_from_attachment(&root);
        let plain = urls
            .iter()
            .find(|u| u.part == "text/plain#0")
            .expect("plain part");
        assert_eq!(plain.url, "https://click.list-manage.com/unsubscribe?u=1");
        let html = urls
            .iter()
            .find(|u| u.part == "text/html#0")
            .expect("html part");
        assert_eq!(html.display_text.as_deref(), Some("unsubscribe"));
    }

    #[test]
    fn test_extract_dedupes_within_part() {
        let root = build_mail(PLAIN_MAIL);
        let urls = extract_from_attachment(&root);
        let count = urls
            .iter()
            .filter(|u| u.url == "https://meli-email.org")
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_extract_dedupes_html_bare_link_already_in_anchor() {
        let root = build_mail(HTML_MAIL);
        let urls = extract_from_attachment(&root);
        let count = urls
            .iter()
            .filter(|u| u.url == "https://meli-email.org/blog.html")
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_extract_caps_total() {
        let mut body = String::from("Content-Type: text/plain; charset=utf-8\n\n");
        for i in 0..600 {
            body.push_str(&format!("https://example.org/{i}\n"));
        }
        let mail = format!("From: a@example.org\nSubject: many\n{body}");
        let root = AttachmentBuilder::new(mail.as_bytes()).build();
        let urls = extract_from_attachment(&root);
        assert_eq!(urls.len(), MAX_URLS_PER_MAIL);
    }

    #[test]
    fn test_decode_entities() {
        assert_eq!(decode_entities("a&amp;b"), "a&b");
        assert_eq!(decode_entities("&#65;"), "A");
        assert_eq!(decode_entities("&#x1F600;"), "\u{1F600}");
        assert_eq!(decode_entities("&nbsp;x"), " x");
        assert_eq!(decode_entities("no entities"), "no entities");
        assert_eq!(decode_entities("&unknown;"), "&unknown;");
        assert_eq!(decode_entities("a&notanentity"), "a&notanentity");
    }

    #[test]
    fn test_deobfuscate_forms() {
        assert_eq!(
            deobfuscate("hxxp://evil[.]com/a").as_deref(),
            Some("http://evil.com/a")
        );
        assert_eq!(
            deobfuscate("hxxps://phish{.}net").as_deref(),
            Some("https://phish.net")
        );
        assert_eq!(
            deobfuscate("h**p://x(dot)yz").as_deref(),
            Some("http://x.yz")
        );
        assert_eq!(deobfuscate("not-a-url").as_deref(), None);
    }
}
