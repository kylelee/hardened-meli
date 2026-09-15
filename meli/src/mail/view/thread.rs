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
    cmp,
    fs::File,
    io::{BufWriter, Write},
};

use futures::future::try_join_all;
use melib::{
    utils::{
        datetime::{timestamp_to_string, UnixTimestamp},
        shellexpand::ShellExpandTrait,
    },
    Address,
};

use super::*;
use crate::{
    components::PageMovement,
    conf::data_types::ThreadLayout,
    terminal::{draw_rounded_frame, frame_ring_areas},
};

#[derive(Debug)]
struct ThreadEntry {
    index: (usize, ThreadNodeHash, usize),
    /// (indentation, `thread_node` index, line number in listing)
    indentation: usize,
    msg_hash: EnvelopeHash,
    seen: bool,
    dirty: bool,
    hidden: bool,
    heading: String,
    timestamp: UnixTimestamp,
    mailview: Box<MailView>,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum ThreadViewFocus {
    #[default]
    None,
    Thread,
    MailView,
}

#[derive(Clone, Copy, Debug)]
enum FocusDirection {
    Left,
    Right,
}

/// Outcome of one pane-chain step: move the focus, stay put but consume the
/// key, or let the key pass through to the parent (listing) component.
enum FocusStep {
    Focus(ThreadViewFocus),
    StayAndConsume,
    PassThrough,
}

impl ThreadViewFocus {
    /// Pane chain step: \[sidebar\] \[grid\] \[thread list\] \[mail detail\].
    /// Right: `Thread`→`None`, `None`→`MailView`, `MailView` stays consumed —
    /// the mail detail state is the terminal stop, so the listing's
    /// `Entry + focus_right → EntryFullscreen` branch must never fire from
    /// arrow keys. Left: `MailView`→`None`; at `None`/`Thread` Left passes
    /// through so the listing component's existing
    /// `Entry + focus_left → set_focus(None)` branch closes the view and
    /// refocuses the grid.
    fn step(self, direction: FocusDirection) -> FocusStep {
        match (self, direction) {
            (Self::MailView, FocusDirection::Left) => FocusStep::Focus(Self::None),
            (Self::None, FocusDirection::Left) => FocusStep::PassThrough,
            (Self::Thread, FocusDirection::Left) => FocusStep::PassThrough,
            (Self::Thread, FocusDirection::Right) => FocusStep::Focus(Self::None),
            (Self::None, FocusDirection::Right) => FocusStep::Focus(Self::MailView),
            (Self::MailView, FocusDirection::Right) => FocusStep::StayAndConsume,
        }
    }
}

#[derive(Debug)]
pub struct ThreadView {
    new_cursor_pos: usize,
    cursor_pos: usize,
    expanded_pos: usize,
    new_expanded_pos: usize,
    reversed: bool,
    coordinates: (AccountHash, MailboxHash, EnvelopeHash),
    thread_group: ThreadHash,
    focus: ThreadViewFocus,
    entries: Vec<ThreadEntry>,
    visible_entries: Vec<Vec<usize>>,
    //indentation_colors: [ThemeAttribute; 6],
    use_color: bool,
    last_width: usize,
    thread_layout: ThreadLayout,
    movement: Option<PageMovement>,
    dirty: bool,
    content: Screen<Virtual>,
    id: ComponentId,
}

impl ThreadView {
    /// @`coordinates`: (account index, `mailbox_hash`, root set `thread_node`
    /// index)
    /// @`expanded_hash`: optional position of expanded entry when we
    ///                   render the `ThreadView`.
    ///                   default: expanded message is the last one.
    pub fn new(
        coordinates: (AccountHash, MailboxHash, EnvelopeHash),
        thread_group: ThreadHash,
        expanded_hash: Option<EnvelopeHash>,
        go_to_first_unread: bool,
        focus: Option<ThreadViewFocus>,
        context: &mut Context,
    ) -> Self {
        let theme_default = crate::conf::value(context, "theme_default");
        let mut view = Self {
            reversed: false,
            coordinates,
            thread_group,
            focus: focus.unwrap_or_default(),
            entries: Vec::new(),
            cursor_pos: 1,
            new_cursor_pos: 0,
            dirty: true,
            id: ComponentId::default(),
            //indentation_colors: [
            //    crate::conf::value(context, "mail.view.thread.indentation.a"),
            //    crate::conf::value(context, "mail.view.thread.indentation.b"),
            //    crate::conf::value(context, "mail.view.thread.indentation.c"),
            //    crate::conf::value(context, "mail.view.thread.indentation.d"),
            //    crate::conf::value(context, "mail.view.thread.indentation.e"),
            //    crate::conf::value(context, "mail.view.thread.indentation.f"),
            //],
            use_color: context.settings.terminal.use_color(),
            last_width: 0,
            thread_layout: *mailbox_settings!(
                context[coordinates.0][&coordinates.1].listing.thread_layout
            ),
            expanded_pos: 0,
            new_expanded_pos: 0,
            visible_entries: vec![],
            movement: None,
            content: Screen::<Virtual>::new(theme_default),
        };
        view.initiate(expanded_hash, go_to_first_unread, context);
        view.new_cursor_pos = view.new_expanded_pos;
        // A single-mail thread has no thread-list pane (draw renders only
        // the mail view); start at the mail detail so paging keys reach the
        // mail content without an extra focus step.
        if view.entries.len() == 1 {
            view.focus = ThreadViewFocus::MailView;
        }
        view
    }

    pub fn update(&mut self, context: &mut Context) {
        if self.entries.is_empty() {
            return;
        }

        let old_entries = std::mem::take(&mut self.entries);

        let old_focused_entry = if self.entries.len() > self.cursor_pos {
            Some(self.entries.remove(self.cursor_pos))
        } else {
            None
        };

        let old_expanded_entry = if self.entries.len() > self.expanded_pos {
            Some(self.entries.remove(self.expanded_pos))
        } else {
            None
        };

        let expanded_hash = old_expanded_entry.as_ref().map(|e| e.msg_hash);
        self.initiate(expanded_hash, false, context);

        let mut old_cursor = 0;
        let mut new_cursor = 0;
        loop {
            if old_cursor >= old_entries.len() || new_cursor >= self.entries.len() {
                break;
            }
            if old_entries[old_cursor].msg_hash == self.entries[new_cursor].msg_hash
                || old_entries[old_cursor].index == self.entries[new_cursor].index
                || old_entries[old_cursor].heading == self.entries[new_cursor].heading
            {
                self.entries[new_cursor].hidden = old_entries[old_cursor].hidden;
                old_cursor += 1;
            }
            new_cursor += 1;
            self.recalc_visible_entries();
        }

        if let Some(old_focused_entry) = old_focused_entry {
            if let Some(new_entry_idx) = self.entries.iter().position(|e| {
                e.msg_hash == old_focused_entry.msg_hash
                    || (e.index.1 == old_focused_entry.index.1
                        && e.index.2 == old_focused_entry.index.2)
            }) {
                self.cursor_pos = new_entry_idx;
            }
        }
        if let Some(old_expanded_entry) = old_expanded_entry {
            if let Some(new_entry_idx) = self.entries.iter().position(|e| {
                e.msg_hash == old_expanded_entry.msg_hash
                    || (e.index.1 == old_expanded_entry.index.1
                        && e.index.2 == old_expanded_entry.index.2)
            }) {
                self.expanded_pos = new_entry_idx;
            }
        }
        self.set_dirty(true);
    }

    fn initiate(
        &mut self,
        expanded_hash: Option<EnvelopeHash>,
        go_to_first_unread: bool,
        context: &mut Context,
    ) {
        #[inline(always)]
        fn make_entry(
            i: (usize, ThreadNodeHash, usize),
            (account_hash, mailbox_hash, msg_hash): (AccountHash, MailboxHash, EnvelopeHash),
            seen: bool,
            initialize_now: bool,
            timestamp: UnixTimestamp,
            context: &mut Context,
        ) -> ThreadEntry {
            let (ind, _, _) = i;
            ThreadEntry {
                index: i,
                indentation: ind,
                mailview: Box::new(MailView::new(
                    Some((account_hash, mailbox_hash, msg_hash)),
                    initialize_now,
                    context,
                )),
                msg_hash,
                seen,
                dirty: true,
                hidden: false,
                heading: String::new(),
                timestamp,
            }
        }

        let collection = context.accounts[&self.coordinates.0].collection.clone();
        let threads = collection.get_threads(self.coordinates.1);

        if !threads.groups.contains_key(&self.thread_group) {
            return;
        }
        let (account_hash, mailbox_hash, _) = self.coordinates;

        // Find out how many entries there are going to be, and prioritize
        // initialization to the open entry and the most recent ones.
        //
        // This helps skip initializing the whole thread at once, which will make the UI
        // loading slower.
        //
        // This won't help at all if the latest entry is a reply to an older entry but
        // oh well.
        let mut total_entries = vec![];
        for (_, thread_node_hash) in threads.thread_iter(self.thread_group) {
            if let Some(msg_hash) = threads.thread_nodes()[&thread_node_hash].message() {
                if Some(msg_hash) == expanded_hash {
                    continue;
                }
                let env_ref = collection.get_env(msg_hash);
                total_entries.push((msg_hash, env_ref.timestamp));
            };
        }
        total_entries.sort_by_key(|e| cmp::Reverse(e.1));
        let tokens = f64::from(u32::try_from(total_entries.len()).unwrap_or(0)) * 0.29;
        let tokens = tokens.ceil() as usize;
        total_entries.truncate(tokens);

        // Now, only the expanded envelope plus the ones that remained in total_entries
        // (around 30% of the total messages in the thread) will be scheduled
        // for loading immediately. The others will be lazily loaded when the
        // user opens them for reading.

        let thread_iter = threads.thread_iter(self.thread_group);
        self.entries.clear();
        let mut earliest_unread = 0;
        let mut earliest_unread_entry = 0;
        for (line, (ind, thread_node_hash)) in thread_iter.enumerate() {
            let entry = if let Some(msg_hash) = threads.thread_nodes()[&thread_node_hash].message()
            {
                let (is_seen, timestamp) = {
                    let env_ref = collection.get_env(msg_hash);
                    if !env_ref.is_seen()
                        && (earliest_unread == 0 || env_ref.timestamp < earliest_unread)
                    {
                        earliest_unread = env_ref.timestamp;
                        earliest_unread_entry = self.entries.len();
                    }
                    (env_ref.is_seen(), env_ref.timestamp)
                };
                let initialize_now = if total_entries.is_empty() {
                    false
                } else {
                    // ExtractIf but it hasn't been stabilized yet.
                    // https://doc.rust-lang.org/std/vec/struct.Vec.html#method.extract_if
                    let mut i = 0;
                    let mut result = false;
                    while i < total_entries.len() {
                        if total_entries[i].0 == msg_hash {
                            total_entries.remove(i);
                            result = true;
                            break;
                        } else {
                            i += 1;
                        }
                    }
                    result
                };
                make_entry(
                    (ind, thread_node_hash, line),
                    (account_hash, mailbox_hash, msg_hash),
                    is_seen,
                    initialize_now || expanded_hash == Some(msg_hash),
                    timestamp,
                    context,
                )
            } else {
                continue;
            };
            match expanded_hash {
                Some(expanded_hash) if expanded_hash == entry.msg_hash => {
                    self.new_expanded_pos = self.entries.len();
                    self.expanded_pos = self.new_expanded_pos + 1;
                }
                _ => {}
            }
            self.entries.push(entry);
        }
        if expanded_hash.is_none() {
            self.new_expanded_pos = self
                .entries
                .iter()
                .enumerate()
                .reduce(|a, b| if a.1.timestamp > b.1.timestamp { a } else { b })
                .map(|el| el.0)
                .unwrap_or(0);
            self.expanded_pos = self.new_expanded_pos + 1;
        }
        if go_to_first_unread && earliest_unread > 0 {
            self.new_expanded_pos = earliest_unread_entry;
            self.expanded_pos = earliest_unread_entry + 1;
        }

        let height = self.entries.len();
        let mut width = 0;

        for e in &mut self.entries {
            let envelope: EnvelopeRef = context.accounts[&self.coordinates.0]
                .collection
                .get_env(e.msg_hash);
            let thread_node = &threads.thread_nodes()[&e.index.1];
            let from = Address::display_name_slice(envelope.from(), None);
            let date = timestamp_to_string(envelope.date(), Some("%Y-%m-%d %H:%M\0"), true);
            e.heading = if thread_node.show_subject() {
                let subject = envelope.subject();
                format!(
                    "{date} {subject:`>indent$} {from}",
                    indent = 2 * e.index.0 + subject.grapheme_width(),
                )
            } else {
                format!(
                    "{date} {from:`>indent$}",
                    indent = 2 * e.index.0 + from.grapheme_width()
                )
            };
            width = width.max(e.heading.grapheme_width() + 1);
        }
        if !self.content.resize_with_context(width, height, context) {
            return;
        }
        let theme_default = crate::conf::value(context, "theme_default");
        let highlight_theme = crate::conf::value(context, "theme_default");
        if self.reversed {
            for (y, e) in self.entries.iter().rev().enumerate() {
                {
                    let area = self
                        .content
                        .area()
                        .skip_rows(y)
                        .take(e.heading.grapheme_width() + 1, height - 1);
                    self.content.grid_mut().write_string(
                        &e.heading,
                        if e.seen {
                            theme_default.fg
                        } else {
                            highlight_theme.fg
                        },
                        if e.seen {
                            theme_default.bg
                        } else {
                            highlight_theme.bg
                        },
                        theme_default.attrs,
                        area,
                        None,
                        None,
                    );
                }
            }
        } else {
            for (y, e) in self.entries.iter().enumerate() {
                {
                    let area = self
                        .content
                        .area()
                        .skip_rows(y)
                        .take(e.heading.grapheme_width(), height - 1);
                    self.content.grid_mut().write_string(
                        &e.heading,
                        if e.seen {
                            theme_default.fg
                        } else {
                            highlight_theme.fg
                        },
                        if e.seen {
                            theme_default.bg
                        } else {
                            highlight_theme.bg
                        },
                        theme_default.attrs,
                        area,
                        None,
                        None,
                    );
                }
            }
        }
        self.visible_entries = vec![(0..self.entries.len()).collect()];
    }

