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

//! Built-in compose body editor: an in-composer `ratatui-textarea` widget
//! used by [`Composer`] while `ViewMode::EditBody` is
//! active.
//!
//! Rendering follows the command palette's bridge pattern: the textarea is
//! drawn into an origin-anchored temporary ratatui `Buffer` and then blitted
//! into the meli `CellBuffer` at the body area's screen position. Unlike the
//! palette's input there is no `Block` border: the composer already draws
//! the body chrome and the focus column.

use super::*;
use ratatui_textarea::{CursorMove, TextArea, WrapMode};

/// Multi-line editor for the draft body, wrapping a `ratatui-textarea`
/// `TextArea`.
#[derive(Debug)]
pub(super) struct BodyEditor {
    /// `ratatui-textarea` keeps render caches in interior-mutable cells
    /// (`RefCell`/`Cell`), so `TextArea` is `!Sync`; the mutex keeps the
    /// component `Send + Sync` like every other `Component` field. The UI
    /// thread is the only accessor, so the lock is uncontended.
    textarea: std::sync::Mutex<TextArea<'static>>,
    /// Whether the last [`BodyEditor::process_event`] modified the text
    /// content (cursor movement / scrolling report `false`).
    modified: bool,
    dirty: bool,
    id: ComponentId,
}

impl BodyEditor {
    pub(super) fn new(context: &Context) -> Self {
        Self {
            textarea: std::sync::Mutex::new(Self::fresh_textarea(context, vec![String::new()])),
            modified: false,
            dirty: true,
            id: ComponentId::default(),
        }
    }

    /// A textarea preloaded with `lines` and configured for the body:
    /// no line numbers, word soft-wrap, the theme's default style and no
    /// whole-line cursor underline (the cursor itself already renders
    /// reversed, like the palette's input).
    fn fresh_textarea(context: &Context, lines: Vec<String>) -> TextArea<'static> {
        let mut textarea = TextArea::new(lines);
        textarea.remove_line_number();
        textarea.set_wrap_mode(WrapMode::Word);
        textarea.set_style(ratatui::style::Style::from(crate::conf::value(
            context,
            "theme_default",
        )));
        textarea.set_cursor_line_style(ratatui::style::Style::default());
        textarea
    }

    /// Lock the textarea (see the field docs). Unpoisoning on access: a
    /// panic while holding the lock must not brick every later draw.
    fn textarea(&self) -> std::sync::MutexGuard<'_, TextArea<'static>> {
        self.textarea.lock().unwrap_or_else(|err| err.into_inner())
    }

    /// Replace the editor contents with `body`, preserving the configured
    /// wrap/style, and put the cursor at the top (reply bodies are quoted
    /// below, so the caret lands above the quote).
    pub(super) fn load(&mut self, body: &str) {
        let mut lines: Vec<String> = body.split('\n').map(String::from).collect();
        if lines.is_empty() {
            // `TextArea` requires at least one line; an empty `body` already
            // yields `[""]`, so this is a belt-and-braces invariant guard.
            lines.push(String::new());
        }
        let mut textarea = self.textarea();
        // `TextArea` has no set-lines API: swapping in a fresh instance drops
        // the per-instance configuration, so carry it over.
        let style = textarea.style();
        let cursor_line_style = textarea.cursor_line_style();
        let wrap_mode = textarea.wrap_mode();
        *textarea = TextArea::new(lines);
        textarea.remove_line_number();
        textarea.set_style(style);
        textarea.set_cursor_line_style(cursor_line_style);
        textarea.set_wrap_mode(wrap_mode);
        textarea.move_cursor(CursorMove::Top);
        drop(textarea);
        self.modified = false;
    }

    /// The editor contents as logical lines joined by `\n` (soft wrap is
    /// display-only and never mutates the logical lines).
    pub(super) fn text(&self) -> String {
        self.textarea().lines().join("\n")
    }

    /// Whether the last input changed the text content.
    pub(super) fn take_modified(&mut self) -> bool {
        std::mem::take(&mut self.modified)
    }
}

impl std::fmt::Display for BodyEditor {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "body editor")
    }
}

