//
// meli
//
// Copyright 2026 meli authors
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

//! crossterm ↔ meli terminal event translation and raw key re-encoding.
//!
//! Input translation ([`translate_key_event`], [`translate_mouse_event`],
//! [`BridgeEvent`]) maps crossterm 0.29 events onto meli's existing
//! [`Key`]/[`MouseEvent`] vocabulary, preserving the semantics of the
//! pre-migration input path:
//!
//! - `Enter` maps to <code>[Key::Char]('\n')</code> and `Tab` to
//!   <code>[Key::Char]('\t')</code>.
//! - `Ctrl`/`Alt` modified characters map to [`Key::Ctrl`] (lowercased, since
//!   control bytes are case-insensitive on the wire) / [`Key::Alt`].
//! - `KeyEventKind::Press` and `KeyEventKind::Repeat` are accepted;
//!   `KeyEventKind::Release` is dropped (`None`).
//! - Modifier combinations the legacy parser could not handle (e.g.
//!   `Shift`+arrow keys, media/lock/modifier keys, bare mouse motion) are
//!   dropped, matching the input events meli could receive before the
//!   migration.
//! - Mouse coordinates convert from crossterm's 0-based to meli's 1-based;
//!   scroll wheel maps to `Press(WheelUp/WheelDown)` and drags to `Hold`.
//! - [`Event::Paste`] maps to [`Key::Paste`] and [`Event::Resize`] is surfaced
//!   as [`BridgeEvent::Resize`] so the caller can route it (it has no meli
//!   `Key` representation).
//!
//! [`encode_key`] re-encodes a meli [`Key`] into the raw byte stream a
//! terminal would have sent for it (the stream the pre-migration reader
//! handed to the input thread), so the embedded terminal
//! (`terminal::embedded`) and the `ThreadEvent::Input((Key, Vec<u8>))`
//! contract keep receiving the same bytes after the crossterm swap. Byte
//! tables follow the legacy wire grammar (control bytes, SS3/CSI sequences,
//! SGR mouse, bracketed paste).
//!
//! The second half of the bridge maps rendering vocabulary onto ratatui
//! 0.30: [`From`] conversions for colors, attributes and `ThemeAttribute`s,
//! whole-buffer blits ([`blit_cellbuffer_to_buffer`],
//! [`blit_buffer_to_cellbuffer`]), an ASCII/rounded border glyph set chooser
//! ([`border_set_for`]) and [`Area`]↔`Rect` converters ([`area_to_rect`],
//! [`rect_to_area`]) plus `Layout`-carved placement helpers that reproduce
//! the legacy `Area` algebra pixel-for-pixel ([`center_inside_via_layout`],
//! [`place_inside_via_layout`]). The blits are metadata-blind: OSC8 hyperlink tables
//! and keep flags stay on the meli side, and meli's empty wide-char
//! continuation cells translate to ratatui's `CellDiffOption::Skip`.
//! The placement helpers and converters are consumed by the Wave 3 chrome
//! components (Tabbed/StatusBar/Listing splits, dialog and OSD placement).
//!
//! Note: `From<KeyEvent>`/`From<MouseEvent>` impls returning `Option<Key>`
//! are not possible under the orphan rules (`Option<Key>` is a foreign type),
//! hence the free translation functions.

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton as CrosstermMouseButton,
    MouseEvent as CrosstermMouseEvent, MouseEventKind,
};
use ratatui::buffer::{Buffer as RatatuiBuffer, CellDiffOption};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color as RatatuiColor, Modifier as RatatuiModifier, Style as RatatuiStyle};
use ratatui::symbols::border::Set as RatatuiBorderSet;
use ratatui::widgets::{Block as RatatuiBlock, Widget as RatatuiWidget};

use super::cells::{Attr, CellBuffer};
use super::color::Color;
use super::keys::{Key, MouseButton, MouseEvent};
use super::screen::Area;
use crate::conf::ThemeAttribute;

/// A crossterm [`Event`] translated into meli's input vocabulary.
///
/// This is the routable representation of a translated event: key/mouse/paste
/// input collapses into [`BridgeEvent::Key`], resizes are surfaced distinctly
/// for the main loop (they become `UIEvent::Resize`, not an input `Key`), and
/// events the input loop must ignore map to [`BridgeEvent::Ignored`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BridgeEvent {
    /// A translated key, mouse or paste event.
    Key(Key),
    /// Terminal resize with the new size in columns and rows.
    Resize(u16, u16),
    /// The event must be dropped by the input loop: key `Release` kinds,
    /// focus changes, and events with no meli equivalent.
    Ignored,
}

impl From<Event> for BridgeEvent {
    fn from(ev: Event) -> Self {
        match ev {
            Event::Key(key_event) => match translate_key_event(key_event) {
                Some(key) => Self::Key(key),
                None => Self::Ignored,
            },
            Event::Mouse(mouse_event) => match translate_mouse_event(mouse_event) {
                Some(key) => Self::Key(key),
                None => Self::Ignored,
            },
            Event::Paste(buf) => Self::Key(Key::Paste(buf)),
            Event::Resize(columns, rows) => Self::Resize(columns, rows),
            Event::FocusGained | Event::FocusLost => Self::Ignored,
        }
    }
}

/// Translate a crossterm keyboard event into a meli [`Key`].
///
/// Returns `None` when the event must be dropped: `KeyEventKind::Release`,
/// key codes with no meli equivalent (media/lock/modifier keys), or named
/// keys carrying modifiers (the legacy parser rejected those sequences, so
/// they never reached meli as input).
pub fn translate_key_event(ev: KeyEvent) -> Option<Key> {
    if ev.kind == KeyEventKind::Release {
        // meli's input model has no key-release concept; only `Press` and
        // `Repeat` reach the main loop.
        return None;
    }
    let KeyEvent {
        code, modifiers, ..
    } = ev;
    match code {
        KeyCode::Backspace => named(Key::Backspace, modifiers),
        KeyCode::Left => named(Key::Left, modifiers),
        KeyCode::Right => named(Key::Right, modifiers),
        KeyCode::Up => named(Key::Up, modifiers),
        KeyCode::Down => named(Key::Down, modifiers),
        KeyCode::Home => named(Key::Home, modifiers),
        KeyCode::End => named(Key::End, modifiers),
        KeyCode::PageUp => named(Key::PageUp, modifiers),
        KeyCode::PageDown => named(Key::PageDown, modifiers),
        KeyCode::Delete => named(Key::Delete, modifiers),
        KeyCode::Insert => named(Key::Insert, modifiers),
        KeyCode::F(n) => named(Key::F(n), modifiers),
        KeyCode::Esc => named(Key::Esc, modifiers),
        KeyCode::Null => Some(Key::Null),
        KeyCode::Enter => modified_char('\n', modifiers),
        KeyCode::Tab => modified_char('\t', modifiers),
        KeyCode::BackTab => {
            if modifiers == KeyModifiers::SHIFT {
                // Parity with the pre-migration path: `ESC [ Z` parsed to
                // `BackTab` there, which fell through the conversion
                // catch-all to `Key::Char(' ')`.
                Some(Key::Char(' '))
            } else {
                None
            }
        }
        KeyCode::Char(c) => modified_char(c, modifiers),
        KeyCode::CapsLock
        | KeyCode::ScrollLock
        | KeyCode::NumLock
        | KeyCode::PrintScreen
        | KeyCode::Pause
        | KeyCode::Menu
        | KeyCode::KeypadBegin
        | KeyCode::Media(_)
        | KeyCode::Modifier(_) => None,
    }
}

/// Translate a named key, dropping any modifier combination: the legacy
/// parser rejected modifier-carrying CSI sequences (`CSI 1;2D` etc.), so
/// meli never received them as input and has no `Key` variant to hold them.
fn named(key: Key, modifiers: KeyModifiers) -> Option<Key> {
    if modifiers.is_empty() {
        Some(key)
    } else {
        None
    }
}

/// Translate a character-ish key code (`Char`/`Enter`/`Tab`): `CONTROL`
/// maps to `Ctrl` with the character lowercased (control bytes are
/// case-insensitive on the wire), then `ALT` to `Alt`; anything else
/// (`SHIFT` alone, `SUPER`/`HYPER`/`META` which cannot arrive in legacy
/// mode) passes the character through unchanged.
fn modified_char(c: char, modifiers: KeyModifiers) -> Option<Key> {
    if modifiers.contains(KeyModifiers::CONTROL) {
        Some(Key::Ctrl(c.to_ascii_lowercase()))
    } else if modifiers.contains(KeyModifiers::ALT) {
        Some(Key::Alt(c))
    } else {
        Some(Key::Char(c))
    }
}

/// Translate a crossterm mouse event into a meli [`Key::Mouse`].
///
/// Coordinates convert from crossterm's 0-based to meli's 1-based. Returns
/// `None` for kinds with no meli equivalent (bare motion, horizontal wheel):
/// the legacy parser could not handle their SGR codes, so meli never saw
/// them.
pub fn translate_mouse_event(ev: CrosstermMouseEvent) -> Option<Key> {
    // meli's MouseEvent coordinates are 1-based; crossterm's are 0-based.
    let x = ev.column.saturating_add(1);
    let y = ev.row.saturating_add(1);
    Some(Key::Mouse(match ev.kind {
        MouseEventKind::Down(button) => MouseEvent::Press(button.into(), x, y),
        MouseEventKind::Up(_) => MouseEvent::Release(x, y),
        MouseEventKind::Drag(_) => MouseEvent::Hold(x, y),
        MouseEventKind::ScrollUp => MouseEvent::Press(MouseButton::WheelUp, x, y),
        MouseEventKind::ScrollDown => MouseEvent::Press(MouseButton::WheelDown, x, y),
        MouseEventKind::Moved | MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight => {
            return None
        }
    }))
}

/// Encode a meli [`Key`] as the raw byte sequence a terminal sends for it.
///
/// Reproduces the stream that the pre-migration reader yielded for the
/// same key press, per the legacy `parse_event` tables: control keys as
/// their raw control bytes, `Alt(c)` as `ESC c`, arrows/editing keys as
/// CSI/SS3 sequences, `F(1..=4)` as `ESC O P..S`, `F(5..=12)` as
/// `CSI 15,17..21,23,24 ~`, mouse events in SGR form and pastes wrapped in
/// bracketed-paste markers.
///
/// Legacy ambiguity notes (same as the wire format itself): `Ctrl('i')`,
/// `Ctrl('j')` and `Ctrl('m')` share bytes with `Tab`, `Ctrl+J` and `Enter`;
/// `Null`, `Ctrl('@')` and `Ctrl(' ')` all encode as `NUL`; `Char('\r')`
/// encodes like `Enter`.
pub fn encode_key(key: &Key) -> Vec<u8> {
    match key {
        Key::Backspace => vec![0x7f],
        Key::Left => b"\x1b[D".to_vec(),
        Key::Right => b"\x1b[C".to_vec(),
        Key::Up => b"\x1b[A".to_vec(),
        Key::Down => b"\x1b[B".to_vec(),
        Key::Home => b"\x1b[H".to_vec(),
        Key::End => b"\x1b[F".to_vec(),
        Key::PageUp => b"\x1b[5~".to_vec(),
        Key::PageDown => b"\x1b[6~".to_vec(),
        Key::Delete => b"\x1b[3~".to_vec(),
        Key::Insert => b"\x1b[2~".to_vec(),
        // F1-F4 use the SS3 form (`ESC O P..S`); F5-F12 the `CSI number ~`
        // form, per the legacy `parse_csi` table. meli only binds F1-F12;
        // anything else falls back to a generic (unbound) CSI form.
        Key::F(1) => b"\x1bOP".to_vec(),
        Key::F(2) => b"\x1bOQ".to_vec(),
        Key::F(3) => b"\x1bOR".to_vec(),
        Key::F(4) => b"\x1bOS".to_vec(),
        Key::F(5) => b"\x1b[15~".to_vec(),
        Key::F(6) => b"\x1b[17~".to_vec(),
        Key::F(7) => b"\x1b[18~".to_vec(),
        Key::F(8) => b"\x1b[19~".to_vec(),
        Key::F(9) => b"\x1b[20~".to_vec(),
        Key::F(10) => b"\x1b[21~".to_vec(),
        Key::F(11) => b"\x1b[23~".to_vec(),
        Key::F(12) => b"\x1b[24~".to_vec(),
        Key::F(n) => format!("\x1b[{n}~").into_bytes(),
        // The legacy parser reads `\r` (and `\n` outside raw mode) as
        // `Char('\n')`; `Char('\t')` is the plain tab byte.
        Key::Char('\n') | Key::Char('\r') => b"\r".to_vec(),
        Key::Char(c) => c.to_string().into_bytes(),
        Key::Alt(c) => {
            // `ESC` followed by the character, exactly as the legacy parser
            // decodes the `Alt(ch)` arm of `parse_event`.
            let mut buf = vec![0x1b];
            buf.extend(c.to_string().as_bytes());
            buf
        }
        Key::Ctrl(c) => vec![ctrl_byte(*c)],
        Key::Null => vec![0x00],
        Key::Esc => vec![0x1b],
        Key::Mouse(mouse_event) => encode_mouse_event(mouse_event),
        Key::Paste(buf) => {
            let mut bytes = b"\x1b[200~".to_vec();
            bytes.extend_from_slice(buf.as_bytes());
            bytes.extend_from_slice(b"\x1b[201~");
            bytes
        }
    }
}