    fn highlight_line(
        &self,
        grid: &mut CellBuffer,
        dest_area: Area,
        src_area: Area,
        idx: usize,
        context: &Context,
    ) {
        if self
            .visible_entries
            .iter()
            .flat_map(|v| v.iter())
            .nth(self.cursor_pos)
            == Some(&idx)
        {
            let mut highlight = crate::conf::value(context, "highlight");
            if self.use_color {
                highlight.attrs |= Attr::REVERSE;
            }

            grid.change_theme(dest_area, highlight);
            return;
        }

        grid.copy_area(self.content.grid(), dest_area, src_area);
    }

    fn draw_list(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        if self.entries.is_empty() {
            context.dirty_areas.push_back(area);
            return;
        }
        let height = self.content.area().height();
        if height == 0 {
            context.dirty_areas.push_back(area);
            return;
        }
        let rows = area.height();
        if rows == 0 {
            context.dirty_areas.push_back(area);
            return;
        }
        if let Some(mvm) = self.movement.take() {
            match mvm {
                PageMovement::Up(amount) => {
                    self.new_cursor_pos = self.new_cursor_pos.saturating_sub(amount);
                }
                PageMovement::PageUp(multiplier) => {
                    self.new_cursor_pos = self.new_cursor_pos.saturating_sub(rows * multiplier);
                }
                PageMovement::Down(amount) => {
                    if self.new_cursor_pos + amount + 1 < height {
                        self.new_cursor_pos += amount;
                    } else if self.new_cursor_pos + amount > height {
                        self.new_cursor_pos = height - 1;
                    } else {
                        self.new_cursor_pos = (height / rows) * rows;
                    }
                }
                PageMovement::PageDown(multiplier) => {
                    if self.new_cursor_pos + rows * multiplier + 1 < height {
                        self.new_cursor_pos += rows * multiplier;
                    } else {
                        self.new_cursor_pos = (height / rows) * rows;
                    }
                }
                PageMovement::Right(_) | PageMovement::Left(_) => {}
                PageMovement::Home => {
                    self.new_cursor_pos = 0;
                }
                PageMovement::End => {
                    self.new_cursor_pos = (height / rows) * rows;
                }
            }
            // A page/home/end movement moved the selection; the mail pane
            // follows it live, same as the scroll arms.
            self.sync_expanded_to_cursor();
        }
        if self.new_cursor_pos >= self.entries.len() {
            self.new_cursor_pos = self.entries.len().saturating_sub(1);
        }
        let prev_page_no = (self.cursor_pos).wrapping_div(rows);
        let page_no = (self.new_cursor_pos).wrapping_div(rows);

        let top_idx = page_no * rows;
        // returns the **line** of an entry in the ThreadView grid.
        let get_entry_area = |idx: usize| self.content.area().skip_rows(idx).take_rows(1);

        if self.dirty || (page_no != prev_page_no) {
            grid.clear_area(area, crate::conf::value(context, "theme_default"));
            let visibles: Vec<&usize> =
                self.visible_entries.iter().flat_map(|v| v.iter()).collect();

            for (visible_entry_counter, v) in visibles.iter().skip(top_idx).take(rows).enumerate() {
                let idx = *v;

                grid.copy_area(
                    self.content.grid(),
                    area.skip_rows(visible_entry_counter).take_rows(1),
                    self.content.area().skip_rows(*idx).take_rows(1),
                );
            }
            // If cursor position has changed, remove the highlight from the previous
            // position and apply it in the new one.
            self.cursor_pos = self.new_cursor_pos;
            if self.cursor_pos + 1 > visibles.len() {
                self.cursor_pos = visibles.len().saturating_sub(1);
            }
            let idx = *visibles[self.cursor_pos];
            let src_area = get_entry_area(idx);
            let dest_area = area.skip_rows(self.cursor_pos - top_idx).take_rows(1);

            self.highlight_line(grid, dest_area, src_area, idx, context);
            if rows < visibles.len() {
                ScrollBar::default().set_show_arrows(true).draw(
                    grid,
                    area.nth_col(area.width().saturating_sub(1)),
                    context,
                    self.cursor_pos,
                    rows,
                    visibles.len(),
                );
            }
            if top_idx + rows > visibles.len() {
                grid.clear_area(
                    area.skip_rows(visibles.len() - top_idx),
                    crate::conf::value(context, "theme_default"),
                );
            }
        } else {
            let old_cursor_pos = self.cursor_pos;
            self.cursor_pos = self.new_cursor_pos;
            // If cursor position has changed, remove the highlight from the previous
            // position and apply it in the new one.
            let visibles: Vec<&usize> =
                self.visible_entries.iter().flat_map(|v| v.iter()).collect();
            for &idx in &[old_cursor_pos, self.cursor_pos] {
                let entry_idx = *visibles[idx];
                let src_area = get_entry_area(entry_idx);
                let dest_area = area.skip_rows(visibles[..idx].len() - top_idx).take_rows(1);

                self.highlight_line(grid, dest_area, src_area, entry_idx, context);
                if rows < visibles.len() {
                    ScrollBar::default().set_show_arrows(true).draw(
                        grid,
                        area.nth_col(area.width().saturating_sub(1)),
                        context,
                        self.cursor_pos,
                        rows,
                        visibles.len(),
                    );
                }
            }
        }
        context.dirty_areas.push_back(area);
    }

    /// Calculate if a `ThreadLayout` value of `Auto` would be vertical.
    fn calculate_auto_thread_layout_is_vertical(&self) -> bool {
        if self.last_width == 0 {
            return true;
        }
        // Sorry, I'm just hardcoding this for now.
        self.content.area().width().min(self.last_width / 2) > 62
    }

    fn thread_root_envelope_hash(&self, context: &Context) -> EnvelopeHash {
        let account = &context.accounts[&self.coordinates.0];
        let threads = account.collection.get_threads(self.coordinates.1);
        let thread_root = threads.thread_iter(self.thread_group).next().unwrap().1;
        let thread_node = &threads.thread_nodes()[&thread_root];
        thread_node.message().unwrap_or_else(|| {
            let mut iter_ptr = thread_node.children()[0];
            while threads.thread_nodes()[&iter_ptr].message().is_none() {
                iter_ptr = threads.thread_nodes()[&iter_ptr].children()[0];
            }
            threads.thread_nodes()[&iter_ptr].message().unwrap()
        })
    }

    fn draw_vert(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        if self.entries.is_empty() {
            return;
        }
        let mid = self.content.area().width().min(area.width() / 2);
        if matches!(self.thread_layout, ThreadLayout::Auto)
            && !self.calculate_auto_thread_layout_is_vertical()
        {
            return self.draw_horz(grid, area, context);
        }

        let theme_default = crate::conf::value(context, "theme_default");
        // First draw the thread subject on the first row
        if self.dirty {
            grid.clear_area(area, theme_default);
            let i = self.thread_root_envelope_hash(context);
            let envelope: EnvelopeRef = context.accounts[&self.coordinates.0].collection.get_env(i);

            let (_, y) = grid.write_string(
                &envelope.subject(),
                theme_default.fg,
                theme_default.bg,
                theme_default.attrs,
                area,
                None,
                Some(0),
            );
            context.dirty_areas.push_back(area);
            grid.clear_area(area.nth_col(mid), theme_default);
            grid.clear_area(area.skip(mid, y + 1), theme_default);
        };
        let area = area.skip_rows(2);
        let (width, height) = self.content.area().size();
        if height == 0 || width == 0 {
            return;
        }

        let tab_focused = crate::conf::value(context, "tab.focused");
        let tab_unfocused = crate::conf::value(context, "tab.unfocused");
        /* Rounded pane frames (visual chrome only): the pane holding the
         * interaction focus is framed with "tab.focused", the other with
         * "tab.unfocused". In the split state the thread list owns the
         * cursor, so it is the focused pane. Each pane's content is drawn
         * inside the frame's inner area (the helper's return value) so the
         * ring owns its own cells: no content column is clipped and no
         * content bleeds onto the ring. */
        match self.focus {
            ThreadViewFocus::None => {
                let list_area = area.take_cols(mid.saturating_sub(1));
                let list_inner = draw_rounded_frame(grid, list_area, tab_focused);
                for frame_area in frame_ring_areas(list_area) {
                    context.dirty_areas.push_back(frame_area);
                }
                self.draw_list(grid, list_inner, context);
                let mail_area = area.skip_cols(mid + 1);
                let mail_inner = draw_rounded_frame(grid, mail_area, tab_unfocused);
                for frame_area in frame_ring_areas(mail_area) {
                    context.dirty_areas.push_back(frame_area);
                }
                self.entries[self.new_expanded_pos]
                    .mailview
                    .draw(grid, mail_inner, context);
            }
            ThreadViewFocus::Thread => {
                grid.clear_area(area.skip_cols(mid + 1), theme_default);
                let inner = draw_rounded_frame(grid, area, tab_focused);
                for frame_area in frame_ring_areas(area) {
                    context.dirty_areas.push_back(frame_area);
                }
                self.draw_list(grid, inner, context);
            }
            ThreadViewFocus::MailView => {
                let inner = draw_rounded_frame(grid, area, tab_focused);
                for frame_area in frame_ring_areas(area) {
                    context.dirty_areas.push_back(frame_area);
                }
                self.entries[self.new_expanded_pos]
                    .mailview
                    .draw(grid, inner, context);
            }
        }
        context.dirty_areas.push_back(area);
    }

    fn draw_horz(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        if self.entries.is_empty() {
            return;
        }

        let mid = self.content.area().width().min(area.height() / 2);

        let theme_default = crate::conf::value(context, "theme_default");
        // First draw the thread subject on the first row
        if self.dirty {
            grid.clear_area(area, theme_default);
            let i = self.thread_root_envelope_hash(context);
            let envelope: EnvelopeRef = context.accounts[&self.coordinates.0].collection.get_env(i);

            grid.write_string(
                &envelope.subject(),
                theme_default.fg,
                theme_default.bg,
                theme_default.attrs,
                area,
                None,
                Some(0),
            );
            context.dirty_areas.push_back(area);
        };

        let area = area.skip_rows(2);
        let (width, height) = self.content.area().size();
        if height == 0 || height == self.cursor_pos || width == 0 {
            return;
        }

        let tab_focused = crate::conf::value(context, "tab.focused");
        let tab_unfocused = crate::conf::value(context, "tab.unfocused");
        /* Rounded pane frames (visual chrome only): same convention as the
         * vertical split — focused pane "tab.focused", other
         * "tab.unfocused"; in the split state the thread list owns the
         * cursor. As in the vertical split, each frame is drawn before its
         * pane's content and the content is placed in the frame's inner
         * area so the ring never clips a content column. */
        match self.focus {
            ThreadViewFocus::None => {
                let list_area = area.take_rows(mid);
                let list_inner = draw_rounded_frame(grid, list_area, tab_focused);
                for frame_area in frame_ring_areas(list_area) {
                    context.dirty_areas.push_back(frame_area);
                }
                self.draw_list(grid, list_inner, context);
                let mail_area = area.skip_rows(mid + 1);
                let mail_inner = draw_rounded_frame(grid, mail_area, tab_unfocused);
                for frame_area in frame_ring_areas(mail_area) {
                    context.dirty_areas.push_back(frame_area);
                }
                self.entries[self.new_expanded_pos]
                    .mailview
                    .draw(grid, mail_inner, context);
            }
            ThreadViewFocus::Thread => {
                self.dirty = true;
                let inner = draw_rounded_frame(grid, area, tab_focused);
                for frame_area in frame_ring_areas(area) {
                    context.dirty_areas.push_back(frame_area);
                }
                self.draw_list(grid, inner, context);
            }
            ThreadViewFocus::MailView => {
                let inner = draw_rounded_frame(grid, area, tab_focused);
                for frame_area in frame_ring_areas(area) {
                    context.dirty_areas.push_back(frame_area);
                }
                self.entries[self.new_expanded_pos]
                    .mailview
                    .draw(grid, inner, context);
            }
        }
        context.dirty_areas.push_back(area);
    }

