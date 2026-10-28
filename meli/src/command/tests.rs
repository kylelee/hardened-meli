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

/// `pipe` and `pipe-attachment` must carry the whole rest of the line as a
/// single command string: shell syntax (quotes, pipes, paths with spaces)
/// rides verbatim into `sh -c` and is never split into bin/args. Errors on
/// missing or malformed arguments must keep erroring.
#[test]
fn test_command_parser_pipe_whole_line() {
    assert_eq!(
        parse_command(b"pipe cat 'foo bar' | wc -c").unwrap(),
        View(ViewAction::Pipe {
            command: "cat 'foo bar' | wc -c".to_string(),
        }),
    );
    assert_eq!(
        parse_command(b"pipe /usr/local/bin/my-filter --opt \"a b\"").unwrap(),
        View(ViewAction::Pipe {
            command: "/usr/local/bin/my-filter --opt \"a b\"".to_string(),
        }),
    );
    // A single binary without arguments still parses.
    assert_eq!(
        parse_command(b"pipe less").unwrap(),
        View(ViewAction::Pipe {
            command: "less".to_string(),
        }),
    );
    // An empty command is a missing argument, not an empty command line.
    parse_command(b"pipe").unwrap_err();
    parse_command(b"pipe ").unwrap_err();

    assert_eq!(
        parse_command(b"pipe-attachment 0 git am -3").unwrap(),
        View(ViewAction::PipeAttachment {
            index: 0,
            command: "git am -3".to_string(),
        }),
    );
    assert_eq!(
        parse_command(b"pipe-attachment 1 chafa --scale 2x --polite on").unwrap(),
        View(ViewAction::PipeAttachment {
            index: 1,
            command: "chafa --scale 2x --polite on".to_string(),
        }),
    );
    // Missing index or command, or a non-numeric index, keep erroring.
    parse_command(b"pipe-attachment").unwrap_err();
    parse_command(b"pipe-attachment 0").unwrap_err();
    parse_command(b"pipe-attachment 0 ").unwrap_err();
    parse_command(b"pipe-attachment x git am").unwrap_err();

    // CR/LF must never ride into the action: a mid-line newline fails the
    // parse outright.
    parse_command(b"pipe cat\nrm -rf ~").unwrap_err();
    parse_command(b"pipe-attachment 0 cat\necho pwned").unwrap_err();
}

/// `terminal` with no arguments maps to `Terminal(None)` (whitespace-only
/// tails included); everything after the first space rides as one whole
/// command line for `sh -c`, and a non-space boundary (`terminalx`) must
/// not parse as `terminal`.
#[test]
fn test_command_parser_terminal() {
    assert_eq!(parse_command(b"terminal").unwrap(), Terminal(None));
    assert_eq!(parse_command(b"terminal    ").unwrap(), Terminal(None));
    assert_eq!(
        parse_command(b"terminal git status | less").unwrap(),
        Terminal(Some("git status | less".into())),
    );
    parse_command(b"terminalx").unwrap_err();
}

