//
// meli
//
// Copyright 2025 meli security audit (C4 regression)
//
// This file is part of meli.
//
// meli is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// meli is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with meli. If not, see <http://www.gnu.org/licenses/>.
//
// SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later

//! C4 regression: unbounded MIME multipart nesting depth vs the stack
//! (CWE-674).
//!
//! `AttachmentBuilder`'s multipart splitting recursed once per nesting
//! level with no bound, so a small crafted message walked the stack
//! pointer into its guard page and aborted the process with SIGABRT.
//! `catch_unwind` cannot intercept that; both crash vectors (`meli view`
//! on the main thread, and the sqlite3 reindex job going through
//! `Envelope::body_bytes`) funnel into the same builder recursion, which
//! is now capped: past the cap the remaining subtree is kept as a single
//! `application/octet-stream` leaf attachment.
//!
//! The deep-nesting case runs on a spawned 2 MiB-stack thread, the
//! strictest stack a `melib` caller uses. Pre-fix overflow thresholds
//! measured on that stack were roughly 375 levels in debug builds and
//! 1900 in release builds; 16000 levels must now parse to completion.

use melib::email::{
    attachment_types::{ContentType, Text},
    Attachment, Envelope,
};

/// Number of `multipart/*` levels in a parsed attachment tree.
fn multipart_depth(a: &Attachment) -> usize {
    if let ContentType::Multipart { parts, .. } = a.content_type() {
        1 + parts.iter().map(multipart_depth).max().unwrap_or(0)
    } else {
        0
    }
}

/// Nested multipart mail shaped like the PoC walk harness: the envelope
/// itself declares `multipart/mixed` (so `Envelope::from_bytes` also runs
/// the `check_if_has_attachments_quick` scan over the raw body) and each
/// level wraps the next one with a distinct boundary, with a text/plain
/// part at the core.
fn nested_multipart(depth: usize) -> Vec<u8> {
    let mut m = String::from(
        "From: a@b.example\r\nSubject: nest\r\nContent-Type: multipart/mixed; \
         boundary=\"B000000\"\r\n\r\n",
    );
    for i in 0..depth {
        m.push_str(&format!(
            "--B{i:06}\r\nContent-Type: multipart/mixed; boundary=\"B{:06}\"\r\n\r\n",
            i + 1
        ));
    }
    m.push_str(&format!(
        "--B{depth:06}\r\nContent-Type: text/plain\r\n\r\nhello"
    ));
    for i in (1..=depth).rev() {
        m.push_str(&format!("\r\n--B{i:06}--\r\n"));
    }
    m.push_str("\r\n--B000000--\r\n");
    m.into_bytes()
}

#[test]
fn c4_nesting_16k_levels_does_not_overflow() {
    const DEPTH: usize = 16_000;
    const STACK: usize = 2 * 1024 * 1024;
    let mail = nested_multipart(DEPTH);
    let worker = std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || {
            let env = Envelope::from_bytes(&mail, None).expect("envelope should parse");
            assert_eq!(env.subject().as_ref(), "nest");
            let body = env.body_bytes(&mail);
            let mut node = &body;
            let mut levels = 0usize;
            while let ContentType::Multipart { parts, .. } = node.content_type() {
                assert!(!parts.is_empty(), "capped tree must not have empty levels");
                levels += 1;
                node = &parts[0];
            }
            // The chain past the cap must have collapsed into exactly one
            // opaque leaf which still carries the remaining bytes, not a
            // partially-parsed multipart.
            assert!(matches!(
                node.content_type(),
                ContentType::OctetStream { .. }
            ));
            assert!(
                node.raw().windows(5).any(|w| w == b"hello"),
                "fallback leaf must contain the remaining subtree bytes"
            );
            (multipart_depth(&body), levels)
        })
        .expect("spawn worker thread");
    let (tree_depth, levels) = worker.join().expect("no stack overflow or panic");
    assert_eq!(levels, 100, "descent must stop at the depth cap");
    assert_eq!(tree_depth, 100, "whole tree must stop at the depth cap");
}

#[test]
fn c4_shallow_nesting_unaffected() {
    // Legitimate nesting (real mails stay under ~10 levels) must parse to
    // full depth with the core text intact. `nested_multipart(d)` has
    // `d + 1` multipart levels, all preserved while `d + 1 <= 100`.
    for d in [1_usize, 2, 5, 10, 50, 99] {
        let mail = nested_multipart(d);
        let env = Envelope::from_bytes(&mail, None).expect("shallow mail should parse");
        let body = env.body_bytes(&mail);
        assert_eq!(
            multipart_depth(&body),
            d + 1,
            "nesting depth {d} must be fully preserved"
        );
        assert_eq!(body.text(Text::Plain), "hello");
    }
}

#[test]
fn c4_cap_boundary_collapses_exactly_one_subtree() {
    // At 101 multipart levels exactly the 101st collapses; the first 100
    // stay real multiparts and the leaf keeps the remaining bytes.
    let mail = nested_multipart(100);
    let env = Envelope::from_bytes(&mail, None).expect("mail should parse");
    let body = env.body_bytes(&mail);
    assert_eq!(multipart_depth(&body), 100);
    let mut node = &body;
    while let ContentType::Multipart { parts, .. } = node.content_type() {
        node = &parts[0];
    }
    assert!(matches!(
        node.content_type(),
        ContentType::OctetStream { .. }
    ));
    assert!(node.raw().windows(5).any(|w| w == b"hello"));
}

#[test]
fn c4_breadth_many_shallow_parts_unaffected() {
    // Width must not be conflated with depth: many sibling parts at one
    // level all parse.
    const COUNT: usize = 500;
    let mut m = String::from(
        "From: a@b.example\r\nSubject: breadth\r\nContent-Type: multipart/mixed; \
         boundary=\"BB\"\r\n\r\n",
    );
    for i in 0..COUNT {
        m.push_str(&format!(
            "--BB\r\nContent-Type: text/plain\r\n\r\npart {i}\r\n"
        ));
    }
    m.push_str("--BB--\r\n");
    let mail = m.into_bytes();
    let env = Envelope::from_bytes(&mail, None).expect("wide mail should parse");
    let body = env.body_bytes(&mail);
    match body.content_type() {
        ContentType::Multipart { parts, .. } => assert_eq!(parts.len(), COUNT),
        other => panic!("expected multipart root, got {other:?}"),
    }
    assert_eq!(multipart_depth(&body), 1);
}