impl Component for BodyEditor {
    fn draw(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        if area.width() == 0 || area.height() == 0 {
            return;
        }
        let theme_default = crate::conf::value(context, "theme_default");
        {
            let textarea = self.textarea();
            // Render into an origin-anchored buffer of the area's size (the
            // blit reads origin-relative coordinates), then blit at the
            // area's screen position. No `Block`: the composer already draws
            // the body chrome.
            let buf_rect =
                ratatui::layout::Rect::new(0, 0, area.width() as u16, area.height() as u16);
            let mut buf = ratatui::buffer::Buffer::empty(buf_rect);
            buf.set_style(buf_rect, ratatui::style::Style::from(theme_default));
            ratatui::widgets::Widget::render(&*textarea, buf_rect, &mut buf);
            crate::terminal::ratatui_bridge::blit_buffer_to_cellbuffer_at(&buf, grid, area);
        }
        context.dirty_areas.push_back(area);
        self.dirty = false;
    }

    fn process_event(&mut self, event: &mut UIEvent, _context: &mut Context) -> bool {
        let key = match event {
            UIEvent::Input(ref key) | UIEvent::InsertInput(ref key) => key,
            _ => return false,
        };
        match key {
            // The textarea's own `paste()` has no system clipboard; insert
            // the bracketed-paste payload directly.
            Key::Paste(s) => {
                self.textarea().insert_str(s);
                self.modified = true;
            }
            _ => {
                let modified = self
                    .textarea()
                    .input(crate::terminal::ratatui_bridge::key_to_textarea_input(key));
                self.modified |= modified;
            }
        }
        true
    }

    fn is_dirty(&self) -> bool {
        self.dirty
    }

    fn set_dirty(&mut self, value: bool) {
        self.dirty = value;
    }

    fn id(&self) -> ComponentId {
        self.id
    }

    fn shortcuts(&self, _context: &Context) -> ShortcutMaps {
        Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_context() -> Context {
        crate::golden::mock_context()
    }

    fn send(editor: &mut BodyEditor, context: &mut Context, key: Key) -> bool {
        let mut event = UIEvent::Input(key);
        let consumed = editor.process_event(&mut event, context);
        let _ = context.replies();
        consumed
    }

    #[test]
    fn load_and_text_roundtrip_multiline() {
        let mut context = new_context();
        let mut editor = BodyEditor::new(&context);
        editor.load("first line\nsecond line\nthird");
        assert_eq!(editor.text(), "first line\nsecond line\nthird");
        let _ = context.replies();
    }

    #[test]
    fn load_empty_does_not_panic_and_text_is_empty() {
        let mut context = new_context();
        let mut editor = BodyEditor::new(&context);
        editor.load("");
        assert_eq!(editor.text(), "");
        // Loading again with content, then back to empty, stays stable.
        editor.load("x");
        assert_eq!(editor.text(), "x");
        editor.load("");
        assert_eq!(editor.text(), "");
        let _ = context.replies();
    }

    #[test]
    fn char_input_is_modified_arrow_is_not() {
        let mut context = new_context();
        let mut editor = BodyEditor::new(&context);
        editor.load("");
        assert!(send(&mut editor, &mut context, Key::Char('a')));
        assert!(editor.take_modified(), "char input must report modified");
        assert!(!editor.take_modified(), "take_modified must reset");
        assert!(send(&mut editor, &mut context, Key::Left));
        assert!(send(&mut editor, &mut context, Key::Down));
        assert!(
            !editor.take_modified(),
            "cursor movement must not report modified"
        );
    }

    #[test]
    fn tab_char_inserts_indent() {
        let mut context = new_context();
        let mut editor = BodyEditor::new(&context);
        editor.load("");
        assert!(send(&mut editor, &mut context, Key::Char('\t')));
        assert!(editor.take_modified());
        // Default tab length is 4 spaces, not a literal tab.
        assert_eq!(editor.text(), "    ");
    }

    #[test]
    fn paste_inserts_multiline_text() {
        let mut context = new_context();
        let mut editor = BodyEditor::new(&context);
        editor.load("");
        let mut event = UIEvent::Input(Key::Paste("one\ntwo".to_string()));
        assert!(editor.process_event(&mut event, &mut context));
        let _ = context.replies();
        assert!(editor.take_modified());
        assert_eq!(editor.text(), "one\ntwo");
    }

    #[test]
    fn non_input_events_are_not_consumed() {
        let mut context = new_context();
        let mut editor = BodyEditor::new(&context);
        let mut event = UIEvent::Resize;
        assert!(!editor.process_event(&mut event, &mut context));
        let _ = context.replies();
    }
}
