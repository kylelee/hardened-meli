//
// meli - backends module
//
// Copyright 2017 Manos Pitsidianakis
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
// along with meli. If not, see <http://www.gnu.org/licenses/>.
//
// SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later

use crate::{
    backends::{
        AccountHash, BackendEvent, EnvelopeHash, EnvelopeHashBatch, LazyCountSet, MailboxHash,
        RefreshEvent, RefreshEventKind,
    },
    utils::logging::LogLevel,
};

#[test]
fn test_lazy_count_set() {
    let mut new = LazyCountSet::default();
    assert_eq!(new.len(), 0);
    new.set_not_yet_seen(10);
    assert_eq!(new.len(), 10);
    for i in 0..10 {
        assert!(new.insert_existing(EnvelopeHash(i)));
    }
    assert_eq!(new.len(), 10);
    assert!(new.insert_existing(EnvelopeHash(10)));
    assert_eq!(new.len(), 11);
}

#[test]
fn test_backend_event_flatten() {
    const NOTICE: BackendEvent = BackendEvent::Notice {
        description: String::new(),
        content: None,
        level: LogLevel::ERROR,
    };
    const ACS: BackendEvent = BackendEvent::AccountStateChange {
        message: std::borrow::Cow::<'static, str>::Borrowed(""),
    };
    const REFRESH_1: RefreshEvent = RefreshEvent {
        account_hash: AccountHash(0),
        mailbox_hash: MailboxHash(0),
        kind: RefreshEventKind::Rescan,
    };
    const REFRESH_2: RefreshEvent = RefreshEvent {
        account_hash: AccountHash(0),
        mailbox_hash: MailboxHash(0),
        kind: RefreshEventKind::MailboxDelete(MailboxHash(0)),
    };
    const REFRESH_3: RefreshEvent = RefreshEvent {
        account_hash: AccountHash(0),
        mailbox_hash: MailboxHash(1),
        kind: RefreshEventKind::MailboxDelete(MailboxHash(1)),
    };
    const REFRESH_4: RefreshEvent = RefreshEvent {
        account_hash: AccountHash(0),
        mailbox_hash: MailboxHash(1),
        kind: RefreshEventKind::MailboxSubscribe(MailboxHash(1)),
    };

    assert_eq!(BackendEvent::flatten(vec![]), vec![]);
    assert_eq!(
        BackendEvent::flatten(vec![NOTICE, ACS.clone()]),
        vec![NOTICE, ACS.clone()]
    );
    assert_eq!(
        BackendEvent::flatten(vec![
            NOTICE,
            BackendEvent::Refresh(REFRESH_1.clone()),
            ACS.clone()
        ]),
        vec![
            NOTICE,
            BackendEvent::Refresh(REFRESH_1.clone()),
            ACS.clone()
        ]
    );
    assert_eq!(
        BackendEvent::flatten(vec![
            NOTICE,
            BackendEvent::RefreshBatch(vec![REFRESH_1.clone()]),
            ACS.clone()
        ]),
        vec![
            NOTICE,
            BackendEvent::RefreshBatch(vec![REFRESH_1.clone()]),
            ACS.clone()
        ]
    );
    assert_eq!(
        BackendEvent::flatten(vec![
            NOTICE,
            BackendEvent::Refresh(REFRESH_1.clone()),
            BackendEvent::Refresh(REFRESH_2.clone()),
            ACS.clone()
        ]),
        vec![
            NOTICE,
            BackendEvent::RefreshBatch(vec![REFRESH_1.clone(), REFRESH_2.clone()]),
            ACS.clone()
        ]
    );
    assert_eq!(
        BackendEvent::flatten(vec![
            NOTICE,
            BackendEvent::Refresh(REFRESH_1.clone()),
            BackendEvent::RefreshBatch(vec![REFRESH_2.clone()]),
            ACS.clone()
        ]),
        vec![
            NOTICE,
            BackendEvent::RefreshBatch(vec![REFRESH_1.clone(), REFRESH_2.clone()]),
            ACS.clone()
        ]
    );
    assert_eq!(
        BackendEvent::flatten(vec![
            NOTICE,
            BackendEvent::RefreshBatch(vec![REFRESH_1.clone()]),
            BackendEvent::Refresh(REFRESH_2.clone()),
            ACS.clone()
        ]),
        vec![
            NOTICE,
            BackendEvent::RefreshBatch(vec![REFRESH_1.clone(), REFRESH_2.clone()]),
            ACS.clone()
        ]
    );
    assert_eq!(
        BackendEvent::flatten(vec![
            ACS.clone(),
            BackendEvent::RefreshBatch(vec![REFRESH_1.clone()]),
            BackendEvent::Refresh(REFRESH_2.clone()),
            BackendEvent::Refresh(REFRESH_3.clone()),
            BackendEvent::Refresh(REFRESH_4.clone()),
        ]),
        vec![
            ACS.clone(),
            BackendEvent::RefreshBatch(vec![
                REFRESH_1.clone(),
                REFRESH_2.clone(),
                REFRESH_3.clone(),
                REFRESH_4.clone(),
            ]),
        ]
    );
}

