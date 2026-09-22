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
    convert::TryInto,
    iter::FromIterator,
};

use indexmap::IndexSet;
use melib::{Address, SortField, SortOrder, TagHash, Threads};

use super::*;
use crate::{
    components::PageMovement,
    jobs::JoinHandle,
    segment_tree::SegmentTree,
    terminal::{draw_rounded_frame, frame_flush_areas},
};

macro_rules! row_attr {
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
                color_cache.base.fg
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
                color_cache.base.attrs
            },
        }
    }};
}

/// A list of all mail (`Envelope`s) in a `Mailbox`. On `\n` it opens the
/// `Envelope` content in a `ThreadView`.
#[derive(Debug)]
pub struct CompactListing {
    /// (x, y, z): x is accounts, y is mailboxes, z is index inside a mailbox.
    cursor_pos: (AccountHash, MailboxHash, usize),
    new_cursor_pos: (AccountHash, MailboxHash, usize),
    length: usize,
    sort: (SortField, SortOrder),
    subsort: (SortField, SortOrder),
    /// Cache current view.
    data_columns: DataColumns<5>,
    rows_drawn: SegmentTree,
    rows: RowsState<(ThreadHash, EnvelopeHash)>,

    #[allow(clippy::type_complexity)]
    search_job: Option<(String, MailboxHash, JoinHandle<Result<SearchResult>>)>,
    #[allow(clippy::type_complexity)]
    select_job: Option<(String, MailboxHash, JoinHandle<Result<SearchResult>>)>,
    filter_term: String,
    filtered_selection: Vec<ThreadHash>,
    filtered_order: HashMap<ThreadHash, usize>,
    /// If we must redraw on next redraw event
    dirty: bool,
    force_draw: bool,
    /// If `self.view` exists or not.
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

impl MailListingTrait for CompactListing {
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
            .selection()
            .values()
            .cloned()
            .any(std::convert::identity);
        let cursor_iter;
        let sel_iter = if !is_selection_empty {
            cursor_iter = None;
            Some(
                self.selection()
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

    /// Fill the `self.data_columns` `CellBuffers` with the contents of the
    /// account mailbox the user has chosen.
    fn refresh_mailbox(&mut self, context: &mut Context, force: bool) {
        self.set_dirty(true);
        let old_cursor_pos = self.cursor_pos;
        let same_mailbox = self.cursor_pos.0 == self.new_cursor_pos.0
            && self.cursor_pos.1 == self.new_cursor_pos.1;
        if !same_mailbox {
            self.cursor_pos.2 = 0;
            self.new_cursor_pos.2 = 0;
        }
        self.cursor_pos.1 = self.new_cursor_pos.1;
        self.cursor_pos.0 = self.new_cursor_pos.0;

        self.color_cache = ColorCache::new(context, IndexStyle::Compact);

        // Get mailbox as a reference.
        //
        match context.accounts[&self.cursor_pos.0].load(self.cursor_pos.1, true) {
            Ok(()) => {}
            Err(_) => {
                self.length = 0;
                let message: String =
                    context.accounts[&self.cursor_pos.0][&self.cursor_pos.1].status();
                if self.data_columns.columns[0].resize_with_context(message.len(), 1, context) {
                    let area_col_0 = self.data_columns.columns[0].area();
                    self.data_columns.columns[0].grid_mut().write_string(
                        message.as_str(),
                        self.color_cache.theme_default.fg,
                        self.color_cache.theme_default.bg,
                        self.color_cache.theme_default.attrs,
                        area_col_0,
                        None,
                        None,
                    );
                }
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

        if self
            .get_thread_under_cursor(self.cursor_pos.2)
            .and_then(|thread| {
                self.rows
                    .thread_to_env
                    .get(&thread)
                    .and_then(|e| Some((thread, e.first()?)))
            })
            .is_some()
        {
            if !force && old_cursor_pos == self.new_cursor_pos {
                self.kick_parent(self.parent, ListingMessage::UpdateView, context);
            } else if self.unfocused() {
                self.force_draw = true;
                self.dirty = true;
                self.set_focus(Focus::Entry, context);
            }
        }
    }

    fn redraw_threads_list(
        &mut self,
        context: &Context,
        items: Box<dyn Iterator<Item = ThreadHash>>,
    ) {
        let account = &context.accounts[&self.cursor_pos.0];
        let threads = account.collection.get_threads(self.cursor_pos.1);

        self.length = 0;
        let mut min_width = (0, 0, 0, 0, 0);
        #[allow(clippy::type_complexity)]
        let mut row_widths: (
            SmallVec<[u8; 1024]>,
            SmallVec<[u8; 1024]>,
            SmallVec<[u8; 1024]>,
            SmallVec<[u8; 1024]>,
            SmallVec<[u8; 1024]>,
        ) = (
            SmallVec::new(),
            SmallVec::new(),
            SmallVec::new(),
            SmallVec::new(),
            SmallVec::new(),
        );

        let tags_lck = account.collection.tag_index.read().unwrap();
        // Hold one envelope read guard for the whole rebuild: `make_entry_string`
        // needs the envelope map for the deterministic attachment check, and a
        // per-row `get_env` would both churn the lock and nest read locks.
        let envelopes = account.collection.envelopes.read().unwrap();

        let mut other_subjects = IndexSet::new();
        let mut tags = IndexSet::new();
        let mut from_address_list = Vec::new();
        let mut from_address_set: std::collections::HashSet<Box<str>> =
            std::collections::HashSet::new();
        let mut highlight_self: bool;
        let my_address: Address = context.accounts[&self.cursor_pos.0]
            .settings
            .account
            .main_identity_address();
        let should_highlight_self = mailbox_settings!(
            context[self.cursor_pos.0][&self.cursor_pos.1]
                .listing
                .highlight_self
        )
        .is_true();
        let highlight_self_colwidth: usize = mailbox_settings!(
            context[self.cursor_pos.0][&self.cursor_pos.1]
                .listing
                .highlight_self_flag
        )
        .as_ref()
        .map(|s| s.as_str())
        .unwrap_or(super::DEFAULT_HIGHLIGHT_SELF_FLAG)
        .grapheme_width();
        let mut itoa_buffer = itoa::Buffer::new();
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

                continue;
            }
            let Some(root_envelope) = envelopes.get(&root_env_hash) else {
                // Stale thread root: skip the row instead of drawing a bogus one.
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
            highlight_self = false;
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

                highlight_self |= should_highlight_self
                    && (envelope.recipient_any(&my_address) || envelope.sender_any(&my_address));
                for addr in envelope.from().iter() {
                    if addr.get_email().is_empty() || from_address_set.contains(addr.get_email()) {
                        continue;
                    }
                    from_address_set.insert(addr.get_email().into());
                    from_address_list.push(addr.clone());
                }
            }

            let row_attr = row_attr!(
                self.color_cache,
                unseen: threads.thread_ref(thread).unseen() > 0,
                highlighted: false,
                selected: false
            );
            self.rows.row_attr_cache.insert(self.length, row_attr);

            let entry_strings = self.make_entry_string(
                root_envelope,
                context,
                &tags_lck,
                &from_address_list,
                &threads,
                &envelopes,
                &other_subjects,
                &tags,
                highlight_self,
                thread,
            );
            row_widths.0.push(
                itoa_buffer
                    .format(self.length)
                    .len()
                    .try_into()
                    .unwrap_or(255),
            );
            /* date */
            row_widths.1.push(
                entry_strings
                    .date
                    .grapheme_width()
                    .try_into()
                    .unwrap_or(255),
            );
            /* from */
            row_widths.2.push(
                entry_strings
                    .from
                    .grapheme_width()
                    .try_into()
                    .unwrap_or(255),
            );
            row_widths.3.push(
                (entry_strings.flag.grapheme_width()
                    + usize::from(entry_strings.highlight_self) * highlight_self_colwidth)
                    .try_into()
                    .unwrap_or(255),
            );
            row_widths.4.push(
                (entry_strings.subject.grapheme_width() + 1 + entry_strings.tags.grapheme_width())
                    .try_into()
                    .unwrap_or(255),
            );
            min_width.1 = min_width.1.max(entry_strings.date.grapheme_width()); /* date */
            min_width.2 = min_width.2.max(entry_strings.from.grapheme_width()); /* from */
            min_width.3 = min_width.3.max(
                entry_strings.flag.grapheme_width()
                    + usize::from(entry_strings.highlight_self) * highlight_self_colwidth,
            );
            min_width.4 = min_width.4.max(
                entry_strings.subject.grapheme_width() + 1 + entry_strings.tags.grapheme_width(),
            ); /* subject */
            self.rows.insert_thread(
                thread,
                (thread, root_env_hash),
                threads
                    .thread_to_envelope
                    .get(&thread)
                    .cloned()
                    .unwrap_or_default()
                    .into(),
                entry_strings,
            );
            self.length += 1;
        }

        min_width.0 = itoa_buffer.format(self.length.saturating_sub(1)).len();

        self.data_columns.elasticities[0].set_rigid();
        self.data_columns.elasticities[1].set_rigid();
        self.data_columns.elasticities[2].set_grow(15, Some(35));
        self.data_columns.elasticities[3].set_rigid();
        self.data_columns.elasticities[4].set_rigid();
        self.data_columns
            .cursor_config
            .set_handle(true)
            .set_theme(self.color_cache.highlighted);
        self.data_columns
            .theme_config
            .set_theme(self.color_cache.base);

        /* index column */
        _ = self.data_columns.columns[0].resize_with_context(min_width.0, self.rows.len(), context);
        /* date column */
        _ = self.data_columns.columns[1].resize_with_context(min_width.1, self.rows.len(), context);
        /* from column */
        _ = self.data_columns.columns[2].resize_with_context(min_width.2, self.rows.len(), context);
        // flags column
        _ = self.data_columns.columns[3].resize_with_context(min_width.3, self.rows.len(), context);
        // subject column
        _ = self.data_columns.columns[4].resize_with_context(min_width.4, self.rows.len(), context);
        self.data_columns.segment_tree[0] = row_widths.0.into();
        self.data_columns.segment_tree[1] = row_widths.1.into();
        self.data_columns.segment_tree[2] = row_widths.2.into();
        self.data_columns.segment_tree[3] = row_widths.3.into();
        self.data_columns.segment_tree[4] = row_widths.4.into();

        self.rows_drawn =
            SegmentTree::from(std::iter::repeat_n(1, self.rows.len()).collect::<SmallVec<_>>());
        debug_assert_eq!(self.rows_drawn.array.len(), self.rows.len());
        self.draw_rows(context, 0, 80.min(self.rows.len().saturating_sub(1)));
        if self.length == 0 && self.filter_term.is_empty() {
            let message: String = account[&self.cursor_pos.1].status();
            if self.data_columns.columns[0].resize_with_context(message.len(), 1, context) {
                let area_col_0 = self.data_columns.columns[0].area();
                self.data_columns.columns[0].grid_mut().write_string(
                    &message,
                    self.color_cache.theme_default.fg,
                    self.color_cache.theme_default.bg,
                    self.color_cache.theme_default.attrs,
                    area_col_0,
                    None,
                    None,
                );
            }
        }
    }
}

impl ListingTrait for CompactListing {
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
        let thread_hash = if let Some(h) = self.get_thread_under_cursor(idx) {
            h
        } else {
            return;
        };

        let account = &context.accounts[&self.cursor_pos.0];
        let threads = account.collection.get_threads(self.cursor_pos.1);
        let thread = threads.thread_ref(thread_hash);

        let highlighted = self.cursor_pos.2 == idx;
        let selected = self.rows.is_thread_selected(thread_hash);
        let row_attr = row_attr!(
            self.color_cache,
            unseen: thread.unseen() > 0,
            highlighted: highlighted,
            selected: selected
        );
        // A base row (not the cursor row, not selected) is repainted on
        // the pane background so un-highlighting a row hands its bg back
        // to the pane fill; highlighted and selected rows keep their own.
        let pane_fill = crate::conf::value(
            context,
            if self.grid_has_keyboard {
                "pane.focused"
            } else {
                "pane.unfocused"
            },
        );
        let row_attr = if highlighted || selected {
            row_attr
        } else {
            ThemeAttribute {
                bg: pane_fill.bg,
                ..row_attr
            }
        };
        let x = self.data_columns.widths[0]
            + self.data_columns.widths[1]
            + self.data_columns.widths[2]
            + 3 * 2;

        for c in grid.row_iter(area, 0..area.width(), 0) {
            grid[c]
                .set_fg(row_attr.fg)
                .set_bg(row_attr.bg)
                .set_attrs(row_attr.attrs);
        }

        grid.copy_area(
            self.data_columns.columns[3].grid(),
            area.skip_cols(x),
            self.data_columns.columns[3].area().nth_row(idx),
        );
        for c in grid.row_iter(area, x..area.width(), 0) {
            grid[c].set_bg(row_attr.bg).set_attrs(row_attr.attrs);
        }
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
        if self.length == 0 {
            grid.clear_area(area, pane_fill);

            grid.copy_area(
                self.data_columns.columns[0].grid(),
                area,
                self.data_columns.columns[0].area(),
            );
            context.dirty_areas.push_back(area);
            self.force_draw = false;
            return;
        }
        let rows = area.height();
        if rows == 0 {
            return;
        }

        self.perform_movement(Some(rows));

        if self.force_draw {
            grid.clear_area(area, pane_fill);
        }

        let prev_page_no = (self.cursor_pos.2).wrapping_div(rows);
        let page_no = (self.new_cursor_pos.2).wrapping_div(rows);

        let top_idx = page_no * rows;
        let end_idx = self.length.saturating_sub(1).min(top_idx + rows - 1);
        self.draw_rows(context, top_idx, end_idx);

        /* If cursor position has changed, remove the highlight from the previous
         * position and apply it in the new one. */
        if self.cursor_pos.2 != self.new_cursor_pos.2 && prev_page_no == page_no {
            let old_cursor_pos = self.cursor_pos;
            self.cursor_pos = self.new_cursor_pos;
            for &(idx, highlight) in &[(old_cursor_pos.2, false), (self.new_cursor_pos.2, true)] {
                if idx >= self.length {
                    continue; //bounds check
                }
                let new_area = area.nth_row(idx % rows);
                self.data_columns
                    .draw(grid, idx, self.cursor_pos.2, grid.bounds_iter(new_area));
                if highlight {
                    let selected = self
                        .get_thread_under_cursor(idx)
                        .map(|h| self.rows.is_thread_selected(h))
                        .unwrap_or(false);
                    let row_attr = row_attr!(self.color_cache, unseen: false, highlighted: true, selected: selected);
                    grid.change_theme(new_area, row_attr);
                } else if let Some(row_attr) = self.rows.row_attr_cache.get(&idx) {
                    // Un-highlighted row: bg returns to the pane fill
                    // unless the row is selected.
                    let selected = self
                        .get_thread_under_cursor(idx)
                        .map(|h| self.rows.is_thread_selected(h))
                        .unwrap_or(false);
                    let row_attr = if selected {
                        *row_attr
                    } else {
                        ThemeAttribute {
                            bg: pane_fill.bg,
                            ..*row_attr
                        }
                    };
                    grid.change_theme(new_area, row_attr);
                }
                context.dirty_areas.push_back(new_area);
            }
            if *account_settings!(context[self.cursor_pos.0].listing.relative_list_indices) {
                self.draw_relative_numbers(grid, area, top_idx, pane_fill.bg, context);
                context.dirty_areas.push_back(area);
            }
            if !self.force_draw {
                return;
            }
        } else if self.cursor_pos != self.new_cursor_pos {
            self.cursor_pos = self.new_cursor_pos;
        }
        if self.new_cursor_pos.2 >= self.length {
            self.new_cursor_pos.2 = self.length - 1;
            self.cursor_pos.2 = self.new_cursor_pos.2;
        }

        grid.clear_area(area, pane_fill);
        /* Page_no has changed, so draw new page */
        _ = self.data_columns.recalc_widths(area.size(), top_idx);
        /* copy table columns */
        self.data_columns
            .draw(grid, top_idx, self.cursor_pos.2, grid.bounds_iter(area));
        if *account_settings!(context[self.cursor_pos.0].listing.relative_list_indices) {
            self.draw_relative_numbers(grid, area, top_idx, pane_fill.bg, context);
        }
        /* apply each row colors separately */
        for i in top_idx..(top_idx + area.height()) {
            if let Some(row_attr) = self.rows.row_attr_cache.get(&i) {
                // Base rows (not the cursor row, not selected) sit
                // directly on the pane background: they keep their theme
                // fg/attrs (unseen bold, zebra fg accents) but their bg
                // follows the pane, so an unfocused grid dims as a whole.
                // The cursor row is re-applied with its own highlight
                // below and selected rows keep their own fill.
                let highlighted = i == self.cursor_pos.2;
                let selected = self
                    .get_thread_under_cursor(i)
                    .map(|h| self.rows.is_thread_selected(h))
                    .unwrap_or(false);
                let row_attr = if highlighted || selected {
                    *row_attr
                } else {
                    ThemeAttribute {
                        bg: pane_fill.bg,
                        ..*row_attr
                    }
                };
                grid.change_theme(area.nth_row(i % rows), row_attr);
            }
        }

        /* highlight cursor */
        let selected = self
            .get_thread_under_cursor(self.cursor_pos.2)
            .map(|h| self.rows.is_thread_selected(h))
            .unwrap_or(false);
        let row_attr = row_attr!(
            self.color_cache,
            unseen: false,
            highlighted: true,
            selected: selected
        );
        grid.change_theme(area.nth_row(self.cursor_pos.2 % rows), row_attr);

        /* Clear the gap below the last entry with the pane background:
         * empty rows follow the keyboard focus like every other empty
         * cell of the pane. */
        if top_idx + rows > self.length {
            grid.change_theme(area.skip_rows(self.length - top_idx), pane_fill);
        }

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
        self.rows.row_updates.clear();
        for v in self.selection_mut().values_mut() {
            *v = false;
        }

        let account = &context.accounts[&self.cursor_pos.0];
        let threads = account.collection.get_threads(self.cursor_pos.1);
        for env_hash in results {
            if !account.collection.contains_key(&env_hash) {
                continue;
            }
            let Some(env_thread_node_hash) = threads.envelope_to_thread_node.get(&env_hash) else {
                continue;
            };
            let Some(thread_node) = threads.thread_nodes.get(env_thread_node_hash) else {
                continue;
            };
            let thread = threads.find_group(thread_node.group);
            if self.filtered_order.contains_key(&thread) {
                continue;
            }
            if self.rows.all_threads.contains(&thread) {
                self.filtered_selection.push(thread);
                self.filtered_order
                    .insert(thread, self.filtered_selection.len() - 1);
            }
        }
        if !self.filtered_selection.is_empty() {
            threads.group_inner_sort_by(
                &mut self.filtered_selection,
                self.sort,
                &context.accounts[&self.cursor_pos.0].collection.envelopes,
            );
            self.new_cursor_pos.2 = 0;
        } else {
            _ = self.data_columns.columns[0].resize_with_context(0, 0, context);
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
                /* If self.rows.row_updates is not empty and we exit a thread, the row_update
                 * events will be performed but the list will not be drawn.
                 * So force a draw in any case.
                 */
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

impl std::fmt::Display for CompactListing {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "mail")
    }
}

impl CompactListing {
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
        let color_cache = ColorCache::new(context, IndexStyle::Compact);
        let sort = *mailbox_settings!(context[coordinates.0][&coordinates.1].listing.sort);
        Box::new(Self {
            cursor_pos: (AccountHash::default(), MailboxHash::default(), 0),
            new_cursor_pos: (coordinates.0, coordinates.1, 0),
            length: 0,
            sort,
            subsort: (SortField::Date, SortOrder::Desc),
            search_job: None,
            select_job: None,
            filter_term: String::new(),
            filtered_selection: Vec::new(),
            filtered_order: HashMap::default(),
            focus: Focus::None,
            data_columns: DataColumns::new(color_cache.theme_default),
            rows_drawn: SegmentTree::default(),
            rows: RowsState::default(),
            dirty: true,
            force_draw: true,
            color_cache,
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
        highlight_self: bool,
        hash: ThreadHash,
    ) -> EntryStrings {
        let thread = threads.thread_ref(hash);
        let mut tags = String::new();
        let flags = root_envelope.flags();
        let mut colors: SmallVec<[_; 8]> = SmallVec::new();
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
        EntryStrings {
            date: DateString(self.format_date(context, thread.date())),
            subject: if thread.len() > 1 {
                SubjectString(format!("{} ({})", subject, thread.len()))
            } else {
                SubjectString(subject)
            },
            flag: FlagString::new(
                flags,
                self.selection()
                    .get(&root_envelope.hash())
                    .cloned()
                    .unwrap_or(false),
                thread.snoozed(),
                thread.unseen() > 0,
                // Deterministic envelope-level check: `Thread::has_attachments`
                // is a counter aggregated once at thread insertion and goes
                // stale after refresh/rebuild (see `thread_has_attachments`).
                thread_has_attachments(threads, envelopes, hash),
                context,
                (self.cursor_pos.0, self.cursor_pos.1),
            ),
            from: FromString(Address::display_name_slice(from, None)),
            tags: TagString(tags, colors),
            unseen: thread.unseen() > 0,
            highlight_self,
        }
    }

    fn get_thread_under_cursor(&self, cursor: usize) -> Option<ThreadHash> {
        if self.filter_term.is_empty() {
            self.rows
                .thread_order
                .iter()
                .find(|(_, &r)| r == cursor)
                .map(|(h, _)| h)
                .cloned()
        } else {
            self.filtered_selection.get(cursor).cloned()
        }
    }

    fn update_line(&mut self, context: &Context, env_hash: EnvelopeHash) {
        let account = &context.accounts[&self.cursor_pos.0];

        if !account.contains_key(env_hash) {
            /* The envelope has been renamed or removed, so wait for the appropriate
             * event to arrive */
            return;
        }
        let tags_lck = account.collection.tag_index.read().unwrap();
        // One envelope read guard for the row: `make_entry_string` needs the
        // map for the deterministic attachment check.
        let envelopes = account.collection.envelopes.read().unwrap();
        let Some(envelope) = envelopes.get(&env_hash) else {
            /* The envelope has been renamed or removed, so wait for the appropriate
             * event to arrive */
            log::error!(
                "Could not update compact listing row: envelope {env_hash} is no longer in the \
                 mailbox"
            );
            return;
        };
        let thread_hash = self.rows.env_to_thread[&env_hash];
        let threads = account.collection.get_threads(self.cursor_pos.1);
        let thread = threads.thread_ref(thread_hash);
        let idx = self.rows.thread_order[&thread_hash];
        let row_attr = row_attr!(
            self.color_cache,
            unseen: thread.unseen() > 0,
            highlighted: false,
            selected: self.rows.is_thread_selected(thread_hash)
        );
        self.rows.row_attr_cache.insert(idx, row_attr);

        let mut other_subjects = IndexSet::new();
        let mut tags = IndexSet::new();
        let mut from_address_list = Vec::new();
        let mut from_address_set: std::collections::HashSet<Box<str>> =
            std::collections::HashSet::new();
        let mut highlight_self: bool = false;
        let should_highlight_self = mailbox_settings!(
            context[self.cursor_pos.0][&self.cursor_pos.1]
                .listing
                .highlight_self
        )
        .is_true();
        let my_address: Address = context.accounts[&self.cursor_pos.0]
            .settings
            .account
            .main_identity_address();
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
            highlight_self |= should_highlight_self
                && (envelope.recipient_any(&my_address) || envelope.sender_any(&my_address));
            for addr in envelope.from().iter() {
                if addr.get_email().is_empty() || from_address_set.contains(addr.get_email()) {
                    continue;
                }
                from_address_set.insert(addr.get_email().into());
                from_address_list.push(addr.clone());
            }
        }

        let mut entry_strings = self.make_entry_string(
            envelope,
            context,
            &tags_lck,
            &from_address_list,
            &threads,
            &envelopes,
            &other_subjects,
            &tags,
            highlight_self,
            thread_hash,
        );
        entry_strings.highlight_self = should_highlight_self && {
            let my_address: Address = context.accounts[&self.cursor_pos.0]
                .settings
                .account
                .main_identity_address();
            envelope.recipient_any(&my_address) || envelope.sender_any(&my_address)
        };
        let columns = &mut self.data_columns.columns;
        for n in 0..=4 {
            let area = columns[n].area().nth_row(idx);
            columns[n].grid_mut().clear_area(area, row_attr);
        }
        self.rows_drawn.update(idx, 1);

        *self.rows.entries.get_mut(idx).unwrap() = ((thread_hash, env_hash), entry_strings);
    }

    fn draw_rows(&mut self, context: &Context, start: usize, end: usize) {
        if self.length == 0 {
            return;
        }
        debug_assert!(end >= start);
        if self.rows_drawn.get_max(start, end) == 0 {
            return;
        }
        for i in start..=end {
            self.rows_drawn.update(i, 0);
        }
        let min_width = (
            self.data_columns.columns[0].area().width(),
            self.data_columns.columns[1].area().width(),
            self.data_columns.columns[2].area().width(),
            self.data_columns.columns[3].area().width(),
            self.data_columns.columns[4].area().width(),
        );

        for i in 0..self.data_columns.columns.len() {
            let area = self.data_columns.columns[i]
                .area()
                .skip_rows(start)
                .take_rows(end - start + 1);
            self.data_columns.columns[i]
                .grid_mut()
                .clear_area(area, self.color_cache.theme_default);
        }

        let columns = &mut self.data_columns.columns;
        let mut itoa_buffer = itoa::Buffer::new();
        // Resolved once per draw: `text_format_regexps` returns the
        // formatter list *by value* (an `IndexMap` lookup plus per-entry
        // `ThemeValue` → `FormatTag` resolution, copying up to 64 inline
        // entries), and it used to be re-resolved twice per visible row.
        let from_formatters = crate::conf::text_format_regexps(context, "listing.from");
        let subject_formatters = crate::conf::text_format_regexps(context, "listing.subject");
        for (idx, ((_thread_hash, root_env_hash), strings)) in self
            .rows
            .entries
            .iter()
            .enumerate()
            .skip(start)
            .take(end - start + 1)
        {
            if !context.accounts[&self.cursor_pos.0].contains_key(*root_env_hash) {
                //debug!("key = {}", root_env_hash);
                //debug!(
                //    "name = {} {}",
                //    account[&self.cursor_pos.1].name(),
                //    context.accounts[&self.cursor_pos.0].name()
                //);
                //debug!("{:#?}", context.accounts);

                continue;
            }
            let row_attr = self.rows.row_attr_cache[&idx];
            let (x, _) = {
                let area = columns[0].area().nth_row(idx);
                columns[0].grid_mut().write_string(
                    itoa_buffer.format(idx),
                    row_attr.fg,
                    row_attr.bg,
                    row_attr.attrs,
                    area,
                    None,
                    None,
                )
            };
            for c in {
                let area = columns[0].area();
                columns[0].grid_mut().row_iter(area, x..min_width.0, idx)
            } {
                columns[0].grid_mut()[c]
                    .set_bg(row_attr.bg)
                    .set_attrs(row_attr.attrs);
            }
            let (x, _) = {
                let area = columns[1].area().nth_row(idx);
                columns[1].grid_mut().write_string(
                    &strings.date,
                    row_attr.fg,
                    row_attr.bg,
                    row_attr.attrs,
                    area,
                    None,
                    None,
                )
            };
            for c in {
                let area = columns[1].area();
                columns[1].grid_mut().row_iter(area, x..min_width.1, idx)
            } {
                columns[1].grid_mut()[c]
                    .set_bg(row_attr.bg)
                    .set_attrs(row_attr.attrs);
            }
            let (x, _) = {
                let area = columns[2].area().nth_row(idx);
                columns[2].grid_mut().write_string(
                    &strings.from,
                    row_attr.fg,
                    row_attr.bg,
                    row_attr.attrs,
                    area,
                    None,
                    None,
                )
            };
            for c in {
                let area = columns[2].area();
                columns[2].grid_mut().row_iter(area, x..min_width.2, idx)
            } {
                columns[2].grid_mut()[c]
                    .set_bg(row_attr.bg)
                    .set_attrs(row_attr.attrs)
                    .set_ch(' ');
            }
            {
                for text_formatter in &from_formatters {
                    let t = columns[2].grid_mut().insert_tag(text_formatter.tag);
                    for (start, end) in text_formatter.regexp.find_iter(strings.from.as_str()) {
                        columns[2].grid_mut().set_tag(t, (start, idx), (end, idx));
                    }
                }
            }
            {
                let mut area_col_3 = columns[3].area().nth_row(idx);
                area_col_3 = area_col_3.skip_cols(columns[3].grid_mut().write_string(
                    &strings.flag,
                    row_attr.fg,
                    row_attr.bg,
                    row_attr.attrs,
                    area_col_3,
                    None,
                    None,
                ));
                if strings.highlight_self {
                    let (x, _) = columns[3].grid_mut().write_string(
                        mailbox_settings!(
                            context[self.cursor_pos.0][&self.cursor_pos.1]
                                .listing
                                .highlight_self_flag
                        )
                        .as_ref()
                        .map(|s| s.as_str())
                        .unwrap_or(super::DEFAULT_HIGHLIGHT_SELF_FLAG),
                        self.color_cache.highlight_self.fg,
                        row_attr.bg,
                        row_attr.attrs | Attr::FORCE_TEXT,
                        area_col_3,
                        None,
                        None,
                    );
                    for c in columns[3].grid().row_iter(area_col_3, 0..x, 0) {
                        columns[3].grid_mut()[c].set_keep_fg(true);
                    }
                    area_col_3 = area_col_3.skip_cols(x + 1);
                }
                for c in columns[3].grid().row_iter(area_col_3, 0..min_width.3, 0) {
                    columns[3].grid_mut()[c]
                        .set_bg(row_attr.bg)
                        .set_attrs(row_attr.attrs);
                }
            }
            {
                let mut area_col_4 = columns[4].area().nth_row(idx);
                area_col_4 = area_col_4.skip_cols(columns[4].grid_mut().write_string(
                    &strings.subject,
                    row_attr.fg,
                    row_attr.bg,
                    row_attr.attrs,
                    area_col_4,
                    None,
                    None,
                ));
                {
                    for text_formatter in &subject_formatters {
                        let t = columns[4].grid_mut().insert_tag(text_formatter.tag);
                        for (start, end) in
                            text_formatter.regexp.find_iter(strings.subject.as_str())
                        {
                            columns[4].grid_mut().set_tag(t, (start, idx), (end, idx));
                        }
                    }
                }
                area_col_4 = area_col_4.skip_cols(1);
                for (t, &color) in strings.tags.split_whitespace().zip(strings.tags.1.iter()) {
                    let color = color.unwrap_or(self.color_cache.tag_default.bg);
                    let (x, _) = columns[4].grid_mut().write_string(
                        t,
                        self.color_cache.tag_default.fg,
                        color,
                        self.color_cache.tag_default.attrs,
                        area_col_4,
                        None,
                        None,
                    );
                    for c in columns[4].grid().row_iter(area_col_4, 0..(x + 2), 0) {
                        columns[4].grid_mut()[c]
                            .set_bg(color)
                            .set_keep_fg(true)
                            .set_keep_bg(true)
                            .set_keep_attrs(true);
                    }
                    area_col_4 = area_col_4.skip_cols(x + 2);
                }
                for c in columns[4].grid().row_iter(area_col_4, 0..min_width.4, 0) {
                    columns[4].grid_mut()[c]
                        .set_ch(' ')
                        .set_bg(row_attr.bg)
                        .set_attrs(row_attr.attrs);
                }
            }
        }
        if self.length == 0 && self.filter_term.is_empty() {
            let account = &context.accounts[&self.cursor_pos.0];
            let message: String = account[&self.cursor_pos.1].status();
            if self.data_columns.columns[0].resize_with_context(message.len(), 1, context) {
                let area_col_0 = self.data_columns.columns[0].area();
                self.data_columns.columns[0].grid_mut().write_string(
                    message.as_str(),
                    self.color_cache.theme_default.fg,
                    self.color_cache.theme_default.bg,
                    self.color_cache.theme_default.attrs,
                    area_col_0,
                    None,
                    None,
                );
            }
        }
    }

    fn select(&mut self, search_term: &str, results: Result<SearchResult>, context: &mut Context) {
        match results {
            Ok(result) => {
                super::notify_if_search_degraded(context, &result);
                let account = &context.accounts[&self.cursor_pos.0];
                let threads = account.collection.get_threads(self.cursor_pos.1);
                for env_hash in result.envelopes {
                    if !account.collection.contains_key(&env_hash) {
                        continue;
                    }
                    let Some(env_thread_node_hash) = threads.envelope_to_thread_node.get(&env_hash)
                    else {
                        continue;
                    };
                    let Some(thread_node) = threads.thread_nodes.get(env_thread_node_hash) else {
                        continue;
                    };
                    let thread = threads.find_group(thread_node.group);
                    if self.rows.all_threads.contains(&thread) {
                        self.selection_mut()
                            .entry(env_hash)
                            .and_modify(|entry| *entry = true);
                    }
                }
            }
            Err(err) => {
                self.cursor_pos.2 = 0;
                self.new_cursor_pos.2 = 0;
                let message =
                    format!("Encountered an error while searching for `{search_term}`: {err}.");
                log::error!("{}", message);
                context.replies.push_back(UIEvent::Notification {
                    title: Some("Could not perform search".into()),
                    source: None,
                    body: message.into(),
                    kind: Some(crate::types::NotificationType::Error(err.kind)),
                });
            }
        }
    }

    fn draw_relative_numbers(
        &self,
        grid: &mut CellBuffer,
        area: Area,
        top_idx: usize,
        pane_bg: Color,
        context: &Context,
    ) {
        let width = self.data_columns.widths[0];
        let area = area.take_cols(width);
        let account = &context.accounts[&self.cursor_pos.0];
        let threads = account.collection.get_threads(self.cursor_pos.1);
        // Stack-formatted per row: `to_string()` allocated a `String` for
        // every visible row on every draw.
        let mut itoa_buffer = itoa::Buffer::new();
        for i in 0..area.height() {
            let idx = top_idx + i;
            if idx >= self.length {
                break;
            }
            let row_attr = if let Some(thread_hash) = self.get_thread_under_cursor(idx) {
                let thread = threads.thread_ref(thread_hash);
                let highlighted = self.new_cursor_pos.2 == idx;
                let selected = self.rows.is_thread_selected(thread_hash);
                let row_attr = row_attr!(
                    self.color_cache,
                    unseen: thread.unseen() > 0,
                    highlighted: highlighted,
                    selected: selected
                );
                if highlighted || selected {
                    row_attr
                } else {
                    ThemeAttribute {
                        bg: pane_bg,
                        ..row_attr
                    }
                }
            } else {
                row_attr!(self.color_cache, unseen: false, highlighted: true, selected: false)
            };

            grid.clear_area(area.nth_row(i), row_attr);
            let number: isize = if self.new_cursor_pos.2.saturating_sub(top_idx) == i {
                self.new_cursor_pos.2 as isize
            } else {
                (i as isize - (self.new_cursor_pos.2 - top_idx) as isize).abs()
            };
            grid.write_string(
                itoa_buffer.format(number),
                row_attr.fg,
                row_attr.bg,
                row_attr.attrs,
                area.nth_row(i),
                None,
                None,
            );
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
                        self.new_cursor_pos.2 = self.length - 1;
                    }
                }
                PageMovement::PageDown(multiplier) => {
                    if self.new_cursor_pos.2 + rows * multiplier + 1 < self.length {
                        self.new_cursor_pos.2 += rows * multiplier;
                    } else if self.new_cursor_pos.2 + rows * multiplier > self.length {
                        self.new_cursor_pos.2 = self.length - 1;
                    } else {
                        self.new_cursor_pos.2 = (self.length.saturating_sub(1) / rows) * rows;
                    }
                }
                PageMovement::Right(amount) => {
                    self.data_columns.x_offset += amount;
                    self.data_columns.x_offset = self.data_columns.x_offset.min(
                        self.data_columns
                            .widths
                            .iter()
                            .map(|w| w + 2)
                            .sum::<usize>()
                            .saturating_sub(2),
                    );
                }
                PageMovement::Left(amount) => {
                    self.data_columns.x_offset = self.data_columns.x_offset.saturating_sub(amount);
                }
                PageMovement::Home => {
                    self.new_cursor_pos.2 = 0;
                }
                PageMovement::End => {
                    self.new_cursor_pos.2 = self.length - 1;
                }
            }
        }
    }
}