/// The command palette's completion table must advertise the `terminal`
/// command, and its listed parser must resolve to the exact `Action`.
#[test]
fn test_command_completion_terminal_entry() {
    let (_, _, parser) = COMMAND_COMPLETION
        .iter()
        .find(|(tag, _, _)| *tag == "terminal ")
        .expect("terminal command must be listed in the command completion table");
    let (_, parsed) = parser(b"terminal git status | less").unwrap();
    assert_eq!(parsed.unwrap(), Terminal(Some("git status | less".into())));
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
    check_wrong_arg_count!(
        "flag toggle junk extra",
        b"flag toggle junk extra",
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

/// Every new mail action command must map to its exact `Action`.
#[test]
fn test_command_parser_mail_actions() {
    use super::{Action::*, ComposeAction, ListingAction};

    assert_eq!(
        parse_command(b"reply").unwrap(),
        Compose(ComposeAction::Reply)
    );
    assert_eq!(
        parse_command(b"reply all").unwrap(),
        Compose(ComposeAction::ReplyToAll)
    );
    assert_eq!(
        parse_command(b"reply author").unwrap(),
        Compose(ComposeAction::ReplyToAuthor)
    );
    assert_eq!(
        parse_command(b"forward").unwrap(),
        Compose(ComposeAction::Forward)
    );
    assert_eq!(
        parse_command(b"forward inline").unwrap(),
        Compose(ComposeAction::ForwardInline)
    );
    assert_eq!(
        parse_command(b"forward attachment").unwrap(),
        Compose(ComposeAction::ForwardAttachment)
    );
    assert_eq!(
        parse_command(b"new-mail").unwrap(),
        Compose(ComposeAction::New)
    );
    assert_eq!(
        parse_command(b"open").unwrap(),
        Listing(ListingAction::OpenEntry)
    );
    assert_eq!(
        parse_command(b"refresh").unwrap(),
        Listing(ListingAction::Refresh)
    );
}

/// `refresh` takes an optional `ACCOUNT` and `MAILBOX_PATH`: with no arguments it
/// refreshes the current mailbox listing (unchanged), with an account it
/// requests a refresh of the account's mailboxes, and with both it targets one
/// specific mailbox.
#[test]
fn test_command_parser_refresh() {
    use super::{AccountAction, Action, ListingAction};

    // No arguments: the current mailbox listing, with or without trailing
    // whitespace.
    assert_eq!(
        parse_command(b"refresh").unwrap(),
        Action::Listing(ListingAction::Refresh)
    );
    assert_eq!(
        parse_command(b"refresh ").unwrap(),
        Action::Listing(ListingAction::Refresh)
    );

    // One argument: all mailboxes of the account.
    assert_eq!(
        parse_command(b"refresh accname").unwrap(),
        Action::AccountAction(
            "accname".to_string(),
            AccountAction::Refresh { mailbox: None },
        )
    );

    // Two arguments: one specific mailbox of the account; quoted arguments
    // are accepted.
    assert_eq!(
        parse_command(b"refresh accname INBOX").unwrap(),
        Action::AccountAction(
            "accname".to_string(),
            AccountAction::Refresh {
                mailbox: Some("INBOX".to_string()),
            },
        )
    );
    assert_eq!(
        parse_command(b"refresh \"acc name\" \"INBOX/Foo\"").unwrap(),
        Action::AccountAction(
            "acc name".to_string(),
            AccountAction::Refresh {
                mailbox: Some("INBOX/Foo".to_string()),
            },
        )
    );
}

/// Malformed `refresh` input must be reported as structured command errors.
#[test]
fn test_command_parser_refresh_rejects_bad_input() {
    use crate::command::parser;

    // More arguments than the accepted ACCOUNT MAILBOX_PATH form.
    for cmd in [
        "refresh accname INBOX extra",
        "refresh accname INBOX extra even-more",
    ] {
        let (_, res) = parser::refresh(cmd.as_bytes())
            .unwrap_or_else(|err| panic!("{cmd} failed to parse at all: {err}"));
        assert!(
            matches!(res, Err(CommandError::WrongNumberOfArguments { .. })),
            "{cmd} must be rejected, got {res:?}"
        );
    }

    // Empty quoted arguments.
    for cmd in ["refresh \"\"", "refresh accname \"\""] {
        let (_, res) = parser::refresh(cmd.as_bytes())
            .unwrap_or_else(|err| panic!("{cmd} failed to parse at all: {err}"));
        assert!(
            matches!(res, Err(CommandError::BadValue { .. })),
            "{cmd} must be rejected, got {res:?}"
        );
    }
}

/// Prefix disambiguation: the single-word parsers must not swallow the
/// multi-word commands, and `open` must not swallow the existing
/// `open-in-tab`.
#[test]
fn test_command_parser_mail_action_disambiguation() {
    use super::{Action::*, ComposeAction, ListingAction};

    assert_eq!(
        parse_command(b"open-in-tab").unwrap(),
        Listing(ListingAction::OpenInNewTab)
    );
    assert_eq!(
        parse_command(b"reply all").unwrap(),
        Compose(ComposeAction::ReplyToAll)
    );
    assert_eq!(
        parse_command(b"reply author").unwrap(),
        Compose(ComposeAction::ReplyToAuthor)
    );
    assert_eq!(
        parse_command(b"reply").unwrap(),
        Compose(ComposeAction::Reply)
    );
    assert_eq!(
        parse_command(b"forward inline").unwrap(),
        Compose(ComposeAction::ForwardInline)
    );
    assert_eq!(
        parse_command(b"forward attachment").unwrap(),
        Compose(ComposeAction::ForwardAttachment)
    );
    assert_eq!(
        parse_command(b"forward").unwrap(),
        Compose(ComposeAction::Forward)
    );
}

#[test]
fn test_command_parser_public_inbox_import() {
    use melib::email::MessageID;

    // Unquoted arguments.
    let (rest, parsed) =
        parser::public_inbox_import(b"public-inbox import foo bar message@example.com").unwrap();
    assert_eq!(rest, b"");
    assert!(
        matches!(
            parsed,
            Ok(Action::Listing(ListingAction::PublicInboxImport {
                thread,
                ref account,
                ref mailbox_path,
                ref message_id,
            })) if (thread, account.as_str(), mailbox_path.as_str(), message_id)
                == (false, "foo", "bar", &MessageID::new("message@example.com"))
        ),
        "{:?}",
        parsed
    );
    // Quoted arguments and bracketed message ID, thread variant.
    let (rest, parsed) = parser::public_inbox_import(
        b"public-inbox import-thread foo \"bar\" <message@example.com>",
    )
    .unwrap();
    assert_eq!(rest, b"");
    assert!(
        matches!(
            parsed,
            Ok(Action::Listing(ListingAction::PublicInboxImport {
                thread,
                ref account,
                ref mailbox_path,
                ref message_id,
            })) if (thread, account.as_str(), mailbox_path.as_str(), message_id)
                == (true, "foo", "bar", &MessageID::new("message@example.com"))
        ),
        "{:?}",
        parsed
    );
    // Unknown subcommand must report the two possible values.
    let (rest, parsed) =
        parser::public_inbox_import(b"public-inbox import-foo foo bar <message@example.com>")
            .unwrap();
    assert_eq!(rest, b"");
    assert_eq!(
        parsed.unwrap_err().to_string(),
        "Bad value/argument: import-foo. Possible values are: import, import-thread"
    );
    // Routed through the general command parser as well.
    assert_eq!(
        parse_command(b"public-inbox import foo bar <message@example.com>").unwrap(),
        Action::Listing(ListingAction::PublicInboxImport {
            thread: false,
            account: "foo".to_string(),
            mailbox_path: "bar".to_string(),
            message_id: MessageID::new("<message@example.com>"),
        }),
    );
    // Missing arguments fail to parse.
    parse_command(b"public-inbox import foo").unwrap_err();
    // Trailing junk after the message ID fails to parse.
    parse_command(b"public-inbox import foo bar <m@example.com> baz").unwrap_err();
}

/// `help` opens the general guide (`meli.7`), accepts no arguments, and is
/// reachable from the `new-tab` parser chain like its `man` sibling.
#[test]
fn test_command_parser_help() {
    #[cfg(feature = "cli-docs")]
    {
        assert_eq!(
            parse_command(b"help").unwrap(),
            Action::Tab(TabAction::Man(crate::manpages::ManPages::Guide))
        );
        // Leading/trailing whitespace is trimmed, like every other command.
        assert_eq!(
            parse_command(b"  help  ").unwrap(),
            Action::Tab(TabAction::Man(crate::manpages::ManPages::Guide))
        );
    }
    #[cfg(not(feature = "cli-docs"))]
    {
        assert_eq!(
            parse_command(b"help").unwrap_err().to_string(),
            "this meli binary has not been compiled with the cli-docs feature"
        );
    }
    // `help` takes no arguments.
    assert!(matches!(
        parse_command(b"help me").unwrap_err(),
        CommandError::WrongNumberOfArguments { .. }
    ));
    // It must not shadow the `man` command or any other parser.
    parse_command(b"manpage").unwrap_err();
}

/// `cd` parses its argument as a raw string (kept unexpanded until the
/// state layer validates it), and rejects missing arguments.
#[test]
fn test_command_parser_cd() {
    assert_eq!(
        parse_command(b"cd /tmp").unwrap(),
        Action::ChangeCurrentDirectory("/tmp".to_string())
    );
    // Tilde and relative forms are preserved verbatim.
    assert_eq!(
        parse_command(b"cd ~/mail").unwrap(),
        Action::ChangeCurrentDirectory("~/mail".to_string())
    );
    assert_eq!(
        parse_command(b"cd ..").unwrap(),
        Action::ChangeCurrentDirectory("..".to_string())
    );
    // Interior spaces survive (paths may contain them); trailing
    // whitespace is trimmed like every other command.
    assert_eq!(
        parse_command(b"cd /tmp/my folder  ").unwrap(),
        Action::ChangeCurrentDirectory("/tmp/my folder".to_string())
    );
    // `cd` requires an argument.
    assert!(matches!(
        parse_command(b"cd").unwrap_err(),
        CommandError::WrongNumberOfArguments { .. }
    ));
    assert!(matches!(
        parse_command(b"cd ").unwrap_err(),
        CommandError::WrongNumberOfArguments { .. }
    ));
    // The rest of the line is taken as one path string, spaces included.
    assert_eq!(
        parse_command(b"cd one two").unwrap(),
        Action::ChangeCurrentDirectory("one two".to_string())
    );
}

/// `pwd`/`cwd` keep parsing to the argument-less `CurrentDirectory`.
#[test]
fn test_command_parser_pwd() {
    assert_eq!(parse_command(b"pwd").unwrap(), Action::CurrentDirectory);
    assert_eq!(parse_command(b"cwd").unwrap(), Action::CurrentDirectory);
    assert!(matches!(
        parse_command(b"pwd extra").unwrap_err(),
        CommandError::WrongNumberOfArguments { .. }
    ));
}
