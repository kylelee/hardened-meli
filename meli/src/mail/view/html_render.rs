/*
 * meli
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
 * along with meli. If not, see <http://www.gnu.org/licenses/>.
 *
 * SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later
 */

//! Built-in HTML rendering for the mail view: sanitize untrusted HTML with
//! [`ammonia`], then render it to plain text with [`html2text`].

use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    sync::Arc,
};

use melib::{Error, Result};

/// Tag/attribute/scheme 白名单与 `~/.config/meli/sanitize_html.py` 的 `nh3.clean()`
/// 参数逐项对齐；其余 Builder 设置**刻意保留 ammonia 默认**以维持 nh3 行为
/// 一致（nh3 即 ammonia 的 Python 绑定，锁定同版本 4.1.4）：
/// - `link_rel` 默认 Some("noopener noreferrer")：nh3 默认相同
/// - `url_relative` 默认 PassThrough：相对 URL 原样保留，nh3 默认相同
/// - `generic_attributes` 默认 {"lang","title"}：**勿清空**，nh3 未覆盖时保留同款默认
/// - `clean_content_tags` 默认 {"script","style"}：连内容一起删除，nh3 相同
///
/// 与 nh3 parity 的**有意偏离**：`attribute_filter` 对每个保留的 attribute
/// value 做首尾 trim（ASCII/Unicode 空白 + ASCII/Unicode 控制字符 +
/// 常见隐形填充符 U+200B-U+200F、U+2060、U+FEFF、U+00AD），整段 fragment
/// 末尾同样 trim 一次。nh3 自身不裁这些，留给消费者做后处理；meli 在
/// sanitize 阶段直接处理，因为 sanitize 输出紧接着喂给 `html2text`
/// 渲染，过早的不可见填充会让文本宽度错算并出现"幽灵"链接脚注。
pub(crate) fn sanitize(input: &str) -> String {
    let tags: HashSet<&str> = [
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
    ]
    .into();
    let mut tag_attributes: HashMap<&str, HashSet<&str>> = HashMap::new();
    tag_attributes.insert("a", ["href", "title"].into());
    let cleaned = ammonia::Builder::default()
        .tags(tags)
        .tag_attributes(tag_attributes)
        .url_schemes(["http", "https", "mailto"].into())
        .strip_comments(true)
        .attribute_filter(|_element, attr, value| {
            // Trim leading/trailing whitespace + invisible chars from every
            // kept attribute value. Return `Borrowed` (not `None`) when
            // nothing changed: ammonia's contract is `None` = drop,
            // `Some` = keep (replace only if differs).
            let trimmed = value.trim_matches(trim_predicate);
            if trimmed.len() == value.len() {
                return Some(Cow::Borrowed(value));
            }
            // A value made entirely of trimmable chars (e.g. `href="   "`)
            // becomes `href=""` after trim and would render as a phantom
            // footnote by html2text. Drop the attribute instead.
            if trimmed.is_empty() {
                return None;
            }
            // ammonia validated URL schemes on the *original* (pre-trim)
            // value in `clean_child`, before this filter runs, and never
            // re-checks what we write back. A WHATWG URL parser only
            // strips C0 controls + ASCII space before scheme detection,
            // so `Url::parse` of a Cf-padded value (e.g.
            // `\u{200B}javascript:alert(1)`) returns
            // `RelativeUrlWithoutBase` — the untrimmed value would be
            // kept as inert relative text, but trimming the prefix
            // *activates* the scheme into a live `javascript:` href, the
            // exact regression golden case 5 exists to prevent.
            // Re-validate; drop on doubt.
            if attr == "href" && !is_safe_url(trimmed) {
                return None;
            }
            Some(Cow::Owned(trimmed.to_owned()))
        })
        .clean(input)
        .to_string();
    // Strip leading/trailing whitespace + invisible chars from the whole
    // fragment. Inner `<pre>` whitespace is preserved by html5ever
    // serialization.
    cleaned.trim_matches(trim_predicate).to_owned()
}

/// Re-validate a trimmed `href` against the same allowlist ammonia used on
/// the original value. Returns `true` for URLs whose scheme parses as one
/// of the configured allowlist (`http`/`https`/`mailto`) or for relative
/// URLs (`RelativeUrlWithoutBase`, kept by `url_relative = PassThrough`).
fn is_safe_url(value: &str) -> bool {
    match url::Url::parse(value) {
        Ok(u) => matches!(u.scheme(), "http" | "https" | "mailto"),
        Err(url::ParseError::RelativeUrlWithoutBase) => true,
        Err(_) => false,
    }
}

