/*
 * meli - MCP URL risk scoring engine.
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

//! Local, offline URL risk scoring for e-mail links.
//!
//! The engine evaluates one URL (plus, optionally, the display text of the
//! HTML anchor it appeared in) against a set of explainable signals, sums
//! the weights of the signals that fire, and maps the total to a `low` /
//! `medium` / `high` risk band. Every hit is reported with its signal name,
//! weight and a human-readable detail; there is no black-box score.
//!
//! The signal set is a synthesis of established phishing-detection
//! practice:
//!
//! - lexical and host-based features used across the phishing-detection
//!   literature (IP-literal and encoded-IP hosts, credentials in the
//!   authority, non-standard ports, many/random subdomains, hyphen-heavy
//!   and suspicious TLDs, brand keywords planted in subdomains, excessive
//!   length and percent-encoding);
//! - homograph defence as recommended by Unicode TS #39-lite: punycode
//!   (`xn--`) labels, mixed-script labels and confusable skeletons
//!   compared against a brand table (RFC 3492 decoding is implemented
//!   locally, no network, no external crates);
//! - link-text/`href` host mismatch, the single strongest phishing cue in
//!   HTML mail;
//! - classification-only (zero-weight) signals for `mailto:` and
//!   unsubscribe machinery, so agents can separate infrastructure links
//!   from threats.
//!
//! Privacy: scoring is purely local. The engine never fetches, resolves or
//! otherwise contacts anything referenced by a URL.

use std::collections::HashMap;

/// Weight-and-name row of the default signal table.
#[derive(Clone, Copy)]
pub struct SignalSpec {
    pub name: &'static str,
    pub weight: u8,
}

impl SignalSpec {
    pub const fn classification(name: &'static str) -> Self {
        Self { name, weight: 0 }
    }
}

/// Default weight table. Order matters only for `signal_names()` output.
pub const DEFAULT_SIGNALS: &[SignalSpec] = &[
    SignalSpec {
        name: "scheme_dangerous",
        weight: 60,
    },
    SignalSpec {
        name: "scheme_non_web",
        weight: 15,
    },
    SignalSpec {
        name: "ip_literal_host",
        weight: 45,
    },
    SignalSpec {
        name: "obfuscated_ip_encoding",
        weight: 20,
    },
    SignalSpec {
        name: "credentials_in_url",
        weight: 45,
    },
    SignalSpec {
        name: "nonstandard_port",
        weight: 15,
    },
    SignalSpec {
        name: "href_text_mismatch",
        weight: 55,
    },
    SignalSpec {
        name: "brand_in_subdomain",
        weight: 45,
    },
    SignalSpec {
        name: "brand_homograph",
        weight: 60,
    },
    SignalSpec {
        name: "mixed_script_homograph",
        weight: 45,
    },
    SignalSpec {
        name: "idn_punycode_label",
        weight: 15,
    },
    SignalSpec {
        name: "shortener_domain",
        weight: 25,
    },
    SignalSpec {
        name: "excessive_subdomains",
        weight: 15,
    },
    SignalSpec {
        name: "random_looking_subdomain",
        weight: 20,
    },
    SignalSpec {
        name: "tracker_domain",
        weight: 10,
    },
    SignalSpec {
        name: "tracking_params",
        weight: 5,
    },
    SignalSpec {
        name: "redirect_param",
        weight: 10,
    },
    SignalSpec {
        name: "suspicious_tld",
        weight: 20,
    },
    SignalSpec {
        name: "hyphen_heavy_host",
        weight: 10,
    },
    SignalSpec {
        name: "long_url",
        weight: 10,
    },
    SignalSpec {
        name: "heavy_percent_encoding",
        weight: 10,
    },
    SignalSpec {
        name: "obfuscated_spelling",
        weight: 15,
    },
    SignalSpec::classification("mailto_classified"),
    SignalSpec::classification("unsubscribe_classified"),
];

/// All signal names accepted in `[mcp.url_scoring.signal_weights]`.
pub fn signal_names() -> Vec<&'static str> {
    DEFAULT_SIGNALS.iter().map(|s| s.name).collect()
}

/// Risk band of a score, computed from the configured thresholds.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    Low,
    Medium,
    High,
}

impl Risk {
    pub fn of(score: u8, low_medium: u8, medium_high: u8) -> Self {
        if score >= medium_high {
            Self::High
        } else if score >= low_medium {
            Self::Medium
        } else {
            Self::Low
        }
    }
}

/// A fired signal with its resolved weight and explanation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SignalHit {
    pub name: &'static str,
    pub weight: u8,
    pub detail: String,
}

/// The scoring result for one URL.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ScoredUrl {
    pub url: String,
    pub score: u8,
    pub risk: Risk,
    pub signals: Vec<SignalHit>,
}

/// Weights resolved once per session (defaults + user overrides).
#[derive(Clone, Debug)]
pub struct Weights {
    map: HashMap<&'static str, u8>,
}

impl Weights {
    pub fn new(resolve: impl Fn(&str, u8) -> u8) -> Self {
        Self {
            map: DEFAULT_SIGNALS
                .iter()
                .map(|s| (s.name, resolve(s.name, s.weight)))
                .collect(),
        }
    }

    pub fn get(&self, name: &'static str) -> u8 {
        self.map.get(name).copied().unwrap_or(0)
    }
}

/// Well-known link shorteners.
const SHORTENERS: &[&str] = &[
    "bit.ly",
    "t.co",
    "tinyurl.com",
    "goo.gl",
    "is.gd",
    "ow.ly",
    "buff.ly",
    "cutt.ly",
    "rebrand.ly",
    "s.id",
    "tiny.cc",
    "shorturl.at",
    "v.gd",
    "bit.do",
    "mtr.cool",
    "lnkd.in",
];

/// Well-known mail-provider click-tracking (and analytics) domains. These
/// are classified, not condemned: newsletter infrastructure routinely
/// rewrites links through them, so the weight is low.
const TRACKERS: &[&str] = &[
    "list-manage.com",
    "mandrillapp.com",
    "sendgrid.net",
    "sendgrid.com",
    "mailgun.net",
    "constantcontact.com",
    "cmail19.com",
    "cmail20.com",
    "click.mailchimp.com",
    "t.email.microsoft.com",
    "links.email.azure.com",
    "github.usermgmt.com",
];

/// TLDs with a consistently poor reputation in phishing feeds
/// (cheap/free registration, or the file-extension lookalikes `.zip` and
/// `.mov`).
const SUSPICIOUS_TLDS: &[&str] = &[
    "zip", "mov", "top", "tk", "ml", "cf", "gq", "work", "click", "country", "download", "loan",
    "review", "gdn", "kim",
];

/// Brand keyword table: `(token, official registered domains)`. A token
/// found in a *subdomain* while the registered domain is not one of the
/// official entries is the classic "brand planted next to an attacker
/// domain" pattern (`paypal.com.secure-login.example.ru`).
const BRANDS: &[(&str, &[&str])] = &[
    ("paypal", &["paypal.com"]),
    ("apple", &["apple.com", "icloud.com", "mzstatic.com"]),
    ("icloud", &["icloud.com", "apple.com"]),
    (
        "microsoft",
        &[
            "microsoft.com",
            "live.com",
            "office.com",
            "office365.com",
            "outlook.com",
            "msn.com",
            "azure.com",
            "microsoftonline.com",
        ],
    ),
    (
        "outlook",
        &["outlook.com", "microsoft.com", "office365.com"],
    ),
    (
        "office365",
        &["office365.com", "microsoft.com", "office.com"],
    ),
    (
        "google",
        &[
            "google.com",
            "gmail.com",
            "googleapis.com",
            "gstatic.com",
            "youtube.com",
            "googleusercontent.com",
            "blogger.com",
            "googlemail.com",
        ],
    ),
    ("gmail", &["gmail.com", "google.com", "googlemail.com"]),
    ("facebook", &["facebook.com", "fb.com", "fb.me"]),
    ("instagram", &["instagram.com"]),
    ("whatsapp", &["whatsapp.com"]),
    ("amazon", &["amazon.com", "amazonaws.com", "amzn.to"]),
    ("netflix", &["netflix.com", "nflximg.com", "nflxvideo.net"]),
    ("linkedin", &["linkedin.com", "lnkd.in"]),
    ("dropbox", &["dropbox.com", "dropboxusercontent.com"]),
    ("dhl", &["dhl.com"]),
    ("fedex", &["fedex.com"]),
    ("usps", &["usps.com"]),
    ("chase", &["chase.com"]),
    ("wellsfargo", &["wellsfargo.com"]),
    ("bankofamerica", &["bankofamerica.com"]),
    ("hsbc", &["hsbc.com", "hsbc.co.uk"]),
    ("binance", &["binance.com"]),
    ("coinbase", &["coinbase.com"]),
    ("metamask", &["metamask.io"]),
    ("discord", &["discord.com", "discordapp.com"]),
    ("steam", &["steampowered.com", "steamcommunity.com"]),
    (
        "github",
        &["github.com", "githubusercontent.com", "githubassets.com"],
    ),
    ("wechat", &["wechat.com", "weixin.qq.com"]),
    ("weixin", &["weixin.qq.com", "wechat.com"]),
    ("alipay", &["alipay.com", "alibaba.com"]),
    ("taobao", &["taobao.com", "alicdn.com"]),
    ("netease", &["163.com", "126.com", "netease.com"]),
];

/// Confusable-to-ASCII map used to build comparison skeletons, in the
/// spirit of Unicode TS #39 (a pragmatic subset of the common Cyrillic and
/// Greek lookalikes that dominate real homograph phishing).
const CONFUSABLES: &[(char, char)] = &[
    // Cyrillic
    ('а', 'a'),
    ('е', 'e'),
    ('о', 'o'),
    ('р', 'p'),
    ('с', 'c'),
    ('у', 'y'),
    ('х', 'x'),
    ('і', 'i'),
    ('ј', 'j'),
    ('һ', 'h'),
    ('ԛ', 'q'),
    ('ԝ', 'w'),
    ('ɡ', 'g'),
    ('ѕ', 's'),
    ('ӏ', 'l'),
    ('Ӏ', 'l'),
    ('І', 'i'),
    // Greek
    ('α', 'a'),
    ('ο', 'o'),
    ('ρ', 'p'),
    ('τ', 't'),
    ('ν', 'v'),
    ('κ', 'k'),
    ('ι', 'i'),
    ('ε', 'e'),
    ('ς', 's'),
    ('χ', 'x'),
    ('υ', 'u'),
];

/// Map lookalike characters to their ASCII skeleton, leaving everything
/// else untouched.
pub fn confusable_skeleton(s: &str) -> String {
    s.chars()
        .map(|c| {
            CONFUSABLES
                .iter()
                .find(|(from, _)| *from == c)
                .map_or(c, |(_, to)| *to)
        })
        .collect()
}

/// Decode a single punycode label (RFC 3492) to Unicode.
///
/// `input` is the part after `xn--` (or the raw label if it already
/// contains an ASCII prefix before the last `-`). Returns `None` on
/// malformed input or out-of-range code points.
pub fn punycode_decode(input: &str) -> Option<String> {
    const BASE: u32 = 36;
    const TMIN: u32 = 1;
    const TMAX: u32 = 26;
    const SKEW: u32 = 38;
    const DAMP: u32 = 700;
    const INITIAL_BIAS: u32 = 72;
    const INITIAL_N: u32 = 128;

    fn digit(d: char) -> Option<u32> {
        match d {
            'a'..='z' | 'A'..='Z' => Some(d.to_ascii_lowercase() as u32 - 'a' as u32),
            '0'..='9' => Some(d as u32 - '0' as u32 + 26),
            _ => None,
        }
    }

    fn adapt(mut delta: u32, num_points: u32, first_time: bool) -> u32 {
        delta = if first_time { delta / DAMP } else { delta / 2 };
        delta += delta / num_points;
        let mut k = 0;
        while delta > ((BASE - TMIN) * TMAX) / 2 {
            delta /= BASE - TMIN;
            k += BASE;
        }
        k + (((BASE - TMIN + 1) * delta) / (delta + SKEW))
    }

    let (basic, encoded) = match input.rfind('-') {
        Some(pos) => (&input[..pos], &input[pos + 1..]),
        None => ("", input),
    };
    let mut output: Vec<char> = basic.chars().collect();
    for c in &output {
        if !c.is_ascii() {
            return None;
        }
    }
    let mut n = INITIAL_N;
    let mut i: u32 = 0;
    let mut bias = INITIAL_BIAS;
    let mut chars = encoded.chars().peekable();
    while chars.peek().is_some() {
        let old_i = i;
        let mut w: u32 = 1;
        let mut k = BASE;
        loop {
            let d = digit(chars.next()?)?;
            if d > (u32::MAX - i) / w {
                return None;
            }
            i += d * w;
            let t = if k <= bias {
                TMIN
            } else if k >= bias + TMAX {
                TMAX
            } else {
                k - bias
            };
            if d < t {
                break;
            }
            if w > u32::MAX / (BASE - t) {
                return None;
            }
            w *= BASE - t;
            k += BASE;
        }
        let out_len = output.len() as u32 + 1;
        bias = adapt(i - old_i, out_len, old_i == 0);
        n = n.checked_add(i / out_len)?;
        i %= out_len;
        let ch = char::from_u32(n)?;
        output.insert(i as usize, ch);
        i += 1;
    }
    Some(output.into_iter().collect())
}

/// Decode one host label, returning the Unicode form of `xn--` labels and
/// the label itself otherwise.
fn decode_label(label: &str) -> String {
    if let Some(rest) = label
        .strip_prefix("xn--")
        .or_else(|| label.strip_prefix("XN--"))
    {
        punycode_decode(rest).unwrap_or_else(|| label.to_string())
    } else {
        label.to_string()
    }
}

/// Coarse classification of a link, surfaced next to the score.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkClass {
    /// `http(s)` link.
    Web,
    /// `mailto:` link.
    Mail,
    /// Something else (`ftp`, `javascript`, opaque, ...).
    Other,
}

/// Structured breakdown of a URL used by the signal checks.
struct UrlParts {
    scheme: String,
    host: String,
    port: Option<u16>,
    userinfo: bool,
    query: String,
    unparseable: bool,
}

fn split_url(url: &str) -> UrlParts {
    let mut rest = url.trim();
    let (scheme, rest_after) = match rest.find("://") {
        Some(pos) => (rest[..pos].to_ascii_lowercase(), &rest[pos + 3..]),
        None => match rest.find(':') {
            Some(pos)
                if pos > 0
                    && rest[..pos]
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.') =>
            {
                (rest[..pos].to_ascii_lowercase(), &rest[pos + 1..])
            }
            _ => ("http".to_string(), rest),
        },
    };
    rest = rest_after;
    // authority ends at first '/', '?' or '#'
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let query = rest[authority_end..]
        .split_once('?')
        .map_or("", |(_, q)| q.split('#').next().unwrap_or(""))
        .to_string();
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let userinfo = rest[..authority_end].contains('@');
    // host[:port]
    let (host, port) = if let Some(stripped) = authority.strip_prefix('[') {
        // IPv6 literal
        match stripped.find(']') {
            Some(end) => {
                let host = format!("[{}]", &stripped[..end]);
                let after = &stripped[end + 1..];
                let port = after.strip_prefix(':').and_then(|p| p.parse::<u16>().ok());
                (host, port)
            }
            None => (authority.to_string(), None),
        }
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => {
                (h.to_string(), p.parse::<u16>().ok())
            }
            _ => (authority.to_string(), None),
        }
    };
    UrlParts {
        scheme,
        host: host.to_ascii_lowercase(),
        port,
        userinfo,
        query,
        unparseable: host.is_empty(),
    }
}

/// Registered domain (naively the last two labels) and TLD of a host.
fn registered_domain(host: &str) -> (String, String) {
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() >= 2 {
        (
            format!("{}.{}", labels[labels.len() - 2], labels[labels.len() - 1]),
            labels[labels.len() - 1].to_string(),
        )
    } else {
        (
            host.to_string(),
            labels.last().copied().unwrap_or_default().to_string(),
        )
    }
}

/// Does `value` encode a 32-bit IPv4 address in a non-dotted form?
///
/// Catches decimal (`2130706433`), hexadecimal (`0x7f000001`) and
/// zero-padded octal (`0177000000001`) host spellings, as well as short
/// dotted forms (`127.1`).
/// Does `value` encode a 32-bit IPv4 address in a non-dotted form?
///
/// Catches decimal (`2130706433`), hexadecimal (`0x7f000001`) and
/// zero-padded octal (`0177.0.0.1`) host spellings, as well as short
/// dotted forms (`127.1`). Plain dotted-quad decimal hosts are *not*
/// obfuscated (that is `ip_literal_host` territory) and return `None`.
fn encoded_ipv4(host: &str) -> Option<String> {
    if host.is_empty() {
        return None;
    }
    if let Some(hex) = host
        .strip_prefix("0x")
        .filter(|h| !h.is_empty() && h.len() <= 8 && h.chars().all(|c| c.is_ascii_hexdigit()))
    {
        let value = u32::from_str_radix(hex, 16).ok()?;
        return Some(format_ipv4(value));
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() > 4 || labels.iter().any(|l| l.is_empty()) {
        return None;
    }
    let parse_num = |label: &str| -> Option<u64> {
        if let Some(octal) = label.strip_prefix('0').filter(|s| !s.is_empty()) {
            if !octal.chars().all(|c| ('0'..='7').contains(&c)) {
                return None;
            }
            u64::from_str_radix(octal, 8).ok()
        } else if label.chars().all(|c| c.is_ascii_digit()) {
            label.parse::<u64>().ok()
        } else {
            None
        }
    };
    // Plain decimal dotted-quad is not an *encoding*: leave it to the
    // ip-literal signal.
    let is_plain_octet = |l: &str| {
        l.chars().all(|c| c.is_ascii_digit())
            && (l == "0" || !l.starts_with('0'))
            && l.parse::<u32>().is_ok_and(|v| v <= 255)
    };
    if labels.len() == 4 && labels.iter().all(|l| is_plain_octet(l)) {
        return None;
    }
    let n = labels.len();
    let mut value: u64 = 0;
    for label in &labels[..n - 1] {
        let octet = parse_num(label)?;
        if octet > 255 {
            return None;
        }
        value = (value << 8) | octet;
    }
    let last = parse_num(labels[n - 1])?;
    let (shift, max_last) = match n {
        1 => (0u32, 1u64 << 32),
        2 => (24, 1 << 24),
        3 => (16, 1 << 16),
        _ => (0, 1 << 8),
    };
    if last >= max_last {
        return None;
    }
    let value = (value << shift) | last;
    if value > u32::MAX as u64 {
        return None;
    }
    Some(format_ipv4(value as u32))
}

fn format_ipv4(v: u32) -> String {
    format!(
        "{}.{}.{}.{}",
        v >> 24,
        (v >> 16) & 0xff,
        (v >> 8) & 0xff,
        v & 0xff
    )
}

fn is_ascii_ipv4(host: &str) -> bool {
    let parts: Vec<&str> = host.split('.').collect();
    parts.len() == 4
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 3
                && p.chars().all(|c| c.is_ascii_digit())
                && p.parse::<u8>().is_ok()
        })
}

/// Extract a URL-ish host from a piece of display text, if the text is
/// itself a link (e.g. `https://bank.example` or `www.bank.example`).
fn host_of_display_text(text: &str) -> Option<String> {
    let text = text.trim();
    if text.chars().any(|c| c.is_whitespace()) {
        return None;
    }
    if text.len() > 2048 {
        return None;
    }
    let candidate = if text.contains("://") {
        text.to_string()
    } else if text.starts_with("www.") && text.contains('.') {
        format!("http://{text}")
    } else {
        return None;
    };
    let parts = split_url(&candidate);
    (!parts.host.is_empty() && !parts.unparseable).then_some(parts.host)
}

/// Heuristic "random-looking" subdomain label: long, mixed digits and
/// letters, high character diversity.
fn looks_random(label: &str) -> bool {
    label.len() >= 8
        && label.chars().any(|c| c.is_ascii_digit())
        && label.chars().any(|c| c.is_ascii_alphabetic())
        && {
            let distinct = label
                .chars()
                .collect::<std::collections::BTreeSet<_>>()
                .len();
            distinct * 10 >= label.len() * 7
        }
}

/// Score one URL.
///
/// `display_text` is the anchor text the URL appeared under in HTML mail
/// (if any); it powers the `href_text_mismatch` signal. `low_medium` and
/// `medium_high` are the configured band thresholds.
pub fn score_url(
    url: &str,
    display_text: Option<&str>,
    weights: &Weights,
    low_medium: u8,
    medium_high: u8,
) -> ScoredUrl {
    let mut hits: Vec<SignalHit> = Vec::new();
    let push = |name: &'static str, detail: String, hits: &mut Vec<SignalHit>| {
        hits.push(SignalHit {
            name,
            weight: weights.get(name),
            detail,
        });
    };

    let parts = split_url(url);
    let class = match parts.scheme.as_str() {
        "http" | "https" => LinkClass::Web,
        "mailto" => LinkClass::Mail,
        _ => LinkClass::Other,
    };

    if class == LinkClass::Mail {
        push(
            "mailto_classified",
            "mailto: link; classified as an e-mail address, not scored".into(),
            &mut hits,
        );
        return ScoredUrl {
            url: url.to_string(),
            score: 0,
            risk: Risk::of(0, low_medium, medium_high),
            signals: hits,
        };
    }

    let lower_url = url.to_ascii_lowercase();
    let is_unsubscribe = parts.host.contains("unsubscribe")
        || parts.host.contains("list-unsubscribe")
        || parts.host.contains("u=r")
        || parts.query.contains("unsubscribe")
        || lower_url.contains("/unsubscribe")
        || lower_url.contains("/optout");

    match parts.scheme.as_str() {
        "javascript" | "data" | "vbscript" | "file" => {
            push(
                "scheme_dangerous",
                format!(
                    "`{}` scheme can execute or reference local content",
                    parts.scheme
                ),
                &mut hits,
            );
        }
        "http" | "https" => {}
        other if !parts.unparseable => {
            push(
                "scheme_non_web",
                format!("non-web scheme `{other}`"),
                &mut hits,
            );
        }
        _ => {}
    }

    if !parts.unparseable {
        let host = &parts.host;
        let (registered, tld) = registered_domain(host);
        let labels: Vec<&str> = host.split('.').collect();
        let subdomain_labels: Vec<&str> = if labels.len() > 2 {
            labels[..labels.len() - 2].to_vec()
        } else {
            Vec::new()
        };

        if host.starts_with('[') || is_ascii_ipv4(host) {
            push(
                "ip_literal_host",
                format!("host is a raw IP address (`{host}`)"),
                &mut hits,
            );
        }
        if let Some(decoded) = encoded_ipv4(host) {
            push(
                "obfuscated_ip_encoding",
                format!("host `{host}` encodes IP address `{decoded}` in a non-dotted form"),
                &mut hits,
            );
            push(
                "ip_literal_host",
                format!("host resolves visually to raw IP address `{decoded}`"),
                &mut hits,
            );
        }
        if parts.userinfo {
            push(
                "credentials_in_url",
                "URL embeds `user[:password]@` before the host; classic hidden-host trick".into(),
                &mut hits,
            );
        }
        if let Some(port) = parts.port {
            let standard = matches!((parts.scheme.as_str(), port), ("http", 80) | ("https", 443));
            if !standard {
                push(
                    "nonstandard_port",
                    format!("non-standard port {port}"),
                    &mut hits,
                );
            }
        }
        if let Some(display) = display_text.map(str::trim).filter(|d| !d.is_empty()) {
            if let Some(display_host) = host_of_display_text(display) {
                if display_host != parts.host {
                    push(
                        "href_text_mismatch",
                        format!(
                            "link text points to `{display_host}` but the href host is `{}`",
                            parts.host
                        ),
                        &mut hits,
                    );
                }
            }
        }
        if SHORTENERS.contains(&registered.as_str()) {
            push(
                "shortener_domain",
                format!("`{registered}` is a link shortener (destination is hidden)"),
                &mut hits,
            );
        }
        if TRACKERS
            .iter()
            .any(|t| host == *t || host.ends_with(&format!(".{t}")))
        {
            push(
                "tracker_domain",
                format!("`{registered}` is a known click-tracking domain"),
                &mut hits,
            );
        }
        if SUSPICIOUS_TLDS.contains(&tld.as_str()) {
            push(
                "suspicious_tld",
                format!("top-level domain `.{tld}` is commonly abused in phishing"),
                &mut hits,
            );
        }
        if labels.len() >= 5 {
            push(
                "excessive_subdomains",
                format!("host has {} labels (`{host}`)", labels.len()),
                &mut hits,
            );
        }
        if let Some(first) = subdomain_labels
            .iter()
            .find(|l| **l != "www" && looks_random(l))
        {
            push(
                "random_looking_subdomain",
                format!("random-looking subdomain `{first}`"),
                &mut hits,
            );
        }
        if labels
            .iter()
            .any(|l| l.matches('-').count() >= 3 || l.starts_with('-') || l.ends_with('-'))
        {
            push(
                "hyphen_heavy_host",
                format!("host labels contain abnormal hyphenation (`{host}`)"),
                &mut hits,
            );
        }
        // Punycode / homograph analysis on the host labels.
        let mut has_punycode = false;
        let mut mixed_script = false;
        let mut decoded_labels: Vec<String> = Vec::with_capacity(labels.len());
        for label in &labels {
            let decoded = decode_label(label);
            if decoded != *label {
                has_punycode = true;
            }
            let has_non_ascii = !decoded.is_ascii();
            let has_ascii_alnum = decoded.chars().any(|c| c.is_ascii_alphanumeric());
            if has_non_ascii && has_ascii_alnum {
                mixed_script = true;
            }
            if !has_non_ascii && !label.is_ascii() {
                mixed_script = true;
            }
            decoded_labels.push(decoded);
        }
        let decoded_host = decoded_labels.join(".");
        let (decoded_registered, _) = registered_domain(&decoded_host);
        let skeleton_registered = confusable_skeleton(&decoded_registered);
        let skeleton_host = confusable_skeleton(&decoded_host);
        if has_punycode {
            push(
                "idn_punycode_label",
                format!("host contains punycode (`xn--`) labels: `{host}`"),
                &mut hits,
            );
        }
        if mixed_script {
            push(
                "mixed_script_homograph",
                format!("host mixes scripts after decoding (`{decoded_host}`)"),
                &mut hits,
            );
        }
        // Brand checks.
        'brand: {
            for (token, official) in BRANDS {
                // 1) brand token planted in a subdomain of a domain the
                //    brand does not own.
                if subdomain_labels
                    .iter()
                    .any(|l| l.to_ascii_lowercase().contains(token))
                    && !official.contains(&registered.as_str())
                {
                    push(
                        "brand_in_subdomain",
                        format!(
                            "brand `{token}` appears in a subdomain of `{registered}`, which is                              not an official `{token}` domain"
                        ),
                        &mut hits,
                    );
                    break 'brand;
                }
                // 2) the registered domain itself is a confusable
                //    skeleton of one of the brand's official domains
                //    (homograph registration).
                if !decoded_registered.is_ascii()
                    && official.iter().any(|o| {
                        let o_skeleton = confusable_skeleton(o);
                        if skeleton_registered == o_skeleton {
                            return true;
                        }
                        skeleton_registered.starts_with(&format!("{token}."))
                            && o.starts_with(&format!("{token}."))
                            && skeleton_registered
                                .split_once('.')
                                .map(|(first, _)| first)
                                .zip(o.split_once('.').map(|(first, _)| first))
                                .is_some_and(|(a, b)| a == b)
                    })
                {
                    push(
                        "brand_homograph",
                        format!(
                            "registered domain `{decoded_registered}` is a lookalike of `{token}`                              (skeleton `{skeleton_registered}`)"
                        ),
                        &mut hits,
                    );
                    break 'brand;
                }
            }
        }
        let _ = skeleton_host;
        if parts.query.split('&').any(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (k.eq_ignore_ascii_case("url")
                || k.eq_ignore_ascii_case("link")
                || k.eq_ignore_ascii_case("redirect")
                || k.eq_ignore_ascii_case("redirect_uri")
                || k.eq_ignore_ascii_case("next")
                || k.eq_ignore_ascii_case("goto")
                || k.eq_ignore_ascii_case("dest"))
                && (v.starts_with("http://") || v.starts_with("https://") || v.starts_with("//"))
        }) {
            push(
                "redirect_param",
                "query embeds a full URL in a redirect-style parameter".into(),
                &mut hits,
            );
        }
        if parts.query.split('&').any(|kv| {
            kv.starts_with("utm_")
                || kv.starts_with("mc_cid")
                || kv.starts_with("mc_eid")
                || kv.starts_with("yclid")
                || kv.starts_with("fbclid")
        }) {
            push(
                "tracking_params",
                "URL carries analytics/tracking parameters".into(),
                &mut hits,
            );
        }
    }

    if url.len() > 120 {
        push(
            "long_url",
            format!("URL is {} characters long", url.len()),
            &mut hits,
        );
    }
    if url.matches('%').count() >= 4 {
        push(
            "heavy_percent_encoding",
            format!(
                "URL contains {} percent-encoded octets",
                url.matches('%').count()
            ),
            &mut hits,
        );
    }
    if is_unsubscribe {
        push(
            "unsubscribe_classified",
            "link matches list-unsubscribe machinery; classified, not scored".into(),
            &mut hits,
        );
    }

    hits.sort_unstable_by(|a, b| b.weight.cmp(&a.weight).then_with(|| a.name.cmp(b.name)));
    let score: u32 = hits.iter().map(|h| h.weight as u32).sum();
    let score = score.min(100) as u8;
    ScoredUrl {
        url: url.to_string(),
        score,
        risk: Risk::of(score, low_medium, medium_high),
        signals: hits,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weights() -> Weights {
        Weights::new(|_, w| w)
    }

    fn score(url: &str) -> ScoredUrl {
        score_url(url, None, &weights(), 30, 60)
    }

    fn score_with_display(url: &str, display: &str) -> ScoredUrl {
        score_url(url, Some(display), &weights(), 30, 60)
    }

    fn names(s: &ScoredUrl) -> Vec<&'static str> {
        s.signals.iter().map(|h| h.name).collect()
    }

    #[test]
    fn test_punycode_decode_ascii_only() {
        assert_eq!(punycode_decode("hello-"), Some("hello".to_string()));
        // A label with no basic separator is entirely delta-encoded; a
        // bare ASCII word only round-trips through the `xn--word-` form.
        assert_eq!(punycode_decode("abc-"), Some("abc".to_string()));
    }

    #[test]
    fn test_punycode_decode_homograph_label_skeleton() {
        // The classic "apple" Cyrillic homograph prefix
        // (xn--80ak6aa92e.com decodes to `аррӏе`, an all-Cyrillic
        // lookalike of "apple"): the decoded label must be non-ASCII and
        // its confusable skeleton must read "apple".
        let decoded = punycode_decode("80ak6aa92e").expect("decodes");
        assert_ne!(decoded, "apple");
        assert!(decoded.chars().all(|c| !c.is_ascii()));
        assert_eq!(confusable_skeleton(&decoded), "apple");
        // A genuinely mixed-script label: `café`.
        let mixed = punycode_decode("caf-dma").expect("decodes");
        assert!(!mixed.is_ascii());
        assert!(mixed.chars().any(|c| c.is_ascii_alphabetic()));
        assert_eq!(mixed, "café");
    }

    #[test]
    fn test_punycode_decode_malformed() {
        assert!(punycode_decode("").is_some());
        // Invalid digit characters
        assert!(punycode_decode("!!").is_none());
    }

    // ---- per-signal positive/negative coverage ----

    #[test]
    fn test_signal_scheme_dangerous() {
        let s = score("javascript:alert(1)");
        assert!(names(&s).contains(&"scheme_dangerous"));
        assert_eq!(s.risk, Risk::High);
        let s = score("https://meli-email.org");
        assert!(!names(&s).contains(&"scheme_dangerous"));
        assert_eq!(s.score, 0);
    }

    #[test]
    fn test_signal_scheme_non_web() {
        let s = score("ftp://ftp.example.org/pub");
        assert!(names(&s).contains(&"scheme_non_web"));
        assert!(!names(&s).contains(&"scheme_dangerous"));
        let s = score("https://example.org");
        assert!(!names(&s).contains(&"scheme_non_web"));
    }

    #[test]
    fn test_signal_ip_literal() {
        let s = score("http://127.0.0.1:8080/login");
        assert!(names(&s).contains(&"ip_literal_host"));
        assert!(names(&s).contains(&"nonstandard_port"));
        let s = score("https://[2001:db8::1]/");
        assert!(names(&s).contains(&"ip_literal_host"));
        let s = score("https://example.org");
        assert!(!names(&s).contains(&"ip_literal_host"));
    }

    #[test]
    fn test_signal_encoded_ipv4() {
        let s = score("http://2130706433/paypal/login");
        assert!(names(&s).contains(&"obfuscated_ip_encoding"));
        assert!(names(&s).contains(&"ip_literal_host"));
        assert!(s.score >= 60, "encoded IP must reach high: {}", s.score);
        let s = score("http://0x7f000001/");
        assert!(names(&s).contains(&"obfuscated_ip_encoding"));
        let s = score("http://127.1/");
        assert!(names(&s).contains(&"obfuscated_ip_encoding"));
        // Negative: ordinary hostnames never decode to IPs.
        let s = score("https://example.org");
        assert!(!names(&s).contains(&"obfuscated_ip_encoding"));
        let s = score("https://0-static.example.org");
        assert!(!names(&s).contains(&"obfuscated_ip_encoding"));
    }

    #[test]
    fn test_signal_credentials_in_url() {
        let s = score("http://user:pass@site.example/");
        assert!(names(&s).contains(&"credentials_in_url"));
        let s = score("https://site.example/");
        assert!(!names(&s).contains(&"credentials_in_url"));
    }

    #[test]
    fn test_signal_href_text_mismatch() {
        let s = score_with_display("http://192.168.13.37/verify", "https://secure.bank.example");
        assert!(names(&s).contains(&"href_text_mismatch"));
        assert!(s.score >= 60);
        // Identical hosts: no mismatch.
        let s = score_with_display("https://github.com/meli", "https://github.com/meli");
        assert!(!names(&s).contains(&"href_text_mismatch"));
        // Prose display text ("Click here") is not a URL: no mismatch.
        let s = score_with_display("https://github.com/meli", "Click here");
        assert!(!names(&s).contains(&"href_text_mismatch"));
    }

    #[test]
    fn test_signal_shortener() {
        let s = score("https://bit.ly/3abcXYZ");
        assert!(names(&s).contains(&"shortener_domain"));
        assert!(s.score < 30, "shortener alone stays low: {}", s.score);
        let s = score("https://meli-email.org");
        assert!(!names(&s).contains(&"shortener_domain"));
    }

    #[test]
    fn test_signal_subdomains() {
        let s = score("http://a.b.c.d.example.org/");
        assert!(names(&s).contains(&"excessive_subdomains"));
        let s = score("https://en.wikipedia.org/wiki/Email");
        assert!(!names(&s).contains(&"excessive_subdomains"));
        let s = score("http://5jnv93kd0a.example.org/");
        assert!(names(&s).contains(&"random_looking_subdomain"));
        let s = score("https://docs.python.org/3/");
        assert!(!names(&s).contains(&"random_looking_subdomain"));
    }

    #[test]
    fn test_signal_brand_in_subdomain() {
        let s = score("http://paypal.com.secure-login.example.ru/");
        assert!(names(&s).contains(&"brand_in_subdomain"));
        assert!(
            s.score >= 60,
            "brand-in-subdomain phish must reach high: {}",
            s.score
        );
        // Negative: the brand's own subdomains.
        let s = score("https://outlook.office365.com/mail/");
        assert!(!names(&s).contains(&"brand_in_subdomain"));
        let s = score("https://docs.microsoft.com/en-us/windows/");
        assert!(!names(&s).contains(&"brand_in_subdomain"));
        let s = score("https://accounts.google.com/signin/");
        assert!(!names(&s).contains(&"brand_in_subdomain"));
    }

    #[test]
    fn test_signal_homograph_punycode() {
        // All-Cyrillic `аррӏе.com` homograph: punycode + brand skeleton.
        let s = score("https://xn--80ak6aa92e.com/");
        assert!(names(&s).contains(&"idn_punycode_label"));
        assert!(names(&s).contains(&"brand_homograph"));
        assert_eq!(s.risk, Risk::High);
        // Mixed-script label (ASCII + non-ASCII in one label).
        let s = score("https://xn--caf-dma.com/");
        assert!(names(&s).contains(&"mixed_script_homograph"));
        // Negative: an ASCII-only host never trips the IDN signals.
        let s = score("https://meli-email.org/");
        assert!(!names(&s).contains(&"idn_punycode_label"));
        assert!(!names(&s).contains(&"brand_homograph"));
        assert!(!names(&s).contains(&"mixed_script_homograph"));
    }

    #[test]
    fn test_signal_tld_hyphens_length_encoding() {
        let s = score("http://free-stuff.example.top/");
        assert!(names(&s).contains(&"suspicious_tld"));
        let s = score("http://get-your-free-prize-now.example.org/");
        assert!(names(&s).contains(&"hyphen_heavy_host"));
        let s = score("http://example.org/");
        assert!(!names(&s).contains(&"hyphen_heavy_host"));
        let long = format!("https://example.org/{}", "a".repeat(140));
        assert!(names(&score(&long)).contains(&"long_url"));
        let s = score("https://example.org/%61%62%63%64/");
        assert!(names(&s).contains(&"heavy_percent_encoding"));
    }

    #[test]
    fn test_signal_tracker_and_unsubscribe_classification() {
        let s = score("https://click.list-manage.com/unsubscribe?u=abc&id=1");
        assert!(names(&s).contains(&"tracker_domain"));
        assert!(names(&s).contains(&"unsubscribe_classified"));
        assert!(
            s.score < 30,
            "newsletter unsubscribe stays low: {}",
            s.score
        );
        let s = score("https://example.org/?utm_source=newsletter");
        assert!(names(&s).contains(&"tracking_params"));
    }

    #[test]
    fn test_signal_redirect_param() {
        let s = score("https://example.org/redirect?url=https://evil.example/");
        assert!(names(&s).contains(&"redirect_param"));
        let s = score("https://example.org/page?id=42");
        assert!(!names(&s).contains(&"redirect_param"));
    }

    #[test]
    fn test_mailto_is_classified_not_scored() {
        let s = score("mailto:user@example.org?subject=hi");
        assert_eq!(s.score, 0);
        assert_eq!(s.risk, Risk::Low);
        assert!(names(&s).contains(&"mailto_classified"));
    }

    // ---- corpus regression ----

    #[test]
    fn test_phishing_corpus_all_high() {
        let corpus = [
            ("http://paypal.com.secure-login.example.ru/", None),
            ("http://2130706433/paypal/webscr/login", None),
            ("https://xn--80ak6aa92e.com/", None),
            (
                "http://192.168.13.37:8080/verify",
                Some("https://secure.bank.example"),
            ),
            ("http://secure-login.example.com@93.184.216.34/", None),
            ("ftp://user@ftp.evil.example:2121/bank.exe", None),
            (
                "data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==",
                None,
            ),
        ];
        for (url, display) in corpus {
            let s = score_url(url, display, &weights(), 30, 60);
            assert_eq!(
                s.risk,
                Risk::High,
                "{url} scored {} ({:?})",
                s.score,
                s.signals
            );
        }
    }

    #[test]
    fn test_normal_corpus_zero_false_positives() {
        let corpus = [
            "https://github.com/meli-delivery/meli/pull/123",
            "https://meli-email.org/",
            "https://www.google.com/search?q=meli+email+client",
            "https://en.wikipedia.org/wiki/Email",
            "mailto:manos@pitsidianak.is?subject=meli",
            "https://unsubscribe.list-manage.com/unsubscribe?u=1&id=2",
            "https://bit.ly/3abcXYZ",
            "https://docs.microsoft.com/en-us/windows/apps/",
            "https://outlook.office365.com/mail/inbox",
            "https://accounts.google.com/signin/v2",
            "https://storage.googleapis.com/bucket/object.png",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
        ];
        for url in corpus {
            let s = score(url);
            assert_eq!(
                s.risk,
                Risk::Low,
                "{url} scored {} ({:?})",
                s.score,
                s.signals
            );
        }
    }

    #[test]
    fn test_custom_thresholds_and_weights() {
        let weights = Weights::new(|name, default| {
            if name == "ip_literal_host" {
                10
            } else {
                default
            }
        });
        let s = score_url("http://127.0.0.1/", None, &weights, 10, 20);
        assert!(
            s.score >= 10,
            "custom low_medium=10 makes an IP host medium"
        );
        assert_eq!(s.risk, Risk::Medium);
    }
}
