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

use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    convert::TryFrom,
    fs::File,
    future::Future,
    io::{BufWriter, Write},
    ops::{Deref, DerefMut},
    pin::Pin,
};

use futures::future::try_join_all;
use melib::{
    backends::EnvelopeHashBatch, mbox::MboxMetadata, utils::datetime, Flag, FlagOp,
    ShellExpandTrait, Threads, UnixTimestamp,
};
use smallvec::SmallVec;

use super::*;
use crate::{
    accounts::{JobRequest, MailboxStatus, SearchResult},
    components::ExtendShortcutsMaps,
    jobs::IsAsync,
    terminal::{
        draw_rounded_frame, frame_flush_areas,
        ratatui_bridge::{area_to_rect, rect_to_area},
    },
};
use ratatui::layout::{Constraint, Layout};

pub const DEFAULT_ATTACHMENT_FLAG: &str = concat!("📎", emoji_text_presentation_selector!());

/// The default `listing.selected_flag`.
///
/// The `☑️` literal already carries the emoji-presentation selector
/// (`U+FE0F`) — which is also the documented default for
/// `listing.selected_flag` — so appending the *text*-presentation
/// selector as well produced a self-contradictory `☑ U+FE0F U+FE0E`
/// sequence that terminals resolve differently (and the flush layer emits
/// both, `FORCE_TEXT` before `FORCE_EMOJI`). Keep emoji presentation as
/// the only selector: it matches the documented default, the two-column
/// grid accounting of `grapheme_width`, and the golden corpus.
pub const DEFAULT_SELECTED_FLAG: &str = "☑️";
pub const DEFAULT_UNSEEN_FLAG: &str = concat!("●", emoji_text_presentation_selector!());
pub const DEFAULT_SNOOZED_FLAG: &str = concat!("💤", emoji_text_presentation_selector!());
pub const DEFAULT_HIGHLIGHT_SELF_FLAG: &str = concat!("✸", emoji_text_presentation_selector!());

/// Tell the user that a search only returned in-memory fallback matches
/// because the authoritative backend (remote server or sqlite3 index)
/// failed. Shared by every listing component that applies a
/// [`SearchResult`].
fn notify_if_search_degraded(context: &mut Context, result: &SearchResult) {
    if result.degraded {
        context.replies.push_back(UIEvent::Notification {
            title: None,
            source: None,
            body: "Server search failed; showing local matches only. Results may be incomplete."
                .into(),
            kind: Some(crate::types::NotificationType::Info),
        });
    }
}

#[derive(Debug, Default)]
pub struct RowsState<T> {
    pub selection: HashMap<EnvelopeHash, bool>,
    pub row_updates: SmallVec<[EnvelopeHash; 8]>,
    // [ref:FIXME]: env vec should have at least one element guaranteed
    pub thread_to_env: HashMap<ThreadHash, SmallVec<[EnvelopeHash; 8]>>,
    pub env_to_thread: HashMap<EnvelopeHash, ThreadHash>,
    pub thread_order: HashMap<ThreadHash, usize>,
    pub env_order: HashMap<EnvelopeHash, usize>,
    #[allow(clippy::type_complexity)]
    pub entries: Vec<(T, EntryStrings)>,
    pub all_threads: HashSet<ThreadHash>,
    pub all_envelopes: HashSet<EnvelopeHash>,
    pub row_attr_cache: HashMap<usize, ThemeAttribute>,
}

impl<T> RowsState<T> {
    #[inline(always)]
    #[must_use]
    pub fn clear(&mut self, take_selection: bool) -> Option<HashMap<EnvelopeHash, bool>> {
        self.row_updates.clear();
        self.thread_to_env.clear();
        self.env_to_thread.clear();
        self.thread_order.clear();
        self.env_order.clear();
        self.entries.clear();
        self.all_threads.clear();
        self.all_envelopes.clear();
        self.row_attr_cache.clear();
        if take_selection {
            Some(std::mem::take(&mut self.selection))
        } else {
            self.selection.clear();
            None
        }
    }

    #[inline(always)]
    pub fn restore_selection(&mut self, previous_selection: Option<HashMap<EnvelopeHash, bool>>) {
        if let Some(prev) = previous_selection {
            for (h, b) in prev {
                if !b || !self.selection.contains_key(&h) {
                    continue;
                }
                self.row_update_add_envelope(h);
                self.selection.insert(h, true);
            }
        }
    }

    #[inline(always)]
    pub fn is_thread_selected(&self, thread: ThreadHash) -> bool {
        debug_assert!(self.all_threads.contains(&thread));
        debug_assert!(self.thread_order.contains_key(&thread));
        debug_assert!(self.thread_to_env.contains_key(&thread));
        self.thread_to_env
            .get(&thread)
            .iter()
            .flat_map(|v| v.iter())
            .any(|env_hash| self.selection[env_hash])
    }

    #[inline(always)]
    pub fn insert_thread(
        &mut self,
        thread: ThreadHash,
        metadata: T,
        mut env_hashes: SmallVec<[EnvelopeHash; 8]>,
        entry_strings: EntryStrings,
    ) {
        env_hashes.dedup();
        env_hashes.retain(|h| !self.all_envelopes.contains(h));
        if env_hashes.is_empty() {
            return;
        }
        let index = self.entries.len();
        for &env_hash in &env_hashes {
            self.selection.insert(env_hash, false);
            self.env_to_thread.insert(env_hash, thread);
            self.env_order.insert(env_hash, index);
            self.all_envelopes.insert(env_hash);
        }
        if self.all_threads.insert(thread) {
            self.thread_order.insert(thread, index);
            self.thread_to_env.insert(thread, env_hashes);
        } else {
            self.thread_to_env
                .entry(thread)
                .or_default()
                .extend_from_slice(&env_hashes);
        }
        self.entries.push((metadata, entry_strings));
    }

    #[inline(always)]
    pub fn row_update_add_thread(&mut self, thread: ThreadHash) {
        let env_hashes = self.thread_to_env.entry(thread).or_default().clone();
        for env_hash in env_hashes {
            self.row_updates.push(env_hash);
        }
    }

    #[inline(always)]
    pub fn row_update_add_envelope(&mut self, env_hash: EnvelopeHash) {
        self.row_updates.push(env_hash);
    }

    #[inline(always)]
    pub fn contains_thread(&self, thread: ThreadHash) -> bool {
        debug_assert_eq!(
            self.all_threads.contains(&thread),
            self.thread_order.contains_key(&thread)
        );
        debug_assert_eq!(
            self.thread_order.contains_key(&thread),
            self.thread_to_env.contains_key(&thread)
        );
        self.thread_order.contains_key(&thread)
    }

    #[inline(always)]
    pub fn contains_env(&self, env_hash: EnvelopeHash) -> bool {
        self.all_envelopes.contains(&env_hash)
    }

    #[inline(always)]
    pub fn update_selection_with_thread(
        &mut self,
        thread: ThreadHash,
        mut cl: impl FnMut(&mut bool),
    ) {
        let env_hashes = self.thread_to_env.entry(thread).or_default().clone();
        for env_hash in env_hashes {
            self.selection.entry(env_hash).and_modify(&mut cl);
            self.row_updates.push(env_hash);
        }
    }

    #[inline(always)]
    pub fn update_selection_with_env(
        &mut self,
        env_hash: EnvelopeHash,
        mut cl: impl FnMut(&mut bool),
    ) {
        self.selection.entry(env_hash).and_modify(&mut cl);
        self.row_updates.push(env_hash);
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline(always)]
    pub fn clear_selection(&mut self) {
        for (k, v) in self.selection.iter_mut() {
            if *v {
                *v = false;
                self.row_updates.push(*k);
            }
        }
    }

    pub fn rename_env(&mut self, old_hash: EnvelopeHash, new_hash: EnvelopeHash) {
        self.row_updates.push(new_hash);
        if let Some(row) = self.env_order.remove(&old_hash) {
            self.env_order.insert(new_hash, row);
        }
        if let Some(thread) = self.env_to_thread.remove(&old_hash) {
            self.env_to_thread.insert(new_hash, thread);
            self.thread_to_env
                .entry(thread)
                .or_default()
                .retain(|h| *h != old_hash);
            self.thread_to_env.entry(thread).or_default().push(new_hash);
        }
        let selection_status = self.selection.remove(&old_hash).unwrap_or(false);
        self.selection.insert(new_hash, selection_status);
        self.all_envelopes.remove(&old_hash);
        self.all_envelopes.insert(new_hash);
    }
}

mod conversations;
pub use self::conversations::*;

mod compact;
pub use self::compact::*;

mod thread;
pub use self::thread::*;

mod plain;
pub use self::plain::*;

mod offline;
pub use self::offline::*;

#[derive(Clone, Copy, Debug)]
pub enum Focus {
    None,
    Entry,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Modifier {
    #[default]
    SymmetricDifference,
    Union,
    Difference,
    Intersection,
}

impl std::fmt::Display for Modifier {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::SymmetricDifference => write!(fmt, "><"),
            Self::Union => write!(fmt, "+"),
            Self::Difference => write!(fmt, "-"),
            Self::Intersection => write!(fmt, "*"),
        }
    }
}

#[derive(Debug, Default)]
/// Save theme colors to avoid looking them up again and again from settings
pub struct ColorCache {
    pub theme_default: ThemeAttribute,

    // Single base attribute for listing rows: the former even/odd zebra
    // striping was removed, so every row shares this by default.
    pub base: ThemeAttribute,
    pub unseen: ThemeAttribute,
    pub highlighted: ThemeAttribute,
    pub selected: ThemeAttribute,
    pub highlighted_selected: ThemeAttribute,
    pub tag_default: ThemeAttribute,
    pub highlight_self: ThemeAttribute,

    // Conversations
    pub subject: ThemeAttribute,
    pub from: ThemeAttribute,
    pub date: ThemeAttribute,
}

impl ColorCache {
    pub fn new(context: &Context, style: IndexStyle) -> Self {
        let default = Self {
            theme_default: crate::conf::value(context, "theme_default"),
            tag_default: crate::conf::value(context, "mail.listing.tag_default"),
            highlight_self: crate::conf::value(context, "mail.listing.highlight_self"),
            ..Self::default()
        };
        let mut ret = match style {
            IndexStyle::Plain => Self {
                base: crate::conf::value(context, "mail.listing.plain"),
                unseen: crate::conf::value(context, "mail.listing.plain.unseen"),
                highlighted: crate::conf::value(context, "mail.listing.plain.highlighted"),
                selected: crate::conf::value(context, "mail.listing.plain.selected"),
                highlighted_selected: crate::conf::value(
                    context,
                    "mail.listing.plain.highlighted_selected",
                ),
                ..default
            },
            IndexStyle::Threaded => Self {
                base: crate::conf::value(context, "mail.listing.plain"),
                unseen: crate::conf::value(context, "mail.listing.plain.unseen"),
                highlighted: crate::conf::value(context, "mail.listing.plain.highlighted"),
                selected: crate::conf::value(context, "mail.listing.plain.selected"),
                highlighted_selected: crate::conf::value(
                    context,
                    "mail.listing.plain.highlighted_selected",
                ),
                ..default
            },
            IndexStyle::Compact => Self {
                base: crate::conf::value(context, "mail.listing.compact"),
                unseen: crate::conf::value(context, "mail.listing.compact.unseen"),
                highlighted: crate::conf::value(context, "mail.listing.compact.highlighted"),
                selected: crate::conf::value(context, "mail.listing.compact.selected"),
                highlighted_selected: crate::conf::value(
                    context,
                    "mail.listing.compact.highlighted_selected",
                ),
                ..default
            },
            IndexStyle::Conversations => Self {
                subject: crate::conf::value(context, "mail.listing.conversations.subject"),
                from: crate::conf::value(context, "mail.listing.conversations.from"),
                date: crate::conf::value(context, "mail.listing.conversations.date"),
                selected: crate::conf::value(context, "mail.listing.conversations.selected"),
                unseen: crate::conf::value(context, "mail.listing.conversations.unseen"),
                highlighted: crate::conf::value(context, "mail.listing.conversations.highlighted"),
                highlighted_selected: crate::conf::value(
                    context,
                    "mail.listing.conversations.highlighted_selected",
                ),
                base: crate::conf::value(context, "mail.listing.conversations"),
                ..default
            },
        };
        if !context.settings.terminal.use_color() {
            ret.highlighted.attrs |= Attr::REVERSE;
            ret.tag_default.attrs |= Attr::REVERSE;
            ret.highlight_self.attrs |= Attr::REVERSE;
            ret.highlighted_selected.attrs |= Attr::REVERSE | Attr::DIM;
        }
        ret
    }
}

#[derive(Debug)]
pub struct EntryStrings {
    pub date: DateString,
    pub subject: SubjectString,
    pub flag: FlagString,
    pub from: FromString,
    pub tags: TagString,
    pub unseen: bool,
    pub highlight_self: bool,
}

#[macro_export]
macro_rules! digits_of_num {
    ($num:expr) => {{
        const GUESS: [usize; 65] = [
            1, 0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 5, 5, 5, 6, 6, 6, 6, 7, 7, 7, 8, 8,
            8, 9, 9, 9, 9, 10, 10, 10, 11, 11, 11, 12, 12, 12, 12, 13, 13, 13, 14, 14, 14, 15, 15,
            15, 15, 16, 16, 16, 17, 17, 17, 18, 18, 18, 18, 19,
        ];
        const TENS: [usize; 20] = [
            1,
            10,
            100,
            1000,
            10000,
            100000,
            1000000,
            10000000,
            100000000,
            1000000000,
            10000000000,
            100000000000,
            1000000000000,
            10000000000000,
            100000000000000,
            1000000000000000,
            10000000000000000,
            100000000000000000,
            1000000000000000000,
            10000000000000000000,
        ];
        const SIZE_IN_BITS: usize = std::mem::size_of::<usize>() * 8;

        let leading_zeros = $num.leading_zeros() as usize;
        let base_two_digits: usize = SIZE_IN_BITS - leading_zeros;
        let x = GUESS[base_two_digits];
        x + if $num >= TENS[x] { 1 } else { 0 }
    }};
}

macro_rules! column_str {
    (
        struct $name:ident($($t:ty),+)) => {
        #[derive(Debug)]
        pub struct $name($(pub $t),+);

        impl Deref for $name {
            type Target = String;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }
        impl DerefMut for $name {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }
    };
}

column_str!(struct DateString(String));
column_str!(struct FromString(String));
column_str!(struct SubjectString(String));
column_str!(struct FlagString(String));
column_str!(struct TagString(String, SmallVec<[Option<Color>; 8]>));

/// Deterministic, envelope-level check whether any envelope in the thread
/// rooted at `thread_hash` carries an attachment.
///
/// Do **not** use `Thread::has_attachments()` for this decision: that counter
/// is aggregated exactly once, when the thread is inserted into the [`Threads`]
/// tree, and therefore goes stale after a refresh or rebuild (the paperclip
/// then "sometimes shows, later stops showing"). This walks the thread's
/// `message()` nodes instead and consults each envelope's own parse-time
/// cached `has_attachments`, the same source the plain listing relies on, so
/// the result tracks the envelopes currently in the collection.
pub(crate) fn thread_has_attachments(
    threads: &Threads,
    envelopes: &HashMap<EnvelopeHash, Envelope>,
    thread_hash: ThreadHash,
) -> bool {
    threads.thread_iter(thread_hash).any(|(_, h)| {
        threads.thread_nodes()[&h]
            .message()
            .and_then(|env_hash| envelopes.get(&env_hash))
            .is_some_and(Envelope::has_attachments)
    })
}

#[cfg(test)]
mod thread_has_attachments_tests {
    use std::{
        collections::HashMap,
        sync::{Arc, RwLock},
    };

    use melib::{Envelope, EnvelopeHash, Threads};

    use super::thread_has_attachments;

    /// The old `Thread::has_attachments()` counter and the new helper must
    /// diverge once the counter is stale: a `Thread` whose counter says zero
    /// while one of its envelopes still reports an attachment must keep the
    /// marker. `Thread::attachments` is a public field reachable through
    /// `Threads::thread_ref_mut`, so the stale state a refresh/rebuild leaves
    /// behind is reproducible from meli without touching melib.
    #[test]
    fn stale_thread_counter_does_not_hide_the_attachment() {
        let raw = b"From: Carol Example <carol@example.org>\r\nTo: Bob Example <bob@example.org>\r\nSubject: attached mail\r\nMessage-ID: <stale-counter@x.example>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"bnd\"\r\n\r\n--bnd\nContent-Type: text/plain; charset=utf-8\n\nbody\n--bnd\nContent-Type: application/pdf; name=\"doc.pdf\"\nContent-Disposition: attachment; filename=\"doc.pdf\"\n\n%PDF-1.4 fake\n--bnd--\n";
        let envelope = Envelope::from_bytes(raw, None).unwrap();
        assert!(
            envelope.has_attachments(),
            "fixture must carry an attachment"
        );
        let env_hash = envelope.hash();
        let envelopes: Arc<RwLock<HashMap<EnvelopeHash, Envelope>>> =
            Arc::new(RwLock::new(HashMap::from([(env_hash, envelope)])));
        let mut threads = Threads::new(1);
        threads.insert(&envelopes, env_hash);
        let thread_hash = threads.envelope_to_thread[&env_hash];
        // Simulate the post-refresh stale counter: inserted with an
        // attachment, then zeroed without rebuilding the thread tree.
        threads.thread_ref_mut(thread_hash).attachments = 0;
        assert!(
            !threads.thread_ref(thread_hash).has_attachments(),
            "the counter is deliberately stale for this regression test"
        );
        let envelopes_lck = envelopes.read().unwrap();
        assert!(
            thread_has_attachments(&threads, &envelopes_lck, thread_hash),
            "the envelope-level helper must still report the attachment"
        );
        assert_ne!(
            threads.thread_ref(thread_hash).has_attachments(),
            thread_has_attachments(&threads, &envelopes_lck, thread_hash),
            "the stale counter and the deterministic helper must diverge"
        );
    }
}

impl FlagString {
    pub(self) fn new(
        f: Flag,
        is_selected: bool,
        is_snoozed: bool,
        is_unseen: bool,
        has_attachments: bool,
        context: &Context,
        coordinates: (AccountHash, MailboxHash),
    ) -> Self {
        Self(format!(
            "{flag_passed}{flag_replied}{flag_seen}{flag_trashed}{flag_draft}{flag_flagged} \
             {selected}{snoozed}{unseen}{attachments}{whitespace}",
            flag_passed = if f.contains(Flag::PASSED) { "P" } else { "" },
            flag_replied = if f.contains(Flag::REPLIED) { "R" } else { "" },
            flag_seen = if f.contains(Flag::SEEN) { "S" } else { "" },
            flag_trashed = if f.contains(Flag::TRASHED) { "T" } else { "" },
            flag_draft = if f.contains(Flag::DRAFT) { "D" } else { "" },
            flag_flagged = if f.contains(Flag::FLAGGED) { "F" } else { "" },
            selected = if is_selected {
                mailbox_settings!(context[coordinates.0][&coordinates.1].listing.selected_flag)
                    .as_ref()
                    .map(|s| s.as_str())
                    .unwrap_or(DEFAULT_SELECTED_FLAG)
            } else {
                ""
            },
            snoozed = if is_snoozed {
                mailbox_settings!(
                    context[coordinates.0][&coordinates.1]
                        .listing
                        .thread_snoozed_flag
                )
                .as_ref()
                .map(|s| s.as_str())
                .unwrap_or(DEFAULT_SNOOZED_FLAG)
            } else {
                ""
            },
            unseen = if is_unseen {
                mailbox_settings!(context[coordinates.0][&coordinates.1].listing.unseen_flag)
                    .as_ref()
                    .map(|s| s.as_str())
                    .unwrap_or(DEFAULT_UNSEEN_FLAG)
            } else {
                ""
            },
            attachments = if has_attachments {
                mailbox_settings!(
                    context[coordinates.0][&coordinates.1]
                        .listing
                        .attachment_flag
                )
                .as_ref()
                .map(|s| s.as_str())
                .unwrap_or(DEFAULT_ATTACHMENT_FLAG)
            } else {
                ""
            },
            whitespace = if is_selected || is_unseen || is_snoozed || has_attachments {
                " "
            } else {
                ""
            },
        ))
    }
}

#[derive(Clone, Copy, Debug)]
struct MailboxMenuEntry {
    depth: usize,
    indentation: u32,
    has_sibling: bool,
    visible: bool,
    collapsed: bool,
    mailbox_hash: MailboxHash,
    index_style: Option<IndexStyle>,
}

#[derive(Debug)]
struct AccountMenuEntry {
    name: String,
    hash: AccountHash,
    index: usize,
    entries: SmallVec<[MailboxMenuEntry; 16]>,
}

impl AccountMenuEntry {
    fn entry_by_hash(&self, needle: MailboxHash) -> Option<usize> {
        self.entries.iter().enumerate().find_map(|(i, e)| {
            if e.mailbox_hash == needle {
                Some(i)
            } else {
                None
            }
        })
    }

    /// Visual height / rows of account entry in the sidebar menu.
    fn height(&self) -> usize {
        let mut ctr = 1;
        let mut is_collapsed = false;
        let mut collapsed_depth = 0;
        for e in &self.entries {
            match (is_collapsed, e.collapsed) {
                (true, _) if e.depth > collapsed_depth => continue,
                (true, _) => {
                    is_collapsed = false;
                }
                (false, true) => {
                    is_collapsed = true;
                    collapsed_depth = e.depth;
                }
                (false, false) => {}
            }
            ctr += 1;
        }
        ctr
    }

    /// Visual offset of cursor taking into account collapsed mailboxes.
    fn cursor_y_offset(&self, cursor: usize) -> usize {
        if cursor == 0 {
            return cursor;
        }
        let mut ctr = 1;
        let mut is_collapsed = false;
        let mut collapsed_depth = 0;
        for (i, e) in self.entries.iter().enumerate() {
            if cursor == i {
                return ctr;
            }
            match (is_collapsed, e.collapsed) {
                (true, _) if e.depth > collapsed_depth => continue,
                (true, _) => {
                    is_collapsed = false;
                }
                (false, true) => {
                    is_collapsed = true;
                    collapsed_depth = e.depth;
                }
                (false, false) => {}
            }
            ctr += 1;
        }
        ctr
    }
}

pub trait MailListingTrait: ListingTrait {
    fn as_component(&self) -> &dyn Component
    where
        Self: Sized,
    {
        self
    }

    fn as_component_mut(&mut self) -> &mut dyn Component
    where
        Self: Sized,
    {
        self
    }

