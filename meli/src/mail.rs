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

//! Entities that handle Mail specific functions.

use std::{future::Future, pin::Pin};

use indexmap::IndexMap;
use melib::{
    backends::{AccountHash, Mailbox, MailboxHash},
    email::{attachment_types::*, attachments::*},
    text::{TextProcessing, Truncate},
    thread::ThreadNodeHash,
};
use uuid::Uuid;

use super::*;

pub type AttachmentBoxFuture = Pin<Box<dyn Future<Output = Result<AttachmentBuilder>> + Send>>;
pub type AttachmentFilterBox = Box<dyn FnOnce(AttachmentBuilder) -> AttachmentBoxFuture + Send>;

/// The fixed 30/70 two-pane split every layout shares: the left pane
/// (listing grid or thread list) keeps `width * 3 / 10` columns, one gap
/// column separates the panes, the right pane (mail view) takes the rest.
/// All layouts — the listing components' grid/view split, `Listing::draw`'s
/// view placement, and the thread view's own list/mail split — must go
/// through this so the panes line up column-for-column.
pub(crate) fn pane_split(area: Area) -> (Area, Area) {
    let list_cols = area.width() * 3 / 10;
    (area.take_cols(list_cols), area.skip_cols(list_cols + 1))
}

/// The single gap column between the two panes of [`pane_split`].
pub(crate) fn pane_gap(area: Area) -> Area {
    area.nth_col(area.width() * 3 / 10)
}

pub mod listing;
pub use crate::listing::*;
pub mod view;
pub use crate::view::*;
pub mod compose;
pub use self::compose::*;

pub mod pgp;

pub mod status;
pub use self::status::*;
