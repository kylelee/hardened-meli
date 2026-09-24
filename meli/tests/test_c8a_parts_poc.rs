//
// meli
//
// Copyright 2026 Kyle Lee
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
// along with meli.  If not, see <http://www.gnu.org/licenses/>.
//
// SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later

//! C8a PoC (P1, CWE-835/CWE-1287): the multipart boundary twin loops
//! (`multipart_parts` / `parts_f` in `melib/src/email/parser.rs`) must
//! always terminate and never panic on a hostile mail body. Promoted from
//! the audit scratch examples `melib/examples/{c8_hang,c8a_oob}.rs`; the
//! direct twin-level unit tests live in `melib/src/email/parser/tests.rs`.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc;
use std::time::Duration;

use melib::email::Envelope;

const HEADERS: &[u8] =
    b"From: a@b.example\r\nSubject: s\r\nContent-Type: multipart/mixed; boundary=\"BOUND\"\r\n\r\n";

/// The canonical 113-byte repro (headers 82 bytes + body 31 bytes): the
/// boundary token appears without a preceding `--`, so the first boundary
/// loop re-slices to the same position forever (pre-fix: 100% CPU hang).
const HANG_MAIL_113: &[u8] = b"From: a@b.example\r\nSubject: s\r\n\
     Content-Type: multipart/mixed; boundary=\"BOUND\"\r\n\r\n\
     \r\nBOUND\r\nmore text\r\n--BOUND--\r\n";

fn mail_with(body: &[u8]) -> Vec<u8> {
    [HEADERS, body].concat()
}

/// Run `f` on a worker thread bounded by `limit`: a parser hang fails the
/// test deterministically instead of wedging CI; a panic inside `f` is
/// surfaced as this test's failure. On timeout the worker is leaked (a hung
/// parser thread cannot be killed safely); acceptable in a failing test.
fn assert_completes_within<T: Send + 'static>(
    limit: Duration,
    label: &str,
    f: impl FnOnce() -> T + Send + 'static,
) -> T {
    let (sender, receiver) = mpsc::channel();
    let _worker = std::thread::Builder::new()
        .name(format!("c8a_{label}"))
        .spawn(move || {
            let _ = sender.send(catch_unwind(AssertUnwindSafe(f)));
        })
        .expect("failed to spawn C8a worker thread");
    match receiver.recv_timeout(limit) {
        Ok(Ok(value)) => value,
        Ok(Err(payload)) => std::panic::resume_unwind(payload),
        Err(_) => panic!(
            "{label}: Envelope::from_bytes did not complete within {limit:?} \
             (C8a/CWE-835 non-termination regression)"
        ),
    }
}

#[test]
fn poc_c8a_hang_113_bytes() {
    assert_eq!(HANG_MAIL_113.len(), 113);
    let parsed = assert_completes_within(Duration::from_secs(10), "113-byte hang mail", || {
        Envelope::from_bytes(HANG_MAIL_113, None)
    });
    // Graceful degradation: the hostile multipart must still yield a usable
    // envelope (the degraded body simply has no parts).
    let envelope = parsed.expect("hostile multipart must not fail Envelope::from_bytes");
    assert_eq!(envelope.subject(), "s");
    assert_eq!(envelope.body_bytes(HANG_MAIL_113).attachments().len(), 1);
}

#[test]
fn poc_c8a_hang_forms() {
    // Boundary occurrences not prefixed by `--`: pre-fix infinite loop in
    // both twin first-loops; must terminate and degrade to an envelope
    // whose multipart body has no parts.
    for body in [
        &b"xxBOUND"[..],
        b"body text\r\nBOUND\r\n--BOUND--\r\n",
        b"\r\nBOUND\r\nmore text\r\n--BOUND--\r\n",
    ] {
        let mail = mail_with(body);
        let parsed = assert_completes_within(Duration::from_secs(10), "hang form", move || {
            Envelope::from_bytes(&mail, None)
        });
        let envelope = parsed.expect("hang form must not fail Envelope::from_bytes");
        assert_eq!(envelope.subject(), "s");
        let raw = mail_with(body);
        assert_eq!(
            envelope.body_bytes(&raw).attachments().len(),
            1,
            "degraded multipart keeps itself, gains no parts"
        );
    }
}

#[test]
fn poc_c8a_panic_forms() {
    // (a) body ends exactly at a dash-boundary with EOF (no CRLF, no
    //     closing `--`): pre-fix `input[0]` OOB in both twin first-loops.
    //     `--BOUND\r\n` and `--BOUND--` never panicked and must not now.
    // (b) empty part before the closing boundary: pre-fix `end - 3`
    //     underflow in both twin second-loops.
    // (c) part content starting with the boundary bytes: pre-fix
    //     `&input[end - 2..end]` underflow (end < 2).
    let panic_bodies: &[&[u8]] = &[
        b"--BOUND",
        b"x\r\n--BOUND",
        b"preamble\r\n--BOUND",
        b"--BOUND\r\n--BOUND--\r\n",
        b"--BOUND\r\nBOUND-prefix line\r\n--BOUND--\r\n",
    ];
    for body in panic_bodies {
        let mail = mail_with(body);
        let r = catch_unwind(AssertUnwindSafe(move || {
            let _ = Envelope::from_bytes(&mail, None);
        }));
        if let Err(payload) = &r {
            let msg = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "non-string payload".into());
            eprintln!("[c8a body {body:?}]: PANIC: {msg}");
        }
        assert!(
            r.is_ok(),
            "body {body:?} panicked (C8a/CWE-1287 regression)"
        );
    }
    for body in [&b"--BOUND\r\n"[..], b"--BOUND--"] {
        let mail = mail_with(body);
        let r = catch_unwind(AssertUnwindSafe(move || {
            let _ = Envelope::from_bytes(&mail, None);
        }));
        assert!(r.is_ok(), "body {body:?} must never panic");
    }
}

#[test]
fn poc_c8a_clean_mail_freeze() {
    // Behavior freeze: a regular two-part CRLF multipart parses exactly as
    // before the hardening (both parts present, text intact).
    let raw: &[u8] = b"From: a@b.example\r\n\
         Subject: s\r\n\
         Content-Type: multipart/mixed; boundary=\"BOUND\"\r\n\
         \r\n\
         preamble\r\n\
         --BOUND\r\n\
         Content-Type: text/plain\r\n\
         \r\n\
         hello world.\r\n\
         --BOUND\r\n\
         Content-Type: text/plain\r\n\
         \r\n\
         second part\r\n\
         --BOUND--\r\n";
    let envelope = Envelope::from_bytes(raw, None).expect("clean two-part mail must parse");
    assert_eq!(envelope.subject(), "s");
    let body = envelope.body_bytes(raw);
    assert_eq!(body.content_type().to_string(), "multipart/mixed");
    let text = body.text(melib::email::attachment_types::Text::Plain);
    // Inline text parts concatenate with no separator (pre-fix behavior).
    assert_eq!(text, "hello world.second part");
    // Root multipart + 2 leaf parts.
    assert_eq!(body.attachments().len(), 3);
}
