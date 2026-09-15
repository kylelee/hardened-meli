//
// meli
//
// Copyright 2017- Emmanouil Pitsidianakis <manos@pitsidianak.is>
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

use super::*;

#[test]
fn test_command_parser() {
    let mut input = "sort".to_string();
    macro_rules! match_input {
        ($input:expr) => {{
            let mut sugg: HashSet<String> = Default::default();
            //print!("{}", $input);
            for (_tags, _desc, tokens, _) in COMMAND_COMPLETION.iter() {
                //    //println!("{:?}, {:?}, {:?}", _tags, _desc, tokens);
                let _ = tokens.matches(&mut $input.as_str(), &mut sugg);
                //    if !m.is_empty() {
                //        //print!("{:?} ", desc);
                //        //println!(" result = {:#?}\n\n", m);
                //    }
            }
            //println!("suggestions = {:#?}", sugg);
            sugg.into_iter()
                .map(|s| format!("{}{}", $input.as_str(), s.as_str()))
                .collect::<HashSet<String>>()
        }};
    }
    assert_eq!(
        &match_input!(input),
        &IntoIterator::into_iter(["sort date".to_string(), "sort subject".to_string()]).collect(),
    );
    input = "so".to_string();
    assert_eq!(
        &match_input!(input),
        &IntoIterator::into_iter(["sort".to_string()]).collect(),
    );
    input = "so ".to_string();
    assert_eq!(&match_input!(input), &HashSet::default(),);
    input = "to".to_string();
    assert_eq!(
        &match_input!(input),
        &IntoIterator::into_iter(["toggle".to_string()]).collect(),
    );
    input = "toggle ".to_string();
    assert_eq!(
        &match_input!(input),
        &IntoIterator::into_iter([
            "toggle mouse".to_string(),
            "toggle sign".to_string(),
            "toggle encrypt".to_string(),
            "toggle thread_snooze".to_string()
        ])
        .collect(),
    );
}

#[test]
fn test_command_parser_all() {
    use CommandError::*;

    for cmd in [
        "set unseen",
        "set seen",
        "delete",
        "copyto somewhere",
        "moveto somewhere",
        "import fpath mpath",
        "close  ",
        "go 5",
        "save-all-attachment",
    ] {
        parse_command(cmd.as_bytes()).unwrap_or_else(|err| panic!("{cmd} failed {err}"));
    }

    assert_eq!(
        parse_command(b"setfafsfoo").unwrap_err().to_string(),
        Parsing {
            inner: "setfafsfoo".into(),
            kind: "".into(),
        }
        .to_string(),
    );
    assert_eq!(
        parse_command(b"set foo").unwrap_err().to_string(),
        BadValue {
            inner: "foo".into(),
            suggestions: Some(&[
                "seen",
                "unseen",
                "plain",
                "threaded",
                "compact",
                "conversations"
            ])
        }
        .to_string(),
    );
    assert_eq!(
        parse_command(b"moveto ").unwrap_err().to_string(),
        WrongNumberOfArguments {
            too_many: false,
            takes: (1, Some(1)),
            given: 0,
            __func__: "moveto",
            inner: "".into(),
        }
        .to_string(),
    );
    assert_eq!(
        parse_command(b"reindex 1 2 3").unwrap_err().to_string(),
        WrongNumberOfArguments {
            too_many: true,
            takes: (1, Some(1)),
            given: 2,
            __func__: "reindex",
            inner: "".into(),
        }
        .to_string(),
    );
    assert_eq!(
        parse_command(b"save-all-attachment").unwrap(),
        View(ViewAction::SaveAllAttachments),
    );
    assert_eq!(
        parse_command(b"save-all-attachment extra")
            .unwrap_err()
            .to_string(),
        WrongNumberOfArguments {
            too_many: true,
            takes: (0, Some(0)),
            given: 1,
            __func__: "save_all_attachment",
            inner: "needs at least 0 arguments.".into(),
        }
        .to_string(),
    );
}