/// Raw control byte for a `Ctrl(c)` key, per the legacy `parse_event`
/// families: `0x01..=0x1A` decode as `Ctrl('a'..='z')` and
/// `0x1C..=0x1F` as `Ctrl('4'..='7')`; `0x00` (`NUL`) decodes as
/// `Key::Null`, making `Ctrl('@')`/`Ctrl(' ')`/`Null` collide on the wire.
/// Other ASCII characters use the traditional `& 0x1f` mask (`Ctrl('[')`
/// shares `ESC`'s byte, `Ctrl('\\')`/`Ctrl(']')`/`Ctrl('^')`/`Ctrl('_')`
/// the `0x1C..=0x1F` range). Non-ASCII characters have no legacy encoding;
/// the mask is applied to the low byte as a deterministic fallback.
fn ctrl_byte(c: char) -> u8 {
    match c {
        '@' | ' ' => 0x00,
        'a'..='z' => c as u8 - b'a' + 0x01,
        '4'..='7' => c as u8 - b'4' + 0x1c,
        c if c.is_ascii() => c as u8 & 0x1f,
        c => (c as u32 & 0x1f) as u8,
    }
}

/// SGR mouse encoding (`ESC [ < Cb ; Cx ; Cy M|m`), the format crossterm
/// parses when `?1006` mode is on. `Cb`: 0/1/2 = left/middle/
/// right button, 32 = button-held motion (`Hold`), 64/65 = wheel up/down;
/// a lowercase `m` terminator marks release. Coordinates are 1-based, like
/// meli's `MouseEvent`.
fn encode_mouse_event(ev: &MouseEvent) -> Vec<u8> {
    match ev {
        MouseEvent::Press(button, x, y) => {
            let cb = match button {
                MouseButton::Left => 0,
                MouseButton::Middle => 1,
                MouseButton::Right => 2,
                MouseButton::WheelUp => 64,
                MouseButton::WheelDown => 65,
            };
            format!("\x1b[<{cb};{x};{y}M").into_bytes()
        }
        MouseEvent::Release(x, y) => format!("\x1b[<0;{x};{y}m").into_bytes(),
        MouseEvent::Hold(x, y) => format!("\x1b[<32;{x};{y}M").into_bytes(),
    }
}

impl From<CrosstermMouseButton> for MouseButton {
    fn from(val: CrosstermMouseButton) -> Self {
        match val {
            CrosstermMouseButton::Left => Self::Left,
            CrosstermMouseButton::Right => Self::Right,
            CrosstermMouseButton::Middle => Self::Middle,
        }
    }
}

// ---------------------------------------------------------------------------
// ratatui conversions, blits, border sets and area converters (todo 8).
// Not wired into any component yet; Wave 3 todos (11-16) consume these.
// ---------------------------------------------------------------------------

impl From<Color> for RatatuiColor {
    /// Total map onto ratatui's color vocabulary: `Default` is the terminal
    /// default (`Reset`), the eight named colors keep their names, `Byte`
    /// becomes `Indexed` and `Rgb` stays `Rgb`.
    ///
    /// One naming trap: meli's `White` is ANSI white (SGR 37), which ratatui
    /// calls [`RatatuiColor::Gray`]; ratatui's `White` is bright white
    /// (SGR 97) and corresponds to meli's `Color::Byte(15)`.
    fn from(val: Color) -> Self {
        match val {
            Color::Default => Self::Reset,
            Color::Black => Self::Black,
            Color::Red => Self::Red,
            Color::Green => Self::Green,
            Color::Yellow => Self::Yellow,
            Color::Blue => Self::Blue,
            Color::Magenta => Self::Magenta,
            Color::Cyan => Self::Cyan,
            Color::White => Self::Gray,
            Color::Byte(b) => Self::Indexed(b),
            Color::Rgb(r, g, b) => Self::Rgb(r, g, b),
        }
    }
}

impl From<RatatuiColor> for Color {
    /// Inverse of [`From<Color> for RatatuiColor`]. The six ratatui bright
    /// colors (SGR 90..97) have no named meli variant and map to their xterm
    /// 256-color indices 8..=15, which is exactly how meli's own writer
    /// expresses them.
    fn from(val: RatatuiColor) -> Self {
        match val {
            RatatuiColor::Reset => Self::Default,
            RatatuiColor::Black => Self::Black,
            RatatuiColor::Red => Self::Red,
            RatatuiColor::Green => Self::Green,
            RatatuiColor::Yellow => Self::Yellow,
            RatatuiColor::Blue => Self::Blue,
            RatatuiColor::Magenta => Self::Magenta,
            RatatuiColor::Cyan => Self::Cyan,
            RatatuiColor::Gray => Self::White,
            RatatuiColor::DarkGray => Self::Byte(8),
            RatatuiColor::LightRed => Self::Byte(9),
            RatatuiColor::LightGreen => Self::Byte(10),
            RatatuiColor::LightYellow => Self::Byte(11),
            RatatuiColor::LightBlue => Self::Byte(12),
            RatatuiColor::LightMagenta => Self::Byte(13),
            RatatuiColor::LightCyan => Self::Byte(14),
            RatatuiColor::White => Self::Byte(15),
            RatatuiColor::Indexed(b) => Self::Byte(b),
            RatatuiColor::Rgb(r, g, b) => Self::Rgb(r, g, b),
        }
    }
}

impl From<Attr> for RatatuiModifier {
    /// Bit-for-bit map of the attributes both vocabularies share.
    ///
    /// [`Attr::UNDERCURL`] and [`Attr::FORCE_TEXT`] are intentionally
    /// dropped: ratatui has no undercurl modifier, and `FORCE_TEXT` only
    /// drives meli's own U+FE0E text-presentation logic. The flush byte
    /// layer keeps reading both straight from the meli `Cell` attrs, so no
    /// information is lost where it matters.
    fn from(val: Attr) -> Self {
        let mut ret = Self::empty();
        if val.intersects(Attr::BOLD) {
            ret |= Self::BOLD;
        }
        if val.intersects(Attr::DIM) {
            ret |= Self::DIM;
        }
        if val.intersects(Attr::ITALICS) {
            ret |= Self::ITALIC;
        }
        if val.intersects(Attr::UNDERLINE) {
            ret |= Self::UNDERLINED;
        }
        if val.intersects(Attr::BLINK) {
            ret |= Self::SLOW_BLINK;
        }
        if val.intersects(Attr::REVERSE) {
            ret |= Self::REVERSED;
        }
        if val.intersects(Attr::HIDDEN) {
            ret |= Self::HIDDEN;
        }
        ret
    }
}

impl From<RatatuiModifier> for Attr {
    /// Inverse of [`From<Attr> for RatatuiModifier`].
    /// [`RatatuiModifier::RAPID_BLINK`] and [`RatatuiModifier::CROSSED_OUT`]
    /// have no meli `Attr` counterpart and are dropped.
    fn from(val: RatatuiModifier) -> Self {
        let mut ret = Self::DEFAULT;
        if val.intersects(RatatuiModifier::BOLD) {
            ret |= Self::BOLD;
        }
        if val.intersects(RatatuiModifier::DIM) {
            ret |= Self::DIM;
        }
        if val.intersects(RatatuiModifier::ITALIC) {
            ret |= Self::ITALICS;
        }
        if val.intersects(RatatuiModifier::UNDERLINED) {
            ret |= Self::UNDERLINE;
        }
        if val.intersects(RatatuiModifier::SLOW_BLINK) {
            ret |= Self::BLINK;
        }
        if val.intersects(RatatuiModifier::REVERSED) {
            ret |= Self::REVERSE;
        }
        if val.intersects(RatatuiModifier::HIDDEN) {
            ret |= Self::HIDDEN;
        }
        ret
    }
}

impl From<ThemeAttribute> for RatatuiStyle {
    /// A `ThemeAttribute` becomes a ratatui `Style` carrying its fg/bg colors
    /// and shared modifiers.
    ///
    /// `UNDERCURL`/`FORCE_TEXT` bits do not survive (see
    /// [`From<Attr> for RatatuiModifier`]); the flush layer reads them from
    /// the meli `Cell` itself. meli's `Color::Default` is the explicit
    /// terminal default, so it maps to `Reset`, never to ratatui's
    /// "unspecified" (`None`).
    fn from(ThemeAttribute { fg, bg, attrs }: ThemeAttribute) -> Self {
        Self::new()
            .fg(fg.into())
            .bg(bg.into())
            .add_modifier(attrs.into())
    }
}

/// Copy a meli [`CellBuffer`] into a ratatui [`ratatui::buffer::Buffer`],
/// cell for cell over
/// the intersection of their dimensions (mismatched sizes clamp; they never
/// panic).
///
/// Per cell: glyph, fg/bg colors and modifiers are copied; the underline
/// color is reset (meli keeps no per-cell underline color); a meli
/// empty-continuation cell (the right half of a wide glyph) becomes a
/// [`CellDiffOption::Skip`] cell, ratatui's equivalent of "covered by the
/// preceding cell, do not draw". OSC8 hyperlink tables are `CellBuffer`
/// metadata and have no ratatui counterpart, so they are not copied.
pub fn blit_cellbuffer_to_buffer(src: &CellBuffer, dst: &mut RatatuiBuffer) {
    let (dst_x, dst_y) = (dst.area.x, dst.area.y);
    let cols = src.cols.min(dst.area.width as usize);
    let rows = src.rows.min(dst.area.height as usize);
    for y in 0..rows {
        for x in 0..cols {
            let (Some(src_cell), Some(dst_cell)) = (
                src.get(x, y),
                dst.cell_mut((dst_x + x as u16, dst_y + y as u16)),
            ) else {
                continue;
            };
            dst_cell.set_char(src_cell.ch());
            dst_cell.fg = src_cell.fg().into();
            dst_cell.bg = src_cell.bg().into();
            dst_cell.underline_color = RatatuiColor::Reset;
            dst_cell.modifier = src_cell.attrs().into();
            dst_cell.set_diff_option(if src_cell.empty() {
                CellDiffOption::Skip
            } else {
                CellDiffOption::None
            });
        }
    }
}

/// Copy a ratatui [`ratatui::buffer::Buffer`] into a meli [`CellBuffer`], cell for cell over
/// the intersection of their dimensions (mismatched sizes clamp; they never
/// panic).
///
/// Per cell: the symbol's first char becomes the cell glyph (meli cells hold
/// a single `char`, so a multi-codepoint grapheme cluster collapses to its
/// leading char), fg/bg colors and modifiers are copied, and a
/// [`CellDiffOption::Skip`] cell becomes a meli empty-continuation cell.
/// Underline colors are dropped (no meli equivalent). Keep flags and OSC8
/// hyperlink tables belong to the destination and are left untouched.
pub fn blit_buffer_to_cellbuffer(src: &RatatuiBuffer, dst: &mut CellBuffer) {
    let (src_x, src_y) = (src.area.x, src.area.y);
    let cols = (src.area.width as usize).min(dst.cols);
    let rows = (src.area.height as usize).min(dst.rows);
    for y in 0..rows {
        for x in 0..cols {
            let (Some(src_cell), Some(dst_cell)) = (
                src.cell((src_x + x as u16, src_y + y as u16)),
                dst.get_mut(x, y),
            ) else {
                continue;
            };
            let ch = src_cell.symbol().chars().next().unwrap_or(' ');
            let continuation = src_cell.diff_option == CellDiffOption::Skip;
            dst_cell.overwrite(
                ch,
                src_cell.fg.into(),
                src_cell.bg.into(),
                src_cell.modifier.into(),
                continuation,
            );
        }
    }
}

