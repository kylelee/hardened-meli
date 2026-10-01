/*
 * meli
 *
 * Copyright 2017-2018 Manos Pitsidianakis
 * Copyright 2026 Kyle Lee
 *
 * This file is part of meli.
 *
 * meli is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * meli is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with meli. If not, see <http://www.gnu.org/licenses/>.
 */

//! A parser module for user commands passed through
//! [`Command`](crate::types::UIMode::Command) mode.

use std::{borrow::Cow, str::FromStr};

use melib::{
    nom::{
        self,
        branch::alt,
        bytes::complete::{is_a, is_not, tag, take_until, take_while},
        character::complete::{digit1, not_line_ending},
        combinator::{map, map_res},
        error::Error as NomError,
        multi::separated_list1,
        sequence::{pair, preceded, separated_pair},
        IResult,
    },
    parser::BytesExt,
    SortField, SortOrder,
};

#[cfg(test)]
mod tests;

pub mod actions;
#[macro_use]
pub mod error;
#[macro_use]
pub mod argcheck;
pub mod history;
pub mod parser;
use actions::MailboxOperation;
use error::CommandError;
pub use parser::parse_command;

// Only `ComposeAction` itself is re-exported: glob-importing its variants
// would make the bare name `New` ambiguous with `TabAction::New` in every
// module that reaches the command module through the crate root's glob.
pub use crate::actions::{
    AccountAction::{self, *},
    Action::{self, *},
    ComposeAction, ComposerTabAction, FlagAction,
    ListingAction::{self, *},
    MailingListAction::{self, *},
    TabAction::{self, *},
    TagAction,
    ViewAction::{self, *},
};

/// Macro to create a const table with every command part that can be
/// auto-completed and its description
macro_rules! define_commands {
    ( [$({ tags: [$( $tags:literal),*], desc: $desc:literal, parser: $parser:path}),*]) => {
        pub const COMMAND_COMPLETION: &[(&str, &str, fn(&[u8]) -> IResult<&[u8], Result<Action, CommandError>>)] = &[$($( ($tags, $desc, $parser) ),*),* ];
    };
}

pub fn quoted_argument(input: &[u8]) -> IResult<&[u8], &str> {
    if input.is_empty() {
        return Err(nom::Err::Error(NomError {
            input,
            code: nom::error::ErrorKind::Tag,
        }));
    }

    if input[0] == b'"' {
        let mut i = 1;
        while i < input.len() {
            if input[i] == b'\"' && input[i - 1] != b'\\' {
                return Ok((&input[i + 1..], unsafe {
                    std::str::from_utf8_unchecked(&input[1..i])
                }));
            }
            i += 1;
        }
        Err(nom::Err::Error(NomError {
            input,
            code: nom::error::ErrorKind::Tag,
        }))
    } else {
        map_res(is_not(" "), std::str::from_utf8)(input)
    }
}

fn eof(input: &[u8]) -> IResult<&[u8], ()> {
    if input.is_empty() {
        Ok((input, ()))
    } else {
        Err(nom::Err::Error(NomError {
            input,
            code: nom::error::ErrorKind::Tag,
        }))
    }
}

