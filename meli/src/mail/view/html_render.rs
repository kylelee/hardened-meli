/*
 * meli
 *
 * Copyright 2026 Manos Pitsidianakis
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
    ammonia::Builder::default()
        .tags(tags)
        .tag_attributes(tag_attributes)
        .url_schemes(["http", "https", "mailto"].into())
        .strip_comments(true)
        .clean(input)
        .to_string()
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
}