    fn recalc_visible_entries(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        if self.entries.iter().any(|e| e.dirty) {
            self.visible_entries = self
                .entries
                .iter()
                .enumerate()
                .fold(
                    (vec![Vec::new()], SmallVec::<[_; 8]>::new(), false),
                    |(mut visies, mut stack, is_prev_hidden), (idx, e)| {
                        match (e.hidden, is_prev_hidden) {
                            (true, false) => {
                                visies.last_mut().unwrap().push(idx);
                                stack.push(e.indentation);
                                (visies, stack, e.hidden)
                            }
                            (true, true)
                                if !stack.is_empty() && stack[stack.len() - 1] == e.indentation =>
                            {
                                visies.push(vec![idx]);
                                (visies, stack, e.hidden)
                            }
                            (true, true) => (visies, stack, e.hidden),
                            (false, true)
                                if stack[stack.len() - 1] >= e.indentation
                                    && stack.len() > 1
                                    && stack[stack.len() - 2] >= e.indentation =>
                            {
                                // [ref:FIXME]: pop all until e.indentation
                                visies.push(vec![idx]);
                                stack.pop();
                                (visies, stack, e.hidden)
                            }
                            (false, true) if stack[stack.len() - 1] >= e.indentation => {
                                visies.push(vec![idx]);
                                stack.pop();
                                (visies, stack, e.hidden)
                            }
                            (false, true) => (visies, stack, is_prev_hidden),
                            (false, false) => {
                                visies.last_mut().unwrap().push(idx);
                                (visies, stack, e.hidden)
                            }
                        }
                    },
                )
                .0;
        }
        if self.reversed {
            self.visible_entries.reverse()
        }
    }

    /// Current position in self.entries (not in drawn entries which might
    /// exclude nonvisible ones)
    fn current_pos(&self) -> Option<usize> {
        self.visible_entries
            .iter()
            .flat_map(|v| v.iter())
            .nth(self.new_cursor_pos)
            .copied()
    }

    /// Make the mail pane follow the thread-list selection: expand the entry
    /// under the cursor. Called from the selection-movement paths only
    /// (scroll arms, page movements applied at draw time, `focus_right`,
    /// `open_entry`) — refresh/reorder paths (`update`,
    /// `reverse_thread_order`) anchor the expanded entry deliberately and
    /// must NOT go through here.
    fn sync_expanded_to_cursor(&mut self) {
        if let Some(pos) = self.current_pos() {
            self.new_expanded_pos = pos;
            self.expanded_pos = pos;
        }
    }
}

impl std::fmt::Display for ThreadView {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        if let Some(e) = self.entries.first() {
            e.mailview.fmt(fmt)
        } else {
            write!(fmt, "view thread")
        }
    }
}

impl Component for ThreadView {
    fn draw(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        if self.entries.is_empty() {
            self.set_dirty(false);
        }
        if !self.is_dirty() {
            return;
        }
        self.last_width = area.width();

        // If user has selected another mail to view, change to it
        if self.new_expanded_pos != self.expanded_pos {
            self.expanded_pos = self.new_expanded_pos;
        }

        if self.entries.len() == 1 {
            /* A single-mail thread has no thread-list chrome, but the pane
             * is still a pane: draw the rounded frame ring unconditionally
             * (the mail pane is the only visible, interactive pane, so it
             * takes the focused attribute regardless of `self.focus`,
             * which the `p`/`t` visibility toggles may have left as
             * `None`) and render the mail view inside the frame's inner
             * area. */
            let tab_focused = crate::conf::value(context, "tab.focused");
            let inner = draw_rounded_frame(grid, area, tab_focused);
            for frame_area in frame_ring_areas(area) {
                context.dirty_areas.push_back(frame_area);
            }
            self.entries[self.new_expanded_pos]
                .mailview
                .draw(grid, inner, context);
        } else if matches!(self.thread_layout, ThreadLayout::Horizontal) {
            self.draw_horz(grid, area, context);
        } else {
            self.draw_vert(grid, area, context);
        }
        self.set_dirty(false);
    }

    fn process_event(&mut self, event: &mut UIEvent, context: &mut Context) -> bool {
        if matches!(
            (&event, self.entries.is_empty()),
            (UIEvent::Action(Listing(OpenInNewTab)), false)
        ) {
            // Handle this before self.mailview does
            let mut new_tab = Self::new(
                self.coordinates,
                self.thread_group,
                Some(self.entries[self.expanded_pos].msg_hash),
                false,
                Some(self.focus),
                context,
            );
            new_tab.set_dirty(true);
            context
                .replies
                .push_back(UIEvent::Action(Tab(New(Some(Box::new(new_tab))))));
            return true;
        }

        // Pane chain (Left: [mail detail]→[thread list]→[mail listing]→
        // [sidebar]; Right: the reverse up to [mail detail], the terminal
        // stop). Runs in all focus states, ahead of the embedded mail view.
        // Left pass-through relies on the listing component's existing
        // `Focus::Entry + focus_left → set_focus(Focus::None)` branch to
        // close the view and refocus the grid; Right at the mail-detail
        // state stays consumed so the listing's
        // `Entry + focus_right → EntryFullscreen` branch never fires from
        // arrow keys (the chain has no hide-grid stop).
        let shortcuts = self.shortcuts(context);
        if let UIEvent::Input(ref key) = *event {
            let direction = if shortcut!(key == shortcuts[Shortcuts::THREAD_VIEW]["focus_left"]) {
                Some(FocusDirection::Left)
            } else if shortcut!(key == shortcuts[Shortcuts::THREAD_VIEW]["focus_right"]) {
                Some(FocusDirection::Right)
            } else {
                None
            };
            match direction {
                // A single-mail thread has no conversation stop ("thread
                // view, if any"): pass Left through so the listing exits
                // the view directly instead of stopping at a degenerate
                // empty split.
                Some(FocusDirection::Left)
                    if matches!(self.focus, ThreadViewFocus::MailView)
                        && self.entries.len() <= 1 => {}
                Some(direction) => match self.focus.step(direction) {
                    FocusStep::Focus(new_focus) => {
                        // Right must open the thread-list selection (same
                        // sync as the GENERAL open_entry arm below).
                        if matches!(direction, FocusDirection::Right) {
                            self.sync_expanded_to_cursor();
                        }
                        self.focus = new_focus;
                        self.set_dirty(true);
                        return true;
                    }
                    FocusStep::StayAndConsume => return true,
                    FocusStep::PassThrough => {}
                },
                None => {}
            }
        }

        // Pre-detection for thread-list navigation keys at split focus
        // (None): let the thread list win over the embedded mail view, but
        // yield whenever the expanded mail view has an active modal/subview:
        // dialogs (charset selector, URL confirmation, …) are navigable ONLY
        // via GENERAL shortcuts (Selector has no direction-key branches), so
        // stealing their keys would make them inoperable. The gate only
        // narrows the flipped key set — status-quo routing is restored while
        // a modal is open; it introduces no new flips. If rest were to
        // return false for a thread key (analytically unreachable: every
        // matching arm returns true on guard hit), control simply falls
        // through to the status-quo flow below.
        if let UIEvent::Input(ref key) = *event {
            if matches!(self.focus, ThreadViewFocus::None)
                && self.entries.len() > 1
                && !self.entries[self.new_expanded_pos]
                    .mailview
                    .has_active_modal()
                && self.is_thread_view_input(key, context)
                && self.process_event_rest(event, context)
            {
                return true;
            }
        }

        if matches!(
            self.focus,
            ThreadViewFocus::None | ThreadViewFocus::MailView
        ) && !self.entries.is_empty()
            && self.entries[self.new_expanded_pos]
                .mailview
                .process_event(event, context)
        {
            return true;
        }

        if let UIEvent::Input(ref key) = *event {
            // Vertical scroll keys belong to the mail view while it is focused:
            // once the body reaches its top/bottom edge (headers-walk exhausted,
            // pager declines to scroll), consume the key as a no-op instead of
            // letting it bubble into the thread-list scroll arms (which would
            // switch to the previous/next message). The `is_loaded` gate keeps
            // Init/LoadingBody/Error scrolls bubbling so the thread list stays
            // browsable while the body is still loading.
            if matches!(self.focus, ThreadViewFocus::MailView)
                && !self.entries.is_empty()
                && self.entries[self.new_expanded_pos].mailview.is_loaded()
                && (shortcut!(key == shortcuts[Shortcuts::THREAD_VIEW]["scroll_up"])
                    || shortcut!(key == shortcuts[Shortcuts::THREAD_VIEW]["scroll_down"]))
            {
                return true;
            }
        }

        self.process_event_rest(event, context)
    }

    fn is_dirty(&self) -> bool {
        self.dirty
            || (!matches!(self.focus, ThreadViewFocus::Thread)
                && !self.entries.is_empty()
                && self.entries[self.new_expanded_pos].mailview.is_dirty())
    }

    fn set_dirty(&mut self, value: bool) {
        self.dirty = value;
        if let Some(entry) = self.entries.get_mut(self.new_expanded_pos) {
            entry.mailview.set_dirty(value);
        }
    }

    fn shortcuts(&self, context: &Context) -> ShortcutMaps {
        let mut map = if !self.entries.is_empty() {
            self.entries[self.new_expanded_pos]
                .mailview
                .shortcuts(context)
        } else {
            ShortcutMaps::default()
        };

        map.insert(
            Shortcuts::GENERAL,
            mailbox_settings!(
                context[self.coordinates.0][&self.coordinates.1]
                    .shortcuts
                    .general
            )
            .key_values(),
        );
        let mut thread_view_map = mailbox_settings!(
            context[self.coordinates.0][&self.coordinates.1]
                .shortcuts
                .thread_view
        )
        .key_values();
        let (account_hash, mailbox_hash, _) = self.coordinates;
        if mailbox_settings!(context has [account_hash][&mailbox_hash]) {
            for command in mailbox_settings!(
                context[account_hash][&mailbox_hash]
                    .shortcuts
                    .thread_view
                    .commands
            ) {
                thread_view_map.retain(|_, shortcut| shortcut != &command.shortcut);
            }
        }
        map.insert(Shortcuts::THREAD_VIEW, thread_view_map);

        map
    }

    fn id(&self) -> ComponentId {
        self.id
    }

    fn kill(&mut self, id: ComponentId, context: &mut Context) {
        debug_assert!(self.id == id);
        context
            .replies
            .push_back(UIEvent::Action(Tab(Kill(self.id))));
    }
}

