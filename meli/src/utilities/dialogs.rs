/*
 * meli
 *
 * Copyright 2020 Manos Pitsidianakis
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

use super::*;

const OK: &str = "OK";
const CANCEL: &str = "Cancel";
const CANCEL_OFFSET: usize = "OK    ".len();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SelectorCursor {
    Unfocused,
    /// Cursor is at an entry
    Entry(usize),
    /// Cursor is located on the Ok button
    Ok,
    /// Cursor is located on the Cancel button
    Cancel,
}

/// Shows a little window with options for user to select.
///
/// Instantiate with `Selector::new()`. Set `single_only` to true if user should
/// only choose one of the options. After passing input events to this
/// component, check `Selector::is_done` to see if the user has finalised their
/// choices. Collect the choices by consuming the `Selector` with
/// `Selector::collect()`
pub struct Selector<
    T: 'static + PartialEq + std::fmt::Debug + Clone + Sync + Send,
    F: 'static + Sync + Send,
> {
    /// allow only one selection
    single_only: bool,
    entries: Vec<(T, bool)>,
    entry_titles: Vec<String>,
    theme_default: ThemeAttribute,

    cursor: SelectorCursor,
    scroll_x_cursor: usize,
    movement: Option<PageMovement>,
    title: String,
    content: Screen<Virtual>,
    initialized: bool,
    /// If `true`, user has finished their selection
    done: bool,
    done_fn: F,
    /// Invoked whenever the highlighted entry changes (arrow-key
    /// navigation), with the newly highlighted identifier. Used for live
    /// previews: the theme picker applies each theme as the cursor moves,
    /// so the whole UI reflects the selection without further keys.
    cursor_callback: Option<SelectorCursorCallback<T>>,
    dirty: bool,
    id: ComponentId,
}

/// Live-preview hook type for [`Selector`]: receives the newly
/// highlighted entry and the context to push preview events onto.
pub type SelectorCursorCallback<T> = Box<dyn Fn(&T, &mut Context) + Send + Sync>;

pub type UIConfirmationDialog = Selector<
    bool,
    Option<Box<dyn FnOnce(ComponentId, bool) -> Option<UIEvent> + 'static + Sync + Send>>,
>;

pub type UIDialog<T> = Selector<
    T,
    Option<Box<dyn FnOnce(ComponentId, &[T]) -> Option<UIEvent> + 'static + Sync + Send>>,
>;

impl<T: 'static + PartialEq + std::fmt::Debug + Clone + Sync + Send, F: 'static + Sync + Send>
    std::fmt::Debug for Selector<T, F>
{
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        std::fmt::Display::fmt("Selector", f)
    }
}

impl<T: 'static + PartialEq + std::fmt::Debug + Clone + Sync + Send, F: 'static + Sync + Send>
    std::fmt::Display for Selector<T, F>
{
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        std::fmt::Display::fmt("Selector", f)
    }
}

impl<T: 'static + PartialEq + std::fmt::Debug + Clone + Sync + Send, F: 'static + Sync + Send>
    PartialEq for Selector<T, F>
{
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries
    }
}

impl<T: 'static + PartialEq + std::fmt::Debug + Clone + Sync + Send> Component for UIDialog<T> {
    fn draw(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        Selector::draw(self, grid, area, context);
    }

    fn process_event(&mut self, event: &mut UIEvent, context: &mut Context) -> bool {
        if let UIEvent::ConfigReload { old_settings: _ } = event {
            // Theme preview (toggle_theme) broadcasts ConfigReload on every
            // arrow key. `set_dirty` resets `initialized`, so the next draw
            // rebuilds the content grid with the new palette - that rebuild
            // is what recolors the entry list, because entry colors are
            // baked into the content grid at initialize() time. It is cheap
            // for a single dialog; the preview flicker was fixed on the
            // compositing side (state.rs flushes the overlay grid directly,
            // never an intermediate dialog-less frame).
            self.theme_default = crate::conf::value(context, "theme_default");
            self.set_dirty(true);
            return false;
        }

        let shortcuts = self.shortcuts(context);
        // A dialog with no entries has nothing to select: keep the cursor off
        // the entry list so the `Entry(c)` arms below cannot index it. An
        // empty entry list (e.g. "select recipients" with no candidate
        // addresses) used to panic on Enter.
        if self.entries.is_empty() && matches!(self.cursor, SelectorCursor::Entry(_)) {
            self.cursor = SelectorCursor::Ok;
        }
        match (event, self.cursor) {
            (UIEvent::Input(Key::Char('\n')), _) if self.single_only => {
                /* User can only select one entry, so Enter key finalises the selection */
                self.done = true;
                if let Some(event) = self.done() {
                    context.replies.push_back(event);
                    self.unrealize(context);
                }
                return true;
            }
            (UIEvent::Input(Key::Char('\n')), SelectorCursor::Entry(c)) if !self.single_only => {
                /* User can select multiple entries, so Enter key toggles the entry under the
                 * cursor */
                if let Some(e) = self.entries.get_mut(c) {
                    e.1 = !e.1;
                }
                self.dirty = true;
                self.initialized = false;
                return true;
            }
            (UIEvent::Input(Key::Char('\n')), SelectorCursor::Ok) if !self.single_only => {
                self.done = true;
                if let Some(event) = self.done() {
                    context.replies.push_back(event);
                    self.unrealize(context);
                }
                return true;
            }
            (UIEvent::Input(key), _)
                if *key == Key::Esc || shortcut!(key == shortcuts[Shortcuts::GENERAL]["quit"]) =>
            {
                // Layered quit: the dialog is the focused surface, so the
                // quit binding (`q`/`Esc` by default) closes the dialog
                // and is consumed here. Returning `false` would leak the
                // key to the components below and `StatusBar` would turn
                // it into `UIEvent::Exit`, quitting the application.
                for e in self.entries.iter_mut() {
                    e.1 = false;
                }
                if !self.done {
                    self.unrealize(context);
                }
                self.done = true;
                _ = self.done();
                self.cancel(context);
                self.set_dirty(true);
                return true;
            }
            (UIEvent::Input(Key::Char('\n')), SelectorCursor::Cancel) if !self.single_only => {
                for e in self.entries.iter_mut() {
                    e.1 = false;
                }
                self.done = true;
                if let Some(event) = self.done() {
                    context.replies.push_back(event);
                    self.unrealize(context);
                }
                return true;
            }
            (UIEvent::Input(ref key), SelectorCursor::Unfocused)
                if !self.entries.is_empty()
                    && shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_down"]) =>
            {
                if self.single_only {
                    self.entries[0].1 = true;
                }
                self.cursor = SelectorCursor::Entry(0);
                self.fire_cursor_callback(context);
                self.dirty = true;
                self.initialized = false;
                return true;
            }
            (UIEvent::Input(ref key), SelectorCursor::Entry(c))
                if shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_up"]) && c > 0 =>
            {
                if self.single_only {
                    // Redraw selection
                    self.entries[c].1 = false;
                    self.entries[c - 1].1 = true;
                }
                self.cursor = SelectorCursor::Entry(c - 1);
                self.fire_cursor_callback(context);
                self.dirty = true;
                self.initialized = false;
                return true;
            }
            (UIEvent::Input(ref key), SelectorCursor::Ok)
            | (UIEvent::Input(ref key), SelectorCursor::Cancel)
                if !self.entries.is_empty()
                    && shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_up"]) =>
            {
                let c = self.entries.len().saturating_sub(1);
                self.cursor = SelectorCursor::Entry(c);
                self.fire_cursor_callback(context);
                self.dirty = true;
                self.initialized = false;
                return true;
            }
            (UIEvent::Input(ref key), SelectorCursor::Entry(c))
                if c < self.entries.len().saturating_sub(1)
                    && shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_down"]) =>
            {
                if self.single_only {
                    // Redraw selection
                    self.entries[c].1 = false;
                    self.entries[c + 1].1 = true;
                }
                self.cursor = SelectorCursor::Entry(c + 1);
                self.fire_cursor_callback(context);
                self.dirty = true;
                self.initialized = false;
                return true;
            }
            (UIEvent::Input(ref key), SelectorCursor::Entry(_))
                if !self.single_only
                    && shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_down"]) =>
            {
                self.cursor = SelectorCursor::Ok;
                self.set_dirty(true);
                self.initialized = false;
                return true;
            }
            (UIEvent::Input(ref key), SelectorCursor::Ok)
                if shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_right"]) =>
            {
                self.cursor = SelectorCursor::Cancel;
                self.set_dirty(true);
                self.initialized = false;
                return true;
            }
            (UIEvent::Input(ref key), SelectorCursor::Cancel)
                if shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_left"]) =>
            {
                self.cursor = SelectorCursor::Ok;
                self.set_dirty(true);
                self.initialized = false;
                return true;
            }
            (UIEvent::Input(ref key), _)
                if shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_left"]) =>
            {
                self.movement = Some(PageMovement::Left(1));
                self.set_dirty(true);
                self.initialized = false;
                return true;
            }
            (UIEvent::Input(ref key), _)
                if shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_right"]) =>
            {
                self.movement = Some(PageMovement::Right(1));
                self.set_dirty(true);
                self.initialized = false;
                return true;
            }
            (UIEvent::Input(ref key), _)
                if shortcut!(key == shortcuts[Shortcuts::GENERAL]["prev_page"]) =>
            {
                self.movement = Some(PageMovement::PageUp(1));
                self.set_dirty(true);
                self.initialized = false;
                return true;
            }
            (UIEvent::Input(ref key), _)
                if shortcut!(key == shortcuts[Shortcuts::GENERAL]["next_page"]) =>
            {
                self.movement = Some(PageMovement::PageDown(1));
                self.set_dirty(true);
                self.initialized = false;
                return true;
            }
            (UIEvent::Input(ref key), _)
                if shortcut!(key == shortcuts[Shortcuts::GENERAL]["home_page"]) =>
            {
                self.movement = Some(PageMovement::Home);
                self.set_dirty(true);
                self.initialized = false;
                return true;
            }
            (UIEvent::Input(ref key), _)
                if shortcut!(key == shortcuts[Shortcuts::GENERAL]["end_page"]) =>
            {
                self.movement = Some(PageMovement::End);
                self.set_dirty(true);
                self.initialized = false;
                return true;
            }
            (UIEvent::Input(ref key), _)
                if shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_up"])
                    || shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_down"]) =>
            {
                return true;
            }
            _ => {}
        }

        false
    }

    fn shortcuts(&self, context: &Context) -> ShortcutMaps {
        let mut map = ShortcutMaps::default();
        map.insert(
            Shortcuts::GENERAL,
            context.settings.shortcuts.general.key_values(),
        );
        map
    }

    fn is_dirty(&self) -> bool {
        self.dirty
    }

    fn set_dirty(&mut self, value: bool) {
        self.dirty = value;
        self.initialized = false;
    }

    fn id(&self) -> ComponentId {
        self.id
    }
}