    fn perform_action(
        &mut self,
        context: &mut Context,
        envs_to_set: SmallVec<[EnvelopeHash; 8]>,
        a: &ListingAction,
    ) {
        fn inner(
            context: &mut Context,
            envs_to_set: SmallVec<[EnvelopeHash; 8]>,
            account_hash: AccountHash,
            mailbox_hash: MailboxHash,
            a: &ListingAction,
        ) {
            let env_hashes = if let Ok(batch) = EnvelopeHashBatch::try_from(envs_to_set.as_slice())
            {
                batch
            } else {
                return;
            };
            let account = &mut context.accounts[&account_hash];
            match a {
                ListingAction::Flag(FlagAction::Set(Flag::SEEN)) | ListingAction::SetSeen => {
                    if let Err(err) =
                        account.set_flags(env_hashes, mailbox_hash, vec![FlagOp::Set(Flag::SEEN)])
                    {
                        context.replies.push_back(UIEvent::Notification {
                            title: Some("Could not set seen flag".into()),
                            source: None,
                            body: err.to_string().into(),
                            kind: Some(NotificationType::Error(err.kind)),
                        });
                    }
                }
                ListingAction::Flag(FlagAction::Unset(Flag::SEEN)) | ListingAction::SetUnseen => {
                    if let Err(err) =
                        account.set_flags(env_hashes, mailbox_hash, vec![FlagOp::UnSet(Flag::SEEN)])
                    {
                        context.replies.push_back(UIEvent::Notification {
                            title: Some("Could not unset seen flag".into()),
                            source: None,
                            body: err.to_string().into(),
                            kind: Some(NotificationType::Error(err.kind)),
                        });
                    }
                }
                ListingAction::Flag(FlagAction::Set(flag)) => {
                    if let Err(err) =
                        account.set_flags(env_hashes, mailbox_hash, vec![FlagOp::Set(*flag)])
                    {
                        context.replies.push_back(UIEvent::Notification {
                            title: Some("Could not set flag".into()),
                            source: None,
                            body: err.to_string().into(),
                            kind: Some(NotificationType::Error(err.kind)),
                        });
                    }
                }
                ListingAction::Flag(FlagAction::Unset(flag)) => {
                    if let Err(err) =
                        account.set_flags(env_hashes, mailbox_hash, vec![FlagOp::UnSet(*flag)])
                    {
                        context.replies.push_back(UIEvent::Notification {
                            title: Some("Could not unset flag".into()),
                            source: None,
                            body: err.to_string().into(),
                            kind: Some(NotificationType::Error(err.kind)),
                        });
                    }
                }
                ListingAction::Tag(TagAction::Add(ref tag_str)) => {
                    if let Err(err) = account.set_flags(
                        env_hashes,
                        mailbox_hash,
                        vec![FlagOp::SetTag(tag_str.into())],
                    ) {
                        context.replies.push_back(UIEvent::Notification {
                            title: Some("Could not add tag".into()),
                            source: None,
                            body: err.to_string().into(),
                            kind: Some(NotificationType::Error(err.kind)),
                        });
                    }
                }
                ListingAction::Tag(TagAction::Remove(ref tag_str)) => {
                    if let Err(err) = account.set_flags(
                        env_hashes,
                        mailbox_hash,
                        vec![FlagOp::UnSetTag(tag_str.into())],
                    ) {
                        context.replies.push_back(UIEvent::Notification {
                            title: Some("Could not remove tag".into()),
                            source: None,
                            body: err.to_string().into(),
                            kind: Some(NotificationType::Error(err.kind)),
                        });
                    }
                }
                ListingAction::SendToTrash => {
                    use melib::backends::SpecialUsageMailbox;

                    let Some(trash_mbox_hash) = account
                        .special_use_mailbox(SpecialUsageMailbox::Trash)
                        .or_else(|| account.special_use_mailbox(SpecialUsageMailbox::Junk))
                    else {
                        context.replies.push_back(UIEvent::Notification {
                            title: Some("Could not send mail to trash".into()),
                            source: None,
                            body: "Cannot send mail to trash because no Trash folder is \
                                   configured."
                                .into(),
                            kind: Some(NotificationType::Info),
                        });
                        return;
                    };
                    let job = account.backend.lock().unwrap().copy_messages(
                        env_hashes,
                        mailbox_hash,
                        trash_mbox_hash,
                        /* move? */ true,
                    );
                    match job {
                        Err(err) => {
                            context.replies.push_back(UIEvent::Notification {
                                title: Some("Could not send mail to trash".into()),
                                source: None,
                                body: err.to_string().into(),
                                kind: Some(NotificationType::Error(err.kind)),
                            });
                        }
                        Ok(fut) => {
                            let handle = account.main_loop_handler.job_executor.spawn(
                                "move-to-trash".into(),
                                fut,
                                account.is_async(),
                            );
                            account.insert_job(
                                handle.job_id,
                                JobRequest::Generic {
                                    name: "taking out the trash".into(),
                                    handle,
                                    on_finish: None,
                                    log_level: LogLevel::INFO,
                                },
                            );
                        }
                    }
                }
                ListingAction::Delete => {
                    let job = account
                        .backend
                        .lock()
                        .unwrap()
                        .delete_messages(env_hashes.clone(), mailbox_hash);
                    match job {
                        Err(err) => {
                            context.replies.push_back(UIEvent::Notification {
                                title: Some("Could not delete mail".into()),
                                source: None,
                                body: err.to_string().into(),
                                kind: Some(NotificationType::Error(err.kind)),
                            });
                        }
                        Ok(fut) => {
                            let handle = account.main_loop_handler.job_executor.spawn(
                                "delete".into(),
                                fut,
                                account.is_async(),
                            );
                            account.insert_job(
                                handle.job_id,
                                JobRequest::DeleteMessages { env_hashes, handle },
                            );
                        }
                    }
                }
                ListingAction::CopyTo(ref mailbox_path) => {
                    match account.mailbox_by_path(mailbox_path).and_then(
                        |destination_mailbox_hash| {
                            account.backend.lock().unwrap().copy_messages(
                                env_hashes,
                                mailbox_hash,
                                destination_mailbox_hash,
                                /* move? */ false,
                            )
                        },
                    ) {
                        Err(err) => {
                            context.replies.push_back(UIEvent::Notification {
                                title: Some("Could not copy mail".into()),
                                source: None,
                                body: err.to_string().into(),
                                kind: Some(NotificationType::Error(err.kind)),
                            });
                        }
                        Ok(fut) => {
                            let handle = account.main_loop_handler.job_executor.spawn(
                                "copy-to-mailbox".into(),
                                fut,
                                account.is_async(),
                            );
                            account.insert_job(
                                handle.job_id,
                                JobRequest::Generic {
                                    name: "message copying".into(),
                                    handle,
                                    on_finish: None,
                                    log_level: LogLevel::INFO,
                                },
                            );
                        }
                    }
                }
                ListingAction::CopyToOtherAccount(ref _account_name, ref _mailbox_path) => {
                    context.replies.push_back(UIEvent::Notification {
                        title: Some("Could not copy mail".into()),
                        source: None,
                        body: "Copying to another account is currently unimplemented".into(),
                        kind: Some(NotificationType::Error(ErrorKind::NotImplemented)),
                    });
                }
                ListingAction::MoveTo(ref mailbox_path) => {
                    match account.mailbox_by_path(mailbox_path).and_then(
                        |destination_mailbox_hash| {
                            account.backend.lock().unwrap().copy_messages(
                                env_hashes,
                                mailbox_hash,
                                destination_mailbox_hash,
                                /* move? */ true,
                            )
                        },
                    ) {
                        Err(err) => {
                            context.replies.push_back(UIEvent::Notification {
                                title: Some("Could not move mail".into()),
                                source: None,
                                body: err.to_string().into(),
                                kind: Some(NotificationType::Error(err.kind)),
                            });
                        }
                        Ok(fut) => {
                            let handle = account.main_loop_handler.job_executor.spawn(
                                "move-to-mailbox".into(),
                                fut,
                                account.is_async(),
                            );
                            account.insert_job(
                                handle.job_id,
                                JobRequest::Generic {
                                    name: "message moving".into(),
                                    handle,
                                    on_finish: None,
                                    log_level: LogLevel::INFO,
                                },
                            );
                        }
                    }
                }
                ListingAction::ExportMbox(format, ref path) => {
                    let futures: Result<Vec<_>> = envs_to_set
                        .iter()
                        .map(|&env_hash| account.envelope_bytes_by_hash(env_hash))
                        .collect::<Result<Vec<_>>>();
                    let mut path = path.to_path_buf();
                    if path.is_relative() {
                        path = context.current_dir().join(&path);
                    }
                    path = path.expand();
                    let account = &mut context.accounts[&account_hash];
                    let format = (*format).unwrap_or_default();
                    let collection = account.collection.clone();
                    let (sender, mut receiver) = crate::jobs::oneshot::channel();
                    let fut: Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>> =
                        Box::pin(async move {
                            let cl = async move {
                                // fully capture variables.
                                let _ = (&envs_to_set, &collection);
                                let bytes: Vec<Vec<u8>> = try_join_all(futures?).await?;
                                let envs: Vec<_> = envs_to_set
                                    .iter()
                                    .map(|&env_hash| {
                                        collection.get_env(env_hash).ok_or_else(|| {
                                            melib::Error::new(format!(
                                                "Could not export mbox: envelope {env_hash} is no \
                                                 longer in the mailbox"
                                            ))
                                            .set_kind(melib::error::ErrorKind::NotFound)
                                        })
                                    })
                                    .collect::<Result<Vec<_>>>()?;
                                if path.is_dir() {
                                    let Some(first_env) = envs.first() else {
                                        return Err(melib::Error::new(
                                            "Could not export mbox: there is nothing to export",
                                        )
                                        .set_kind(melib::error::ErrorKind::NotFound));
                                    };
                                    let mut filename = if envs.len() == 1 {
                                        format!("{}.mbox", first_env.message_id()).into()
                                    } else {
                                        let now = datetime::timestamp_to_string(
                                            datetime::now(),
                                            Some(datetime::formats::RFC3339_DATETIME),
                                            false,
                                        );
                                        format!(
                                            "{}-{}-{}_envelopes.mbox",
                                            now,
                                            first_env.message_id(),
                                            envs.len(),
                                        )
                                        .into()
                                    };
                                    crate::sanitize_separator(&mut filename);
                                    path.push(filename.as_ref());
                                }
                                let mut file = BufWriter::new(
                                    File::options()
                                        .read(true)
                                        .write(true)
                                        .create_new(true)
                                        .open(&path)?,
                                );
                                let mut iter = envs.iter().zip(bytes);
                                let tags_lck = collection.tag_index.read().unwrap();
                                if let Some((env, ref bytes)) = iter.next() {
                                    let tags: Vec<&str> = env
                                        .tags()
                                        .iter()
                                        .filter_map(|h| tags_lck.get(h).map(|s| s.as_str()))
                                        .collect();
                                    format.append(
                                        &mut file,
                                        bytes.as_slice(),
                                        env.from().first(),
                                        Some(env.date()),
                                        (env.flags(), tags),
                                        MboxMetadata::CClient,
                                        true,
                                        false,
                                    )?;
                                }
                                for (env, bytes) in iter {
                                    let tags: Vec<&str> = env
                                        .tags()
                                        .iter()
                                        .filter_map(|h| tags_lck.get(h).map(|s| s.as_str()))
                                        .collect();
                                    format.append(
                                        &mut file,
                                        bytes.as_slice(),
                                        env.from().first(),
                                        Some(env.date()),
                                        (env.flags(), tags),
                                        MboxMetadata::CClient,
                                        false,
                                        false,
                                    )?;
                                }
                                file.flush()?;
                                Ok(path)
                            };
                            let r: Result<PathBuf> = cl.await;
                            let _ = sender.send(r);
                            Ok(())
                        });
                    let handle = account.main_loop_handler.job_executor.spawn(
                        "exporting-mbox".into(),
                        fut,
                        IsAsync::Blocking,
                    );
                    account.insert_job(
                        handle.job_id,
                        JobRequest::Generic {
                            name: "exporting mbox".into(),
                            handle,
                            on_finish: Some(CallbackFn(Box::new(move |context: &mut Context| {
                                context.replies.push_back(match receiver.try_recv() {
                                    Err(_) | Ok(None) => UIEvent::Notification {
                                        title: Some("Could not export mbox".into()),
                                        source: None,
                                        body: "Job was canceled.".into(),
                                        kind: Some(NotificationType::Info),
                                    },
                                    Ok(Some(Err(err))) => UIEvent::Notification {
                                        title: Some("Could not export mbox".into()),
                                        source: None,
                                        body: err.to_string().into(),
                                        kind: Some(NotificationType::Error(err.kind)),
                                    },
                                    Ok(Some(Ok(path))) => UIEvent::Notification {
                                        title: Some("Successfully exported mbox".into()),
                                        source: None,
                                        body: format!("Wrote to file {}", path.display()).into(),
                                        kind: Some(NotificationType::Info),
                                    },
                                });
                            }))),
                            log_level: LogLevel::INFO,
                        },
                    );
                }
                ListingAction::MoveToOtherAccount(ref _account_name, ref _mailbox_path) => {
                    context.replies.push_back(UIEvent::Notification {
                        title: Some("Could not move mail".into()),
                        source: None,
                        body: "Moving to another account is currently unimplemented".into(),
                        kind: Some(NotificationType::Error(ErrorKind::NotImplemented)),
                    });
                }
                _ => unreachable!(),
            }
        }
        let account_hash = self.coordinates().0;
        let mailbox_hash = self.coordinates().1;
        /*{
            let threads_lck = account.collection.get_threads(mailbox_hash);
            for thread_hash in thread_hashes {
                for (_, h) in threads_lck.thread_iter(thread_hash) {
                    envs_to_set.push(threads_lck.thread_nodes()[&h].message().unwrap());
                }
                self.row_updates().push(thread_hash);
            }
        }
        */
        inner(context, envs_to_set, account_hash, mailbox_hash, a);
        self.set_dirty(true);
    }

    fn row_updates(&mut self) -> &mut SmallVec<[EnvelopeHash; 8]>;
    fn selection(&self) -> &HashMap<EnvelopeHash, bool>;
    fn selection_mut(&mut self) -> &mut HashMap<EnvelopeHash, bool>;
    fn get_focused_items(&self, _context: &Context) -> SmallVec<[EnvelopeHash; 8]>;
    fn redraw_threads_list(
        &mut self,
        context: &Context,
        items: Box<dyn Iterator<Item = ThreadHash>>,
    );

    fn redraw_envelope_list(
        &mut self,
        _context: &Context,
        _items: Box<dyn Iterator<Item = EnvelopeHash>>,
    ) {
    }

    /// Use `force` when there have been changes in the mailbox or account lists
    /// in `context`
    fn refresh_mailbox(&mut self, context: &mut Context, force: bool);
}

pub trait ListingTrait: Component {
    fn coordinates(&self) -> (AccountHash, MailboxHash);
    fn set_coordinates(&mut self, _: (AccountHash, MailboxHash));
    fn next_entry(&mut self, context: &mut Context);
    fn prev_entry(&mut self, context: &mut Context);
    fn draw_list(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context);
    fn highlight_line(&mut self, grid: &mut CellBuffer, area: Area, idx: usize, context: &Context);
    fn filter(&mut self, _filter_term: String, _results: Vec<EnvelopeHash>, _context: &Context) {}
    fn unfocused(&self) -> bool;
    fn view_area(&self) -> Option<Area>;
    fn set_modifier_active(&mut self, _new_val: bool);
    fn modifier_active(&self) -> bool;
    fn set_modifier_command(&mut self, _new_val: Option<Modifier>);
    fn modifier_command(&self) -> Option<Modifier>;
    fn set_movement(&mut self, mvm: PageMovement);
    fn focus(&self) -> Focus;
    fn set_focus(&mut self, new_value: Focus, context: &mut Context);

    fn kick_parent(&self, parent: ComponentId, msg: ListingMessage, context: &mut Context) {
        log::trace!(
            "kick_parent self is {} parent is {parent} msg is {msg:?}",
            self.id()
        );
        context.replies.push_back(UIEvent::IntraComm {
            from: self.id(),
            to: parent,
            content: Box::new(msg),
        });
    }

    fn format_date(&self, context: &Context, epoch: UnixTimestamp) -> String {
        let d = std::time::UNIX_EPOCH + std::time::Duration::from_secs(epoch);
        let now: std::time::Duration = std::time::SystemTime::now()
            .duration_since(d)
            .unwrap_or_else(|_| std::time::Duration::new(u64::MAX, 0));
        match now.as_secs() {
            n if context.settings.listing.recent_dates && n < 60 * 60 => format!(
                "{} minute{} ago",
                n / (60),
                if n / 60 == 1 { "" } else { "s" }
            ),
            n if context.settings.listing.recent_dates && n < 24 * 60 * 60 => format!(
                "{} hour{} ago",
                n / (60 * 60),
                if n / (60 * 60) == 1 { "" } else { "s" }
            ),
            n if context.settings.listing.recent_dates && n < 7 * 24 * 60 * 60 => format!(
                "{} day{} ago",
                n / (24 * 60 * 60),
                if n / (24 * 60 * 60) == 1 { "" } else { "s" }
            ),
            _ => melib::utils::datetime::timestamp_to_string(
                epoch,
                context
                    .settings
                    .listing
                    .datetime_fmt
                    .as_deref()
                    .or(Some("%Y-%m-%d %T")),
                false,
            ),
        }
    }
}

#[derive(Debug)]
pub enum ListingComponent {
    Compact(Box<CompactListing>),
    Conversations(Box<ConversationsListing>),
    Offline(Box<OfflineListing>),
    Plain(Box<PlainListing>),
    Threaded(Box<ThreadListing>),
}
use crate::ListingComponent::*;

impl std::ops::Deref for ListingComponent {
    type Target = dyn MailListingTrait;

    fn deref(&self) -> &Self::Target {
        match &self {
            Compact(ref l) => l.as_ref(),
            Conversations(ref l) => l.as_ref(),
            Offline(ref l) => l.as_ref(),
            Plain(ref l) => l.as_ref(),
            Threaded(ref l) => l.as_ref(),
        }
    }
}

impl std::ops::DerefMut for ListingComponent {
    fn deref_mut(&mut self) -> &mut (dyn MailListingTrait + 'static) {
        match self {
            Compact(l) => l.as_mut(),
            Conversations(l) => l.as_mut(),
            Offline(l) => l.as_mut(),
            Plain(l) => l.as_mut(),
            Threaded(l) => l.as_mut(),
        }
    }
}

impl ListingComponent {
    fn id(&self) -> ComponentId {
        match self {
            Compact(l) => l.as_component().id(),
            Conversations(l) => l.as_component().id(),
            Offline(l) => l.as_component().id(),
            Plain(l) => l.as_component().id(),
            Threaded(l) => l.as_component().id(),
        }
    }