impl ThreadView {
    /// The big `match *event` tail of [`ThreadView::process_event`], moved
    /// verbatim so the focus-None pre-detection can run it ahead of the
    /// embedded mail view. Line-for-line equivalent to the former inline
    /// match: branch logic, guards and return values are untouched, and the
    /// trailing `_ =>` arm still forwards unmatched events to ALL entries'
    /// mail views.
    fn process_event_rest(&mut self, event: &mut UIEvent, context: &mut Context) -> bool {
        let shortcuts = self.shortcuts(context);
        let (account_hash, mailbox_hash, _) = self.coordinates;
        match *event {
            UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::THREAD_VIEW]["toggle_layout"]) =>
            {
                if self.entries.len() > 1 {
                    match self.thread_layout {
                        ThreadLayout::Auto if self.calculate_auto_thread_layout_is_vertical() => {
                            self.thread_layout = ThreadLayout::Horizontal;
                        }
                        ThreadLayout::Auto => {
                            self.thread_layout = ThreadLayout::Vertical;
                        }
                        ThreadLayout::Horizontal => {
                            self.thread_layout = ThreadLayout::Auto;
                        }
                        ThreadLayout::Vertical => {
                            self.thread_layout = ThreadLayout::Horizontal;
                        }
                    }
                    context
                        .replies
                        .push_back(UIEvent::StatusEvent(StatusEvent::UpdateSubStatus(format!(
                            "thread_layout set to {}",
                            toml::Value::try_from(self.thread_layout).expect("Cannot fail")
                        ))));
                    self.set_dirty(true);
                }
                true
            }
            UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::THREAD_VIEW]["scroll_up"]) =>
            {
                if self.cursor_pos > 0 {
                    self.new_cursor_pos = self.new_cursor_pos.saturating_sub(1);
                    self.sync_expanded_to_cursor();
                    self.set_dirty(true);
                }
                true
            }
            UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::THREAD_VIEW]["scroll_down"]) =>
            {
                let height = self.visible_entries.iter().flat_map(|v| v.iter()).count();
                if height > 0 && self.new_cursor_pos + 1 < height {
                    self.new_cursor_pos += 1;
                    self.sync_expanded_to_cursor();
                    self.set_dirty(true);
                }
                true
            }
            UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::THREAD_VIEW]["prev_page"]) =>
            {
                self.movement = Some(PageMovement::PageUp(1));
                self.set_dirty(true);
                true
            }
            UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::THREAD_VIEW]["next_page"]) =>
            {
                self.movement = Some(PageMovement::PageDown(1));
                self.set_dirty(true);
                true
            }
            UIEvent::Input(ref k) if shortcut!(k == shortcuts[Shortcuts::GENERAL]["home_page"]) => {
                self.movement = Some(PageMovement::Home);
                self.set_dirty(true);
                true
            }
            UIEvent::Input(ref k) if shortcut!(k == shortcuts[Shortcuts::GENERAL]["end_page"]) => {
                self.movement = Some(PageMovement::End);
                self.set_dirty(true);
                true
            }
            UIEvent::Input(ref k)
                if shortcut!(k == shortcuts[Shortcuts::GENERAL]["open_entry"]) =>
            {
                if self.entries.len() > 1 && self.current_pos().is_some() {
                    self.sync_expanded_to_cursor();
                    if matches!(self.focus, ThreadViewFocus::Thread) {
                        self.focus = ThreadViewFocus::None;
                    }
                    self.set_dirty(true);
                }
                true
            }
            UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::THREAD_VIEW]["toggle_mailview"]) =>
            {
                self.focus = match self.focus {
                    ThreadViewFocus::None | ThreadViewFocus::MailView => ThreadViewFocus::Thread,
                    ThreadViewFocus::Thread => ThreadViewFocus::None,
                };
                self.set_dirty(true);
                true
            }
            UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::THREAD_VIEW]["toggle_threadview"]) =>
            {
                self.focus = match self.focus {
                    ThreadViewFocus::None | ThreadViewFocus::Thread => ThreadViewFocus::MailView,
                    ThreadViewFocus::MailView => ThreadViewFocus::None,
                };
                self.set_dirty(true);
                true
            }
            UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::THREAD_VIEW]["reverse_thread_order"]) =>
            {
                if !self.entries.is_empty() {
                    self.reversed = !self.reversed;
                    let expanded_hash = self.entries[self.expanded_pos].msg_hash;
                    self.initiate(Some(expanded_hash), false, context);
                    self.set_dirty(true);
                }
                true
            }
            UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::THREAD_VIEW]["collapse_subtree"]) =>
            {
                if self.entries.is_empty() {
                    return true;
                }
                if let Some(current_pos) = self.current_pos() {
                    self.entries[current_pos].hidden = !self.entries[current_pos].hidden;
                    self.entries[current_pos].dirty = true;
                    {
                        let visible_entries: Vec<&usize> =
                            self.visible_entries.iter().flat_map(|v| v.iter()).collect();
                        // search_old_cursor_pos
                        self.new_cursor_pos =
                            (|entries: Vec<&usize>, x: usize| {
                                let mut low = 0;
                                let mut high = entries.len() - 1;
                                while low <= high {
                                    let mid = low + (high - low) / 2;
                                    if *entries[mid] == x {
                                        return mid;
                                    }
                                    if x > *entries[mid] {
                                        low = mid + 1;
                                    } else {
                                        high = mid - 1;
                                    }
                                }
                                high + 1 //mid
                            })(visible_entries, self.cursor_pos);
                    }
                    self.cursor_pos = self.new_cursor_pos;
                    self.recalc_visible_entries();
                    self.set_dirty(true);
                }
                true
            }
            UIEvent::Resize | UIEvent::VisibilityChange(true) => {
                self.set_dirty(true);
                false
            }
            UIEvent::EnvelopeRename(ref old_hash, ref new_hash) => {
                let account = &context.accounts[&self.coordinates.0];
                for e in self.entries.iter_mut() {
                    if e.msg_hash == *old_hash {
                        e.msg_hash = *new_hash;
                        let seen: bool = account.collection.get_env(*new_hash).is_seen();
                        e.dirty = e.seen != seen;
                        e.seen = seen;
                        e.mailview.process_event(
                            &mut UIEvent::EnvelopeRename(*old_hash, *new_hash),
                            context,
                        );
                        self.set_dirty(true);
                        break;
                    }
                }
                false
            }
            UIEvent::EnvelopeUpdate(ref env_hash) => {
                let account = &context.accounts[&self.coordinates.0];
                for e in self.entries.iter_mut() {
                    if e.msg_hash == *env_hash {
                        let seen: bool = account.collection.get_env(*env_hash).is_seen();
                        e.dirty = e.seen != seen;
                        e.seen = seen;
                        e.mailview
                            .process_event(&mut UIEvent::EnvelopeUpdate(*env_hash), context);
                        self.set_dirty(true);
                        break;
                    }
                }
                false
            }
            UIEvent::Input(ref key)
                if mailbox_settings!(context has [account_hash][&mailbox_hash])
                    && mailbox_settings!(
                        context[account_hash][&mailbox_hash]
                            .shortcuts
                            .thread_view
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
                true
            }
            UIEvent::Action(View(ViewAction::ExportThread(ref path))) => {
                // Save entire thread as eml files in a directory path
                let mut path = std::path::Path::new(path).to_path_buf().expand();
                if path.is_relative() {
                    path = context.current_dir().join(&path);
                }
                if !path.try_exists().unwrap_or(false) || !path.is_dir() {
                    context.replies.push_back(UIEvent::Notification {
                        title: Some("Thread export".into()),
                        source: None,
                        body: format!(
                            "Path {path} either does not exist, or is not a directory.",
                            path = path.display()
                        )
                        .into(),
                        kind: Some(NotificationType::Info),
                    });
                    return true;
                }

                let mut results = vec![];
                for entry in &self.entries {
                    match entry.mailview.state {
                        MailViewState::Init { .. } | MailViewState::LoadingBody { .. } => {
                            context.replies.push_back(UIEvent::Notification {
                                title: Some("Thread export".into()),
                                source: None,
                                body: "Thread is still loading".into(),
                                kind: Some(NotificationType::Info),
                            });
                            return true;
                        }
                        MailViewState::Error { .. } => {
                            context.replies.push_back(UIEvent::Notification {
                                title: Some("Thread export".into()),
                                source: None,
                                body: "Thread messages could not be loaded because of errors"
                                    .into(),
                                kind: Some(NotificationType::Info),
                            });
                            return true;
                        }
                        MailViewState::Loaded {
                            ref bytes,
                            ref env,
                            env_view: _,
                            stack: _,
                        } => {
                            path.push(format!("{}.eml", env.message_id()));
                            if let Err(err) = save_attachment(&path, bytes) {
                                log::error!("Failed to create file at {}: {err}", path.display());
                                context.replies.push_back(UIEvent::Notification {
                                    title: Some(
                                        format!("Failed to create file at {}", path.display())
                                            .into(),
                                    ),
                                    body: err.to_string().into(),
                                    source: Some(err),
                                    kind: Some(NotificationType::Error(melib::ErrorKind::External)),
                                });
                                results.push(false);
                            } else {
                                results.push(true);
                            }
                            path.pop();
                        }
                    }
                }

                let failures = results.iter().filter(|b| **b).count();
                let body = if failures == results.len() {
                    "Could not export thread, check error logs.".into()
                } else if failures > 0 {
                    format!(
                        "Saved at {path}: {failures}/{total} could not be saved, check error logs.",
                        path = path.display(),
                        total = results.len()
                    )
                    .into()
                } else {
                    format!(
                        "Saved {total} mail{s} at {path}",
                        path = path.display(),
                        total = results.len(),
                        s = if results.len() == 1 { "" } else { "s" }
                    )
                    .into()
                };

                context.replies.push_back(UIEvent::Notification {
                    title: Some("Thread export".into()),
                    source: None,
                    body,
                    kind: Some(NotificationType::Info),
                });

                true
            }
            UIEvent::Action(View(ViewAction::ExportThreadMbox(ref format, ref path))) => {
                // Save entire thread as eml files in a directory path
                let mut path = std::path::Path::new(path).to_path_buf().expand();
                if path.is_relative() {
                    path = context.current_dir().join(&path);
                }
                let format = (*format).unwrap_or_default();
                let (account_hash, _, _) = self.coordinates;
                let account = &mut context.accounts[&account_hash];
                let collection = account.collection.clone();
                let envs_to_set = self
                    .entries
                    .iter()
                    .map(|e| e.msg_hash)
                    .collect::<Vec<EnvelopeHash>>();

                let futures: Result<Vec<_>> = envs_to_set
                    .iter()
                    .map(|&env_hash| account.envelope_bytes_by_hash(env_hash))
                    .collect::<Result<Vec<_>>>();
                let (sender, mut receiver) = crate::jobs::oneshot::channel();
                let fut = Box::pin(async move {
                    let cl = async move {
                        // fully capture variables.
                        let _ = (&envs_to_set, &collection);
                        let bytes: Vec<Vec<u8>> = try_join_all(futures?).await?;
                        let envs: Vec<_> = envs_to_set
                            .iter()
                            .map(|&env_hash| collection.get_env(env_hash))
                            .collect();
                        if path.is_dir() {
                            let mut filename = if envs.len() == 1 {
                                format!("{}.mbox", envs[0].message_id()).into()
                            } else {
                                let now = melib::utils::datetime::timestamp_to_string(
                                    melib::utils::datetime::now(),
                                    Some(melib::utils::datetime::formats::RFC3339_DATETIME),
                                    false,
                                );
                                format!(
                                    "{}-{}-{}_envelopes.mbox",
                                    now,
                                    envs[0].message_id(),
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
                                .open(&path)
                                .chain_err_related_path(&path)?,
                        );
                        let mut iter = envs.iter().zip(bytes);
                        let tags_lck = collection.tag_index.read().unwrap();
                        if let Some((env, ref bytes)) = iter.next() {
                            let tags: Vec<&str> = env
                                .tags()
                                .iter()
                                .filter_map(|h| tags_lck.get(h).map(|s| s.as_str()))
                                .collect();
                            format
                                .append(
                                    &mut file,
                                    bytes.as_slice(),
                                    env.from().first(),
                                    Some(env.date()),
                                    (env.flags(), tags),
                                    melib::mbox::MboxMetadata::CClient,
                                    true,
                                    false,
                                )
                                .chain_err_related_path(&path)?;
                        }
                        for (env, bytes) in iter {
                            let tags: Vec<&str> = env
                                .tags()
                                .iter()
                                .filter_map(|h| tags_lck.get(h).map(|s| s.as_str()))
                                .collect();
                            format
                                .append(
                                    &mut file,
                                    bytes.as_slice(),
                                    env.from().first(),
                                    Some(env.date()),
                                    (env.flags(), tags),
                                    melib::mbox::MboxMetadata::CClient,
                                    false,
                                    false,
                                )
                                .chain_err_related_path(&path)?;
                        }
                        file.flush().chain_err_related_path(&path)?;
                        Ok(path)
                    };
                    let r: Result<PathBuf> = cl.await;
                    let _ = sender.send(r);
                    Ok(())
                });
                let handle = account.main_loop_handler.job_executor.spawn(
                    "exporting-thread-mbox".into(),
                    fut,
                    crate::jobs::IsAsync::Blocking,
                );
                account.insert_job(
                    handle.job_id,
                    JobRequest::Generic {
                        name: "exporting mbox".into(),
                        handle,
                        on_finish: Some(CallbackFn(Box::new(move |context: &mut Context| {
                            context.replies.push_back(match receiver.try_recv() {
                                Err(_) | Ok(None) => UIEvent::Notification {
                                    title: Some("Thread mbox export".into()),
                                    source: None,
                                    body: "Could not export thread as mbox: Job was canceled."
                                        .into(),
                                    kind: Some(NotificationType::Info),
                                },
                                Ok(Some(Err(err))) => UIEvent::Notification {
                                    title: Some("Thread mbox export".into()),
                                    source: None,
                                    body: err.to_string().into(),
                                    kind: Some(NotificationType::Error(err.kind)),
                                },
                                Ok(Some(Ok(path))) => UIEvent::Notification {
                                    title: Some("Thread mbox export".into()),
                                    source: None,
                                    body: format!("Wrote to file {}", path.display()).into(),
                                    kind: Some(NotificationType::Info),
                                },
                            });
                        }))),
                        log_level: LogLevel::INFO,
                    },
                );
                true
            }
            _ => {
                // [ref:VERIFY]: In what case do we need to forward a handled UIEvent to all
                // entries?
                if self
                    .entries
                    .iter_mut()
                    .any(|entry| entry.mailview.process_event(event, context))
                {
                    return true;
                }
                false
            }
        }
    }

    /// Pure detection of thread-view navigation inputs; must stay in sync
    /// with the Input arm guards of `process_event_rest`: every key that can
    /// match an arm there must return `true` here. Judges against the same
    /// parsed `ShortcutMaps` that `process_event_rest` matches on (commands
    /// conflict keys are retain-removed during assembly), NOT a re-derivation
    /// from `mailbox_settings!`, so the two can never drift. Pushes nothing —
    /// the commands side effect happens exactly once, inside rest's arm
    /// guard.
    ///
    /// The `focus_left`/`focus_right` entries are EXCLUDED (by name): focus
    /// keys are handled exclusively by the pre-arms; letting them into the
    /// gated pre-detection would run `process_event_rest`'s `_ =>`
    /// all-entries mailview forwarding on them (double delivery, and a
    /// potential pager steal under GENERAL `scroll_left`/`scroll_right` arrow
    /// rebinds).
    fn is_thread_view_input(&self, key: &Key, context: &Context) -> bool {
        let shortcuts = self.shortcuts(context);
        if shortcuts
            .get(Shortcuts::THREAD_VIEW)
            .map(|section| {
                section
                    .iter()
                    .any(|(name, k)| !matches!(name, &"focus_left" | &"focus_right") && k == key)
            })
            .unwrap_or(false)
        {
            return true;
        }
        if shortcuts
            .get(Shortcuts::GENERAL)
            .map(|section| {
                ["home_page", "end_page", "open_entry"]
                    .iter()
                    .any(|name| section.get(name).map(|k| k == key).unwrap_or(false))
            })
            .unwrap_or(false)
        {
            return true;
        }
        let (account_hash, mailbox_hash, _) = self.coordinates;
        mailbox_settings!(context has [account_hash][&mailbox_hash])
            && mailbox_settings!(
                context[account_hash][&mailbox_hash]
                    .shortcuts
                    .thread_view
                    .commands
            )
            .iter()
            .any(|cmd| cmd.shortcut == *key)
    }
}