/// Copy a ratatui buffer into a meli [`CellBuffer`] anchored at `area`.
///
/// Unlike [`blit_buffer_to_cellbuffer`] (which targets the grid origin
/// and suits full-screen copies only), this blits at `area`'s
/// upper-left corner.
///
/// Per cell: the symbol's first char becomes the cell glyph, fg/bg
/// colors and modifiers are copied. The intersection of `area` and the
/// source buffer dimensions is copied; everything else is untouched.
pub fn blit_buffer_to_cellbuffer_at(src: &RatatuiBuffer, dst: &mut CellBuffer, area: Area) {
    debug_assert_eq!(dst.generation(), area.generation());
    let (x0, y0) = area.upper_left();
    let width = area.width().min(src.area().width as usize);
    let height = area.height().min(src.area().height as usize);
    for y in 0..height {
        for x in 0..width {
            let (Some(s), Some(d)) = (src.cell((x as u16, y as u16)), dst.get_mut(x0 + x, y0 + y))
            else {
                continue;
            };
            d.overwrite(
                s.symbol().chars().next().unwrap_or(' '),
                s.fg.into(),
                s.bg.into(),
                s.modifier.into(),
                false,
            );
        }
    }
}

/// Border glyph set for pane chrome: the rounded set normally, or meli's
/// `ascii_drawing` set (`-`/`|`/`+`) when the terminal must stay ASCII-only.
///
/// The ASCII glyphs mirror meli's own boundary table in
/// `cells.rs::boundaries::Boundary::to_char(true)`: horizontal segments are
/// `-`, verticals `|`, and every corner or joint `+`.
pub fn border_set_for(ascii_drawing: bool) -> RatatuiBorderSet<'static> {
    if ascii_drawing {
        RatatuiBorderSet {
            top_left: "+",
            top_right: "+",
            bottom_left: "+",
            bottom_right: "+",
            vertical_left: "|",
            vertical_right: "|",
            horizontal_top: "-",
            horizontal_bottom: "-",
        }
    } else {
        ratatui::symbols::border::ROUNDED
    }
}

/// Draw a rounded pane frame over `area` in `grid`, returning the inner
/// area.
///
/// The frame is rendered through ratatui's `Block` widget: a temporary
/// [`RatatuiBuffer`] receives a `Block::bordered()` render with the
/// [`border_set_for`] glyph set (rounded corners normally, meli's ASCII set
/// when the grid is in `ascii_drawing` mode) and `border_attr` as the border
/// style, and then only the border ring is blitted back. Interior cells are
/// left untouched, so the frame can be layered over already-drawn pane
/// content without erasing it.
///
/// The returned inner area matches `boundaries::create_box`'s contract
/// (`area.skip(1, 1)` minus the last row/column), so call sites can swap a
/// `create_box` call for this helper without touching their content
/// algebra. Unlike `create_box`, the frame does not join corners with
/// pre-existing boundary glyphs (a `Block` cannot render joints); interior
/// dividers that must join keep using `create_box`.
///
/// Areas narrower than two columns or shorter than two rows cannot hold a
/// frame; they are returned unchanged and nothing is drawn.
pub fn draw_rounded_frame(grid: &mut CellBuffer, area: Area, border_attr: ThemeAttribute) -> Area {
    debug_assert_eq!(grid.generation(), area.generation());
    if area.width() < 2 || area.height() < 2 {
        return area;
    }
    let mut buf = RatatuiBuffer::empty(Rect::new(0, 0, area.width() as u16, area.height() as u16));
    let block = RatatuiBlock::bordered()
        .border_set(border_set_for(grid.ascii_drawing))
        .border_style(RatatuiStyle::from(border_attr));
    RatatuiWidget::render(block, buf.area, &mut buf);
    let (x0, y0) = area.upper_left();
    let last_row = area.height() - 1;
    let last_col = area.width() - 1;
    let blit_ring_cell = |grid: &mut CellBuffer, x: usize, y: usize| {
        let (Some(src), Some(dst)) = (buf.cell((x as u16, y as u16)), grid.get_mut(x0 + x, y0 + y))
        else {
            return;
        };
        dst.overwrite(
            src.symbol().chars().next().unwrap_or(' '),
            src.fg.into(),
            src.bg.into(),
            src.modifier.into(),
            false,
        );
    };
    for x in 0..area.width() {
        blit_ring_cell(grid, x, 0);
        blit_ring_cell(grid, x, last_row);
    }
    for y in 0..area.height() {
        blit_ring_cell(grid, 0, y);
        blit_ring_cell(grid, last_col, y);
    }
    area.skip(1, 1).skip_rows_from_end(1).skip_cols_from_end(1)
}

/// The four border strips (top/bottom rows, left/right columns) of `area`.
///
/// For callers that blit a frame and must push the touched cells for
/// flushing. Degenerate areas clamp: every strip stays within `area`,
/// never out of bounds.
pub fn frame_ring_areas(area: Area) -> [Area; 4] {
    [
        area.nth_row(0),
        area.nth_row(area.height().saturating_sub(1)),
        area.nth_col(0),
        area.nth_col(area.width().saturating_sub(1)),
    ]
}

/// Convert a meli [`Area`] into a ratatui [`Rect`], keeping the absolute
/// screen position and the size. An empty area becomes a zero rect.
pub fn area_to_rect(area: Area) -> Rect {
    let (x, y) = area.upper_left();
    Rect::new(
        x as u16,
        y as u16,
        area.width() as u16,
        area.height() as u16,
    )
}

/// Convert a ratatui [`Rect`] into a meli [`Area`] anchored on `root`.
///
/// meli areas are bounds-checked against a screen canvas (they carry a
/// [`super::screen::ScreenGeneration`]), so a bare `Rect` cannot become an
/// `Area` on its own. `rect` coordinates are absolute screen positions (as
/// produced by [`area_to_rect`] and kept by `Layout::split`), and must lie
/// within `root`; rects reaching past `root` clamp to an empty area (via
/// `Area::skip`/`Area::take`), never out of bounds.
pub fn rect_to_area(rect: Rect, root: Area) -> Area {
    let (root_x, root_y) = root.upper_left();
    root.skip(
        (rect.x as usize).saturating_sub(root_x),
        (rect.y as usize).saturating_sub(root_y),
    )
    .take(rect.width as usize, rect.height as usize)
}

/// Center a `(width, height)` box inside `area`, pixel-identical to
/// [`Area::align_inside`] with `Alignment::Center` on both axes, with the
/// resulting split carved through a ratatui [`Layout`].
///
/// meli's centering rounds the leading pad as `max(len/2, box/2) - box/2`
/// (integer division), which differs from ratatui's `Flex::Center` on odd
/// sizes; the pad is therefore derived with meli's formula and handed to the
/// solver as explicit `Length` segments, so the delivered area matches the
/// previous math exactly for every size, including over-large boxes (which
/// clamp like `align_inside` did).
pub fn center_inside_via_layout(area: Area, (width, height): (usize, usize)) -> Area {
    if area.is_empty() || width == 0 || height == 0 {
        return area;
    }
    let (area_width, area_height) = (area.width(), area.height());
    let pad_x = std::cmp::max(area_width / 2, width / 2) - width / 2;
    let pad_y = std::cmp::max(area_height / 2, height / 2) - height / 2;
    // `align_inside` keeps `width` on the horizontal axis and takes
    // `min(height, area height)` on the vertical one; `take` clamps both to
    // the space left after skipping the pad.
    let box_width = width.min(area_width - pad_x);
    let box_height = height.min(area_height).min(area_height - pad_y);
    let [_, horizontal_segment] = Layout::horizontal([
        Constraint::Length(pad_x as u16),
        Constraint::Length(box_width as u16),
    ])
    .areas(area_to_rect(area));
    let [_, vertical_segment] = Layout::vertical([
        Constraint::Length(pad_y as u16),
        Constraint::Length(box_height as u16),
    ])
    .areas(horizontal_segment);
    rect_to_area(vertical_segment, area)
}