    /// Propagate the keyboard-focus flag to the listing grid component:
    /// its Entry-state subpane ring renders focused while the grid (not
    /// the open view) holds the keyboard.
    fn set_grid_has_keyboard(&mut self, value: bool) {
        match self {
            Compact(l) => l.set_grid_has_keyboard(value),
            Conversations(l) => l.set_grid_has_keyboard(value),
            Plain(l) => l.set_grid_has_keyboard(value),
            Threaded(l) => l.set_grid_has_keyboard(value),
            Offline(_) => {}
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
enum ListingFocus {
    Menu,
    MailList,
    View,
}

impl ListingComponent {
    /// The (thread, envelope) under the grid cursor, if any.
    fn cursor_selection(&self) -> Option<(ThreadHash, EnvelopeHash)> {
        match self {
            Compact(l) => l.cursor_selection(),
            Conversations(l) => l.cursor_selection(),
            Plain(l) => l.cursor_selection(),
            Threaded(l) => l.cursor_selection(),
            Offline(_) => None,
        }
    }

    /// Queue an `OpenEntryUnderCursor` for the cursor entry (view refresh
    /// while the grid holds the keyboard).
    fn kick_open_under_cursor(&self, context: &mut Context) {
        match self {
            Compact(l) => l.kick_open_under_cursor(context),
            Conversations(l) => l.kick_open_under_cursor(context),
            Plain(l) => l.kick_open_under_cursor(context),
            Threaded(l) => l.kick_open_under_cursor(context),
            Offline(_) => {}
        }
    }
}

impl ListingComponent {
    /// Apply a search result to the active component.
    pub fn filter(&mut self, filter_term: String, results: Vec<EnvelopeHash>, context: &Context) {
        match self {
            Compact(l) => l.filter(filter_term, results, context),
            Conversations(l) => l.filter(filter_term, results, context),
            Plain(l) => l.filter(filter_term, results, context),
            Threaded(l) => l.filter(filter_term, results, context),
            Offline(_) => {}
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CursorPos {
    account: usize,
    menu: MenuEntryCursor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MenuEntryCursor {
    Status,
    Mailbox(usize),
}

impl std::ops::Sub<MenuEntryCursor> for isize {
    type Output = Self;

    fn sub(self, other: MenuEntryCursor) -> Self {
        if let MenuEntryCursor::Mailbox(v) = other {
            v as Self - self
        } else {
            self - 1
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShowMenuScrollbar {
    Never,
    True,
    False,
}

#[derive(Debug)]
pub struct Listing {
    component: ListingComponent,
    accounts: Vec<AccountMenuEntry>,
    status: Option<AccountStatus>,
    dirty: bool,
    cursor_pos: CursorPos,
    menu_cursor_pos: CursorPos,
    menu: Screen<Virtual>,
    menu_scrollbar_show_timer: crate::jobs::Timer,
    show_menu_scrollbar: ShowMenuScrollbar,
    startup_checks_rate: RateLimit,
    id: ComponentId,
    // Configurable settings
    theme_default: ThemeAttribute,
    sidebar_divider: char,
    sidebar_divider_theme: ThemeAttribute,
    // State
    menu_visibility: bool,
    focus: ListingFocus,
    /// Cached `is_menu_visible()` from the previous draw: when the sidebar
    /// occlusion flips, every pane must repaint or stale pixels (the old
    /// sidebar/grid split) survive the layout shift.
    prev_menu_visible: bool,
    /// Cached layout4 flag (view owns the whole pane): flips force a full
    /// repaint of every pane.
    prev_view_fullscreen: bool,
    /// on the grid refresh the view only when the selection changed.
    last_opened_env: Option<EnvelopeHash>,
    view: Option<Box<ThreadView>>,
}

impl std::fmt::Display for Listing {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self.component {
            Compact(ref l) => write!(f, "{l}"),
            Conversations(ref l) => write!(f, "{l}"),
            Offline(ref l) => write!(f, "{l}"),
            Plain(ref l) => write!(f, "{l}"),
            Threaded(ref l) => write!(f, "{l}"),
        }
    }
}

impl Component for Listing {
    fn draw(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        #[cfg(debug_assertions)]
        let __draw_span = crate::state::DrawSpan::enter("Listing");
        if !self.is_dirty() {
            return;
        }
        let total_cols = area.width();

        let menu_visible = self.is_menu_visible();
        if menu_visible != self.prev_menu_visible {
            // The sidebar occlusion flipped: every pane must repaint or
            // stale pixels (the old sidebar/grid split) survive the shift.
            self.prev_menu_visible = menu_visible;
            self.dirty = true;
            self.component.set_dirty(true);
            if let Some(view) = self.view.as_mut() {
                view.set_dirty(true);
            }
        }

        // Fixed split: the mailbox list keeps 30% of the width (layout1),
        // the right pane 70%.
        let right_component_width = if menu_visible {
            total_cols - total_cols * 3 / 10
        } else {
            total_cols
        };
        let mid = area.width().saturating_sub(right_component_width);
        /* Top-level horizontal split via ratatui Layout: sidebar | divider
         * column | right side. The degenerate single-pane cases (no
         * sidebar) hand the whole area to the right pane without a divider
         * column, so the split is only computed when the sidebar is
         * visible. */
        let (menu_area, divider_area, list_area) =
            if right_component_width != total_cols && right_component_width != 0 {
                let [menu, divider, right] = Layout::horizontal([
                    Constraint::Length(mid as u16),
                    Constraint::Length(1),
                    Constraint::Min(0),
                ])
                .areas(area_to_rect(area));
                (
                    rect_to_area(menu, area),
                    rect_to_area(divider, area),
                    rect_to_area(right, area),
                )
            } else {
                (
                    area,
                    Area::new_empty(area.generation()),
                    Area::new_empty(area.generation()),
                )
            };
        if self.dirty && mid != 0 {
            for row in grid.bounds_iter(divider_area) {
                for c in row {
                    grid[c]
                        .set_ch(self.sidebar_divider)
                        .set_fg(self.sidebar_divider_theme.fg)
                        .set_bg(self.sidebar_divider_theme.bg)
                        .set_attrs(self.sidebar_divider_theme.attrs);
                }
            }
            context.dirty_areas.push_back(divider_area);
        }

        let account_hash = self.accounts[self.cursor_pos.account].hash;
        /* Rounded pane frames (visual chrome only): the pane holding the
         * keyboard focus is framed with "tab.focused", the other with
         * "tab.unfocused". With a view open (layout2/3/4) the listing
         * skips its own frame — the grid subpane and the view draw their
         * own rings — and the component, its view_area and the view keep
         * the full pane area. */
        let tab_focused = crate::conf::value(context, "tab.focused");
        let tab_unfocused = crate::conf::value(context, "tab.unfocused");
        let (menu_attr, list_attr) = if matches!(self.focus, ListingFocus::Menu) {
            (tab_focused, tab_unfocused)
        } else {
            (tab_unfocused, tab_focused)
        };
        let view_drawn = self.status.is_none() && self.component.unfocused() && self.view.is_some();
        // Layout4: the view owns the whole pane (thread list | mail view
        // inside, 30/70) — the grid is hidden, exactly two panes on
        // screen. Layout2/layout3 keep [grid 30% | view 70%].
        let view_fullscreen = self.view.as_ref().is_some_and(|v| {
            !v.is_single_mail() && !matches!(v.thread_view_focus(), ThreadViewFocus::Thread)
        });
        if view_fullscreen != self.prev_view_fullscreen {
            // The grid pane appears/disappears: every pane must repaint or
            // stale pixels survive the shift.
            self.prev_view_fullscreen = view_fullscreen;
            self.dirty = true;
            self.component.set_dirty(true);
            if let Some(view) = self.view.as_mut() {
                view.set_dirty(true);
            }
        }

        if right_component_width == total_cols {
            if Self::should_replace_with_offline(context, account_hash)
                && !matches!(self.component, ListingComponent::Offline(_))
            {
                self.component.unrealize(context);
                self.component =
                    Offline(OfflineListing::new((account_hash, MailboxHash::default())));
                self.component
                    .process_event(&mut UIEvent::VisibilityChange(true), context);
                self.component.realize(self.id().into(), context);
            }

            let content_area = if view_drawn {
                area
            } else {
                let inner = draw_rounded_frame(grid, area, list_attr);
                for frame_area in frame_flush_areas(grid, area) {
                    context.dirty_areas.push_back(frame_area);
                }
                inner
            };
            if let Some(s) = self.status.as_mut() {
                s.draw(grid, content_area, context);
            } else if view_drawn && view_fullscreen {
                // Layout4: only the view is on screen.
                if let Some(view) = &mut self.view {
                    view.draw(grid, content_area, context);
                }
            } else {
                self.component.draw(grid, content_area, context);
                if self.component.unfocused() {
                    if let Some(view) = &mut self.view {
                        // Computed here (not via the component's cached
                        // `view_area`, which a not-dirty component draw
                        // leaves stale — bound to a previous screen
                        // generation) with the same fixed split the
                        // components render: grid 30% | gap | view.
                        let (_, view_area) = crate::mail::pane_split(area);
                        view.draw(grid, view_area, context);
                    }
                }
            }
        } else if right_component_width == 0 {
            let menu_inner = draw_rounded_frame(grid, area, menu_attr);
            for frame_area in frame_flush_areas(grid, area) {
                context.dirty_areas.push_back(frame_area);
            }
            self.draw_menu(grid, menu_inner, context);
        } else {
            let menu_inner = draw_rounded_frame(grid, menu_area, menu_attr);
            for frame_area in frame_flush_areas(grid, menu_area) {
                context.dirty_areas.push_back(frame_area);
            }
            self.draw_menu(grid, menu_inner, context);
            if Self::should_replace_with_offline(context, account_hash)
                && !matches!(self.component, ListingComponent::Offline(_))
            {
                self.component.unrealize(context);
                self.component =
                    Offline(OfflineListing::new((account_hash, MailboxHash::default())));
                self.component
                    .process_event(&mut UIEvent::VisibilityChange(true), context);
                self.component.realize(self.id().into(), context);
            }
            let content_area = if view_drawn {
                list_area
            } else {
                let inner = draw_rounded_frame(grid, list_area, list_attr);
                for frame_area in frame_flush_areas(grid, list_area) {
                    context.dirty_areas.push_back(frame_area);
                }
                inner
            };
            if let Some(s) = self.status.as_mut() {
                s.draw(grid, content_area, context);
            } else {
                let area = list_area;
                self.component.draw(grid, content_area, context);
                if self.component.unfocused() {
                    if let Some(view) = &mut self.view {
                        // Computed here (not via the component's cached
                        // `view_area`, which a not-dirty component draw
                        // leaves stale — bound to a previous screen
                        // generation) with the same fixed split the
                        // components render: grid 30% | gap | view.
                        let (_, view_area) = crate::mail::pane_split(area);
                        view.draw(grid, view_area, context);
                    }
                }
            }
        }
        self.dirty = false;
        // Grid-focus layouts (layout2/layout3): a cursor move on the grid
        // (j/k, paging, filtering) refreshes the open view to the newly
        // selected entry; the layout follows the selection (single mail →
        // layout2, thread → layout3) because the view is rebuilt.
        if self.status.is_none()
            && matches!(self.focus, ListingFocus::MailList)
            && self.view.is_some()
            && self.component.unfocused()
        {
            let selection = self.component.cursor_selection();
            if let Some((_, env_hash)) = selection {
                if self.last_opened_env != Some(env_hash) {
                    self.last_opened_env = Some(env_hash);
                    self.component.kick_open_under_cursor(context);
                }
            }
        }
    }

    fn process_event(&mut self, event: &mut UIEvent, context: &mut Context) -> bool {
        // Listing-level search commands are not pane-scoped: at `MailList`
        // or `Menu` focus deliver them to the component before any focus
        // routing below — a menu-held keyboard must not swallow them.
        // At `View` focus the view owns the keyboard and the action is its
        // in-body search, so it must fall through to the view-first routing
        // further down instead of being hijacked by the grid component.
        // (Style switches and entry operations stay on their own paths
        // below: they are handled by the listing itself.)
        if matches!(self.focus, ListingFocus::MailList | ListingFocus::Menu)
            && matches!(
                event,
                UIEvent::Action(Action::Listing(
                    ListingAction::Search { .. } | ListingAction::Select { .. }
                ))
            )
        {
            log::debug!(
                "listing: forwarding {event:?} to the component (focus {:?})",
                self.focus
            );
            return self.component.process_event(event, context);
        }
        match event {
            UIEvent::ConfigReload { old_settings: _ } => {
                self.theme_default = crate::conf::value(context, "theme_default");
                let account_hash = context.accounts[self.cursor_pos.account].hash();
                self.sidebar_divider =
                    *account_settings!(context[account_hash].listing.sidebar_divider);
                self.sidebar_divider_theme = conf::value(context, "mail.sidebar_divider");
                self.menu.grid_mut().empty();
                self.set_dirty(true);
            }
            UIEvent::Timer(n) if *n == self.menu_scrollbar_show_timer.id() => {
                if self.show_menu_scrollbar == ShowMenuScrollbar::True {
                    self.show_menu_scrollbar = ShowMenuScrollbar::False;
                    self.set_dirty(true);
                    self.menu.grid_mut().empty();
                }
                return true;
            }
            UIEvent::StartupCheck(ref f)
                if self.component.coordinates().1 == *f && !self.startup_checks_rate.tick() =>
            {
                return false;
            }
            UIEvent::Timer(n) if *n == self.startup_checks_rate.id() => {
                if self.startup_checks_rate.active {
                    self.startup_checks_rate.reset();
                    return self.process_event(
                        &mut UIEvent::StartupCheck(self.component.coordinates().1),
                        context,
                    );
                }
            }
            UIEvent::AccountStatusChange(account_hash, msg) => {
                let account_index: usize = context
                    .accounts
                    .get_index_of(account_hash)
                    .expect("Invalid account_hash in UIEventMailbox{Delete,Create}");
                if self.cursor_pos.account == account_index {
                    // This is a background reconcile of the current account (the
                    // watcher reporting that it (re)connected or refreshed), not
                    // a user navigation, so it must not steal the keyboard focus
                    // from the sidebar. `change_account` resets the focus via
                    // `close_view`, so remember the pre-reconcile focus and put
                    // it back. A `View` focus cannot survive because
                    // `change_account` force-closes the open view, and
                    // `close_view` already lands the grid for that case.
                    let previous_focus = match self.focus {
                        ListingFocus::Menu => Some(ListingFocus::Menu),
                        ListingFocus::MailList => Some(ListingFocus::MailList),
                        ListingFocus::View => None,
                    };
                    self.change_account(context);
                    match previous_focus {
                        // `change_account` (via `close_view`) lands the
                        // keyboard on the grid; handing the focus back to
                        // the sidebar must take the grid's keyboard
                        // highlight with it (see `focus_menu`).
                        Some(ListingFocus::Menu) => self.focus_menu(),
                        Some(focus) => self.focus = focus,
                        None => {}
                    }
                } else {
                    let previous_collapsed_mailboxes: BTreeSet<MailboxHash> = self.accounts
                        [account_index]
                        .entries
                        .iter()
                        .filter_map(|e| {
                            if e.collapsed {
                                Some(e.mailbox_hash)
                            } else {
                                None
                            }
                        })
                        .collect::<_>();
                    let previous_index_styles: BTreeMap<MailboxHash, IndexStyle> = self.accounts
                        [account_index]
                        .entries
                        .iter()
                        .filter_map(|e| Some((e.mailbox_hash, e.index_style?)))
                        .collect::<_>();
                    self.accounts[account_index].entries = context.accounts[&*account_hash]
                        .list_mailboxes()
                        .into_iter()
                        .filter(|mailbox_node| {
                            context.accounts[&*account_hash][&mailbox_node.hash]
                                .ref_mailbox
                                .is_subscribed()
                        })
                        .map(|f| MailboxMenuEntry {
                            depth: f.depth,
                            indentation: f.indentation,
                            has_sibling: f.has_sibling,
                            mailbox_hash: f.hash,
                            visible: true,
                            collapsed: if previous_collapsed_mailboxes.is_empty() {
                                context.accounts[&*account_hash][&f.hash].conf.collapsed
                            } else {
                                previous_collapsed_mailboxes.contains(&f.hash)
                            },
                            index_style: previous_index_styles.get(&f.hash).copied(),
                        })
                        .collect::<_>();
                    self.menu.grid_mut().empty();
                    context
                        .replies
                        .push_back(UIEvent::StatusEvent(StatusEvent::UpdateStatus(match msg {
                            Some(msg) => format!("{} {}", self.status(context), msg),
                            None => self.status(context),
                        })));
                    if let Some((acc_hash, mb_hash)) = self.status_watch() {
                        context
                            .replies
                            .push_back(UIEvent::StatusEvent(StatusEvent::FocusMailbox(
                                acc_hash, mb_hash,
                            )));
                    }
                }
            }
            UIEvent::MailboxCreate((account_hash, mailbox_hash)) => {
                let account_index = context
                    .accounts
                    .get_index_of(account_hash)
                    .expect("Invalid account_hash in UIEventMailbox{Delete,Create}");
                self.menu.grid_mut().empty();
                let previous_collapsed_mailboxes: BTreeSet<MailboxHash> = self.accounts
                    [account_index]
                    .entries
                    .iter()
                    .filter_map(|e| {
                        if e.collapsed {
                            Some(e.mailbox_hash)
                        } else {
                            None
                        }
                    })
                    .collect::<_>();
                let previous_index_styles: BTreeMap<MailboxHash, IndexStyle> = self.accounts
                    [account_index]
                    .entries
                    .iter()
                    .filter_map(|e| Some((e.mailbox_hash, e.index_style?)))
                    .collect::<_>();
                self.accounts[account_index].entries = context.accounts[&*account_hash]
                    .list_mailboxes()
                    .into_iter()
                    .filter(|mailbox_node| {
                        context.accounts[&*account_hash][&mailbox_node.hash]
                            .ref_mailbox
                            .is_subscribed()
                    })
                    .map(|f| MailboxMenuEntry {
                        depth: f.depth,
                        indentation: f.indentation,
                        has_sibling: f.has_sibling,
                        mailbox_hash: f.hash,
                        visible: true,
                        collapsed: previous_collapsed_mailboxes.contains(&f.hash),
                        index_style: previous_index_styles.get(&f.hash).copied(),
                    })
                    .collect::<_>();
                let fallback = if let MenuEntryCursor::Mailbox(ref mut cur) = self.cursor_pos.menu {
                    *cur = std::cmp::min(
                        self.accounts[self.cursor_pos.account]
                            .entries
                            .len()
                            .saturating_sub(1),
                        *cur,
                    );
                    *cur
                } else {
                    0
                };
                if self.component.coordinates() == (*account_hash, *mailbox_hash) {
                    self.component
                        .process_event(&mut UIEvent::VisibilityChange(false), context);
                    self.component.set_coordinates((
                        self.accounts[self.cursor_pos.account].hash,
                        self.accounts[self.cursor_pos.account].entries[fallback].mailbox_hash,
                    ));
                    self.component.refresh_mailbox(context, true);
                    self.component
                        .process_event(&mut UIEvent::VisibilityChange(true), context);
                }
                self.push_status_watch(
                    self.status(context),
                    self.status_watch(),
                    &mut context.replies,
                );
                self.set_dirty(true);
                return true;
            }
            UIEvent::ChangeMode(UIMode::Normal) => {
                self.set_dirty(true);
            }
            UIEvent::Resize => {
                self.set_dirty(true);
            }
            UIEvent::Action(Action::ViewMailbox(ref idx)) => {
                if let Some(MailboxMenuEntry { mailbox_hash, .. }) =
                    self.accounts[self.cursor_pos.account].entries.get(*idx)
                {
                    let account_hash = self.accounts[self.cursor_pos.account].hash;
                    let mailbox_hash = *mailbox_hash;
                    self.cursor_pos.menu = MenuEntryCursor::Mailbox(*idx);
                    self.close_view(context);
                    self.component
                        .process_event(&mut UIEvent::VisibilityChange(false), context);
                    self.component.set_coordinates((account_hash, mailbox_hash));
                    self.component
                        .process_event(&mut UIEvent::VisibilityChange(true), context);
                    self.menu.grid_mut().empty();
                    self.set_dirty(true);
                }
                return true;
            }
            UIEvent::IntraComm {
                from,
                to,
                ref content,
            } if (*from, *to) == (self.component.id(), self.id()) => {
                match content.downcast_ref::<ListingMessage>().copied() {
                    None => {}
                    Some(ListingMessage::FocusUpdate { new_value }) => {
                        if let Some(ref mut view) = self.view {
                            view.process_event(
                                &mut UIEvent::VisibilityChange(!matches!(new_value, Focus::None)),
                                context,
                            );
                        }
                        match new_value {
                            Focus::None => {
                                // The grid released the entry: drop any
                                // residual view (idempotent with
                                // `close_view`) and land the keyboard on
                                // the grid.
                                if let Some(view) = self.view.take() {
                                    view.unrealize(context);
                                }
                                if self.focus == ListingFocus::View {
                                    self.focus = ListingFocus::MailList;
                                }
                            }
                            Focus::Entry => {
                                // The entry is open; the keyboard stays on
                                // the grid (layout2/layout3 keep the grid
                                // pane focused after opening — the
                                // `OpenEntryUnderCursor` reply has run
                                // first, so the view exists).
                                self.component.set_grid_has_keyboard(true);
                            }
                        }
                        // Need to clear gap between sidebar and listing component, if any.
                        self.dirty = true;
                    }
                    Some(ListingMessage::UpdateView) => {
                        if let Some(ref mut view) = self.view {
                            view.set_dirty(true);
                        }
                    }
                    Some(ListingMessage::OpenEntryUnderCursor {
                        env_hash,
                        thread_hash,
                        go_to_first_unread,
                    }) => {
                        let (a, m) = self.component.coordinates();
                        if let Some(view) = self.view.take() {
                            view.unrealize(context);
                        }
                        self.last_opened_env = Some(env_hash);
                        let mut view = Box::new(ThreadView::new(
                            (a, m, env_hash),
                            thread_hash,
                            Some(env_hash),
                            go_to_first_unread,
                            Some(ThreadViewFocus::Thread),
                            context,
                        ));
                        // The keyboard stays on the grid after opening
                        // (layout2/layout3 open grid-focused): the view's
                        // rings render dimmed.
                        view.set_grid_focused(true);
                        self.view = Some(view);
                    }
                }
                return true;
            }
            #[cfg(feature = "debug-tracing")]
            UIEvent::IntraComm {
                from,
                to,
                ref content,
            } if *from == self.component.id() || *to == self.id() => {
                log::debug!(
                    "BUG intracomm event: {:?} downcast content {:?}",
                    event,
                    content.downcast_ref::<ListingMessage>().copied()
                );
                log::debug!(
                    "BUG component is {} and self id is {}",
                    self.component.id(),
                    self.id()
                );
            }
            _ => {}
        }

        // View-first routing: while a view is open, non-Input events (live
        // updates, refreshes) always reach it first; Input events reach it
        // first only when it owns the keyboard (`ListingFocus::View`). At
        // `MailList` focus the grid owns the keyboard, so Input events skip
        // the view entirely.
        if self.component.unfocused()
            && self.view.is_some()
            && (!matches!(&*event, UIEvent::Input(_)) || self.focus == ListingFocus::View)
            && self
                .view
                .as_mut()
                .map(|v| v.process_event(event, context))
                .unwrap_or(false)
        {
            return true;
        }

        if matches!(self.focus, ListingFocus::MailList | ListingFocus::View)
            && self.status.is_some()
        {
            if let Some(s) = self.status.as_mut() {
                if s.process_event(event, context) {
                    return true;
                }
            }
        }

        // Only the `UIEvent::Input` arms below resolve shortcut bindings,
        // so skip rebuilding (and re-hashing) this map — which also clones
        // the focused component's and the mail view's shortcut sections —
        // for every other event: backend syncs can deliver hundreds of
        // non-key events per second.
        let shortcuts = if matches!(event, UIEvent::Input(_)) {
            let mut m = self.shortcuts(context);
            m.insert(
                Shortcuts::GENERAL,
                context.settings.shortcuts.general.key_values(),
            );
            m
        } else {
            ShortcutMaps::default()
        };

        // Pane-chain pre-arms (Hyprland-style window management over
        // [sidebar] [grid] [thread list] [mail detail]): they run ahead of
        // the component's own arms so the direction keys always move the
        // focus along the chain, and the exit keys close the focus layer
        // (the mail pane only at the mail-detail layer, else the whole
        // view). `status` keeps its own layered quit arm below.
        if let UIEvent::Input(k) = &*event {
            let focus_right = shortcut!(k == shortcuts[Shortcuts::LISTING]["focus_right"]);
            let focus_left = shortcut!(k == shortcuts[Shortcuts::LISTING]["focus_left"]);
            let open_entry = shortcut!(k == shortcuts[Shortcuts::LISTING]["open_entry"]);
            let exit_entry = shortcut!(k == shortcuts[Shortcuts::LISTING]["exit_entry"])
                || context.settings.shortcuts.general.quit.contains(k);
            if self.status.is_none() {
                match self.focus {
                    ListingFocus::MailList if focus_right || open_entry => {
                        // From layout1's grid: open the cursor entry. The
                        // view is created by the queued
                        // `OpenEntryUnderCursor` reply; the focus stays on
                        // the grid (layout2 for a single mail, layout3
                        // for a thread). With the view already open
                        // (layout2/layout3 grid focus) the key moves the
                        // keyboard onto the right pane (layout2: the mail
                        // view, layout3: → layout4's thread list).
                        if self.view.is_some() {
                            if let Some(view) = self.view.as_mut() {
                                view.enter_split();
                            }
                            self.component.set_grid_has_keyboard(false);
                            if let Some(view) = self.view.as_mut() {
                                view.set_grid_focused(false);
                            }
                            self.focus = ListingFocus::View;
                            // The grid's and the view's pane highlights
                            // swap with the keyboard; both panes must
                            // repaint on the next frame.
                            self.set_dirty(true);
                        } else {
                            self.component.set_focus(Focus::Entry, context);
                        }
                        return true;
                    }
                    ListingFocus::MailList if focus_left => {
                        // From layout2's or layout3's grid: close the
                        // view back to layout1 with the focus on the
                        // mailbox list. From layout1's grid: hand the
                        // focus to the mailbox list.
                        match self.view.as_ref() {
                            Some(view) if view.is_single_mail() => {
                                self.close_view(context);
                                if self.menu_visibility {
                                    self.focus_menu();
                                }
                            }
                            Some(view)
                                if matches!(view.thread_view_focus(), ThreadViewFocus::Thread) =>
                            {
                                self.close_view(context);
                                if self.menu_visibility {
                                    self.focus_menu();
                                }
                            }
                            Some(_) => {}
                            None => {
                                if self.menu_visibility {
                                    self.focus_menu();
                                }
                            }
                        }
                        self.set_dirty(true);
                        return true;
                    }
                    ListingFocus::MailList if exit_entry && self.view.is_some() => {
                        // quit from layout2/layout3's grid: back to
                        // layout1, the focus on the grid.
                        self.close_view(context);
                        return true;
                    }
                    ListingFocus::View if focus_left => {
                        // From layout4's thread list: close the mail pane
                        // and land on layout3's grid. From layout2's mail
                        // view (single-mail Left passes through the view):
                        // focus layout2's grid.
                        self.component.set_grid_has_keyboard(true);
                        if let Some(view) = self.view.as_mut() {
                            if !view.is_single_mail() {
                                view.close_mail_pane();
                            }
                            view.set_grid_focused(true);
                        }
                        self.focus = ListingFocus::MailList;
                        self.set_dirty(true);
                        return true;
                    }
                    ListingFocus::View if exit_entry => {
                        // quit from layout2's mail view: close the view
                        // back to layout1. quit from layout4: close the
                        // mail pane and land on layout3's grid.
                        if self.view.as_ref().is_some_and(|v| v.is_single_mail()) {
                            self.close_view(context);
                        } else {
                            if let Some(view) = self.view.as_mut() {
                                view.close_mail_pane();
                                view.set_grid_focused(true);
                            }
                            self.component.set_grid_has_keyboard(true);
                            self.focus = ListingFocus::MailList;
                            self.set_dirty(true);
                        }
                        return true;
                    }
                    _ => {}
                }
            }
        }

        let mut have_forwarded_to_component = false;
        // Forward events to self.component if it's focused, otherwise forward any
        // unhandled events to self.component at the end of this function.
        if matches!(self.focus, ListingFocus::MailList | ListingFocus::View)
            && self.status.is_none()
            && {
                have_forwarded_to_component = true;
                self.component.process_event(event, context)
            }
        {
            return true;
        }
        // Background-job completions (search/select results) reach the
        // component here — the focus gate above only handles input.
        if matches!(event, UIEvent::StatusEvent(StatusEvent::JobFinished(_))) {
            return self.component.process_event(event, context);
        }
        if matches!(self.focus, ListingFocus::MailList | ListingFocus::View) {
            match *event {
                UIEvent::Input(ref k)
                    if self.status.is_some()
                        && context.settings.shortcuts.general.quit.contains(k) =>
                {
                    // Layered quit: close the open `AccountStatus`
                    // sub-view instead of exiting the application.
                    self.status = None;
                    self.set_dirty(true);
                    return true;
                }
                UIEvent::Input(ref k)
                    if self.status.is_some()
                        && shortcut!(k == shortcuts[Shortcuts::LISTING]["focus_left"]) =>
                {
                    self.focus_menu();
                    self.set_dirty(true);
                }
                UIEvent::Input(ref k)
                    if shortcut!(k == shortcuts[Shortcuts::LISTING]["next_mailbox"])
                        || shortcut!(k == shortcuts[Shortcuts::LISTING]["prev_mailbox"]) =>
                {
                    self.component.set_modifier_active(false);
                    let amount = context.cmd_buf_clear().unwrap_or(1);
                    let target = match k {
                        k if shortcut!(k == shortcuts[Shortcuts::LISTING]["next_mailbox"]) => {
                            match self.cursor_pos.menu {
                                MenuEntryCursor::Status => amount.saturating_sub(1),
                                MenuEntryCursor::Mailbox(idx) => idx + amount,
                            }
                        }
                        k if shortcut!(k == shortcuts[Shortcuts::LISTING]["prev_mailbox"]) => {
                            match self.cursor_pos.menu {
                                MenuEntryCursor::Status => {
                                    return true;
                                }
                                MenuEntryCursor::Mailbox(idx) => {
                                    if idx >= amount {
                                        idx - amount
                                    } else {
                                        return true;
                                    }
                                }
                            }
                        }
                        _ => return true,
                    };
                    if self.accounts[self.cursor_pos.account]
                        .entries
                        .get(target)
                        .is_some()
                    {
                        self.cursor_pos.menu = MenuEntryCursor::Mailbox(target)
                    } else {
                        return true;
                    }
                    self.change_account(context);
                    return true;
                }
                UIEvent::Input(ref k)
                    if shortcut!(k == shortcuts[Shortcuts::LISTING]["next_account"])
                        || shortcut!(k == shortcuts[Shortcuts::LISTING]["prev_account"]) =>
                {
                    self.component.set_modifier_active(false);
                    let amount = context.cmd_buf_clear().unwrap_or(1);
                    match k {
                        k if shortcut!(k == shortcuts[Shortcuts::LISTING]["next_account"]) => {
                            if self.cursor_pos.account + amount < self.accounts.len() {
                                self.cursor_pos.account += amount;
                                let _new_val = self.cursor_pos.account;
                                self.cursor_pos.menu = if let Some(idx) = context.accounts[_new_val]
                                    .default_mailbox()
                                    .and_then(|h| self.accounts[_new_val].entry_by_hash(h))
                                {
                                    MenuEntryCursor::Mailbox(idx)
                                } else {
                                    MenuEntryCursor::Status
                                };
                            } else {
                                return true;
                            }
                        }
                        k if shortcut!(k == shortcuts[Shortcuts::LISTING]["prev_account"]) => {
                            if self.cursor_pos.account >= amount {
                                self.cursor_pos.account -= amount;
                                let _new_val = self.cursor_pos.account;
                                self.cursor_pos.menu = if let Some(idx) = context.accounts[_new_val]
                                    .default_mailbox()
                                    .and_then(|h| self.accounts[_new_val].entry_by_hash(h))
                                {
                                    MenuEntryCursor::Mailbox(idx)
                                } else {
                                    MenuEntryCursor::Status
                                };
                            } else {
                                return true;
                            }
                        }
                        _ => return false,
                    }
                    self.change_account(context);

                    return true;
                }
                UIEvent::Input(ref k)
                    if shortcut!(k == shortcuts[Shortcuts::LISTING]["toggle_menu_visibility"]) =>
                {
                    self.menu_visibility = !self.menu_visibility;
                    self.set_dirty(true);
                }
                _ => {}
            }

            if self.status.is_none() {
                match event {
                    UIEvent::Action(ref action) => match action {
                        Action::Listing(ListingAction::SetPlain) => {
                            self.set_index_style(IndexStyle::Plain, context);
                            return true;
                        }
                        Action::Listing(ListingAction::SetThreaded) => {
                            self.set_index_style(IndexStyle::Threaded, context);
                            return true;
                        }
                        Action::Listing(ListingAction::SetCompact) => {
                            self.set_index_style(IndexStyle::Compact, context);
                            return true;
                        }
                        Action::Listing(ListingAction::SetConversations) => {
                            self.set_index_style(IndexStyle::Conversations, context);
                            return true;
                        }
                        Action::Listing(ListingAction::Import(file_path, mailbox_path)) => {
                            let file_path = file_path.expand();
                            let account = &mut context.accounts[self.cursor_pos.account];
                            if let Err(err) = account
                                .mailbox_by_path(mailbox_path)
                                .and_then(|mailbox_hash| {
                                    Ok((
                                        std::fs::read(&file_path).chain_err_summary(|| {
                                            format!("Could not read {}", file_path.display())
                                        })?,
                                        mailbox_hash,
                                    ))
                                })
                                .and_then(|(bytes, mailbox_hash)| {
                                    account.save(&bytes, mailbox_hash, None)
                                })
                            {
                                context.replies.push_back(UIEvent::Notification {
                                    title: Some("Could not import mail".into()),
                                    source: None,
                                    body: err.to_string().into(),
                                    kind: Some(NotificationType::Error(err.kind)),
                                });
                            }
                            return true;
                        }
                        Action::Listing(a @ ListingAction::SetSeen)
                        | Action::Listing(a @ ListingAction::SetUnseen)
                        | Action::Listing(a @ ListingAction::Delete)
                        | Action::Listing(a @ ListingAction::CopyTo(_))
                        | Action::Listing(a @ ListingAction::MoveTo(_))
                        | Action::Listing(a @ ListingAction::CopyToOtherAccount(_, _))
                        | Action::Listing(a @ ListingAction::MoveToOtherAccount(_, _))
                        | Action::Listing(a @ ListingAction::ExportMbox(_, _))
                        | Action::Listing(a @ ListingAction::Flag(_))
                        | Action::Listing(a @ ListingAction::Tag(_))
                        | Action::Listing(a @ ListingAction::SendToTrash) => {
                            let focused = self.component.get_focused_items(context);
                            self.component.perform_action(context, focused, a);
                            let should_be_unselected: bool = matches!(
                                a,
                                ListingAction::Delete
                                    | ListingAction::MoveTo(_)
                                    | ListingAction::MoveToOtherAccount(_, _)
                                    | ListingAction::SendToTrash
                            );
                            let mut row_updates: SmallVec<[EnvelopeHash; 8]> = SmallVec::new();
                            for (k, v) in self.component.selection_mut().iter_mut() {
                                if *v {
                                    *v = !should_be_unselected;
                                    row_updates.push(*k);
                                }
                            }
                            self.component.row_updates().extend(row_updates);
                            return true;
                        }
                        Action::Listing(ListingAction::ClearSelection) => {
                            // Clear selection.
                            let row_updates: SmallVec<[EnvelopeHash; 8]> =
                                self.component.get_focused_items(context);
                            for h in &row_updates {
                                if let Some(val) = self.component.selection_mut().get_mut(h) {
                                    *val = false;
                                }
                            }
                            self.component.row_updates().extend(row_updates);
                            self.component.set_dirty(true);
                            return true;
                        }
                        _ => {}
                    },
                    UIEvent::Input(ref key)
                        if shortcut!(key == shortcuts[Shortcuts::LISTING]["scroll_up"])
                            || matches!(
                                key,
                                Key::Mouse(MouseEvent::Press(MouseButton::WheelUp, _, __))
                            ) =>
                    {
                        self.component.set_modifier_active(false);
                        let amount = context.cmd_buf_clear().unwrap_or(1);
                        self.component.set_movement(PageMovement::Up(amount));
                        return true;
                    }
                    UIEvent::Input(ref key)
                        if shortcut!(key == shortcuts[Shortcuts::LISTING]["scroll_down"])
                            || matches!(
                                key,
                                Key::Mouse(MouseEvent::Press(MouseButton::WheelDown, _, __))
                            ) =>
                    {
                        self.component.set_modifier_active(false);
                        let amount = context.cmd_buf_clear().unwrap_or(1);
                        self.component.set_movement(PageMovement::Down(amount));
                        return true;
                    }
                    UIEvent::Input(ref key)
                        if shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_right"]) =>
                    {
                        self.component.set_modifier_active(false);
                        let amount = context.cmd_buf_clear().unwrap_or(1);
                        self.component.set_movement(PageMovement::Right(amount));
                        return true;
                    }
                    UIEvent::Input(ref key)
                        if shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_left"]) =>
                    {
                        let amount = context.cmd_buf_clear().unwrap_or(1);
                        self.component.set_movement(PageMovement::Left(amount));
                        return true;
                    }
                    UIEvent::Input(ref key)
                        if shortcut!(key == shortcuts[Shortcuts::LISTING]["prev_page"]) =>
                    {
                        self.component.set_modifier_active(false);
                        let mult = context.cmd_buf_clear().unwrap_or(1);
                        self.component.set_movement(PageMovement::PageUp(mult));
                        return true;
                    }
                    UIEvent::Input(ref key)
                        if shortcut!(key == shortcuts[Shortcuts::LISTING]["next_page"]) =>
                    {
                        self.component.set_modifier_active(false);
                        let mult = context.cmd_buf_clear().unwrap_or(1);
                        self.component.set_movement(PageMovement::PageDown(mult));
                        return true;
                    }
                    UIEvent::Input(ref key)
                        if shortcut!(key == shortcuts[Shortcuts::GENERAL]["home_page"]) =>
                    {
                        self.component.set_modifier_active(false);
                        _ = context.cmd_buf_clear();
                        self.component.set_movement(PageMovement::Home);
                        return true;
                    }
                    UIEvent::Input(ref key)
                        if shortcut!(key == shortcuts[Shortcuts::GENERAL]["end_page"]) =>
                    {
                        self.component.set_modifier_active(false);
                        _ = context.cmd_buf_clear();
                        self.component.set_movement(PageMovement::End);
                        return true;
                    }
                    UIEvent::Input(ref key)
                        if shortcut!(key == shortcuts[Shortcuts::LISTING]["search"]) =>
                    {
                        context
                            .replies
                            .push_back(UIEvent::CmdInput(Key::Paste("search ".to_string())));
                        context
                            .replies
                            .push_back(UIEvent::ChangeMode(UIMode::Command));
                        return true;
                    }
                    UIEvent::Input(ref key)
                        if shortcut!(key == shortcuts[Shortcuts::LISTING]["set_seen"]) =>
                    {
                        let mut event = UIEvent::Action(Action::Listing(ListingAction::SetSeen));
                        if self.process_event(&mut event, context) {
                            return true;
                        }
                    }
                    UIEvent::Input(ref key)
                        if shortcut!(key == shortcuts[Shortcuts::LISTING]["send_to_trash"]) =>
                    {
                        let mut event =
                            UIEvent::Action(Action::Listing(ListingAction::SendToTrash));
                        if self.process_event(&mut event, context) {
                            return true;
                        }
                    }
                    UIEvent::Input(ref key)
                        if shortcut!(key == shortcuts[Shortcuts::LISTING]["refresh"]) =>
                    {
                        let account = &mut context.accounts[self.cursor_pos.account];
                        if let MenuEntryCursor::Mailbox(idx) = self.cursor_pos.menu {
                            if let Some(&mailbox_hash) = account.mailboxes_order.get(idx) {
                                if let Err(err) = account.refresh(mailbox_hash) {
                                    context.replies.push_back(UIEvent::Notification {
                                        title: Some("Could not refresh.".into()),
                                        source: None,
                                        body: err.to_string().into(),
                                        kind: Some(NotificationType::Error(err.kind)),
                                    });
                                }
                            }
                        }
                        return true;
                    }
                    UIEvent::Input(ref key)
                        if !self.component.unfocused()
                            && shortcut!(
                                key == shortcuts[Shortcuts::LISTING]["union_modifier"]
                            )
                            && self.component.modifier_active() =>
                    {
                        self.component.set_modifier_command(Some(Modifier::Union));
                        if let Some(cmd_buf) = context.cmd_buf() {
                            context
                                .replies
                                .push_back(UIEvent::StatusEvent(StatusEvent::BufSet(
                                    if let Some(modf) = self.component.modifier_command() {
                                        format!("{modf} {cmd_buf}")
                                    } else {
                                        cmd_buf.to_string()
                                    },
                                )));
                        }
                        return true;
                    }
                    UIEvent::Input(ref key)
                        if !self.component.unfocused()
                            && shortcut!(key == shortcuts[Shortcuts::LISTING]["diff_modifier"])
                            && self.component.modifier_active() =>
                    {
                        self.component
                            .set_modifier_command(Some(Modifier::Difference));
                        if let Some(cmd_buf) = context.cmd_buf() {
                            context
                                .replies
                                .push_back(UIEvent::StatusEvent(StatusEvent::BufSet(
                                    if let Some(modf) = self.component.modifier_command() {
                                        format!("{modf} {cmd_buf}")
                                    } else {
                                        cmd_buf.to_string()
                                    },
                                )));
                        }
                        return true;
                    }
                    UIEvent::Input(ref key)
                        if !self.component.unfocused()
                            && shortcut!(
                                key == shortcuts[Shortcuts::LISTING]["intersection_modifier"]
                            )
                            && self.component.modifier_active() =>
                    {
                        self.component
                            .set_modifier_command(Some(Modifier::Intersection));
                        if let Some(cmd_buf) = context.cmd_buf() {
                            context
                                .replies
                                .push_back(UIEvent::StatusEvent(StatusEvent::BufSet(
                                    if let Some(modf) = self.component.modifier_command() {
                                        format!("{modf} {cmd_buf}")
                                    } else {
                                        cmd_buf.to_string()
                                    },
                                )));
                        }
                        return true;
                    }
                    UIEvent::Input(ref key)
                        if self.component.unfocused()
                            && shortcut!(key == shortcuts[Shortcuts::LISTING]["next_entry"]) =>
                    {
                        self.component.next_entry(context);
                        return true;
                    }
                    UIEvent::Input(ref key)
                        if self.component.unfocused()
                            && shortcut!(
                                key == shortcuts[Shortcuts::LISTING]["previous_entry"]
                            ) =>
                    {
                        self.component.prev_entry(context);
                        return true;
                    }
                    UIEvent::Input(Key::Esc) | UIEvent::Input(Key::Char('\x1b'))
                        if !self.component.unfocused()
                            && (context.cmd_buf().is_some()
                                || self.component.modifier_active()
                                || !self.component.get_focused_items(context).is_empty()) =>
                    {
                        // Clear command buffer.
                        _ = context.cmd_buf_clear();
                        self.component.set_modifier_active(false);
                        // Clear selection.
                        let row_updates: SmallVec<[EnvelopeHash; 8]> =
                            self.component.get_focused_items(context);
                        for h in &row_updates {
                            if let Some(val) = self.component.selection_mut().get_mut(h) {
                                *val = false;
                            }
                        }
                        self.component.row_updates().extend(row_updates);
                        self.component.set_dirty(true);
                        return true;
                    }
                    UIEvent::Input(Key::Char(c)) if c.is_ascii_digit() => {
                        self.component.set_modifier_active(true);
                        context.cmd_buf_push(*c, self.component.modifier_command());
                        return true;
                    }
                    _ => {}
                }
            }
        } else if self.focus == ListingFocus::Menu {
            match *event {
                UIEvent::Input(ref k)
                    if shortcut!(k == shortcuts[Shortcuts::LISTING]["focus_right"]) =>
                {
                    // Right from layout1 switches the layout directly:
                    // open the cursor entry — a single mail → layout2, a
                    // thread → layout3 — with the focus on the grid
                    // (mirroring `open_mailbox` for the mailbox switch).
                    self.cursor_pos = self.menu_cursor_pos;
                    self.change_account(context);
                    self.focus = ListingFocus::MailList;
                    self.component.set_grid_has_keyboard(true);
                    self.component.set_focus(Focus::Entry, context);
                    self.set_dirty(true);
                    context
                        .replies
                        .push_back(UIEvent::StatusEvent(StatusEvent::ScrollUpdate(
                            ScrollUpdate::End(self.id),
                        )));
                    self.push_status_watch(
                        self.status(context),
                        self.status_watch(),
                        &mut context.replies,
                    );
                    return true;
                }
                UIEvent::Input(ref k)
                    if shortcut!(k == shortcuts[Shortcuts::LISTING]["open_mailbox"])
                        && self.menu_cursor_pos.menu == MenuEntryCursor::Status =>
                {
                    self.cursor_pos = self.menu_cursor_pos;
                    self.change_account(context);
                    self.set_dirty(true);
                    self.focus = ListingFocus::MailList;
                    self.component.set_grid_has_keyboard(true);
                    context
                        .replies
                        .push_back(UIEvent::StatusEvent(StatusEvent::ScrollUpdate(
                            ScrollUpdate::End(self.id),
                        )));
                    return true;
                }
                UIEvent::Input(ref k)
                    if shortcut!(k == shortcuts[Shortcuts::LISTING]["toggle_mailbox_collapse"])
                        && matches!(self.menu_cursor_pos.menu, MenuEntryCursor::Mailbox(_)) =>
                {
                    let target_mailbox_idx =
                        if let MenuEntryCursor::Mailbox(idx) = self.menu_cursor_pos.menu {
                            idx
                        } else {
                            return false;
                        };
                    if let Some(target) = self.accounts[self.menu_cursor_pos.account]
                        .entries
                        .get_mut(target_mailbox_idx)
                    {
                        target.collapsed = !(target.collapsed);
                        self.dirty = true;
                        self.menu.grid_mut().empty();
                        context
                            .replies
                            .push_back(UIEvent::StatusEvent(StatusEvent::ScrollUpdate(
                                ScrollUpdate::End(self.id),
                            )));
                        return true;
                    }
                    return false;
                }
                UIEvent::Input(ref k)
                    if shortcut!(k == shortcuts[Shortcuts::LISTING]["open_mailbox"]) =>
                {
                    self.cursor_pos = self.menu_cursor_pos;
                    self.change_account(context);
                    self.focus = ListingFocus::MailList;
                    self.component.set_grid_has_keyboard(true);
                    self.set_dirty(true);
                    context
                        .replies
                        .push_back(UIEvent::StatusEvent(StatusEvent::ScrollUpdate(
                            ScrollUpdate::End(self.id),
                        )));
                    self.push_status_watch(
                        self.status(context),
                        self.status_watch(),
                        &mut context.replies,
                    );
                    return true;
                }
                UIEvent::Input(ref k)
                    if shortcut!(k == shortcuts[Shortcuts::LISTING]["scroll_up"])
                        || shortcut!(k == shortcuts[Shortcuts::LISTING]["scroll_down"]) =>
                {
                    self.component.set_modifier_active(false);
                    let mut amount = context.cmd_buf_clear().unwrap_or(1);
                    if shortcut!(k == shortcuts[Shortcuts::LISTING]["scroll_up"]) {
                        while amount > 0 {
                            match self.menu_cursor_pos {
                                CursorPos {
                                    ref mut account,
                                    menu: MenuEntryCursor::Status,
                                } => {
                                    if *account > 0 {
                                        *account -= 1;
                                        self.menu_cursor_pos.menu =
                                            if self.accounts[*account].entries.is_empty() {
                                                MenuEntryCursor::Status
                                            } else {
                                                MenuEntryCursor::Mailbox(
                                                    self.accounts[*account]
                                                        .entries
                                                        .len()
                                                        .saturating_sub(1),
                                                )
                                            };
                                    } else {
                                        return true;
                                    }
                                }
                                CursorPos {
                                    ref account,
                                    menu: MenuEntryCursor::Mailbox(ref mut mailbox_idx),
                                } => loop {
                                    if *mailbox_idx > 0 {
                                        *mailbox_idx -= 1;
                                        if self.accounts[*account].entries[*mailbox_idx].visible {
                                            break;
                                        }
                                    } else {
                                        self.menu_cursor_pos.menu = MenuEntryCursor::Status;
                                        break;
                                    }
                                },
                            }

                            amount -= 1;
                        }
                    } else if shortcut!(k == shortcuts[Shortcuts::LISTING]["scroll_down"]) {
                        while amount > 0 {
                            match self.menu_cursor_pos {
                                // If current account has mailboxes, go to first mailbox
                                CursorPos {
                                    ref account,
                                    ref mut menu,
                                } if !self.accounts[*account].entries.is_empty()
                                    && *menu == MenuEntryCursor::Status =>
                                {
                                    if let Some(idx) = context.accounts[*account]
                                        .default_mailbox()
                                        .and_then(|h| self.accounts[*account].entry_by_hash(h))
                                    {
                                        *menu = MenuEntryCursor::Mailbox(idx);
                                    }
                                }
                                // If current account has no mailboxes, go to next account
                                CursorPos {
                                    ref mut account,
                                    ref mut menu,
                                } if *account + 1 < self.accounts.len()
                                    && *menu == MenuEntryCursor::Status =>
                                {
                                    *account += 1;
                                    *menu = MenuEntryCursor::Status;
                                }
                                // If current account has no mailboxes and there is no next account,
                                // return true
                                CursorPos {
                                    menu: MenuEntryCursor::Status,
                                    ..
                                } => {
                                    return true;
                                }
                                CursorPos {
                                    ref mut account,
                                    menu: MenuEntryCursor::Mailbox(ref mut mailbox_idx),
                                } => loop {
                                    if (*mailbox_idx + 1) < self.accounts[*account].entries.len() {
                                        *mailbox_idx += 1;
                                        if self.accounts[*account].entries[*mailbox_idx].visible {
                                            break;
                                        }
                                    } else if *account + 1 < self.accounts.len() {
                                        *account += 1;
                                        self.menu_cursor_pos.menu = MenuEntryCursor::Status;
                                        break;
                                    } else {
                                        return true;
                                    }
                                },
                            }

                            amount -= 1;
                        }
                    }
                    // Layout1: moving the mailbox selection switches the
                    // mailbox right away — the grid follows, the focus
                    // stays on the mailbox list.
                    if self.menu_cursor_pos != self.cursor_pos {
                        self.cursor_pos = self.menu_cursor_pos;
                        self.change_account(context);
                        self.focus_menu();
                    }
                    if self.show_menu_scrollbar != ShowMenuScrollbar::Never {
                        self.menu_scrollbar_show_timer.rearm();
                        self.show_menu_scrollbar = ShowMenuScrollbar::True;
                    }
                    self.menu.grid_mut().empty();
                    self.set_dirty(true);
                    return true;
                }
                UIEvent::Input(ref k)
                    if shortcut!(k == shortcuts[Shortcuts::LISTING]["next_mailbox"])
                        || shortcut!(k == shortcuts[Shortcuts::LISTING]["prev_mailbox"]) =>
                {
                    self.component.set_modifier_active(false);
                    let amount = context.cmd_buf_clear().unwrap_or(1);
                    let target = match k {
                        k if shortcut!(k == shortcuts[Shortcuts::LISTING]["next_mailbox"]) => {
                            match self.menu_cursor_pos.menu {
                                MenuEntryCursor::Status => amount.saturating_sub(1),
                                MenuEntryCursor::Mailbox(idx) => idx + amount,
                            }
                        }
                        k if shortcut!(k == shortcuts[Shortcuts::LISTING]["prev_mailbox"]) => {
                            match self.menu_cursor_pos.menu {
                                MenuEntryCursor::Status => {
                                    return true;
                                }
                                MenuEntryCursor::Mailbox(idx) => {
                                    if idx >= amount {
                                        idx - amount
                                    } else {
                                        return true;
                                    }
                                }
                            }
                        }
                        _ => return true,
                    };
                    if self.accounts[self.menu_cursor_pos.account]
                        .entries
                        .get(target)
                        .is_some()
                    {
                        self.menu_cursor_pos.menu = MenuEntryCursor::Mailbox(target)
                    } else {
                        return true;
                    }
                    if self.show_menu_scrollbar != ShowMenuScrollbar::Never {
                        self.menu_scrollbar_show_timer.rearm();
                        self.show_menu_scrollbar = ShowMenuScrollbar::True;
                    }
                    self.menu.grid_mut().empty();
                    return true;
                }
                UIEvent::Input(ref k)
                    if shortcut!(k == shortcuts[Shortcuts::LISTING]["next_account"])
                        || shortcut!(k == shortcuts[Shortcuts::LISTING]["prev_account"])
                        || shortcut!(k == shortcuts[Shortcuts::LISTING]["next_page"])
                        || shortcut!(k == shortcuts[Shortcuts::LISTING]["prev_page"]) =>
                {
                    self.component.set_modifier_active(false);
                    let amount = context.cmd_buf_clear().unwrap_or(1);
                    match k {
                        k if shortcut!(k == shortcuts[Shortcuts::LISTING]["next_account"])
                            || shortcut!(k == shortcuts[Shortcuts::LISTING]["next_page"]) =>
                        {
                            if self.menu_cursor_pos.account + amount >= self.accounts.len() {
                                // Go to last mailbox.
                                self.menu_cursor_pos.menu = if self.accounts
                                    [self.menu_cursor_pos.account]
                                    .entries
                                    .is_empty()
                                {
                                    MenuEntryCursor::Status
                                } else {
                                    MenuEntryCursor::Mailbox(
                                        self.accounts[self.menu_cursor_pos.account]
                                            .entries
                                            .len()
                                            .saturating_sub(1),
                                    )
                                };
                            } else if self.menu_cursor_pos.account + amount < self.accounts.len() {
                                self.menu_cursor_pos.account += amount;
                                let _new_val = self.menu_cursor_pos.account;
                                self.menu_cursor_pos.menu = if let Some(idx) = context.accounts
                                    [_new_val]
                                    .default_mailbox()
                                    .and_then(|h| self.accounts[_new_val].entry_by_hash(h))
                                {
                                    MenuEntryCursor::Mailbox(idx)
                                } else {
                                    MenuEntryCursor::Status
                                };
                            } else {
                                return true;
                            }
                        }
                        k if shortcut!(k == shortcuts[Shortcuts::LISTING]["prev_account"])
                            || shortcut!(k == shortcuts[Shortcuts::LISTING]["prev_page"]) =>
                        {
                            if self.menu_cursor_pos.account >= amount {
                                self.menu_cursor_pos.account -= amount;
                                let _new_val = self.menu_cursor_pos.account;
                                self.menu_cursor_pos.menu = if let Some(idx) = context.accounts
                                    [_new_val]
                                    .default_mailbox()
                                    .and_then(|h| self.accounts[_new_val].entry_by_hash(h))
                                {
                                    MenuEntryCursor::Mailbox(idx)
                                } else {
                                    MenuEntryCursor::Status
                                };
                            } else {
                                return true;
                            }
                        }
                        _ => return false,
                    }
                    if self.show_menu_scrollbar != ShowMenuScrollbar::Never {
                        self.menu_scrollbar_show_timer.rearm();
                        self.show_menu_scrollbar = ShowMenuScrollbar::True;
                    }
                    self.menu.grid_mut().empty();
                    self.set_dirty(true);

                    return true;
                }
                UIEvent::Input(ref key)
                    if shortcut!(key == shortcuts[Shortcuts::GENERAL]["home_page"]) =>
                {
                    if matches!(
                        self.menu_cursor_pos,
                        CursorPos {
                            account: 0,
                            menu: MenuEntryCursor::Mailbox(0)
                        }
                    ) {
                        // Can't go anywhere upwards, we're on top already.
                        return true;
                    }
                    match (
                        self.menu_cursor_pos.menu,
                        context.accounts[self.menu_cursor_pos.account]
                            .default_mailbox()
                            .and_then(|h| {
                                self.accounts[self.menu_cursor_pos.account].entry_by_hash(h)
                            }),
                    ) {
                        (MenuEntryCursor::Mailbox(0), _) => {
                            self.menu_cursor_pos.account = 0;
                        }
                        (MenuEntryCursor::Mailbox(_), Some(v)) => {
                            self.menu_cursor_pos.menu = MenuEntryCursor::Mailbox(v);
                        }
                        _ => return true,
                    }
                    if self.show_menu_scrollbar != ShowMenuScrollbar::Never {
                        self.menu_scrollbar_show_timer.rearm();
                        self.show_menu_scrollbar = ShowMenuScrollbar::True;
                    }
                    self.menu.grid_mut().empty();
                    self.set_dirty(true);
                    return true;
                }
                UIEvent::Input(ref key)
                    if shortcut!(key == shortcuts[Shortcuts::GENERAL]["end_page"]) =>
                {
                    let CursorPos {
                        ref mut account,
                        ref mut menu,
                    } = self.menu_cursor_pos;
                    if matches!(
                        (*account, *menu),
                        (a, MenuEntryCursor::Mailbox(
                            i
                        )) if a == self.accounts.len().saturating_sub(1) && i ==
                            self.accounts[*account].entries.len().saturating_sub(1)
                    ) {
                        // Do nothing, this is the End.
                        // "Father?"
                        // "Yes, son?"
                        // "I want to kill you"
                        // "Come on, baby"
                        return true;
                    } else if matches!(
                        *menu,
                        MenuEntryCursor::Mailbox(
                            i
                        ) if i ==
                            self.accounts[*account].entries.len().saturating_sub(1)
                    ) {
                        *account = self.accounts.len().saturating_sub(1);
                        *menu = if let Some(idx) = context.accounts[*account]
                            .default_mailbox()
                            .and_then(|h| self.accounts[*account].entry_by_hash(h))
                        {
                            MenuEntryCursor::Mailbox(idx)
                        } else {
                            MenuEntryCursor::Status
                        };
                    } else if !self.accounts[*account].entries.is_empty() {
                        *menu = MenuEntryCursor::Mailbox(
                            self.accounts[*account].entries.len().saturating_sub(1),
                        );
                    } else {
                        *menu = MenuEntryCursor::Status;
                    }
                    if self.show_menu_scrollbar != ShowMenuScrollbar::Never {
                        self.menu_scrollbar_show_timer.rearm();
                        self.show_menu_scrollbar = ShowMenuScrollbar::True;
                    }
                    self.menu.grid_mut().empty();
                    self.set_dirty(true);
                    return true;
                }
                _ => {}
            }
        }
        let (account_hash, mailbox_hash) = self.component.coordinates();
        match *event {
            UIEvent::Input(ref k) if shortcut!(k == shortcuts[Shortcuts::LISTING]["new_mail"]) => {
                let account_hash = context.accounts[self.cursor_pos.account].hash();
                let composer = Composer::with_account(account_hash, context);
                context
                    .replies
                    .push_back(UIEvent::Action(Tab(New(Some(Box::new(composer))))));
                return true;
            }
            UIEvent::Action(Action::Tab(ManageMailboxes)) => {
                let account_pos = self.cursor_pos.account;
                let mgr = MailboxManager::new(context, account_pos);
                context
                    .replies
                    .push_back(UIEvent::Action(Tab(New(Some(Box::new(mgr))))));
                return true;
            }
            UIEvent::Action(Action::Tab(ManageJobs)) => {
                let mgr = JobManager::new(context);
                context
                    .replies
                    .push_back(UIEvent::Action(Tab(New(Some(Box::new(mgr))))));
                return true;
            }
            UIEvent::Action(Action::Compose(ComposeAction::Mailto(ref mailto))) => {
                let account_hash = context.accounts[self.cursor_pos.account].hash();
                let mut composer = Composer::with_account(account_hash, context);
                composer.set_draft(mailto.into(), context);
                context
                    .replies
                    .push_back(UIEvent::Action(Tab(New(Some(Box::new(composer))))));
                return true;
            }
            UIEvent::StartupCheck(_)
            | UIEvent::MailboxUpdate(_)
            | UIEvent::EnvelopeUpdate(_)
            | UIEvent::EnvelopeRename(_, _)
            | UIEvent::EnvelopeRemove(_, _) => {
                self.dirty = true;
                // clear menu to force redraw
                self.menu.grid_mut().empty();
                self.push_status_watch(
                    self.status(context),
                    self.status_watch(),
                    &mut context.replies,
                );
            }
            UIEvent::Input(Key::Backspace) if context.cmd_buf().is_some() => {
                context.cmd_buf_pop(self.component.modifier_command());
                return true;
            }
            UIEvent::Input(Key::Esc) | UIEvent::Input(Key::Char('\x1b'))
                if context.cmd_buf().is_some() =>
            {
                self.component.set_modifier_active(false);
                _ = context.cmd_buf_clear();
                return true;
            }
            UIEvent::Input(Key::Char(c)) if c.is_ascii_digit() => {
                self.component.set_modifier_active(true);
                context.cmd_buf_push(c, self.component.modifier_command());
                return true;
            }
            UIEvent::Input(ref key)
                if mailbox_settings!(context has [account_hash][&mailbox_hash])
                    && mailbox_settings!(
                        context[account_hash][&mailbox_hash]
                            .shortcuts
                            .listing
                            .commands
                    )
                    .iter()
                    .any(|cmd| {
                        if cmd.shortcut == *key {
                            for cmd in &cmd.command {
                                context.replies.push_back(UIEvent::Command(cmd.to_string()));
                            }
                            return true;
                        }
                        false
                    }) =>
            {
                return true;
            }
            _ => {}
        }
        // Forward unhandled events to self.component if that hasn't happened already.
        if !(have_forwarded_to_component
            || (matches!(self.focus, ListingFocus::MailList | ListingFocus::View)
                && self.status.is_none()
                && self.component.unfocused()))
        {
            self.component.process_event(event, context);
        }
        false
    }

    fn is_dirty(&self) -> bool {
        self.dirty
            || self
                .status
                .as_ref()
                .map(Component::is_dirty)
                .unwrap_or_else(|| self.component.is_dirty())
            || if self.component.unfocused() {
                self.view.as_ref().map(|v| v.is_dirty()).unwrap_or(false)
            } else {
                self.component.is_dirty()
            }
    }

    fn set_dirty(&mut self, value: bool) {
        self.dirty = value;
        if let Some(s) = self.status.as_mut() {
            s.set_dirty(value);
        } else {
            self.component.set_dirty(value);
            if self.component.unfocused() {
                if let Some(ref mut view) = self.view {
                    view.set_dirty(value);
                }
            }
        }
    }

    fn shortcuts(&self, context: &Context) -> ShortcutMaps {
        let mut map = ShortcutMaps::default();
        if self.focus != ListingFocus::Menu && self.component.unfocused() {
            if let Some(ref view) = self.view {
                map.extend_shortcuts(view.shortcuts(context));
            }
        }
        map.extend_shortcuts(if let Some(s) = self.status.as_ref() {
            s.shortcuts(context)
        } else {
            self.component.shortcuts(context)
        });
        let mut config_map = context.settings.shortcuts.listing.key_values();
        if self.focus != ListingFocus::Menu {
            config_map.shift_remove("open_mailbox");
        }
        let (account_hash, mailbox_hash) = self.component.coordinates();
        if mailbox_settings!(context has [account_hash][&mailbox_hash]) {
            for command in mailbox_settings!(
                context[account_hash][&mailbox_hash]
                    .shortcuts
                    .listing
                    .commands
            ) {
                // Shadow only the colliding key: a `ShortcutKeys` binding
                // holds up to two keys (`"Down,j"`), and dropping the
                // whole field would silently disable the sibling key too.
                config_map.retain(|_, shortcut| {
                    shortcut.0.retain(|k| k != &command.shortcut);
                    !shortcut.0.is_empty()
                });
            }
        }
        map.insert(Shortcuts::LISTING, config_map);

        map
    }

    fn id(&self) -> ComponentId {
        self.id
    }

    fn status_watch(&self) -> Option<(AccountHash, MailboxHash)> {
        let CursorPos {
            account,
            menu: MenuEntryCursor::Mailbox(idx),
        } = self.cursor_pos
        else {
            return None;
        };
        let entry = self.accounts.get(account)?;
        let mailbox = entry.entries.get(idx)?;
        Some((entry.hash, mailbox.mailbox_hash))
    }

    fn children(&self) -> IndexMap<ComponentId, &dyn Component> {
        let mut ret = IndexMap::default();
        ret.insert(
            self.component.id(),
            match &self.component {
                Compact(l) => l.as_component(),
                Conversations(l) => l.as_component(),
                Offline(l) => l.as_component(),
                Plain(l) => l.as_component(),
                Threaded(l) => l.as_component(),
            },
        );

        ret
    }

    fn children_mut(&mut self) -> IndexMap<ComponentId, &mut dyn Component> {
        let mut ret = IndexMap::default();
        ret.insert(
            self.component.id(),
            match &mut self.component {
                Compact(l) => l.as_component_mut(),
                Conversations(l) => l.as_component_mut(),
                Offline(l) => l.as_component_mut(),
                Plain(l) => l.as_component_mut(),
                Threaded(l) => l.as_component_mut(),
            },
        );

        ret
    }
}

impl Listing {
    /// Whether the offline placeholder should replace the listing
    /// component while drawing: only when the account is offline *and* has
    /// no mailbox list to show. A cached mailbox list (loaded by the
    /// cache-first `Mailboxes` job at cold start) means the listing can
    /// already render offline content — the envelopes come from the sqlite3
    /// offline cache — so replacing it with the placeholder would hide the
    /// cached mail behind `offline: …` until the (possibly slow) connect
    /// attempt succeeds.
    fn should_replace_with_offline(context: &mut Context, account_hash: AccountHash) -> bool {
        context.is_online(account_hash).is_err()
            && context.accounts[&account_hash].list_mailboxes().is_empty()
    }

    /// Emit both the legacy `UpdateStatus` string and the structured
    /// `FocusMailbox` event for the active cursor, so the status bar
    /// stays in sync with the focused mailbox as the cursor moves inside
    /// the listing. Centralised helper for the eight call sites that
    /// updated the status string before this refactor. The status string
    /// and the `(AccountHash, MailboxHash)` are passed in (rather than
    /// read from `context`) so the caller can release the immutable
    /// `&Context` borrow before mutably borrowing `context.replies`.
    pub fn push_status_watch(
        &self,
        status: String,
        status_watch: Option<(AccountHash, MailboxHash)>,
        replies: &mut VecDeque<UIEvent>,
    ) {
        replies.push_back(UIEvent::StatusEvent(StatusEvent::UpdateStatus(status)));
        if let Some((acc_hash, mb_hash)) = status_watch {
            replies.push_back(UIEvent::StatusEvent(StatusEvent::FocusMailbox(
                acc_hash, mb_hash,
            )));
        }
    }

    pub fn new(context: &mut Context) -> Self {
        let account_entries: Vec<AccountMenuEntry> = context
            .accounts
            .iter()
            .enumerate()
            .map(|(i, (h, a))| {
                let entries: SmallVec<[MailboxMenuEntry; 16]> = a
                    .list_mailboxes()
                    .into_iter()
                    .filter(|mailbox_node| a[&mailbox_node.hash].ref_mailbox.is_subscribed())
                    .map(|f| MailboxMenuEntry {
                        depth: f.depth,
                        indentation: f.indentation,
                        has_sibling: f.has_sibling,
                        mailbox_hash: f.hash,
                        visible: true,
                        collapsed: a[&f.hash].conf.collapsed,
                        index_style: a[&f.hash].conf.conf_override().listing.index_style,
                    })
                    .collect::<_>();

                AccountMenuEntry {
                    name: a.name().to_string(),
                    hash: *h,
                    index: i,
                    entries,
                }
            })
            .collect();
        let first_account_hash = account_entries[0].hash;
        // The sidebar is hidden on launch when the account setting says so;
        // its visibility also decides which pane owns the initial keyboard
        // focus. A visible sidebar (the layout1 default) is focused so the
        // first keystrokes navigate the mailbox list, while a hidden sidebar
        // leaves the focus on the mail list grid.
        let menu_visibility =
            !*account_settings!(context[first_account_hash].listing.hide_sidebar_on_launch);
        let mut ret = Self {
            last_opened_env: None,
            component: Offline(OfflineListing::new((
                first_account_hash,
                MailboxHash::default(),
            ))),
            view: None,
            accounts: account_entries,
            status: None,
            dirty: true,
            cursor_pos: CursorPos {
                account: 0,
                menu: MenuEntryCursor::Mailbox(0),
            },
            menu_cursor_pos: CursorPos {
                account: 0,
                menu: MenuEntryCursor::Mailbox(0),
            },
            menu: Screen::<Virtual>::new(crate::conf::value(context, "mail.sidebar")),
            menu_scrollbar_show_timer: context.main_loop_handler.job_executor.clone().create_timer(
                std::time::Duration::from_secs(0),
                std::time::Duration::from_millis(1200),
            ),
            show_menu_scrollbar: ShowMenuScrollbar::Never,
            startup_checks_rate: RateLimit::new(
                2,
                1000,
                context.main_loop_handler.job_executor.clone(),
            ),
            theme_default: conf::value(context, "theme_default"),
            id: ComponentId::default(),
            sidebar_divider: *account_settings!(
                context[first_account_hash].listing.sidebar_divider
            ),
            sidebar_divider_theme: conf::value(context, "mail.sidebar_divider"),
            menu_visibility,
            focus: if menu_visibility {
                ListingFocus::Menu
            } else {
                ListingFocus::MailList
            },
            prev_menu_visible: menu_visibility,
            prev_view_fullscreen: false,
        };
        ret.component.realize(ret.id().into(), context);
        {
            let _new_val = ret.cursor_pos.account;
            if let Some(idx) = context.accounts[_new_val]
                .default_mailbox()
                .and_then(|h| ret.accounts[_new_val].entry_by_hash(h))
            {
                ret.cursor_pos.menu = MenuEntryCursor::Mailbox(idx);
                ret.menu_cursor_pos.menu = MenuEntryCursor::Mailbox(idx);
            }
        }
        ret.change_account(context);
        // `change_account` switches to the account's index style, and
        // `set_index_style` calls `close_view`, which by contract lands the
        // keyboard on the grid. Re-assert the launch focus here so a visible
        // sidebar still owns the keyboard once construction settles; the
        // field initializer above covers the no-mailbox/offline path that
        // never reaches `set_index_style`.
        ret.focus = if menu_visibility {
            ListingFocus::Menu
        } else {
            ListingFocus::MailList
        };
        // Keep `grid_has_keyboard` in sync with the launch focus (see
        // `focus_menu`): the pane background highlights the keyboard holder.
        ret.component
            .set_grid_has_keyboard(matches!(ret.focus, ListingFocus::MailList));
        ret
    }

    fn draw_menu(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        // The whole pane must fall back to the focused-pane background, not
        // the (content-area) `theme_default`: rows not covered by the menu
        // buffer (short folder trees) and the per-account spacing rows must
        // keep the pane background when the theme changes. The background
        // follows the keyboard focus ("pane.focused" while the sidebar owns
        // it, "pane.unfocused" otherwise); entry strings keep their own
        // `mail.sidebar*` colors on top of it.
        let pane_fill = crate::conf::value(
            context,
            if matches!(self.focus, ListingFocus::Menu) {
                "pane.focused"
            } else {
                "pane.unfocused"
            },
        );
        grid.clear_area(area, pane_fill);
        let total_height: usize = 3 * (self.accounts.len())
            + self
                .accounts
                .iter()
                .map(|entry| entry.height())
                .sum::<usize>();
        let min_width: usize = area.width();
        let (width, height) = self.menu.grid().size();
        let cursor = match self.focus {
            ListingFocus::MailList | ListingFocus::View => self.cursor_pos,
            ListingFocus::Menu => self.menu_cursor_pos,
        };
        if min_width > width || height < total_height || self.dirty {
            let _ = self.menu.resize(min_width, total_height);
            // Re-fill the offscreen menu buffer with the *current*
            // sidebar theme: `CellBuffer::resize` grows with the
            // construction-time default cell, which carries the theme
            // that was active when the `Screen` was created, so stale
            // colors would leak through around `write_string` runs
            // (account title tails, spacing rows) after a theme switch.
            let menu_area = self.menu.area();
            self.menu.grid_mut().clear_area(menu_area, pane_fill);
            let mut y = 0;
            for a in 0..self.accounts.len() {
                let menu_area = self.menu.area().skip_rows(y);
                y += self.print_account(menu_area, a, context);
                y += 3;
            }
        }

        let rows = area.height();
        /* `area` is the menu pane's inner area (inside the rounded frame
         * ring), so a pane shorter than the ring (outer height < 4) can
         * yield zero rows here. The `skip_offset` computation below
         * divides by `rows`, so bail out early instead of panicking. */
        if rows == 0 {
            return;
        }
        const SCROLLING_CONTEXT: usize = 3;
        let y_offset = (cursor.account)
            + self
                .accounts
                .iter()
                .take(cursor.account)
                .map(|entry| entry.height())
                .sum::<usize>()
            + self
                .accounts
                .get(cursor.account)
                .map(|acc| {
                    acc.cursor_y_offset(match cursor.menu {
                        MenuEntryCursor::Status => 0,
                        MenuEntryCursor::Mailbox(idx) => idx + 1,
                    })
                })
                .unwrap_or_default()
            + SCROLLING_CONTEXT;
        let skip_offset = if y_offset <= rows {
            0
        } else {
            rows * y_offset.wrapping_div(rows).saturating_sub(1) + y_offset.wrapping_rem(rows)
        };

        grid.copy_area(
            self.menu.grid(),
            /* `area` is already the pane's inner area inside the rounded
             * frame ring, drawn before this call by Listing::draw, so the
             * menu content is copied as-is with no extra inset. */
            area,
            self.menu
                .area()
                .skip_rows(skip_offset.min((self.menu.area().height() - 1).saturating_sub(rows)))
                .take_rows((skip_offset + rows).min(self.menu.area().height() - 1)),
        );
        if self.show_menu_scrollbar == ShowMenuScrollbar::True && total_height > rows {
            if self.focus == ListingFocus::Menu {
                context
                    .replies
                    .push_back(UIEvent::StatusEvent(StatusEvent::ScrollUpdate(
                        ScrollUpdate::Update {
                            id: self.id,
                            context: ScrollContext {
                                shown_lines: skip_offset + rows,
                                total_lines: total_height,
                                has_more_lines: false,
                            },
                        },
                    )));
            }
            ScrollBar::default().set_show_arrows(true).draw(
                grid,
                area.nth_col(area.width().saturating_sub(1)),
                context,
                // position
                skip_offset,
                // visible_rows
                rows,
                // length
                total_height,
            );
        } else if total_height < rows {
            context
                .replies
                .push_back(UIEvent::StatusEvent(StatusEvent::ScrollUpdate(
                    ScrollUpdate::End(self.id),
                )));
        }

        context.dirty_areas.push_back(area);
    }

    /// Print a single account in the menu area.
    fn print_account(&mut self, mut area: Area, aidx: usize, context: &Context) -> usize {
        let account_y = self.menu.area().height() - area.height();
        #[derive(Clone, Copy, Debug)]
        struct Line {
            collapsed: bool,
            depth: usize,
            inc: isize,
            indentation: u32,
            has_sibling: bool,
            mailbox_idx: MailboxHash,
            count: Option<usize>,
            collapsed_count: Option<usize>,
        }
        // Each entry and its index in the account
        let mailboxes: HashMap<MailboxHash, Mailbox> = context.accounts[self.accounts[aidx].index]
            .mailbox_entries
            .iter()
            .map(|(&hash, entry)| (hash, entry.ref_mailbox.clone()))
            .collect();

        let cursor = match self.focus {
            ListingFocus::MailList | ListingFocus::View => self.cursor_pos,
            ListingFocus::Menu => self.menu_cursor_pos,
        };

        let must_highlight_account: bool = cursor.account == self.accounts[aidx].index;

        let mut lines: Vec<Line> = Vec::new();
        let mail_sidebar_highlighted_value =
            crate::conf::value(context, "mail.sidebar_highlighted");
        let mail_sidebar_highlighted_account_name_value =
            crate::conf::value(context, "mail.sidebar_highlighted_account_name");
        let mail_sidebar_account_name_value =
            crate::conf::value(context, "mail.sidebar_account_name");
        let mail_sidebar_highlighted_index_value =
            crate::conf::value(context, "mail.sidebar_highlighted_index");
        let mail_sidebar_highlighted_unread_count_value =
            crate::conf::value(context, "mail.sidebar_highlighted_unread_count");
        let mail_sidebar_highlighted_account_value =
            crate::conf::value(context, "mail.sidebar_highlighted_account");
        let mail_sidebar_highlighted_account_index_value =
            crate::conf::value(context, "mail.sidebar_highlighted_account_index");
        let mail_sidebar_highlighted_account_unread_count_value =
            crate::conf::value(context, "mail.sidebar_highlighted_account_unread_count");
        let mail_sidebar_value = crate::conf::value(context, "mail.sidebar");
        let mail_sidebar_index_value = crate::conf::value(context, "mail.sidebar_index");
        let mail_sidebar_unread_count_value =
            crate::conf::value(context, "mail.sidebar_unread_count");
        // Base entry rows sit directly on the pane background, like the
        // blank cells of `draw_menu`: they keep their `mail.sidebar*`
        // fg/attrs but their bg follows the pane, so an unfocused sidebar
        // dims as a whole. Only the cursor entry keeps its own
        // `mail.sidebar_highlighted*` fill so the selection stays legible
        // in both states; the active-account accents keep their fg accent
        // and ride the pane background too.
        let pane_fill = crate::conf::value(
            context,
            if matches!(self.focus, ListingFocus::Menu) {
                "pane.focused"
            } else {
                "pane.unfocused"
            },
        );
        let on_pane_bg = |attr: &ThemeAttribute| ThemeAttribute {
            bg: pane_fill.bg,
            ..*attr
        };
        let mail_sidebar_account_name_value = on_pane_bg(&mail_sidebar_account_name_value);
        let mail_sidebar_highlighted_account_name_value =
            on_pane_bg(&mail_sidebar_highlighted_account_name_value);
        let mail_sidebar_highlighted_account_value =
            on_pane_bg(&mail_sidebar_highlighted_account_value);
        let mail_sidebar_highlighted_account_index_value =
            on_pane_bg(&mail_sidebar_highlighted_account_index_value);
        let mail_sidebar_highlighted_account_unread_count_value =
            on_pane_bg(&mail_sidebar_highlighted_account_unread_count_value);
        let mail_sidebar_value = on_pane_bg(&mail_sidebar_value);
        let mail_sidebar_index_value = on_pane_bg(&mail_sidebar_index_value);
        let mail_sidebar_unread_count_value = on_pane_bg(&mail_sidebar_unread_count_value);
        let has_sibling_str: &str = account_settings!(
            context[self.accounts[aidx].hash]
                .listing
                .sidebar_mailbox_tree_has_sibling
        )
        .as_ref()
        .map(|s| s.as_str())
        .unwrap_or(" ");
        let no_sibling_str: &str = account_settings!(
            context[self.accounts[aidx].hash]
                .listing
                .sidebar_mailbox_tree_no_sibling
        )
        .as_ref()
        .map(|s| s.as_str())
        .unwrap_or(" ");

        let has_sibling_leaf_str: &str = account_settings!(
            context[self.accounts[aidx].hash]
                .listing
                .sidebar_mailbox_tree_has_sibling_leaf
        )
        .as_ref()
        .map(|s| s.as_str())
        .unwrap_or(" ");

        let no_sibling_leaf_str: &str = account_settings!(
            context[self.accounts[aidx].hash]
                .listing
                .sidebar_mailbox_tree_no_sibling_leaf
        )
        .as_ref()
        .map(|s| s.as_str())
        .unwrap_or(" ");
        let relative_menu_indices = *account_settings!(
            context[self.accounts[aidx].hash]
                .listing
                .relative_menu_indices
        );
        for (
            i,
            &MailboxMenuEntry {
                depth,
                indentation,
                has_sibling,
                mailbox_hash,
                visible: _,
                collapsed,
                index_style: _,
            },
        ) in self.accounts[aidx].entries.iter().enumerate()
        {
            if mailboxes[&mailbox_hash].is_subscribed() {
                match context.accounts[self.accounts[aidx].index][&mailbox_hash].status {
                    MailboxStatus::Failed(_) => {
                        lines.push(Line {
                            collapsed,
                            depth,
                            inc: i as isize,
                            indentation,
                            has_sibling,
                            mailbox_idx: mailbox_hash,
                            count: None,
                            collapsed_count: None,
                        });
                    }
                    _ => {
                        lines.push(Line {
                            collapsed,
                            depth,
                            inc: i as isize,
                            indentation,
                            has_sibling,
                            mailbox_idx: mailbox_hash,
                            count: mailboxes[&mailbox_hash].count().ok().map(|(v, _)| v),
                            collapsed_count: None,
                        });
                    }
                }
            }
        }

        let account_attrs = if must_highlight_account {
            if cursor.menu == MenuEntryCursor::Status {
                let mut v = mail_sidebar_highlighted_value;
                if !context.settings.terminal.use_color() {
                    v.attrs |= Attr::REVERSE;
                }
                v
            } else {
                mail_sidebar_highlighted_account_name_value
            }
        } else {
            mail_sidebar_account_name_value
        };
        // Print account name first
        let (account_name_width, _) = self.menu.grid_mut().write_string(
            &self.accounts[aidx].name,
            account_attrs.fg,
            account_attrs.bg,
            account_attrs.attrs,
            area,
            None,
            None,
        );
        // The account title row must carry its (possibly highlighted)
        // background to the end of the pane, like folder rows do via
        // their tail loop below; otherwise the cells past the account
        // name keep whatever the offscreen buffer held before.
        for c in self
            .menu
            .grid_mut()
            .row_iter(area, account_name_width..area.width(), 0)
        {
            self.menu.grid_mut()[c]
                .set_fg(account_attrs.fg)
                .set_bg(account_attrs.bg)
                .set_attrs(account_attrs.attrs);
        }
        area = self.menu.area().skip_rows(account_y);

        if lines.is_empty() {
            self.menu.grid_mut().write_string(
                "offline",
                crate::conf::value(context, "error_message").fg,
                account_attrs.bg,
                account_attrs.attrs,
                area.skip_rows(1),
                None,
                None,
            );
            return 0;
        }

        let lines_len = lines.len();
        let mut idx = 0;
        let mut branches = String::with_capacity(16);

        // What depth to skip if a mailbox is toggled to collapse
        // The value should be the collapsed mailbox's indentation, so that its children
        // are not visible.
        let mut skip: Option<usize> = None;
        let mut skipped_counter: usize = 0;
        'grid_loop: for y in 0..area.height() {
            if idx == lines_len {
                break;
            }
            let mut l = lines[idx];
            while let Some(p) = skip {
                if l.depth > p {
                    self.accounts[aidx].entries[idx].visible = false;
                    idx += 1;
                    skipped_counter += 1;
                    if idx >= lines.len() {
                        break 'grid_loop;
                    }
                    l = lines[idx];
                } else {
                    skip = None;
                }
            }
            self.accounts[aidx].entries[idx].visible = true;
            if l.collapsed {
                skip = Some(l.depth);
                // Calculate total unseen from hidden children mailboxes
                let mut idx = idx + 1;
                let mut counter = 0;
                while idx < lines.len() {
                    if lines[idx].depth <= l.depth {
                        break;
                    }
                    counter += lines[idx].count.unwrap_or(0);
                    idx += 1;
                }
                l.collapsed_count = Some(counter);
            }
            let (att, index_att, unread_count_att) = if must_highlight_account {
                if match cursor.menu {
                    MenuEntryCursor::Mailbox(c) => c == idx,
                    _ => false,
                } {
                    let mut ret = (
                        mail_sidebar_highlighted_value,
                        mail_sidebar_highlighted_index_value,
                        mail_sidebar_highlighted_unread_count_value,
                    );

                    if !context.settings.terminal.use_color() {
                        ret.0.attrs |= Attr::REVERSE;
                        ret.1.attrs |= Attr::REVERSE;
                        ret.2.attrs |= Attr::REVERSE;
                    }
                    ret
                } else {
                    (
                        mail_sidebar_highlighted_account_value,
                        mail_sidebar_highlighted_account_index_value,
                        mail_sidebar_highlighted_account_unread_count_value,
                    )
                }
            } else {
                (
                    mail_sidebar_value,
                    mail_sidebar_index_value,
                    mail_sidebar_unread_count_value,
                )
            };
            self.menu.grid_mut().change_theme(area.nth_row(y + 1), att);

            // Calculate how many columns the mailbox index tags should occupy with right
            // alignment, eg.
            //  1
            //  2
            // ...
            //  9
            // 10
            let total_mailbox_no_digits = {
                let mut len = lines_len;
                let mut ctr = 1;
                while len > 9 {
                    ctr += 1;
                    len /= 10;
                }
                ctr
            };

            let (x, _) = self.menu.grid_mut().write_string(
                &if relative_menu_indices && must_highlight_account {
                    format!(
                        "{:>width$}",
                        (l.inc - cursor.menu).abs(),
                        width = total_mailbox_no_digits
                    )
                } else {
                    format!("{:>width$}", l.inc, width = total_mailbox_no_digits)
                },
                index_att.fg,
                index_att.bg,
                index_att.attrs,
                area.nth_row(y + 1),
                None,
                None,
            );
            area = self.menu.area().skip_rows(account_y);
            {
                branches.clear();
                branches.push_str(no_sibling_str);
                let leading_zeros = l.indentation.leading_zeros();
                let mut o = 1_u32.wrapping_shl(31_u32.saturating_sub(leading_zeros));
                for _ in 0..(32_u32.saturating_sub(leading_zeros)) {
                    if l.indentation & o > 0 {
                        branches.push_str(has_sibling_str);
                    } else {
                        branches.push_str(no_sibling_str);
                    }
                    o >>= 1;
                }
                if l.depth > 0 {
                    if l.has_sibling {
                        branches.push_str(has_sibling_leaf_str);
                    } else {
                        branches.push_str(no_sibling_leaf_str);
                    }
                }
            }
            let x = self
                .menu
                .grid_mut()
                .write_string(
                    &branches,
                    att.fg,
                    att.bg,
                    att.attrs,
                    area.nth_row(y + 1).skip_cols(x),
                    None,
                    None,
                )
                .0
                + x;
            area = self.menu.area().skip_rows(account_y);
            let x = self
                .menu
                .grid_mut()
                .write_string(
                    context.accounts[self.accounts[aidx].index].mailbox_entries[&l.mailbox_idx]
                        .name(),
                    att.fg,
                    att.bg,
                    att.attrs,
                    area.nth_row(y + 1).skip_cols(x),
                    None,
                    None,
                )
                .0
                + x
                + 1;
            area = self.menu.area().skip_rows(account_y);

            // Unread message count
            let count_string: Cow<'static, str> = match (l.count, l.collapsed_count) {
                (None, None) if context.settings.terminal.ascii_drawing => "...".into(),
                (None, None) => "…".into(),
                (Some(0), None) => "".into(),
                (Some(0), Some(0)) | (None, Some(0)) => "v".into(),
                (Some(0), Some(coll)) => format!("({coll}) v").into(),
                (Some(c), Some(0)) => format!("{c} v").into(),
                (Some(c), Some(coll)) => format!("{c} ({coll}) v").into(),
                (Some(c), None) => format!("{c}").into(),
                (None, Some(coll)) => format!("({coll}) v").into(),
            };

            let skip_cols = {
                let val = area.width().saturating_sub(count_string.len());
                let skip_cols = x.min(val);
                if skip_cols == val && matches!(self.show_menu_scrollbar, ShowMenuScrollbar::True) {
                    skip_cols.saturating_sub(1)
                } else {
                    skip_cols
                }
            };
            let (x, _) = self.menu.grid_mut().write_string(
                count_string.as_ref(),
                unread_count_att.fg,
                unread_count_att.bg,
                unread_count_att.attrs
                    | if l.count.unwrap_or(0) > 0 {
                        Attr::BOLD
                    } else {
                        Attr::DEFAULT
                    },
                area.nth_row(y + 1).skip_cols(skip_cols),
                None,
                None,
            );
            area = self.menu.area().skip_rows(account_y);
            for c in self
                .menu
                .grid_mut()
                .row_iter(area, (x + skip_cols)..area.width(), y + 1)
            {
                self.menu.grid_mut()[c]
                    .set_fg(att.fg)
                    .set_bg(att.bg)
                    .set_attrs(att.attrs);
            }
            idx += 1;
        }
        if idx == 0 {
            0
        } else {
            idx - 1 - skipped_counter
        }
    }

    fn change_account(&mut self, context: &mut Context) {
        // The view belongs to the previous account/mailbox; close it so no
        // stale view (or sidebar occlusion) survives the switch.
        self.close_view(context);
        let account_hash = context.accounts[self.cursor_pos.account].hash();
        let previous_collapsed_mailboxes: BTreeSet<MailboxHash> = self.accounts
            [self.cursor_pos.account]
            .entries
            .iter()
            .filter_map(|e| {
                if e.collapsed {
                    Some(e.mailbox_hash)
                } else {
                    None
                }
            })
            .collect::<_>();
        let previous_index_styles: BTreeMap<MailboxHash, IndexStyle> = self.accounts
            [self.cursor_pos.account]
            .entries
            .iter()
            .filter_map(|e| Some((e.mailbox_hash, e.index_style?)))
            .collect::<_>();
        self.accounts[self.cursor_pos.account].entries = context.accounts[self.cursor_pos.account]
            .list_mailboxes()
            .into_iter()
            .filter(|mailbox_node| {
                context.accounts[self.cursor_pos.account][&mailbox_node.hash]
                    .ref_mailbox
                    .is_subscribed()
            })
            .map(|f| MailboxMenuEntry {
                depth: f.depth,
                indentation: f.indentation,
                has_sibling: f.has_sibling,
                mailbox_hash: f.hash,
                visible: true,
                collapsed: if previous_collapsed_mailboxes.is_empty() {
                    context.accounts[self.cursor_pos.account][&f.hash]
                        .conf
                        .collapsed
                } else {
                    previous_collapsed_mailboxes.contains(&f.hash)
                },
                index_style: previous_index_styles.get(&f.hash).copied(),
            })
            .collect::<_>();
        if let (
            ListingComponent::Offline(_),
            MenuEntryCursor::Mailbox(ref mut idx),
            Some(default),
        ) = (
            &self.component,
            &mut self.cursor_pos.menu,
            context.accounts[self.cursor_pos.account]
                .default_mailbox()
                .and_then(|h| self.accounts[self.cursor_pos.account].entry_by_hash(h)),
        ) {
            *idx = default;
            self.menu_cursor_pos.menu = MenuEntryCursor::Mailbox(default);
        }
        match self.cursor_pos.menu {
            MenuEntryCursor::Mailbox(idx) => {
                // Account might have no mailboxes yet if it's offline
                if let Some(MailboxMenuEntry {
                    mailbox_hash,
                    index_style,
                    ..
                }) = self.accounts[self.cursor_pos.account].entries.get(idx)
                {
                    self.component
                        .process_event(&mut UIEvent::VisibilityChange(false), context);
                    self.component
                        .set_coordinates((account_hash, *mailbox_hash));
                    self.component.refresh_mailbox(context, true);

                    // Check if per-mailbox configuration overrides general configuration
                    let index_style_override =
                        *mailbox_settings!(context[account_hash][mailbox_hash].listing.index_style);
                    self.set_index_style(index_style.unwrap_or(index_style_override), context);
                } else if !matches!(self.component, ListingComponent::Offline(_)) {
                    self.component.unrealize(context);
                    self.component =
                        Offline(OfflineListing::new((account_hash, MailboxHash::default())));
                    self.component.realize(self.id().into(), context);
                }
                self.component
                    .process_event(&mut UIEvent::VisibilityChange(true), context);
                self.status = None;
                self.push_status_watch(
                    self.status(context),
                    self.status_watch(),
                    &mut context.replies,
                );
            }
            MenuEntryCursor::Status if context.is_online(account_hash).is_ok() => {
                self.open_status(self.cursor_pos.account, context);
            }
            MenuEntryCursor::Status => {
                self.component.unrealize(context);
                self.component =
                    Offline(OfflineListing::new((account_hash, MailboxHash::default())));
                self.component.realize(self.id().into(), context);
                self.component
                    .process_event(&mut UIEvent::VisibilityChange(true), context);
                self.status = None;
                self.cursor_pos.menu = MenuEntryCursor::Mailbox(0);
                self.push_status_watch(
                    self.status(context),
                    self.status_watch(),
                    &mut context.replies,
                );
            }
        }
        self.sidebar_divider = *account_settings!(context[account_hash].listing.sidebar_divider);
        self.set_dirty(true);
        self.menu_cursor_pos = self.cursor_pos;
        // clear menu to force redraw
        self.menu.grid_mut().empty();
        if *account_settings!(context[account_hash].listing.show_menu_scrollbar) {
            self.show_menu_scrollbar = ShowMenuScrollbar::True;
            self.menu_scrollbar_show_timer.rearm();
        } else {
            self.show_menu_scrollbar = ShowMenuScrollbar::Never;
        }
    }

    fn open_status(&mut self, account_idx: usize, context: &mut Context) {
        self.status = Some(AccountStatus::new(account_idx, self.theme_default));
        self.menu.grid_mut().empty();
        self.push_status_watch(
            self.status(context),
            self.status_watch(),
            &mut context.replies,
        );
    }

    /// Move the keyboard focus onto the mailbox sidebar and arm the
    /// transient scrollbar hint (unless the sidebar is configured to
    /// never show one).
    fn focus_menu(&mut self) {
        self.focus = ListingFocus::Menu;
        // Keep `grid_has_keyboard` mirroring `focus == ListingFocus::MailList`
        // so every pane renders the keyboard-held pane highlighted and the
        // rest dimmed from a single source of truth.
        self.component.set_grid_has_keyboard(false);
        if self.show_menu_scrollbar != ShowMenuScrollbar::Never {
            self.menu_scrollbar_show_timer.rearm();
            self.show_menu_scrollbar = ShowMenuScrollbar::True;
        }
    }

    /// Close the open thread view and land the keyboard on the grid. The
    /// sidebar (if it was hidden by the open mail pane) comes back
    /// automatically via `is_menu_visible`.
    fn close_view(&mut self, context: &mut Context) {
        if let Some(view) = self.view.take() {
            view.unrealize(context);
        }
        self.last_opened_env = None;
        if !matches!(self.component.focus(), Focus::None) {
            self.component.set_focus(Focus::None, context);
        }
        self.component.set_grid_has_keyboard(true);
        self.focus = ListingFocus::MailList;
        self.set_dirty(true);
    }

    fn is_menu_visible(&self) -> bool {
        // The mailbox list is only visible in layout1: any open view
        // (layout2/3/4) replaces it — exactly two panes are on screen.
        self.menu_visibility && self.view.is_none()
    }

    fn set_index_style(&mut self, new_style: IndexStyle, context: &mut Context) {
        // The view belongs to the previous listing style; close it so no
        // stale view (or sidebar occlusion) survives the switch.
        self.close_view(context);
        let old = match new_style {
            IndexStyle::Plain => {
                if matches!(self.component, Plain(_)) {
                    return;
                }
                let coordinates = self.component.coordinates();
                std::mem::replace(
                    &mut self.component,
                    Plain(PlainListing::new(self.id, coordinates, context)),
                )
            }
            IndexStyle::Threaded => {
                if matches!(self.component, Threaded(_)) {
                    return;
                }
                let coordinates = self.component.coordinates();
                std::mem::replace(
                    &mut self.component,
                    Threaded(ThreadListing::new(self.id, coordinates, context)),
                )
            }
            IndexStyle::Compact => {
                if matches!(self.component, Compact(_)) {
                    return;
                }
                let coordinates = self.component.coordinates();
                std::mem::replace(
                    &mut self.component,
                    Compact(CompactListing::new(self.id, coordinates, context)),
                )
            }
            IndexStyle::Conversations => {
                if matches!(self.component, Conversations(_)) {
                    return;
                }
                let coordinates = self.component.coordinates();
                std::mem::replace(
                    &mut self.component,
                    Conversations(ConversationsListing::new(self.id, coordinates, context)),
                )
            }
        };
        if let MenuEntryCursor::Mailbox(idx) = self.cursor_pos.menu {
            if let Some(mbox_entry) = self.accounts[self.cursor_pos.account].entries.get_mut(idx) {
                mbox_entry.index_style = Some(new_style);
            }
        }
        self.component
            .process_event(&mut UIEvent::VisibilityChange(true), context);
        old.unrealize(context);
        self.component.realize(self.id.into(), context);
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ListingMessage {
    FocusUpdate {
        new_value: Focus,
    },
    OpenEntryUnderCursor {
        env_hash: EnvelopeHash,
        thread_hash: ThreadHash,
        go_to_first_unread: bool,
    },
    UpdateView,
}

struct TagsIterator<'envelope, 'context> {
    context: &'context Context,
    account_hash: AccountHash,
    mailbox_hash: MailboxHash,
    tags: &'context BTreeMap<melib::TagHash, String>,
    iter: indexmap::set::Iter<'envelope, melib::TagHash>,
}

impl<'envelope, 'context> TagsIterator<'envelope, 'context> {
    #[inline]
    fn new(
        iter: indexmap::set::Iter<'envelope, melib::TagHash>,
        context: &'context Context,
        account_hash: AccountHash,
        mailbox_hash: MailboxHash,
        tags: &'context BTreeMap<melib::TagHash, String>,
    ) -> Self {
        Self {
            context,
            account_hash,
            mailbox_hash,
            tags,
            iter,
        }
    }
}

impl<'envelope, 'context> Iterator for TagsIterator<'envelope, 'context> {
    type Item = (&'context str, Option<Color>);

    fn next(&mut self) -> Option<Self::Item> {
        let Self {
            ref account_hash,
            ref mailbox_hash,
            tags,
            context,
            ref mut iter,
        } = self;
        let mut t = iter.next()?;
        while mailbox_settings!(context[*account_hash][mailbox_hash].tags.ignore_tags).contains(t)
            || account_settings!(context[*account_hash].tags.ignore_tags).contains(t)
            || context.settings.tags.ignore_tags.contains(t)
            || !tags.contains_key(t)
        {
            t = iter.next()?;
        }
        let color = mailbox_settings!(context[*account_hash][mailbox_hash].tags.colors)
            .get(t)
            .cloned()
            .or_else(|| {
                account_settings!(context[*account_hash].tags.colors)
                    .get(t)
                    .cloned()
                    .or_else(|| context.settings.tags.colors.get(t).cloned())
            });
        let s = if let Some(s) =
            mailbox_settings!(context[*account_hash][mailbox_hash].tags.rename).get(t)
        {
            s.as_str()
        } else {
            tags.get(t)?.as_str()
        };
        Some((s, color))
    }
}

#[cfg(test)]
mod listing_menu_tests {
    use std::sync::OnceLock;

    use melib::{
        backends::{
            AccountHash, BackendEventConsumer, BackendMailbox, Backends, Mailbox, MailboxHash,
            MailboxPermissions, SpecialUsageMailbox,
        },
        Result,
    };

    use super::*;
    use crate::{
        accounts::{build_mailboxes_order, MailboxEntry, MailboxStatus},
        components::Component,
        conf::{composing::SendMail, FileMailboxConf},
        terminal::Key,
        types::UIEvent,
        Context,
    };

    #[derive(Debug)]
    struct TestMailbox {
        hash: MailboxHash,
        name: String,
    }

    impl BackendMailbox for TestMailbox {
        fn hash(&self) -> MailboxHash {
            self.hash
        }

        fn name(&self) -> &str {
            &self.name
        }

        // Each mailbox must have a distinct path: `build_mailboxes_order`
        // sorts by path, so a shared hardcoded path would make both entries
        // compare equal and leave the INBOX-first detection ambiguous.
        fn path(&self) -> &str {
            &self.name
        }

        fn children(&self) -> &[MailboxHash] {
            &[]
        }

        fn clone(&self) -> Mailbox {
            Box::new(Self {
                hash: self.hash,
                name: self.name.clone(),
            })
        }

        fn special_usage(&self) -> SpecialUsageMailbox {
            SpecialUsageMailbox::Normal
        }

        fn parent(&self) -> Option<MailboxHash> {
            None
        }

        fn permissions(&self) -> MailboxPermissions {
            MailboxPermissions::default()
        }

        fn is_subscribed(&self) -> bool {
            true
        }

        fn set_is_subscribed(&mut self, _: bool) -> Result<()> {
            Ok(())
        }

        fn set_special_usage(&mut self, _: SpecialUsageMailbox) -> Result<()> {
            Ok(())
        }

        fn count(&self) -> Result<(usize, usize)> {
            Ok((0, 0))
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    /// Shared HOME for the mock contexts below. Environment variables are
    /// process-global, so parallel tests must not race each other by pointing
    /// them at tempdirs that get deleted while another test constructs its
    /// `Context` (which reads `MELI_CONFIG`/XDG vars).
    fn shared_test_home() -> &'static tempfile::TempDir {
        static HOME: OnceLock<tempfile::TempDir> = OnceLock::new();
        HOME.get_or_init(|| {
            let tempdir = tempfile::tempdir().unwrap();
            std::env::set_var("HOME", tempdir.path());
            std::env::set_var("XDG_CONFIG_HOME", tempdir.path().join(".config"));
            std::env::set_var(
                "XDG_DATA_HOME",
                tempdir.path().join(".local").join(".share"),
            );
            tempdir
        })
    }

    fn mock_context() -> Context {
        // Retry: parallel suites (conf tests) also overwrite the process-global
        // `MELI_CONFIG`, which can make `Settings::new()` inside `new_mock` fail
        // spuriously.
        let mut ctx = None;
        for _ in 0..3 {
            if let Ok(candidate) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                Context::new_mock(shared_test_home())
            })) {
                ctx = Some(candidate);
                break;
            }
        }
        let mut ctx = ctx.unwrap_or_else(|| Context::new_mock(shared_test_home()));
        // The default `send_mail` (`ShellCommand("false")`) races: the child can
        // exit before meli finishes writing the message to its stdin, panicking
        // with a broken pipe. An empty command makes `Account::send` return a
        // deterministic error without spawning anything.
        let account_hash = *ctx.accounts.iter().next().unwrap().0;
        ctx.accounts[&account_hash].settings.send_mail = SendMail::ShellCommand(String::new());
        ctx
    }

    /// Register two mailboxes (`INBOX`, `Archive`) on the mock account and
    /// rebuild the account's mailbox tree/order.
    ///
    /// `Listing::new` snapshots the account entries via `list_mailboxes`, so
    /// the mailboxes must be registered before it is called. The
    /// INBOX-first comparator in `build_mailboxes_order` guarantees the order
    /// `INBOX` (index 0), `Archive` (index 1).
    fn register_two_mailboxes(context: &mut Context) -> (AccountHash, MailboxHash, MailboxHash) {
        let account_hash = *context.accounts.iter().next().unwrap().0;
        let account = context.accounts.get_mut(&account_hash).unwrap();
        for name in ["INBOX", "Archive"] {
            let mailbox_hash = MailboxHash::from_bytes(name.as_bytes());
            account.mailbox_entries.insert(
                mailbox_hash,
                MailboxEntry::new(
                    MailboxStatus::Available,
                    name.to_string(),
                    Box::new(TestMailbox {
                        hash: mailbox_hash,
                        name: name.to_string(),
                    }),
                    FileMailboxConf::default(),
                ),
            );
        }
        build_mailboxes_order(
            &mut account.tree,
            &account.mailbox_entries,
            &mut account.mailboxes_order,
        );
        (
            account_hash,
            MailboxHash::from_bytes(b"INBOX"),
            MailboxHash::from_bytes(b"Archive"),
        )
    }

    /// Register a second mock account so the cross-account navigation
    /// paths (the sidebar `scroll_up`/`scroll_down` account wrap) are
    /// exercisable. Built like the mock account in `Context::new_mock`.
    fn register_second_account(context: &mut Context) -> AccountHash {
        let name = "second".to_string();
        let mut account_conf = crate::conf::AccountConf::default();
        account_conf.conf.format = "maildir".to_string();
        account_conf.account.format = "maildir".to_string();
        account_conf.account.root_mailbox = shared_test_home().path().display().to_string();
        let account_hash = AccountHash::from_bytes(name.as_bytes());
        let account = crate::accounts::Account::new(
            account_hash,
            name,
            account_conf,
            &Backends::new(),
            context.main_loop_handler.clone(),
            BackendEventConsumer::new(std::sync::Arc::new(|_, _| {})),
        )
        .unwrap();
        context.accounts.insert(account_hash, account);
        account_hash
    }

    #[test]
    fn listing_menu_focus_right_opens_selected_mailbox() {
        let mut ctx = mock_context();
        // Pin the shortcuts this test drives so that a `MELI_CONFIG` template
        // drift cannot change what the keys mean.
        ctx.settings.shortcuts.listing.focus_left = Key::Left.into();
        ctx.settings.shortcuts.listing.focus_right = Key::Right.into();
        ctx.settings.shortcuts.listing.scroll_up = Key::Up.into();
        ctx.settings.shortcuts.listing.scroll_down = Key::Down.into();
        let (account_hash, _inbox_hash, archive_hash) = register_two_mailboxes(&mut ctx);
        let mut listing = Listing::new(&mut ctx);
        // `Listing::new` lands on the visible sidebar now; this test drives
        // the layout1 grid → sidebar transition, so restore the grid start.
        listing.focus = ListingFocus::MailList;
        assert_eq!(
            listing.cursor_pos.menu,
            MenuEntryCursor::Mailbox(0),
            "precondition: INBOX is the open mailbox"
        );

        listing.process_event(&mut UIEvent::Input(Key::Left), &mut ctx);
        assert_eq!(listing.focus, ListingFocus::Menu);

        listing.process_event(&mut UIEvent::Input(Key::Down), &mut ctx);
        assert_eq!(
            listing.menu_cursor_pos.menu,
            MenuEntryCursor::Mailbox(1),
            "Down must move the menu cursor to Archive without opening it"
        );

        let consumed = listing.process_event(&mut UIEvent::Input(Key::Right), &mut ctx);
        assert!(consumed);
        assert_eq!(listing.focus, ListingFocus::MailList);
        assert_eq!(
            listing.cursor_pos.menu,
            MenuEntryCursor::Mailbox(1),
            "focus_right must adopt the sidebar-selected mailbox (Archive)"
        );
        assert_eq!(
            listing.component.coordinates(),
            (account_hash, archive_hash),
            "focus_right must open the sidebar-selected mailbox"
        );
    }

    #[test]
    fn listing_menu_focus_right_at_status_opens_status() {
        let mut ctx = mock_context();
        ctx.settings.shortcuts.listing.focus_left = Key::Left.into();
        ctx.settings.shortcuts.listing.focus_right = Key::Right.into();
        ctx.settings.shortcuts.listing.scroll_up = Key::Up.into();
        ctx.settings.shortcuts.listing.scroll_down = Key::Down.into();
        register_two_mailboxes(&mut ctx);
        let mut listing = Listing::new(&mut ctx);
        // `Listing::new` lands on the visible sidebar now; start on the
        // grid so the Left transition below is still exercised.
        listing.focus = ListingFocus::MailList;

        listing.process_event(&mut UIEvent::Input(Key::Left), &mut ctx);
        assert_eq!(listing.focus, ListingFocus::Menu);

        listing.menu_cursor_pos.menu = MenuEntryCursor::Status;
        let consumed = listing.process_event(&mut UIEvent::Input(Key::Right), &mut ctx);
        assert!(consumed);
        assert_eq!(listing.focus, ListingFocus::MailList);
        assert!(
            listing.status.is_some(),
            "focus_right at the Status entry must open the account status view"
        );
    }

    /// Navigation key group on the sidebar (mailbox list): with the
    /// default bindings both the vim keys (`j`/`k`) and the arrow keys
    /// drive the menu cursor through the same `listing.scroll_up` /
    /// `scroll_down` arms.
    #[test]
    fn listing_menu_vim_and_arrow_keys_move_cursor() {
        let mut ctx = mock_context();
        ctx.settings.shortcuts.listing.focus_left = ShortcutKeys::double(Key::Left, Key::Char('h'));
        register_two_mailboxes(&mut ctx);
        let mut listing = Listing::new(&mut ctx);
        // `Listing::new` lands on the visible sidebar now; start on the
        // grid so the Left transition into the sidebar is still exercised.
        listing.focus = ListingFocus::MailList;
        listing.process_event(&mut UIEvent::Input(Key::Left), &mut ctx);
        assert_eq!(listing.focus, ListingFocus::Menu);

        let start = listing.menu_cursor_pos.menu;
        // Vim down then back up.
        assert!(
            listing.process_event(&mut UIEvent::Input(Key::Char('j')), &mut ctx),
            "'j' must be consumed at the sidebar"
        );
        assert_ne!(
            listing.menu_cursor_pos.menu, start,
            "'j' must move the menu cursor down"
        );
        assert!(
            listing.process_event(&mut UIEvent::Input(Key::Char('k')), &mut ctx),
            "'k' must be consumed at the sidebar"
        );
        assert_eq!(
            listing.menu_cursor_pos.menu, start,
            "'k' must move the menu cursor back up"
        );
        // Arrow keys drive the same arms.
        assert!(
            listing.process_event(&mut UIEvent::Input(Key::Down), &mut ctx),
            "Down must be consumed at the sidebar"
        );
        assert_ne!(
            listing.menu_cursor_pos.menu, start,
            "Down must move the menu cursor down"
        );
        assert!(
            listing.process_event(&mut UIEvent::Input(Key::Up), &mut ctx),
            "Up must be consumed at the sidebar"
        );
        assert_eq!(
            listing.menu_cursor_pos.menu, start,
            "Up must move the menu cursor back up"
        );
    }

    /// Navigation key group on the mail list: the vertical keys — vim keys
    /// and arrow keys alike — are consumed by the listing's scroll arms;
    /// the horizontal pair belongs to the pane chain now: h/Left hand the
    /// focus to the sidebar, l/Right hand it back (opening the
    /// sidebar-selected mailbox).
    #[test]
    fn listing_mail_list_navigation_keygroup_consumed() {
        let mut ctx = mock_context();
        register_two_mailboxes(&mut ctx);
        let mut listing = Listing::new(&mut ctx);
        // `Listing::new` lands on the visible sidebar now; the key group
        // under test is the grid's, so start from the mail list.
        listing.focus = ListingFocus::MailList;
        for key in [Key::Char('j'), Key::Down, Key::Char('k'), Key::Up] {
            assert!(
                listing.process_event(&mut UIEvent::Input(key.clone()), &mut ctx),
                "{key:?} must be consumed by the mail list scroll arms"
            );
        }
        for key in [Key::Char('h'), Key::Left] {
            assert!(
                listing.process_event(&mut UIEvent::Input(key.clone()), &mut ctx),
                "{key:?} must be consumed by the pane chain"
            );
            assert_eq!(listing.focus, ListingFocus::Menu);
            // l/Right hands the focus back to the mail list (and opens the
            // sidebar-selected mailbox), so the next iteration starts from
            // the grid again.
            assert!(
                listing.process_event(&mut UIEvent::Input(Key::Char('l')), &mut ctx),
                "focus_right at the sidebar must be consumed"
            );
            assert_eq!(listing.focus, ListingFocus::MailList);
        }
    }

    /// Layered quit: with the `AccountStatus` sub-view open, the quit
    /// binding (`Esc`/`q` by default) must close the status view and be
    /// consumed, instead of bubbling to the application-level exit path.
    #[test]
    fn quit_key_closes_account_status() {
        for key in [Key::Esc, Key::Char('q')] {
            let mut ctx = mock_context();
            ctx.settings.shortcuts.listing.focus_left = Key::Left.into();
            ctx.settings.shortcuts.listing.focus_right = Key::Right.into();
            ctx.settings.shortcuts.listing.scroll_up = Key::Up.into();
            register_two_mailboxes(&mut ctx);
            let mut listing = Listing::new(&mut ctx);
            // `Listing::new` lands on the visible sidebar now; start from
            // the grid so Left still drives the layout1 pane switch.
            listing.focus = ListingFocus::MailList;

            listing.process_event(&mut UIEvent::Input(Key::Left), &mut ctx);
            listing.menu_cursor_pos.menu = MenuEntryCursor::Status;
            listing.process_event(&mut UIEvent::Input(Key::Right), &mut ctx);
            assert!(listing.status.is_some(), "precondition: status view open");

            let consumed = listing.process_event(&mut UIEvent::Input(key.clone()), &mut ctx);
            assert!(consumed, "{key:?} must be consumed by the listing");
            assert!(
                listing.status.is_none(),
                "{key:?} must close the account status view, not the application"
            );
        }
    }

    /// Layered quit: with a mail view open on the focused entry, the quit
    /// binding must exit the view back to the list (the same
    /// `exit_entry` path as `i`), not the application.
    #[test]
    fn quit_key_exits_open_mail_view() {
        for key in [Key::Esc, Key::Char('q')] {
            let mut ctx = mock_context();
            let (_a, inbox_hash, _arch) = register_two_mailboxes(&mut ctx);
            let bytes = b"From: Carol Example <carol@example.org>\r\nTo: Bob Example <bob@example.org>\r\nSubject: quit-exit mail\r\nMessage-ID: <quit-exit@x.example>\r\nDate: Thu, 2 Jan 2025 09:30:00 +0000\r\n\r\nquit exit body\r\n";
            let mut env = Envelope::from_bytes(bytes, None).unwrap();
            env.set_flags(melib::Flag::SEEN);
            let account_hash = *ctx.accounts.iter().next().unwrap().0;
            ctx.accounts[&account_hash]
                .collection
                .insert(env, inbox_hash);

            ctx.settings.shortcuts.listing.open_entry = Key::Char('\n').into();
            let mut listing = Listing::new(&mut ctx);
            // `Listing::new` lands on the visible sidebar now; the test
            // opens an entry with the grid's `open_entry`, so start there.
            listing.focus = ListingFocus::MailList;
            let theme_default = crate::conf::value(&ctx, "theme_default");
            let mut screen = Screen::<Virtual>::new(theme_default);
            assert!(screen.resize(80, 24));
            let area = screen.area();
            listing.draw(screen.grid_mut(), area, &mut ctx);
            let mut event = UIEvent::Input(Key::Char('\n'));
            assert!(listing.process_event(&mut event, &mut ctx));
            for _ in 0..8 {
                let replies = ctx.replies();
                if replies.is_empty() {
                    break;
                }
                for mut ev in replies {
                    let _ = listing.process_event(&mut ev, &mut ctx);
                }
            }
            assert!(
                listing.view.is_some(),
                "precondition: open_entry must create the view"
            );
            assert!(
                listing.component.unfocused(),
                "precondition: entry focus while the view is open"
            );

            let consumed = listing.process_event(&mut UIEvent::Input(key.clone()), &mut ctx);
            assert!(consumed, "{key:?} must be consumed by the listing");
            assert!(
                !listing.component.unfocused(),
                "{key:?} must exit the open mail view back to the list"
            );
        }
    }

    /// Regression: a fresh listing must start with the keyboard on the
    /// mailbox sidebar (layout1) when the sidebar is visible on launch, so
    /// the first keystrokes navigate mailboxes instead of the mail list.
    #[test]
    fn startup_focus_lands_on_visible_sidebar() {
        let mut ctx = mock_context();
        register_two_mailboxes(&mut ctx);
        let listing = Listing::new(&mut ctx);
        assert!(
            listing.is_menu_visible(),
            "the sidebar must be visible at construction by default"
        );
        assert_eq!(
            listing.focus,
            ListingFocus::Menu,
            "a visible sidebar owns the initial keyboard focus"
        );
    }

    /// Regression: with `listing.hide_sidebar_on_launch = true` the sidebar
    /// is not visible at construction, so the initial focus must stay on
    /// the mail list grid — there is no sidebar to focus.
    #[test]
    fn startup_focus_stays_on_grid_when_sidebar_hidden() {
        let mut ctx = mock_context();
        register_two_mailboxes(&mut ctx);
        ctx.settings.listing.hide_sidebar_on_launch = true;
        let listing = Listing::new(&mut ctx);
        assert!(
            !listing.is_menu_visible(),
            "`hide_sidebar_on_launch` must keep the sidebar hidden"
        );
        assert_eq!(
            listing.focus,
            ListingFocus::MailList,
            "a sidebar hidden on launch leaves the initial focus on the grid"
        );
    }

    /// Regression: shortly after construction the account watcher reports the
    /// current account coming online/refreshing. That background reconcile
    /// runs `change_account`, which force-closes the (not yet open) view and
    /// lands the keyboard on the grid; it must not steal the sidebar focus
    /// that `Listing::new` just granted.
    #[test]
    fn account_status_change_keeps_startup_menu_focus() {
        let mut ctx = mock_context();
        let (account_hash, ..) = register_two_mailboxes(&mut ctx);
        let mut listing = Listing::new(&mut ctx);
        assert_eq!(
            listing.focus,
            ListingFocus::Menu,
            "precondition: construction focuses the visible sidebar"
        );

        listing.process_event(
            &mut UIEvent::AccountStatusChange(account_hash, None),
            &mut ctx,
        );

        assert!(
            listing.is_menu_visible(),
            "the startup account reconcile must keep the sidebar visible"
        );
        assert_eq!(
            listing.focus,
            ListingFocus::Menu,
            "the startup account reconcile must not steal the keyboard from the sidebar"
        );
    }

    /// Regression: when the sidebar is hidden on launch there is no sidebar
    /// focus to preserve, so the startup account reconcile must leave the
    /// keyboard on the grid.
    #[test]
    fn account_status_change_keeps_grid_focus_when_sidebar_hidden() {
        let mut ctx = mock_context();
        let (account_hash, ..) = register_two_mailboxes(&mut ctx);
        ctx.settings.listing.hide_sidebar_on_launch = true;
        let mut listing = Listing::new(&mut ctx);
        assert!(
            !listing.is_menu_visible(),
            "precondition: `hide_sidebar_on_launch` keeps the sidebar hidden"
        );
        assert_eq!(
            listing.focus,
            ListingFocus::MailList,
            "precondition: a hidden sidebar leaves the focus on the grid"
        );

        listing.process_event(
            &mut UIEvent::AccountStatusChange(account_hash, None),
            &mut ctx,
        );

        assert!(
            !listing.is_menu_visible(),
            "`hide_sidebar_on_launch` must keep the sidebar hidden across the reconcile"
        );
        assert_eq!(
            listing.focus,
            ListingFocus::MailList,
            "with no visible sidebar the reconcile leaves the keyboard on the grid"
        );
    }

    /// Regression: a `View` focus cannot survive the reconcile because
    /// `change_account` force-closes the open view; the grid is the only
    /// consistent landing for the keyboard.
    #[test]
    fn account_status_change_lands_on_grid_when_view_force_closed() {
        let mut ctx = mock_context();
        let (account_hash, ..) = register_two_mailboxes(&mut ctx);
        let mut listing = Listing::new(&mut ctx);
        listing.focus = ListingFocus::View;

        listing.process_event(
            &mut UIEvent::AccountStatusChange(account_hash, None),
            &mut ctx,
        );

        assert_eq!(
            listing.focus,
            ListingFocus::MailList,
            "the force-closed view must land the keyboard on the grid"
        );
    }

    /// Regression: when the reconcile restores a sidebar focus it must
    /// also take the grid's keyboard highlight back (`focus_menu`).
    /// `change_account` runs `close_view`, which sets the grid's
    /// `grid_has_keyboard`; leaving it set while `focus` returns to
    /// `Menu` paints both panes with "pane.focused" while the outer ring
    /// says the grid is unfocused.
    #[test]
    fn account_status_change_menu_focus_keeps_grid_dimmed() {
        let mut ctx = mock_context();
        let (account_hash, inbox_hash, _archive) = register_two_mailboxes(&mut ctx);
        for mid in ["solo-a", "solo-b"] {
            let bytes = format!(
                "From: a@b.example\r\nTo: c@d.example\r\nSubject: {mid}\r\n\
                 Message-ID: <{mid}@x.example>\r\n\
                 Date: Thu, 1 Jan 2026 00:00:00 +0000\r\n\r\n{mid}\r\n"
            );
            let env = Envelope::from_bytes(bytes.as_bytes(), None).unwrap();
            ctx.accounts[&account_hash]
                .collection
                .insert(env, inbox_hash);
        }
        let mut listing = Listing::new(&mut ctx);
        assert!(
            matches!(listing.focus, ListingFocus::Menu),
            "precondition: construction focuses the visible sidebar"
        );
        let theme_default = crate::conf::value(&ctx, "theme_default");
        let mut screen = Screen::<Virtual>::new(theme_default);
        assert!(screen.resize(80, 24));
        // Baseline: with the invariant intact at construction, the grid
        // paints dim while the sidebar is focused.
        assert_grid_painted_dimmed(&mut screen, &mut listing, &mut ctx);

        listing.process_event(
            &mut UIEvent::AccountStatusChange(account_hash, None),
            &mut ctx,
        );

        assert!(
            matches!(listing.focus, ListingFocus::Menu),
            "the reconcile must not steal the keyboard from the sidebar"
        );
        assert_grid_painted_dimmed(&mut screen, &mut listing, &mut ctx);
    }

    /// Pin the pane-chain keys so a `MELI_CONFIG` template drift cannot
    /// change what the keys mean in the chain tests below.
    fn pin_pane_chain_keys(ctx: &mut Context) {
        ctx.settings.shortcuts.listing.focus_left = Key::Left.into();
        ctx.settings.shortcuts.listing.focus_right = Key::Right.into();
        ctx.settings.shortcuts.listing.exit_entry = Key::Esc.into();
    }

    fn make_drawn_listing(ctx: &mut Context) -> Listing {
        let mut listing = Listing::new(ctx);
        // `Listing::new` lands on a visible sidebar now; the pane-chain and
        // mailbox-scroll tests below exercise the grid-focused layout1, so
        // hand the focus to the mail list explicitly (the launch-time state
        // when the sidebar is hidden).
        listing.focus = ListingFocus::MailList;
        let theme_default = crate::conf::value(ctx, "theme_default");
        let mut screen = Screen::<Virtual>::new(theme_default);
        assert!(screen.resize(80, 24));
        let area = screen.area();
        listing.draw(screen.grid_mut(), area, ctx);
        listing
    }

    /// Draw `listing` on the test screen and assert the invariant commit
    /// 444b17a5 established: `grid_has_keyboard` mirrors
    /// `focus == ListingFocus::MailList`, so while the sidebar holds the
    /// keyboard (`focus == Menu`) the grid pane must paint the dim
    /// "pane.unfocused" fill, never "pane.focused". The screen persists
    /// across calls (like the real terminal) so incremental repaints
    /// compose; the flags are forced so the assertion observes the
    /// current focus state, not a stale frame.
    ///
    /// Layout1 geometry (`Listing::draw`): the sidebar keeps 30% of the
    /// 80-column screen, a 1-column divider follows, and the grid pane
    /// fills the rest inside its rounded frame — inner cells are
    /// x in 26..79, y in 1..23.
    fn assert_grid_painted_dimmed(
        screen: &mut Screen<Virtual>,
        listing: &mut Listing,
        ctx: &mut Context,
    ) {
        assert!(
            matches!(listing.focus, ListingFocus::Menu),
            "precondition: this assertion applies to a sidebar-focused listing"
        );
        listing.set_dirty(true);
        listing.component.set_dirty(true);
        let area = screen.area();
        listing.draw(screen.grid_mut(), area, ctx);

        let pane_focused_bg = crate::conf::value(ctx, "pane.focused").bg;
        let pane_unfocused_bg = crate::conf::value(ctx, "pane.unfocused").bg;
        let theme_default_bg = crate::conf::value(ctx, "theme_default").bg;
        let grid = screen.grid();
        // Entry rows and the empty-mailbox hint keep `theme_default` bg,
        // which the default theme shares with "pane.focused"; flag the
        // focused fill only where the theme distinguishes the two.
        if pane_focused_bg != theme_default_bg {
            for y in 1..23 {
                for x in 26..79 {
                    assert_ne!(
                        grid[(x, y)].bg(),
                        pane_focused_bg,
                        "grid cell ({x}, {y}) paints the focused pane fill \
                         while the sidebar holds the keyboard"
                    );
                }
            }
        }
        // The strip below the entry rows is pure pane fill: it must carry
        // the dim fill exactly (a leaked grid_has_keyboard repaints it
        // with "pane.focused" and fails this check).
        for y in 18..23 {
            for x in 26..79 {
                assert_eq!(
                    grid[(x, y)].bg(),
                    pane_unfocused_bg,
                    "grid cell ({x}, {y}) in the blank strip must paint the \
                     dim pane fill while the sidebar holds the keyboard"
                );
            }
        }
    }

    /// Build a listing over a two-mail thread (root + reply via
    /// `In-Reply-To`), after a first draw so the grid rows exist.
    fn pane_chain_setup(ctx: &mut Context) -> Listing {
        let (_a, inbox_hash, _arch) = register_two_mailboxes(ctx);
        let root_bytes = b"From: a@b.example\r\nTo: c@d.example\r\nSubject: chain\r\nMessage-ID: <chain-root@x.example>\r\nDate: Thu, 1 Jan 2026 00:00:00 +0000\r\n\r\nroot\r\n";
        let reply_bytes = b"From: c@d.example\r\nTo: a@b.example\r\nSubject: Re: chain\r\nMessage-ID: <chain-reply@x.example>\r\nIn-Reply-To: <chain-root@x.example>\r\nDate: Thu, 1 Jan 2026 00:01:00 +0000\r\n\r\nreply\r\n";
        let solo_bytes = b"From: s@x.example\r\nTo: y@x.example\r\nSubject: chain solo\r\nMessage-ID: <chain-solo@x.example>\r\nDate: Thu, 1 Jan 2026 00:02:00 +0000\r\n\r\nsolo\r\n";
        let account_hash = *ctx.accounts.iter().next().unwrap().0;
        for bytes in [
            root_bytes.as_slice(),
            reply_bytes.as_slice(),
            solo_bytes.as_slice(),
        ] {
            let env = Envelope::from_bytes(bytes, None).unwrap();
            ctx.accounts[&account_hash]
                .collection
                .insert(env, inbox_hash);
        }
        pin_pane_chain_keys(ctx);
        make_drawn_listing(ctx)
    }

    /// Send one key and pump the queued replies (the view is created by the
    /// `OpenEntryUnderCursor` reply).
    fn pane_step(listing: &mut Listing, ctx: &mut Context, key: Key) -> bool {
        let consumed = listing.process_event(&mut UIEvent::Input(key), ctx);
        // Apply pending grid movements (the scroll arms defer them to
        // draw) and let the cursor-following view refresh queue up.
        {
            let theme_default = crate::conf::value(ctx, "theme_default");
            let mut screen = Screen::<Virtual>::new(theme_default);
            let _ = screen.resize(80, 24);
            let area = screen.area();
            listing.draw(screen.grid_mut(), area, ctx);
        }
        for _ in 0..8 {
            let replies = ctx.replies();
            if replies.is_empty() {
                break;
            }
            for mut ev in replies {
                let _ = listing.process_event(&mut ev, ctx);
            }
        }
        consumed
    }

    /// Layout1 → layout2: a single-mail entry opens the mail view directly,
    /// the focus stays on the grid, the mailbox list is hidden.
    #[test]
    fn layout1_open_single_mail_is_layout2() {
        let mut ctx = mock_context();
        let mut listing = pane_chain_setup(&mut ctx);
        // The setup's cursor rests on the solo mail (the newest entry).
        assert!(pane_step(&mut listing, &mut ctx, Key::Right));
        let view = listing.view.as_ref().expect("view opened");
        assert!(view.is_single_mail());
        assert!(
            matches!(view.thread_view_focus(), ThreadViewFocus::MailView),
            "a single-mail view opens at the mail view (layout2)"
        );
        assert!(
            matches!(listing.focus, ListingFocus::MailList),
            "layout2 opens with the focus on the grid"
        );
        assert!(!listing.is_menu_visible(), "the mailbox list is hidden");
    }

    /// Layout1 → layout3: a thread entry opens the thread list, the focus
    /// stays on the grid.
    #[test]
    fn layout1_open_thread_is_layout3() {
        let mut ctx = mock_context();
        let mut listing = pane_chain_setup(&mut ctx);
        // Move onto the two-mail thread (the second row: the solo mail is
        // the newest entry and rests on top).
        assert!(pane_step(&mut listing, &mut ctx, Key::Char('j')));
        assert!(pane_step(&mut listing, &mut ctx, Key::Right));
        let view = listing.view.as_ref().expect("view opened");
        assert!(!view.is_single_mail());
        assert!(
            matches!(view.thread_view_focus(), ThreadViewFocus::Thread),
            "a thread view opens at the whole thread list (layout3)"
        );
        assert!(matches!(listing.focus, ListingFocus::MailList));
        assert!(!listing.is_menu_visible());
    }

    /// layout2 + search: running `:search ollama` (the `/` shortcut) must
    /// filter the grid to matching envelopes, with the focus on the first
    /// result (the chain stays at layout2 — the view follows the new
    /// cursor entry on subsequent j/k).
    #[test]
    fn layout2_search_filters_grid() {
        let mut ctx = mock_context();
        let mut listing = pane_chain_setup(&mut ctx);
        assert!(pane_step(&mut listing, &mut ctx, Key::Right));
        // layout2: grid open, view single mail.
        let sel = listing.component.cursor_selection();
        let (_thread, _env) = sel.expect("cursor must point at a thread");
        // Fire the search command directly (the parser does this after
        // `:` or `/`).
        let mut event = UIEvent::Action(Action::Listing(ListingAction::Search {
            term: "chain".to_string(),
            raw_search: false,
        }));
        assert!(listing.process_event(&mut event, &mut ctx));
        for _ in 0..8 {
            for mut ev in ctx.replies() {
                let _ = listing.process_event(&mut ev, &mut ctx);
            }
        }
        // Simulate the JobFinished reply: feed results straight into the
        // component filter (the async job path is exercised in the
        // integration search tests; here we just need the grid to update).
        let all_envs: Vec<_> = listing
            .component
            .cursor_selection()
            .into_iter()
            .map(|(_, e)| e)
            .collect();
        listing
            .component
            .filter("chain".to_string(), all_envs, &ctx);
        listing.set_dirty(true);
        // Draw and assert the first row points at the cursor (filtered
        // result, not the stale initial thread).
        let theme_default = crate::conf::value(&ctx, "theme_default");
        let mut screen = Screen::<Virtual>::new(theme_default);
        assert!(screen.resize(80, 24));
        let area = screen.area();
        listing.draw(screen.grid_mut(), area, &mut ctx);
        let grid = screen.grid();
        let row1: String = (0..30).map(|x| grid[(x, 1)].ch()).collect();
        // The grid subpane is on the left (layout2).
        assert!(
            row1.starts_with("│0"),
            "layout2 grid must render the filtered result, row1 was {row1:?}"
        );
        // The grid must still hold the focus (search does not steal it).
        assert!(matches!(listing.focus, ListingFocus::MailList));
    }

    /// Structured queries (`from:…`, `subject:…`) must go through melib's
    /// `is_match`; only the unimplemented `Body`/`AllText` variants fall
    /// back to `Account::search`'s header substring scan.
    #[test]
    fn search_structured_query_uses_melib_match() {
        let mut ctx = mock_context();
        let listing = pane_chain_setup(&mut ctx);
        let (_account_hash, mailbox_hash) = listing.component.coordinates();
        let account = ctx.accounts.values().next().unwrap();
        let sort = (melib::SortField::Date, melib::SortOrder::Desc);
        let results = futures::executor::block_on(
            account
                .search("from:s@x.example", false, sort, mailbox_hash)
                .expect("structured query must parse"),
        )
        .expect("structured query must scan")
        .envelopes;
        // Only the solo mail is from s@x.example.
        assert_eq!(results.len(), 1, "from: must match exactly solo");
        let subject = account
            .collection
            .envelopes
            .read()
            .unwrap()
            .get(&results[0])
            .unwrap()
            .subject()
            .to_string();
        assert_eq!(subject, "chain solo");
        // A subject: query is structured too, and all three mails carry
        // "chain" in the subject.
        let results = futures::executor::block_on(
            account
                .search("subject:chain", false, sort, mailbox_hash)
                .expect("subject query must parse"),
        )
        .expect("subject query must scan")
        .envelopes;
        assert_eq!(results.len(), 3, "subject:chain must match all three");
        // A bare term stays on the substring fallback (melib's Body
        // is_match is unimplemented) and still matches.
        let results = futures::executor::block_on(
            account
                .search("solo", false, sort, mailbox_hash)
                .expect("bare term must parse"),
        )
        .expect("bare term must scan")
        .envelopes;
        assert_eq!(results.len(), 1, "bare term must match the solo subject");
    }

    /// Layout2 grid ⇄ mail view with Left/Right; Left from the grid closes
    /// the view back to layout1.
    #[test]
    fn layout2_focus_roundtrip_and_back_to_layout1() {
        let mut ctx = mock_context();
        let mut listing = pane_chain_setup(&mut ctx);
        assert!(pane_step(&mut listing, &mut ctx, Key::Right));
        {
            let theme_default = crate::conf::value(&ctx, "theme_default");
            let mut screen = Screen::<Virtual>::new(theme_default);
            assert!(screen.resize(80, 24));
            let area = screen.area();
            listing.draw(screen.grid_mut(), area, &mut ctx);
        }

        // Right: focus the mail view (layout2 right pane).
        assert!(pane_step(&mut listing, &mut ctx, Key::Right));

        assert!(matches!(
            listing.view.as_ref().unwrap().thread_view_focus(),
            ThreadViewFocus::MailView
        ));

        // Left: back to the grid (layout2 left pane), the view stays.
        assert!(pane_step(&mut listing, &mut ctx, Key::Left));
        assert!(matches!(listing.focus, ListingFocus::MailList));
        assert!(
            listing.view.as_ref().is_some_and(|v| v.is_single_mail()),
            "the view stays open (layout2)"
        );

        // Left from the grid: close the view → layout1, focus on the
        // mailbox list (same landing as layout3's grid).
        assert!(pane_step(&mut listing, &mut ctx, Key::Left));
        assert!(listing.view.is_none());
        assert!(matches!(listing.focus, ListingFocus::Menu));
        assert!(listing.is_menu_visible());
    }

    /// Regression: Left from layout2's grid (single-mail view open)
    /// closes the view and lands the focus on the mailbox list, the
    /// same landing as layout3's grid and layout1's grid.
    #[test]
    fn layout2_grid_left_closes_to_mailbox_list() {
        let mut ctx = mock_context();
        let mut listing = pane_chain_setup(&mut ctx);
        assert!(pane_step(&mut listing, &mut ctx, Key::Right));
        assert!(
            listing.view.as_ref().unwrap().is_single_mail(),
            "precondition: layout2 (single-mail view open)"
        );
        assert!(matches!(listing.focus, ListingFocus::MailList));

        assert!(pane_step(&mut listing, &mut ctx, Key::Left));
        assert!(listing.view.is_none(), "the view must close");
        assert!(
            matches!(listing.focus, ListingFocus::Menu),
            "the focus must land on the mailbox list"
        );
        assert!(listing.is_menu_visible());
    }

    /// With the sidebar hidden (`menu_visibility == false`), Left from
    /// layout2's grid still closes the view but cannot land on the
    /// hidden mailbox list: the focus stays on the grid.
    #[test]
    fn layout2_grid_left_with_hidden_sidebar_stays_on_grid() {
        let mut ctx = mock_context();
        let mut listing = pane_chain_setup(&mut ctx);
        listing.menu_visibility = false;
        assert!(pane_step(&mut listing, &mut ctx, Key::Right));
        assert!(
            listing.view.as_ref().unwrap().is_single_mail(),
            "precondition: layout2 (single-mail view open)"
        );

        assert!(pane_step(&mut listing, &mut ctx, Key::Left));
        assert!(listing.view.is_none(), "the view must close");
        assert!(
            matches!(listing.focus, ListingFocus::MailList),
            "with the sidebar hidden the focus must stay on the grid"
        );
        assert!(!listing.menu_visibility);
    }

    /// Layout4 renders exactly two panes: the thread list (30%) | mail
    /// view (70%) inside the whole view area — the grid is hidden (no
    /// grid subpane ring at its 30% boundary).
    #[test]
    fn layout4_renders_two_panes_without_grid() {
        let mut ctx = mock_context();
        let mut listing = pane_chain_setup(&mut ctx);
        // Onto the thread, open (layout3), then Right → layout4.
        assert!(pane_step(&mut listing, &mut ctx, Key::Char('j')));
        assert!(pane_step(&mut listing, &mut ctx, Key::Right));
        assert!(pane_step(&mut listing, &mut ctx, Key::Right));
        assert!(matches!(listing.focus, ListingFocus::View));

        let theme_default = crate::conf::value(&ctx, "theme_default");
        let mut screen = Screen::<Virtual>::new(theme_default);
        assert!(screen.resize(80, 24));
        let area = screen.area();
        listing.set_dirty(true);
        listing.draw(screen.grid_mut(), area, &mut ctx);
        let grid = screen.grid();
        // The view's thread-list ring opens at column 0...
        assert_eq!(grid[(0, 0)].ch(), '╭', "thread-list ring top-left");
        // ...and spans the same 30% width as the listing grid in
        // layout1-3: its right edge sits at column 23 (the grid's would
        // too), then one gap column, then the mail pane.
        assert_eq!(
            grid[(23, 0)].ch(),
            '╮',
            "thread-list ring must span 30% like the layout1-3 grid"
        );
        assert_eq!(grid[(24, 0)].ch(), ' ', "single gap column at the split");
        // The mail pane ring opens at the 30% boundary of the view.
        assert_eq!(grid[(25, 0)].ch(), '╭', "mail-view ring top-left");
        assert_eq!(grid[(79, 0)].ch(), '╮', "mail-view ring top-right");
        // No grid row (index-prefixed "0  <date>") anywhere on screen.
        for y in 0..area.height() {
            let row: String = (0..area.width()).map(|x| grid[(x, y)].ch()).collect();
            assert!(
                !row.contains("0  2026-"),
                "no grid row may render in layout4; row {y}: {row:.40?}"
            );
        }

        // The same geometry holds on a wide terminal: the two rings stay
        // separate 30/70 panes with one gap column (190 -> ring x0..56,
        // gap x57, mail ring x58..189), never one merged full-width
        // frame.
        assert!(screen.resize(190, 40));
        listing.set_dirty(true);
        if let Some(view) = listing.view.as_mut() {
            view.set_dirty(true);
        }
        let area = screen.area();
        listing.draw(screen.grid_mut(), area, &mut ctx);
        let grid = screen.grid();
        assert_eq!(
            grid[(56, 0)].ch(),
            '╮',
            "thread-list ring right edge at 30% of 190"
        );
        assert_eq!(grid[(57, 0)].ch(), ' ', "single gap column at 190 cols");
        assert_eq!(
            grid[(58, 0)].ch(),
            '╭',
            "mail-view ring top-left at 190 cols"
        );
        assert_eq!(
            grid[(189, 0)].ch(),
            '╮',
            "mail-view ring top-right at 190 cols"
        );

        // The real UI renders the listing inside a `Tabbed` container:
        // its outer body frame must not paint over the pinned listing's
        // own pane rings (that merged the layout4 pane tops into one
        // full-width border).
        let mut tabbed = crate::utilities::Tabbed::new(vec![Box::new(listing)], &ctx);
        tabbed.set_dirty(true);
        let area = screen.area();
        tabbed.draw(screen.grid_mut(), area, &mut ctx);
        let grid = screen.grid();
        assert_eq!(
            grid[(56, 0)].ch(),
            '╮',
            "Tabbed must not overwrite the thread-list ring edge"
        );
        assert_eq!(
            grid[(57, 0)].ch(),
            ' ',
            "gap column survives the Tabbed draw"
        );
        assert_eq!(
            grid[(58, 0)].ch(),
            '╭',
            "Tabbed must not overwrite the mail-view ring edge"
        );
    }

    /// Layout3 → layout4 (Right), layout4 roundtrips and returns to
    /// layout3 (Left / quit from the thread list).
    #[test]
    fn layout3_layout4_transitions() {
        let mut ctx = mock_context();
        let mut listing = pane_chain_setup(&mut ctx);
        assert!(pane_step(&mut listing, &mut ctx, Key::Char('j')));
        assert!(pane_step(&mut listing, &mut ctx, Key::Right));
        assert!(matches!(
            listing.view.as_ref().unwrap().thread_view_focus(),
            ThreadViewFocus::Thread
        ));

        // Right: layout4 — the thread list (30%) | mail view (70%), focus
        // on the thread list.
        assert!(pane_step(&mut listing, &mut ctx, Key::Right));
        assert!(matches!(listing.focus, ListingFocus::View));
        assert!(matches!(
            listing.view.as_ref().unwrap().thread_view_focus(),
            ThreadViewFocus::None
        ));

        // Right: focus the mail view (layout4 right pane).
        assert!(pane_step(&mut listing, &mut ctx, Key::Right));
        assert!(matches!(
            listing.view.as_ref().unwrap().thread_view_focus(),
            ThreadViewFocus::MailView
        ));

        // Left: back to the thread list.
        assert!(pane_step(&mut listing, &mut ctx, Key::Left));
        assert!(matches!(
            listing.view.as_ref().unwrap().thread_view_focus(),
            ThreadViewFocus::None
        ));

        // Left from the thread list: layout3 — the whole thread list
        // (70%), focus on the grid.
        assert!(pane_step(&mut listing, &mut ctx, Key::Left));
        assert!(matches!(listing.focus, ListingFocus::MailList));
        assert!(matches!(
            listing.view.as_ref().unwrap().thread_view_focus(),
            ThreadViewFocus::Thread
        ));
    }

    /// The full async search pipeline on layout2: the `search` command
    /// spawns a job; its completion (`JobFinished`) reaches the component,
    /// the filter applies and the grid shows only the matching rows with
    /// the cursor on the first result.
    #[test]
    fn layout2_search_full_job_pipeline() {
        let mut ctx = mock_context();
        let mut listing = pane_chain_setup(&mut ctx);
        assert!(pane_step(&mut listing, &mut ctx, Key::Right));

        // `search solo` — only the solo mail matches (subject "chain
        // solo"), the two-mail thread does not.
        let mut event = UIEvent::Action(Action::Listing(ListingAction::Search {
            term: "solo".to_string(),
            raw_search: false,
        }));
        assert!(listing.process_event(&mut event, &mut ctx));
        for _ in 0..8 {
            for mut ev in ctx.replies() {
                let _ = listing.process_event(&mut ev, &mut ctx);
            }
        }

        // Drive the job executor like the main loop does: dispatch every
        // JobFinished until the component's search job completes (other
        // background jobs finish too — refresh/init — and are ignored by
        // the component's search_job match).
        let mut handled = false;
        for _ in 0..100 {
            match ctx.receiver.recv_timeout(std::time::Duration::from_secs(5)) {
                Ok(crate::ThreadEvent::JobFinished(id)) => {
                    ctx.main_loop_handler.job_executor.set_job_finished(id);
                    let mut ev = UIEvent::StatusEvent(crate::types::StatusEvent::JobFinished(id));
                    if listing.process_event(&mut ev, &mut ctx) {
                        handled = true;
                        break;
                    }
                }
                Ok(crate::ThreadEvent::UIEvent(mut ev)) => {
                    let _ = listing.process_event(&mut ev, &mut ctx);
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        let _ = handled; // the JobFinished may also have been consumed in
                         // the earlier reply drain — the draw assertions below are the
                         // source of truth.
        for _ in 0..8 {
            for mut ev in ctx.replies() {
                let _ = listing.process_event(&mut ev, &mut ctx);
            }
        }

        listing.set_dirty(true);
        let theme_default = crate::conf::value(&ctx, "theme_default");
        let mut screen = Screen::<Virtual>::new(theme_default);
        assert!(screen.resize(80, 24));
        let area = screen.area();
        listing.draw(screen.grid_mut(), area, &mut ctx);
        let grid = screen.grid();
        let rows: Vec<String> = (0..area.height())
            .map(|y| (0..30).map(|x| grid[(x, y)].ch()).collect())
            .collect();
        // Exactly one matching row: the first grid row index is visible,
        // the second is not (the filter dropped the non-matching thread).
        assert_eq!(
            rows.iter().filter(|r| r.contains("│0")).count(),
            1,
            "exactly one row index visible: {rows:?}"
        );
        assert_eq!(
            rows.iter().filter(|r| r.contains("│1")).count(),
            0,
            "the non-matching row must be filtered out: {rows:?}"
        );
        assert!(
            rows.iter()
                .any(|r| r.chars().filter(|c| c.is_ascii_digit()).count() > 4),
            "the matching row must still render content: {rows:?}"
        );
        assert!(matches!(listing.focus, ListingFocus::MailList));
    }

    /// Regression: on a sync (local) backend the search must not run on
    /// the UI thread — the fallback scan reads every mail file in the
    /// mailbox, which would freeze the UI on large mailboxes. The Search
    /// action therefore spawns a job for every backend: right after the
    /// action the grid is still unfiltered (the filter can only be
    /// applied by this thread processing the job's `JobFinished`), and
    /// the filtered view appears only once the job completes.
    #[test]
    fn search_action_spawns_job_on_sync_backend() {
        let mut ctx = mock_context();
        // Precondition: the mock account is a local backend (no async
        // driver, no remote search) — the path that used to run the scan
        // inline under `futures::executor::block_on`.
        let account_hash = *ctx.accounts.iter().next().unwrap().0;
        assert!(matches!(
            ctx.accounts[&account_hash].is_async(),
            crate::jobs::IsAsync::Blocking
        ));
        assert!(
            !ctx.accounts[&account_hash]
                .backend_capabilities
                .supports_search
        );
        let mut listing = pane_chain_setup(&mut ctx);
        assert!(pane_step(&mut listing, &mut ctx, Key::Right));

        let mut event = UIEvent::Action(Action::Listing(ListingAction::Search {
            term: "solo".to_string(),
            raw_search: false,
        }));
        assert!(listing.process_event(&mut event, &mut ctx));

        let draw_rows = |listing: &mut Listing, ctx: &mut Context| -> Vec<String> {
            listing.set_dirty(true);
            let theme_default = crate::conf::value(ctx, "theme_default");
            let mut screen = Screen::<Virtual>::new(theme_default);
            assert!(screen.resize(80, 24));
            let area = screen.area();
            listing.draw(screen.grid_mut(), area, ctx);
            (0..area.height())
                .map(|y| (0..30).map(|x| screen.grid()[(x, y)].ch()).collect())
                .collect()
        };
        // The UI thread must be free immediately: the filter cannot have
        // been applied inline (only processing the job's `JobFinished`
        // applies it), so both rows are still on screen.
        let rows = draw_rows(&mut listing, &mut ctx);
        assert_eq!(
            rows.iter().filter(|r| r.contains("│0")).count(),
            1,
            "precondition: the unfiltered grid shows the first row"
        );
        assert!(
            rows.iter().any(|r| r.contains("│1")),
            "the search must not apply its filter on the UI thread \
             (the non-matching row is still visible)"
        );

        // Drive the job executor like the main loop does until the search
        // job's completion arrives and applies the filter.
        let mut filtered = false;
        for _ in 0..100 {
            match ctx.receiver.recv_timeout(std::time::Duration::from_secs(5)) {
                Ok(crate::ThreadEvent::JobFinished(id)) => {
                    ctx.main_loop_handler.job_executor.set_job_finished(id);
                    let mut ev = UIEvent::StatusEvent(crate::types::StatusEvent::JobFinished(id));
                    let _ = listing.process_event(&mut ev, &mut ctx);
                    for _ in 0..8 {
                        let replies = ctx.replies();
                        if replies.is_empty() {
                            break;
                        }
                        for mut ev in replies {
                            let _ = listing.process_event(&mut ev, &mut ctx);
                        }
                    }
                    let rows = draw_rows(&mut listing, &mut ctx);
                    if rows.iter().filter(|r| r.contains("│0")).count() == 1
                        && !rows.iter().any(|r| r.contains("│1"))
                    {
                        filtered = true;
                        break;
                    }
                }
                Ok(crate::ThreadEvent::UIEvent(mut ev)) => {
                    let _ = listing.process_event(&mut ev, &mut ctx);
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
        assert!(
            filtered,
            "the spawned search job must apply the filter once it completes"
        );
    }

    /// Right from layout1 (with the focus on the mailbox list) switches
    /// the layout directly: a single mail → layout2, a thread → layout3.
    #[test]
    fn layout1_menu_right_opens_directly() {
        for case in [false, true] {
            let mut ctx = mock_context();

            let mut listing = pane_chain_setup(&mut ctx);
            // Focus the mailbox list (from the grid with Left); for the
            // thread case move the grid cursor onto the thread entry
            // first (still in layout1).
            if case {
                assert!(pane_step(&mut listing, &mut ctx, Key::Char('j')));
            }
            assert!(pane_step(&mut listing, &mut ctx, Key::Left));
            assert!(matches!(listing.focus, ListingFocus::Menu));

            assert!(pane_step(&mut listing, &mut ctx, Key::Right));
            let view = listing.view.as_ref().expect("Right must open the entry");
            assert_eq!(
                view.is_single_mail(),
                !case,
                "Right must land on layout2 for a single mail, layout3 for a thread"
            );
            assert!(
                matches!(listing.focus, ListingFocus::MailList),
                "the opened layout keeps the focus on the grid"
            );
            assert!(!listing.is_menu_visible());
        }
    }

    /// Left from layout3's grid closes the view to layout1 with the focus
    /// on the mailbox list; quit from layout4 lands on layout3's grid.
    #[test]
    fn layout3_left_goes_to_mailbox_list_and_l4_quit() {
        let mut ctx = mock_context();
        let mut listing = pane_chain_setup(&mut ctx);
        assert!(pane_step(&mut listing, &mut ctx, Key::Char('j')));
        assert!(pane_step(&mut listing, &mut ctx, Key::Right));

        // Left from layout3's grid: layout1, focus on the mailbox list.
        assert!(pane_step(&mut listing, &mut ctx, Key::Left));
        assert!(listing.view.is_none());
        assert!(matches!(listing.focus, ListingFocus::Menu));
        assert!(listing.is_menu_visible());

        // Reopen from layout1 (Right from the mailbox list opens the
        // entry directly → layout3, Right → layout4's thread list,
        // Right → layout4's mail view), quit → layout3.
        assert!(pane_step(&mut listing, &mut ctx, Key::Right));
        assert!(pane_step(&mut listing, &mut ctx, Key::Right));
        assert!(pane_step(&mut listing, &mut ctx, Key::Right));
        assert!(matches!(
            listing.view.as_ref().unwrap().thread_view_focus(),
            ThreadViewFocus::MailView
        ));
        assert!(pane_step(&mut listing, &mut ctx, Key::Char('q')));
        assert!(matches!(listing.focus, ListingFocus::MailList));
        assert!(
            matches!(
                listing.view.as_ref().unwrap().thread_view_focus(),
                ThreadViewFocus::Thread
            ),
            "quit from layout4 must land on layout3"
        );

        // quit from layout3's grid: layout1, focus on the grid.
        assert!(pane_step(&mut listing, &mut ctx, Key::Char('q')));
        assert!(listing.view.is_none());
        assert!(matches!(listing.focus, ListingFocus::MailList));
    }

    /// The grid cursor move refreshes the open view and the layout follows
    /// the selection (single mail → layout2, thread → layout3).
    #[test]
    fn grid_cursor_moves_refresh_view_and_layout() {
        let mut ctx = mock_context();
        let mut listing = pane_chain_setup(&mut ctx);
        // Cursor on the solo mail → layout2.
        assert!(pane_step(&mut listing, &mut ctx, Key::Right));
        assert!(listing.view.as_ref().unwrap().is_single_mail());

        // Move down onto the thread → layout3 (grid focus kept).
        assert!(pane_step(&mut listing, &mut ctx, Key::Char('j')));
        for _ in 0..8 {
            let replies = ctx.replies();
            if replies.is_empty() {
                break;
            }
            for mut ev in replies {
                let _ = listing.process_event(&mut ev, &mut ctx);
            }
        }
        listing.set_dirty(true);
        let theme_default = crate::conf::value(&ctx, "theme_default");
        let mut screen = Screen::<Virtual>::new(theme_default);
        assert!(screen.resize(80, 24));
        let area = screen.area();
        listing.draw(screen.grid_mut(), area, &mut ctx);
        for _ in 0..8 {
            let replies = ctx.replies();
            if replies.is_empty() {
                break;
            }
            for mut ev in replies {
                let _ = listing.process_event(&mut ev, &mut ctx);
            }
        }
        let view = listing.view.as_ref().expect("view refreshed");
        assert!(
            !view.is_single_mail(),
            "the view must follow the cursor onto the thread (layout3)"
        );
        assert!(matches!(view.thread_view_focus(), ThreadViewFocus::Thread));
        assert!(matches!(listing.focus, ListingFocus::MailList));

        // Move back up onto the solo mail → layout2 again.
        assert!(pane_step(&mut listing, &mut ctx, Key::Char('k')));
        for _ in 0..8 {
            let replies = ctx.replies();
            if replies.is_empty() {
                break;
            }
            for mut ev in replies {
                let _ = listing.process_event(&mut ev, &mut ctx);
            }
        }
        listing.set_dirty(true);
        let theme_default = crate::conf::value(&ctx, "theme_default");
        let mut screen = Screen::<Virtual>::new(theme_default);
        assert!(screen.resize(80, 24));
        let area = screen.area();
        listing.draw(screen.grid_mut(), area, &mut ctx);
        for _ in 0..8 {
            let replies = ctx.replies();
            if replies.is_empty() {
                break;
            }
            for mut ev in replies {
                let _ = listing.process_event(&mut ev, &mut ctx);
            }
        }
        assert!(
            listing.view.as_ref().unwrap().is_single_mail(),
            "the view must follow the cursor back onto the single mail (layout2)"
        );
    }

    /// Layout1: moving the mailbox selection with the keyboard switches
    /// the mailbox right away (the grid follows, the focus stays).
    #[test]
    fn layout1_menu_scroll_switches_mailbox() {
        let mut ctx = mock_context();
        let (_a, inbox_hash, archive_hash) = register_two_mailboxes(&mut ctx);
        pin_pane_chain_keys(&mut ctx);
        let mut listing = make_drawn_listing(&mut ctx);

        // Focus the mailbox list (Left from the grid).
        assert!(pane_step(&mut listing, &mut ctx, Key::Left));
        assert!(matches!(listing.focus, ListingFocus::Menu));

        // Move down to Archive: the cursor applies, the grid follows, the
        // focus stays on the mailbox list.
        assert!(pane_step(&mut listing, &mut ctx, Key::Down));
        assert_eq!(listing.component.coordinates().1, archive_hash);
        assert_eq!(listing.cursor_pos.menu, listing.menu_cursor_pos.menu);
        assert!(
            matches!(listing.focus, ListingFocus::Menu),
            "the focus must stay on the mailbox list"
        );

        // Back up to INBOX.
        assert!(pane_step(&mut listing, &mut ctx, Key::Up));
        assert_eq!(listing.component.coordinates().1, inbox_hash);
        assert!(matches!(listing.focus, ListingFocus::Menu));
    }

    /// Layout1: scrolling the sidebar across the account boundary runs
    /// `change_account`, whose `close_view` sets the grid's
    /// `grid_has_keyboard`; the sidebar focus must take that highlight
    /// back (`focus_menu`), or both panes paint "pane.focused" at once
    /// while the outer ring says the grid is unfocused.
    #[test]
    fn menu_scroll_across_accounts_keeps_grid_dimmed() {
        let mut ctx = mock_context();
        register_second_account(&mut ctx);
        register_two_mailboxes(&mut ctx);
        pin_pane_chain_keys(&mut ctx);
        let mut listing = make_drawn_listing(&mut ctx);

        // Focus the mailbox list (Left from the grid).
        assert!(pane_step(&mut listing, &mut ctx, Key::Left));
        assert!(matches!(listing.focus, ListingFocus::Menu));

        // Scroll down past the last mailbox: the cursor wraps onto the
        // second account's status entry. (The status page covers the grid
        // pane while the cursor rests on a status entry, so the fill is
        // asserted after scrolling back to a mailbox below.)
        assert!(pane_step(&mut listing, &mut ctx, Key::Down));
        assert!(pane_step(&mut listing, &mut ctx, Key::Down));
        assert_eq!(
            listing.menu_cursor_pos.account, 1,
            "the cursor must have wrapped onto the second account"
        );
        assert!(matches!(listing.focus, ListingFocus::Menu));

        // Scroll back up: the previous account's last mailbox. The grid is
        // visible again and must stay dim — the sidebar still holds the
        // keyboard across both crossings.
        assert!(pane_step(&mut listing, &mut ctx, Key::Up));
        assert_eq!(listing.menu_cursor_pos.account, 0);
        assert!(
            matches!(listing.focus, ListingFocus::Menu),
            "the focus must stay on the mailbox list"
        );
        let theme_default = crate::conf::value(&ctx, "theme_default");
        let mut screen = Screen::<Virtual>::new(theme_default);
        assert!(screen.resize(80, 24));
        assert_grid_painted_dimmed(&mut screen, &mut listing, &mut ctx);
    }

    /// `search` must work with an open view (every pane-chain layout has
    /// one): the `/` key opens the command line, and the executed
    /// `Action::Listing(Search)` must reach the grid component instead of
    /// being dropped by an unfocused guard.
    #[test]
    fn search_action_runs_with_open_view() {
        let mut ctx = mock_context();
        let mut listing = pane_chain_setup(&mut ctx);
        assert!(pane_step(&mut listing, &mut ctx, Key::Right));
        assert!(
            listing.component.unfocused(),
            "sanity: the view is open (Entry state)"
        );

        // `search` is a two-key group by default (`/` and `F3`); both
        // bound keys must open the command line with the same
        // `CmdInput("search ")` reply.
        for key in [Key::Char('/'), Key::F(3)] {
            let label = format!("{key:?}");
            let mut event = UIEvent::Input(key);
            assert!(
                listing.process_event(&mut event, &mut ctx),
                "the search key {label} must be consumed with the view open"
            );
            assert!(
                ctx.replies()
                    .iter()
                    .any(|r| matches!(r, UIEvent::CmdInput(_))),
                "the search key {label} must open the command line"
            );
        }

        let mut event = UIEvent::Action(Action::Listing(ListingAction::Search {
            term: String::new(),
            raw_search: false,
        }));
        assert!(
            listing.process_event(&mut event, &mut ctx),
            "the search action must be handled with the view open"
        );
    }

    /// `refresh` is a two-key group by default (`F5` and `C-r`); both bound
    /// keys must reach the same listing handler. The handler has no state
    /// guard, so either key is consumed even with an open view. The mock
    /// account's backend rejects the synthetic mailbox hash and
    /// `Account::refresh` folds that error into `Ok(())`, leaving no reply to
    /// observe, so consumption is the observable asserted here (the shortcuts
    /// map itself is pinned in `conf::tests`).
    #[test]
    fn refresh_dual_key_is_consumed_by_listing() {
        let mut ctx = mock_context();
        let mut listing = pane_chain_setup(&mut ctx);

        for key in [Key::F(5), Key::Ctrl('r')] {
            let label = format!("{key:?}");
            let mut event = UIEvent::Input(key);
            assert!(
                listing.process_event(&mut event, &mut ctx),
                "the refresh key {label} must be consumed by a focused listing"
            );
        }

        // A key outside the group must not be swallowed as a refresh.
        let mut event = UIEvent::Input(Key::F(9));
        assert!(
            !listing.process_event(&mut event, &mut ctx),
            "an unbound key must not be consumed as refresh"
        );
    }

    /// With the mail view focused, `Action::Listing(Search)` is the pager's
    /// in-body search, not a grid filter: the listing must route it to the
    /// view instead of hijacking it for the grid component. The payload uses
    /// a term the grid component rejects synchronously (it parses the query),
    /// so an accidental delivery would queue a "Could not perform search"
    /// error; the pager accepts any literal pattern.
    #[test]
    fn search_action_routes_to_pager_in_view_focus() {
        let mut ctx = mock_context();
        let mut listing = pane_chain_setup(&mut ctx);
        // Right opens the single-mail view (focus stays on the grid), a
        // second Right hands the keyboard to the view.
        assert!(pane_step(&mut listing, &mut ctx, Key::Right));
        assert!(pane_step(&mut listing, &mut ctx, Key::Right));
        assert!(
            matches!(listing.focus, ListingFocus::View),
            "precondition: the mail view owns the keyboard"
        );
        assert!(listing.view.is_some(), "precondition: the view is open");

        // The pager only exists once the body is `Loaded`; drive the expanded
        // entry to that state so the in-body search has somewhere to land.
        let body = b"From: s@x.example\r\nTo: y@x.example\r\nSubject: chain solo\r\n\
                     Message-ID: <chain-solo@x.example>\r\nDate: Thu, 1 Jan 2026 00:02:00 \
                     +0000\r\n\r\nsolo\r\n"
            .to_vec();
        listing
            .view
            .as_mut()
            .unwrap()
            .load_expanded_entry_for_tests(body, &mut ctx);

        let mut event = UIEvent::Action(Action::Listing(ListingAction::Search {
            term: "~invalid~ ((".to_string(),
            raw_search: false,
        }));
        assert!(
            listing.process_event(&mut event, &mut ctx),
            "the pager must consume the in-body search"
        );
        assert!(
            !ctx.replies().iter().any(|r| matches!(
                r,
                UIEvent::Notification {
                    title,
                    kind: Some(crate::types::NotificationType::Error(_)),
                    ..
                } if title.as_deref() == Some("Could not perform search")
            )),
            "the grid component must not receive the pager's search action"
        );
    }
}