impl Component for UIConfirmationDialog {
    fn draw(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        Selector::draw(self, grid, area, context);
    }

    fn process_event(&mut self, event: &mut UIEvent, context: &mut Context) -> bool {
        if let UIEvent::ConfigReload { old_settings: _ } = event {
            self.initialise(context);
            self.set_dirty(true);
            return false;
        }

        let shortcuts = self.shortcuts(context);
        // A dialog with no entries has nothing to select: keep the cursor off
        // the entry list so the `Entry(c)` arms below cannot index it. An
        // empty entry list (e.g. "select recipients" with no candidate
        // addresses) used to panic on Enter.
        if self.entries.is_empty() && matches!(self.cursor, SelectorCursor::Entry(_)) {
            self.cursor = SelectorCursor::Ok;
        }
        match (event, self.cursor) {
            (UIEvent::Input(Key::Char('\n')), _) if self.single_only => {
                /* User can only select one entry, so Enter key finalises the selection */
                self.done = true;
                if let Some(event) = self.done() {
                    context.replies.push_back(event);
                    self.unrealize(context);
                }
                self.set_dirty(true);
                self.initialized = false;
                return true;
            }
            (UIEvent::Input(Key::Char('\n')), SelectorCursor::Entry(c)) if !self.single_only => {
                /* User can select multiple entries, so Enter key toggles the entry under the
                 * cursor */
                if let Some(e) = self.entries.get_mut(c) {
                    e.1 = !e.1;
                }
                self.set_dirty(true);
                self.initialized = false;
                return true;
            }
            (UIEvent::Input(Key::Char('\n')), SelectorCursor::Ok) if !self.single_only => {
                self.done = true;
                if let Some(event) = self.done() {
                    context.replies.push_back(event);
                    self.unrealize(context);
                }
                self.set_dirty(true);
                self.initialized = false;
                return true;
            }
            (UIEvent::Input(key), _)
                if *key == Key::Esc || shortcut!(key == shortcuts[Shortcuts::GENERAL]["quit"]) =>
            {
                // Layered quit: same as `UIDialog` above - the quit
                // binding (`q`/`Esc` by default) closes the dialog and is
                // consumed, so it cannot leak to the application-level
                // exit path.
                for e in self.entries.iter_mut() {
                    e.1 = false;
                }
                if !self.done {
                    self.unrealize(context);
                }
                self.done = true;
                _ = self.done();
                self.cancel(context);
                self.set_dirty(true);
                self.initialized = false;
                return true;
            }
            (UIEvent::Input(Key::Char('\n')), SelectorCursor::Cancel) if !self.single_only => {
                for e in self.entries.iter_mut() {
                    e.1 = false;
                }
                self.done = true;
                if let Some(event) = self.done() {
                    context.replies.push_back(event);
                    self.unrealize(context);
                }
                self.set_dirty(true);
                self.initialized = false;
                return true;
            }
            (UIEvent::Input(ref key), SelectorCursor::Entry(c))
                if shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_up"]) && c > 0 =>
            {
                if self.single_only {
                    // Redraw selection
                    self.entries[c].1 = false;
                    self.entries[c - 1].1 = true;
                }
                self.cursor = SelectorCursor::Entry(c - 1);
                self.set_dirty(true);
                self.initialized = false;
                return true;
            }
            (UIEvent::Input(ref key), SelectorCursor::Ok)
            | (UIEvent::Input(ref key), SelectorCursor::Cancel)
                if !self.entries.is_empty()
                    && shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_up"]) =>
            {
                let c = self.entries.len().saturating_sub(1);
                self.cursor = SelectorCursor::Entry(c);
                self.set_dirty(true);
                self.initialized = false;
                return true;
            }
            (UIEvent::Input(ref key), SelectorCursor::Unfocused)
                if !self.entries.is_empty()
                    && shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_down"]) =>
            {
                if self.single_only {
                    self.entries[0].1 = true;
                }
                self.cursor = SelectorCursor::Entry(0);
                self.set_dirty(true);
                self.initialized = false;
                return true;
            }
            (UIEvent::Input(ref key), SelectorCursor::Entry(c))
                if c < self.entries.len().saturating_sub(1)
                    && shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_down"]) =>
            {
                if self.single_only {
                    // Redraw selection
                    self.entries[c].1 = false;
                    self.entries[c + 1].1 = true;
                }
                self.cursor = SelectorCursor::Entry(c + 1);
                self.set_dirty(true);
                self.initialized = false;
                return true;
            }
            (UIEvent::Input(ref key), SelectorCursor::Entry(_))
                if !self.single_only
                    && shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_down"]) =>
            {
                self.cursor = SelectorCursor::Ok;
                self.set_dirty(true);
                self.initialized = false;
                return true;
            }
            (UIEvent::Input(ref key), SelectorCursor::Ok)
                if shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_right"]) =>
            {
                self.cursor = SelectorCursor::Cancel;
                self.set_dirty(true);
                self.initialized = false;
                return true;
            }
            (UIEvent::Input(ref key), SelectorCursor::Cancel)
                if shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_left"]) =>
            {
                self.cursor = SelectorCursor::Ok;
                self.set_dirty(true);
                self.initialized = false;
                return true;
            }
            (UIEvent::Input(ref key), _)
                if shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_left"])
                    || shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_right"])
                    || shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_up"])
                    || shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_down"]) =>
            {
                return true
            }
            _ => {}
        }

        false
    }

