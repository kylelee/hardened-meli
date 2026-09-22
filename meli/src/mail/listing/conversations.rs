/*
 * meli
 *
 * Copyright 2017-2018 Manos Pitsidianakis
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
    collections::{BTreeMap, HashMap},
    iter::FromIterator,
};

use indexmap::IndexSet;
use melib::{Address, SortField, SortOrder, TagHash, Threads};

use super::*;
use crate::{components::PageMovement, jobs::JoinHandle};

macro_rules! row_attr {
    ($field:ident, $color_cache:expr, unseen: $unseen:expr, highlighted: $highlighted:expr, selected: $selected:expr  $(,)*) => {{
        let color_cache = &$color_cache;
        let unseen = $unseen;
        let highlighted = $highlighted;
        let selected = $selected;
        ThemeAttribute {
            fg: if highlighted && selected {
                color_cache.highlighted_selected.fg
            } else if highlighted {
                color_cache.highlighted.fg
            } else if selected {
                color_cache.selected.fg
            } else if unseen {
                color_cache.unseen.fg
            } else {
                color_cache.$field.fg
            },
            bg: if highlighted && selected {
                color_cache.highlighted_selected.bg
            } else if highlighted {
                color_cache.highlighted.bg
            } else if selected {
                color_cache.selected.bg
            } else if unseen {
                color_cache.unseen.bg
            } else {
                color_cache.base.bg
            },
            attrs: if highlighted && selected {
                color_cache.highlighted_selected.attrs
            } else if highlighted {
                color_cache.highlighted.attrs
            } else if selected {
                color_cache.selected.attrs
            } else if unseen {
                color_cache.unseen.attrs
            } else {
                color_cache.$field.attrs
            },
        }
    }};
    ($color_cache:expr, unseen: $unseen:expr, highlighted: $highlighted:expr, selected: $selected:expr  $(,)*) => {{
        let color_cache = &$color_cache;
        let unseen = $unseen;
        let highlighted = $highlighted;
        let selected = $selected;
        ThemeAttribute {
            fg: if highlighted && selected {
                color_cache.highlighted_selected.fg
            } else if highlighted {
                color_cache.highlighted.fg
            } else if selected {
                color_cache.selected.fg
            } else if unseen {
                color_cache.unseen.fg
            } else {
                color_cache.theme_default.fg
            },
            bg: if highlighted && selected {
                color_cache.highlighted_selected.bg
            } else if highlighted {
                color_cache.highlighted.bg
            } else if selected {
                color_cache.selected.bg
            } else if unseen {
                color_cache.unseen.bg
            } else {
                color_cache.base.bg
            },
            attrs: if highlighted && selected {
                color_cache.highlighted_selected.attrs
            } else if highlighted {
                color_cache.highlighted.attrs
            } else if selected {
                color_cache.selected.attrs
            } else if unseen {
                color_cache.unseen.attrs
            } else {
                color_cache.theme_default.attrs
            },
        }
    }};
}

/// A list of all mail (`Envelope`s) in a `Mailbox`. On `\n` it opens the
/// `Envelope` content in a `ThreadView`.
#[derive(Debug)]
pub struct ConversationsListing {
    /// (x, y, z): x is accounts, y is mailboxes, z is index inside a mailbox.
    cursor_pos: (AccountHash, MailboxHash, usize),
    new_cursor_pos: (AccountHash, MailboxHash, usize),
    length: usize,
    sort: (SortField, SortOrder),
    subsort: (SortField, SortOrder),
    rows: RowsState<(ThreadHash, EnvelopeHash)>,
    error: std::result::Result<(), String>,

    #[allow(clippy::type_complexity)]
    search_job: Option<(String, MailboxHash, JoinHandle<Result<SearchResult>>)>,
    filter_term: String,
    filtered_selection: Vec<ThreadHash>,
    filtered_order: HashMap<ThreadHash, usize>,
    /// If we must redraw on next redraw event
    dirty: bool,
    force_draw: bool,
    /// If `self.view` is visible or not.
    focus: Focus,
    color_cache: ColorCache,

    movement: Option<PageMovement>,
    modifier_active: bool,
    modifier_command: Option<Modifier>,
    view_area: Option<Area>,
    /// Whether the grid (not the open view) holds the keyboard focus; the
    /// Entry-state subpane ring then renders focused.
    grid_has_keyboard: bool,
    parent: ComponentId,
    id: ComponentId,
}

impl MailListingTrait for ConversationsListing {
    fn row_updates(&mut self) -> &mut SmallVec<[EnvelopeHash; 8]> {
        &mut self.rows.row_updates
    }

    fn selection(&self) -> &HashMap<EnvelopeHash, bool> {
        &self.rows.selection
    }

    fn selection_mut(&mut self) -> &mut HashMap<EnvelopeHash, bool> {
        &mut self.rows.selection
    }

    fn get_focused_items(&self, _context: &Context) -> SmallVec<[EnvelopeHash; 8]> {
        let is_selection_empty = !self
            .rows
            .selection
            .values()
            .cloned()
            .any(std::convert::identity);
        let cursor_iter;
        let sel_iter = if !is_selection_empty {
            cursor_iter = None;
            Some(
                self.rows
                    .selection
                    .iter()
                    .filter(|(_, v)| **v)
                    .map(|(k, _)| *k),
            )
        } else {
            if let Some(env_hashes) = self
                .get_thread_under_cursor(self.cursor_pos.2)
                .and_then(|thread| self.rows.thread_to_env.get(&thread).cloned())
            {
                cursor_iter = Some(env_hashes.into_iter());
            } else {
                cursor_iter = None;
            }
            None
        };
        let iter = sel_iter
            .into_iter()
            .flatten()
            .chain(cursor_iter.into_iter().flatten());
        SmallVec::from_iter(iter)
    }

    fn refresh_mailbox(&mut self, context: &mut Context, force: bool) {
        self.set_dirty(true);
        let old_mailbox_hash = self.cursor_pos.1;
        let old_cursor_pos = self.cursor_pos;
        let same_mailbox = self.cursor_pos.0 == self.new_cursor_pos.0
            && self.cursor_pos.1 == self.new_cursor_pos.1;
        if !same_mailbox {
            self.cursor_pos.2 = 0;
            self.new_cursor_pos.2 = 0;
        }
        self.cursor_pos.1 = self.new_cursor_pos.1;
        self.cursor_pos.0 = self.new_cursor_pos.0;

        self.color_cache = ColorCache::new(context, IndexStyle::Conversations);

        // Get mailbox as a reference.
        //
        match context.accounts[&self.cursor_pos.0].load(self.cursor_pos.1, true) {
            Ok(()) => {}
            Err(_) => {
                let message: String =
                    context.accounts[&self.cursor_pos.0][&self.cursor_pos.1].status();
                self.error = Err(message);
                return;
            }
        }

        let threads = context.accounts[&self.cursor_pos.0]
            .collection
            .get_threads(self.cursor_pos.1);
        let mut roots = threads.roots();
        threads.group_inner_sort_by(
            &mut roots,
            self.sort,
            &context.accounts[&self.cursor_pos.0].collection.envelopes,
        );
        drop(threads);

        let previous_selection = self.rows.clear(same_mailbox);
        self.redraw_threads_list(
            context,
            Box::new(roots.into_iter()) as Box<dyn Iterator<Item = ThreadHash>>,
        );
        self.rows.restore_selection(previous_selection);

        if !force && old_cursor_pos == self.new_cursor_pos && old_mailbox_hash == self.cursor_pos.1
        {
            self.kick_parent(self.parent, ListingMessage::UpdateView, context);
        } else if self.unfocused()
            && self
                .get_thread_under_cursor(self.cursor_pos.2)
                .and_then(|thread| {
                    self.rows
                        .thread_to_env
                        .get(&thread)
                        .and_then(|e| Some((thread, e.first()?)))
                })
                .is_some()
        {
            self.force_draw = true;
            self.dirty = true;
            self.set_focus(Focus::Entry, context);
        }
    }

    fn redraw_threads_list(
        &mut self,
        context: &Context,
        items: Box<dyn Iterator<Item = ThreadHash>>,
    ) {
        let account = &context.accounts[&self.cursor_pos.0];

        let threads = account.collection.get_threads(self.cursor_pos.1);
        let tags_lck = account.collection.tag_index.read().unwrap();
        // Hold one envelope read guard for the whole rebuild: `make_entry_string`
        // needs the envelope map for the deterministic attachment check, and a
        // per-row `get_env` would both churn the lock and nest read locks.
        let envelopes = account.collection.envelopes.read().unwrap();

        self.length = 0;
        if self.error.is_err() {
            self.error = Ok(());
        }
        let mut max_entry_columns = 0;

        let mut other_subjects = IndexSet::new();
        let mut tags = IndexSet::new();
        let mut from_address_list = Vec::new();
        let mut from_address_set: std::collections::HashSet<Box<str>> =
            std::collections::HashSet::new();
        'items_for_loop: for thread in items {
            let thread_node = &threads.thread_nodes()[&threads.thread_ref(thread).root()];
            let root_env_hash = if let Some(h) = thread_node.message().or_else(|| {
                if thread_node.children().is_empty() {
                    return None;
                }
                let mut iter_ptr = thread_node.children()[0];
                while threads.thread_nodes()[&iter_ptr].message().is_none() {
                    if threads.thread_nodes()[&iter_ptr].children().is_empty() {
                        return None;
                    }
                    iter_ptr = threads.thread_nodes()[&iter_ptr].children()[0];
                }
                threads.thread_nodes()[&iter_ptr].message()
            }) {
                h
            } else {
                continue 'items_for_loop;
            };
            if !envelopes.contains_key(&root_env_hash) {
                //log::debug!("key = {}", root_env_hash);
                //log::debug!(
                //    "name = {} {}",
                //    account[&self.cursor_pos.1].name(),
                //    context.accounts[&self.cursor_pos.0].name()
                //);
                //log::debug!("{:#?}", context.accounts);

                continue 'items_for_loop;
            }
            let Some(root_envelope) = envelopes.get(&root_env_hash) else {
                // Stale thread root: skip the conversation row.
                continue 'items_for_loop;
            };
            use melib::search::QueryTrait;
            if let Some(filter_query) = mailbox_settings!(
                context[self.cursor_pos.0][&self.cursor_pos.1]
                    .listing
                    .filter
            )
            .as_ref()
            {
                if !root_envelope.is_match(filter_query) {
                    continue;
                }
            }
            other_subjects.clear();
            tags.clear();
            from_address_list.clear();
            from_address_set.clear();
            for (envelope, show_subject) in threads
                .thread_iter(thread)
                .filter_map(|(_, h)| {
                    Some((
                        threads.thread_nodes()[&h].message()?,
                        threads.thread_nodes()[&h].show_subject(),
                    ))
                })
                .filter_map(|(env_hash, show_subject)| {
                    Some((envelopes.get(&env_hash)?, show_subject))
                })
            {
                if show_subject {
                    other_subjects.insert(envelope.subject().to_string());
                }
                if account.backend_capabilities.supports_tags {
                    for &t in envelope.tags().iter() {
                        tags.insert(t);
                    }
                }

                for addr in envelope.from().iter() {
                    if addr.get_email().is_empty() || from_address_set.contains(addr.get_email()) {
                        continue;
                    }
                    from_address_set.insert(addr.get_email().into());
                    from_address_list.push(addr.clone());
                }
            }

            let strings = self.make_entry_string(
                root_envelope,
                context,
                &tags_lck,
                &from_address_list,
                &threads,
                &envelopes,
                &other_subjects,
                &tags,
                thread,
            );
            max_entry_columns = std::cmp::max(
                max_entry_columns,
                strings.flag.len()
                    + 3
                    + strings.subject.grapheme_width()
                    + 1
                    + strings.tags.grapheme_width(),
            );
            max_entry_columns = std::cmp::max(
                max_entry_columns,
                strings.date.len() + 1 + strings.from.grapheme_width(),
            );
            self.rows.insert_thread(
                thread,
                (thread, root_env_hash),
                threads
                    .thread_to_envelope
                    .get(&thread)
                    .cloned()
                    .unwrap_or_default()
                    .into(),
                strings,
            );
            self.length += 1;
        }

        if self.length == 0 && self.filter_term.is_empty() {
            let message: String = account[&self.cursor_pos.1].status();
            self.error = Err(message);
        }
    }
}

impl ListingTrait for ConversationsListing {
    fn coordinates(&self) -> (AccountHash, MailboxHash) {
        (self.new_cursor_pos.0, self.new_cursor_pos.1)
    }

    fn set_coordinates(&mut self, coordinates: (AccountHash, MailboxHash)) {
        if self.coordinates() != (coordinates.0, coordinates.1) {
            self.new_cursor_pos = (coordinates.0, coordinates.1, 0);
        }
        self.focus = Focus::None;
        self.filtered_selection.clear();
        self.filtered_order.clear();
        self.filter_term.clear();
        self.rows.row_updates.clear();
    }

    fn next_entry(&mut self, context: &mut Context) {
        if self
            .get_thread_under_cursor(self.new_cursor_pos.2 + 1)
            .is_some()
        {
            // [ref:TODO]: makes this less ugly.
            self.movement = Some(PageMovement::Down(1));
            self.perform_movement(None);
            self.force_draw = true;
            self.dirty = true;
            self.set_focus(Focus::Entry, context);
        }
    }

    fn prev_entry(&mut self, context: &mut Context) {
        if self.new_cursor_pos.2 == 0 {
            return;
        }
        if self
            .get_thread_under_cursor(self.new_cursor_pos.2 - 1)
            .is_some()
        {
            // [ref:TODO]: makes this less ugly.
            self.movement = Some(PageMovement::Up(1));
            self.perform_movement(None);
            self.force_draw = true;
            self.dirty = true;
            self.set_focus(Focus::Entry, context);
        }
    }

    fn highlight_line(&mut self, grid: &mut CellBuffer, area: Area, idx: usize, context: &Context) {
        if self.length == 0 {
            return;
        }
        self.draw_rows(grid, area, context, idx);
    }

    /// Draw the list of `Envelope`s.
    fn draw_list(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        if self.cursor_pos.1 != self.new_cursor_pos.1 || self.cursor_pos.0 != self.new_cursor_pos.0
        {
            self.refresh_mailbox(context, false);
        }
        // Pane background: the grid fills with "pane.focused" while it
        // holds the keyboard, "pane.unfocused" otherwise; rows keep their
        // own theme colors on top of it.
        let pane_fill = crate::conf::value(
            context,
            if self.grid_has_keyboard {
                "pane.focused"
            } else {
                "pane.unfocused"
            },
        );
        if let Err(message) = self.error.as_ref() {
            grid.clear_area(area, pane_fill);
            grid.write_string(
                message,
                self.color_cache.theme_default.fg,
                self.color_cache.theme_default.bg,
                self.color_cache.theme_default.attrs,
                area,
                None,
                None,
            );
            context.dirty_areas.push_back(area);
            return;
        }
        let rows = area.height() / 3;

        if rows == 0 {
            return;
        }

        self.perform_movement(Some(rows));

        let prev_page_no = (self.cursor_pos.2).wrapping_div(rows);
        let page_no = (self.new_cursor_pos.2).wrapping_div(rows);

        let top_idx = page_no * rows;

        // If cursor position has changed, remove the highlight from the previous
        // position and apply it in the new one.
        if self.cursor_pos.2 != self.new_cursor_pos.2 && prev_page_no == page_no {
            let old_cursor_pos = self.cursor_pos;
            self.cursor_pos = self.new_cursor_pos;
            for idx in &[old_cursor_pos.2, self.new_cursor_pos.2] {
                if *idx >= self.length {
                    continue; //bounds check
                }
                let new_area = area.skip_rows(3 * (*idx % rows)).take_rows(2);
                self.highlight_line(grid, new_area, *idx, context);
                context.dirty_areas.push_back(new_area);
            }
            if !self.force_draw {
                return;
            }
        } else if self.cursor_pos != self.new_cursor_pos {
            self.cursor_pos = self.new_cursor_pos;
        }
        if self.new_cursor_pos.2 >= self.length {
            self.new_cursor_pos.2 = self.length.saturating_sub(1);
            self.cursor_pos.2 = self.new_cursor_pos.2;
        }

        grid.clear_area(area, pane_fill);
        // Page_no has changed, so draw new page
        self.draw_rows(grid, area, context, top_idx);

        self.highlight_line(
            grid,
            area.skip_rows(3 * (self.cursor_pos.2 % rows)).take_rows(3),
            self.cursor_pos.2,
            context,
        );

        self.force_draw = false;
        context.dirty_areas.push_back(area);
    }

    fn filter(&mut self, filter_term: String, results: Vec<EnvelopeHash>, context: &Context) {
        if filter_term.is_empty() {
            return;
        }

        self.length = 0;
        self.filtered_selection.clear();
        self.filtered_order.clear();
        self.filter_term = filter_term;

        let account = &context.accounts[&self.cursor_pos.0];
        let threads = account.collection.get_threads(self.cursor_pos.1);
        let (mut missing_env, mut no_thread_node, mut not_in_rows, mut mapped) =
            (0usize, 0usize, 0usize, 0usize);
        let results_len = results.len();
        for env_hash in results {
            if !account.collection.contains_key(&env_hash) {
                missing_env += 1;
                continue;
            }
            let Some(env_thread_node_hash) = threads.envelope_to_thread_node.get(&env_hash) else {
                no_thread_node += 1;
                continue;
            };
            let Some(thread_node) = threads.thread_nodes.get(env_thread_node_hash) else {
                no_thread_node += 1;
                continue;
            };
            let thread = threads.find_group(thread_node.group);
            if self.filtered_order.contains_key(&thread) {
                continue;
            }
            if self.rows.all_threads.contains(&thread) {
                mapped += 1;
                self.filtered_selection.push(thread);
                self.filtered_order
                    .insert(thread, self.filtered_selection.len().saturating_sub(1));
            } else {
                not_in_rows += 1;
            }
        }
        log::debug!(
            "conversations filter `{}`: {} results -> {} mapped (missing_env {missing_env}, \
             no_thread_node {no_thread_node}, not_in_rows {not_in_rows}); mailbox {} has {} \
             threads in rows",
            self.filter_term,
            results_len,
            mapped,
            self.cursor_pos.1,
            self.rows.all_threads.len()
        );
        if !self.filtered_selection.is_empty() {
            threads.group_inner_sort_by(
                &mut self.filtered_selection,
                self.sort,
                &context.accounts[&self.cursor_pos.0].collection.envelopes,
            );
            self.new_cursor_pos.2 = 0;
        }
        let previous_selection = self.rows.clear(true);
        self.redraw_threads_list(
            context,
            Box::new(self.filtered_selection.clone().into_iter())
                as Box<dyn Iterator<Item = ThreadHash>>,
        );
        self.rows.restore_selection(previous_selection);
        // The row set was rebuilt: force a full list repaint — the
        // incremental `row_updates` path would leave stale rows on
        // screen until the next keypress.
        self.force_draw = true;
    }

    fn view_area(&self) -> Option<Area> {
        self.view_area
    }

    fn unfocused(&self) -> bool {
        !matches!(self.focus, Focus::None)
    }

    fn modifier_active(&self) -> bool {
        self.modifier_active
    }

    fn set_modifier_active(&mut self, new_val: bool) {
        self.modifier_active = new_val;
    }

    fn set_modifier_command(&mut self, new_val: Option<Modifier>) {
        self.modifier_command = new_val;
    }

    fn modifier_command(&self) -> Option<Modifier> {
        self.modifier_command
    }

    fn set_movement(&mut self, mvm: PageMovement) {
        self.movement = Some(mvm);
        self.set_dirty(true);
    }

    fn set_focus(&mut self, new_value: Focus, context: &mut Context) {
        match new_value {
            Focus::None => {
                self.dirty = true;
                // If self.rows.row_updates is not empty and we exit a thread, the row_update
                // events will be performed but the list will not be drawn. So force a draw in
                // any case.
                self.force_draw = true;
            }
            Focus::Entry => {
                if self.cursor_selection().is_some() {
                    self.force_draw = true;
                    self.dirty = true;
                    self.kick_open_under_cursor(context);
                    self.cursor_pos.2 = self.new_cursor_pos.2;
                } else {
                    return;
                }
            }
        }
        self.focus = new_value;
        self.kick_parent(
            self.parent,
            ListingMessage::FocusUpdate { new_value },
            context,
        );
    }

    fn focus(&self) -> Focus {
        self.focus
    }
}

impl std::fmt::Display for ConversationsListing {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "mail")
    }
}

impl ConversationsListing {
    /// The (thread, envelope) under the cursor, if any.
    pub(crate) fn cursor_selection(&self) -> Option<(ThreadHash, EnvelopeHash)> {
        self.get_thread_under_cursor(self.new_cursor_pos.2)
            .and_then(|thread| {
                self.rows
                    .thread_to_env
                    .get(&thread)
                    .and_then(|e| Some((thread, *e.first()?)))
            })
    }

    /// Queue an `OpenEntryUnderCursor` for the cursor entry: refreshes the
    /// open view to the newly selected mail while the grid holds the
    /// keyboard (the layout follows the selection).
    pub(crate) fn kick_open_under_cursor(&self, context: &mut Context) {
        if let Some((thread_hash, env_hash)) = self.cursor_selection() {
            self.kick_parent(
                self.parent,
                ListingMessage::OpenEntryUnderCursor {
                    thread_hash,
                    env_hash,
                    go_to_first_unread: true,
                },
                context,
            );
        }
    }

    /// Mark that the grid (not the open view) holds the keyboard focus;
    /// the Entry-state subpane ring then renders focused.
    pub(crate) fn set_grid_has_keyboard(&mut self, value: bool) {
        self.grid_has_keyboard = value;
        self.dirty = true;
        self.force_draw = true;
    }

    pub fn new(
        parent: ComponentId,
        coordinates: (AccountHash, MailboxHash),
        context: &Context,
    ) -> Box<Self> {
        let sort = *mailbox_settings!(context[coordinates.0][&coordinates.1].listing.sort);
        Box::new(Self {
            cursor_pos: (coordinates.0, MailboxHash::default(), 0),
            new_cursor_pos: (coordinates.0, coordinates.1, 0),
            length: 0,
            sort,
            subsort: (SortField::Date, SortOrder::Desc),
            rows: RowsState::default(),
            error: Ok(()),
            search_job: None,
            filter_term: String::new(),
            filtered_selection: Vec::new(),
            filtered_order: HashMap::default(),
            dirty: true,
            force_draw: true,
            focus: Focus::None,
            color_cache: ColorCache::default(),
            movement: None,
            modifier_active: false,
            modifier_command: None,
            view_area: None,
            grid_has_keyboard: false,
            parent,
            id: ComponentId::default(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn make_entry_string(
        &self,
        root_envelope: &Envelope,
        context: &Context,
        tags_lck: &BTreeMap<TagHash, String>,
        from: &[Address],
        threads: &Threads,
        envelopes: &HashMap<EnvelopeHash, Envelope>,
        other_subjects: &IndexSet<String>,
        tags_set: &IndexSet<TagHash>,
        hash: ThreadHash,
    ) -> EntryStrings {
        let thread = threads.thread_ref(hash);
        let mut tags = String::new();
        let mut colors = SmallVec::new();
        let account = &context.accounts[&self.cursor_pos.0];
        if account.backend_capabilities.supports_tags {
            let tags_iter = TagsIterator::new(
                tags_set.iter(),
                context,
                self.cursor_pos.0,
                self.cursor_pos.1,
                tags_lck,
            );
            for (t, c) in tags_iter {
                tags.push(' ');
                tags.push_str(t);
                tags.push(' ');
                colors.push(c);
            }
            if !tags.is_empty() {
                tags.pop();
            }
        }
        let subject = if *mailbox_settings!(
            context[self.cursor_pos.0][&self.cursor_pos.1]
                .listing
                .thread_subject_pack
        ) {
            other_subjects
                .into_iter()
                .fold(String::new(), |mut acc, s| {
                    if s.trim().is_empty() {
                        return acc;
                    }
                    if !acc.is_empty() {
                        acc.push_str(", ");
                    }
                    acc.push_str(s.trim());
                    acc
                })
        } else {
            root_envelope.subject().trim().to_string()
        };
        let subject = if thread.len() > 1 {
            format!("{} ({})", subject, thread.len())
        } else {
            subject
        };
        // Head the title with the attachment flag. This is the single
        // deterministic attachment marker for a conversation row: the flags
        // column no longer carries one, so a stale `Thread` counter cannot
        // make the paperclip flicker. Detection is envelope-level via
        // `thread_has_attachments` (see its docs).
        let subject = if thread_has_attachments(threads, envelopes, hash) {
            let flag = mailbox_settings!(
                context[self.cursor_pos.0][&self.cursor_pos.1]
                    .listing
                    .attachment_flag
            )
            .as_ref()
            .map(|s| s.as_str())
            .unwrap_or(DEFAULT_ATTACHMENT_FLAG);
            format!("{flag} {subject}")
        } else {
            subject
        };
        EntryStrings {
            date: DateString(self.format_date(context, thread.date())),
            subject: SubjectString(subject),
            flag: FlagString::new(
                root_envelope.flags(),
                self.rows
                    .selection
                    .get(&root_envelope.hash())
                    .cloned()
                    .unwrap_or(false),
                thread.snoozed(),
                thread.unseen() > 0,
                // The title head is the sole attachment marker (see above);
                // the flags column stays free of the paperclip.
                false,
                context,
                (self.cursor_pos.0, self.cursor_pos.1),
            ),
            from: FromString(Address::display_name_slice(from, None)),
            tags: TagString(tags, colors),
            unseen: thread.unseen() > 0,
            highlight_self: false,
        }
    }

    fn get_thread_under_cursor(&self, cursor: usize) -> Option<ThreadHash> {
        if self.filter_term.is_empty() {
            self.rows
                .thread_order
                .iter()
                .find(|(_, &r)| r == cursor)
                .map(|(k, _)| *k)
        } else {
            self.filtered_selection.get(cursor).cloned()
        }
    }

    fn update_line(&mut self, context: &Context, env_hash: EnvelopeHash) {
        let account = &context.accounts[&self.cursor_pos.0];
        let thread_hash = self.rows.env_to_thread[&env_hash];
        let threads = account.collection.get_threads(self.cursor_pos.1);
        let tags_lck = account.collection.tag_index.read().unwrap();
        // One envelope read guard for the row: `make_entry_string` needs the
        // map for the deterministic attachment check.
        let envelopes = account.collection.envelopes.read().unwrap();
        let idx: usize = self.rows.thread_order[&thread_hash];

        let mut other_subjects = IndexSet::new();
        let mut tags = IndexSet::new();
        let mut from_address_list = Vec::new();
        let mut from_address_set: std::collections::HashSet<Box<str>> =
            std::collections::HashSet::new();
        for (envelope, show_subject) in threads
            .thread_iter(thread_hash)
            .filter_map(|(_, h)| {
                threads.thread_nodes()[&h]
                    .message()
                    .map(|env_hash| (env_hash, threads.thread_nodes()[&h].show_subject()))
            })
            .filter_map(|(env_hash, show_subject)| Some((envelopes.get(&env_hash)?, show_subject)))
        {
            if show_subject {
                other_subjects.insert(envelope.subject().to_string());
            }
            if account.backend_capabilities.supports_tags {
                for &t in envelope.tags().iter() {
                    tags.insert(t);
                }
            }
            for addr in envelope.from().iter() {
                if addr.get_email().is_empty() || from_address_set.contains(addr.get_email()) {
                    continue;
                }
                from_address_set.insert(addr.get_email().into());
                from_address_list.push(addr.clone());
            }
        }
        let Some(envelope) = envelopes.get(&env_hash) else {
            // Stale row: the envelope was removed, leave the previous entry
            // strings in place instead of fabricating a row.
            log::error!(
                "Could not update conversation row: envelope {env_hash} is no longer in the mailbox"
            );
            return;
        };
        let strings = self.make_entry_string(
            envelope,
            context,
            &tags_lck,
            &from_address_list,
            &threads,
            &envelopes,
            &other_subjects,
            &tags,
            thread_hash,
        );
        if let Some(row) = self.rows.entries.get_mut(idx) {
            row.1 = strings;
        }
    }

    fn draw_rows(&self, grid: &mut CellBuffer, area: Area, context: &Context, top_idx: usize) {
        let pane_fill = crate::conf::value(
            context,
            if self.grid_has_keyboard {
                "pane.focused"
            } else {
                "pane.unfocused"
            },
        );
        let account = &context.accounts[&self.cursor_pos.0];
        let threads = account.collection.get_threads(self.cursor_pos.1);
        grid.clear_area(area, pane_fill);
        for (idx, ((thread_hash, root_env_hash), strings)) in
            self.rows.entries.iter().enumerate().skip(top_idx)
        {
            if !context.accounts[&self.cursor_pos.0].contains_key(*root_env_hash) {
                continue;
            }
            let area = area.skip_rows(3 * (idx - top_idx)).take_rows(3);
            let thread = threads.thread_ref(*thread_hash);

            // The four `row_attr!` inputs are shared by the flag, subject,
            // date and from rows: evaluate them once per row. In
            // particular `is_thread_selected` walks the whole thread (one
            // `selection` HashMap lookup per envelope), so calling it per
            // themed column cost 4 × thread length lookups per row/frame.
            let unseen = thread.unseen() > 0;
            let highlighted = self.cursor_pos.2 == idx;
            let selected = self.rows.is_thread_selected(*thread_hash);

            // A base conversation block (not the cursor block, not
            // selected) sits directly on the pane background: the four
            // themed rows keep their fg/attrs accents (unseen bold, zebra
            // fg, subject/from/date colors) but their bg follows the pane,
            // so an unfocused grid dims as a whole. Highlighted and
            // selected blocks keep their own fills.
            let on_pane = !highlighted && !selected;
            let themed = |attr: ThemeAttribute| {
                if on_pane {
                    ThemeAttribute {
                        bg: pane_fill.bg,
                        ..attr
                    }
                } else {
                    attr
                }
            };

            let row_attr = themed(row_attr!(
                self.color_cache,
                unseen: unseen,
                highlighted: highlighted,
                selected: selected
            ));
            // draw flags
            let (mut x, _) = grid.write_string(
                &strings.flag,
                row_attr.fg,
                row_attr.bg,
                row_attr.attrs,
                area,
                None,
                None,
            );
            if !strings.flag.is_empty() {
                for c in grid.row_iter(area, x..(x + 1), 0) {
                    grid[c].set_bg(row_attr.bg);
                }
                x += 1;
            }
            let subject_attr = themed(row_attr!(
                subject,
                self.color_cache,
                unseen: unseen,
                highlighted: highlighted,
                selected: selected
            ));
            // draw subject
            let (x_, subject_overflowed) = grid.write_string(
                &strings.subject,
                subject_attr.fg,
                subject_attr.bg,
                subject_attr.attrs,
                area.skip_cols(x),
                None,
                None,
            );
            x += x_;
            let mut subject_overflowed = subject_overflowed > 0;
            for (t, &color) in strings.tags.split_whitespace().zip(strings.tags.1.iter()) {
                if subject_overflowed {
                    break;
                };
                let area = area.skip_cols(x).take_cols(t.grapheme_width() + 2);
                let color = color.unwrap_or(self.color_cache.tag_default.bg);
                let (_x, _y) = grid.write_string(
                    t,
                    self.color_cache.tag_default.fg,
                    color,
                    self.color_cache.tag_default.attrs,
                    area.skip_cols(1),
                    None,
                    None,
                );
                if _y > 0 {
                    subject_overflowed = true;
                    break;
                }
                for c in grid.row_iter(area, 0..area.width(), 0) {
                    grid[c]
                        .set_keep_fg(true)
                        .set_bg(color)
                        .set_keep_bg(true)
                        .set_attrs(self.color_cache.tag_default.attrs);
                }
                x += _x + 2;
            }
            if !subject_overflowed {
                for c in grid.row_iter(area, x..area.width(), 0) {
                    grid[c].set_ch(' ').set_fg(row_attr.fg).set_bg(row_attr.bg);
                }
            }
            // Next line, draw date
            let date_attr = themed(row_attr!(
                date,
                self.color_cache,
                unseen: unseen,
                highlighted: highlighted,
                selected: selected
            ));
            x = 0;
            x += grid
                .write_string(
                    &strings.date,
                    date_attr.fg,
                    date_attr.bg,
                    date_attr.attrs,
                    area.skip(x, 1),
                    None,
                    None,
                )
                .0;
            for c in grid.row_iter(area, x..(x + 4), 1) {
                grid[c].set_ch('▁').set_fg(row_attr.fg).set_bg(row_attr.bg);
            }
            x += 4;
            let from_attr = themed(row_attr!(
                from,
                self.color_cache,
                unseen: unseen,
                highlighted: highlighted,
                selected: selected
            ));
            // draw from
            x += grid
                .write_string(
                    &strings.from,
                    from_attr.fg,
                    from_attr.bg,
                    from_attr.attrs,
                    area.skip(x, 1),
                    None,
                    None,
                )
                .0;

            for c in grid.row_iter(area, x..area.width(), 1) {
                grid[c].set_ch('▁').set_fg(row_attr.fg).set_bg(row_attr.bg);
            }
        }
    }

    fn perform_movement(&mut self, height: Option<usize>) {
        let rows = height.unwrap_or(1);
        if let Some(mvm) = self.movement.take() {
            match mvm {
                PageMovement::Up(amount) => {
                    self.new_cursor_pos.2 = self.new_cursor_pos.2.saturating_sub(amount);
                }
                PageMovement::PageUp(multiplier) => {
                    self.new_cursor_pos.2 = self.new_cursor_pos.2.saturating_sub(rows * multiplier);
                }
                PageMovement::Down(amount) => {
                    if self.new_cursor_pos.2 + amount + 1 < self.length {
                        self.new_cursor_pos.2 += amount;
                    } else {
                        self.new_cursor_pos.2 = self.length.saturating_sub(1);
                    }
                }
                PageMovement::PageDown(multiplier) => {
                    if self.new_cursor_pos.2 + rows * multiplier + 1 < self.length {
                        self.new_cursor_pos.2 += rows * multiplier;
                    } else if self.new_cursor_pos.2 + rows * multiplier > self.length {
                        self.new_cursor_pos.2 = self.length.saturating_sub(1);
                    } else {
                        self.new_cursor_pos.2 = (self.length.saturating_sub(1) / rows) * rows;
                    }
                }
                PageMovement::Right(_) | PageMovement::Left(_) => {}
                PageMovement::Home => {
                    self.new_cursor_pos.2 = 0;
                }
                PageMovement::End => {
                    self.new_cursor_pos.2 = self.length.saturating_sub(1);
                }
            }
        }
    }
}

impl Component for ConversationsListing {
    fn draw(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        if !self.is_dirty() {
            return;
        }

        {
            let mut area = area;

            if !self.filter_term.is_empty() {
                let (x, y) = grid.write_string(
                    &format!(
                        "{} results for `{}` (Press ESC to exit)",
                        self.filtered_selection.len(),
                        self.filter_term
                    ),
                    self.color_cache.theme_default.fg,
                    self.color_cache.theme_default.bg,
                    self.color_cache.theme_default.attrs,
                    area,
                    None,
                    Some(0),
                );
                for c in grid.row_iter(area, x..area.width(), y) {
                    grid[c] = Cell::default();
                }

                grid.clear_area(area.skip(x, y), self.color_cache.theme_default);
                context.dirty_areas.push_back(area.nth_row(0));

                area = area.skip_rows(1);
            }
            /* Common subpane geometry, computed once for every branch
             * below. In `Focus::Entry` the conversation list keeps
             * rendering as a subpane in the left third while the parent
             * listing skips its own pane frame (the open `ThreadView`
             * draws its own), so the subpane must draw its own rounded
             * frame too — unfocused-styled while the keyboard focus sits
             * in the `ThreadView` (the same convention the `ThreadView`'s
             * internal panes follow), focused while the grid holds the
             * keyboard. `Focus::None` keeps the parent listing's outer
             * pane frame and draws edge to edge. The row math below must
             * share this inner-area basis with `draw_list` (it derives
             * its own rows from the area passed in), otherwise a row
             * refresh would use rows/offsets shifted by the ring and
             * paint over it. */
            // In `Focus::Entry` the grid shrinks to the left 30% column
            // and the view takes the right 70%.
            let list_inner = if matches!(self.focus, Focus::Entry) {
                let (list_area, _) = crate::mail::pane_split(area);
                let inner = draw_rounded_frame(
                    grid,
                    list_area,
                    if self.grid_has_keyboard {
                        crate::conf::value(context, "tab.focused")
                    } else {
                        crate::conf::value(context, "tab.unfocused")
                    },
                );
                for frame_area in frame_flush_areas(grid, list_area) {
                    context.dirty_areas.push_back(frame_area);
                }
                let pane_fill = crate::conf::value(
                    context,
                    if self.grid_has_keyboard {
                        "pane.focused"
                    } else {
                        "pane.unfocused"
                    },
                );
                grid.clear_area(inner, pane_fill);
                inner
            } else {
                area
            };
            let rows = list_inner.height() / 3;
            if rows == 0 {
                /* Initialize coordinates/rows via `draw_list`'s refresh
                 * path; its own `rows == 0` guard then stops it before
                 * rendering anything. */
                if matches!(self.focus, Focus::Entry) {
                    /* The subpane's ring is up but no list row fits its
                     * inner area; still hand the view its split area so
                     * it does not paint over the ring (degenerate pane
                     * heights 3-4). */
                    let (_, entry_area) = crate::mail::pane_split(area);
                    let gap_area = crate::mail::pane_gap(area);
                    grid.clear_area(gap_area, self.color_cache.theme_default);
                    context.dirty_areas.push_back(gap_area);
                    self.view_area = entry_area.into();
                }
                self.dirty = false;
                return;
            }
            if let Some(modifier) = self.modifier_command.take() {
                if let Some(mvm) = self.movement.as_ref() {
                    match mvm {
                        PageMovement::Up(amount) => {
                            for c in self.new_cursor_pos.2.saturating_sub(*amount)
                                ..=self.new_cursor_pos.2
                            {
                                if let Some(thread) = self.get_thread_under_cursor(c) {
                                    self.rows.update_selection_with_thread(
                                        thread,
                                        match modifier {
                                            Modifier::SymmetricDifference => {
                                                |e: &mut bool| *e = !*e
                                            }
                                            Modifier::Union => |e: &mut bool| *e = true,
                                            Modifier::Difference => |e: &mut bool| *e = false,
                                            Modifier::Intersection => |_: &mut bool| {},
                                        },
                                    );
                                }
                            }
                            if modifier == Modifier::Intersection {
                                for c in (0..self.new_cursor_pos.2.saturating_sub(*amount))
                                    .chain((self.new_cursor_pos.2 + 2)..self.length)
                                {
                                    if let Some(thread) = self.get_thread_under_cursor(c) {
                                        self.rows
                                            .update_selection_with_thread(thread, |e| *e = false);
                                    }
                                }
                            }
                        }
                        PageMovement::PageUp(multiplier) => {
                            for c in self.new_cursor_pos.2.saturating_sub(rows * multiplier)
                                ..=self.new_cursor_pos.2
                            {
                                if let Some(thread) = self.get_thread_under_cursor(c) {
                                    self.rows.update_selection_with_thread(
                                        thread,
                                        match modifier {
                                            Modifier::SymmetricDifference => {
                                                |e: &mut bool| *e = !*e
                                            }
                                            Modifier::Union => |e: &mut bool| *e = true,
                                            Modifier::Difference => |e: &mut bool| *e = false,
                                            Modifier::Intersection => |_: &mut bool| {},
                                        },
                                    );
                                }
                            }
                        }
                        PageMovement::Down(amount) => {
                            for c in self.new_cursor_pos.2
                                ..std::cmp::min(self.length, self.new_cursor_pos.2 + amount + 1)
                            {
                                if let Some(thread) = self.get_thread_under_cursor(c) {
                                    self.rows.update_selection_with_thread(
                                        thread,
                                        match modifier {
                                            Modifier::SymmetricDifference => {
                                                |e: &mut bool| *e = !*e
                                            }
                                            Modifier::Union => |e: &mut bool| *e = true,
                                            Modifier::Difference => |e: &mut bool| *e = false,
                                            Modifier::Intersection => |_: &mut bool| {},
                                        },
                                    );
                                }
                            }
                            if modifier == Modifier::Intersection {
                                for c in (0..self.new_cursor_pos.2).chain(
                                    (std::cmp::min(self.length, self.new_cursor_pos.2 + amount + 1)
                                        + 1)..self.length,
                                ) {
                                    if let Some(thread) = self.get_thread_under_cursor(c) {
                                        self.rows
                                            .update_selection_with_thread(thread, |e| *e = false);
                                    }
                                }
                            }
                        }
                        PageMovement::PageDown(multiplier) => {
                            for c in self.new_cursor_pos.2
                                ..std::cmp::min(
                                    self.new_cursor_pos.2 + rows * multiplier + 1,
                                    self.length,
                                )
                            {
                                if let Some(thread) = self.get_thread_under_cursor(c) {
                                    self.rows.update_selection_with_thread(
                                        thread,
                                        match modifier {
                                            Modifier::SymmetricDifference => {
                                                |e: &mut bool| *e = !*e
                                            }
                                            Modifier::Union => |e: &mut bool| *e = true,
                                            Modifier::Difference => |e: &mut bool| *e = false,
                                            Modifier::Intersection => |_: &mut bool| {},
                                        },
                                    );
                                }
                            }
                            if modifier == Modifier::Intersection {
                                for c in (0..self.new_cursor_pos.2).chain(
                                    (std::cmp::min(
                                        self.new_cursor_pos.2 + rows * multiplier + 1,
                                        self.length,
                                    ) + 1)..self.length,
                                ) {
                                    if let Some(thread) = self.get_thread_under_cursor(c) {
                                        self.rows
                                            .update_selection_with_thread(thread, |e| *e = false);
                                    }
                                }
                            }
                        }
                        PageMovement::Right(_) | PageMovement::Left(_) => {}
                        PageMovement::Home => {
                            for c in 0..=self.new_cursor_pos.2 {
                                if let Some(thread) = self.get_thread_under_cursor(c) {
                                    self.rows.update_selection_with_thread(
                                        thread,
                                        match modifier {
                                            Modifier::SymmetricDifference => {
                                                |e: &mut bool| *e = !*e
                                            }
                                            Modifier::Union => |e: &mut bool| *e = true,
                                            Modifier::Difference => |e: &mut bool| *e = false,
                                            Modifier::Intersection => |_: &mut bool| {},
                                        },
                                    );
                                }
                            }
                            if modifier == Modifier::Intersection {
                                for c in (self.new_cursor_pos.2 + 1)..self.length {
                                    if let Some(thread) = self.get_thread_under_cursor(c) {
                                        self.rows
                                            .update_selection_with_thread(thread, |e| *e = false);
                                    }
                                }
                            }
                        }
                        PageMovement::End => {
                            for c in self.new_cursor_pos.2..self.length {
                                if let Some(thread) = self.get_thread_under_cursor(c) {
                                    self.rows.update_selection_with_thread(
                                        thread,
                                        match modifier {
                                            Modifier::SymmetricDifference => {
                                                |e: &mut bool| *e = !*e
                                            }
                                            Modifier::Union => |e: &mut bool| *e = true,
                                            Modifier::Difference => |e: &mut bool| *e = false,
                                            Modifier::Intersection => |_: &mut bool| {},
                                        },
                                    );
                                }
                            }
                            if modifier == Modifier::Intersection {
                                for c in 0..self.new_cursor_pos.2 {
                                    if let Some(thread) = self.get_thread_under_cursor(c) {
                                        self.rows
                                            .update_selection_with_thread(thread, |e| *e = false);
                                    }
                                }
                            }
                        }
                    }
                }
                self.force_draw = true;
            }

            if !self.rows.row_updates.is_empty() {
                // certain rows need to be updated (eg an unseen message was just set seen)
                while let Some(env_hash) = self.rows.row_updates.pop() {
                    if !self.rows.env_to_thread.contains_key(&env_hash) {
                        self.refresh_mailbox(context, true);
                        self.set_dirty(true);
                        break;
                    }
                    self.update_line(context, env_hash);
                    let row: usize = self.rows.env_order[&env_hash];
                    let page_no = (self.new_cursor_pos.2).wrapping_div(rows);

                    let top_idx = page_no * rows;
                    // Update row only if it's currently visible
                    if row >= top_idx && row < top_idx + rows {
                        let area = list_inner.skip_rows(3 * (row % rows)).take_rows(3);
                        self.highlight_line(grid, area, row, context);
                        context.dirty_areas.push_back(area);
                    }
                }
                if self.force_draw {
                    // Draw the entire list
                    self.draw_list(grid, list_inner, context);
                    self.force_draw = false;
                }
            } else {
                // Draw the entire list
                self.draw_list(grid, list_inner, context);
            }
        }
        if matches!(self.focus, Focus::Entry) {
            if self.length == 0 && self.dirty {
                let pane_fill = crate::conf::value(
                    context,
                    if self.grid_has_keyboard {
                        "pane.focused"
                    } else {
                        "pane.unfocused"
                    },
                );
                grid.clear_area(area, pane_fill);
                context.dirty_areas.push_back(area);
                return;
            }

            let (_, entry_area) = crate::mail::pane_split(area);
            let gap_area = crate::mail::pane_gap(area);
            grid.clear_area(gap_area, self.color_cache.theme_default);
            context.dirty_areas.push_back(gap_area);
            self.view_area = entry_area.into();
        }
        self.dirty = false;
    }

    fn process_event(&mut self, event: &mut UIEvent, context: &mut Context) -> bool {
        // Only the `UIEvent::Input` arms below resolve shortcut
        // bindings, so skip rebuilding (and re-hashing) the shortcut
        // maps for every other event: backend syncs can deliver
        // hundreds of non-key events per second.
        let shortcuts = if matches!(event, UIEvent::Input(_)) {
            self.shortcuts(context)
        } else {
            ShortcutMaps::default()
        };

        if let (UIEvent::VisibilityChange(true), _) = (&*event, self.focus) {
            self.force_draw = true;
            self.set_dirty(true);
            return true;
        }

        if self.length > 0 {
            match *event {
                UIEvent::Input(ref key)
                    if !self.unfocused()
                        && shortcut!(key == shortcuts[Shortcuts::LISTING]["select_entry"]) =>
                {
                    if let Some(thread) = self.get_thread_under_cursor(self.new_cursor_pos.2) {
                        self.rows.update_selection_with_thread(thread, |e| *e = !*e);
                        self.set_dirty(true);
                    }
                    return true;
                }
                UIEvent::Input(ref key)
                    if !self.unfocused()
                        && shortcut!(key == shortcuts[Shortcuts::LISTING]["select_motion"]) =>
                {
                    if self.modifier_active && self.modifier_command.is_none() {
                        self.modifier_command = Some(Modifier::default());
                    }
                    return true;
                }
                UIEvent::EnvelopeRename(ref old_hash, ref new_hash) => {
                    let account = &context.accounts[&self.cursor_pos.0];
                    let threads = account.collection.get_threads(self.cursor_pos.1);
                    if !account.collection.contains_key(new_hash) {
                        return false;
                    }
                    let Some(env_thread_node_hash) = threads.envelope_to_thread_node.get(new_hash)
                    else {
                        return false;
                    };
                    let Some(thread_node) = threads.thread_nodes.get(env_thread_node_hash) else {
                        return false;
                    };
                    let thread: ThreadHash = threads.find_group(thread_node.group);
                    drop(threads);
                    if self.rows.thread_order.contains_key(&thread) {
                        self.rows.rename_env(*old_hash, *new_hash);
                    }

                    self.set_dirty(true);
                }
                UIEvent::EnvelopeRemove(ref _env_hash, ref thread_hash) => {
                    if self.rows.thread_order.contains_key(thread_hash) {
                        self.refresh_mailbox(context, false);
                        self.set_dirty(true);
                    }
                }
                UIEvent::EnvelopeUpdate(ref env_hash) => {
                    let account = &context.accounts[&self.cursor_pos.0];
                    let threads = account.collection.get_threads(self.cursor_pos.1);
                    if !account.collection.contains_key(env_hash) {
                        return false;
                    }
                    let Some(env_thread_node_hash) = threads.envelope_to_thread_node.get(env_hash)
                    else {
                        return false;
                    };
                    let Some(thread_node) = threads.thread_nodes.get(env_thread_node_hash) else {
                        return false;
                    };
                    let thread: ThreadHash = threads.find_group(thread_node.group);
                    drop(threads);
                    if self.rows.thread_order.contains_key(&thread) {
                        self.rows.row_updates.push(*env_hash);
                    }

                    self.set_dirty(true);
                }
                UIEvent::Action(ref action) => match action {
                    Action::Sort(field, order) if !self.unfocused() => {
                        let new_order = (*field, *order);
                        if new_order != self.sort {
                            // "Keep it coming"
                            self.sort = (*field, *order);
                            self.refresh_mailbox(context, false);
                            self.set_dirty(true);
                        }
                        return true;
                    }
                    Action::SubSort(field, order) if !self.unfocused() => {
                        self.subsort = (*field, *order);
                        return true;
                    }
                    Action::Listing(ToggleThreadSnooze) if !self.unfocused() => {
                        //if let Some(thread) = self.get_thread_under_cursor(self.cursor_pos.2) {
                        //    let account = &mut context.accounts[&self.cursor_pos.0];
                        //    account
                        //        .collection
                        //        .threads
                        //        .write()
                        //        .unwrap()
                        //        .entry(self.cursor_pos.1)
                        //        .and_modify(|threads| {
                        //            let is_snoozed = threads.thread_ref(thread).snoozed();
                        //            threads.thread_ref_mut(thread).set_snoozed(!is_snoozed);
                        //        });
                        //    self.rows.row_updates.push(thread);
                        //    self.refresh_mailbox(context, false);
                        //}
                        return true;
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        match *event {
            UIEvent::ConfigReload { old_settings: _ } => {
                self.color_cache = ColorCache::new(context, IndexStyle::Conversations);
                self.refresh_mailbox(context, true);
                self.set_dirty(true);
            }
            UIEvent::MailboxUpdate((ref idxa, ref idxf))
                if (*idxa, *idxf) == (self.new_cursor_pos.0, self.cursor_pos.1) =>
            {
                self.refresh_mailbox(context, false);
                self.set_dirty(true);
            }
            UIEvent::StartupCheck(ref f) if *f == self.cursor_pos.1 => {
                self.refresh_mailbox(context, false);
                self.set_dirty(true);
            }
            UIEvent::ChangeMode(UIMode::Normal) => {
                self.set_dirty(true);
            }
            UIEvent::Resize => {
                self.set_dirty(true);
            }
            UIEvent::Action(Action::Listing(Search {
                term: ref filter_term,
                raw_search,
            })) => {
                // The search must work with an open view too (the grid
                // pane keeps rendering the filtered rows); it is a
                // listing-level operation, not a grid-focus one.
                //
                // Every backend runs the search as a job on the executor
                // thread pool: remote backends on the reactor thread,
                // local ones on the blocking pool. A local fallback scan
                // reads every mail file in the mailbox, so driving it on
                // this thread would freeze the UI on large mailboxes. The
                // completion (`JobFinished`) applies the filter.
                match context.accounts[&self.cursor_pos.0].search(
                    filter_term,
                    raw_search,
                    self.sort,
                    self.cursor_pos.1,
                ) {
                    Ok(job) => {
                        let handle = context.accounts[&self.cursor_pos.0]
                            .main_loop_handler
                            .job_executor
                            .spawn(
                                "search".into(),
                                job,
                                context.accounts[&self.cursor_pos.0].is_async(),
                            );
                        self.search_job =
                            Some((filter_term.to_string(), self.cursor_pos.1, handle));
                    }
                    Err(err) => {
                        context.replies.push_back(UIEvent::Notification {
                            title: Some("Could not perform search".into()),
                            source: None,
                            body: err.to_string().into(),
                            kind: Some(crate::types::NotificationType::Error(err.kind)),
                        });
                    }
                };
                self.set_dirty(true);
                return true;
            }
            UIEvent::Input(Key::Esc) | UIEvent::Input(Key::Char('\x1b'))
                if !self.unfocused() && !&self.filter_term.is_empty() =>
            {
                self.set_coordinates((self.new_cursor_pos.0, self.new_cursor_pos.1));
                self.refresh_mailbox(context, false);
                self.set_dirty(true);
                return true;
            }
            UIEvent::StatusEvent(StatusEvent::JobFinished(ref job_id))
                if self
                    .search_job
                    .as_ref()
                    .map(|(_, _, j)| j == job_id)
                    .unwrap_or(false) =>
            {
                let (filter_term, mailbox_hash, mut handle) = self.search_job.take().unwrap();
                match handle.chan.try_recv() {
                    Err(_) => { /* search was canceled */ }
                    Ok(None) => { /* something happened, perhaps a worker thread panicked */ }
                    Ok(Some(Ok(results))) => {
                        log::debug!(
                            "search job finished: {} results for {:?}",
                            results.envelopes.len(),
                            filter_term
                        );
                        super::notify_if_search_degraded(context, &results);
                        if self.cursor_pos.1 == mailbox_hash {
                            self.filter(filter_term, results.envelopes, context)
                        } else {
                            // The user switched mailboxes while the scan
                            // was running: applying the old mailbox's
                            // hashes here would filter the new one with
                            // stale results. Drop them.
                            log::debug!(
                                "dropping stale search results for mailbox {mailbox_hash:?}; \
                                 the listing now shows mailbox {:?}",
                                self.cursor_pos.1
                            );
                        }
                    }
                    Ok(Some(Err(err))) => {
                        context.replies.push_back(UIEvent::Notification {
                            title: Some("Could not perform search".into()),
                            source: None,
                            body: err.to_string().into(),
                            kind: Some(crate::types::NotificationType::Error(err.kind)),
                        });
                    }
                }
                self.set_dirty(true);
            }
            _ => {}
        }

        false
    }

    fn is_dirty(&self) -> bool {
        self.dirty || self.force_draw || !self.rows.row_updates.is_empty()
    }

    fn set_dirty(&mut self, value: bool) {
        self.dirty = value;
    }

    fn shortcuts(&self, context: &Context) -> ShortcutMaps {
        let mut map = ShortcutMaps::default();

        map.insert(
            Shortcuts::LISTING,
            context.settings.shortcuts.listing.key_values(),
        );

        map
    }

    fn id(&self) -> ComponentId {
        self.id
    }
}

#[cfg(test)]
mod tests {
    use melib::backends::{
        BackendMailbox, Mailbox, MailboxHash, MailboxPermissions, SpecialUsageMailbox,
    };
    use melib::Result;

    use super::*;
    use crate::{
        accounts::{build_mailboxes_order, MailboxEntry, MailboxStatus},
        conf::FileMailboxConf,
        terminal::{Screen, Virtual},
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

    /// Register an `INBOX` mailbox on the mock account, modeled on the
    /// `register_two_mailboxes` helper in `meli/src/golden.rs` and the
    /// `listing_menu_tests` precedent.
    fn register_inbox(context: &mut Context) -> (AccountHash, MailboxHash) {
        let account_hash = *context.accounts.iter().next().unwrap().0;
        let mailbox_hash = MailboxHash::from_bytes(b"INBOX");
        let account = context.accounts.get_mut(&account_hash).unwrap();
        account.mailbox_entries.insert(
            mailbox_hash,
            MailboxEntry::new(
                MailboxStatus::Available,
                "INBOX".to_string(),
                Box::new(TestMailbox {
                    hash: mailbox_hash,
                    name: "INBOX".to_string(),
                }),
                FileMailboxConf::default(),
            ),
        );
        build_mailboxes_order(
            &mut account.tree,
            &account.mailbox_entries,
            &mut account.mailboxes_order,
        );
        (account_hash, mailbox_hash)
    }

    /// Read one screen row as its plain text (symbols only).
    fn grid_row_text(grid: &CellBuffer, y: usize) -> String {
        (0..grid.cols).map(|x| grid[(x, y)].ch()).collect()
    }

    /// An open entry's row refresh (e.g. a seen receipt arriving for the
    /// open thread) must stay inside the conversation subpane's rounded
    /// frame: the row-update path derives its row strips from the same
    /// inner area `draw_list` renders in, so the ring keeps its own
    /// cells and the refreshed content lands one row below the ring.
    #[test]
    fn conversations_entry_row_update_stays_inside_frame() {
        let bytes = b"From: Carol Example <carol@example.org>\r\nTo: Bob Example <bob@example.org>\r\nSubject: row update frame mail\r\nMessage-ID: <row-update-solo@x.example>\r\nDate: Thu, 2 Jan 2025 09:30:00 +0000\r\n\r\nrow update body line\r\n";
        let mut ctx = crate::golden::mock_context();
        let (account_hash, inbox_hash) = register_inbox(&mut ctx);
        let mut env = Envelope::from_bytes(bytes, None).unwrap();
        env.set_flags(melib::Flag::SEEN);
        let env_hash = env.hash();
        ctx.accounts[&account_hash]
            .collection
            .insert(env, inbox_hash);

        let mut listing =
            ConversationsListing::new(ComponentId::default(), (account_hash, inbox_hash), &ctx);
        let theme_default = crate::conf::value(&ctx, "theme_default");
        let mut screen = Screen::<Virtual>::new(theme_default);
        assert!(screen.resize(80, 24));
        let area = screen.area();
        // First draw in Focus::None populates `rows` so the cursor has a
        // thread to open; the kick_parent messages of set_focus stay
        // unconsumed in the reply queue (there is no parent component
        // here).
        listing.draw(screen.grid_mut(), area, &mut ctx);
        listing.set_focus(Focus::Entry, &mut ctx);
        listing.draw(screen.grid_mut(), area, &mut ctx);
        let tab_unfocused = crate::conf::value(&ctx, "tab.unfocused");
        {
            let grid = screen.grid();
            // Full 80-col pane: the subpane takes 30% (x=0..=23), so its
            // ring owns columns 0 and 23 and rows 0 and 23.
            assert_eq!(grid[(0, 0)].ch(), '╭', "subpane top-left ring corner");
            assert_eq!(grid[(23, 0)].ch(), '╮', "subpane top-right ring corner");
            let first_row = grid_row_text(grid, 1);
            assert!(
                first_row.contains("row update frame"),
                "first conversation row must render inside the subpane frame; row 1 was {first_row:?}"
            );
        }

        // The row-update path alone (no full redraw): the env hash lands
        // in `rows.row_updates` and only that row's strip is redrawn.
        listing.rows.row_updates.push(env_hash);
        listing.set_dirty(true);
        listing.draw(screen.grid_mut(), area, &mut ctx);
        let grid = screen.grid();
        assert_eq!(
            grid[(0, 0)].ch(),
            '╭',
            "row update must not paint over the ring's top-left corner"
        );
        assert_eq!(
            grid[(23, 0)].ch(),
            '╮',
            "row update must not paint over the ring's top-right corner"
        );
        for y in 1..23 {
            assert_eq!(
                grid[(23, y)].ch(),
                '│',
                "ring right column must survive the row refresh at y={y}"
            );
            assert_eq!(
                grid[(23, y)].fg(),
                tab_unfocused.fg,
                "ring right column must keep the unfocused attr at y={y}"
            );
        }
        assert_eq!(grid[(0, 23)].ch(), '╰', "subpane bottom-left ring corner");
        assert_eq!(grid[(23, 23)].ch(), '╯', "subpane bottom-right ring corner");
        let first_row = grid_row_text(grid, 1);
        assert!(
            first_row.contains("row update frame"),
            "refreshed row must stay at the inner row below the ring; row 1 was {first_row:?}"
        );
        println!("conversations_entry_row_update_stays_inside_frame: ring pinned");
    }

    /// Degenerate pane heights (3-4 rows) in `Focus::Entry`: the
    /// subpane's inner area fits no conversation row, but the subpane
    /// ring is still drawn and the view must still receive its split
    /// area (right 70%) instead of falling back to the whole
    /// pane, which would paint over the ring.
    #[test]
    fn conversations_entry_tiny_height_keeps_split_area() {
        let bytes = b"From: Carol Example <carol@example.org>\r\nTo: Bob Example <bob@example.org>\r\nSubject: tiny height mail\r\nMessage-ID: <tiny-height-solo@x.example>\r\nDate: Thu, 2 Jan 2025 09:30:00 +0000\r\n\r\ntiny body line\r\n";
        let mut ctx = crate::golden::mock_context();
        let (account_hash, inbox_hash) = register_inbox(&mut ctx);
        let mut env = Envelope::from_bytes(bytes, None).unwrap();
        env.set_flags(melib::Flag::SEEN);
        ctx.accounts[&account_hash]
            .collection
            .insert(env, inbox_hash);

        let mut listing =
            ConversationsListing::new(ComponentId::default(), (account_hash, inbox_hash), &ctx);
        let theme_default = crate::conf::value(&ctx, "theme_default");
        let mut screen = Screen::<Virtual>::new(theme_default);
        assert!(screen.resize(80, 4));
        let area = screen.area();
        listing.draw(screen.grid_mut(), area, &mut ctx);
        listing.set_focus(Focus::Entry, &mut ctx);
        listing.draw(screen.grid_mut(), area, &mut ctx);
        let grid = screen.grid();
        // The subpane ring survives even though no row fits inside it.
        assert_eq!(grid[(0, 0)].ch(), '╭', "subpane top-left ring corner");
        assert_eq!(grid[(23, 0)].ch(), '╮', "subpane top-right ring corner");
        assert_eq!(grid[(0, 3)].ch(), '╰', "subpane bottom-left ring corner");
        assert_eq!(grid[(23, 3)].ch(), '╯', "subpane bottom-right ring corner");
        // The view keeps the split area: x=25..=79 on every row.
        let view_area = listing
            .view_area()
            .expect("tiny-height Entry state must still set the view area");
        assert_eq!(view_area.upper_left(), (25, 0));
        assert_eq!(view_area.width(), 55);
        println!("conversations_entry_tiny_height_keeps_split_area: split area pinned");
    }

    /// A thread with any attachment-bearing envelope must head its title with
    /// exactly one attachment flag, immediately before the title start; the
    /// flags column carries no paperclip. Attachment-free threads show none.
    #[test]
    fn conversations_thread_attachment_flag_heads_title() {
        let mut ctx = crate::golden::mock_context();
        let (account_hash, inbox_hash) = register_inbox(&mut ctx);
        // Thread of two: the root carries no attachment, the reply does —
        // the flag must still show, since *any* envelope in the thread
        // having an attachment marks the whole row.
        let root = Envelope::from_bytes(
            b"From: Carol Example <carol@example.org>\r\nTo: Bob Example <bob@example.org>\r\nSubject: attached thread mail\r\nMessage-ID: <attached-root@x.example>\r\nDate: Thu, 2 Jan 2025 09:30:00 +0000\r\n\r\nroot body\r\n",
            None,
        )
        .unwrap();
        let reply = Envelope::from_bytes(
            b"From: Carol Example <carol@example.org>\r\nTo: Bob Example <bob@example.org>\r\nSubject: attachment inside here\r\nMessage-ID: <attached-reply@x.example>\r\nIn-Reply-To: <attached-root@x.example>\r\nReferences: <attached-root@x.example>\r\nDate: Thu, 2 Jan 2025 10:30:00 +0000\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"bnd\"\r\n\r\n--bnd\nContent-Type: text/plain; charset=utf-8\n\nsee attached\n--bnd\nContent-Type: application/pdf; name=\"doc.pdf\"\nContent-Disposition: attachment; filename=\"doc.pdf\"\n\n%PDF-1.4 fake\n--bnd--\n",
            None,
        )
        .unwrap();
        assert!(
            reply.has_attachments(),
            "test reply must parse as bearing an attachment"
        );
        let plain = Envelope::from_bytes(
            b"From: Carol Example <carol@example.org>\r\nTo: Bob Example <bob@example.org>\r\nSubject: plain thread mail\r\nMessage-ID: <plain@x.example>\r\nDate: Thu, 2 Jan 2025 11:30:00 +0000\r\n\r\nplain body\r\n",
            None,
        )
        .unwrap();
        let collection = &mut ctx.accounts[&account_hash].collection;
        collection.insert(root, inbox_hash);
        collection.insert(reply, inbox_hash);
        collection.insert(plain, inbox_hash);

        let mut listing =
            ConversationsListing::new(ComponentId::default(), (account_hash, inbox_hash), &ctx);
        let theme_default = crate::conf::value(&ctx, "theme_default");
        let mut screen = Screen::<Virtual>::new(theme_default);
        assert!(screen.resize(80, 24));
        let area = screen.area();
        listing.draw(screen.grid_mut(), area, &mut ctx);
        let grid = screen.grid();
        let row_texts: Vec<String> = (0..grid.rows).map(|y| grid_row_text(grid, y)).collect();
        let attached_row = row_texts
            .iter()
            .find(|r| r.contains("attachment inside here"))
            .expect("thread with an attachment must be drawn");
        // `thread_subject_pack` (default on) folds the thread's differing
        // subjects into the title; whichever subject comes first marks the
        // title start.
        let title_start = ["attached thread mail", "attachment inside here"]
            .iter()
            .filter_map(|s| attached_row.find(s))
            .min()
            .expect("thread title must be drawn");
        let plain_row = row_texts
            .iter()
            .find(|r| r.contains("plain thread mail"))
            .expect("attachment-free thread must be drawn");
        assert!(
            !plain_row.contains('📎'),
            "attachment-free thread must not show the flag; row was {plain_row:?}"
        );
        // Both rows carry the same flags, so the attachment-free row's title
        // starts at the same column as the attached row's subject string. The
        // title-head flag must therefore begin exactly there, and the flags
        // column before it must be free of the paperclip; a marker left in the
        // flags column would instead start earlier.
        let subject_col = plain_row
            .find("plain thread mail")
            .expect("attachment-free title must be drawn");
        let icon_at = attached_row
            .find('📎')
            .expect("attached row must show an attachment flag");
        assert_eq!(
            icon_at, subject_col,
            "the attachment flag must head the title, not sit in the flags column; row was \
             {attached_row:?}"
        );
        assert!(
            !attached_row[..subject_col].contains('📎'),
            "the flags column must carry no attachment flag; row was {attached_row:?}"
        );
        let gap = &attached_row[icon_at + '📎'.len_utf8()..title_start];
        assert!(
            gap.chars().all(|c| c == ' ' || c == '\u{FE0E}'),
            "attachment flag must sit immediately before the title, gap was {gap:?}"
        );
        assert_eq!(
            attached_row.matches('📎').count(),
            1,
            "the title head must be the only attachment flag; row was {attached_row:?}"
        );
    }
}
