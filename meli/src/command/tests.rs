//
// meli
//
// Copyright 2017- Emmanouil Pitsidianakis <manos@pitsidianak.is>
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

use super::*;

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

/// Extra arguments must yield `WrongNumberOfArguments`, not abort the process.
///
/// These parsers used to declare a `min_arg`/`max_arg` that disagreed with the
/// number of `ArgCheck::inc` calls, so `ArgCheck::finish` tripped an `assert!`
/// (active in release too) for trailing input.
#[test]
fn test_arg_count_mismatch_is_reported_not_panicked() {
    use crate::command::parser;

    macro_rules! check_wrong_arg_count {
        ($cmd:literal, $bytes:literal, $parser:path) => {{
            let (_, res) = $parser($bytes)
                .unwrap_or_else(|err| panic!("{:?} failed to parse at all: {err:?}", $cmd));
            match res {
                Ok(action) => panic!("{:?} should have been rejected, got {action:?}", $cmd),
                Err(err) => assert!(
                    matches!(err, CommandError::WrongNumberOfArguments { .. }),
                    "{:?} produced {err:?}",
                    $cmd
                ),
            }
        }};
    }

    check_wrong_arg_count!("flag set junk extra", b"flag set junk extra", parser::flag);
    check_wrong_arg_count!(
        "flag unset junk extra",
        b"flag unset junk extra",
        parser::flag
    );
    check_wrong_arg_count!("tag add foo extra", b"tag add foo extra", parser::_tag);
    check_wrong_arg_count!(
        "tag remove foo extra",
        b"tag remove foo extra",
        parser::_tag
    );
    check_wrong_arg_count!(
        "create-mailbox a b extra",
        b"create-mailbox a b extra",
        parser::create_mailbox
    );
}

/// An attachment index that cannot be incremented must be rejected by the
/// parser: the handler computes `idx + 1` and would overflow on `usize::MAX`.
#[test]
fn test_remove_attachment_index_overflow_is_rejected() {
    use crate::command::parser::remove_attachment;

    for cmd in [
        "remove-attachment 18446744073709551615",
        "remove-attachment 0",
    ] {
        let (_, res) = remove_attachment(cmd.as_bytes()).unwrap();
        if cmd.ends_with("18446744073709551615") {
            assert!(
                matches!(res, Err(CommandError::BadValue { .. })),
                "huge index must be rejected, got {res:?}"
            );
        } else {
            assert!(res.is_ok(), "valid index must still parse: {res:?}");
        }
    }
}

mod toggle_theme_command {
    use super::parser;
    use crate::command::Action;

    /// `toggle theme` must parse to `Action::ToggleTheme`.
    #[test]
    fn parse_toggle_theme() {
        let (rest, parsed) = parser::toggle(b"toggle theme").unwrap();
        assert!(rest.is_empty());
        assert!(matches!(parsed, Ok(Action::ToggleTheme)));
    }

    /// `toggle_theme` (underscore) must NOT be a valid command - the
    /// canonical name is `toggle theme` with a space.
    #[test]
    fn underscore_form_is_invalid() {
        parser::toggle(b"toggle_theme").unwrap_err();
    }

    /// The full `parse_command` chain must route `toggle theme`.
    #[test]
    fn parse_command_routes_toggle_theme() {
        assert!(matches!(
            parser::parse_command(b"toggle theme"),
            Ok(Action::ToggleTheme)
        ));
    }

    /// Bad subcommand still reports with suggestions including `theme`.
    #[test]
    fn bad_subcommand_suggests_theme() {
        let err = parser::parse_command(b"toggle zzz").unwrap_err();
        let suggestions = match &err {
            crate::command::error::CommandError::BadValue { suggestions, .. } => {
                suggestions.unwrap_or(&[])
            }
            _ => panic!("expected BadValue error, got: {err:?}"),
        };
        assert!(
            suggestions.iter().any(|s| s.contains("theme")),
            "suggestions should include 'theme': {suggestions:?}"
        );
    }
}
