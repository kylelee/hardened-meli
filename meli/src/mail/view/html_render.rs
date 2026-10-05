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
//!
//! [`sanitize`] and [`render`] are `pub` (not `pub(crate)`) on purpose: the
//! `cve` regression crate and the integration test suites exercise this
//! exact hardened path — the same functions the mail view calls.

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
///
/// 第二处与 nh3 parity 的**有意偏离**（CVE-2018-0950 / issue #101）：`href`
/// 的 URL 校验前移到 trim 早退分支**之前**，且 [`is_safe_url`] 拒绝 `\`
/// 开头的 UNC / 设备根路径与 `C:\…` 盘符路径。裸 UNC href 没有 WHATWG
/// scheme，会被 `url_relative = PassThrough` 当作惰性相对引用原样保留，
/// html2text 再把它渲染成可外联的脚注链接——正是 Office 在 RTF/OLE 预览
/// 中自动解引用 `\\attacker\share` 的终端等价面。
///
/// 第三处与 nh3 parity 的**有意偏离**（CVE-2024-42009 / issue #115，反清洗
/// mXSS）：保留下来的属性值只要解码后含标签起始 `<`，整个属性丢弃——`href`
/// 也在内，因为带 scheme 的 URL 可以把 `</style><img src=1 onerror=alert(1)>`
/// 塞进 path/query 骗过 [`is_safe_url`]，而字面 `<` 本就不是合法 URL 字符。
/// html5ever 序列化时会把属性值里的 `<`/`>` 转义成 `&lt;`/`&gt;`，但解析树
/// 里的属性值仍是字面标记串；Roundcube `message_body()` 正是把清洗后的字符串
/// 交给第二次解析，让藏在属性值里的标记串复活。丢弃这类属性使 sanitize 输出
/// 不再携带 `<img`/`onerror` 之类的二次解析原料。
///
/// 第四处与 nh3 parity 的**有意偏离**（CVE-2024-42010 / issue #116，CSS 清洗
/// 绕过）：保留下来的属性值只要以 ASCII 大小写不敏感方式含 `@import`、`url(`、
/// `expression(` 任一子串，整个属性丢弃。Roundcube `mod_css_styles` 的缺陷是
/// CSS 指令原料在清洗后仍以活指令形态留在渲染邮件里，攻击者据此用 CSS 侧信道
/// （`@import`/`url(...)` 的资源请求、属性选择器/动画的时间差）外传信息；清洗
/// 顺序应为「先抽取活原料，再谈白名单」。meli 没有 CSS 引擎、html2text 也不
/// 加载远程资源（第二道防线），但 issue 明确断言清洗输出不得含 `@import`/
/// `url(`——下游任何读回属性值再消费的组件都会拿到活指令。`href` 一并丢弃，
/// 与 42009 同判例：合法 `https:` URL 可把 `url(`/`@import` 塞进 path/query，
/// [`is_safe_url`] 单独拦不住；URL 中裸 `url(`/`@import` 属病态形态，丢弃仅损失
/// 脚注可见性。裸 `background` 不禁——它是普通英文词，含 `url(` 的 `background`
/// 值已被本规则覆盖。实体编码形（`&#64;import`）经 html5ever 解码后同样命中。
pub fn sanitize(input: &str) -> String {
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
            // Re-validate every `href` we are about to write back, *before*
            // the no-trim early return below. ammonia validated URL schemes
            // on the original value in `clean_child`, but its `url_relative
            // = PassThrough` policy keeps whatever `Url::parse` reports as
            // `RelativeUrlWithoutBase` — including the Windows path shapes
            // (`\\host\share`, `\Device\…`, `C:\…`) that are live
            // dereference targets once a launcher or footnote consumer
            // touches them (CVE-2018-0950 / issue #101). A WHATWG URL parser
            // only strips C0 controls + ASCII space before scheme detection,
            // so `Url::parse` of a Cf-padded value (e.g.
            // `\u{200B}javascript:alert(1)`) also returns
            // `RelativeUrlWithoutBase`; trimming the prefix would
            // *activate* the hidden scheme into a live `javascript:` href,
            // the exact regression golden case 5 exists to prevent.
            // Validating the trimmed value covers both the untrimmed bypass
            // and the trim-activation case; drop on doubt.
            if attr == "href" && !is_safe_url(trimmed) {
                return None;
            }
            // CVE-2024-42009 / issue #115 (desanitization mXSS): the
            // Roundcube `message_body()` flow sanitized a string and then
            // handed it to a *second parse*, where a markup string hidden
            // in a retained attribute value came back alive. ammonia
            // parses once and html5ever's serializer escapes `<`/`>` back
            // to `&lt;`/`&gt;`, but the **decoded** attribute value is
            // still a literal `<img src=1 onerror=alert(1)>` string that
            // any downstream consumer re-parsing the attribute value
            // receives as parser input — and the sanitized output itself
            // still carries `onerror`/`<img` as text. Drop every retained
            // attribute whose value contains a tag-open `<`, so the output
            // holds no markup raw material for a second parse. This must
            // cover `href` too: a scheme-prefixed URL such as
            // `https://host/</style><img src=1 onerror=alert(1)>` passes
            // [`is_safe_url`] (the `<` is parsed into the path/query), so
            // the scheme re-check alone would let the payload through. A
            // literal `<` is never valid in a URL anyway (it must be
            // percent-encoded).
            if trimmed.contains('<') {
                return None;
            }
            // CVE-2024-42010 / issue #116 (CSS sanitization bypass): a
            // retained value that carries CSS directive raw material is live
            // fuel for the Roundcube `mod_css_styles` failure mode — a CSS
            // side channel that exfiltrates data through `@import`/`url(...)`
            // requests, attribute selectors and animation timing. meli has no
            // CSS engine and html2text never fetches a remote resource (the
            // second line of defense), but the issue asserts the sanitized
            // output must not carry `@import`/`url(` at all, and any
            // downstream consumer that re-reads the value would receive a
            // live directive. Drop the whole attribute when the decoded
            // value contains any of the directive tokens, ASCII
            // case-insensitively. This must cover `href` too, by the same
            // precedent as the `<` guard above: a legal `https:` URL can
            // hide `url(`/`@import` in its path or query and still pass
            // [`is_safe_url`]. Bare `background` is deliberately *not*
            // banned — it is an ordinary English word, and a `background`
            // value that actually carries a directive contains `url(`, which
            // this rule already drops. Entity-encoded forms (`&#64;import`)
            // are decoded by html5ever before reaching the filter, so they
            // are covered as well.
            if contains_ascii_ci(trimmed, "@import")
                || contains_ascii_ci(trimmed, "url(")
                || contains_ascii_ci(trimmed, "expression(")
            {
                return None;
            }
            if trimmed.len() == value.len() {
                return Some(Cow::Borrowed(value));
            }
            // A value made entirely of trimmable chars (e.g. `href="   "`)
            // becomes `href=""` after trim and would render as a phantom
            // footnote by html2text. Drop the attribute instead.
            if trimmed.is_empty() {
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
///
/// The relative-URL arm is **narrower than nh3 parity on purpose**
/// (CVE-2018-0950 / issue #101): a value with no WHATWG scheme can still be
/// a live Windows/SMB dereference target, so bare UNC paths
/// (`\\host\share`), device/root paths (`\Device\…`, `\\.\…`, `\?\…`) and
/// drive-letter paths (`C:\…`) are rejected even though `Url::parse`
/// reports them as `RelativeUrlWithoutBase`. Genuine relative references
/// (`/path`, `#frag`, `//host`) are unaffected, and `C:/…` needs no special
/// case — `Url::parse` treats `c` as a scheme, which the allowlist rejects.
fn is_safe_url(value: &str) -> bool {
    if value.starts_with('\\') || is_windows_drive_path(value) {
        return false;
    }
    match url::Url::parse(value) {
        Ok(u) => matches!(u.scheme(), "http" | "https" | "mailto"),
        Err(url::ParseError::RelativeUrlWithoutBase) => true,
        Err(_) => false,
    }
}

/// Whether `value` is a Windows drive-letter path with a literal backslash
/// separator (`C:\…`): one ASCII letter, a colon, then `\`.
fn is_windows_drive_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\'
}

/// ASCII case-insensitive substring search with **no allocation** (CVE-2024-42010
/// / issue #116). `needle` is always one of the lower-case ASCII directive
/// tokens, so folding both sides with [`u8::eq_ignore_ascii_case`] is exact:
/// bytes with the high bit set (all UTF-8 multibyte continuation/lead bytes)
/// can never fold to an ASCII byte, so a multibyte sequence cannot produce a
/// false positive. An empty needle never matches (and would make
/// [`slice::windows`] panic).
fn contains_ascii_ci(haystack: &str, needle: &str) -> bool {
    let needle = needle.as_bytes();
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
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

/// Maximum number of decoded input bytes [`render`] hands to the
/// `sanitize` → html2text pipeline.
///
/// This is the CWE-400 open-mail availability bound for the regression
/// tracked as CVE-1999-1016 (MS HTML control in IE5 / Outlook Express 5 /
/// Eudora): a hostile HTML mail with giant form fields pinned the renderer
/// at 100% CPU. Form tags themselves are already inert in [`sanitize`]
/// (`input`/`textarea`/`select`/`button`/`form` are not in the allowlist,
/// so the literal trigger dies there), but the decoded HTML that [`render`]
/// accepts had no size ceiling of its own. The blocking view job renders
/// whatever the message contains the moment the mail is opened, so a 100 MiB
/// HTML mail used to burn ~33 s of 100% CPU and ~1 GiB of allocations on
/// that thread (10 MiB of pure nested-table markup measured ~3.3 s release,
/// the worst shape).
///
/// 10 MiB is far above any legitimate HTML mail body (base64 inflates the
/// wire size by ~33%, so this is a ~14 MiB mail) and bounds the worst-case
/// release render to a few seconds.
pub const MAX_HTML_RENDER_INPUT_BYTES: usize = 10 * 1024 * 1024;

/// Terminal-friendly marker appended to the rendered text when the decoded
/// HTML body exceeded [`MAX_HTML_RENDER_INPUT_BYTES`]. User-visible, so it
/// states both that truncation happened and the cap that caused it.
const HTML_INPUT_TRUNCATED_NOTICE: &str =
    "\n[-- HTML body exceeded the 10 MiB render cap and was truncated --]\n";

/// Largest byte index `<= index` that is a UTF-8 character boundary of `s`.
fn floor_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    let mut floor = index;
    while !s.is_char_boundary(floor) {
        floor -= 1;
    }
    floor
}

/// Maximum element nesting depth [`render`] hands to the html2text stage.
///
/// This is the CWE-674 open-mail availability bound of the CVE-1999-1016
/// regression: html2text 0.17.1 tears its `RenderNode` tree (and the parsed
/// DOM) down with **recursive `Drop` glue** — a gdb-verified
/// `RenderTable → Vec<RenderTableRow> → RenderTableCell → Vec<RenderNode> →
/// RenderTable → …` chain costing ~700 B of stack per table-nesting level.
/// Rust cannot catch a stack overflow, so past the threshold the whole meli
/// process aborts the moment the mail is opened. Measured on the default
/// 2 MiB job-thread stack (tokio `spawn_blocking`): a mail of only ~75 KiB
/// (`<table><tr><td>` repeated 5 000 times, no closing tags — the parser
/// auto-nests them the same way as the balanced form) already aborts the
/// process; 2 000 levels survive. [`MAX_HTML_RENDER_INPUT_BYTES`] bounds
/// input *size*, not depth, so the byte cap alone leaves this crash
/// reachable.
///
/// 256 is ~12× below the measured abort threshold and an order of magnitude
/// above any legitimate HTML-mail nesting (generator mails nest ≲30 tables);
/// html2text's own deep-nesting regression corpus covers 1 000 levels.
pub const MAX_HTML_RENDER_NESTING_DEPTH: usize = 256;

/// Terminal-friendly marker appended to the rendered text when the sanitized
/// document exceeded [`MAX_HTML_RENDER_NESTING_DEPTH`] nesting levels.
const HTML_NESTING_TRUNCATED_NOTICE: &str =
    "\n[-- HTML body exceeded the 256-element nesting-depth render cap and was truncated --]\n";

/// Void elements of [`sanitize`]'s allowlist: serialized without closing
/// tags, so they never contribute nesting depth.
const VOID_TAGS: &[&str] = &["br", "hr"];

/// Cap the element nesting depth of *sanitized* markup at
/// [`MAX_HTML_RENDER_NESTING_DEPTH`] by cutting the document at the first
/// start tag that would exceed it.
///
/// The scan runs on ammonia's serialized output, which is canonical: text
/// `<`/`>` are escaped, comments are stripped, and the only tags present are
/// allowlisted ones, so every `<` starts a real tag and quoted attribute
/// values (where html5ever leaves `<`/`>` unescaped) are the sole nesting a
/// lexer must skip. Returns `None` when the document is within the cap (no
/// allocation), or the truncated prefix past which nothing may nest deeper.
fn cap_nesting_depth(sanitized: &str) -> Option<String> {
    let bytes = sanitized.as_bytes();
    let mut depth: usize = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let tag_start = i;
        match bytes.get(i + 1) {
            Some(b'/') => {
                // End tag: html5ever serializes end tags bare (`</name>`),
                // so the next `>` closes them; nesting pops one level.
                match bytes[i + 2..].iter().position(|&b| b == b'>') {
                    Some(pos) => {
                        depth = depth.saturating_sub(1);
                        i += 2 + pos + 1;
                    }
                    // Unterminated end tag: parse it as text and stop —
                    // whatever follows was cut by the serializer already.
                    None => break,
                }
            }
            Some(b) if b.is_ascii_alphabetic() => {
                // Start tag: scan the name, then walk to the closing `>`
                // skipping double-quoted attribute values (the serializer
                // escapes `"` inside values, so a raw `"` toggles quotes).
                let mut j = i + 1;
                while j < bytes.len() && (bytes[j].is_ascii_alphanumeric()) {
                    j += 1;
                }
                let name = &sanitized[i + 1..j];
                let mut k = j;
                let mut in_quotes = false;
                while k < bytes.len() {
                    match bytes[k] {
                        b'"' => in_quotes = !in_quotes,
                        b'>' if !in_quotes => break,
                        _ => {}
                    }
                    k += 1;
                }
                if k >= bytes.len() {
                    // Unterminated start tag: cannot nest further.
                    break;
                }
                if !VOID_TAGS.contains(&name) {
                    depth += 1;
                    if depth > MAX_HTML_RENDER_NESTING_DEPTH {
                        return Some(sanitized[..tag_start].to_owned());
                    }
                }
                i = k + 1;
            }
            // `<!` / `<?` constructions cannot survive `strip_comments`, but
            // skip them defensively to the next `>` without depth changes.
            Some(b'!') | Some(b'?') => match bytes[i + 2..].iter().position(|&b| b == b'>') {
                Some(pos) => i += 2 + pos + 1,
                None => break,
            },
            // A `<` that is not tag-shaped (cannot happen in serialized
            // output): treat as text.
            _ => i += 1,
        }
    }
    None
}