/// Anchor a `(width, height)` box inside `area` toward the chosen corner,
/// pixel-identical to [`Area::place_inside`], with the resulting placement
/// carved through a ratatui [`Layout`].
///
/// `place_inside` keeps a two-cell margin between the box and the corner
/// (upper/left when the respective flag is `true`, bottom-right otherwise)
/// and saturates when the box is larger than the area; both behaviors are
/// mirrored here before the solver carve so the delivered area matches the
/// previous math exactly for every size. Unlike `place_inside`, a box wider
/// or taller than the area minus its margin can never escape `root` on the
/// opposite side: it clamps to `root` (the full-screen notification call
/// site saturates at the same cells either way).
pub fn place_inside_via_layout(
    area: Area,
    (width, height): (usize, usize),
    upper: bool,
    left: bool,
) -> Area {
    if area.is_empty() || width < 3 || height < 3 {
        return area;
    }
    let (area_x, area_y) = area.upper_left();
    let (max_x, max_y) = area.bottom_right();
    let x = if upper {
        area_x + 2
    } else {
        max_x.saturating_sub(2).saturating_sub(width)
    };
    let y = if left {
        area_y + 2
    } else {
        max_y.saturating_sub(2).saturating_sub(height)
    };
    let upper_left = (x.min(max_x), y.min(max_y));
    let bottom_right = ((x + width).min(max_x), (y + height).min(max_y));
    let pad_x = upper_left.0.saturating_sub(area_x);
    let pad_y = upper_left.1.saturating_sub(area_y);
    let box_width = bottom_right.0.saturating_sub(upper_left.0) + 1;
    let box_height = bottom_right.1.saturating_sub(upper_left.1) + 1;
    let [_, horizontal_segment] = Layout::horizontal([
        Constraint::Length(pad_x as u16),
        Constraint::Length(box_width as u16),
    ])
    .areas(area_to_rect(area));
    let [_, vertical_segment] = Layout::vertical([
        Constraint::Length(pad_y as u16),
        Constraint::Length(box_height as u16),
    ])
    .areas(horizontal_segment);
    rect_to_area(vertical_segment, area)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::keys::{Key, MouseButton, MouseEvent};
    use crossterm::event::{
        Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MediaKeyCode,
        ModifierKeyCode, MouseButton as CTMouseButton, MouseEvent as CTMouseEvent, MouseEventKind,
    };

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn key_mod(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> CTMouseEvent {
        CTMouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    /// Full matrix of `KeyEvent` → `Key` translations covering all 19 meli
    /// `Key` variants plus modifier combinations and unmappable codes.
    #[test]
    fn translate_key_event_matrix() {
        let cases: Vec<(&str, KeyEvent, Option<Key>)> = vec![
            // Named keys (the 19 Key variants, minus Char/Alt/Ctrl/F which get
            // dedicated cases below).
            ("Backspace", key(KeyCode::Backspace), Some(Key::Backspace)),
            ("Left", key(KeyCode::Left), Some(Key::Left)),
            ("Right", key(KeyCode::Right), Some(Key::Right)),
            ("Up", key(KeyCode::Up), Some(Key::Up)),
            ("Down", key(KeyCode::Down), Some(Key::Down)),
            ("Home", key(KeyCode::Home), Some(Key::Home)),
            ("End", key(KeyCode::End), Some(Key::End)),
            ("PageUp", key(KeyCode::PageUp), Some(Key::PageUp)),
            ("PageDown", key(KeyCode::PageDown), Some(Key::PageDown)),
            ("Delete", key(KeyCode::Delete), Some(Key::Delete)),
            ("Insert", key(KeyCode::Insert), Some(Key::Insert)),
            ("Null", key(KeyCode::Null), Some(Key::Null)),
            ("Esc", key(KeyCode::Esc), Some(Key::Esc)),
            // Char passthrough: Enter and Tab are normalized onto Char, like
            // the legacy parser did for `\r` and `\t`.
            ("Enter", key(KeyCode::Enter), Some(Key::Char('\n'))),
            ("Tab", key(KeyCode::Tab), Some(Key::Char('\t'))),
            ("Char_a", key(KeyCode::Char('a')), Some(Key::Char('a'))),
            ("Char_Z", key(KeyCode::Char('Z')), Some(Key::Char('Z'))),
            ("Char_é", key(KeyCode::Char('é')), Some(Key::Char('é'))),
            ("Char_space", key(KeyCode::Char(' ')), Some(Key::Char(' '))),
            // Modifiers on characters.
            (
                "Char_x+CONTROL",
                key_mod(KeyCode::Char('x'), KeyModifiers::CONTROL),
                Some(Key::Ctrl('x')),
            ),
            (
                "Char_X+CONTROL_lowercases",
                key_mod(KeyCode::Char('X'), KeyModifiers::CONTROL),
                Some(Key::Ctrl('x')),
            ),
            (
                "Char_space+CONTROL",
                key_mod(KeyCode::Char(' '), KeyModifiers::CONTROL),
                Some(Key::Ctrl(' ')),
            ),
            (
                "Char_x+ALT",
                key_mod(KeyCode::Char('x'), KeyModifiers::ALT),
                Some(Key::Alt('x')),
            ),
            (
                "Enter+ALT",
                key_mod(KeyCode::Enter, KeyModifiers::ALT),
                Some(Key::Alt('\n')),
            ),
            (
                "Enter+CONTROL",
                key_mod(KeyCode::Enter, KeyModifiers::CONTROL),
                Some(Key::Ctrl('\n')),
            ),
            (
                "Tab+CONTROL",
                key_mod(KeyCode::Tab, KeyModifiers::CONTROL),
                Some(Key::Ctrl('\t')),
            ),
            // CONTROL takes precedence over ALT (legacy terminals cannot
            // express the combination anyway).
            (
                "Char_x+CONTROL+ALT",
                key_mod(
                    KeyCode::Char('x'),
                    KeyModifiers::CONTROL | KeyModifiers::ALT,
                ),
                Some(Key::Ctrl('x')),
            ),
            // SHIFT is folded into the char case by crossterm's parser.
            (
                "Char_a+SHIFT",
                key_mod(KeyCode::Char('a'), KeyModifiers::SHIFT),
                Some(Key::Char('a')),
            ),
            // BackTab arrives as SHIFT+Tab: parity with the old input path
            // which fell through to `Key::Char(' ')`.
            (
                "BackTab+SHIFT",
                key_mod(KeyCode::BackTab, KeyModifiers::SHIFT),
                Some(Key::Char(' ')),
            ),
            ("BackTab_bare", key(KeyCode::BackTab), None),
            // Named keys with modifiers never reached meli pre-migration;
            // keep dropping them.
            (
                "Left+SHIFT_dropped",
                key_mod(KeyCode::Left, KeyModifiers::SHIFT),
                None,
            ),
            (
                "Home+CONTROL_dropped",
                key_mod(KeyCode::Home, KeyModifiers::CONTROL),
                None,
            ),
            (
                "F(1)+ALT_dropped",
                key_mod(KeyCode::F(1), KeyModifiers::ALT),
                None,
            ),
            (
                "Esc+ALT_dropped",
                key_mod(KeyCode::Esc, KeyModifiers::ALT),
                None,
            ),
            // Key codes with no meli equivalent.
            ("Media_Play", key(KeyCode::Media(MediaKeyCode::Play)), None),
            (
                "Modifier_LeftControl",
                key(KeyCode::Modifier(ModifierKeyCode::LeftControl)),
                None,
            ),
            ("CapsLock", key(KeyCode::CapsLock), None),
            ("ScrollLock", key(KeyCode::ScrollLock), None),
            ("NumLock", key(KeyCode::NumLock), None),
            ("PrintScreen", key(KeyCode::PrintScreen), None),
            ("Pause", key(KeyCode::Pause), None),
            ("Menu", key(KeyCode::Menu), None),
            ("KeypadBegin", key(KeyCode::KeypadBegin), None),
        ];
        for (name, input, expected) in &cases {
            let got = translate_key_event(*input);
            println!(
                "case translate_key_event_matrix[{name}]: {input:?} -> {got:?} (want {expected:?})"
            );
            assert_eq!(got, *expected, "case {name} failed for {input:?}");
        }
        // Function keys 1 through 12.
        for n in 1u8..=12 {
            let input = key(KeyCode::F(n));
            let expected = Some(Key::F(n));
            let got = translate_key_event(input);
            println!(
                "case translate_key_event_matrix[F{n}]: {input:?} -> {got:?} (want {expected:?})"
            );
            assert_eq!(got, expected, "case F{n} failed");
        }
        println!(
            "translate_key_event_matrix: {} named cases + 12 F-key cases checked",
            cases.len()
        );
    }

    /// `KeyEventKind::Press` and `Repeat` are accepted; `Release` is dropped.
    #[test]
    fn translate_key_event_kind_filter() {
        for kind in [KeyEventKind::Press, KeyEventKind::Repeat] {
            let input = KeyEvent::new_with_kind(KeyCode::Char('x'), KeyModifiers::NONE, kind);
            let got = translate_key_event(input);
            println!("case translate_key_event_kind_filter[{kind:?} Char('x')]: -> {got:?}");
            assert_eq!(got, Some(Key::Char('x')), "kind {kind:?} must translate");
        }
        for (name, code) in [
            ("Char", KeyCode::Char('x')),
            ("Left", KeyCode::Left),
            ("F(5)", KeyCode::F(5)),
            ("Enter", KeyCode::Enter),
        ] {
            let input = KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Release);
            let got = translate_key_event(input);
            println!("case translate_key_event_kind_filter[Release {name}]: {input:?} -> {got:?}");
            assert_eq!(got, None, "Release of {name} must be dropped");
        }
        println!("translate_key_event_kind_filter: 6 cases checked");
    }

    /// Every `MouseEventKind` maps with crossterm 0-based → meli 1-based
    /// coordinates.
    #[test]
    fn translate_mouse_event_matrix() {
        let cases: Vec<(&str, CTMouseEvent, Option<Key>)> = vec![
            (
                "Down(Left)",
                mouse(MouseEventKind::Down(CTMouseButton::Left), 0, 0),
                Some(Key::Mouse(MouseEvent::Press(MouseButton::Left, 1, 1))),
            ),
            (
                "Down(Middle)",
                mouse(MouseEventKind::Down(CTMouseButton::Middle), 4, 9),
                Some(Key::Mouse(MouseEvent::Press(MouseButton::Middle, 5, 10))),
            ),
            (
                "Down(Right)",
                mouse(MouseEventKind::Down(CTMouseButton::Right), 7, 2),
                Some(Key::Mouse(MouseEvent::Press(MouseButton::Right, 8, 3))),
            ),
            (
                "Up(Left)",
                mouse(MouseEventKind::Up(CTMouseButton::Left), 4, 9),
                Some(Key::Mouse(MouseEvent::Release(5, 10))),
            ),
            (
                "Up(Right)_button_discarded",
                mouse(MouseEventKind::Up(CTMouseButton::Right), 10, 10),
                Some(Key::Mouse(MouseEvent::Release(11, 11))),
            ),
            (
                "Drag(Left)",
                mouse(MouseEventKind::Drag(CTMouseButton::Left), 0, 0),
                Some(Key::Mouse(MouseEvent::Hold(1, 1))),
            ),
            (
                "Drag(Middle)",
                mouse(MouseEventKind::Drag(CTMouseButton::Middle), 3, 3),
                Some(Key::Mouse(MouseEvent::Hold(4, 4))),
            ),
            (
                "Drag(Right)",
                mouse(MouseEventKind::Drag(CTMouseButton::Right), 2, 2),
                Some(Key::Mouse(MouseEvent::Hold(3, 3))),
            ),
            ("Moved_dropped", mouse(MouseEventKind::Moved, 1, 1), None),
            (
                "ScrollUp",
                mouse(MouseEventKind::ScrollUp, 99, 0),
                Some(Key::Mouse(MouseEvent::Press(MouseButton::WheelUp, 100, 1))),
            ),
            (
                "ScrollDown",
                mouse(MouseEventKind::ScrollDown, 0, 99),
                Some(Key::Mouse(MouseEvent::Press(
                    MouseButton::WheelDown,
                    1,
                    100,
                ))),
            ),
            (
                "ScrollLeft_dropped",
                mouse(MouseEventKind::ScrollLeft, 0, 0),
                None,
            ),
            (
                "ScrollRight_dropped",
                mouse(MouseEventKind::ScrollRight, 0, 0),
                None,
            ),
            // Mouse modifiers are ignored (meli's MouseEvent has none).
            (
                "Down(Left)+CONTROL_modifiers_ignored",
                CTMouseEvent {
                    kind: MouseEventKind::Down(CTMouseButton::Left),
                    column: 0,
                    row: 0,
                    modifiers: KeyModifiers::CONTROL,
                },
                Some(Key::Mouse(MouseEvent::Press(MouseButton::Left, 1, 1))),
            ),
            // Saturating conversion at the u16 boundary: no wrap, no panic.
            (
                "Down(Left)_at_u16::MAX",
                mouse(
                    MouseEventKind::Down(CTMouseButton::Left),
                    u16::MAX,
                    u16::MAX,
                ),
                Some(Key::Mouse(MouseEvent::Press(
                    MouseButton::Left,
                    u16::MAX,
                    u16::MAX,
                ))),
            ),
        ];
        for (name, input, expected) in &cases {
            let got = translate_mouse_event(*input);
            println!("case translate_mouse_event_matrix[{name}]: {input:?} -> {got:?} (want {expected:?})");
            assert_eq!(got, *expected, "case {name} failed for {input:?}");
        }
        println!(
            "translate_mouse_event_matrix: {} cases checked",
            cases.len()
        );
    }

    /// Whole-`Event` translation: key/mouse/paste collapse to
    /// `BridgeEvent::Key`, resize is surfaced distinctly, the rest is ignored.
    #[test]
    fn bridge_event_matrix() {
        let cases: Vec<(&str, Event, BridgeEvent)> = vec![
            (
                "Key(Char('q'))",
                Event::Key(key(KeyCode::Char('q'))),
                BridgeEvent::Key(Key::Char('q')),
            ),
            (
                "Key(Up)",
                Event::Key(key(KeyCode::Up)),
                BridgeEvent::Key(Key::Up),
            ),
            (
                "Key(Char('x') Release)_dropped",
                Event::Key(KeyEvent::new_with_kind(
                    KeyCode::Char('x'),
                    KeyModifiers::NONE,
                    KeyEventKind::Release,
                )),
                BridgeEvent::Ignored,
            ),
            (
                "Key(Media)_dropped",
                Event::Key(key(KeyCode::Media(MediaKeyCode::MuteVolume))),
                BridgeEvent::Ignored,
            ),
            (
                "Mouse(Down(Left))",
                Event::Mouse(mouse(MouseEventKind::Down(CTMouseButton::Left), 4, 9)),
                BridgeEvent::Key(Key::Mouse(MouseEvent::Press(MouseButton::Left, 5, 10))),
            ),
            (
                "Mouse(Moved)_dropped",
                Event::Mouse(mouse(MouseEventKind::Moved, 1, 1)),
                BridgeEvent::Ignored,
            ),
            (
                "Mouse(ScrollLeft)_dropped",
                Event::Mouse(mouse(MouseEventKind::ScrollLeft, 1, 1)),
                BridgeEvent::Ignored,
            ),
            (
                "Paste",
                Event::Paste("pasted text".to_string()),
                BridgeEvent::Key(Key::Paste("pasted text".to_string())),
            ),
            (
                "Resize",
                Event::Resize(120, 40),
                BridgeEvent::Resize(120, 40),
            ),
            (
                "FocusGained_dropped",
                Event::FocusGained,
                BridgeEvent::Ignored,
            ),
            ("FocusLost_dropped", Event::FocusLost, BridgeEvent::Ignored),
        ];
        for (name, input, expected) in &cases {
            let got = BridgeEvent::from(input.clone());
            println!("case bridge_event_matrix[{name}]: {input:?} -> {got:?} (want {expected:?})");
            assert_eq!(got, *expected, "case {name} failed for {input:?}");
        }
        println!("bridge_event_matrix: {} cases checked", cases.len());
    }

    /// Byte table per `Key`, matching the legacy `parse_event` grammar
    /// (verified against the crate sources in the local registry cache).
    #[test]
    fn encode_key_byte_table() {
        let cases: Vec<(&str, Key, &[u8])> = vec![
            ("Backspace", Key::Backspace, &[0x7f]),
            ("Left", Key::Left, b"\x1b[D"),
            ("Right", Key::Right, b"\x1b[C"),
            ("Up", Key::Up, b"\x1b[A"),
            ("Down", Key::Down, b"\x1b[B"),
            ("Home", Key::Home, b"\x1b[H"),
            ("End", Key::End, b"\x1b[F"),
            ("PageUp", Key::PageUp, b"\x1b[5~"),
            ("PageDown", Key::PageDown, b"\x1b[6~"),
            ("Delete", Key::Delete, b"\x1b[3~"),
            ("Insert", Key::Insert, b"\x1b[2~"),
            ("F(1)", Key::F(1), b"\x1bOP"),
            ("F(2)", Key::F(2), b"\x1bOQ"),
            ("F(3)", Key::F(3), b"\x1bOR"),
            ("F(4)", Key::F(4), b"\x1bOS"),
            ("F(5)", Key::F(5), b"\x1b[15~"),
            ("F(6)", Key::F(6), b"\x1b[17~"),
            ("F(7)", Key::F(7), b"\x1b[18~"),
            ("F(8)", Key::F(8), b"\x1b[19~"),
            ("F(9)", Key::F(9), b"\x1b[20~"),
            ("F(10)", Key::F(10), b"\x1b[21~"),
            ("F(11)", Key::F(11), b"\x1b[23~"),
            ("F(12)", Key::F(12), b"\x1b[24~"),
            ("Enter(Char('\\n'))", Key::Char('\n'), b"\r"),
            ("Tab(Char('\\t'))", Key::Char('\t'), b"\t"),
            ("Char('a')", Key::Char('a'), b"a"),
            ("Char('Z')", Key::Char('Z'), b"Z"),
            ("Char('é')", Key::Char('é'), "é".as_bytes()),
            // The legacy parser treated both `\r` and (outside raw mode) `\n`
            // as `Char('\n')`; `Char('\r')` therefore encodes as `\r` too.
            ("Char('\\r')_aliases_Enter", Key::Char('\r'), b"\r"),
            ("Alt('x')", Key::Alt('x'), b"\x1bx"),
            ("Alt('Q')", Key::Alt('Q'), b"\x1bQ"),
            ("Alt('\\n')", Key::Alt('\n'), b"\x1b\n"),
            ("Alt('é')", Key::Alt('é'), b"\x1b\xc3\xa9"),
            // Legacy control-byte families:
            // 0x01..=0x1A -> Ctrl('a'..='z'), 0x1C..=0x1F -> Ctrl('4'..='7').
            ("Ctrl('a')", Key::Ctrl('a'), &[0x01]),
            ("Ctrl('z')", Key::Ctrl('z'), &[0x1a]),
            ("Ctrl('4')", Key::Ctrl('4'), &[0x1c]),
            ("Ctrl('5')", Key::Ctrl('5'), &[0x1d]),
            ("Ctrl('6')", Key::Ctrl('6'), &[0x1e]),
            ("Ctrl('7')", Key::Ctrl('7'), &[0x1f]),
            // NUL ambiguity: the legacy parser decoded 0x00 as `Key::Null`,
            // so these three collide on the wire (documented in `encode_key`).
            ("Null", Key::Null, &[0x00]),
            ("Ctrl('@')", Key::Ctrl('@'), &[0x00]),
            ("Ctrl(' ')", Key::Ctrl(' '), &[0x00]),
            ("Esc", Key::Esc, &[0x1b]),
            (
                "Mouse Press(Left)",
                Key::Mouse(MouseEvent::Press(MouseButton::Left, 5, 10)),
                b"\x1b[<0;5;10M",
            ),
            (
                "Mouse Press(Middle)",
                Key::Mouse(MouseEvent::Press(MouseButton::Middle, 1, 1)),
                b"\x1b[<1;1;1M",
            ),
            (
                "Mouse Press(Right)",
                Key::Mouse(MouseEvent::Press(MouseButton::Right, 223, 223)),
                b"\x1b[<2;223;223M",
            ),
            (
                "Mouse Press(WheelUp)",
                Key::Mouse(MouseEvent::Press(MouseButton::WheelUp, 80, 25)),
                b"\x1b[<64;80;25M",
            ),
            (
                "Mouse Press(WheelDown)",
                Key::Mouse(MouseEvent::Press(MouseButton::WheelDown, 80, 25)),
                b"\x1b[<65;80;25M",
            ),
            (
                "Mouse Release",
                Key::Mouse(MouseEvent::Release(9, 9)),
                b"\x1b[<0;9;9m",
            ),
            (
                "Mouse Hold",
                Key::Mouse(MouseEvent::Hold(3, 4)),
                b"\x1b[<32;3;4M",
            ),
            (
                "Paste",
                Key::Paste("hi\nthere".to_string()),
                b"\x1b[200~hi\nthere\x1b[201~",
            ),
            (
                "Paste_empty",
                Key::Paste(String::new()),
                b"\x1b[200~\x1b[201~",
            ),
        ];
        for (name, key, expected) in &cases {
            let got = encode_key(key);
            println!(
                "case encode_key_byte_table[{name}]: {key:?} -> {:02x?} (want {:02x?})",
                got, expected
            );
            assert_eq!(&got, expected, "case {name} failed for {key:?}");
        }
        // Whole control families.
        for (i, c) in ('a'..='z').enumerate() {
            let expected = vec![(i + 1) as u8];
            let got = encode_key(&Key::Ctrl(c));
            println!("case encode_key_byte_table[Ctrl('{c}')]: -> {got:02x?}");
            assert_eq!(got, expected, "Ctrl('{c}') must encode as 0x{:02x}", i + 1);
        }
        for (i, c) in ('4'..='7').enumerate() {
            let expected = vec![0x1c + i as u8];
            let got = encode_key(&Key::Ctrl(c));
            println!("case encode_key_byte_table[Ctrl('{c}')]: -> {got:02x?}");
            assert_eq!(got, expected, "Ctrl('{c}') must encode as 0x1c+{i}");
        }
        println!(
            "encode_key_byte_table: {} named cases + 30 family cases checked",
            cases.len()
        );
    }

    /// Malformed/unexpected input: exotic modifier combinations and states
    /// must not panic and must yield deterministic results.
    #[test]
    fn translate_unexpected_modifier_combos() {
        // SUPER/HYPER/META cannot arrive in legacy mode; they are ignored
        // rather than dropping the key.
        let cases: Vec<(&str, KeyEvent, Option<Key>)> = vec![
            (
                "Char('x')+SUPER+HYPER+META",
                key_mod(
                    KeyCode::Char('x'),
                    KeyModifiers::SUPER | KeyModifiers::HYPER | KeyModifiers::META,
                ),
                Some(Key::Char('x')),
            ),
            (
                "Char('q')+CONTROL+ALT+SHIFT",
                key_mod(
                    KeyCode::Char('q'),
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT,
                ),
                Some(Key::Ctrl('q')),
            ),
            (
                "Char('a')+ALL_MODIFIERS",
                key_mod(KeyCode::Char('a'), KeyModifiers::all()),
                Some(Key::Ctrl('a')),
            ),
            (
                "BackTab+CONTROL",
                key_mod(KeyCode::BackTab, KeyModifiers::CONTROL),
                None,
            ),
            (
                "F(1)+SUPER",
                key_mod(KeyCode::F(1), KeyModifiers::SUPER),
                None,
            ),
        ];
        for (name, input, expected) in &cases {
            let got = translate_key_event(*input);
            println!("case translate_unexpected_modifier_combos[{name}]: {input:?} -> {got:?} (want {expected:?})");
            assert_eq!(got, *expected, "case {name} failed for {input:?}");
        }
        // KeyEventState (keypad/caps/num lock) does not influence translation.
        let keypad = KeyEvent::new_with_kind_and_state(
            KeyCode::Char('5'),
            KeyModifiers::NONE,
            KeyEventKind::Press,
            KeyEventState::KEYPAD,
        );
        assert_eq!(translate_key_event(keypad), Some(Key::Char('5')));
        println!("case translate_unexpected_modifier_combos[KEYPAD Char('5')]: -> {:?} (want Some(Char('5')))", translate_key_event(keypad));
        println!(
            "translate_unexpected_modifier_combos: {} cases + 1 keypad case checked",
            cases.len()
        );
    }

    // ------------------------------------------------------------------
    // ratatui conversions, blits, border sets and area converters (todo 8)
    // ------------------------------------------------------------------

    use crate::terminal::cells::Cell;
    use crate::terminal::{Screen, ScreenGeneration, Virtual};
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::{Color as RColor, Modifier as RModifier, Style as RStyle};
    use ratatui::symbols::border as rborder;

    /// Deterministic xorshift64* PRNG (fixed seed, non-zero) so the random
    /// property frames are identical on every run and every machine.
    struct XorShift(u64);

    impl XorShift {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }

        fn byte(&mut self) -> u8 {
            (self.next() >> 11) as u8
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// Property-test seed. Fixed; change deliberately only.
    const PROPERTY_SEED: u64 = 0x006d_656c_6972_6174;

    const NARROW_CHARS: [char; 7] = ['a', 'Q', '7', ' ', '─', '#', 'é'];
    const WIDE_CHARS: [char; 3] = ['中', '日', '한'];

    fn random_color(rng: &mut XorShift) -> Color {
        match rng.below(4) {
            0 => Color::Default,
            1 => [
                Color::Black,
                Color::Red,
                Color::Green,
                Color::Yellow,
                Color::Blue,
                Color::Magenta,
                Color::Cyan,
                Color::White,
            ][rng.below(8) as usize],
            2 => Color::Byte(rng.byte()),
            _ => Color::Rgb(rng.byte(), rng.byte(), rng.byte()),
        }
    }

    fn random_attrs(rng: &mut XorShift) -> Attr {
        // Only the attrs with a ratatui `Modifier` counterpart:
        // UNDERCURL/FORCE_TEXT are bridge-dropped (pinned in
        // attr_modifier_conversion_matrix) and would break round-trip
        // equality if generated here.
        let mut attrs = Attr::DEFAULT;
        for bit in [
            Attr::BOLD,
            Attr::DIM,
            Attr::ITALICS,
            Attr::UNDERLINE,
            Attr::BLINK,
            Attr::REVERSE,
            Attr::HIDDEN,
        ] {
            if rng.below(2) == 0 {
                attrs |= bit;
            }
        }
        attrs
    }

    /// Fill a grid with mixed glyphs, colors and attributes, including wide
    /// chars whose following cell is a meli empty-continuation cell built
    /// exactly like `CellBuffer::write_string` builds it (same colors/attrs,
    /// blank glyph, `empty` set).
    fn fill_random_frame(grid: &mut CellBuffer, rng: &mut XorShift) {
        let (cols, rows) = (grid.cols, grid.rows);
        for y in 0..rows {
            let mut x = 0;
            while x < cols {
                let wide = rng.below(4) == 0;
                let ch = if wide {
                    WIDE_CHARS[rng.below(WIDE_CHARS.len() as u64) as usize]
                } else {
                    NARROW_CHARS[rng.below(NARROW_CHARS.len() as u64) as usize]
                };
                let (fg, bg) = (random_color(rng), random_color(rng));
                let attrs = random_attrs(rng);
                grid[(x, y)] = Cell::new(ch, fg, bg, attrs);
                if wide && x + 1 < cols {
                    grid[(x + 1, y)] = Cell::new(' ', fg, bg, attrs);
                    grid[(x + 1, y)].set_empty(true);
                    x += 1;
                }
                x += 1;
            }
        }
    }

    /// Every meli `Color` variant maps onto the ratatui color of the same
    /// actual terminal color, and the conversion is lossless in both
    /// directions for every meli color.
    #[test]
    fn color_conversion_matrix() {
        let cases: Vec<(&str, Color, RColor)> = vec![
            ("Default", Color::Default, RColor::Reset),
            ("Black", Color::Black, RColor::Black),
            ("Red", Color::Red, RColor::Red),
            ("Green", Color::Green, RColor::Green),
            ("Yellow", Color::Yellow, RColor::Yellow),
            ("Blue", Color::Blue, RColor::Blue),
            ("Magenta", Color::Magenta, RColor::Magenta),
            ("Cyan", Color::Cyan, RColor::Cyan),
            // meli's `White` is ANSI white (SGR 37), which ratatui calls
            // `Gray`; ratatui's `White` is bright white (SGR 97).
            ("White", Color::White, RColor::Gray),
            ("Byte(0)", Color::Byte(0), RColor::Indexed(0)),
            ("Byte(255)", Color::Byte(255), RColor::Indexed(255)),
            (
                "Rgb",
                Color::Rgb(0xde, 0xad, 0xbe),
                RColor::Rgb(0xde, 0xad, 0xbe),
            ),
        ];
        for (name, meli, rat) in &cases {
            let got: RColor = (*meli).into();
            println!("case color_conversion_matrix[{name}]: {meli:?} -> {got:?} (want {rat:?})");
            assert_eq!(got, *rat, "case {name} failed");
            let back: Color = got.into();
            assert_eq!(back, *meli, "round trip of {name} failed");
        }
        for b in 0..=u8::MAX {
            let meli = Color::Byte(b);
            let back: Color = RColor::from(meli).into();
            assert_eq!(back, meli, "Byte({b}) round trip failed");
        }
        println!(
            "color_conversion_matrix: {} named cases + 256 Byte-index round trips checked",
            cases.len()
        );
    }

    /// The inverse map covers every ratatui color: the six bright colors
    /// (SGR 90..97) become xterm 256-color indices 8..=15, which is exactly
    /// how meli's own writer expresses them.
    #[test]
    fn ratatui_color_reverse_matrix() {
        let cases: Vec<(&str, RColor, Color)> = vec![
            ("Reset", RColor::Reset, Color::Default),
            ("Black", RColor::Black, Color::Black),
            ("Red", RColor::Red, Color::Red),
            ("Green", RColor::Green, Color::Green),
            ("Yellow", RColor::Yellow, Color::Yellow),
            ("Blue", RColor::Blue, Color::Blue),
            ("Magenta", RColor::Magenta, Color::Magenta),
            ("Cyan", RColor::Cyan, Color::Cyan),
            ("Gray", RColor::Gray, Color::White),
            ("DarkGray", RColor::DarkGray, Color::Byte(8)),
            ("LightRed", RColor::LightRed, Color::Byte(9)),
            ("LightGreen", RColor::LightGreen, Color::Byte(10)),
            ("LightYellow", RColor::LightYellow, Color::Byte(11)),
            ("LightBlue", RColor::LightBlue, Color::Byte(12)),
            ("LightMagenta", RColor::LightMagenta, Color::Byte(13)),
            ("LightCyan", RColor::LightCyan, Color::Byte(14)),
            ("White", RColor::White, Color::Byte(15)),
            ("Indexed", RColor::Indexed(77), Color::Byte(77)),
            ("Rgb", RColor::Rgb(1, 2, 3), Color::Rgb(1, 2, 3)),
        ];
        for (name, rat, meli) in &cases {
            let got: Color = (*rat).into();
            println!(
                "case ratatui_color_reverse_matrix[{name}]: {rat:?} -> {got:?} (want {meli:?})"
            );
            assert_eq!(got, *meli, "case {name} failed");
        }
        println!(
            "ratatui_color_reverse_matrix: {} cases checked",
            cases.len()
        );
    }

    /// Attributes map bit-for-bit onto ratatui modifiers; the attrs without
    /// a counterpart are dropped in each direction (pinned here).
    #[test]
    fn attr_modifier_conversion_matrix() {
        let pairs = [
            ("BOLD", Attr::BOLD, RModifier::BOLD),
            ("DIM", Attr::DIM, RModifier::DIM),
            ("ITALICS", Attr::ITALICS, RModifier::ITALIC),
            ("UNDERLINE", Attr::UNDERLINE, RModifier::UNDERLINED),
            ("BLINK", Attr::BLINK, RModifier::SLOW_BLINK),
            ("REVERSE", Attr::REVERSE, RModifier::REVERSED),
            ("HIDDEN", Attr::HIDDEN, RModifier::HIDDEN),
        ];
        for (name, attr, modifier) in pairs {
            let got: RModifier = attr.into();
            assert_eq!(got, modifier, "case {name} failed");
            let back: Attr = modifier.into();
            assert_eq!(back, attr, "reverse of {name} failed");
        }
        let combo = Attr::BOLD | Attr::ITALICS | Attr::REVERSE;
        let got: RModifier = combo.into();
        assert_eq!(
            got,
            RModifier::BOLD | RModifier::ITALIC | RModifier::REVERSED,
            "combination"
        );
        assert_eq!(Attr::from(got), combo, "combination reverse");
        // meli-only attrs are dropped forward.
        assert_eq!(RModifier::from(Attr::UNDERCURL), RModifier::empty());
        assert_eq!(RModifier::from(Attr::FORCE_TEXT), RModifier::empty());
        assert_eq!(
            RModifier::from(Attr::UNDERCURL | Attr::FORCE_TEXT | Attr::BOLD),
            RModifier::BOLD,
            "dropped attrs must not mask shared ones"
        );
        // ratatui-only modifiers are dropped in reverse.
        assert_eq!(Attr::from(RModifier::RAPID_BLINK), Attr::DEFAULT);
        assert_eq!(Attr::from(RModifier::CROSSED_OUT), Attr::DEFAULT);
        assert_eq!(
            Attr::from(RModifier::SLOW_BLINK | RModifier::CROSSED_OUT),
            Attr::BLINK
        );
        println!(
            "attr_modifier_conversion_matrix: {} pairs + combinations + drop cases checked",
            pairs.len()
        );
    }

    /// `ThemeAttribute` becomes a ratatui `Style` with fg/bg/modifier set;
    /// `UNDERCURL`/`FORCE_TEXT` are intentionally not representable.
    #[test]
    fn theme_attribute_to_style_conversion() {
        let attr = ThemeAttribute {
            fg: Color::Green,
            bg: Color::Byte(240),
            attrs: Attr::BOLD | Attr::UNDERLINE,
        };
        let style = RStyle::from(attr);
        assert_eq!(style.fg, Some(RColor::Green));
        assert_eq!(style.bg, Some(RColor::Indexed(240)));
        assert_eq!(style.add_modifier, RModifier::BOLD | RModifier::UNDERLINED);
        assert!(style.sub_modifier.is_empty());

        // UNDERCURL and FORCE_TEXT have no ratatui modifier: the flush layer
        // reads those from the meli `Cell` attrs directly, so dropping them
        // from the style is by design.
        let attr = ThemeAttribute {
            fg: Color::Default,
            bg: Color::Default,
            attrs: Attr::UNDERCURL | Attr::FORCE_TEXT | Attr::DIM,
        };
        let style = RStyle::from(attr);
        assert_eq!(style.add_modifier, RModifier::DIM);

        // meli's `Color::Default` is the explicit terminal default, not an
        // unspecified color, so it maps to `Reset` rather than `None`.
        let style = RStyle::from(ThemeAttribute::default());
        assert_eq!(style.fg, Some(RColor::Reset));
        assert_eq!(style.bg, Some(RColor::Reset));
        assert_eq!(style.add_modifier, RModifier::empty());
        println!("theme_attribute_to_style_conversion: 3 cases checked");
    }

    /// Forward blit: symbol/fg/bg/modifier per cell, underline color reset,
    /// meli empty-continuation cells become ratatui `Skip` cells.
    #[test]
    fn blit_cellbuffer_to_buffer_basics() {
        let mut screen = Screen::<Virtual>::new(ThemeAttribute::default());
        assert!(screen.resize(6, 3));
        let grid = screen.grid_mut();
        grid[(0, 0)] = Cell::new('中', Color::Rgb(1, 2, 3), Color::Byte(200), Attr::BOLD);
        grid[(1, 0)] = Cell::new(' ', Color::Rgb(1, 2, 3), Color::Byte(200), Attr::BOLD);
        grid[(1, 0)].set_empty(true);
        grid[(2, 0)] = Cell::new(
            'x',
            Color::White,
            Color::Default,
            Attr::UNDERLINE | Attr::REVERSE,
        );
        grid[(0, 2)] = Cell::new('y', Color::Default, Color::Default, Attr::DEFAULT);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 6, 3));
        blit_cellbuffer_to_buffer(screen.grid(), &mut buffer);

        let wide = buffer.cell((0, 0)).unwrap();
        assert_eq!(wide.symbol(), "中");
        assert_eq!(wide.fg, RColor::Rgb(1, 2, 3));
        assert_eq!(wide.bg, RColor::Indexed(200));
        assert_eq!(wide.modifier, RModifier::BOLD);
        assert_eq!(wide.underline_color, RColor::Reset);
        assert_eq!(wide.diff_option, CellDiffOption::None);

        let cont = buffer.cell((1, 0)).unwrap();
        assert_eq!(
            cont.diff_option,
            CellDiffOption::Skip,
            "meli empty continuation cell must map to ratatui Skip"
        );
        assert_eq!(cont.symbol(), " ");
        assert_eq!(cont.fg, RColor::Rgb(1, 2, 3));

        let styled = buffer.cell((2, 0)).unwrap();
        assert_eq!(
            styled.fg,
            RColor::Gray,
            "meli White is ANSI 37 = ratatui Gray"
        );
        assert_eq!(styled.bg, RColor::Reset);
        assert_eq!(styled.modifier, RModifier::UNDERLINED | RModifier::REVERSED);

        let plain = buffer.cell((0, 2)).unwrap();
        assert_eq!(plain.fg, RColor::Reset);
        assert_eq!(plain.bg, RColor::Reset);
        assert_eq!(plain.modifier, RModifier::empty());

        println!("blit_cellbuffer_to_buffer_basics: 4 cells pinned");
    }

    /// Reverse blit: ratatui cells land in the meli grid; `Skip` becomes the
    /// meli empty-continuation flag.
    #[test]
    fn blit_buffer_to_cellbuffer_basics() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 3, 2));
        {
            let c = buffer.cell_mut((0, 0)).unwrap();
            c.set_char('A');
            c.fg = RColor::LightRed;
            c.bg = RColor::Indexed(42);
            c.modifier = RModifier::DIM | RModifier::ITALIC;
        }
        {
            let c = buffer.cell_mut((1, 0)).unwrap();
            // Multi-codepoint grapheme cluster: the meli cell holds a single
            // char, so the cluster collapses to its first char.
            c.set_symbol("e\u{301}");
            c.fg = RColor::Gray;
        }
        {
            let c = buffer.cell_mut((2, 0)).unwrap();
            c.set_char('S');
            c.set_diff_option(CellDiffOption::Skip);
        }
        // (0, 1) stays a default cell.
        let mut screen = Screen::<Virtual>::new(ThemeAttribute::default());
        assert!(screen.resize(3, 2));
        blit_buffer_to_cellbuffer(&buffer, screen.grid_mut());
        let grid = screen.grid();
        let a = grid.get(0, 0).unwrap();
        assert_eq!(a.ch(), 'A');
        assert_eq!(a.fg(), Color::Byte(9), "ratatui LightRed -> xterm 9");
        assert_eq!(a.bg(), Color::Byte(42));
        assert_eq!(a.attrs(), Attr::DIM | Attr::ITALICS);
        assert!(!a.empty());
        let g = grid.get(1, 0).unwrap();
        assert_eq!(g.ch(), 'e');
        assert_eq!(g.fg(), Color::White, "ratatui Gray -> meli White");
        let s = grid.get(2, 0).unwrap();
        assert_eq!(s.ch(), 'S');
        assert!(
            s.empty(),
            "ratatui Skip must map to a meli empty continuation cell"
        );
        assert_eq!(s.attrs(), Attr::DEFAULT);
        let d = grid.get(0, 1).unwrap();
        assert_eq!(d.ch(), ' ');
        assert_eq!(d.fg(), Color::Default);
        println!("blit_buffer_to_cellbuffer_basics: 4 cells pinned");
    }

    /// 100 random frames round-trip `CellBuffer -> Buffer -> CellBuffer`
    /// with full equality, wide-char continuation cells included. The RNG
    /// seed is fixed, so every run exercises the same 100 frames.
    #[test]
    fn blit_roundtrip_100_random_frames_property() {
        let mut rng = XorShift(PROPERTY_SEED);
        const FRAMES: usize = 100;
        let mut frames_run = 0usize;
        let mut cells_compared = 0usize;
        let mut continuation_cells = 0usize;
        for frame in 0..FRAMES {
            let cols = 1 + rng.below(37) as usize;
            let rows = 1 + rng.below(13) as usize;
            let mut screen = Screen::<Virtual>::new(ThemeAttribute::default());
            assert!(screen.resize(cols, rows), "resize to {cols}x{rows}");
            fill_random_frame(screen.grid_mut(), &mut rng);
            let orig = screen.grid().clone();
            let mut buffer = Buffer::empty(Rect::new(0, 0, cols as u16, rows as u16));
            blit_cellbuffer_to_buffer(&orig, &mut buffer);
            let mut roundtrip = orig.clone();
            blit_buffer_to_cellbuffer(&buffer, &mut roundtrip);
            // Readable per-cell contract first, so a failure names the exact
            // frame and cell instead of the opaque CellBuffer Debug.
            for y in 0..rows {
                for x in 0..cols {
                    let (a, b) = (orig.get(x, y).unwrap(), roundtrip.get(x, y).unwrap());
                    let where_ = format!("frame {frame} ({cols}x{rows}) cell ({x},{y})");
                    assert_eq!(a.ch(), b.ch(), "{where_} ch");
                    assert_eq!(a.fg(), b.fg(), "{where_} fg");
                    assert_eq!(a.bg(), b.bg(), "{where_} bg");
                    assert_eq!(a.attrs(), b.attrs(), "{where_} attrs");
                    assert_eq!(a.empty(), b.empty(), "{where_} empty");
                    cells_compared += 1;
                    continuation_cells += usize::from(a.empty());
                }
            }
            // Full structural equality: metadata (hyperlink tables,
            // default_cell, flags) plus every whole cell.
            assert_eq!(
                roundtrip, orig,
                "frame {frame} ({cols}x{rows}) full CellBuffer equality"
            );
            frames_run += 1;
        }
        assert_eq!(frames_run, FRAMES, "all property frames must run");
        assert!(
            continuation_cells > 0,
            "generator failed to produce wide-char continuation cells"
        );
        println!(
            "blit_roundtrip_100_random_frames_property: {FRAMES} frames ({frames_run} ran), \
             {cells_compared} cells compared, {continuation_cells} wide-char continuation \
             cells, fixed seed 0x{PROPERTY_SEED:016x}"
        );
    }

    /// Mismatched sizes never panic: both blits copy only the intersection.
    #[test]
    fn blit_clamps_mismatched_sizes() {
        // meli 5x3 -> ratatui 3x5: only the 3x3 intersection is copied.
        let mut src_screen = Screen::<Virtual>::new(ThemeAttribute::default());
        assert!(src_screen.resize(5, 3));
        for y in 0..3 {
            for x in 0..5 {
                src_screen.grid_mut()[(x, y)] = Cell::with_char('S');
            }
        }
        let mut buffer = Buffer::empty(Rect::new(0, 0, 3, 5));
        blit_cellbuffer_to_buffer(src_screen.grid(), &mut buffer);
        assert_eq!(buffer.cell((2, 2)).unwrap().symbol(), "S");
        assert_eq!(
            buffer.cell((0, 3)).unwrap().symbol(),
            " ",
            "rows beyond the meli buffer's height stay untouched"
        );
        assert_eq!(
            buffer.cell((3, 0)),
            None,
            "columns beyond the ratatui buffer's width are out of its area"
        );

        // ratatui 3x5 -> meli 5x3: same intersection rule.
        for y in 0..5 {
            for x in 0..3 {
                let c = buffer.cell_mut((x, y)).unwrap();
                c.set_char('B');
            }
        }
        let mut dst_screen = Screen::<Virtual>::new(ThemeAttribute::default());
        assert!(dst_screen.resize(5, 3));
        for cell in dst_screen.grid_mut().cellvec_mut() {
            *cell = Cell::with_char('D');
        }
        blit_buffer_to_cellbuffer(&buffer, dst_screen.grid_mut());
        assert_eq!(dst_screen.grid().get(2, 2).unwrap().ch(), 'B');
        assert_eq!(
            dst_screen.grid().get(3, 0).unwrap().ch(),
            'D',
            "columns beyond the ratatui buffer's width stay untouched"
        );
        assert_eq!(
            dst_screen.grid().get(0, 0).unwrap().ch(),
            'B',
            "intersection itself must be copied"
        );
        println!("blit_clamps_mismatched_sizes: both directions clamped to the intersection");
    }

    /// Degenerate shapes: 0x0 buffers are a no-op and 1x1 round-trips.
    #[test]
    fn blit_empty_and_1x1_buffers() {
        // 0x0 buffers: no-op, no panic.
        let nil_cb = CellBuffer::nil(Area::new_empty(ScreenGeneration::NIL));
        let mut zero = Buffer::empty(Rect::ZERO);
        blit_cellbuffer_to_buffer(&nil_cb, &mut zero);
        assert_eq!(zero.content().len(), 0);
        let mut nil_dst = CellBuffer::nil(Area::new_empty(ScreenGeneration::NIL));
        blit_buffer_to_cellbuffer(&zero, &mut nil_dst);
        assert!(nil_dst.is_empty());

        // 1x1 round trip.
        let mut screen = Screen::<Virtual>::new(ThemeAttribute::default());
        assert!(screen.resize(1, 1));
        screen.grid_mut()[(0, 0)] =
            Cell::new('q', Color::Byte(123), Color::Cyan, Attr::BOLD | Attr::BLINK);
        let orig = screen.grid().clone();
        let mut buffer = Buffer::empty(Rect::new(0, 0, 1, 1));
        blit_cellbuffer_to_buffer(&orig, &mut buffer);
        let mut roundtrip = orig.clone();
        blit_buffer_to_cellbuffer(&buffer, &mut roundtrip);
        assert_eq!(roundtrip, orig);
        assert_eq!(roundtrip.get(0, 0).unwrap().ch(), 'q');
        println!("blit_empty_and_1x1_buffers: 0x0 no-op + 1x1 round trip ok");
    }

    /// OSC8 hyperlink tables live in `CellBuffer` metadata; blits must leave
    /// them untouched (the ratatui side has no hyperlink representation).
    #[test]
    fn blit_leaves_hyperlink_metadata_untouched() {
        let mut screen = Screen::<Virtual>::new(ThemeAttribute::default());
        assert!(screen.resize(4, 2));
        {
            let grid = screen.grid_mut();
            grid.hyperlinks_table
                .insert(7, "https://meli.example".into());
            grid.hyperlinks_associations.insert((0, 0), (7, (3, 0)));
        }
        let orig = screen.grid().clone();
        let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 2));
        blit_cellbuffer_to_buffer(&orig, &mut buffer);
        let mut roundtrip = orig.clone();
        blit_buffer_to_cellbuffer(&buffer, &mut roundtrip);
        assert_eq!(roundtrip.hyperlinks_table, orig.hyperlinks_table);
        assert_eq!(
            roundtrip.hyperlinks_associations,
            orig.hyperlinks_associations
        );
        assert_eq!(roundtrip.hyperlinks_table.len(), 1);
        assert_eq!(roundtrip.hyperlinks_associations.len(), 1);
        assert_eq!(roundtrip.draw_hyperlinks, orig.draw_hyperlinks);
        println!("blit_leaves_hyperlink_metadata_untouched: tables intact after round trip");
    }

    /// Keep flags (`keep_fg`/`keep_bg`/`keep_attrs`) are neither copied nor
    /// cleared by a blit: they survive `blit_buffer_to_cellbuffer` and stay
    /// in force for ordinary setters afterwards.
    #[test]
    fn blit_preserves_keep_flags() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 1, 1));
        {
            let c = buffer.cell_mut((0, 0)).unwrap();
            c.set_char('k');
            c.fg = RColor::Green;
            c.bg = RColor::Blue;
            c.modifier = RModifier::BOLD;
        }
        let mut screen = Screen::<Virtual>::new(ThemeAttribute::default());
        assert!(screen.resize(1, 1));
        {
            let cell = &mut screen.grid_mut()[(0, 0)];
            cell.set_keep_fg(true)
                .set_keep_bg(true)
                .set_keep_attrs(true);
            cell.set_fg(Color::Red)
                .set_bg(Color::Yellow)
                .set_attrs(Attr::ITALICS);
        }
        blit_buffer_to_cellbuffer(&buffer, screen.grid_mut());
        {
            // The blit copies the source of truth wholesale: fg/bg/attrs
            // change despite the keep flags (a blit is not a styling
            // operation)...
            let after = screen.grid().get(0, 0).unwrap();
            assert_eq!(after.ch(), 'k');
            assert_eq!(after.fg(), Color::Green);
            assert_eq!(after.bg(), Color::Blue);
            assert_eq!(after.attrs(), Attr::BOLD);
        }
        {
            // ...but the keep flags are untouched: ordinary setters keep
            // being ignored until the glyph changes.
            let cell = &mut screen.grid_mut()[(0, 0)];
            cell.set_fg(Color::Black);
            cell.set_bg(Color::Black);
            cell.set_attrs(Attr::REVERSE);
            assert_eq!(cell.fg(), Color::Green, "keep_fg still active after blit");
            assert_eq!(cell.bg(), Color::Blue, "keep_bg still active after blit");
            assert_eq!(
                cell.attrs(),
                Attr::BOLD,
                "keep_attrs still active after blit"
            );
            // `set_ch` clears the keeps, proving they were present all along.
            cell.set_ch('z');
            cell.set_fg(Color::Black);
            assert_eq!(cell.fg(), Color::Black);
        }
        println!("blit_preserves_keep_flags: keep flags survive the blit");
    }

    /// `border_set_for` hands out the rounded set normally and a -/|/+ ASCII
    /// set matching meli's own `ascii_drawing` boundary glyphs.
    #[test]
    fn border_set_matches_meli_ascii_table() {
        assert_eq!(border_set_for(false), rborder::ROUNDED);
        let ascii = border_set_for(true);
        // Glyphs per meli's cells.rs `boundaries::Boundary::to_char(true)`
        // table: horizontal segments are '-', verticals '|', every
        // corner/joint '+' (FULL_HORZ 0b0101 -> '-', FULL_VERT 0b1010 ->
        // '|', 0b1111 -> '+').
        assert_eq!(ascii.horizontal_top, "-");
        assert_eq!(ascii.horizontal_bottom, "-");
        assert_eq!(ascii.vertical_left, "|");
        assert_eq!(ascii.vertical_right, "|");
        assert_eq!(ascii.top_left, "+");
        assert_eq!(ascii.top_right, "+");
        assert_eq!(ascii.bottom_left, "+");
        assert_eq!(ascii.bottom_right, "+");
        println!("border_set_matches_meli_ascii_table: rounded + ascii sets pinned");
    }

    /// `Area` -> `Rect` keeps the absolute position and size; `Rect` ->
    /// `Area` is anchored on a root area (meli areas are bounds-checked
    /// against a screen canvas) and never escapes it.
    #[test]
    fn area_rect_converters() {
        let mut screen = Screen::<Virtual>::new(ThemeAttribute::default());
        assert!(screen.resize(80, 24));
        let root = screen.area();

        // Full-screen area -> origin rect.
        assert_eq!(area_to_rect(root), Rect::new(0, 0, 80, 24));

        // Sub-area with offset -> rect keeps the absolute position.
        let sub = root.skip(10, 3).take(20, 5);
        assert_eq!(area_to_rect(sub), Rect::new(10, 3, 20, 5));

        // Rect -> Area anchored on a root: rect coordinates are absolute
        // screen positions (as produced by `area_to_rect`/`Layout::split`)
        // and the round trip is the identity for in-bounds rects.
        for (x, y, w, h) in [
            (0u16, 0u16, 80u16, 24u16),
            (5, 7, 13, 2),
            (79, 23, 1, 1),
            (40, 0, 40, 24),
        ] {
            let rect = Rect::new(x, y, w, h);
            let area = rect_to_area(rect, root);
            assert_eq!(area_to_rect(area), rect, "round trip for {rect}");
            assert!(root.contains(area), "converted area escapes root: {rect}");
        }

        // Round trip through an offset root (e.g. a Tabbed body starting at
        // row 1): absolute coordinates must not be re-applied as offsets.
        let offset_root = root.skip(3, 2).take(40, 10);
        let rect = Rect::new(10, 5, 7, 4);
        assert_eq!(area_to_rect(rect_to_area(rect, offset_root)), rect);

        // Out-of-root rects clamp to an empty area, never out of bounds.
        let beyond = rect_to_area(Rect::new(100, 30, 10, 10), root);
        assert!(beyond.is_empty());

        // Empty area -> zero rect.
        assert_eq!(
            area_to_rect(Area::new_empty(ScreenGeneration::NIL)),
            Rect::ZERO
        );
        println!("area_rect_converters: round trips + clamping pinned");
    }

    /// Todo 12 regression net: the `Layout`-carved top-level splits used by
    /// the chrome components must reproduce the legacy hand-rolled `Area`
    /// math exactly, for every size. The golden snapshots pin the rendered
    /// pixels at the recorded sizes; this pins the arithmetic itself.
    #[test]
    fn layout_carved_top_level_splits_match_legacy_area_math() {
        let mut screen = Screen::<Virtual>::new(ThemeAttribute::default());
        assert!(screen.resize(64, 48));
        let root = screen.area();

        // StatusBar: [Min(0), Length(k)] for the body/bar strip, then
        // [Length(k-1), Length(1)] inside the strip, equals take_rows(total-k)
        // / nth_row(total-k) / skip_rows(total-1). k is 1 in every mode
        // except Command (k = 2), where the command line is the strip's top
        // row (nth_row(total-k)); with k = 1 the command segment is
        // deliberately empty (the legacy nth_row(total-1) alias of the
        // status row is never drawn: the command bar only draws in Command
        // mode). k > 2 never occurs, so the command row is only pinned for
        // k <= 2.
        for total_rows in 0..=48usize {
            let area = root.take_rows(total_rows);
            for strip_rows in [1usize, 2, 3] {
                if total_rows <= strip_rows {
                    // StatusBar::draw returns early; no split must happen.
                    continue;
                }
                let [body, bar] =
                    Layout::vertical([Constraint::Min(0), Constraint::Length(strip_rows as u16)])
                        .areas(area_to_rect(area));
                assert_eq!(
                    rect_to_area(body, area),
                    area.take_rows(total_rows - strip_rows)
                );
                let bar_area = rect_to_area(bar, area);
                let [command, status] = Layout::vertical([
                    Constraint::Length(strip_rows.saturating_sub(1) as u16),
                    Constraint::Length(1),
                ])
                .areas(bar);
                assert_eq!(rect_to_area(status, area), area.skip_rows(total_rows - 1));
                assert_eq!(bar_area, area.skip_rows(total_rows - strip_rows));
                if strip_rows == 1 {
                    assert!(rect_to_area(command, area).is_empty());
                } else if strip_rows == 2 {
                    assert_eq!(
                        rect_to_area(command, area),
                        area.nth_row(total_rows - strip_rows)
                    );
                }
            }
        }

        // Tabbed: [Length(1), Min(0)] equals nth_row(0) / skip_rows(1).
        for total_rows in 0..=48usize {
            let area = root.take_rows(total_rows);
            let [tab_row, body] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)])
                .areas(area_to_rect(area));
            assert_eq!(rect_to_area(tab_row, area), area.nth_row(0));
            assert_eq!(rect_to_area(body, area), area.skip_rows(1));
        }

        // Listing: [Length(mid), Length(1), Min(0)] equals take_cols(mid) /
        // nth_col(mid) / skip_cols(mid + 1). The three-way split only runs
        // when both panes are visible, i.e. 1 <= mid < total.
        for total_cols in 1..=64usize {
            let area = root.take_cols(total_cols);
            for mid in 1..total_cols {
                let [menu, divider, right] = Layout::horizontal([
                    Constraint::Length(mid as u16),
                    Constraint::Length(1),
                    Constraint::Min(0),
                ])
                .areas(area_to_rect(area));
                assert_eq!(rect_to_area(menu, area), area.take_cols(mid));
                assert_eq!(rect_to_area(divider, area), area.nth_col(mid));
                assert_eq!(rect_to_area(right, area), area.skip_cols(mid + 1));
            }
        }
        println!("layout_carved_top_level_splits_match_legacy_area_math: pinned");
    }

    /// `center_inside_via_layout` must equal `Area::center_inside` for every
    /// area and box size, including over-large boxes (which clamp).
    #[test]
    fn center_inside_via_layout_matches_area_math() {
        let mut screen = Screen::<Virtual>::new(ThemeAttribute::default());
        assert!(screen.resize(32, 32));
        let root = screen.area();
        let check = |area: Area, width: usize, height: usize| {
            assert_eq!(
                center_inside_via_layout(area, (width, height)),
                area.center_inside((width, height)),
                "center {width}x{height} in {}x{}",
                area.width(),
                area.height()
            );
        };
        for area_width in 0..=16usize {
            for area_height in 0..=16usize {
                let area = root.take(area_width, area_height);
                for width in 0..=area_width + 6 {
                    for height in 0..=area_height + 6 {
                        check(area, width, height);
                    }
                }
            }
        }
        // Edge sweep at realistic chrome sizes, around the fitting boundary.
        for (area_width, area_height) in
            [(17usize, 40usize), (40, 17), (80, 24), (60, 20), (200, 50)]
        {
            let area = root.take(area_width, area_height);
            for width in [
                area_width - 3,
                area_width - 2,
                area_width - 1,
                area_width,
                area_width + 1,
                area_width + 2,
                area_width / 2,
            ]
            .into_iter()
            .chain([0, 3, 4])
            {
                for height in [
                    area_height - 3,
                    area_height - 2,
                    area_height - 1,
                    area_height,
                    area_height + 1,
                    area_height + 2,
                    area_height / 2,
                ]
                .into_iter()
                .chain([0, 3, 4])
                {
                    check(area, width, height);
                }
            }
        }
        println!("center_inside_via_layout_matches_area_math: pinned");
    }

    /// `place_inside_via_layout` must equal `Area::place_inside` for every
    /// area and box size, and every corner combination.
    #[test]
    fn place_inside_via_layout_matches_area_math() {
        let mut screen = Screen::<Virtual>::new(ThemeAttribute::default());
        assert!(screen.resize(32, 32));
        let root = screen.area();
        let check = |area: Area, width: usize, height: usize| {
            for (upper, left) in [(false, false), (false, true), (true, false), (true, true)] {
                assert_eq!(
                    place_inside_via_layout(area, (width, height), upper, left),
                    area.place_inside((width, height), upper, left),
                    "place {width}x{height} upper={upper} left={left} in {}x{}",
                    area.width(),
                    area.height()
                );
            }
        };
        for area_width in 0..=16usize {
            for area_height in 0..=16usize {
                let area = root.take(area_width, area_height);
                for width in 0..=area_width + 6 {
                    for height in 0..=area_height + 6 {
                        check(area, width, height);
                    }
                }
            }
        }
        // The OSD call shape: box sizes around the area size, all corners.
        for (area_width, area_height) in
            [(17usize, 40usize), (40, 17), (80, 24), (60, 20), (200, 50)]
        {
            let area = root.take(area_width, area_height);
            for width in [
                area_width - 3,
                area_width - 2,
                area_width - 1,
                area_width,
                area_width + 1,
                area_width + 2,
                area_width / 3,
            ] {
                for height in [
                    area_height - 3,
                    area_height - 2,
                    area_height - 1,
                    area_height,
                    area_height + 1,
                    area_height + 2,
                    area_height,
                ] {
                    check(area, width, height);
                }
            }
        }
        println!("place_inside_via_layout_matches_area_math: pinned");
    }

    /// The rounded frame draws the `border::ROUNDED` ring with the border
    /// style's fg/attrs, leaves interior cells untouched, and returns the
    /// same inner area `create_box` would.
    #[test]
    fn draw_rounded_frame_rounded_ring_and_inner() {
        let mut screen = Screen::<Virtual>::new(ThemeAttribute::default());
        assert!(screen.resize(10, 6));
        let area = screen.area();
        let border_attr = ThemeAttribute {
            fg: Color::Byte(123),
            bg: Color::Default,
            attrs: Attr::BOLD,
        };
        let inner = draw_rounded_frame(screen.grid_mut(), area, border_attr);
        assert_eq!(inner.upper_left(), (1, 1));
        assert_eq!(inner.width(), 8);
        assert_eq!(inner.height(), 4);

        let grid = screen.grid();
        assert_eq!(grid[(0, 0)].ch(), '╭');
        assert_eq!(grid[(9, 0)].ch(), '╮');
        assert_eq!(grid[(0, 5)].ch(), '╰');
        assert_eq!(grid[(9, 5)].ch(), '╯');
        assert_eq!(grid[(5, 0)].ch(), '─');
        assert_eq!(grid[(5, 5)].ch(), '─');
        assert_eq!(grid[(0, 3)].ch(), '│');
        assert_eq!(grid[(9, 3)].ch(), '│');
        for (x, y) in [(0, 0), (9, 0), (0, 5), (9, 5), (5, 0), (0, 3)] {
            assert_eq!(grid[(x, y)].fg(), Color::Byte(123), "fg at {x},{y}");
            assert_eq!(grid[(x, y)].attrs(), Attr::BOLD, "attrs at {x},{y}");
        }
        // Interior is untouched: default cell, no frame style.
        assert_eq!(grid[(5, 3)].ch(), ' ');
        assert_eq!(grid[(5, 3)].fg(), Color::Default);
        assert_eq!(grid[(5, 3)].attrs(), Attr::DEFAULT);
        println!("draw_rounded_frame_rounded_ring_and_inner: ring + inner pinned");
    }

    /// With `ascii_drawing` enabled the frame uses meli's ASCII boundary
    /// glyphs (`+`/`-`/`|`), matching `create_box`'s ASCII mode.
    #[test]
    fn draw_rounded_frame_ascii_set() {
        let mut screen = Screen::<Virtual>::new(ThemeAttribute::default());
        assert!(screen.resize(10, 6));
        screen.grid_mut().ascii_drawing = true;
        let area = screen.area();
        let _ = draw_rounded_frame(screen.grid_mut(), area, ThemeAttribute::default());
        let grid = screen.grid();
        assert_eq!(grid[(0, 0)].ch(), '+');
        assert_eq!(grid[(9, 0)].ch(), '+');
        assert_eq!(grid[(0, 5)].ch(), '+');
        assert_eq!(grid[(9, 5)].ch(), '+');
        assert_eq!(grid[(5, 0)].ch(), '-');
        assert_eq!(grid[(5, 5)].ch(), '-');
        assert_eq!(grid[(0, 3)].ch(), '|');
        assert_eq!(grid[(9, 3)].ch(), '|');
        println!("draw_rounded_frame_ascii_set: ascii glyphs pinned");
    }

    /// Areas too small to hold a frame are no-ops, and
    /// `frame_ring_areas` degenerates to empty strips without panicking.
    #[test]
    fn draw_rounded_frame_tiny_area_noop() {
        let mut screen = Screen::<Virtual>::new(ThemeAttribute::default());
        assert!(screen.resize(4, 4));
        let before = screen.grid().clone();
        let area = screen.area().take(1, 1);
        let ret = draw_rounded_frame(screen.grid_mut(), area, ThemeAttribute::default());
        assert_eq!(ret, area, "tiny area returned unchanged");
        assert_eq!(screen.grid(), &before, "tiny area draw must not write");

        for strip in frame_ring_areas(area) {
            assert!(
                area.contains(strip) || strip.is_empty(),
                "ring strips must stay within the area"
            );
        }
        println!("draw_rounded_frame_tiny_area_noop: guards pinned");
    }
}