/// Character predicate: matches ASCII whitespace, Unicode `White_Space`,
/// ASCII/Unicode control chars, and the Format-category invisibles used as
/// padding/spoof markers in spam and Trojan-Source-style reordering
/// attacks (covers tab/CR/LF, NBSP, zero-width and bidi controls, BOM,
/// soft hyphen, word joiner, mathematical invisible operators, `\0`, etc.).
fn trim_predicate(c: char) -> bool {
    c.is_whitespace()
        || c.is_control()
        || matches!(
            c,
            '\u{00AD}' // SOFT HYPHEN
            | '\u{061C}' // ARABIC LETTER MARK
            | '\u{180E}' // MONGOLIAN VOWEL SEPARATOR (Cf zero-width)
            | '\u{200B}'..='\u{200F}' // ZWSP / ZWNJ / ZWJ / LRM / RLM
            | '\u{202A}'..='\u{202E}' // LRE / RLE / PDF / LRO / RLO (bidi overrides)
            | '\u{2060}' // WORD JOINER
            | '\u{2061}'..='\u{2064}' // FUNCTION APPLICATION / INVISIBLE TIMES/SEP/PLUS
            | '\u{2066}'..='\u{2069}' // LRI / RLI / FSI / PDI (bidi isolates)
            | '\u{FEFF}' // ZERO WIDTH NO-BREAK SPACE / BOM
        )
}

/// Sanitize and render HTML `bytes` to plain text wrapped at `width` display
/// columns.
///
/// Never panics: invalid UTF-8 input is replaced lossily, sanitization always
/// returns a `String`, and rendering failures are reported as [`Error`].
pub(crate) fn render(bytes: &[u8], width: usize) -> Result<String> {
    let html = sanitize(&String::from_utf8_lossy(bytes));
    html2text::config::plain()
        .string_from_read(html.as_bytes(), width)
        .map_err(|err| Error::new("Could not render html to text").set_source(Some(Arc::new(err))))
}

#[cfg(test)]
mod tests {
    use melib::text::TextProcessing;

    use super::*;

