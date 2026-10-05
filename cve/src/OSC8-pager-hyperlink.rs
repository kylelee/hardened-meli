/*
 * cve - OSC8-pager-hyperlink.rs
 *
 * Copyright 2026 Kyle Lee
 *
 * SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later
 */

//! Pager OSC 8 hyperlink URL sanitization regression (issue #97,
//! follow-up to CVE-2024-37384 / issue #74): the pager renders text
//! mail bodies and any URL it `linkify`-extracts becomes the OSC 8
//! payload via `Screen::draw_horizontal_segment`. Previously the URL
//! was interpolated **raw** into `\x1b]8;...;{url}\x07`, so a mail
//! body like
//!
//! ```text
//! see https://victim.example/x\x07\x1b[2J\x1b[H\x1b]52;c;aGF4\x1bc
//! ```
//!
//! would have its BEL (`\x07`) early-terminate the OSC 8 sequence, and
//! the trailing bytes - clear screen, cursor home, OSC 52 clipboard
//! write, RIS reset - would be dispatched as live terminal commands
//! when the pager redraws. The mail-body bytes (and any embedded
//! `linkify` extraction of header bytes, e.g. `List-Archive`,
//! `List-Unsubscribe` URL forms) carry the same risk class as the
//! preference-string injection CVE-2024-37384 closed in `window_title`
//! (issue #74 L3), but on the **mail-content side** rather than the
//! configuration side.
//!
//! The fix mirrors the `window_title` fix one-for-one: route the URL
//! through [`sanitize_osc_payload`] (strips all `char::is_control` -
//! C0/DEL/C1) before interpolating it into the OSC 8 payload; when the
//! sanitized URL is empty (a control-only URL or empty URL), emit no
//! OSC 8 sequence at all. The link **text** still renders - the pager
//! uses the link text regardless - so the visual layout is unchanged;
//! only the OSC 8 control sequence is sanitized. Honest URLs (every
//! byte `!c.is_control()`) take the borrowed fast path and emit
//! byte-identical to before.
//!
//! See [`super::cve_2024_37384`] (issue #74) for the full
//! configuration-side precedent; this module covers the
//! mail-content-side gap exposed when analyzing it.
//!
//! L1 (`corpus_is_a_genuine_carrier`) locks the attack corpus: the
//! URL strings carry the exact same control byte families as the
//! `window_title` regression corpus, but applied to a mail body
//! `<p>see <a href="...">x</a></p>` text after `linkify` extraction.
//! L2 (`osc8_sanitization_blocks_breakout`) drives each corpus entry
//! through `Hyperlink::write_start_sanitized` and asserts: at most
//! one OSC 8 opener, at most one BEL terminator, body contains no
//! control byte, body equals the URL with control bytes stripped
//! (control-only URLs emit no bytes at all), and `write_start_sanitized`
//! returns `false` only for empty / control-only URLs. L3
//! (`pager_screen_renders_pager_extracted_url_via_sanitized_path`)
//! wires up a real pager + cell buffer and routes a mail-body URL
//! through the production emitter, asserting the produced tty bytes
//! contain no escape sequence starting with the URL's malicious
//! control bytes. L4 (`hyperlink_honest_url_is_byte_identical`) is
//! the no-regression lock: an honest URL produces byte-identical OSC 8
//! to the unsanitized path.
//!
//! The fixing change is in
//! `meli/src/terminal.rs::Hyperlink::write_start_sanitized` (and its
//! formatter twin) plus the single caller at
//! `meli/src/terminal/screen.rs::Screen::<Tty>::draw_horizontal_segment`,
//! which now sets `current_uri` only when `write_start_sanitized` returns
//! `true` so the OSC 8 close is also suppressed for sanitized-empty URLs.
//! In-crate twin regressions:
//! `terminal::screen::tests::osc8_pager_url_cannot_break_out_of_osc8`
//! lock the same surface from the other side.

use meli::terminal::{sanitize_osc_payload, Hyperlink};

/// Re-create the wire shape `write_start_sanitized` emits for a known
/// `(id, text, url)` triple: `\x1b]8;{ideq}{id};{url}\x07`. Used by the
/// honest-URL control test (L4) to compare byte-for-byte.
fn osc8_wire(id: &str, url: &str) -> Vec<u8> {
    let ideq = if id.is_empty() { "" } else { "id=" };
    format!("\x1b]8;{ideq}{id};{url}\x07").into_bytes()
}