    fn shortcuts(&self, context: &Context) -> ShortcutMaps {
        let mut map = ShortcutMaps::default();
        map.insert(
            Shortcuts::GENERAL,
            context.settings.shortcuts.general.key_values(),
        );
        map
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
}

impl<T: PartialEq + std::fmt::Debug + Clone + Sync + Send, F: 'static + Sync + Send>
    Selector<T, F>
{
    pub fn new(
        title: impl Into<String>,
        mut entries: Vec<(T, String)>,
        single_only: bool,
        done_fn: F,
        context: &Context,
    ) -> Self {
        let title = title.into();
        let entry_titles = entries
            .iter_mut()
            .map(|(_id, ref mut title)| std::mem::take(title))
            .collect::<Vec<String>>();
        let mut identifiers: Vec<(T, bool)> =
            entries.into_iter().map(|(id, _)| (id, false)).collect();
        if single_only {
            /* set default option */
            identifiers[0].1 = true;
        }

        let theme_default = crate::conf::value(context, "theme_default");
        let mut ret = Self {
            single_only,
            entries: identifiers,
            entry_titles,
            cursor: SelectorCursor::Unfocused,
            scroll_x_cursor: 0,
            movement: None,
            title,
            content: Screen::<Virtual>::new(theme_default),
            initialized: false,
            done: false,
            done_fn,
            cursor_callback: None,
            dirty: true,
            theme_default,
            id: ComponentId::default(),
        };
        ret.initialise(context);
        ret
    }

    fn initialise(&mut self, context: &Context) {
        self.theme_default = crate::conf::value(context, "theme_default");
    }

    /// Set the live-preview hook: invoked with the newly highlighted
    /// entry whenever arrow-key navigation moves the cursor. See
    /// [`Selector::cursor_callback`].
    pub fn set_cursor_callback(&mut self, cb: Option<SelectorCursorCallback<T>>) -> &mut Self {
        self.cursor_callback = cb;
        self
    }

    /// Replace the completion callback invoked when the selection is
    /// finalised (`Enter`; the returned event is delivered). On cancel
    /// (the quit binding, `q`/`Esc` by default) it is still invoked once
    /// with the current selection (in single-selection mode that is the
    /// entry under the cursor), but its result is discarded - cancel is
    /// pure exit.
    /// Mirrors [`Selector::set_cursor_callback`] for the `done_fn` field.
    pub fn set_done_fn(&mut self, f: F) -> &mut Self {
        self.done_fn = f;
        self
    }

    /// Place the cursor on the entry identified by `id` (single-selection
    /// mode also updates the rendering selection). No-op when `id` is not
    /// in the list.
    pub fn set_cursor_to(&mut self, id: &T) -> &mut Self {
        if let Some(pos) = self.entries.iter().position(|(e, _)| e == id) {
            if self.single_only {
                for (i, e) in self.entries.iter_mut().enumerate() {
                    e.1 = i == pos;
                }
            }
            self.cursor = SelectorCursor::Entry(pos);
            self.initialized = false;
            self.dirty = true;
        }
        self
    }

    /// Invoke the live-preview hook (if any) with the currently
    /// highlighted entry. The hook pushes any resulting events onto
    /// `context.replies` itself - the theme picker uses this to apply
    /// each highlighted theme for an immediate preview.
    fn fire_cursor_callback(&self, context: &mut Context) {
        if let (Some(cb), Some((id, _))) = (
            self.cursor_callback.as_ref(),
            self.entries.get(match self.cursor {
                SelectorCursor::Entry(i) => i,
                _ => 0,
            }),
        ) {
            cb(id, context);
        }
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    pub fn collect(self) -> Vec<T> {
        self.entries
            .into_iter()
            .filter(|v| v.1)
            .map(|(id, _)| id)
            .collect()
    }

    fn initialize(&mut self, context: &Context) {
        let mut highlighted_attrs = crate::conf::value(context, "widgets.options.highlighted");
        if !context.settings.terminal.use_color() {
            highlighted_attrs.attrs |= Attr::REVERSE;
        }
        /* Dialogs are focused floating surfaces: accent title from the tab
         * focus vocabulary, a raised-panel background (status.notification)
         * distinct from the content underneath, and dimmed secondary text. */
        let focused_attrs = crate::conf::value(context, "tab.focused");
        let unfocused_attrs = crate::conf::value(context, "tab.unfocused");
        let surface = crate::conf::value(context, "status.notification");
        let entry_attrs = ThemeAttribute {
            bg: surface.bg,
            ..self.theme_default
        };
        let shortcuts = context.settings.shortcuts.general.key_values();
        let navigate_help_string = format!(
            "Navigate options with {} to go down, {} to go up, select with {}, quit with {}",
            shortcuts["scroll_down"],
            shortcuts["scroll_up"],
            Key::Char('\n'),
            shortcuts["quit"]
        );
        let width = std::cmp::max(
            self.entry_titles
                .iter()
                .map(|e| melib::text::TextProcessing::grapheme_width(e.as_str()))
                .max()
                .unwrap_or(0)
                + 3,
            std::cmp::max(
                melib::text::TextProcessing::grapheme_width(self.title.as_str()),
                melib::text::TextProcessing::grapheme_width(navigate_help_string.as_str()),
            ) + 3,
        ) + 3;
        let height = self.entries.len()
            // padding
            + 3
            // buttons row
            + if self.single_only { 3 } else { 5 };
        if !self.content.resize_with_context(width, height, context) {
            self.dirty = false;
            return;
        }
        // Raised panel background for the dialog body.
        let content_area = self.content.area();
        self.content
            .grid_mut()
            .clear_area(content_area, entry_attrs);

        let inner_area = self.content.area();
        let (_, y) = self.content.grid_mut().write_string(
            &self.title,
            focused_attrs.fg,
            surface.bg,
            focused_attrs.attrs | Attr::BOLD,
            inner_area.skip_cols(2),
            None,
            None,
        );

        let y = self
            .content
            .grid_mut()
            .write_string(
                &navigate_help_string,
                unfocused_attrs.fg,
                surface.bg,
                unfocused_attrs.attrs | Attr::ITALICS,
                inner_area.skip_cols(2).skip_rows(y + 2),
                None,
                None,
            )
            .1
            + y
            + 2;

        let inner_area = inner_area.skip_cols(1).skip_rows(y + 2);

        /* Extra room for buttons Okay/Cancel */
        if self.single_only {
            for (i, e) in self.entry_titles.iter().enumerate() {
                let attr = if matches!(self.cursor, SelectorCursor::Entry(e) if e == i) {
                    highlighted_attrs
                } else {
                    entry_attrs
                };
                self.content.grid_mut().write_string(
                    e,
                    attr.fg,
                    attr.bg,
                    attr.attrs,
                    inner_area.nth_row(i),
                    None,
                    None,
                );
            }
        } else {
            for (i, e) in self.entry_titles.iter().enumerate() {
                let attr = if matches!(self.cursor, SelectorCursor::Entry(e) if e == i) {
                    highlighted_attrs
                } else {
                    entry_attrs
                };
                self.content.grid_mut().write_string(
                    &format!("[{}] {}", if self.entries[i].1 { "x" } else { " " }, e),
                    attr.fg,
                    attr.bg,
                    attr.attrs,
                    inner_area.nth_row(i),
                    None,
                    None,
                );
            }
            let inner_area = inner_area.nth_row(self.entry_titles.len() + 2).skip_cols(2);
            let attr = if matches!(self.cursor, SelectorCursor::Ok) {
                highlighted_attrs
            } else {
                entry_attrs
            };
            let (x, y) = self.content.grid_mut().write_string(
                OK,
                attr.fg,
                attr.bg,
                attr.attrs | Attr::BOLD,
                inner_area,
                None,
                None,
            );
            let attr = if matches!(self.cursor, SelectorCursor::Cancel) {
                highlighted_attrs
            } else {
                entry_attrs
            };
            self.content.grid_mut().write_string(
                CANCEL,
                attr.fg,
                attr.bg,
                attr.attrs,
                inner_area.skip(CANCEL_OFFSET + x, y),
                None,
                None,
            );
        }
        self.initialized = true;
    }

    fn draw(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        if !self.initialized {
            self.initialize(context);
        }
        let (width, height) = self.content.area().size();
        /* Dialog centering via ratatui Layout (bridge helper): identical to
         * the previous `align_inside(.., Center, Center)` for every size. */
        let dialog_area = crate::terminal::ratatui_bridge::center_inside_via_layout(
            area,
            (width + 2, height + 2),
        );
        /* Rounded frame in the focus vocabulary: a dialog is, by definition,
         * the focused surface while it is shown, so its border uses
         * tab.focused over the raised overlay panel background. */
        let mut border_attrs = crate::conf::value(context, "tab.focused");
        border_attrs.bg = crate::conf::value(context, "status.notification").bg;
        if !context.settings.terminal.use_color() {
            border_attrs.attrs |= Attr::REVERSE;
        }
        let inner_area =
            crate::terminal::ratatui_bridge::draw_rounded_frame(grid, dialog_area, border_attrs);
        for frame_area in crate::terminal::ratatui_bridge::frame_flush_areas(grid, dialog_area) {
            context.dirty_areas.push_back(frame_area);
        }
        let rows = inner_area.height();
        if let Some(mvm) = self.movement.take() {
            match mvm {
                PageMovement::Up(_) | PageMovement::Down(_) => {}
                PageMovement::Right(amount) => {
                    self.scroll_x_cursor = self.scroll_x_cursor.saturating_add(amount);
                }
                PageMovement::Left(amount) => {
                    self.scroll_x_cursor = self.scroll_x_cursor.saturating_sub(amount);
                }
                PageMovement::PageUp(multiplier) => match self.cursor {
                    SelectorCursor::Unfocused => {
                        self.cursor = SelectorCursor::Entry(0);
                        self.initialize(context);
                    }
                    SelectorCursor::Entry(c) => {
                        self.cursor = SelectorCursor::Entry(c.saturating_sub(multiplier * rows));
                        self.initialize(context);
                    }
                    SelectorCursor::Ok | SelectorCursor::Cancel
                        if !self.entry_titles.is_empty() =>
                    {
                        self.cursor = SelectorCursor::Entry(
                            self.entry_titles.len().saturating_sub(multiplier * rows),
                        );
                        self.initialize(context);
                    }
                    SelectorCursor::Ok | SelectorCursor::Cancel => {}
                },
                PageMovement::PageDown(multiplier) => match self.cursor {
                    SelectorCursor::Unfocused => {
                        self.cursor = SelectorCursor::Entry(
                            self.entry_titles
                                .len()
                                .saturating_sub(1)
                                .min(multiplier * rows),
                        );
                        self.initialize(context);
                    }
                    SelectorCursor::Entry(c)
                        if c.saturating_add(multiplier * rows) < self.entry_titles.len()
                            && !self.entry_titles.is_empty() =>
                    {
                        self.cursor = SelectorCursor::Entry(
                            self.entry_titles
                                .len()
                                .saturating_sub(1)
                                .min(c.saturating_add(multiplier * rows)),
                        );
                        self.initialize(context);
                    }
                    SelectorCursor::Entry(_) => {
                        self.cursor = SelectorCursor::Ok;
                        self.initialize(context);
                    }
                    SelectorCursor::Ok | SelectorCursor::Cancel => {}
                },
                PageMovement::Home if !self.entry_titles.is_empty() => {
                    self.cursor = SelectorCursor::Entry(0);
                    self.initialize(context);
                }
                PageMovement::End
                    if matches!(self.cursor, SelectorCursor::Ok | SelectorCursor::Cancel) => {}
                PageMovement::End
                    if !matches!(self.cursor, SelectorCursor::Entry(c) if c +1 == self.entry_titles.len())
                        && !self.entry_titles.is_empty() =>
                {
                    self.cursor = SelectorCursor::Entry(self.entry_titles.len().saturating_sub(1));
                    self.initialize(context);
                }
                PageMovement::Home | PageMovement::End => {}
            }
        }
        let skip_rows = match self.cursor {
            SelectorCursor::Unfocused => 0,
            SelectorCursor::Entry(e) if e >= rows => e.min(height.saturating_sub(rows)),
            SelectorCursor::Entry(_) => 0,
            SelectorCursor::Ok | SelectorCursor::Cancel => height.saturating_sub(rows),
        };

        self.scroll_x_cursor = self
            .scroll_x_cursor
            .min(width.saturating_sub(inner_area.width()));
        grid.copy_area(
            self.content.grid(),
            inner_area,
            self.content
                .area()
                .skip_cols(self.scroll_x_cursor)
                .skip_rows(skip_rows),
        );

        if height > dialog_area.height() {
            let inner_area = inner_area.skip_rows(1);
            ScrollBar::default().set_show_arrows(true).draw(
                grid,
                inner_area.nth_col(inner_area.width().saturating_sub(1)),
                context,
                // position
                skip_rows,
                // visible_rows
                inner_area.height(),
                // length
                height,
            );
        }
        if width > dialog_area.width() {
            let inner_area = inner_area.skip_cols(1);
            ScrollBar::default().set_show_arrows(true).draw_horizontal(
                grid,
                inner_area.nth_row(inner_area.height().saturating_sub(1)),
                context,
                // position
                self.scroll_x_cursor,
                // visible_cols
                inner_area.width(),
                // length
                width,
            );
        }
        context.dirty_areas.push_back(dialog_area);
        self.dirty = false;
    }
}

impl<T: 'static + PartialEq + std::fmt::Debug + Clone + Sync + Send> UIDialog<T> {
    fn done(&mut self) -> Option<UIEvent> {
        let Self {
            ref mut done_fn,
            ref mut entries,
            ..
        } = self;
        let (cursor, single_only, id) = (self.cursor, self.single_only, self.id);
        done_fn.take().and_then(|done_fn| {
            let selection: Vec<T> = if single_only {
                // In single-selection mode the cursor *is* the selection;
                // the `selected` flags are only a rendering aid and can
                // lag the cursor when keys arrive between draws.
                match cursor {
                    SelectorCursor::Entry(i) => entries
                        .get(i)
                        .map(|(id, _)| id.clone())
                        .into_iter()
                        .collect(),
                    _ => Vec::new(),
                }
            } else {
                entries
                    .iter()
                    .filter(|v| v.1)
                    .map(|(id, _)| id)
                    .cloned()
                    .collect()
            };
            done_fn(id, selection.as_slice())
        })
    }

    fn cancel(&self, context: &mut Context) {
        context.unrealized.insert(self.id());
        context
            .replies
            .push_back(UIEvent::ComponentUnrealize(self.id()));
    }
}

impl UIConfirmationDialog {
    fn done(&mut self) -> Option<UIEvent> {
        let Self {
            ref mut done_fn,
            ref mut entries,
            ref id,
            ..
        } = self;
        done_fn.take().and_then(|done_fn| {
            done_fn(
                *id,
                entries
                    .iter()
                    .filter(|v| v.1)
                    .map(|(id, _)| id)
                    .cloned()
                    .any(std::convert::identity),
            )
        })
    }

