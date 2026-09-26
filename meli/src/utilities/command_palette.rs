/*
 * meli
 *
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

//! VSCode-style command palette: a floating panel centred over the
//! current UI with a single-line input at its top and a fuzzy-matched
//! command list below it.
//!
//! The palette is a plain struct (not a [`Component`]): it never enters
//! the component tree — [`StatusBar`](super::StatusBar) owns it for its
//! whole lifetime, feeds it `UIEvent::CmdInput` keys and draws it over
//! the full screen area when the mode is `UIMode::Command`.

use std::collections::HashSet;

use nucleo_matcher::{
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
    Config, Matcher, Utf32Str,
};
use ratatui_textarea::TextArea;

use super::*;

/// One renderable row in the palette list: a section label or a
/// selectable command entry.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PaletteItem {
    Section(&'static str),
    Entry {
        text: String,
        desc: String,
        /// Char indices of `text` matched by the current query (sorted,
        /// deduplicated); empty when there is no query.
        match_indices: Vec<u32>,
        /// nucleo match score; `0` when there is no query. Entries are
        /// kept score-descending.
        score: u32,
    },
}

/// Command palette: fuzzy-searchable floating command list.
///
/// Keys ([`CommandPalette::process_key`]): typing edits the query, `Tab`
/// completes the input to the selected entry, `Enter` executes it (or the
/// raw input), `Up`/`Down` (`Ctrl-P`/`Ctrl-N`) move the selection, `Esc`
/// closes without executing.
#[derive(Debug)]
pub struct CommandPalette {
    /// `(name, description)` from `COMMAND_COMPLETION`, table order. Names
    /// keep their original tags, including the trailing space that marks
    /// commands requiring arguments.
    commands: Vec<(String, String)>,
    /// Command history, oldest → newest (the history file's order).
    history: Vec<String>,
    /// The query input. `TextArea` keeps render caches in interior-
    /// mutable cells (`RefCell`/`Cell`), so it is `!Sync`; the mutex
    /// keeps the palette shareable like all other component state. The
    /// UI thread is the only accessor, so the lock is uncontended.
    textarea: std::sync::Mutex<TextArea<'static>>,
    matcher: Matcher,
    items: Vec<PaletteItem>,
    /// Index into `items` of the selected entry; section rows are never
    /// selected.
    selection: Option<usize>,
    /// `false` for tests: injected history must never be appended to the
    /// user's real `cmd_history` file.
    log_history: bool,
    dirty: bool,
}

impl CommandPalette {
    pub fn new(history: Vec<String>) -> Self {
        Self::build(history, true)
    }

    /// Test constructor: `history` is injected and never written back to
    /// the `cmd_history` file.
    pub fn new_for_tests(history: Vec<String>) -> Self {
        Self::build(history, false)
    }

    fn build(history: Vec<String>, log_history: bool) -> Self {
        let mut ret = Self {
            commands: crate::command::COMMAND_COMPLETION
                .iter()
                .map(|(name, desc, _)| ((*name).to_string(), (*desc).to_string()))
                .collect(),
            history,
            textarea: std::sync::Mutex::new(Self::fresh_textarea(String::new())),
            matcher: Matcher::new(Config::DEFAULT),
            items: Vec::new(),
            selection: None,
            log_history,
            dirty: true,
        };
        ret.refresh();
        ret
    }

    /// A fresh single-line textarea preloaded with `line` and the cursor
    /// at its end. The cursor-line underline is disabled: the cursor
    /// itself already renders reversed, and the line style would
    /// underline the whole input row.
    fn fresh_textarea(line: String) -> TextArea<'static> {
        let mut textarea = TextArea::new(vec![line]);
        textarea.set_cursor_line_style(ratatui::style::Style::default());
        textarea.move_cursor(ratatui_textarea::CursorMove::End);
        textarea
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn set_dirty(&mut self, value: bool) {
        self.dirty = value;
    }

    /// Lock the textarea (see the field docs). Unpoisoning on access: a
    /// panic while holding the lock must not brick every later palette
    /// draw.
    fn textarea(&self) -> std::sync::MutexGuard<'_, TextArea<'static>> {
        self.textarea.lock().unwrap_or_else(|err| err.into_inner())
    }

    /// Text of the selected entry, if any.
    fn selected_text(&self) -> Option<&str> {
        self.selection
            .and_then(|index| match self.items.get(index) {
                Some(PaletteItem::Entry { text, .. }) => Some(text.as_str()),
                _ => None,
            })
    }

    /// Rebuild `items` for the current query.
    ///
    /// Empty query → `Frequent` (commands used more than once, most-used
    /// first, ties lexicographic, top 10) + `History` (newest → oldest,
    /// deduplicated, minus the frequent entries, top 10) sections, or a
    /// single `All commands` section when there is no history. Non-empty
    /// query → every history entry and command fuzzy-matched by nucleo,
    /// score-descending (the stable sort keeps history entries before
    /// same-scoring table commands).
    fn refresh(&mut self) {
        let query = self
            .textarea()
            .lines()
            .first()
            .map(|line| line.trim().to_string())
            .unwrap_or_default();
        let mut items = Vec::new();
        if query.is_empty() {
            if self.history.is_empty() {
                items.push(PaletteItem::Section("All commands"));
                items.extend(self.commands.iter().map(|(text, desc)| PaletteItem::Entry {
                    text: text.clone(),
                    desc: desc.clone(),
                    match_indices: Vec::new(),
                    score: 0,
                }));
            } else {
                let mut counts: Vec<(String, usize)> = Vec::new();
                for entry in &self.history {
                    match counts.iter_mut().find(|(name, _)| name == entry) {
                        Some((_, count)) => *count += 1,
                        None => counts.push((entry.clone(), 1)),
                    }
                }
                counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                let frequent: Vec<String> = counts
                    .into_iter()
                    .filter(|(_, count)| *count > 1)
                    .take(10)
                    .map(|(name, _)| name)
                    .collect();
                let frequent_set: HashSet<&str> = frequent.iter().map(String::as_str).collect();
                let mut seen = HashSet::new();
                let recent: Vec<String> = self
                    .history
                    .iter()
                    .rev()
                    .filter(|entry| seen.insert(entry.as_str()))
                    .filter(|entry| !frequent_set.contains(entry.as_str()))
                    .take(10)
                    .cloned()
                    .collect();
                if !frequent.is_empty() {
                    items.push(PaletteItem::Section("Frequent"));
                    items.extend(frequent.into_iter().map(|text| PaletteItem::Entry {
                        text,
                        desc: String::new(),
                        match_indices: Vec::new(),
                        score: 0,
                    }));
                }
                if !recent.is_empty() {
                    items.push(PaletteItem::Section("History"));
                    items.extend(recent.into_iter().map(|text| PaletteItem::Entry {
                        text,
                        desc: String::new(),
                        match_indices: Vec::new(),
                        score: 0,
                    }));
                }
            }
        } else {
            let pattern = Pattern::new(
                &query,
                CaseMatching::Smart,
                Normalization::Smart,
                AtomKind::Fuzzy,
            );
            // Candidates: history (newest first, deduplicated) then every
            // command in table order.
            let mut seen = HashSet::new();
            let mut candidates: Vec<(String, String)> = self
                .history
                .iter()
                .rev()
                .filter(|entry| seen.insert(entry.as_str()))
                .map(|entry| (entry.clone(), String::new()))
                .collect();
            candidates.extend(self.commands.iter().cloned());
            let mut buf: Vec<char> = Vec::new();
            let mut matched: Vec<(u32, (String, String), Vec<u32>)> = Vec::new();
            for (name, desc) in candidates {
                let mut indices = Vec::new();
                let Some(score) = pattern.indices(
                    Utf32Str::new(&name, &mut buf),
                    &mut self.matcher,
                    &mut indices,
                ) else {
                    continue;
                };
                // nucleo appends per-atom indices unsorted; sort and
                // dedup for highlight runs.
                indices.sort_unstable();
                indices.dedup();
                matched.push((score, (name, desc), indices));
            }
            matched.sort_by_key(|(score, _, _)| std::cmp::Reverse(*score));
            items.extend(
                matched
                    .into_iter()
                    .map(|(score, (text, desc), match_indices)| PaletteItem::Entry {
                        text,
                        desc,
                        match_indices,
                        score,
                    }),
            );
        }
        self.selection = items
            .iter()
            .position(|item| matches!(item, PaletteItem::Entry { .. }));
        self.items = items;
        self.dirty = true;
    }

    /// Handle a `UIEvent::CmdInput` key. Execution is delegated by pushing
    /// `UIEvent::Command`: parsing, confirmation dialogs and the action
    /// itself stay in `state.rs`.
    pub fn process_key(&mut self, key: &Key, context: &mut Context) {
        match key {
            Key::Esc => {
                // Close without executing, like the old ex mode Esc.
                *self.textarea() = Self::fresh_textarea(String::new());
                self.refresh();
                context
                    .replies
                    .push_back(UIEvent::ChangeMode(UIMode::Normal));
            }
            Key::Char('\n') => {
                let text = self.selected_text().map(str::to_string).unwrap_or_else(|| {
                    self.textarea()
                        .lines()
                        .first()
                        .map(|line| line.trim().to_string())
                        .unwrap_or_default()
                });
                if !text.is_empty() {
                    if parse_command(text.as_bytes()).is_ok()
                        && self.history.last().map(String::as_str) != Some(text.as_str())
                    {
                        self.history.push(text.clone());
                        if self.log_history {
                            crate::command::history::log_cmd(text.clone());
                        }
                    }
                    context.replies.push_back(UIEvent::Command(text));
                    context
                        .replies
                        .push_back(UIEvent::ChangeMode(UIMode::Normal));
                    *self.textarea() = Self::fresh_textarea(String::new());
                    self.refresh();
                }
            }
            Key::Char('\t') => {
                // Complete the input to the selected entry (the first
                // match by default).
                if let Some(text) = self.selected_text() {
                    let text = text.to_string();
                    *self.textarea() = Self::fresh_textarea(text);
                    self.refresh();
                }
            }
            Key::Up | Key::Ctrl('p') => self.move_selection(false),
            Key::Down | Key::Ctrl('n') => self.move_selection(true),
            Key::Paste(s) => {
                // Prefill path (e.g. the listing's `/` search binding):
                // strip line breaks so the input stays single-line.
                let mut textarea = self.textarea();
                for c in s.chars().filter(|c| *c != '\n' && *c != '\r') {
                    textarea.insert_char(c);
                }
                drop(textarea);
                self.refresh();
            }
            key => {
                self.textarea().input(key_to_input(key));
                self.refresh();
            }
        }
        self.dirty = true;
    }

    /// Move the selection to the previous/next entry, skipping section
    /// rows; clamps at the first/last entry.
    fn move_selection(&mut self, down: bool) {
        let Some(current) = self.selection else {
            return;
        };
        let step = if down { 1 } else { -1 };
        let mut next = current as isize + step;
        while next >= 0
            && (next as usize) < self.items.len()
            && !matches!(self.items[next as usize], PaletteItem::Entry { .. })
        {
            next += step;
        }
        if next >= 0 && (next as usize) < self.items.len() {
            self.selection = Some(next as usize);
        }
    }

    /// Draw the palette as a floating panel over `area`: corners at
    /// (10%,10%)–(90%,90%) (80% width/height, centred both ways), input
    /// box on top, list below.
    pub fn draw(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        if area.is_empty() {
            return;
        }
        let screen = crate::terminal::ratatui_bridge::area_to_rect(area);
        let [_, rows_mid, _] = Layout::vertical([
            Constraint::Percentage(10),
            Constraint::Percentage(80),
            Constraint::Percentage(10),
        ])
        .areas(screen);
        let [_, panel_rect, _] = Layout::horizontal([
            Constraint::Percentage(10),
            Constraint::Percentage(80),
            Constraint::Percentage(10),
        ])
        .areas(rows_mid);
        let panel_area = crate::terminal::ratatui_bridge::rect_to_area(panel_rect, area);
        if panel_area.is_empty() {
            return;
        }
        // Dialog vocabulary: notification surface with a tab.focused
        // border whose bg is aligned so the ring melts into the surface.
        let surface = crate::conf::value(context, "status.notification");
        let mut border = crate::conf::value(context, "tab.focused");
        border.bg = surface.bg;
        grid.clear_area(panel_area, surface);
        let inner = crate::terminal::ratatui_bridge::draw_rounded_frame(grid, panel_area, border);
        for strip in crate::terminal::ratatui_bridge::frame_flush_areas(grid, panel_area) {
            context.dirty_areas.push_back(strip);
        }
        if !inner.is_empty() {
            let inner_rect = crate::terminal::ratatui_bridge::area_to_rect(inner);
            let [input_rect, list_rect] =
                Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).areas(inner_rect);
            self.draw_input(grid, area, input_rect, context);
            self.draw_list(grid, area, list_rect, context);
        }
        context.dirty_areas.push_back(panel_area);
        self.dirty = false;
    }

    /// Render the query input (`ratatui-textarea`) into a temporary
    /// buffer and blit it over the panel — the same bridge pattern the
    /// `LineGauge` and pager scrollbar use.
    fn draw_input(
        &self,
        grid: &mut CellBuffer,
        root: Area,
        input_rect: ratatui::layout::Rect,
        context: &Context,
    ) {
        if input_rect.width < 3 || input_rect.height < 3 {
            return;
        }
        let surface = crate::conf::value(context, "status.notification");
        let command_bar = crate::conf::value(context, "status.command_bar");
        let mut textarea = self.textarea();
        textarea.set_block(
            ratatui::widgets::Block::bordered()
                .border_set(crate::terminal::ratatui_bridge::border_set_for(
                    context.settings.terminal.ascii_drawing,
                ))
                .border_style(ratatui::style::Style::from(command_bar)),
        );
        // Render into an origin-anchored buffer of the area's size (the
        // blit reads origin-relative coordinates), then blit at the
        // area's screen position.
        let buf_rect = ratatui::layout::Rect::new(0, 0, input_rect.width, input_rect.height);
        let mut buf = ratatui::buffer::Buffer::empty(buf_rect);
        buf.set_style(buf_rect, ratatui::style::Style::from(surface));
        ratatui::widgets::Widget::render(&*textarea, buf_rect, &mut buf);
        crate::terminal::ratatui_bridge::blit_buffer_to_cellbuffer_at(
            &buf,
            grid,
            crate::terminal::ratatui_bridge::rect_to_area(input_rect, root),
        );
    }

    /// Render the item list (`ratatui` `List` widget) into a temporary
    /// buffer and blit it over the panel.
    fn draw_list(
        &self,
        grid: &mut CellBuffer,
        root: Area,
        list_rect: ratatui::layout::Rect,
        context: &Context,
    ) {
        if list_rect.width == 0 || list_rect.height == 0 {
            return;
        }
        let surface = crate::conf::value(context, "status.notification");
        let hints = crate::conf::value(context, "status.history.hints");
        let highlight = crate::conf::value(context, "widgets.options.highlighted");
        let surface_style = ratatui::style::Style::from(surface);
        let hints_style = ratatui::style::Style::from(hints);
        let matched_style = ratatui::style::Style::default()
            .add_modifier(ratatui::style::Modifier::BOLD | ratatui::style::Modifier::UNDERLINED);
        let rows: Vec<ratatui::widgets::ListItem<'static>> = self
            .items
            .iter()
            .map(|item| match item {
                PaletteItem::Section(label) => ratatui::widgets::ListItem::new(
                    ratatui::text::Line::styled((*label).to_string(), hints_style),
                ),
                PaletteItem::Entry {
                    text,
                    desc,
                    match_indices,
                    ..
                } => {
                    let display = text.trim_end();
                    let mut spans = highlight_spans(display, match_indices, matched_style);
                    if !desc.is_empty() {
                        spans.push(ratatui::text::Span::raw("  "));
                        spans.push(ratatui::text::Span::styled(desc.clone(), hints_style));
                    }
                    ratatui::widgets::ListItem::new(
                        ratatui::text::Line::from(spans).style(surface_style),
                    )
                }
            })
            .collect();
        let symbol: &'static str = if context.settings.terminal.ascii_drawing {
            "> "
        } else {
            "❯ "
        };
        let list = ratatui::widgets::List::new(rows)
            .highlight_symbol(symbol)
            .highlight_style(ratatui::style::Style::from(highlight));
        let mut state = ratatui::widgets::ListState::default();
        state.select(self.selection);
        // Origin-anchored buffer of the area's size (the blit reads
        // origin-relative coordinates), blitted at the area's screen
        // position.
        let buf_rect = ratatui::layout::Rect::new(0, 0, list_rect.width, list_rect.height);
        let mut buf = ratatui::buffer::Buffer::empty(buf_rect);
        buf.set_style(buf_rect, surface_style);
        ratatui::widgets::StatefulWidget::render(&list, buf_rect, &mut buf, &mut state);
        crate::terminal::ratatui_bridge::blit_buffer_to_cellbuffer_at(
            &buf,
            grid,
            crate::terminal::ratatui_bridge::rect_to_area(list_rect, root),
        );
    }
}

/// Split `text` into spans, applying `matched_style` to the char ranges
/// covered by `indices` (which must be sorted and deduplicated, as
/// `refresh` produces them).
fn highlight_spans(
    text: &str,
    indices: &[u32],
    matched_style: ratatui::style::Style,
) -> Vec<ratatui::text::Span<'static>> {
    let chars: Vec<char> = text.chars().collect();
    // Group consecutive indices into [start, end) runs.
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for &index in indices {
        let index = index as usize;
        if index >= chars.len() {
            continue;
        }
        match runs.last_mut() {
            Some(last) if last.1 == index => last.1 = index + 1,
            _ => runs.push((index, index + 1)),
        }
    }
    let mut spans = Vec::with_capacity(runs.len() * 2 + 1);
    let mut pos = 0;
    for (start, end) in runs {
        if start > pos {
            spans.push(ratatui::text::Span::raw(
                chars[pos..start].iter().collect::<String>(),
            ));
        }
        spans.push(ratatui::text::Span::styled(
            chars[start..end].iter().collect::<String>(),
            matched_style,
        ));
        pos = end;
    }
    if pos < chars.len() {
        spans.push(ratatui::text::Span::raw(
            chars[pos..].iter().collect::<String>(),
        ));
    }
    spans
}

/// Map a meli [`Key`] onto the backend-agnostic
/// [`ratatui_textarea::Input`]. Unmapped keys (mouse, F-keys, Insert,
/// Null, …) become `Input::default()` (`Key::Null`), which the textarea
/// ignores.
fn key_to_input(key: &Key) -> ratatui_textarea::Input {
    use ratatui_textarea::{Input, Key as TextAreaKey};
    let (key, ctrl, alt) = match key {
        Key::Char(c) => (TextAreaKey::Char(*c), false, false),
        Key::Alt(c) => (TextAreaKey::Char(*c), false, true),
        Key::Ctrl(c) => (TextAreaKey::Char(*c), true, false),
        Key::Backspace => (TextAreaKey::Backspace, false, false),
        Key::Delete => (TextAreaKey::Delete, false, false),
        Key::Home => (TextAreaKey::Home, false, false),
        Key::End => (TextAreaKey::End, false, false),
        Key::Left => (TextAreaKey::Left, false, false),
        Key::Right => (TextAreaKey::Right, false, false),
        Key::PageUp => (TextAreaKey::PageUp, false, false),
        Key::PageDown => (TextAreaKey::PageDown, false, false),
        _ => return Input::default(),
    };
    Input {
        key,
        ctrl,
        alt,
        shift: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn palette(history: &[&str]) -> CommandPalette {
        CommandPalette::new_for_tests(history.iter().map(|s| s.to_string()).collect())
    }

    fn entry_texts(palette: &CommandPalette) -> Vec<&str> {
        palette
            .items
            .iter()
            .filter_map(|item| match item {
                PaletteItem::Entry { text, .. } => Some(text.as_str()),
                PaletteItem::Section(_) => None,
            })
            .collect()
    }

    fn type_str(palette: &mut CommandPalette, context: &mut Context, input: &str) {
        for c in input.chars() {
            palette.process_key(&Key::Char(c), context);
        }
    }

    #[test]
    fn empty_query_splits_frequent_and_history() {
        let p = palette(&["quit", "go 1", "quit", "search foo"]);
        let labels: Vec<&str> = p
            .items
            .iter()
            .filter_map(|item| match item {
                PaletteItem::Section(label) => Some(*label),
                PaletteItem::Entry { .. } => None,
            })
            .collect();
        assert_eq!(labels, ["Frequent", "History"]);
        // Frequent: `quit` (used twice). History: newest-first dedup,
        // minus the frequent entries.
        assert_eq!(entry_texts(&p), ["quit", "search foo", "go 1"]);
        // The first entry after the Frequent label is selected.
        assert_eq!(p.selection, Some(1));
    }

    #[test]
    fn empty_query_without_history_lists_all_commands() {
        let p = palette(&[]);
        assert_eq!(p.items.first(), Some(&PaletteItem::Section("All commands")));
        let texts = entry_texts(&p);
        assert_eq!(
            texts.len(),
            crate::command::COMMAND_COMPLETION.len(),
            "every completion entry must be listed"
        );
        assert!(texts.contains(&"quit"));
    }

    #[test]
    fn query_matches_ranks_and_highlights() {
        let mut context = crate::golden::mock_context();
        let mut p = palette(&[]);
        type_str(&mut p, &mut context, "qui");
        let texts = entry_texts(&p);
        assert!(
            texts.contains(&"quit"),
            "query `qui` must match `quit`; got {texts:?}"
        );
        let quit_indices = p
            .items
            .iter()
            .find_map(|item| match item {
                PaletteItem::Entry {
                    text,
                    match_indices,
                    ..
                } if text == "quit" => Some(match_indices.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            quit_indices,
            vec![0, 1, 2],
            "`qui` matches the first three chars of `quit`"
        );
        let scores: Vec<u32> = p
            .items
            .iter()
            .filter_map(|item| match item {
                PaletteItem::Entry { score, .. } => Some(*score),
                PaletteItem::Section(_) => None,
            })
            .collect();
        assert!(
            scores.windows(2).all(|w| w[0] >= w[1]),
            "entries must be score-descending; got {scores:?}"
        );
    }

    #[test]
    fn tab_completes_to_selected_entry() {
        let mut context = crate::golden::mock_context();
        let mut p = palette(&[]);
        type_str(&mut p, &mut context, "qui");
        p.process_key(&Key::Char('\t'), &mut context);
        assert_eq!(p.textarea().lines(), vec!["quit".to_string()]);
    }

    #[test]
    fn enter_pushes_command_and_closes() {
        let mut context = crate::golden::mock_context();
        let mut p = palette(&[]);
        type_str(&mut p, &mut context, "qui");
        // The selected entry (top score) is `quit`.
        p.process_key(&Key::Char('\n'), &mut context);
        let replies = context.replies();
        assert!(
            replies
                .iter()
                .any(|event| matches!(event, UIEvent::Command(cmd) if cmd == "quit")),
            "Enter must queue Command(quit); got {replies:?}"
        );
        assert!(
            replies
                .iter()
                .any(|event| matches!(event, UIEvent::ChangeMode(UIMode::Normal))),
            "Enter must queue ChangeMode(Normal); got {replies:?}"
        );
        // History grew in memory (log_history=false → no file write).
        assert_eq!(p.history.last().map(String::as_str), Some("quit"));
        assert_eq!(
            p.textarea().lines(),
            vec![String::new()],
            "input must reset after Enter"
        );
    }
}