/// An unknown or differently-cased `format` setting must not panic:
/// config validation lowercases the format, so the lookup has to be
/// case-insensitive too, and a misspelling is a configuration error.
#[test]
fn backend_lookup_is_case_insensitive_and_reports_unknown_format() {
    use crate::{
        backends::{Backend, Backends},
        error::{Error, ErrorKind},
    };

    let mut backends = Backends::new();
    backends.register(
        "dummy".to_string(),
        Backend {
            create_fn: Box::new(|| {
                Box::new(|_, _, _| {
                    Err(Error::new("dummy backend").set_kind(ErrorKind::NotSupported))
                })
            }),
            validate_conf_fn: Box::new(|_| Ok(())),
        },
    );
    for key in ["dummy", "DUMMY", "dUmMy"] {
        if backends.get(key).is_err() {
            panic!("`{key}` must resolve to the registered backend");
        }
    }
    // `BackendCreator` is not `Debug`, so match instead of `expect_err`.
    let err = match backends.get("dummy-backend-typo") {
        Ok(_) => panic!("an unknown format must be a configuration error"),
        Err(err) => err,
    };
    assert_eq!(err.kind, ErrorKind::Configuration);
}

/// A large [`BackendEvent::RefreshBatch`] must not dump every
/// [`RefreshEvent`] in its `Debug` output (upstream 32733460: batches can
/// be huge, and `Debug` is used in logs). Batches of 30 or more events are
/// summarized as the event count plus the first 30 entries, keeping the
/// output bounded.
#[test]
fn test_backend_event_debug_refresh_batch_output_is_bounded() {
    let big_batch = BackendEvent::RefreshBatch(
        std::iter::repeat_n(
            RefreshEvent {
                account_hash: AccountHash(0),
                mailbox_hash: MailboxHash(0),
                kind: RefreshEventKind::Rescan,
            },
            300,
        )
        .collect(),
    );

    let debug = format!("{big_batch:?}");
    assert!(
        debug.len() < 4096,
        "Debug output for a large batch must be bounded, got {} bytes",
        debug.len()
    );
    assert!(
        debug.contains("length: 300"),
        "the event count must be reported: {debug}"
    );
    assert_eq!(
        debug.matches("RefreshEvent").count(),
        30,
        "only the first 30 events may be printed: {debug}"
    );
}

/// Batches below the size threshold and the other variants must keep
/// printing all their fields, so small updates stay diagnosable.
#[test]
fn test_backend_event_debug_small_refresh_batch_and_other_variants_are_complete() {
    let event = || RefreshEvent {
        account_hash: AccountHash(7),
        mailbox_hash: MailboxHash(9),
        kind: RefreshEventKind::Rescan,
    };

    let small_batch = BackendEvent::RefreshBatch(vec![event(), event(), event()]);
    let debug = format!("{small_batch:?}");
    assert_eq!(
        debug.matches("RefreshEvent").count(),
        3,
        "all events of a small batch must be printed: {debug}"
    );
    assert!(
        !debug.contains(".."),
        "a small batch must not be summarized: {debug}"
    );

    let notice = BackendEvent::Notice {
        description: "description".to_string(),
        content: Some("content".to_string()),
        level: LogLevel::ERROR,
    };
    let debug = format!("{notice:?}");
    for expected in [
        "BackendEvent::Notice",
        "description: \"description\"",
        "content: Some(\"content\")",
        "level: ERROR",
    ] {
        assert!(debug.contains(expected), "missing {expected:?} in {debug}");
    }

    let debug = format!("{:?}", BackendEvent::Refresh(event()));
    assert!(
        debug.contains("BackendEvent::Refresh") && debug.contains("Rescan"),
        "a single refresh must be printed in full: {debug}"
    );

    let account_state_change = BackendEvent::AccountStateChange {
        message: std::borrow::Cow::Borrowed("message"),
    };
    let debug = format!("{account_state_change:?}");
    assert!(
        debug.contains("Backend::AccountStateChange") && debug.contains("message: \"message\""),
        "an account state change must be printed in full: {debug}"
    );
}

/// Upstream 1218cb74: a `Vec<EnvelopeHash>` converts into an
/// [`EnvelopeHashBatch`] by splitting into the first hash and the rest;
/// an empty vector has no first hash and must be rejected.
#[test]
fn test_envelope_hash_batch_try_from_vec() {
    EnvelopeHashBatch::try_from(Vec::<EnvelopeHash>::new()).unwrap_err();

    let single = EnvelopeHashBatch::try_from(vec![EnvelopeHash(1)]).unwrap();
    assert_eq!(single.first, EnvelopeHash(1));
    assert!(single.rest.is_empty());

    let multi =
        EnvelopeHashBatch::try_from(vec![EnvelopeHash(1), EnvelopeHash(2), EnvelopeHash(3)])
            .unwrap();
    assert_eq!(multi.first, EnvelopeHash(1));
    assert_eq!(multi.rest, vec![EnvelopeHash(2), EnvelopeHash(3)]);
}