/// Sanitize and render HTML `bytes` to plain text wrapped at `width` display
/// columns.
///
/// Two CVE-1999-1016 availability bounds guard the pipeline, both enforced
/// *before* html2text and both surfaced as a one-line notice appended to the
/// rendered text:
///
/// - the decoded input is truncated to [`MAX_HTML_RENDER_INPUT_BYTES`] before
///   [`sanitize`] runs when it is larger (cut at the largest UTF-8 character
///   boundary at or below the cap). Truncating before the parse — rather than
///   after — means the sanitizer sees the final document, with no parse
///   differential for a truncated tail to smuggle markup through;
/// - the *sanitized* markup is then truncated at
///   [`MAX_HTML_RENDER_NESTING_DEPTH`] nesting levels, bounding the depth of
///   the trees html2text builds and (recursively) drops.
///
/// Input within both caps behaves exactly as before (no notice).
///
/// Never panics: invalid UTF-8 input is replaced lossily, sanitization always
/// returns a `String`, and rendering failures are reported as [`Error`].
pub fn render(bytes: &[u8], width: usize) -> Result<String> {
    let decoded = String::from_utf8_lossy(bytes);
    let (input, byte_truncated) = if decoded.len() > MAX_HTML_RENDER_INPUT_BYTES {
        (
            &decoded[..floor_char_boundary(&decoded, MAX_HTML_RENDER_INPUT_BYTES)],
            true,
        )
    } else {
        (decoded.as_ref(), false)
    };
    let html = sanitize(input);
    let (html, depth_truncated) = match cap_nesting_depth(&html) {
        Some(truncated) => (truncated, true),
        None => (html, false),
    };
    let mut rendered = html2text::config::plain()
        .string_from_read(html.as_bytes(), width)
        .map_err(|err| {
            Error::new("Could not render html to text").set_source(Some(Arc::new(err)))
        })?;
    if byte_truncated {
        rendered.push_str(HTML_INPUT_TRUNCATED_NOTICE);
    }
    if depth_truncated {
        rendered.push_str(HTML_NESTING_TRUNCATED_NOTICE);
    }
    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

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

    /// CVE-2018-0950 / issue #101: bare Windows path forms carry no WHATWG
    /// scheme, so `Url::parse` reports `RelativeUrlWithoutBase` and the
    /// `url_relative = PassThrough` policy preserved them. A `\\host\share`
    /// UNC href (or a `\Device\…` root path) therefore survived `sanitize`
    /// and reached html2text as a live link footnote — the terminal-side
    /// mirror of the Office RTF/OLE remote-content dereference the CVE
    /// describes. The defense is pushed into `sanitize`: every
    /// Windows-path-shaped relative value is dropped, Cf-padded spellings
    /// included (the trim would otherwise activate them).
    #[test]
    fn sanitize_drops_windows_path_hrefs() {
        for (label, href) in [
            ("bare_unc", r"\\attacker.example\share"),
            ("quad_unc", r"\\\\attacker.example\\share\\payload.mht"),
            (
                "root_path",
                r"\Device\HarddiskVolume1\Windows\System32\evil.dll",
            ),
            ("drive_backslash", r"C:\Windows\System32\evil.exe"),
            ("drive_slash", r"C:/Windows/System32/evil.exe"),
            ("cf_padded_unc", "\u{200B}\\\\attacker.example\\share"),
            ("cf_padded_root", "\u{FEFF}\\Device\\HarddiskVolume1"),
            ("cf_padded_drive", "\u{FEFF}C:\\Windows\\System32\\evil.exe"),
        ] {
            let out = sanitize(&format!("<a href=\"{href}\">link text</a>"));
            assert!(
                !out.contains("href"),
                "{label}: Windows-path href survived sanitize: {out:?}"
            );
            assert!(
                out.contains("link text"),
                "{label}: anchor display text must survive: {out:?}"
            );
            assert!(
                out.contains("rel=\"noopener noreferrer\""),
                "{label}: the injected rel must survive: {out:?}"
            );
        }
    }

    /// The Windows-path ban is deliberately narrow: honest scheme URLs and
    /// the non-Windows relative forms the existing corpus doctrine
    /// protects (`/path`, `#frag`, `//host` protocol-relative) must all
    /// keep passing through unchanged.
    #[test]
    fn sanitize_keeps_honest_and_non_windows_relative_hrefs() {
        for (href, expected) in [
            ("http://e.example/a", r#"href="http://e.example/a""#),
            (
                "https://e.example/a?q=1",
                r#"href="https://e.example/a?q=1""#,
            ),
            ("mailto:x@y.z", r#"href="mailto:x@y.z""#),
            ("/path?q=1", r#"href="/path?q=1""#),
            ("#frag", r##"href="#frag""##),
            ("//host.example/x", r#"href="//host.example/x""#),
        ] {
            let out = sanitize(&format!("<a href=\"{href}\">x</a>"));
            assert!(
                out.contains(expected),
                "{href:?}: {expected} must survive sanitize: {out:?}"
            );
        }
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

    /// CVE-2024-42009 / issue #115 (desanitization mXSS): a retained
    /// attribute value that decodes to a literal markup string
    /// (`</style><img src=1 onerror=alert(1)>`) must be dropped whole, so
    /// the serialized output carries no `onerror`/`<img`/`alert(` second-
    /// parse raw material. `href` is covered too: a scheme-prefixed URL can
    /// hide the markup in its path/query and still pass `is_safe_url`. The
    /// guard must stay narrow — a benign value without a raw `<` survives,
    /// and a lone `>` is not a tag-open.
    #[test]
    fn sanitize_drops_attribute_values_carrying_literal_markup() {
        for (label, payload) in [
            (
                "p_title",
                r#"<p title="</style><img src=1 onerror=alert(1)>">x</p>"#,
            ),
            (
                "p_lang",
                r#"<p lang="</style><img src=1 onerror=alert(1)>">x</p>"#,
            ),
            (
                "a_title",
                r#"<a href="https://e.example" title="</style><svg onload=alert(1)>">x</a>"#,
            ),
            (
                "entity_title",
                r#"<p title="&lt;img src=1 onerror=alert(1)&gt;">x</p>"#,
            ),
            (
                "href_path",
                r#"<a href="https://e.example/</style><img src=1 onerror=alert(1)>">x</a>"#,
            ),
        ] {
            let out = sanitize(payload);
            let lower = out.to_ascii_lowercase();
            for needle in ["onerror", "<img", "alert("] {
                assert!(
                    !lower.contains(needle),
                    "{label}: {needle:?} survived the attribute guard: {out:?}"
                );
            }
        }
        // A benign value with no raw `<` survives unchanged.
        assert_eq!(
            sanitize(r#"<p lang="en" title="hello">x</p>"#),
            r#"<p lang="en" title="hello">x</p>"#
        );
        // A lone `>` is not a tag-open; the value survives, escaped.
        assert_eq!(
            sanitize(r#"<p title="a > b">x</p>"#),
            r#"<p title="a &gt; b">x</p>"#
        );
    }

    /// CVE-2024-42010 / issue #116 (CSS side-channel smuggling): a retained
    /// attribute value that carries CSS directive raw material (`@import`,
    /// `url(`, `expression(`) is live fuel for any downstream consumer that
    /// re-reads the value into a CSS engine (the Roundcube `mod_css_styles`
    /// failure mode). meli has no CSS engine and html2text never fetches a
    /// remote resource, but the issue asserts the sanitized output must not
    /// carry `@import`/`url(` at all, so any kept attribute whose value
    /// contains one of those tokens is dropped whole. `href` is covered too
    /// — a legal `https:` URL can hide `url(`/`@import` in its path/query
    /// and still pass `is_safe_url`. The guard stays narrow: plain
    /// parenthesised prose, a lone `>`, and ordinary https/mailto links are
    /// not directive raw material and survive.
    #[test]
    fn sanitize_drops_attribute_values_carrying_css_directive_tokens() {
        // a[title] carrying the issue's `@import url(...)` shape is dropped
        // whole; the anchor text and the injected `rel` survive.
        assert_eq!(
            sanitize(r#"<a title='@import url("https://e.example/leak")'>x</a>"#),
            r#"<a rel="noopener noreferrer">x</a>"#
        );
        // href query carrying `url(` is dropped: `is_safe_url` alone lets a
        // scheme-prefixed URL through no matter what its path/query holds.
        assert_eq!(
            sanitize(r#"<a href="https://e.example/x.css?q=url(1)">x</a>"#),
            r#"<a rel="noopener noreferrer">x</a>"#
        );

        // Every directive token, every carrier, case-insensitively and
        // after entity decoding by html5ever.
        for (label, payload, expected) in [
            (
                "title_import",
                r#"<p title="@import url(x)">x</p>"#,
                "<p>x</p>",
            ),
            (
                "title_url",
                r#"<p title="url(https://e.example/x)">x</p>"#,
                "<p>x</p>",
            ),
            (
                "title_expression",
                r#"<p title="expression(alert(1))">x</p>"#,
                "<p>x</p>",
            ),
            (
                "title_entity_import",
                r#"<p title="&#64;import url(&#104;ttps://e.example/x)">x</p>"#,
                "<p>x</p>",
            ),
            (
                "title_mixed_case",
                r#"<p title="@IMPORT URL(x)">x</p>"#,
                "<p>x</p>",
            ),
            (
                "href_path_import",
                r#"<a href="https://e.example/@import">x</a>"#,
                r#"<a rel="noopener noreferrer">x</a>"#,
            ),
            (
                "href_query_expression",
                r#"<a href="https://e.example/?q=expression(1)">x</a>"#,
                r#"<a rel="noopener noreferrer">x</a>"#,
            ),
        ] {
            assert_eq!(sanitize(payload), expected, "{label}");
        }

        // Benign controls survive unchanged: a normal title, plain
        // parenthesised prose, and a lone `>` are not CSS directive raw
        // material.
        assert_eq!(
            sanitize(r#"<p title="hello (world)">x</p>"#),
            r#"<p title="hello (world)">x</p>"#
        );
        assert_eq!(
            sanitize(r#"<p title="a > b">x</p>"#),
            r#"<p title="a &gt; b">x</p>"#
        );
        let benign = sanitize(
            r#"<a href="https://e.example" title="docs">https</a> <a href="mailto:x@e.example">mail</a>"#,
        );
        assert!(
            benign.contains(r#"href="https://e.example""#),
            "benign https href dropped: {benign:?}"
        );
        assert!(
            benign.contains(r#"title="docs""#),
            "benign title dropped: {benign:?}"
        );
        assert!(
            benign.contains(r#"href="mailto:x@e.example""#),
            "benign mailto href dropped: {benign:?}"
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

    /// Alias for the cap the regression tests below build inputs against.
    const CAP: usize = MAX_HTML_RENDER_INPUT_BYTES;

    /// Exactly-at-cap input renders in full with no notice: the cap is a
    /// strict `>` bound, not an off-by-one. A whitespace-heavy ASCII body
    /// keeps this at-cap assertion cheap; the content proves the input was
    /// actually processed rather than short-circuited.
    #[test]
    fn render_at_input_cap_has_no_truncation_notice() {
        let mut html = String::with_capacity(CAP);
        html.push_str("<p>");
        html.push_str("at-cap");
        while html.len() < CAP - "</p>".len() {
            html.push(' ');
        }
        html.push_str("</p>");
        assert_eq!(
            html.len(),
            CAP,
            "test input must land byte-exactly on the cap"
        );
        let text = render(html.as_bytes(), 80).expect("at-cap html must render");
        assert!(
            !text.contains("truncated") && !text.contains("render cap"),
            "at-cap input must not be reported as truncated: {text:?}"
        );
        assert!(
            text.contains("at-cap"),
            "at-cap input content must render: {text:?}"
        );
    }

    /// Over-cap ASCII: the prefix up to the cap is sanitized and rendered,
    /// then the notice is appended exactly once, and the output stays bounded
    /// by the cap plus the notice.
    #[test]
    fn render_over_input_cap_truncates_ascii_with_single_notice() {
        let mut html = String::with_capacity(CAP + 8);
        html.push_str("<p>");
        html.push_str(&"A".repeat(CAP));
        html.push_str("</p>");
        assert!(html.len() > CAP, "precondition: input must exceed the cap");
        let text = render(html.as_bytes(), 80).expect("over-cap html must render");
        assert_eq!(
            text.matches(HTML_INPUT_TRUNCATED_NOTICE).count(),
            1,
            "the truncation notice must be appended exactly once"
        );
        assert!(
            text.len() <= CAP + CAP / 80 + HTML_INPUT_TRUNCATED_NOTICE.len() + 8,
            "truncated render must stay bounded by the cap, got {} bytes",
            text.len()
        );
        assert!(
            text.contains('A'),
            "the prefix content must survive the cut: {text:?}"
        );
    }

    /// Over-cap CJK: the cap lands inside a 3-byte character, so the cut must
    /// back up to a UTF-8 character boundary before `sanitize`; no panic,
    /// notice present, output valid UTF-8 by construction.
    #[test]
    fn render_over_input_cap_cuts_on_cjk_char_boundary() {
        // `CAP - 2` ASCII bytes, then `攻` spans bytes `CAP-2..CAP+1`, putting
        // the cut byte `CAP` inside the character.
        let mut html = String::with_capacity(CAP + 64);
        html.push_str(&"a".repeat(CAP - 2));
        html.push_str(&"攻".repeat(16));
        assert!(html.len() > CAP, "precondition: input must exceed the cap");
        assert!(
            !html.is_char_boundary(CAP),
            "precondition: the cap must land inside a multibyte character"
        );
        let text = render(html.as_bytes(), 80).expect("over-cap CJK must render");
        assert_eq!(
            text.matches(HTML_INPUT_TRUNCATED_NOTICE).count(),
            1,
            "CJK truncation must still append the notice"
        );
        assert!(
            !text.contains('攻'),
            "the cut must land before the straddling character: {text:?}"
        );
        assert!(
            std::str::from_utf8(text.as_bytes()).is_ok(),
            "the truncated render must be valid UTF-8"
        );
    }

    /// Over-cap invalid UTF-8: the lossy replacement and the cap compose
    /// without a panic; the cut still lands on a character boundary and the
    /// notice is present.
    #[test]
    fn render_over_input_cap_with_invalid_utf8_does_not_panic() {
        let mut bytes = Vec::with_capacity(CAP + 4);
        bytes.extend(std::iter::repeat_n(b'A', CAP - 1));
        bytes.extend_from_slice(&[0xff, 0xfe, 0xff, 0xff]);
        assert!(bytes.len() > CAP, "precondition: input must exceed the cap");
        let text = render(&bytes, 80).expect("over-cap invalid UTF-8 must render");
        assert_eq!(
            text.matches(HTML_INPUT_TRUNCATED_NOTICE).count(),
            1,
            "notice must survive the lossy decode + cap composition"
        );
        assert!(
            std::str::from_utf8(text.as_bytes()).is_ok(),
            "lossy decode plus boundary cut must produce valid UTF-8"
        );
    }

    /// Truncation cannot smuggle markup: cutting *before* `sanitize` means the
    /// sanitizer parses the final prefix. An unterminated `<script>` head
    /// swallows the whole truncated rest (ammonia removes raw-text content
    /// entirely), and a giant `<input value=…>` void element loses its
    /// attribute with the element.
    #[test]
    fn render_truncation_cannot_smuggle_markup_past_sanitize() {
        // Unterminated `<script>` in the head: the truncated prefix is one
        // raw-text element to EOF, so `clean_content_tags` drops it whole.
        let script = format!("<script>{}", "A".repeat(CAP));
        assert!(
            script.len() > CAP,
            "precondition: input must exceed the cap"
        );
        let text = render(script.as_bytes(), 80).expect("script payload must render");
        assert_eq!(
            text.matches(HTML_INPUT_TRUNCATED_NOTICE).count(),
            1,
            "script payload must append the notice"
        );
        assert!(
            !text.contains('A') && !text.contains("script"),
            "unterminated script content must be removed entirely: {text:?}"
        );

        // Giant form field: `input` is not in the allowlist and carries no
        // child text, so the whole element (with its `value=` attribute)
        // disappears.
        let input = format!("<input type=\"text\" value=\"{}\">", "A".repeat(CAP));
        assert!(input.len() > CAP, "precondition: input must exceed the cap");
        let text = render(input.as_bytes(), 80).expect("input payload must render");
        assert_eq!(
            text.matches(HTML_INPUT_TRUNCATED_NOTICE).count(),
            1,
            "input payload must append the notice"
        );
        let lower = text.to_ascii_lowercase();
        assert!(
            !lower.contains("<input") && !lower.contains("value=") && !lower.contains("type="),
            "no input/value= marker may survive truncation: {text:?}"
        );
    }

    /// The one heavy worst-shape test: a >12 MiB corpus dominated by
    /// `<table>` markup at the maximum legal nesting depth
    /// ([`MAX_HTML_RENDER_NESTING_DEPTH`]) wrapping a bulk of shallow sibling
    /// tables must render `Ok` within a generous watchdog, byte-truncated to
    /// the cap with the notice and a bounded output — and the depth cap must
    /// *not* fire on it (depth stays within the cap). This is the regression
    /// lock for the unbounded open-mail render cost (the CVE-1999-1016
    /// availability face). Every other cap test above uses a cheap shape on
    /// purpose.
    #[test]
    fn render_over_cap_nested_tables_completes_within_watchdog() {
        const WATCHDOG: Duration = Duration::from_secs(180);
        const DEPTH: usize = MAX_HTML_RENDER_NESTING_DEPTH;
        let mut html = String::with_capacity(12 * 1024 * 1024 + DEPTH * 18);
        for _ in 0..DEPTH {
            html.push_str("<table><tr><td>");
        }
        while html.len() < 12 * 1024 * 1024 {
            html.push_str("<table><tr><td></td></tr></table>");
        }
        for _ in 0..DEPTH {
            html.push_str("</td></tr></table>");
        }
        assert!(html.len() > CAP, "precondition: input must exceed the cap");
        let started = Instant::now();
        let text = render(html.as_bytes(), 80).expect("nested-table markup must render");
        let elapsed = started.elapsed();
        assert!(
            elapsed < WATCHDOG,
            "nested-table render took {elapsed:?}, watchdog {WATCHDOG:?}"
        );
        assert_eq!(
            text.matches(HTML_INPUT_TRUNCATED_NOTICE).count(),
            1,
            "over-cap nested tables must be truncated with the notice"
        );
        assert!(
            text.len() <= CAP,
            "nested-table output must stay bounded, got {} bytes",
            text.len()
        );
    }
    // ------------------------------------------------------------------
    // Nesting-depth cap (CWE-674 face of CVE-1999-1016)
    // ------------------------------------------------------------------

    /// The verbatim process killer, now bounded: `<table><tr><td>` repeated
    /// with no closing tags nests the same way the balanced form does (the
    /// parser auto-nests each table inside the open cell), and html2text
    /// drops its render tree with recursive `Drop` glue — ~700 B of stack
    /// per level. At the default 2 MiB job-thread stack 5 000 levels of this
    /// 75 KiB markup used to abort the whole meli process on mail open; the
    /// depth cap must cut it at [`MAX_HTML_RENDER_NESTING_DEPTH`] levels and
    /// render the prefix with the notice instead.
    #[test]
    fn render_deep_open_only_tables_do_not_abort() {
        for levels in [5_000usize, 30_000, 699_051] {
            let html = "<table><tr><td>".repeat(levels);
            let text = render(html.as_bytes(), 80).unwrap_or_else(|err| panic!("{levels}: {err}"));
            assert_eq!(
                text.matches(HTML_NESTING_TRUNCATED_NOTICE).count(),
                1,
                "{levels} open-only levels must be depth-truncated with the notice"
            );
            // The prefix that survives is exactly the capped nesting: tiny.
            assert!(
                text.len() < 4 * 1024,
                "{levels}: capped prefix must render tiny, got {} bytes",
                text.len()
            );
        }
    }

    /// The depth cap boundary: exactly [`MAX_HTML_RENDER_NESTING_DEPTH`]
    /// nested elements render untouched (no notice), one more is cut. The
    /// corpus nests `<b>` — one start tag per level, nothing synthesized and
    /// width-neutral (a `<blockquote>` chain consumes ~2 columns per level
    /// and legitimately fails `TooNarrow` long before 256), because a table
    /// unit (`<table><tr><td>`, three counted tags) may gain an implied
    /// `tbody` in html2text's own parse, which would blur the literal-tag
    /// boundary.
    #[test]
    fn render_nesting_depth_boundary_is_inclusive() {
        // Nest from the inside out: exactly-cap and over-cap documents.
        let mut at_cap = String::from("DEPTH-MARKER");
        for _ in 0..MAX_HTML_RENDER_NESTING_DEPTH {
            at_cap = format!("<b>{at_cap}</b>");
        }
        let text = render(at_cap.as_bytes(), 80).expect("at-depth-cap html must render");
        assert!(
            !text.contains("nesting-depth"),
            "depth exactly at the cap must not be truncated: {text:?}"
        );
        assert!(
            text.contains("DEPTH-MARKER"),
            "at-cap nesting must render its innermost text"
        );

        let over = format!("<b>{at_cap}</b>");
        let text = render(over.as_bytes(), 80).expect("over-depth-cap html must render");
        assert_eq!(
            text.matches(HTML_NESTING_TRUNCATED_NOTICE).count(),
            1,
            "one level past the cap must be truncated with the notice"
        );
        assert!(
            !text.contains("DEPTH-MARKER"),
            "the cut precedes the innermost text of the over-cap document"
        );
    }

    /// The depth scan counts only real element nesting: void elements
    /// (`<br>`, `<hr>` — serialized bare) never nest, and quoted attribute
    /// values (where html5ever leaves `>` and fake closers unescaped) are
    /// skipped by the scanner, so neither can push a shallow document over
    /// the cap or shield a deep one from it.
    #[test]
    fn render_nesting_cap_ignores_void_elements_and_quoted_closers() {
        // Thousands of void elements at shallow depth: no truncation.
        let mut html = String::from("<p>");
        for i in 0..10_000 {
            html.push_str(&format!("line {i}<br>"));
        }
        html.push_str("<hr></p>");
        let text = render(html.as_bytes(), 80).expect("void-heavy html must render");
        assert!(
            !text.contains("nesting-depth"),
            "void elements must not count as nesting: {text:?}"
        );
        assert!(
            text.contains("line 9999"),
            "void-heavy content must render whole: {} bytes",
            text.len()
        );

        // Fake closers inside quoted attribute values must not pop depth:
        // a document that is genuinely over the cap cannot be shielded by
        // embedding `</table>` strings in `title` attributes.
        let mut shielded = String::new();
        for _ in 0..(MAX_HTML_RENDER_NESTING_DEPTH + 4) {
            shielded.push_str("<table><tr><td>");
        }
        shielded.push_str(r#"<a title="</table></table></table>">z</a>"#);
        let text = render(shielded.as_bytes(), 80).expect("shielded html must render");
        assert_eq!(
            text.matches(HTML_NESTING_TRUNCATED_NOTICE).count(),
            1,
            "quoted fake closers must not shield real nesting from the cap"
        );

        // Conversely, `>` inside a quoted value at *shallow* depth must not
        // confuse the scanner into truncation.
        let shallow = r#"<p><a title="a > b </table> c">ok</a></p>"#;
        let text = render(shallow.as_bytes(), 80).expect("quoted-gt html must render");
        assert_eq!(text, "ok\n", "quoted `>` must not disturb rendering");
        assert!(
            !text.contains("nesting-depth"),
            "quoted `>` must not trip the depth cap"
        );
    }

    /// Both caps compose on one hostile document: an over-10 MiB body whose
    /// sanitized form also nests past the depth cap is cut by both, each
    /// notice appears exactly once, and the result stays bounded.
    #[test]
    fn render_both_caps_compose_on_one_document() {
        let mut html = String::with_capacity(CAP + 4096);
        for _ in 0..(MAX_HTML_RENDER_NESTING_DEPTH + 8) {
            html.push_str("<table><tr><td>");
        }
        while html.len() <= CAP {
            html.push_str("<table><tr><td></td></tr></table>");
        }
        assert!(html.len() > CAP, "precondition: over the byte cap");
        let text = render(html.as_bytes(), 80).expect("double-cap html must render");
        assert_eq!(
            text.matches(HTML_INPUT_TRUNCATED_NOTICE).count(),
            1,
            "byte-cap notice must appear exactly once"
        );
        assert_eq!(
            text.matches(HTML_NESTING_TRUNCATED_NOTICE).count(),
            1,
            "depth-cap notice must appear exactly once"
        );
        assert!(
            text.len() < 4 * 1024,
            "depth cut bounds the rendered prefix, got {} bytes",
            text.len()
        );
    }
}