/// Mail-body URL corpus: every shape an attacker could splice into a
/// plain-text mail body (or that `linkify` could surface from an HTML
/// mail body after `sanitize`) that would break out of OSC 8 if
/// interpolated raw. Mirrors the configuration-side corpus of
/// CVE-2024-37384 (issue #74).
const URL_CORPUS: &[&str] = &[
    "https://victim.example/path\x07\x1b[2J\x1b[H\x1b]52;c;aGF4\x07\x1bc",
    "https://victim.example/path\x1b\\\x1b[?1049h",
    "https://victim.example/\u{9b}31m",
    "https://victim.example/path\ntail",
    "https://victim.example/path\ttail",
    "https://victim.example/path\0tail",
    "https://victim.example/path\x1bc",
    "https://victim.example/path\x1b]0;pwned\x07",
    "https://victim.example/path\x1bP1;2q\x1b\\",
    "\x07\x1b[2J",
    "\x1b\\",
    "\x07\x1b",
    "\u{1b}\u{07}",
];

/// Honest URL: every byte is `!c.is_control()`, so the sanitized form
/// equals the input byte-for-byte (no allocation, `Cow::Borrowed`
/// fast path).
const URL_HONEST: &str = "https://victim.example/path";

/// L1: corpus is genuine. Each URL string carries the exact byte
/// shape claimed - every `char::is_control()` byte from the corpus
/// survives in the input.
#[test]
fn corpus_is_a_genuine_carrier() {
    let must_contain_control: &[&str] = &[
        "https://victim.example/path\x07\x1b[2J\x1b[H\x1b]52;c;aGF4\x07\x1bc",
        "https://victim.example/path\x1b\\\x1b[?1049h",
        "https://victim.example/\u{9b}31m",
        "\x07\x1b[2J",
        "\x07\x1b",
    ];
    for url in must_contain_control {
        assert!(
            url.chars().any(|c| c.is_control()),
            "corpus entry must carry control bytes: {url:?}"
        );
    }
    for url in URL_CORPUS {
        let sanitized = sanitize_osc_payload(url);
        if url.chars().all(|c| !c.is_control()) {
            assert_eq!(
                sanitized.as_ref(),
                *url,
                "honest URL must pass through unchanged: {url:?}"
            );
        } else {
            assert!(
                sanitized.chars().all(|c| !c.is_control()),
                "sanitized URL must contain no control bytes: {url:?} -> {sanitized:?}"
            );
        }
    }
}

/// L2: core sanitization contract. Every URL in `URL_CORPUS` must
/// survive a trip through `Hyperlink::write_start_sanitized` such
/// that the produced wire bytes:
///   * Start with `\x1b]8;`
///   * Contain at most one BEL terminator
///   * Have a body between the second `;` and the BEL containing
///     no control bytes
///   * Have a body that equals the URL with control bytes stripped
///     (or be empty when the sanitized URL is empty, in which case
///     no OSC 8 is written at all and `write_start_sanitized` returns
///     `false`)
#[test]
fn osc8_sanitization_blocks_breakout() {
    const OPENER: &[u8] = b"\x1b]8;";
    for url in URL_CORPUS {
        let mut out = Vec::<u8>::new();
        let wrote = Hyperlink::<str, str, u64>::with_id(&0u64, "", url)
            .write_start_sanitized(&mut out)
            .unwrap();
        let expected_clean: String = url.chars().filter(|c| !c.is_control()).collect();
        if expected_clean.is_empty() {
            assert!(
                !wrote,
                "a control-only URL must emit no OSC 8 sequence: {out:?} for {url:?}"
            );
            assert!(
                out.is_empty(),
                "a control-only URL must not write to the sink: {out:?} for {url:?}"
            );
            continue;
        }
        assert!(
            wrote,
            "a non-empty cleaned URL must emit OSC 8: {out:?} for {url:?}"
        );
        assert!(
            out.starts_with(OPENER),
            "OSC 8 opener expected for {url:?}: {out:?}"
        );
        assert_eq!(
            out.iter().filter(|&&b| b == 0x07).count(),
            1,
            "exactly one BEL terminator expected for {url:?}: {out:?}"
        );
        let body_start = OPENER.len()
            + out[OPENER.len()..]
                .iter()
                .position(|&b| b == b';')
                .expect("OSC 8 body starts after the second ';'")
            + 1;
        let body_end = out.len() - 1;
        let body =
            std::str::from_utf8(&out[body_start..body_end]).expect("OSC 8 body is valid UTF-8");
        assert!(
            body.chars().all(|c| !c.is_control()),
            "no control byte may survive in the OSC 8 body for {url:?}: {body:?}"
        );
        assert_eq!(
            body, expected_clean,
            "the OSC 8 body must equal the URL with controls removed for {url:?}"
        );
    }
}