#[test]
fn test_command_parser_raw_search() {
    use super::{Action::*, ListingAction::*};

    assert_eq!(
        parse_command(b"raw-search foo bar").unwrap(),
        Listing(Search {
            term: "foo bar".to_string(),
            raw_search: true,
        }),
    );
    assert_eq!(
        parse_command(b"search foo").unwrap(),
        Listing(Search {
            term: "foo".to_string(),
            raw_search: false,
        }),
    );
    assert_eq!(
        parse_command(b"raw-select baz").unwrap(),
        Listing(Select {
            term: "baz".to_string(),
            raw_search: true,
        }),
    );
    assert_eq!(
        parse_command(b"select qux").unwrap(),
        Listing(Select {
            term: "qux".to_string(),
            raw_search: false,
        }),
    );
    // `rawsearch` (no hyphen) must not be recognized as a raw search.
    parse_command(b"rawsearch foo").unwrap_err();
    // Missing argument.
    assert!(matches!(
        parse_command(b"search").unwrap_err(),
        CommandError::WrongNumberOfArguments {
            too_many: false,
            takes: (1, Some(_)),
            given: 0,
            ..
        }
    ));
}

/// Wiring discriminator: `Account::search` with `raw_search = true` must
/// route to `MailBackend::raw_search`. On the mock account (a maildir
/// backend) the trait default returns `ErrorKind::NotSupported`
/// synchronously; if the flag were dropped, a term that is not a valid
/// melib `Query` would instead fail `Query` parsing with a different
/// error kind.
#[test]
fn test_command_parser_raw_search_account_wiring() {
    use melib::{backends::prelude::*, SortField, SortOrder};

    let mut context = crate::golden::mock_context();
    let account_hash = *context.accounts.iter().next().unwrap().0;
    // Force the non-sqlite3 routing: with the sqlite3 search backend the
    // raw flag is ignored (upstream quirk, kept verbatim), and both
    // calls below would fail melib `Query` parsing instead.
    context.accounts[&account_hash].settings.conf.search_backend =
        crate::conf::data_types::SearchBackend::None;
    let account = &context.accounts[&account_hash];
    let mailbox_hash = account
        .mailbox_entries
        .keys()
        .next()
        .copied()
        .unwrap_or_else(|| MailboxHash::from_bytes(b"INBOX"));

    // The maildir backend does not override `raw_search`: the default
    // method must fail synchronously with `NotSupported`.
    let err = match account.search(
        "~invalid~ ((",
        true,
        (SortField::Date, SortOrder::Desc),
        mailbox_hash,
    ) {
        Err(err) => err,
        Ok(_) => panic!("raw search must fail synchronously on the maildir backend"),
    };
    assert!(
        matches!(err.kind, melib::ErrorKind::NotSupported),
        "raw search must be rejected with NotSupported, got {:?}",
        err.kind
    );

    // Discriminator: with the flag dropped the same term would go
    // through melib `Query` parsing and fail with a different error
    // kind (a parsing error, not `NotSupported`).
    let err = match account.search(
        "~invalid~ ((",
        false,
        (SortField::Date, SortOrder::Desc),
        mailbox_hash,
    ) {
        Err(err) => err,
        Ok(_) => panic!("non-raw search of a non-Query term must fail parsing"),
    };
    assert!(
        !matches!(err.kind, melib::ErrorKind::NotSupported),
        "non-raw search of a non-Query term must fail with a parsing \
         error, not NotSupported; got {:?}",
        err.kind
    );
}

#[test]
fn test_command_error_display() {
    assert_eq!(
        &CommandError::BadValue {
            inner: "foo".into(),
            suggestions: Some(&[
                "seen",
                "unseen",
                "plain",
                "threaded",
                "compact",
                "conversations"
            ])
        }
        .to_string(),
        "Bad value/argument: foo. Possible values are: seen, unseen, plain, threaded, compact, \
         conversations"
    );
}