    /// Golden parity tests for [`sanitize`].
    ///
    /// Every expected value below is a byte-exact fixture captured from
    /// the reference Python sanitizer (nh3 0.3.6, ammonia bindings) at
    /// `~/.config/meli/sanitize_html.py` on this machine:
    ///
    /// ```sh
    /// printf '%s' '<input>' | python3 ~/.config/meli/sanitize_html.py
    /// ```
    ///
    /// Never hand-edit the expected strings; re-run the oracle instead.
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
            assert_eq!(sanitize(input), *expected, "input: {input}");
        }
    }

    #[test]
    fn render_basic_html_to_text() {
        let text = render(b"<p>hello <b>world</b></p>", 40).unwrap();
        // `plain()` keeps the markdown-style emphasis decoration, so bold
        // text is wrapped in `**...**` instead of being flattened.
        assert_eq!(text, "hello **world**\n");
    }

    #[test]
    fn render_list_to_text() {
        let text = render(b"<ul><li>a</li><li>b</li></ul>", 40).unwrap();
        assert_eq!(text, "* a\n* b\n");
    }

    #[test]
    fn render_link_to_text() {
        let text = render(
            br#"<p>see <a href="https://example.com/x">docs</a></p>"#,
            40,
        )
        .unwrap();
        // `plain()` enables link footnotes: the anchor text carries a
        // numbered reference and the target URL is listed at the end of the
        // output, so the link destination stays visible to the reader
        // (phishing visibility). `rich()` would instead hide the URL inside
        // the `RichAnnotation` stream that `into_string()` discards.
        assert!(
            text.contains("see [docs][1]"),
            "unexpected render: {text:?}"
        );
        assert!(
            text.contains("[1]: https://example.com/x"),
            "unexpected render: {text:?}"
        );
    }

    #[test]
    fn render_cjk_paragraph_wraps_by_display_width() {
        let html = "<p>这是一段比较长的中文段落，用来验证内置渲染器是否按照显示列宽进行折行处理，而不是按照字符个数来计算每行长度。</p>";
        let text = render(html.as_bytes(), 20).unwrap();
        for line in text.lines() {
            assert!(
                line.grapheme_width() <= 20,
                "line {line:?} is {} display columns wide, expected <= 20",
                line.grapheme_width()
            );
        }
    }

    #[test]
    fn render_empty_input() {
        assert_eq!(render(&[], 20).unwrap(), "");
    }

    #[test]
    fn render_malformed_html_does_not_panic() {
        let text = render(br#"<p>hello <b>world</i></div><span"#, 40)
            .expect("malformed html must not fail rendering");
        assert!(text.contains("world"), "unexpected render: {text:?}");
    }

    #[test]
    fn render_large_input_does_not_panic() {
        let mut html = String::with_capacity(1 << 21);
        html.push_str("<p>");
        for _ in 0..40_000 {
            html.push_str("lorem ipsum dolor sit amet ");
        }
        html.push_str("</p>");
        assert!(
            html.len() >= 1_000_000,
            "test input must be at least 1MB, got {}",
            html.len()
        );
        let text = render(html.as_bytes(), 80).expect("large html must not fail rendering");
        assert!(text.contains("lorem"), "unexpected render head");
    }

    /// Trim covers ASCII whitespace, NBSP, zero-width spaces, BOM and ASCII
    /// control chars on both the whole fragment and each attribute value.
    #[test]
    fn sanitize_trims_attribute_values_and_fragment_ends() {
        // attribute value padded with whitespace + invisible chars + BOM.
        let out = sanitize(r#"<p><a href="  https://x.example  ">go</a></p>"#);
        assert_eq!(
            out,
            r#"<p><a href="https://x.example" rel="noopener noreferrer">go</a></p>"#
        );

        // NBSP (U+00A0) and zero-width space (U+200B) trimmed from value.
        let out = sanitize("<p><a href=\"\u{00A0}\u{200B}https://y.example\u{00A0}\">x</a></p>");
        assert!(
            out.contains(r#"href="https://y.example""#),
            "NBSP/ZWSP not trimmed: {out:?}"
        );

        // Whole-fragment leading/trailing whitespace + tab + BOM removed.
        let out = sanitize("   \t\u{feff}<p>x</p>\n\n  ");
        assert_eq!(out, "<p>x</p>");

        // inner whitespace inside `<pre>` is preserved (we only trim fragment
        // ends, not intermediate text).
        let out = sanitize("<pre>  keep me  </pre>");
        assert_eq!(out, "<pre>  keep me  </pre>");

        // whitespace-only href: trim yields `""` so the attribute is dropped
        // (no phantom `href=""` link footnote in html2text output).
        let out = sanitize(r#"<a href="   ">x</a>"#);
        assert_eq!(out, r#"<a rel="noopener noreferrer">x</a>"#);
    }

    /// When ammonia drops the rejected `href` (here `javascript:`), the
    /// default `rel` is still injected by `link_rel` afterward. The
    /// `attribute_filter` must not delete `rel` when the injected value has
    /// nothing to trim — regression guard for the
    /// `Some(Cow::Borrowed)` vs `None` semantic in ammonia's contract.
    #[test]
    fn attribute_filter_keeps_injected_rel_unchanged() {
        let out = sanitize(r#"<a href="javascript:alert(1)">x</a>"#);
        assert_eq!(out, r#"<a rel="noopener noreferrer">x</a>"#);
    }

    /// Text inside `<pre>`/`<code>` is not affected by attribute trimming
    /// (regression guard: callback must only touch Element attribute values).
    #[test]
    fn sanitize_does_not_touch_text_inside_pre_or_code() {
        let out = sanitize(r#"<pre>x = " y "</pre><code>q = " z "</code>"#);
        assert!(
            out.contains(r#"<pre>x = " y "</pre>"#),
            "pre text changed: {out:?}"
        );
        assert!(
            out.contains(r#"<code>q = " z "</code>"#),
            "code text changed: {out:?}"
        );
    }

    /// Regression: ammonia's URL scheme check runs on the *pre-trim* value
    /// (where Cf padding hides the scheme from `Url::parse`, so the value
    /// survives as a `RelativeUrlWithoutBase` under `PassThrough`). The
    /// `attribute_filter` must re-validate after stripping Cf prefix — a
    /// trimmed `javascript:` href would otherwise be reactivated.
    #[test]
    fn sanitize_drops_href_whose_cf_padding_hides_javascript_scheme() {
        // ZWSP-prefixed javascript: must drop href entirely (not trim it
        // into a live javascript:).
        let out = sanitize("<a href=\"\u{200B}javascript:alert(1)\">x</a>");
        assert!(
            !out.contains("href"),
            "ZWSP-padded javascript: href survived: {out:?}"
        );
        assert!(
            out.contains("rel=\"noopener noreferrer\""),
            "missing rel: {out:?}"
        );

        // NBSP-prefixed javascript: same.
        let out = sanitize("<a href=\"\u{00A0}javascript:alert(1)\">x</a>");
        assert!(
            !out.contains("href"),
            "NBSP-padded javascript: href survived: {out:?}"
        );

        // BOM-prefixed javascript: same.
        let out = sanitize("<a href=\"\u{FEFF}javascript:alert(1)\">x</a>");
        assert!(
            !out.contains("href"),
            "BOM-padded javascript: href survived: {out:?}"
        );

        // SAFE: ZWSP-prefixed https:// — re-validates to `https` (allowed).
        let out = sanitize("<a href=\"\u{200B}https://ok.example\">x</a>");
        assert!(
            out.contains(r#"href="https://ok.example""#),
            "safe https href was wrongly dropped: {out:?}"
        );

        // SAFE: relative URL stays relative after trim.
        let out = sanitize(r#"<a href="  /path?q=1  ">x</a>"#);
        assert!(
            out.contains(r#"href="/path?q=1""#),
            "relative href not trimmed: {out:?}"
        );
    }

    /// Regression: Cf-category invisibles (bidi controls U+202A–U+202E /
    /// U+2066–U+2069, U+061C, U+180E, math invisibles) are spoof markers;
    /// they must be trimmed like the other Format-category characters.
    #[test]
    fn sanitize_trims_bidi_controls_and_arabic_letter_mark() {
        for (label, c) in [
            ("LRE", '\u{202A}'),
            ("RLO", '\u{202E}'),
            ("LRI", '\u{2066}'),
            ("PDI", '\u{2069}'),
            ("ALM", '\u{061C}'),
            ("MVS", '\u{180E}'),
            ("INVISIBLE_TIMES", '\u{2062}'),
        ] {
            let fragment = format!("{c}<p>x</p>{c}");
            let out = sanitize(&fragment);
            assert_eq!(out, "<p>x</p>", "{label} ({c:?}) not trimmed: {out:?}");
        }
    }

    /// When every char in an attribute value is trimmable (whitespace, Cf,
    /// etc.), trim yields `""`. html2text would otherwise emit a phantom
    /// link footnote for `href=""`. Drop the attribute instead.
    #[test]
    fn sanitize_drops_attribute_when_trim_yields_empty() {
        // Whitespace-only href must drop (not become `href=""`).
        let out = sanitize(r#"<a href="   ">x</a>"#);
        assert!(
            !out.contains("href"),
            "whitespace-only href survived: {out:?}"
        );

        // ZWSP-only title must drop too.
        let out = sanitize("<p><a href=\"https://e.example\" title=\"\u{200B}\">x</a></p>");
        assert!(!out.contains("title"), "ZWSP-only title survived: {out:?}");
        assert!(
            out.contains(r#"href="https://e.example""#),
            "valid href should still pass through: {out:?}"
        );
    }

    /// Tag whitelist mirror of [`sanitize`]'s configuration. Deliberately a
    /// separate copy: it is the independent oracle the adversarial corpus
    /// below is checked against, so widening `sanitize`'s policy without
    /// touching this list fails the corpus tests.
    const INERTNESS_TAG_WHITELIST: &[&str] = &[
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

    /// CVE-2025-66376 (Zimbra, Proofpoint TA488 "tag-splitting") corpus:
    /// `(name, payload)` pairs. Zimbra's client-side sanitizer rewrote the
    /// HTML *string* — stripping `@import` runs and comments — and handed
    /// the mutated string back to the browser to re-parse, so fragments
    /// like `scr@import …;ipt` reassembled into a live `script` /
    /// `<svg onload=eval(atob(…))>` tag. meli's pipeline parses the input
    /// exactly once with a spec-compliant tokenizer, filters the resulting
    /// tree, and re-serializes with escaping — there is no second parse of
    /// mutated text for fragments to reassemble in, and the output then
    /// feeds html2text (plain terminal text), not a JS engine. Payload 1 is
    /// the verbatim exploit string from the Proofpoint report; the rest are
    /// its fragment recipes, the `display:none` carrier, and mXSS classics
    /// of the same sanitizer/browser parse-differential family.
    const TAG_SPLITTING_CORPUS: &[(&str, &str)] = &[
        // 1. verbatim stored-XSS string from the TA488 report: fake
        // `@import` directives and comments fragmenting
        // `<svg onload=eval(atob(...))>` plus a `</s…tyle>` breakout.
        (
            "proofpoint_verbatim",
            r#"</scr@import FHBCuUYUGEQODuCrzISjiZsOR;ipt>WqRodzBMC</s<!--WmxNBeNgyFe@import WjTgoKQtWXrfBKAUnMVGQsKBFCvmwd;JbFJPbKH-->tyle><s_@import;_v__g/___KttsYfUnHEmwoXouXy/___onlo___@import zZhGLNPLxJ;___ad=ev___@import acXApYgYEaXwpIprFa;___al(at__@_import poxscPRqHcoGoodXaFvoY;___ob__("#,
        ),
        // 2. the report's three fragment recipes inside the hidden
        // `display:none` div carrier.
        (
            "hidden_div_fragments",
            r#"<div style="display:none">scr@import FHBCuUYUGEQODuCrzISjiZsOR;ipt onlo@import zZhGLNPLxJ;ad=ev@import acXApYgYEaXwpIprFa;al(at@import poxscPRqHcoGoodXaFvoY;ob('cHcxMjM='))</div>"#,
        ),
        // 3. comment-split script tag: after one spec parse it is a single
        // unknown tag token, never two joinable fragments.
        (
            "comment_split_script",
            r#"<p>hi</p><scr<!--WmxNBeNgyFe-->ipt>alert(1)</scr<!--WmxNBeNgyFe-->ipt>"#,
        ),
        // 4. `</s<!--…-->tyle>` breakout riding on a style element.
        (
            "style_carried_fragments",
            r#"<style>x</s<!--c-->tyle><svg onload=eval(atob('Zm9v'))></style>"#,
        ),
        // 5. the unsplit payload itself.
        (
            "plain_svg_onload",
            r#"<div style="display:none"><svg onload="eval(atob('Z2Q='))"></svg></div>"#,
        ),
        // 6. mXSS classic: SVG-namespace `style` is not raw text, so the
        // quoted `id` used to smuggle `</style><img onerror=…>` past
        // innerHTML-round-trip sanitizers.
        (
            "mxss_svg_style_a",
            r#"<svg><style><a id="</style><img src=x onerror=eval()>">"#,
        ),
        // 7. mXSS classic: mglyph/mtext MathML integration points plus a
        // comment-smuggled `</style>`.
        (
            "mxss_math_mglyph",
            r#"<math><mtext><table><mglyph><style><!--</style><img title="--></mglyph><svg onload=eval()>">"#,
        ),
        // 8. mXSS classic: noscript scripting-flag differential.
        (
            "mxss_noscript",
            r#"<noscript><p title="</noscript><svg onload=eval()>">"#,
        ),
        // 9. mXSS classic: form/math foreign-content breakout.
        (
            "mxss_form_math",
            r#"<form><math><mtext></form><form><mglyph><style></math><img src onerror=eval(1)>"#,
        ),
        // 10. whitelisted attribute whose value carries markup-shaped text;
        //     the serializer must escape it.
        (
            "attr_title_carries_markup",
            r#"<a title="</style><svg onload=eval()>">x</a>"#,
        ),
        // 11. comment removal can join adjacent text nodes — the joined
        //     result must remain escaped text, never markup.
        ("text_reassembly_only", r#"<p>onlo<!--c-->ad=alert(1)</p>"#),
        // 12. `@import` cargo inside a style element dies with the whole
        //     element; it is never stripped piecemeal.
        (
            "import_inside_style_only",
            r#"<style>@import url(evil);s_@import;_v_g/onload=ev</style>ok"#,
        ),
    ];

    /// Extract every tag-shaped construct (`<name …>`, `</name>`, `<!…>`,
    /// `<?…>`) from serialized HTML, skipping text and character
    /// references. Quoted attribute values are skipped so a `<` inside a
    /// value cannot be misread as a tag boundary.
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

    /// Assert one serialized tag is inert: a whitelisted (end-)tag name,
    /// only whitelisted attributes, and `href` values re-validated by the
    /// module's own scheme rules.
    fn assert_tag_is_inert(tag: &str) {
        let inner = tag
            .strip_prefix('<')
            .and_then(|rest| rest.strip_suffix('>'))
            .unwrap_or_default();
        let body = inner.strip_prefix('/').unwrap_or(inner);
        let name: String = body
            .chars()
            .take_while(|c| !c.is_ascii_whitespace() && *c != '/')
            .collect::<String>()
            .to_ascii_lowercase();
        assert!(
            INERTNESS_TAG_WHITELIST.contains(&name.as_str()),
            "non-whitelisted tag survived sanitization: {tag:?}"
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
                matches!(attr.as_str(), "href" | "title" | "rel" | "lang"),
                "non-whitelisted attribute survived sanitization: {tag:?}"
            );
            if attr == "href" {
                if let Some((_, value)) = word.split_once('=') {
                    assert!(
                        is_safe_url(value.trim_matches(['"', '\''])),
                        "unsafe href survived sanitization: {tag:?}"
                    );
                }
            }
        }
    }

    /// Inertness oracle for adversarial input: every tag-shaped construct
    /// in the output passes the whitelist scan, and the output is a
    /// `sanitize` fixed point — a second parse mutates nothing, which is
    /// precisely the reassembly differential that fuels tag-splitting and
    /// mXSS.
    fn assert_sanitized_output_is_inert(name: &str, input: &str) {
        let out = sanitize(input);
        for tag in scan_tags(&out) {
            assert_tag_is_inert(&tag);
        }
        assert_eq!(
            sanitize(&out),
            out,
            "{name}: sanitize output is not a fixed point: {out:?}"
        );
    }

    /// CVE-2025-66376 tag-splitting corpus: no live markup may survive
    /// (see [`TAG_SPLITTING_CORPUS`] for the technique and each payload).
    #[test]
    fn tag_splitting_corpus_yields_no_live_markup() {
        for (name, payload) in TAG_SPLITTING_CORPUS {
            assert_sanitized_output_is_inert(name, payload);
        }
    }

    /// The same corpus must render to terminal text without panicking.
    #[test]
    fn tag_splitting_corpus_renders_to_text_without_panicking() {
        for (name, payload) in TAG_SPLITTING_CORPUS {
            render(payload.as_bytes(), 80)
                .unwrap_or_else(|err| panic!("{name}: render failed: {err}"));
        }
    }

    /// Lock the reassembly semantics that turned the Zimbra bug into RCE:
    /// - the verbatim exploit string collapses to inert text with the
    ///   stray `>` escaped, and its fragments (`<svg`, `onload=`, `atob(`,
    ///   …) never appear contiguously — nothing strips the `@import…;`
    ///   runs separating them;
    /// - the hidden-div carrier keeps its fragment separators verbatim;
    /// - comment removal may join text nodes, but the join stays escaped
    ///   text (inert in the html2text terminal pipeline), never markup;
    /// - a `style` element dies whole, `@import` cargo included.
    #[test]
    fn tag_splitting_fragments_stay_broken_and_never_reassemble() {
        let (_, verbatim) = TAG_SPLITTING_CORPUS[0];
        let out = sanitize(verbatim);
        for fragment in ["<svg", "<script", "onload=", "onerror=", "atob(", "eval("] {
            assert!(
                !out.contains(fragment),
                "fragment {fragment:?} reassembled in verbatim exploit output: {out:?}"
            );
        }
        assert!(
            out.contains("tyle&gt;"),
            "stray `>` not escaped in verbatim exploit output: {out:?}"
        );

        let (_, hidden_div) = TAG_SPLITTING_CORPUS[1];
        let out = sanitize(hidden_div);
        assert!(
            out.contains("scr@import FHBCuUYUGEQODuCrzISjiZsOR;ipt"),
            "fragment separator stripped — tag could reassemble: {out:?}"
        );
        assert!(
            out.contains("onlo@import zZhGLNPLxJ;ad=ev"),
            "fragment separator stripped — tag could reassemble: {out:?}"
        );

        assert_eq!(
            sanitize(r#"<p>onlo<!--c-->ad=alert(1)</p>"#),
            r#"<p>onload=alert(1)</p>"#,
            "text-node joins must stay inert escaped text"
        );

        assert_eq!(
            sanitize(r#"<style>@import url(evil);s_@import;_v_g/onload=ev</style>ok"#),
            "ok",
            "style element must die whole with its @import cargo"
        );
    }
}
