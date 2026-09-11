//
// meli
//
// Copyright 2025 PoC engineers (security audit scratch harness)
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

//! C3 regression: RFC2047-encoded CRLF in a From display name (CWE-93).
//!
//! Historical PoC (pre-fix proof): a From header whose encoded-word display
//! name decoded to `X\r\nBcc:attacker@evil.example` stored the raw CRLF in
//! `Address::display_name`; `Draft::new_reply` copied it into the To: header
//! via `field_from_to_string()`, and the finalised reply re-parsed as
//! `to = []`, `bcc = [attacker@evil.example]` — i.e. the attacker rewrote
//! the SOLE reply recipient into a hidden Bcc.
//!
//! Fixed behavior pinned here: the display name is scrubbed of C0 controls
//! at the decode/storage boundary, and the reply's sole recipient remains
//! the original sender.

use melib::email::{Draft, Envelope, HeaderName};

const ATTACK_MAIL: &[u8] =
    b"From: =?utf-8?Q?X=0D=0ABcc=3Aattacker=40evil=2Eexample?= <innocent@x.example>\r\n\
To: victim@victim.example\r\n\
Subject: hello\r\n\
Message-ID: <m1@x.example>\r\n\
Date: Thu, 1 Jan 2026 00:00:00 +0000\r\n\
\r\n\
please reply\r\n";

#[test]
fn reply_to_rfc2047_crlf_display_name_cannot_rewrite_recipient() {
    let env = Envelope::from_bytes(ATTACK_MAIL, None).expect("parse");

    // Decode/storage boundary: the stored display name must be free of C0
    // control characters (CR/LF stripped), while the remaining display text
    // is preserved.
    let display_name = env.from()[0].get_display_name().expect("display name");
    assert!(
        !display_name.bytes().any(|b| b == b'\r' || b == b'\n'),
        "stored display name must not contain CR/LF: {display_name:?}"
    );
    assert_eq!(display_name, "XBcc:attacker@evil.example");

    let mut reply = Draft::new_reply(&env, ATTACK_MAIL, true);
    reply.set_header(HeaderName::FROM, "victim@victim.example".to_string());
    let to_value = reply.headers()[&HeaderName::TO].to_string();
    assert!(
        !to_value.bytes().any(|b| b == b'\r' || b == b'\n'),
        "reply To header must not contain CR/LF: {to_value:?}"
    );

    let raw = reply
        .finalise()
        .expect("scrubbed display name must finalise without error");

    // Assert on FINAL header bytes, not just API success: the only Bcc line
    // in the outgoing header block is the empty default one; the attacker
    // address appears (if at all) only inside the quoted To display name and
    // the quoted attribution line of the body, never as a header line.
    let header_block = raw.split("\r\n\r\n").next().unwrap();
    let bcc_lines: Vec<&str> = header_block
        .lines()
        .filter(|l| l.starts_with("Bcc:"))
        .collect();
    assert!(
        bcc_lines.iter().all(|l| *l == "Bcc: "),
        "no non-empty Bcc header line may exist: {bcc_lines:?} in {header_block:?}"
    );

    // Re-parse exactly like SMTP submission does (recipients come from
    // envelope.to()/cc()/bcc()): the attacker must not appear as a
    // recipient, and the SOLE reply recipient must be the original sender.
    let reply_env = Envelope::from_bytes(raw.as_bytes(), None).expect("re-parse");
    let bcc: Vec<String> = reply_env
        .bcc()
        .iter()
        .map(|a| a.get_email().to_string())
        .collect();
    assert!(
        !bcc.iter().any(|b| b == "attacker@evil.example"),
        "injected Bcc must never survive into a re-parseable recipient: {bcc:?}"
    );
    assert!(reply_env.bcc().is_empty(), "no Bcc at all: {bcc:?}");
    assert!(reply_env.cc().is_empty());
    let tos: Vec<String> = reply_env
        .to()
        .iter()
        .map(|a| a.get_email().to_string())
        .collect();
    assert_eq!(
        tos,
        vec!["innocent@x.example".to_string()],
        "the sole reply recipient must be the original sender"
    );
}

#[test]
fn reply_from_clean_mail_is_byte_stable() {
    // Positive control / freeze: a clean From display name (quotes included)
    // round-trips through new_reply → finalise unchanged: the reply To header
    // keeps the quoted display name and the sole recipient is the sender.
    let clean_mail: &[u8] = b"From: \"Innocent B. Sender\" <innocent@x.example>\r\n\
To: victim@victim.example\r\n\
Subject: hello\r\n\
Message-ID: <m2@x.example>\r\n\
Date: Thu, 1 Jan 2026 00:00:00 +0000\r\n\
\r\n\
please reply\r\n";
    let env = Envelope::from_bytes(clean_mail, None).expect("parse");
    let mut reply = Draft::new_reply(&env, clean_mail, true);
    reply.set_header(HeaderName::FROM, "victim@victim.example".to_string());
    let raw = reply.finalise().expect("finalise");
    assert!(
        raw.contains("To: \"Innocent B. Sender\" <innocent@x.example>\r\n"),
        "clean quoted display name must be preserved: {raw:?}"
    );
    let reply_env = Envelope::from_bytes(raw.as_bytes(), None).expect("re-parse");
    let tos: Vec<String> = reply_env
        .to()
        .iter()
        .map(|a| a.get_email().to_string())
        .collect();
    assert_eq!(tos, vec!["innocent@x.example".to_string()]);
}

#[test]
fn fmt_mailbox_escapes_embedded_quotes_and_backslashes() {
    use melib::email::Address;

    // Freeze: clean display names format exactly as before the fix.
    assert_eq!(
        Address::new(Some("Jörg T. Doe"), "joerg@example.com").to_string(),
        "\"Jörg T. Doe\" <joerg@example.com>"
    );
    assert_eq!(
        Address::new(Some("plain"), "joerg@example.com").to_string(),
        "plain <joerg@example.com>"
    );
    // Embedded double quotes are escaped per RFC5322 quoted-string.
    assert_eq!(
        Address::new(Some("He said \"hi\""), "a@example.com").to_string(),
        "\"He said \\\"hi\\\"\" <a@example.com>"
    );
    // Embedded backslashes are escaped too (pre-fix this produced an invalid
    // bare backslash inside the quoted string).
    assert_eq!(
        Address::new(Some("back\\slash"), "a@example.com").to_string(),
        "\"back\\\\slash\" <a@example.com>"
    );
    // Display names are scrubbed of C0 controls at construction.
    assert_eq!(
        Address::new(Some("X\r\nBcc:a@b"), "c@d.example").get_display_name(),
        Some("XBcc:a@b")
    );
    assert_eq!(
        Address::new_group("gr\roup", vec![Address::new(None::<&str>, "a@b.example")])
            .get_display_name(),
        Some("group")
    );
}