    fn cancel(&self, context: &mut Context) {
        context.unrealized.insert(self.id());
        context
            .replies
            .push_back(UIEvent::ComponentUnrealize(self.id()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A selector with no entries has nothing to select: movement keys and
    /// Enter must not index the empty entry list (an empty "select
    /// recipients"/tag list used to panic here).
    #[test]
    fn empty_selector_does_not_panic() {
        let mut ctx = crate::golden::mock_context();
        let mut dialog = UIConfirmationDialog::new("no choices", Vec::new(), false, None, &ctx);
        for key in [
            Key::Down,
            Key::Up,
            Key::PageDown,
            Key::PageUp,
            Key::Char('\n'),
            Key::Char(' '),
            Key::Esc,
        ] {
            let mut event = UIEvent::Input(key);
            let _ = dialog.process_event(&mut event, &mut ctx);
        }

        // Same for the multi-choice `UIDialog`.
        let mut dialog: UIDialog<char> = UIDialog::new(
            "no choices",
            Vec::<(char, String)>::new(),
            false,
            None,
            &ctx,
        );
        for key in [Key::Down, Key::Up, Key::Char('\n'), Key::Esc] {
            let mut event = UIEvent::Input(key);
            let _ = dialog.process_event(&mut event, &mut ctx);
        }
    }

    /// Repro for the theme-picker left-edge bleed: after `Selector::draw`
    // paints onto a grid that already contains underlying listing text
    // (exactly what the state.rs overlay compositing does), every cell of
    // the dialog area must be covered by the dialog - no listing text may
    // remain inside dialog_area, in particular the left frame column.
    #[test]
    fn selector_draw_covers_dialog_area() {
        let mut ctx = crate::golden::mock_context();
        let entries: Vec<(String, String)> = (0..60)
            .map(|i| (format!("theme-{i}"), format!("theme-{i} (built-in)")))
            .collect();
        let mut dialog: UIDialog<String> =
            UIDialog::new("theme", entries.clone(), true, None, &ctx);
        dialog.set_cursor_to(&entries[0].0);

        for (cols, rows) in [(100usize, 30usize), (60, 20), (200, 50)] {
            let mut screen = crate::terminal::Screen::<crate::terminal::Virtual>::new(
                crate::conf::value(&ctx, "theme_default"),
            );
            assert!(screen.resize(cols, rows));
            let area = screen.area();
            // Fill the underlying grid with listing-like subject text.
            for y in 0..rows {
                for x in 0..cols {
                    screen.grid_mut()[(x, y)].set_ch('S');
                }
            }
            dialog.draw(screen.grid_mut(), area, &mut ctx);

            // Locate the dialog: its content screen drives the centered
            // box; recompute the same way draw() did.
            let (w, h) = dialog.content.area().size();
            let dialog_area =
                crate::terminal::ratatui_bridge::center_inside_via_layout(area, (w + 2, h + 2));
            let (x0, _y0) = dialog_area.upper_left();
            let last_x = dialog_area.bottom_right().0;
            for y in dialog_area.upper_left().1..=dialog_area.bottom_right().1 {
                for x in x0..=last_x {
                    let ch = screen.grid()[(x, y)].ch();
                    assert_ne!(
                        ch, 'S',
                        "cell ({x},{y}) inside dialog_area still shows underlying text"
                    );
                }
            }
        }
    }
}