impl Component for CompactListing {
    fn draw(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        if !self.is_dirty() {
            return;
        }

        if matches!(self.focus, Focus::None) {
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

                grid.clear_area(area.skip(x, y).nth_row(y), self.color_cache.theme_default);
                context.dirty_areas.push_back(area);

                area = area.skip_rows(y + 1);
            }

            let rows = area.height();
            if rows == 0 {
                /* Initialize coordinates/rows via `draw_list`'s refresh
                 * path; its own `rows == 0` guard then stops it before
                 * rendering anything. */
                self.draw_list(grid, area, context);
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
                                ..self.length.min(self.new_cursor_pos.2 + amount + 1)
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
                                    self.length.min(self.new_cursor_pos.2 + amount) + 1
                                        ..self.length,
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
                                ..self
                                    .length
                                    .min(self.new_cursor_pos.2 + rows * multiplier + 1)
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
                                    self.length.min(self.new_cursor_pos.2 + rows * multiplier) + 1
                                        ..self.length,
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
                                for c in (self.new_cursor_pos.2)..self.length {
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
                    self.force_draw |= row >= top_idx && row < top_idx + rows;
                }
                if self.force_draw {
                    /* Draw the entire list */
                    self.draw_list(grid, area, context);
                    self.force_draw = false;
                }
            } else {
                /* Draw the entire list */
                self.draw_list(grid, area, context);
            }
        } else {
            // Split render: the grid keeps the left 30% column, the
            // view the right 70%; the keyboard only toggles the ring
            // highlight.
            // Equal in height to the pane chain's thread list.
            if self.length == 0 {
                if self.dirty {
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
                }
                self.view_area = area.into();
            } else {
                let (list_area, view_area) = crate::mail::pane_split(area);
                let ring = if self.grid_has_keyboard {
                    crate::conf::value(context, "tab.focused")
                } else {
                    crate::conf::value(context, "tab.unfocused")
                };
                let list_inner = draw_rounded_frame(grid, list_area, ring);
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
                grid.clear_area(list_inner, pane_fill);
                self.draw_list(grid, list_inner, context);
                let gap_area = crate::mail::pane_gap(area);
                grid.clear_area(gap_area, self.color_cache.theme_default);
                context.dirty_areas.push_back(gap_area);
                self.view_area = view_area.into();
            }
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
                    if let Some(thread_hash) = self.get_thread_under_cursor(self.new_cursor_pos.2) {
                        self.rows
                            .update_selection_with_thread(thread_hash, |e| *e = !*e);
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
                UIEvent::Action(ref action) => {
                    match action {
                        Action::Sort(field, order) if !self.unfocused() => {
                            let new_order = (*field, *order);
                            if new_order != self.sort {
                                // "Keep it coming"
                                self.sort = (*field, *order);
                                self.refresh_mailbox(context, true);
                                self.set_dirty(true);
                                self.force_draw = true;
                            }
                            return true;
                        }
                        Action::Sort(field, order) if !self.unfocused() => {
                            self.sort = (*field, *order);
                            if !self.filtered_selection.is_empty() {
                                self.set_dirty(true);
                            }
                            self.refresh_mailbox(context, false);
                            return true;
                        }
                        Action::SubSort(field, order) if !self.unfocused() => {
                            self.subsort = (*field, *order);
                            // [ref:FIXME]: perform subsort.
                            return true;
                        }
                        Action::Listing(ToggleThreadSnooze) if !self.unfocused() => {
                            // [ref:FIXME]: Re-implement toggle thread snooze
                            /*
                            let thread = self.get_thread_under_cursor(self.cursor_pos.2);
                            let account = &mut context.accounts[&self.cursor_pos.0];
                            account
                                .collection
                                .threads
                                .write()
                                .unwrap()
                                .entry(self.cursor_pos.1)
                                .and_modify(|threads| {
                                    let is_snoozed = threads.thread_ref(thread).snoozed();
                                    threads.thread_ref_mut(thread).set_snoozed(!is_snoozed);
                                });
                            self.rows.row_updates.push(thread);
                            self.refresh_mailbox(context, false);
                            */
                            return true;
                        }

                        _ => {}
                    }
                }
                _ => {}
            }
        }
        match *event {
            UIEvent::ConfigReload { old_settings: _ } => {
                self.color_cache = ColorCache::new(context, IndexStyle::Compact);
                self.refresh_mailbox(context, true);
                self.set_dirty(true);
                self.force_draw = true;
            }
            UIEvent::MailboxUpdate((ref idxa, ref idxf))
                if (*idxa, *idxf) == (self.new_cursor_pos.0, self.cursor_pos.1) =>
            {
                self.refresh_mailbox(context, false);
                self.set_dirty(true);
                self.force_draw = true;
            }
            UIEvent::StartupCheck(ref f) if *f == self.cursor_pos.1 => {
                self.refresh_mailbox(context, false);
                self.set_dirty(true);
                self.force_draw = true;
            }
            UIEvent::EnvelopeRename(_, ref new_hash) => {
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
                if self.rows.contains_thread(thread) {
                    self.rows.row_update_add_thread(thread);
                }

                self.set_dirty(true);
            }
            UIEvent::EnvelopeRemove(_, ref thread_hash) => {
                if self.rows.thread_order.contains_key(thread_hash) {
                    self.refresh_mailbox(context, false);
                    self.set_dirty(true);
                    self.force_draw = true;
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
                if self.rows.contains_thread(thread) {
                    self.rows.row_update_add_thread(thread);
                }

                self.set_dirty(true);
            }
            UIEvent::ChangeMode(UIMode::Normal) => {
                self.set_dirty(true);
            }
            UIEvent::Resize => {
                self.set_dirty(true);
            }
            UIEvent::Input(Key::Esc) if !self.unfocused() && !self.filter_term.is_empty() => {
                self.set_coordinates((self.new_cursor_pos.0, self.new_cursor_pos.1));
                self.refresh_mailbox(context, false);
                self.set_dirty(true);
                self.force_draw = true;
                return true;
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
            UIEvent::Action(Action::Listing(Select {
                term: ref search_term,
                raw_search,
            })) => {
                match context.accounts[&self.cursor_pos.0].search(
                    search_term,
                    raw_search,
                    self.sort,
                    self.cursor_pos.1,
                ) {
                    Ok(job) => {
                        let mut handle = context.accounts[&self.cursor_pos.0]
                            .main_loop_handler
                            .job_executor
                            .spawn(
                                "select-by-search".into(),
                                job,
                                context.accounts[&self.cursor_pos.0].is_async(),
                            );
                        if let Ok(Some(search_result)) = try_recv_timeout!(&mut handle.chan) {
                            self.select(search_term, search_result, context);
                        } else {
                            self.select_job =
                                Some((search_term.to_string(), self.cursor_pos.1, handle));
                        }
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
            UIEvent::StatusEvent(StatusEvent::JobFinished(ref job_id))
                if self
                    .select_job
                    .as_ref()
                    .map(|(_, _, j)| j == job_id)
                    .unwrap_or(false) =>
            {
                let (search_term, mailbox_hash, mut handle) = self.select_job.take().unwrap();
                match handle.chan.try_recv() {
                    Err(_) => { /* search was canceled */ }
                    Ok(None) => { /* something happened, perhaps a worker thread panicked */ }
                    Ok(Some(results)) => {
                        if self.cursor_pos.1 == mailbox_hash {
                            self.select(&search_term, results, context);
                        } else {
                            // The user switched mailboxes while the scan
                            // was running: selecting stale envelopes
                            // would corrupt the new mailbox's selection.
                            log::debug!(
                                "dropping stale select results for mailbox {mailbox_hash:?}; \
                                 the listing now shows mailbox {:?}",
                                self.cursor_pos.1
                            );
                        }
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
        BackendMailbox, Mailbox, MailboxPermissions, SpecialUsageMailbox,
    };

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

    /// Register an `INBOX` and an `Archive` mailbox on the mock account,
    /// modeled on the `register_two_mailboxes` helper in
    /// `meli/src/golden.rs` and the `listing_menu_tests` precedent.
    fn register_mailboxes(context: &mut Context) -> (AccountHash, MailboxHash, MailboxHash) {
        let account_hash = *context.accounts.iter().next().unwrap().0;
        let inbox_hash = MailboxHash::from_bytes(b"INBOX");
        let archive_hash = MailboxHash::from_bytes(b"Archive");
        let account = context.accounts.get_mut(&account_hash).unwrap();
        for (hash, name) in [(inbox_hash, "INBOX"), (archive_hash, "Archive")] {
            account.mailbox_entries.insert(
                hash,
                MailboxEntry::new(
                    MailboxStatus::Available,
                    name.to_string(),
                    Box::new(TestMailbox {
                        hash,
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
        (account_hash, inbox_hash, archive_hash)
    }

    /// Insert two INBOX mails, build a drawn listing over them, and fire
    /// `search findme` (the job spawns; the filter applies only when its
    /// `JobFinished` is processed). Returns the pieces needed to drive
    /// the job's completion manually.
    fn setup_listing_with_pending_search(
        ctx: &mut Context,
    ) -> (
        Box<CompactListing>,
        AccountHash,
        MailboxHash,
        MailboxHash,
        crate::jobs::JobId,
    ) {
        let mails: [&[u8]; 2] = [
            b"From: a@b.example\r\nTo: c@d.example\r\nSubject: findme alpha\r\nMessage-ID: <findme-alpha@x.example>\r\nDate: Thu, 1 Jan 2026 00:00:00 +0000\r\n\r\nfindme alpha body\r\n",
            b"From: a@b.example\r\nTo: c@d.example\r\nSubject: beta\r\nMessage-ID: <beta@x.example>\r\nDate: Thu, 1 Jan 2026 00:01:00 +0000\r\n\r\nbeta body\r\n",
        ];
        let (account_hash, inbox_hash, archive_hash) = register_mailboxes(ctx);
        for bytes in mails {
            let env = Envelope::from_bytes(bytes, None).unwrap();
            ctx.accounts[&account_hash]
                .collection
                .insert(env, inbox_hash);
        }

        let mut listing =
            CompactListing::new(ComponentId::default(), (account_hash, inbox_hash), ctx);
        let theme_default = crate::conf::value(ctx, "theme_default");
        let mut screen = Screen::<Virtual>::new(theme_default);
        assert!(screen.resize(80, 24));
        let area = screen.area();
        // First draw populates the rows for INBOX.
        listing.draw(screen.grid_mut(), area, ctx);
        assert_eq!(listing.length, 2, "precondition: both mails are listed");

        let mut event = UIEvent::Action(Action::Listing(ListingAction::Search {
            term: "findme".to_string(),
            raw_search: false,
        }));
        assert!(listing.process_event(&mut event, ctx));
        let job_id = listing
            .search_job
            .as_ref()
            .expect("the search must spawn a background job")
            .2
            .job_id;
        (listing, account_hash, inbox_hash, archive_hash, job_id)
    }

    /// Wait until the executor signals the search job's completion
    /// (without delivering it to the listing) so the test controls when
    /// the `JobFinished` event is processed.
    fn wait_for_job_completion(ctx: &Context, job_id: crate::jobs::JobId) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            match ctx.receiver.recv_timeout(std::time::Duration::from_secs(1)) {
                Ok(crate::ThreadEvent::JobFinished(id)) if id == job_id => return,
                Ok(_) => {}
                Err(_) => {}
            }
        }
        panic!("the search job did not finish within five seconds");
    }

    /// Deliver a finished job like the main loop does.
    fn deliver_job_finished(
        listing: &mut CompactListing,
        ctx: &mut Context,
        job_id: crate::jobs::JobId,
    ) {
        ctx.main_loop_handler.job_executor.set_job_finished(job_id);
        let mut ev = UIEvent::StatusEvent(StatusEvent::JobFinished(job_id));
        let _ = listing.process_event(&mut ev, ctx);
    }

    fn draw_grid_text(listing: &mut CompactListing, ctx: &mut Context) -> String {
        listing.set_dirty(true);
        let theme_default = crate::conf::value(ctx, "theme_default");
        let mut screen = Screen::<Virtual>::new(theme_default);
        assert!(screen.resize(80, 24));
        let area = screen.area();
        listing.draw(screen.grid_mut(), area, ctx);
        let grid = screen.grid();
        (0..grid.rows)
            .map(|y| (0..grid.cols).map(|x| grid[(x, y)].ch()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Regression: a search job that completes after the user switched
    /// mailboxes must not apply the originating mailbox's hashes as a
    /// filter on the now-current mailbox. The stale result is dropped:
    /// the filter term stays unset and the old mailbox's rows all stay
    /// visible when the user switches back.
    #[test]
    fn stale_search_job_after_mailbox_switch_does_not_filter() {
        let mut ctx = crate::golden::mock_context();
        let (mut listing, account_hash, inbox_hash, archive_hash, job_id) =
            setup_listing_with_pending_search(&mut ctx);
        wait_for_job_completion(&ctx, job_id);

        // The user switches mailboxes while the scan runs, like the real
        // transition does.
        listing.set_coordinates((account_hash, archive_hash));
        listing.refresh_mailbox(&mut ctx, false);
        assert_eq!(listing.cursor_pos.1, archive_hash);

        deliver_job_finished(&mut listing, &mut ctx, job_id);

        // The stale result was dropped: no filter was applied.
        assert!(
            listing.filter_term.is_empty(),
            "the stale search result must not set the filter term, got {:?}",
            listing.filter_term
        );
        assert!(
            listing.filtered_selection.is_empty(),
            "the stale search result must not feed the filtered selection"
        );

        // Switching back to the searched mailbox must show every row
        // again: no stale filter is in effect.
        listing.set_coordinates((account_hash, inbox_hash));
        listing.refresh_mailbox(&mut ctx, false);
        let text = draw_grid_text(&mut listing, &mut ctx);
        assert!(
            text.contains("findme alpha") && text.contains("beta"),
            "all INBOX rows must remain visible after the stale completion, got:\n{text}"
        );
    }

    /// Positive control for the stale-completion guard: when the listing
    /// still shows the mailbox the search was issued on, the completed
    /// job's filter must apply.
    #[test]
    fn search_job_on_current_mailbox_still_filters() {
        let mut ctx = crate::golden::mock_context();
        let (mut listing, _account_hash, _inbox_hash, _archive_hash, job_id) =
            setup_listing_with_pending_search(&mut ctx);
        wait_for_job_completion(&ctx, job_id);

        deliver_job_finished(&mut listing, &mut ctx, job_id);

        assert_eq!(
            listing.filter_term, "findme",
            "the search filter must be applied on completion"
        );
        assert_eq!(
            listing.filtered_selection.len(),
            1,
            "only the matching thread must be in the filtered selection"
        );
        let text = draw_grid_text(&mut listing, &mut ctx);
        assert!(
            text.contains("findme alpha"),
            "the matching row must be visible, got:\n{text}"
        );
        assert!(
            !text.contains("beta"),
            "the non-matching row must be filtered out, got:\n{text}"
        );
    }
}