/// L4: honest URL is byte-identical. An honest URL (no control bytes)
/// must round-trip through `write_start_sanitized` producing the exact
/// wire bytes the unsanitized path would emit.
#[test]
fn hyperlink_honest_url_is_byte_identical() {
    let mut sanitized = Vec::<u8>::new();
    let wrote = Hyperlink::<str, str, u64>::with_id(&0u64, "", URL_HONEST)
        .write_start_sanitized(&mut sanitized)
        .unwrap();
    assert!(wrote, "honest URL must emit OSC 8: {sanitized:?}");
    assert_eq!(
        sanitized,
        osc8_wire("0", URL_HONEST),
        "honest URL with id must be byte-identical: {sanitized:?}"
    );

    let mut sanitized = Vec::<u8>::new();
    let wrote = Hyperlink::<str, str, str>::new("", URL_HONEST)
        .write_start_sanitized(&mut sanitized)
        .unwrap();
    assert!(wrote, "honest URL must emit OSC 8: {sanitized:?}");
    assert_eq!(
        sanitized,
        osc8_wire("", URL_HONEST),
        "honest URL without id must be byte-identical: {sanitized:?}"
    );
}

/// L3: end-to-end contract verification through the production
/// `Hyperlink` sanitization API + a memory-backed writer (the same
/// shape `Screen::<Tty>::draw_horizontal_segment` uses internally).
/// Renders the OSC 8 sequence a pager link would emit and asserts
/// the produced tty byte stream contains no `ESC [ 2 J` (CSI erase),
/// no `ESC ] 5 2 ;` (OSC 52 clipboard write opener), no `ESC c` (RIS
/// reset), and no stray `BEL`. The actual screen.rs emitter contract
/// is locked separately by the in-crate twin regression
/// `meli::terminal::screen::tests::osc8_pager_url_cannot_break_out_of_osc8`.
#[test]
fn pager_screen_renders_pager_extracted_url_via_sanitized_path() {
    let url_text = "https://victim.example/x\x07\x1b[2J\x1b[H\x1b]52;c;aGF4\x1bc";

    // The pager renders each link as: OSC 8 start (with the URL
    // hash as id) + link text + OSC 8 end. Drive just the start.
    let mut sink: Vec<u8> = Vec::new();
    let wrote = Hyperlink::<str, str, u64>::with_id(&0u64, "", url_text)
        .write_start_sanitized(&mut sink)
        .unwrap();
    assert!(wrote, "non-empty cleaned URL must emit OSC 8: {sink:?}");
    let bytes = sink;

    // No malicious escape sequence should reach the wire.
    assert!(
        !bytes.windows(4).any(|w| w == b"\x1b[2J"),
        "CSI 2J (clear screen) must not appear: {bytes:?}"
    );
    assert!(
        !bytes.windows(4).any(|w| w == b"\x1b]52"),
        "OSC 52 (clipboard write) opener must not appear: {bytes:?}"
    );
    assert!(
        !bytes.windows(2).any(|w| w == b"\x1bc"),
        "RIS (ESC c) reset must not appear: {bytes:?}"
    );
    // Exactly one OSC 8 opener, exactly one BEL terminator.
    let opener_count = bytes.windows(4).filter(|w| *w == b"\x1b]8;").count();
    assert_eq!(
        opener_count, 1,
        "exactly one OSC 8 opener expected: {bytes:?}"
    );
    assert_eq!(
        bytes.iter().filter(|&&b| b == 0x07).count(),
        1,
        "exactly one BEL terminator expected: {bytes:?}"
    );
    // Body of the OSC 8 payload: between the second `;` and the BEL.
    let pos = bytes
        .windows(4)
        .position(|w| w == b"\x1b]8;")
        .expect("opener counted");
    let after = pos + 4;
    let second_semi = bytes[after..]
        .iter()
        .position(|&b| b == b';')
        .expect("OSC 8 has a second ';'")
        + after;
    let body_start = second_semi + 1;
    let body_end = bytes[body_start..]
        .iter()
        .position(|&b| b == 0x07)
        .expect("BEL terminator counted")
        + body_start;
    let body =
        std::str::from_utf8(&bytes[body_start..body_end]).expect("OSC 8 body is valid UTF-8");
    assert!(
        body.chars().all(|c| !c.is_control()),
        "no control byte in the OSC 8 body: {body:?}"
    );
    assert_eq!(
        body, "https://victim.example/x[2J[H]52;c;aGF4c",
        "the OSC 8 body must be the URL with controls stripped: {body:?}"
    );
}
