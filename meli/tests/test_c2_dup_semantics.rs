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

//! C2 auxiliary: Envelope duplicate-header semantics on re-parse (which
//! occurrence feeds envelope.to()/cc()/bcc() used for SMTP RCPT at
//! melib/src/smtp.rs).
//!
//! The audit confirmed duplicate headers are last-wins; this pins that
//! semantic exactly so it is visible and intentional. Duplicates per se are
//! NOT a defect to "fix" here — the CRLF *injection* that could create a
//! hidden duplicate is rejected upstream (see test_c2_mailto_poc.rs and
//! test_c3_reply_poc.rs).

use melib::email::Envelope;

#[test]
fn dup_header_last_occurrence_wins() {
    let raw = "From: a@b.example\r\n\
               To: first@b.example\r\n\
               Bcc: first-bcc@b.example\r\n\
               Subject: s\r\n\
               Bcc: attacker@evil.example\r\n\
               Cc: attacker-cc@evil.example\r\n\
               \r\n\
               body\r\n";
    let env = Envelope::from_bytes(raw.as_bytes(), None).expect("parse");
    // PINNED: for duplicate headers the LAST occurrence wins. This is exactly
    // why a smuggled duplicate Bcc would silently override the visible one —
    // the reason CRLF injection must be rejected before finalisation.
    let bcc: Vec<String> = env
        .bcc()
        .iter()
        .map(|a| a.get_email().to_string())
        .collect();
    assert_eq!(bcc, vec!["attacker@evil.example".to_string()]);
    let tos: Vec<String> = env.to().iter().map(|a| a.get_email().to_string()).collect();
    assert_eq!(tos, vec!["first@b.example".to_string()]);
    let ccs: Vec<String> = env.cc().iter().map(|a| a.get_email().to_string()).collect();
    assert_eq!(ccs, vec!["attacker-cc@evil.example".to_string()]);
}

#[test]
fn no_space_after_colon_header_still_parses() {
    // The exact header-line shape that CRLF injection produces
    // ("...hi\r\nBcc:attacker@evil.example") parses as a real Bcc header —
    // no space after the colon required. This motivates the finalise()
    // chokepoint: rejection must happen before bytes hit the wire.
    let raw = "From: a@b.example\r\nBcc:attacker@evil.example\r\n\r\nbody\r\n";
    let env = Envelope::from_bytes(raw.as_bytes(), None).expect("parse");
    let bcc: Vec<String> = env
        .bcc()
        .iter()
        .map(|a| a.get_email().to_string())
        .collect();
    assert_eq!(bcc, vec!["attacker@evil.example".to_string()]);
}