define_commands!([
                 { tags: ["set", "set seen", "set unseen", "set plain", "set threaded", "set compact"],
                   desc: "set [seen/unseen], toggles message's Seen flag. set [plain/threaded/compact/conversations] changes the mail listing view",
                   parser: parser::set
                 },
                 { tags: ["delete"],
                   desc: "delete message",
                   parser: parser::delete_message
                 },
                 { tags: ["copyto", "moveto"],
                   desc: "copy/move message",
                   parser: parser::copymove
                 },
                { tags: ["import "],
                  desc: "import FILESYSTEM_PATH MAILBOX_PATH",
                  parser: parser::import
                },
                { tags: ["public-inbox import ", "public-inbox import-thread "],
                  desc: "public-inbox [import/import-thread] ACCOUNT_NAME MAILBOX_PATH MESSAGE_ID",
                  parser: parser::public_inbox_import
                },
                 { tags: ["close"],
                   desc: "close non-sticky tabs",
                   parser: parser::close
                 },
                 { tags: ["go"],
                   desc: "go <n>, switch to nth mailbox in this account",
                   parser: parser::goto
                 },
                 { tags: ["subsort"],
                   desc: "subsort [date/subject] [asc/desc], sorts first level replies in threads.",
                   parser: parser::subsort
                 },
                { tags: ["sort"],
                  desc: "sort [date/subject] [asc/desc], sorts threads.",
                  parser: parser::sort
                },
                { tags: ["sort"],
                  desc: "sort <column index> [asc/desc], sorts table columns.",
                  parser: parser::sort_column
                },
                { tags: ["toggle thread_snooze"],
                  desc: "turn off new notifications for this thread",
                  parser: parser::toggle
                },
                { tags: ["toggle theme"],
                  desc: "open the theme picker: live-preview themes with the arrow keys",
                  parser: parser::toggle
                },
                { tags: ["toggle mouse"],
                  desc: "toggle mouse support",
                  parser: parser::toggle
                },
                { tags: ["search"],
                  desc: "search <TERM>, searches list with given term",
                  parser: parser::search
                },
                { tags: ["clear-selection"],
                  desc: "clear-selection",
                  parser: parser::select
                },
                { tags: ["select"],
                  desc: "select <TERM>, selects envelopes matching with given term",
                  parser: parser::select
                },
                { tags: ["export-mbox "],
                  desc: "export-mbox PATH",
                  parser: parser::export_mbox
                },
                { tags: ["list-archive", "list-post", "list-unsubscribe", "list-"],
                  desc: "list-[unsubscribe/post/archive]",
                  parser: parser::mailinglist
                },
                { tags: ["setenv "],
                  desc: "setenv VAR=VALUE",
                  parser: parser::setenv
                },
                { tags: ["printenv "],
                  desc: "printenv VAR",
                  parser: parser::printenv
                },
                { tags: ["mailto "],
                  desc: "mailto MAILTO_ADDRESS",
                  parser: parser::mailto
                },
                { tags: ["new-mail"],
                  desc: "open a new composer with an empty message",
                  parser: parser::new_mail
                },
                { tags: ["reply"],
                  desc: "reply to the sender of the selected message",
                  parser: parser::reply
                },
                { tags: ["reply all"],
                  desc: "reply to the sender and all recipients",
                  parser: parser::reply_all
                },
                { tags: ["reply author"],
                  desc: "reply to the author of the selected message only",
                  parser: parser::reply_author
                },
                { tags: ["forward"],
                  desc: "forward the selected message, choosing inline or attachment",
                  parser: parser::forward
                },
                { tags: ["forward inline"],
                  desc: "forward the selected message inline",
                  parser: parser::forward_inline
                },
                { tags: ["forward attachment"],
                  desc: "forward the selected message as an attachment",
                  parser: parser::forward_attachment
                },
                /* Pipe pager contents to binary */
                { tags: ["pipe "],
                  desc: "pipe EXECUTABLE ARGS",
                  parser: parser::pipe
                },
                /* Filter pager contents through binary */
                { tags: ["filter "],
                  desc: "filter EXECUTABLE ARGS",
                  parser: parser::filter
                },
                { tags: ["add-attachment ", "add-attachment-file-picker "],
                  desc: "add-attachment PATH",
                  parser: parser::add_attachment
                },
                { tags: ["remove-attachment "],
                  desc: "remove-attachment INDEX",
                  parser: parser::remove_attachment
                },
                { tags: ["save-draft"],
                  desc: "save draft",
                  parser: parser::save_draft
                },
                { tags: ["discard-draft"],
                  desc: "discard draft",
                  parser: parser::discard_draft
                },
                { tags: ["toggle sign "],
                  desc: "switch between sign/unsign for this draft",
                  parser: parser::toggle
                },
                { tags: ["toggle encrypt"],
                  desc: "toggle encryption for this draft",
                  parser: parser::toggle
                },
                { tags: ["create-mailbox "],
                  desc: "create-mailbox ACCOUNT MAILBOX_PATH",
                  parser: parser::create_mailbox
                },
                { tags: ["subscribe-mailbox "],
                  desc: "subscribe-mailbox ACCOUNT MAILBOX_PATH",
                  parser: parser::sub_mailbox
                },
                { tags: ["unsubscribe-mailbox "],
                  desc: "unsubscribe-mailbox ACCOUNT MAILBOX_PATH",
                  parser: parser::unsub_mailbox
                },
                { tags: ["rename-mailbox "],
                  desc: "rename-mailbox ACCOUNT MAILBOX_PATH_SRC MAILBOX_PATH_DEST",
                  parser: parser::rename_mailbox
                },
                { tags: ["delete-mailbox "],
                  desc: "delete-mailbox ACCOUNT MAILBOX_PATH",
                  parser: parser::delete_mailbox
                },
                { tags: ["reindex "],
                  desc: "reindex ACCOUNT, rebuild account cache in the background",
                  parser: parser::reindex
                },
                { tags: ["open-in-tab"],
                  desc: "opens envelope view in new tab",
                  parser: parser::open_in_new_tab
                },
                { tags: ["open"],
                  desc: "open the mail or thread under the cursor",
                  parser: parser::open
                },
                { tags: ["refresh"],
                  desc: "refresh the current mailbox",
                  parser: parser::refresh
                },
                { tags: ["save-attachment ", "save-attachment-picker "],
                  desc: "save-attachment INDEX PATH",
                  parser: parser::save_attachment
                },
                { tags: ["save-all-attachment"],
                  desc: "save-all-attachment",
                  parser: parser::save_all_attachment
                },
                { tags: ["export-mail "],
                  desc: "export-mail PATH",
                  parser: parser::export_mail
                },
                { tags: ["export-thread "],
                  desc: "export-thread PATH",
                  parser: parser::export_thread
                },
                { tags: ["export-thread-mbox "],
                  desc: "export-thread-mbox PATH",
                  parser: parser::export_thread_mbox
                },
                { tags: ["add-addresses-to-contacts "],
                  desc: "add-addresses-to-contacts",
                  parser: parser::add_addresses_to_contacts
                },
                { tags: ["tag", "tag add", "tag remove"],
                   desc: "tag [add/remove], edits message's tags.",
                   parser: parser::_tag
                },
                { tags: ["flag", "flag set", "flag unset"],
                   desc: "flag [set/unset] NAME, sets or unsets a message flag: seen, flagged, draft, passed, replied or trash",
                   parser: parser::flag
                },
                { tags: ["print "],
                  desc: "print ACCOUNT SETTING",
                  parser: parser::print_account_setting
                },
                { tags: ["print "],
                  desc: "print SETTING",
                  parser: parser::print_setting
                },
                { tags: ["toggle mouse"],
                  desc: "toggle mouse support",
                  parser: parser::toggle
                },
                { tags: ["manage-mailboxes"],
                  desc: "view and manage mailbox preferences",
                  parser: parser::manage_mailboxes
                },
                { tags: ["man"],
                  desc: "read documentation",
                  parser: parser::view_manpage
                },
                { tags: ["manage-jobs"],
                  desc: "view and manage jobs",
                  parser: parser::manage_jobs
                },
                { tags: ["quit"],
                  desc: "quit meli",
                  parser: parser::quit
                },
                { tags: ["reload-config"],
                  desc: "reload configuration file",
                  parser: parser::reload_config
                }
]);
