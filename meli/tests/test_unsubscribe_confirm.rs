//
// meli
//
// Copyright 2026 W1-T3 security hardening (List-Unsubscribe confirmation gate)
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

//! W1-T3: the `List-Unsubscribe` confirmation gate.
//!
//! `MailView` must ask the user to confirm the exact target address (the C2
//! amplifier fix) before any unsubscribe e-mail is composed or sent, or any
//! URL is handed to the system launcher. These tests pin down the target
//! extraction: what the dialog shows must be what the action will use.
//!
//! The mock-session confirm/cancel paths (real `UIConfirmationDialog` driven
//! with key events against a mock `Context`) live in
//! `meli/src/mail/view/tests.rs` because they need `Context::new_mock`, which
//! is `#[cfg(test)]`-only.

use melib::{
    email::{list_management::ListAction, HeaderName},
    Draft,
};

use meli::mail::view::{unsubscribe_action, UnsubscribeAction};

fn options(header_value: &[u8]) -> Vec<ListAction<'_>> {
    ListAction::parse_options_list(header_value)
        .expect("could not parse List-Unsubscribe value")
        .to_vec()
}

#[test]
fn unsubscribe_action_picks_mailto_and_shows_exact_recipient() {
    let parsed = options(b"<mailto:unsubscribe@list.example?subject=bye>, <https://unsubscribe.example/leave?token=1>");
    let action = unsubscribe_action(&parsed).expect("mailto option must be actionable");
    let UnsubscribeAction::Send(ref mailto) = action else {
        panic!("mailto option must be preferred over URL, got {action:?}");
    };

    let shown = action.target_description();
    assert_eq!(shown, "unsubscribe@list.example");

    // What is shown must be what will be sent: the draft's To: header is built
    // from the same parsed addresses (parser.rs builds `To` out of
    // `Mailto::address`).
    let draft: Draft = mailto.clone().into();
    let to = draft.headers().get(&HeaderName::TO).expect("draft has To:");
    assert_eq!(to.trim(), shown);
}

#[test]
fn unsubscribe_action_picks_url_when_no_mailto() {
    let parsed = options(b"<https://unsubscribe.example/one-click-post>");
    let action = unsubscribe_action(&parsed).expect("url option must be actionable");
    match action {
        UnsubscribeAction::OpenUrl(ref url) => {
            assert_eq!(url, "https://unsubscribe.example/one-click-post");
            assert_eq!(action.target_description(), url.as_str());
        }
        other => panic!("expected OpenUrl, got {other:?}"),
    }
}

#[test]
fn unsubscribe_action_skips_unparseable_mailto_and_falls_through() {
    let parsed = options(b"<mailto:>, <https://fallback.example/unsub>");
    // `mailto:` with no address fails to parse; dispatch must fall through to
    // the URL option instead of sending to an empty recipient list.
    let action = unsubscribe_action(&parsed).expect("fallback URL must be actionable");
    assert!(matches!(action, UnsubscribeAction::OpenUrl(_)));
}

#[test]
fn unsubscribe_action_rejects_crlf_smuggle_instead_of_sending() {
    // C2 amplifier input: percent-encoded CR/LF in the mailto address part.
    // The parser rejects it (RFC6068); the gate must then treat the option as
    // unparseable, NOT display or send it.
    let raw = b"mailto:victim@victim.example%0d%0aBcc:attacker@evil.example";
    let parsed = vec![ListAction::Email(raw.as_slice())];
    assert_eq!(unsubscribe_action(&parsed), None);
}

#[test]
fn unsubscribe_action_handles_malformed_input_without_panicking() {
    // Valid-UTF-8 malformations only for the mailto probes: `Mailto::try_from`
    // formats its parse error with `{:?}`, and melib's `ParsingError` Debug
    // impl panics on invalid UTF-8 input (pre-existing, tracked in
    // issues.md); invalid UTF-8 is probed via the URL branch, which only
    // does a lossy conversion.
    let malformed: &[&[u8]] = &[
        b"mailto:",
        b"mailto:?",
        b"mailto:not an address",
        b"mailto:%zz",
        b"",
    ];
    for raw in malformed {
        if let Some(action) = unsubscribe_action(&[ListAction::Email(raw)]) {
            let _ = action.target_description();
        }
    }
    for raw in [
        b"https://\xff\xfe/".as_slice(),
        b"",
        b"https://example.com/%zz",
    ] {
        if let Some(action) = unsubscribe_action(&[ListAction::Url(raw)]) {
            let _ = action.target_description();
        }
    }
}

#[test]
fn unsubscribe_action_none_for_no_options() {
    assert_eq!(unsubscribe_action(&[ListAction::No]), None);
    assert_eq!(unsubscribe_action(&[]), None);
}