#[cfg(test)]
mod focus_tests {
    use std::sync::OnceLock;

    use melib::{
        backends::{BackendMailbox, Mailbox, MailboxHash, MailboxPermissions, SpecialUsageMailbox},
        Envelope, Mail, Result,
    };

    use super::*;
    use crate::{
        accounts::{MailboxEntry, MailboxStatus},
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

        fn path(&self) -> &str {
            "INBOX"
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

    /// Register the mock account's `INBOX` mailbox and return the account and
    /// mailbox hashes.
    ///
    /// `ThreadView::new`/`shortcuts` go through `mailbox_settings!`, which
    /// indexes `mailbox_entries` directly and panics on a missing entry, so a
    /// `MailboxEntry` must be registered before constructing the view.
    fn register_inbox(context: &mut Context) -> (AccountHash, MailboxHash) {
        let account_hash = *context.accounts.iter().next().unwrap().0;
        let mailbox_hash = MailboxHash::from_bytes(b"INBOX");
        context.accounts[&account_hash].mailbox_entries.insert(
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
        (account_hash, mailbox_hash)
    }

    /// Build a `ThreadView` over a two-message thread (a root and a reply
    /// carrying `In-Reply-To`).
    fn make_two_mail_thread_view(context: &mut Context, focus: ThreadViewFocus) -> ThreadView {
        let (account_hash, mailbox_hash) = register_inbox(context);

        let root_envelope = Envelope::from_bytes(ROOT_MAIL_BYTES, None)
            .expect("could not parse root test envelope");
        let reply_envelope = Envelope::from_bytes(REPLY_MAIL_BYTES, None)
            .expect("could not parse reply test envelope");
        let root_hash = root_envelope.hash();
        context.accounts[&account_hash]
            .collection
            .insert(root_envelope, mailbox_hash);
        context.accounts[&account_hash]
            .collection
            .insert(reply_envelope, mailbox_hash);

        let thread_group = {
            let threads = context.accounts[&account_hash]
                .collection
                .get_threads(mailbox_hash);
            threads.find_group(threads.envelope_to_thread[&root_hash])
        };

        ThreadView::new(
            (account_hash, mailbox_hash, root_hash),
            thread_group,
            None,
            false,
            Some(focus),
            context,
        )
    }

    /// Exhaustive check of the pane-chain step table: Left: MailView→None,
    /// None/Thread→PassThrough; Right: Thread→None, None→MailView,
    /// MailView→StayAndConsume.
    #[test]
    fn thread_view_focus_step_transitions_exhaustive() {
        assert!(matches!(
            ThreadViewFocus::MailView.step(FocusDirection::Left),
            FocusStep::Focus(ThreadViewFocus::None)
        ));
        assert!(matches!(
            ThreadViewFocus::None.step(FocusDirection::Left),
            FocusStep::PassThrough
        ));
        assert!(matches!(
            ThreadViewFocus::Thread.step(FocusDirection::Left),
            FocusStep::PassThrough
        ));
        assert!(matches!(
            ThreadViewFocus::Thread.step(FocusDirection::Right),
            FocusStep::Focus(ThreadViewFocus::None)
        ));
        assert!(matches!(
            ThreadViewFocus::None.step(FocusDirection::Right),
            FocusStep::Focus(ThreadViewFocus::MailView)
        ));
        assert!(matches!(
            ThreadViewFocus::MailView.step(FocusDirection::Right),
            FocusStep::StayAndConsume
        ));
    }

    /// Build a `ThreadView` over a single-mail thread (only the root mail);
    /// the reply is simply not inserted.
    fn make_single_mail_thread_view(context: &mut Context, focus: ThreadViewFocus) -> ThreadView {
        let (account_hash, mailbox_hash) = register_inbox(context);
        let root_envelope = Envelope::from_bytes(ROOT_MAIL_BYTES, None)
            .expect("could not parse root test envelope");
        let root_hash = root_envelope.hash();
        context.accounts[&account_hash]
            .collection
            .insert(root_envelope, mailbox_hash);
        let thread_group = {
            let threads = context.accounts[&account_hash]
                .collection
                .get_threads(mailbox_hash);
            threads.find_group(threads.envelope_to_thread[&root_hash])
        };
        ThreadView::new(
            (account_hash, mailbox_hash, root_hash),
            thread_group,
            None,
            false,
            Some(focus),
            context,
        )
    }

    /// The single-mail fast path (`entries.len() == 1`) draws the mailview
    /// over the whole area with no thread-list chrome; it must still be a
    /// framed pane like every other: a rounded ring at the area edge (row 0 —
    /// the fast path has no two-row thread-subject header), the inner area
    /// left to the mail view, and the ring painted with the `tab.focused`
    /// attribute — the mail pane is the only visible, interactive pane in
    /// this state.
    #[test]
    fn thread_view_single_mail_frame_ring_inner_and_focus_attr() {
        let mut ctx = mock_context();
        let mut view = make_single_mail_thread_view(&mut ctx, ThreadViewFocus::MailView);
        view.set_dirty(true);

        let theme_default = crate::conf::value(&ctx, "theme_default");
        let mut screen = crate::terminal::Screen::<crate::terminal::Virtual>::new(theme_default);
        assert!(screen.resize(80, 24));
        let area = screen.area();
        view.draw(screen.grid_mut(), area, &mut ctx);

        let grid = screen.grid();
        let last_col = area.width() - 1;
        let last_row = area.height() - 1;
        assert_eq!(grid[(0, 0)].ch(), '╭', "top-left rounded corner");
        assert_eq!(grid[(last_col, 0)].ch(), '╮', "top-right rounded corner");
        assert_eq!(grid[(0, last_row)].ch(), '╰', "bottom-left rounded corner");
        assert_eq!(
            grid[(last_col, last_row)].ch(),
            '╯',
            "bottom-right rounded corner"
        );

        // Ring attribute: the focused-pane convention (`tab.focused`), same
        // as the split layouts' mail-focused state.
        let tab_focused = crate::conf::value(&ctx, "tab.focused");
        assert_eq!(grid[(0, 0)].fg(), tab_focused.fg);

        // Ring owns exactly its edge cells (bars on the edge rows/columns),
        // with no glyph bleeding into the inner area the mail view cleared.
        assert_eq!(grid[(5, 0)].ch(), '─', "top edge horizontal bar");
        assert_eq!(grid[(0, 5)].ch(), '│', "left edge vertical bar");
        assert_eq!(grid[(5, last_row)].ch(), '─', "bottom edge horizontal bar");
        assert_eq!(grid[(last_col, 5)].ch(), '│', "right edge vertical bar");
        assert_eq!(
            grid[(2, 2)].ch(),
            ' ',
            "no ring glyph may bleed into the inner area"
        );

        // Content clip guard (the b1 lesson): drive the expanded entry's
        // mail view to the `Loaded` state through the public
        // `MailViewState::load_bytes` (same helper as the sibling focus
        // tests) and redraw — the From value must render contiguous inside
        // the ring's inner area, starting at its left edge (x == 1), never
        // under or over the ring columns.
        load_expanded_entry(&mut view, &mut ctx, ROOT_MAIL_BYTES);
        view.set_dirty(true);
        view.draw(screen.grid_mut(), area, &mut ctx);
        let grid = screen.grid();
        let row_string = |y: usize| -> String {
            (0..area.width())
                .map(|x| grid[(x, y)].ch())
                .collect::<String>()
        };
        let header_y = (0..area.height())
            .find(|&y| row_string(y).contains("a@b.example"))
            .expect("From header value must render");
        let header_row = row_string(header_y);
        let header_x = header_row
            .find("a@b.example")
            .expect("substring presence checked above");
        assert!(
            header_x >= 1,
            "content must start inside the ring, not on the ring column"
        );
        assert_eq!(
            header_row.chars().next(),
            Some('│'),
            "the ring column must stay intact on a content row"
        );
        // The body line renders flush under the header block, inside the
        // ring: `│root` (the Message-ID header row also contains "root",
        // so anchor the match at the ring column + line start).
        assert!(
            (0..area.height())
                .map(row_string)
                .any(|s| s.trim_start().starts_with("│root")),
            "mail body must render inside the ring"
        );
    }

    /// The single-mail fast path draws its frame unconditionally: the `p`/`t`
    /// visibility toggles can leave `focus` at `None` (the toggle branches
    /// have no single-mail guard), and the frame must not depend on that
    /// state.
    #[test]
    fn thread_view_single_mail_frame_at_focus_none() {
        let mut ctx = mock_context();
        let mut view = make_single_mail_thread_view(&mut ctx, ThreadViewFocus::None);
        view.set_dirty(true);

        let theme_default = crate::conf::value(&ctx, "theme_default");
        let mut screen = crate::terminal::Screen::<crate::terminal::Virtual>::new(theme_default);
        assert!(screen.resize(80, 24));
        let area = screen.area();
        view.draw(screen.grid_mut(), area, &mut ctx);

        let grid = screen.grid();
        let last_col = area.width() - 1;
        let last_row = area.height() - 1;
        assert_eq!(grid[(0, 0)].ch(), '╭', "top-left rounded corner");
        assert_eq!(grid[(last_col, last_row)].ch(), '╯', "bottom-right corner");
        let tab_focused = crate::conf::value(&ctx, "tab.focused");
        assert_eq!(
            grid[(0, 0)].fg(),
            tab_focused.fg,
            "frame is unconditionally focused-styled at focus None"
        );
        println!("thread_view_single_mail_frame_at_focus_none: pinned");
    }

    /// Left at the terminal `Thread` state must pass through unconsumed so
    /// the listing component's `Focus::Entry + focus_left → set_focus(None)`
    /// branch closes the view and refocuses the grid.
    #[test]
    fn thread_view_focus_left_at_thread_state_passes_through_to_listing() {
        let mut ctx = mock_context();
        let mut view = make_two_mail_thread_view(&mut ctx, ThreadViewFocus::Thread);
        view.new_cursor_pos = 0;

        let mut event = UIEvent::Input(Key::Left);
        let consumed = view.process_event(&mut event, &mut ctx);

        assert!(
            !consumed,
            "`focus_left` at `Thread` state must pass through to the listing"
        );
        assert!(
            matches!(view.focus, ThreadViewFocus::Thread),
            "pass-through must not change focus"
        );
        assert_eq!(
            view.new_cursor_pos, 0,
            "pass-through must not move the cursor"
        );
    }

    /// Left at the split state must pass through unconsumed (same listing
    /// exit branch) instead of entering the thread-list-only state.
    #[test]
    fn thread_view_focus_left_at_none_passes_through_to_listing() {
        let mut ctx = mock_context();
        let mut view = make_two_mail_thread_view(&mut ctx, ThreadViewFocus::None);
        view.new_cursor_pos = 0;

        let mut event = UIEvent::Input(Key::Left);
        let consumed = view.process_event(&mut event, &mut ctx);

        assert!(
            !consumed,
            "`focus_left` at split (`None`) state must pass through to the listing"
        );
        assert!(
            matches!(view.focus, ThreadViewFocus::None),
            "pass-through must not change focus"
        );
        assert_eq!(
            view.new_cursor_pos, 0,
            "pass-through must not move the cursor"
        );
    }

    /// Single-mail refinement: a single-mail thread has no conversation stop,
    /// so Left at the mail-detail state passes through (the listing exits the
    /// view directly) instead of stopping at a degenerate empty split.
    #[test]
    fn thread_view_focus_left_at_mailview_single_mail_passes_through() {
        let mut ctx = mock_context();
        let mut view = make_single_mail_thread_view(&mut ctx, ThreadViewFocus::MailView);

        let mut event = UIEvent::Input(Key::Left);
        let consumed = view.process_event(&mut event, &mut ctx);

        assert!(
            !consumed,
            "`focus_left` at `MailView` over a single-mail thread must pass through"
        );
        assert!(
            matches!(view.focus, ThreadViewFocus::MailView),
            "pass-through must not change focus"
        );
    }

    /// Lock: in a multi-mail thread, Left at the mail-detail state keeps
    /// falling back to the thread-list split.
    #[test]
    fn thread_view_focus_left_at_mailview_multi_mail_falls_back_to_split() {
        let mut ctx = mock_context();
        let mut view = make_two_mail_thread_view(&mut ctx, ThreadViewFocus::MailView);

        let mut event = UIEvent::Input(Key::Left);
        let consumed = view.process_event(&mut event, &mut ctx);

        assert!(consumed, "`focus_left` at `MailView` must be consumed");
        assert!(
            matches!(view.focus, ThreadViewFocus::None),
            "`focus_left` at `MailView` must fall back to the split state"
        );
    }

    /// Lock: Right at the mail-detail state stays consumed (terminal stop of
    /// the chain; the listing's `Entry + focus_right → EntryFullscreen`
    /// branch must never fire from arrow keys).
    #[test]
    fn thread_view_focus_right_at_mailview_stays_consumed() {
        let mut ctx = mock_context();
        let mut view = make_two_mail_thread_view(&mut ctx, ThreadViewFocus::MailView);

        let mut event = UIEvent::Input(Key::Right);
        let consumed = view.process_event(&mut event, &mut ctx);

        assert!(
            consumed,
            "`focus_right` at `MailView` must stay consumed (terminal stop)"
        );
        assert!(
            matches!(view.focus, ThreadViewFocus::MailView),
            "terminal stop must not change focus"
        );
    }

    /// `focus_right` must open the thread-list SELECTION: after the cursor
    /// moves away from the initially expanded entry, Right (split → mail
    /// detail) must expand the entry under the cursor, not keep the stale
    /// expanded one.
    #[test]
    fn thread_view_focus_right_at_none_opens_cursor_selected_mail() {
        let mut ctx = mock_context();
        ctx.settings.shortcuts.thread_view.scroll_up = Key::Up;
        let mut view = make_two_mail_thread_view(&mut ctx, ThreadViewFocus::None);
        assert_eq!(
            view.new_cursor_pos, view.new_expanded_pos,
            "sanity: cursor starts on the expanded (newest) entry"
        );
        let stale_expanded_hash = view.entries[view.new_expanded_pos].msg_hash;

        let mut up = UIEvent::Input(Key::Up);
        assert!(view.process_event(&mut up, &mut ctx));
        assert_eq!(view.new_cursor_pos, 0, "sanity: cursor moved to the root");

        let mut right = UIEvent::Input(Key::Right);
        assert!(view.process_event(&mut right, &mut ctx));
        assert!(
            matches!(view.focus, ThreadViewFocus::MailView),
            "Right must focus the mail view"
        );
        assert_eq!(
            view.new_expanded_pos, 0,
            "Right must expand the cursor-selected entry, not the stale one"
        );
        assert_eq!(view.expanded_pos, 0);
        assert_ne!(
            view.entries[view.new_expanded_pos].msg_hash, stale_expanded_hash,
            "the shown mail must be the selected one"
        );
    }

    /// Same selection contract from the thread-list-only state: Right
    /// (thread list → split) must expand the cursor-selected entry.
    #[test]
    fn thread_view_focus_right_at_thread_opens_cursor_selected_mail() {
        let mut ctx = mock_context();
        ctx.settings.shortcuts.thread_view.scroll_up = Key::Up;
        let mut view = make_two_mail_thread_view(&mut ctx, ThreadViewFocus::Thread);

        let mut up = UIEvent::Input(Key::Up);
        assert!(view.process_event(&mut up, &mut ctx));
        assert_eq!(view.new_cursor_pos, 0, "sanity: cursor moved to the root");

        let mut right = UIEvent::Input(Key::Right);
        assert!(view.process_event(&mut right, &mut ctx));
        assert!(
            matches!(view.focus, ThreadViewFocus::None),
            "Right must fall back to the split view"
        );
        assert_eq!(
            view.new_expanded_pos, 0,
            "Right must expand the cursor-selected entry, not the stale one"
        );
        assert_eq!(view.expanded_pos, 0);
    }

    /// A single-mail thread has no thread-list pane (draw renders only the
    /// mail view), so the view must START at the mail detail — paging keys
    /// must reach the mail content without an extra focus step.
    #[test]
    fn thread_view_single_mail_starts_focused_on_mail_view() {
        let mut ctx = mock_context();
        let view = make_single_mail_thread_view(&mut ctx, ThreadViewFocus::None);

        assert!(
            matches!(view.focus, ThreadViewFocus::MailView),
            "single-mail thread must start focused on the mail view, not the \
             invisible thread list"
        );
    }

    /// Reported flow: open a single-mail thread from the listing (Right),
    /// then page — `PageDown` must page the MAIL content immediately, not be
    /// eaten by the invisible one-entry thread list's page arms.
    #[test]
    fn thread_view_single_mail_pages_mail_without_extra_focus_step() {
        let mut ctx = mock_context();
        let mut view = make_single_mail_thread_view(&mut ctx, ThreadViewFocus::None);
        load_expanded_entry(&mut view, &mut ctx, ROOT_MAIL_BYTES);

        let mut page_down = UIEvent::Input(Key::PageDown);
        let consumed = view.process_event(&mut page_down, &mut ctx);

        assert!(consumed, "PageDown must be consumed");
        assert!(
            view.movement.is_none(),
            "PageDown must page the mail content, not the invisible thread list"
        );
    }

    /// A `None` focus over a single-mail thread (reachable via the p/t
    /// visibility toggles) must not strand paging keys on the dead
    /// thread-list arms either.
    #[test]
    fn thread_view_single_mail_at_none_routes_paging_to_mail() {
        let mut ctx = mock_context();
        let mut view = make_single_mail_thread_view(&mut ctx, ThreadViewFocus::None);
        load_expanded_entry(&mut view, &mut ctx, ROOT_MAIL_BYTES);
        view.focus = ThreadViewFocus::None;

        let mut page_down = UIEvent::Input(Key::PageDown);
        let consumed = view.process_event(&mut page_down, &mut ctx);

        assert!(consumed, "PageDown must be consumed");
        assert!(
            view.movement.is_none(),
            "PageDown must reach the mail view even at focus None over a single mail"
        );
    }

    /// `j`/`k` selection switching must switch the mail pane content live,
    /// with no Enter step: the expanded entry follows the cursor.
    #[test]
    fn thread_view_scroll_switches_mail_content_live() {
        let mut ctx = mock_context();
        ctx.settings.shortcuts.thread_view.scroll_up = Key::Up;
        let mut view = make_two_mail_thread_view(&mut ctx, ThreadViewFocus::None);
        assert_eq!(view.new_cursor_pos, 1, "sanity: cursor starts on the reply");

        let mut up = UIEvent::Input(Key::Up);
        assert!(view.process_event(&mut up, &mut ctx));

        assert_eq!(view.new_cursor_pos, 0, "cursor moved to the root");
        assert_eq!(
            view.new_expanded_pos, 0,
            "the mail pane must follow the selection immediately (no Enter)"
        );
        assert_eq!(view.expanded_pos, 0);
    }

    /// Page/Home/End selection switching (applied at draw time from
    /// `self.movement`) must switch the mail pane content live as well.
    #[test]
    fn thread_view_page_movement_switches_mail_content_live() {
        let mut ctx = mock_context();
        let mut view = make_two_mail_thread_view(&mut ctx, ThreadViewFocus::None);
        assert_eq!(view.new_cursor_pos, 1, "sanity: cursor starts on the reply");

        let mut home = UIEvent::Input(Key::Home);
        assert!(view.process_event(&mut home, &mut ctx));

        let theme = crate::conf::value(&ctx, "theme_default");
        let mut screen = crate::terminal::Screen::<crate::terminal::Virtual>::new(theme);
        let _ = screen.resize(80, 24);
        let screen_area = screen.area();
        view.draw_list(screen.grid_mut(), screen_area, &mut ctx);

        assert_eq!(view.new_cursor_pos, 0, "Home moved the cursor");
        assert_eq!(
            view.new_expanded_pos, 0,
            "the mail pane must follow the page-moved selection"
        );
    }

    /// `reverse_thread_order` deliberately re-anchors the expanded entry
    /// across the reorder; live-follow must not clobber it (the reorder
    /// never goes through the selection-movement paths).
    #[test]
    fn thread_view_reverse_keeps_expanded_anchor() {
        let mut ctx = mock_context();
        let mut view = make_two_mail_thread_view(&mut ctx, ThreadViewFocus::None);
        // `initiate` leaves `expanded_pos` at a +1 sentinel until the first
        // draw commits it (draw() top); commit it the same way before use.
        view.expanded_pos = view.new_expanded_pos;
        let expanded_hash = view.entries[view.new_expanded_pos].msg_hash;

        let mut reverse = UIEvent::Input(Key::Ctrl('r'));
        assert!(view.process_event(&mut reverse, &mut ctx));

        assert_eq!(
            view.entries[view.new_expanded_pos].msg_hash, expanded_hash,
            "reverse must keep the expanded mail anchored"
        );
    }

    /// Raw bytes of the root mail used by the thread-view builders; must stay
    /// identical to the bytes inserted into the mock collection.
    const ROOT_MAIL_BYTES: &[u8] = b"From: a@b.example\r\n\
To: c@d.example\r\n\
Subject: focus\r\n\
Message-ID: <focus-root@x.example>\r\n\
Date: Thu, 1 Jan 2026 00:00:00 +0000\r\n\
\r\n\
root\r\n";

    /// Raw bytes of the reply mail from `make_two_mail_thread_view`; must stay
    /// identical to the reply constructed there. `ThreadView::new` with no
    /// expanded hash expands the newest mail, i.e. the reply.
    const REPLY_MAIL_BYTES: &[u8] = b"From: c@d.example\r\n\
To: a@b.example\r\n\
Subject: Re: focus\r\n\
Message-ID: <focus-reply@x.example>\r\n\
In-Reply-To: <focus-root@x.example>\r\n\
Date: Thu, 1 Jan 2026 00:01:00 +0000\r\n\
\r\n\
reply\r\n";

    /// Raw bytes of a long-body reply variant used by the scroll-to-bottom
    /// case: same headers as `REPLY_MAIL_BYTES` (so loading it into the
    /// expanded reply entry rewrites no header values), but with a ~60-line
    /// body that overflows an 80x24 mail pane, making the pager actually
    /// scroll instead of sitting at its bottom edge from the start.
    const LONG_MAIL_BYTES: &[u8] = b"From: c@d.example\r\n\
To: a@b.example\r\n\
Subject: Re: focus\r\n\
Message-ID: <focus-reply@x.example>\r\n\
In-Reply-To: <focus-root@x.example>\r\n\
Date: Thu, 1 Jan 2026 00:01:00 +0000\r\n\
\r\n\
Long body line 01: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 02: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 03: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 04: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 05: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 06: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 07: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 08: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 09: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 10: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 11: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 12: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 13: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 14: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 15: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 16: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 17: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 18: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 19: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 20: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 21: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 22: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 23: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 24: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 25: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 26: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 27: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 28: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 29: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 30: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 31: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 32: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 33: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 34: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 35: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 36: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 37: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 38: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 39: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 40: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 41: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 42: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 43: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 44: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 45: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 46: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 47: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 48: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 49: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 50: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 51: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 52: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 53: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 54: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 55: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 56: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 57: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 58: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 59: the quick brown fox jumps over the lazy dog again.\r\n\
Long body line 60: the quick brown fox jumps over the lazy dog again.\r\n\
";

    /// Drive the expanded entry's mail view to the `Loaded` state through the
    /// public `MailViewState::load_bytes` (simpler than hand-building the
    /// `Loaded` variant).
    fn load_expanded_entry(view: &mut ThreadView, context: &mut Context, bytes: &[u8]) {
        let expanded_pos = view.new_expanded_pos;
        MailViewState::load_bytes(
            &mut view.entries[expanded_pos].mailview,
            bytes.to_vec(),
            context,
        );
    }

    /// Core flip: at focus None, a thread-view navigation key must move the
    /// thread list cursor even though the expanded (Loaded) mail view would
    /// swallow it via the headers-walk (`headers_cursor(0) < headers_no(5)`).
    #[test]
    fn thread_view_input_down_at_focus_none_moves_thread_cursor() {
        let mut ctx = mock_context();
        ctx.settings.shortcuts.thread_view.scroll_down = Key::Down;
        ctx.settings.shortcuts.pager.scroll_down = Key::Down;
        let mut view = make_two_mail_thread_view(&mut ctx, ThreadViewFocus::None);
        load_expanded_entry(&mut view, &mut ctx, REPLY_MAIL_BYTES);
        view.new_cursor_pos = 0;

        let mut event = UIEvent::Input(Key::Down);
        let consumed = view.process_event(&mut event, &mut ctx);

        assert!(consumed, "Down must be consumed");
        assert_eq!(
            view.new_cursor_pos, 1,
            "thread list cursor must advance at focus None"
        );
    }

    /// Up twin of `thread_view_input_down_at_focus_none_moves_thread_cursor`:
    /// at focus None, a rebound thread-view `scroll_up` must move the thread
    /// list cursor up even when the pager binds the same key and the
    /// expanded (Loaded) mail view would otherwise swallow it.
    #[test]
    fn thread_view_input_up_at_focus_none_moves_thread_cursor() {
        let mut ctx = mock_context();
        ctx.settings.shortcuts.thread_view.scroll_up = Key::Up;
        ctx.settings.shortcuts.pager.scroll_up = Key::Up;
        let mut view = make_two_mail_thread_view(&mut ctx, ThreadViewFocus::None);
        load_expanded_entry(&mut view, &mut ctx, REPLY_MAIL_BYTES);
        view.new_cursor_pos = 1;

        let mut event = UIEvent::Input(Key::Up);
        let consumed = view.process_event(&mut event, &mut ctx);

        assert!(consumed, "Up must be consumed");
        assert_eq!(
            view.new_cursor_pos, 0,
            "thread list cursor must move up at focus None"
        );
    }

    /// Modal gate: while the expanded mail view has an active modal, 'j'
    /// (thread `scroll_down` AND the Selector's GENERAL `scroll_down` default)
    /// must go to the modal; the thread cursor must not move.
    #[test]
    fn thread_view_input_yields_to_active_modal() {
        let mut ctx = mock_context();
        // 'j' is simultaneously the thread `scroll_down` and the Selector's
        // GENERAL `scroll_down` (both injected: the arrow-key defaults no
        // longer bind 'j').
        ctx.settings.shortcuts.thread_view.scroll_down = Key::Char('j');
        ctx.settings.shortcuts.general.scroll_down = Key::Char('j');
        let mut view = make_two_mail_thread_view(&mut ctx, ThreadViewFocus::None);
        load_expanded_entry(&mut view, &mut ctx, REPLY_MAIL_BYTES);
        view.new_cursor_pos = 0;
        view.entries[view.new_expanded_pos]
            .mailview
            .open_force_charset_modal_for_tests(&ctx);

        let mut event = UIEvent::Input(Key::Char('j'));
        let consumed = view.process_event(&mut event, &mut ctx);

        assert!(consumed, "the modal must consume 'j'");
        assert_eq!(
            view.new_cursor_pos, 0,
            "'j' must reach the modal, not the thread list, while a modal is open"
        );
    }

    /// Status quo lock: at focus `MailView` the embedded mail view keeps
    /// consuming Down first; the thread cursor must not move.
    #[test]
    fn thread_view_input_down_at_focus_mailview_keeps_status_quo() {
        let mut ctx = mock_context();
        ctx.settings.shortcuts.thread_view.scroll_down = Key::Down;
        ctx.settings.shortcuts.pager.scroll_down = Key::Down;
        let mut view = make_two_mail_thread_view(&mut ctx, ThreadViewFocus::MailView);
        load_expanded_entry(&mut view, &mut ctx, REPLY_MAIL_BYTES);
        view.new_cursor_pos = 0;

        let mut event = UIEvent::Input(Key::Down);
        let consumed = view.process_event(&mut event, &mut ctx);

        assert!(
            consumed,
            "the embedded mail view consumes Down (headers-walk)"
        );
        assert_eq!(
            view.new_cursor_pos, 0,
            "thread cursor must not move at focus MailView"
        );
    }

    /// Core: at focus `MailView`, once the headers-walk is exhausted and the
    /// (short-body) pager sits at its bottom edge, Down must be consumed as
    /// a no-op instead of bubbling into the thread-list scroll arm (which
    /// would switch to the next message).
    #[test]
    fn thread_view_input_down_at_mailview_body_end_stops_no_mail_switch() {
        let mut ctx = mock_context();
        ctx.settings.shortcuts.thread_view.scroll_down = Key::Down;
        ctx.settings.shortcuts.pager.scroll_down = Key::Down;
        let mut view = make_two_mail_thread_view(&mut ctx, ThreadViewFocus::MailView);
        load_expanded_entry(&mut view, &mut ctx, REPLY_MAIL_BYTES);
        view.new_cursor_pos = 0;

        // 6 = 5 headers-walk steps + 1 bubble past the body bottom;
        // coupled to REPLY_MAIL_BYTES and the headers_no construction
        // default of 5 (no draw: rows_lt_height stays at its construction
        // default false, i.e. a body already at its bottom edge).
        for _ in 0..6 {
            let mut event = UIEvent::Input(Key::Down);
            assert!(
                view.process_event(&mut event, &mut ctx),
                "Down must stay consumed at every step"
            );
        }
        assert_eq!(
            view.new_cursor_pos, 0,
            "Down at the body bottom must not switch to the next message"
        );
    }

    /// Up twin: at focus `MailView`, with the headers-walk at its start and
    /// the pager at its top edge, Up must be consumed as a no-op instead of
    /// bubbling into the thread-list scroll-up arm (previous message).
    #[test]
    fn thread_view_input_up_at_mailview_body_top_stops_no_mail_switch() {
        let mut ctx = mock_context();
        ctx.settings.shortcuts.thread_view.scroll_up = Key::Up;
        ctx.settings.shortcuts.pager.scroll_up = Key::Up;
        let mut view = make_two_mail_thread_view(&mut ctx, ThreadViewFocus::MailView);
        load_expanded_entry(&mut view, &mut ctx, REPLY_MAIL_BYTES);
        view.new_cursor_pos = 1;
        // The rest scroll_up arm guards on the OLD value of cursor_pos; set
        // it explicitly instead of relying on construction defaults.
        view.cursor_pos = 1;

        let mut event = UIEvent::Input(Key::Up);
        let consumed = view.process_event(&mut event, &mut ctx);

        assert!(consumed, "Up must be consumed");
        assert_eq!(
            view.new_cursor_pos, 1,
            "Up at the body top must not switch to the previous message"
        );
    }

    /// Status quo lock: while the expanded body is still loading (state not
    /// `Loaded`), Down must keep bubbling to the thread list so the thread
    /// stays browsable — probes the `is_loaded` gate of the interception.
    #[test]
    fn thread_view_input_down_while_body_loading_moves_thread_cursor() {
        let mut ctx = mock_context();
        ctx.settings.shortcuts.thread_view.scroll_down = Key::Down;
        ctx.settings.shortcuts.pager.scroll_down = Key::Down;
        let mut view = make_two_mail_thread_view(&mut ctx, ThreadViewFocus::MailView);
        // No load_expanded_entry: the expanded entry's init_futures fail on
        // the mock backend, so its mail view state stays Init.
        view.new_cursor_pos = 0;

        let mut event = UIEvent::Input(Key::Down);
        let consumed = view.process_event(&mut event, &mut ctx);

        assert!(consumed, "Down must be consumed by the thread list");
        assert_eq!(
            view.new_cursor_pos, 1,
            "Down must keep moving the thread cursor while the body is loading"
        );
    }

    /// Guard: the interception is scoped to focus `MailView` — at focus
    /// `Thread` (thread list fullscreen) a thread-view `scroll_down` must
    /// keep moving the thread list cursor (no mail view is involved).
    #[test]
    fn thread_view_input_down_at_thread_focus_moves_cursor() {
        let mut ctx = mock_context();
        ctx.settings.shortcuts.thread_view.scroll_down = Key::Down;
        ctx.settings.shortcuts.pager.scroll_down = Key::Down;
        let mut view = make_two_mail_thread_view(&mut ctx, ThreadViewFocus::Thread);
        view.new_cursor_pos = 0;

        let mut event = UIEvent::Input(Key::Down);
        let consumed = view.process_event(&mut event, &mut ctx);

        assert!(consumed, "Down must be consumed");
        assert_eq!(
            view.new_cursor_pos, 1,
            "thread list cursor must advance at focus Thread"
        );
    }

    /// Up twin of `thread_view_input_down_at_thread_focus_moves_cursor`:
    /// at focus `Thread`, a thread-view `scroll_up` must keep moving the
    /// thread list cursor up.
    #[test]
    fn thread_view_input_up_at_thread_focus_moves_cursor() {
        let mut ctx = mock_context();
        ctx.settings.shortcuts.thread_view.scroll_up = Key::Up;
        ctx.settings.shortcuts.pager.scroll_up = Key::Up;
        let mut view = make_two_mail_thread_view(&mut ctx, ThreadViewFocus::Thread);
        // The rest scroll_up arm guards on the OLD value of cursor_pos; set
        // it explicitly instead of relying on construction defaults.
        view.cursor_pos = 1;
        view.new_cursor_pos = 1;

        let mut event = UIEvent::Input(Key::Up);
        let consumed = view.process_event(&mut event, &mut ctx);

        assert!(consumed, "Up must be consumed");
        assert_eq!(
            view.new_cursor_pos, 0,
            "thread list cursor must move up at focus Thread"
        );
    }

    /// Guard: the interception covers only the vertical scroll keys — a
    /// non-vertical thread-view key (`collapse_subtree`, rebound to a fresh
    /// 'x' to avoid the default 'h' colliding with the envelope view's
    /// `toggle_expand_headers`) must still reach its `process_event_rest`
    /// arm at focus `MailView` and flip `hidden`.
    #[test]
    fn thread_view_non_vertical_key_collapse_subtree_not_intercepted_at_mailview() {
        let mut ctx = mock_context();
        ctx.settings.shortcuts.thread_view.collapse_subtree = Key::Char('x');
        let mut view = make_two_mail_thread_view(&mut ctx, ThreadViewFocus::MailView);
        load_expanded_entry(&mut view, &mut ctx, REPLY_MAIL_BYTES);
        // Construction leaves the thread-list cursor on the expanded (newest)
        // mail; the collapse arm toggles the entry under that cursor.

        let mut event = UIEvent::Input(Key::Char('x'));
        let consumed = view.process_event(&mut event, &mut ctx);

        assert!(consumed, "'x' must be consumed");
        assert!(
            view.entries[view.new_expanded_pos].hidden,
            "collapse_subtree must still toggle the entry at focus MailView"
        );
    }

    /// EnvelopeView-level contract the thread-view interception relies on:
    /// the headers-walk consumes exactly `headers_no` Downs (construction
    /// default 5), the next Down falls through unconsumed (default pager
    /// sits at its bottom edge, `rows_lt_height == false`), and Ups walk
    /// back up to the top.
    #[test]
    fn envelope_view_headers_walk_down_then_fallthrough_roundtrip() {
        let mut ctx = mock_context();
        ctx.settings.shortcuts.pager.scroll_down = Key::Down;
        ctx.settings.shortcuts.pager.scroll_up = Key::Up;
        let mail = Mail::new(REPLY_MAIL_BYTES.to_vec(), None).expect("could not parse reply mail");
        let mut env_view = EnvelopeView::new(mail, None, None, None, ctx.main_loop_handler.clone());

        for step in 1..=5 {
            let mut event = UIEvent::Input(Key::Down);
            assert!(
                env_view.process_event(&mut event, &mut ctx),
                "Down #{step} must be consumed by the headers walk"
            );
        }
        assert_eq!(env_view.headers_cursor, 5, "headers walk must be exhausted");

        let mut event = UIEvent::Input(Key::Down);
        assert!(
            !env_view.process_event(&mut event, &mut ctx),
            "Down past the body bottom must fall through unconsumed"
        );

        for step in 1..=5 {
            let mut event = UIEvent::Input(Key::Up);
            assert!(
                env_view.process_event(&mut event, &mut ctx),
                "Up #{step} must be consumed by the headers walk"
            );
        }
        assert_eq!(
            env_view.headers_cursor, 0,
            "headers walk must return to the top"
        );
    }

    /// Single-mail status quo lock (green before and after the fix): a
    /// single-entry thread has no mail switching; every Down (headers-walk,
    /// then the body-edge no-op) and the final Up must be consumed with the
    /// cursor never moving.
    #[test]
    fn thread_view_single_mail_input_no_mail_switch() {
        let mut ctx = mock_context();
        ctx.settings.shortcuts.thread_view.scroll_down = Key::Down;
        ctx.settings.shortcuts.thread_view.scroll_up = Key::Up;
        ctx.settings.shortcuts.pager.scroll_down = Key::Down;
        ctx.settings.shortcuts.pager.scroll_up = Key::Up;
        let mut view = make_single_mail_thread_view(&mut ctx, ThreadViewFocus::MailView);
        load_expanded_entry(&mut view, &mut ctx, ROOT_MAIL_BYTES);
        view.new_cursor_pos = 0;

        for _ in 0..6 {
            let mut event = UIEvent::Input(Key::Down);
            assert!(
                view.process_event(&mut event, &mut ctx),
                "Down must stay consumed in a single-mail thread"
            );
        }
        let mut event = UIEvent::Input(Key::Up);
        assert!(
            view.process_event(&mut event, &mut ctx),
            "Up must be consumed in a single-mail thread"
        );
        assert_eq!(
            view.new_cursor_pos, 0,
            "single-mail thread cursor must never move"
        );
    }

    /// Unbound key routing: 'c' (bound only in `envelope_view`) must still reach
    /// the EXPANDED entry's mail view and open its contact selector there.
    #[test]
    fn thread_view_unbound_key_still_reaches_expanded_mailview() {
        let mut ctx = mock_context();
        let mut view = make_two_mail_thread_view(&mut ctx, ThreadViewFocus::None);
        load_expanded_entry(&mut view, &mut ctx, REPLY_MAIL_BYTES);
        view.new_cursor_pos = 0;

        let mut event = UIEvent::Input(Key::Char('c'));
        let consumed = view.process_event(&mut event, &mut ctx);

        assert!(consumed, "'c' must be consumed by the expanded mail view");
        assert_eq!(
            view.new_cursor_pos, 0,
            "thread cursor must not move for 'c'"
        );
        assert!(
            view.entries[view.new_expanded_pos]
                .mailview
                .has_active_modal(),
            "'c' must open the contact selector in the EXPANDED entry's mail view"
        );
        for (idx, entry) in view.entries.iter().enumerate() {
            if idx != view.new_expanded_pos {
                assert!(
                    !entry.mailview.has_active_modal(),
                    "non-expanded entry {idx} must not consume 'c'"
                );
            }
        }
    }

    /// Non-Input ordering: Resize still returns false (status quo).
    #[test]
    fn thread_view_non_input_event_ordering_unchanged() {
        let mut ctx = mock_context();
        let mut view = make_two_mail_thread_view(&mut ctx, ThreadViewFocus::None);

        let mut event = UIEvent::Resize;
        assert!(!view.process_event(&mut event, &mut ctx));
    }

    /// `has_active_modal` across the three layers, over the field
    /// combinations reachable through public/test seams.
    #[test]
    fn thread_view_has_active_modal_accessors_truth_table() {
        let mut ctx = mock_context();

        // Envelope layer: fresh view has no modal; force_charset opens one.
        let mail = Mail::new(REPLY_MAIL_BYTES.to_vec(), None).expect("could not parse reply mail");
        let mut env_view = EnvelopeView::new(mail, None, None, None, ctx.main_loop_handler.clone());
        assert!(!env_view.has_active_modal());
        env_view.set_force_charset_modal_for_tests(&ctx);
        assert!(env_view.has_active_modal());

        // State layer: non-Loaded variants never report a modal.
        let state = MailViewState::Init {
            pending_action: None,
        };
        assert!(!state.has_active_modal());

        // View layer through a real ThreadView entry: Init → false, Loaded →
        // false, Loaded + modal → true.
        let mut view = make_two_mail_thread_view(&mut ctx, ThreadViewFocus::None);
        let pos = view.new_expanded_pos;
        assert!(!view.entries[pos].mailview.has_active_modal());
        load_expanded_entry(&mut view, &mut ctx, REPLY_MAIL_BYTES);
        assert!(!view.entries[pos].mailview.has_active_modal());
        view.entries[pos]
            .mailview
            .open_force_charset_modal_for_tests(&ctx);
        assert!(view.entries[pos].mailview.has_active_modal());
    }

    /// `is_thread_view_input` truth table: true for every `THREAD_VIEW`
    /// section key EXCEPT the focus keys (handled exclusively by the
    /// pre-arms) plus the three GENERAL keys the thread arms reference;
    /// false for the focus keys and for pager/listing/general keys that
    /// don't collide with the detected set.
    #[test]
    fn thread_view_is_thread_view_input_truth_table() {
        let mut ctx = mock_context();
        let view = make_two_mail_thread_view(&mut ctx, ThreadViewFocus::None);

        let shortcuts = view.shortcuts(&ctx);
        let mut thread_keys: Vec<&Key> = shortcuts[Shortcuts::THREAD_VIEW]
            .iter()
            .filter(|(name, _)| !matches!(**name, "focus_left" | "focus_right"))
            .map(|(_, key)| key)
            .collect();
        for name in ["home_page", "end_page", "open_entry"] {
            if let Some(key) = shortcuts[Shortcuts::GENERAL].get(name) {
                thread_keys.push(key);
            }
        }
        for key in &thread_keys {
            assert!(
                view.is_thread_view_input(key, &ctx),
                "thread-view key {key:?} must be detected"
            );
        }
        for name in ["focus_left", "focus_right"] {
            if let Some(key) = shortcuts[Shortcuts::THREAD_VIEW].get(name) {
                assert!(
                    !view.is_thread_view_input(key, &ctx),
                    "focus key {name:?} ({key:?}) must NOT be detected — it is handled \
                     exclusively by the pre-arms"
                );
            }
        }

        let sections = [
            (&ctx.settings.shortcuts.pager.key_values(), "pager"),
            (&ctx.settings.shortcuts.listing.key_values(), "listing"),
            (&ctx.settings.shortcuts.general.key_values(), "general"),
        ];
        for (section, label) in sections {
            for (name, key) in section.iter() {
                // Skip keys that collide with thread keys by default (e.g.
                // pager 'j'/'k', listing Left/Right, general 'h'/'l').
                if thread_keys.contains(&key) {
                    continue;
                }
                assert!(
                    !view.is_thread_view_input(key, &ctx),
                    "{label} key {name:?} ({key:?}) must not be detected as thread input"
                );
            }
        }
    }

    /// Long-body scroll-to-bottom (two assertions in one, the strongest
    /// green-state scenario): with focus `MailView` over a body taller than
    /// the pane, (a) every Down before the bottom must scroll the body —
    /// consumed by the mail view with the thread cursor frozen, proving the
    /// interception does not swallow genuine body scrolling — and (b) the
    /// first Down after the pager really reaches the last line must stop
    /// there as a consumed no-op instead of switching to the next mail.
    /// Red-state reference: without the interception, the first Down that
    /// hits the body edge would bubble into the thread-list `scroll_down`
    /// arm and advance `new_cursor_pos`.
    ///
    /// Unlike the short-body cases this needs draws between keys: the
    /// `EnvelopeView` lazily builds its pager on its first draw, and the
    /// pager only consumes Down once a draw has computed `rows_lt_height`.
    ///
    /// Draw count is empirically anchored at ONE draw per Down (no priming
    /// draw): `load_bytes` leaves the mail view `initialized`, so
    /// `MailView::draw`'s `!initialized` early return does not apply, and
    /// the fresh pager draws dirty on the very first draw and pushes its
    /// `ScrollUpdate` right away. The consumed key itself keeps the chain
    /// dirty (headers-walk/pager `set_dirty(true)` bubbles up through
    /// `ThreadView::is_dirty`'s mailview term), so each draw goes through
    /// the production redraw path with no manual `set_dirty`.
    ///
    /// Bottom detection uses the pager's `StatusEvent::ScrollUpdate`
    /// (pushed at the end of `Pager::draw`): `Update` carries `shown_lines`
    /// and `total_lines` plus `has_more_lines`, which only reports the lazy
    /// line breaker (finished after the first draw for a 60-line body, as
    /// `PAGES_AHEAD_TO_RENDER_NO` is 16), so the bottom condition is
    /// `!has_more_lines && shown_lines >= total_lines`; `End` (content
    /// fits the pane, search only) is treated as the bottom too. Replies
    /// are drained and matched per event type: the mock backend's
    /// `set_flags` and init-futures `Notification`s are noise here.
    #[test]
    fn thread_view_input_down_long_body_scrolls_to_bottom_then_stops() {
        let mut ctx = mock_context();
        ctx.settings.shortcuts.thread_view.scroll_down = Key::Down;
        ctx.settings.shortcuts.pager.scroll_down = Key::Down;
        let mut view = make_two_mail_thread_view(&mut ctx, ThreadViewFocus::MailView);
        load_expanded_entry(&mut view, &mut ctx, LONG_MAIL_BYTES);
        view.new_cursor_pos = 0;

        let theme_default = crate::conf::value(&ctx, "theme_default");
        let mut screen = crate::terminal::Screen::<crate::terminal::Virtual>::new(theme_default);
        assert!(screen.resize(80, 24));
        let area = screen.area();

        let mut reached_bottom = false;
        let mut rounds = 0;
        while rounds < 100 {
            rounds += 1;
            let mut event = UIEvent::Input(Key::Down);
            assert!(
                view.process_event(&mut event, &mut ctx),
                "Down #{rounds} must be consumed while the body scrolls"
            );
            assert_eq!(
                view.new_cursor_pos, 0,
                "Down #{rounds} scrolls the body, not the thread list"
            );

            view.draw(screen.grid_mut(), area, &mut ctx);

            let mut bottom = false;
            for reply in ctx.replies.drain(0..) {
                bottom |= match reply {
                    UIEvent::StatusEvent(StatusEvent::ScrollUpdate(
                        crate::components::ScrollUpdate::End(_),
                    )) => true,
                    UIEvent::StatusEvent(StatusEvent::ScrollUpdate(
                        crate::components::ScrollUpdate::Update {
                            context:
                                crate::components::ScrollContext {
                                    shown_lines,
                                    total_lines,
                                    has_more_lines,
                                },
                            ..
                        },
                    )) => !has_more_lines && shown_lines >= total_lines,
                    _ => false,
                };
            }
            if bottom {
                reached_bottom = true;
                break;
            }
        }
        assert!(
            reached_bottom,
            "the long body must reach its bottom within 100 Down+draw rounds \
             (stopped after {rounds})"
        );

        let mut event = UIEvent::Input(Key::Down);
        assert!(
            view.process_event(&mut event, &mut ctx),
            "Down at the body bottom must be consumed as a no-op"
        );
        assert_eq!(
            view.new_cursor_pos, 0,
            "Down at the body bottom must not switch to the next mail"
        );
    }
}
