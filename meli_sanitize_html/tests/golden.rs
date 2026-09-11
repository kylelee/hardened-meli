// SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later
//! Golden parity tests for [`meli_sanitize_html::sanitize`].
//!
//! Every expected value below is a byte-exact fixture captured from
//! the reference Python sanitizer (nh3 0.3.6, ammonia bindings) at
//! `~/.config/meli/sanitize_html.py` on this machine:
//!
//! ```sh
//! printf '%s' '<input>' | python3 ~/.config/meli/sanitize_html.py
//! ```
//!
//! Never hand-edit the expected strings; re-run the oracle instead.

/// `(input, expected)` pairs; the numbered comment above each tuple
/// names the case class it locks.
const CASES: &[(&str, &str)] = &[
    // 1. basic whitelist retention: whitelisted tags pass through unchanged.
    (
        r#"<p>hello <b>world</b></p>"#,
        r#"<p>hello <b>world</b></p>"#,
    ),
    // 2. non-whitelist tag stripped keeping children text.
    (r#"<div>text</div>"#, r#"text"#),
    // 3. script removed with its content, trailing text kept.
    (r#"<script>alert(1)</script>after"#, r#"after"#),
    // 4. style removed with its content, trailing text kept.
    (r#"<style>p{}</style>x"#, r#"x"#),
    // 5. javascript: URL scheme rejected (href dropped, anchor kept).
    (
        r#"<a href="javascript:alert(1)">x</a>"#,
        r#"<a rel="noopener noreferrer">x</a>"#,
    ),
    // 6. data: scheme rejected, mailto: kept in one input.
    (
        r#"<a href="data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==">bad</a> <a href="mailto:x@y.z">good</a>"#,
        r#"<a rel="noopener noreferrer">bad</a> <a href="mailto:x@y.z" rel="noopener noreferrer">good</a>"#,
    ),
    // 7. relative URL passed through (url_relative=PassThrough).
    (
        r#"<a href="/path?q=1">x</a>"#,
        r#"<a href="/path?q=1" rel="noopener noreferrer">x</a>"#,
    ),
    // 8. a[href]/a[title] kept, name dropped, rel replaced by link_rel noopener noreferrer.
    (
        r#"<a href="https://e.example" title="t" name="n" rel="stylesheet">x</a>"#,
        r#"<a href="https://e.example" title="t" rel="noopener noreferrer">x</a>"#,
    ),
    // 9. generic attrs: lang/title kept via generic_attributes, class dropped.
    (
        r#"<p lang="en" title="t" class="c">x</p>"#,
        r#"<p lang="en" title="t">x</p>"#,
    ),
    // 10. HTML comment stripped (strip_comments).
    (r#"<!-- c --><p>x</p>"#, r#"<p>x</p>"#),
    // 11. template element vanishes entirely with its content.
    (r#"<template><p>t</p></template>after"#, r#"after"#),
    // 12. nested mismatched tags normalized by the HTML parser.
    (r#"<b><i>x</b></i>"#, r#"<b><i>x</i></b>"#),
    // 13. entities: named and numeric character references re-serialized.
    (
        r#"<p>&amp; &lt; &copy; &#x27; &#128169;</p>"#,
        r#"<p>&amp; &lt; © ' 💩</p>"#,
    ),
    // 14. td attributes cleared (tag_attributes full-replacement proof).
    (
        r#"<table><tr><td align="center">c</td></tr></table>"#,
        r#"<table><tr><td>c</td></tr></table>"#,
    ),
    // 15. SVG/MathML namespace script removed with content.
    (r#"<svg><script>alert(1)</script></svg>"#, r#""#),
    // 16a. ul whitelist regression lock: plain list.
    (
        r#"<ul><li>a</li><li>b</li></ul>"#,
        r#"<ul><li>a</li><li>b</li></ul>"#,
    ),
    // 16b. ul whitelist regression lock: nested inside ol.
    (
        r#"<ol><li>x<ul><li>y</li></ul></li></ol>"#,
        r#"<ol><li>x<ul><li>y</li></ul></li></ol>"#,
    ),
];

#[test]
fn golden_parity_with_python_nh3() {
    for (input, expected) in CASES {
        assert_eq!(
            meli_sanitize_html::sanitize(input),
            *expected,
            "input: {input}"
        );
    }
}
