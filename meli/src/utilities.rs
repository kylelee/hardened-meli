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

//! Various useful utilities.

use std::collections::{HashSet, VecDeque};

use indexmap::IndexMap;
use melib::{text::Reflow, ShellExpandTrait};
use ratatui::layout::{Constraint, Layout};

use super::*;
use crate::{
    accounts::MailboxStatus, components::ExtendShortcutsMaps, jobs::JobId,
    melib::text::TextProcessing,
};

mod pager;
pub use self::pager::*;

mod text;
pub use self::text::*;

mod widgets;
pub use self::widgets::*;

mod dialogs;
pub use self::dialogs::*;

mod tables;
pub use self::tables::*;

#[cfg(test)]
pub mod tests;

pub type AutoCompleteFn = Box<dyn Fn(&Context, &str) -> Vec<AutoCompleteEntry> + Send + Sync>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SearchMovement {
    Previous,
    #[default]
    Next,
    First,
    Last,
}

#[derive(Clone, Debug, Default)]
pub struct SearchPattern {
    pattern: String,
    positions: Vec<(usize, usize)>,
    cursor: usize,
    movement: Option<SearchMovement>,
}

/// Status bar.
#[derive(Debug)]
pub struct StatusBar {
    container: Box<dyn Component>,
    status_message: String,
    substatus_message: String,
    ex_buffer: TextField,
    ex_buffer_cmd_history_pos: Option<usize>,
    display_buffer: String,
    mode: UIMode,
    mouse: bool,
    height: usize,
    dirty: bool,
    id: ComponentId,
    progress_spinner: ProgressSpinner,
    in_progress_jobs: HashSet<JobId>,
    done_jobs: HashSet<JobId>,
    /// Mailbox the active listing child has focused on, if any. Drives the
    /// central `LineGauge` (whose ratio is the mailbox's
    /// `MailboxStatus::Parsing(done, total)`) and narrows
    /// `AccountStatusChange`/`MailboxUpdate` redraw arms to that exact
    /// mailbox. `None` whenever no listing owns the focus.
    focus: Option<(AccountHash, MailboxHash)>,

    /// Unseen-count floor for the focused mailbox: tracks how far the
    /// user has read mail down since the mailbox was focused, so the
    /// `📨 new` count in the status bar only surfaces mail that arrived
    /// after focusing (and not unseen mail that existed before).
    unseen_floor: usize,
    /// Mailbox total seen at the last settled (non-parsing) observation
    /// of the focused mailbox; `None` when the mailbox has not been
    /// observed settled, or the focus changed. Classifies an incoming
    /// parse session as the initial fetch (settled total was 0) versus
    /// an incremental sync (settled total was > 0).
    settled_total: Option<usize>,
    /// Classification of the focused mailbox's current parse session:
    /// `Some(true)` = initial fetch — arrivals are absorbed into the
    /// floor and never surface as `📨 new`; `Some(false)` = incremental
    /// sync — arrivals surface immediately. `None` while no parse
    /// session is active.
    parse_first_fetch: Option<bool>,
    auto_complete: Box<AutoComplete>,
    cmd_history: Vec<String>,
}

impl std::fmt::Display for StatusBar {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "status bar")
    }
}

impl StatusBar {
    const MOUSE_MODE: &str = "🖱️ ";
    const MOUSE_MODE_ASCII: &str = "(mouse)";

    pub fn new(context: &Context, container: Box<dyn Component>) -> Self {
        let progress_spinner = Self::make_progress_spinner(context);

        Self {
            container,
            status_message: String::with_capacity(256),
            substatus_message: String::with_capacity(256),
            ex_buffer: TextField::new(UText::new(String::with_capacity(256)), None),
            ex_buffer_cmd_history_pos: None,
            display_buffer: String::with_capacity(8),
            dirty: true,
            mode: UIMode::Normal,
            mouse: context.settings.terminal.use_mouse.is_true(),
            height: 1,
            id: ComponentId::default(),
            auto_complete: AutoComplete::new(Vec::new()),
            progress_spinner,
            in_progress_jobs: HashSet::default(),
            done_jobs: HashSet::default(),
            focus: None,
            unseen_floor: 0,
            settled_total: None,
            parse_first_fetch: None,
            cmd_history: crate::command::history::old_cmd_history(),
        }
    }

    /// Build the mailbox-status carousel: the `ProgressSpinner` engine
    /// cycling through envelope glyphs while network refresh work is in
    /// flight. Defaults to the six-glyph mailbox carousel (the classic
    /// `|/-\` on ASCII terminals); `progress_spinner_sequence` overrides
    /// the frames as before.
    fn make_progress_spinner(context: &Context) -> ProgressSpinner {
        let mut progress_spinner = ProgressSpinner::new(20, context);
        match context.settings.terminal.progress_spinner_sequence.as_ref() {
            Some(conf::terminal::ProgressSpinnerSequence::Integer(k)) => {
                progress_spinner.set_kind(*k);
            }
            Some(conf::terminal::ProgressSpinnerSequence::Custom {
                ref frames,
                ref interval_ms,
            }) => {
                progress_spinner.set_custom_kind(frames.clone(), *interval_ms);
            }
            None => {
                let frames: Vec<String> = if context.settings.terminal.emoji_capable() {
                    // Six mailbox glyphs: one frame per 120 ms tick.
                    ["📨", "📬", "📪", "📭", "📩", "📫"]
                        .iter()
                        .map(|s| (*s).to_string())
                        .collect()
                } else if context.settings.terminal.ascii_drawing {
                    // Plain-ASCII fallback for terminals without unicode
                    // braille or emoji support.
                    ["|", "/", "-", "\\", "|", "/"]
                        .iter()
                        .map(|s| (*s).to_string())
                        .collect()
                } else {
                    // Braille-pattern carousel — the user's preferred
                    // unicode-but-not-emoji analogue. 10 frames at
                    // 120 ms ≈ 1.2 s per rotation.
                    ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]
                        .iter()
                        .map(|s| (*s).to_string())
                        .collect()
                };
                progress_spinner.set_custom_kind(frames, 120);
            }
        }
        progress_spinner
    }

    fn draw_status_bar(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        let mut attribute = crate::conf::value(context, "status.bar");
        if !context.settings.terminal.use_color() {
            attribute.attrs |= Attr::REVERSE;
        }
        grid.clear_area(area, attribute);
        /* The row is one flowing line from column 0:
         * [mailbox-status icon] | [mail counts] [transient message]
         * [scroll %] | [gauge while fetching] | [keyboard hints], with
         * the remaining columns left blank — the hints follow the
         * counts left-aligned instead of hugging the right edge. The
         * gauge width comes from the focused mailbox's
         * `MailboxStatus::Parsing(done, total)`; `write_string` returns
         * the x relative to the area it was passed, so each chained
         * write accumulates into `x`. */
        let (gauge_label, gauge_w) = self.gauge_metrics(context);
        let (hints_spans, hints_w) = self.hints_metrics(context);
        let left_area = area;
        /* Left segment, laid out from column 0: mode chip (only outside
         * Normal mode, where the mode carries information — Insert
         * editing, Command line, Embedded terminal, Fork), mouse flag,
         * backend chip, mailbox label, transient status/substatus
         * message and the scroll percentage. The legacy
         * `NORMAL | Mailbox: …` line is gone: `Listing` no longer
         * reports a redundant status string (its `Component::status`
         * override was removed), so the focus chips own the left edge.
         * `write_string` returns the x relative to the area it was
         * passed, so each chained write accumulates into `x`. */
        let mut x = if self.mode != UIMode::Normal {
            let mode_str = self.mode.to_string();
            let mode_attribute = Self::mode_indicator_attrs(self.mode, context);
            let (x_rel, _) = grid.write_string(
                &mode_str,
                mode_attribute.fg,
                mode_attribute.bg,
                mode_attribute.attrs,
                left_area,
                None,
                None,
            );
            x_rel
        } else {
            0
        };
        if self.mouse {
            let alt = if context.settings.terminal.ascii_drawing {
                Self::MOUSE_MODE_ASCII
            } else {
                Self::MOUSE_MODE
            };
            let flag = context
                .settings
                .terminal
                .mouse_flag
                .as_deref()
                .unwrap_or(alt);
            // Two writes instead of `format!(" {flag}")`: the status bar is
            // redrawn on every keystroke and every spinner tick.
            let (x_rel, _) = grid.write_string(
                " ",
                attribute.fg,
                attribute.bg,
                attribute.attrs,
                left_area.skip_cols(x),
                None,
                None,
            );
            x += x_rel;
            let (x_rel, _) = grid.write_string(
                flag,
                attribute.fg,
                attribute.bg,
                attribute.attrs,
                left_area.skip_cols(x),
                None,
                None,
            );
            x += x_rel;
        }
        // Mailbox status icon: ✘ when the focused account is offline,
        // the carousel (the spinner engine, mailbox envelope glyphs)
        // while network refresh work is in flight, and a fixed 📫 when
        // idle. Without emoji, the fallback is `#`; plain ASCII keeps
        // `*` (matches the legacy spinner-fallback glyph).
        if self.focus_offline(context) {
            let glyph = if context.settings.terminal.emoji_capable() {
                "✘"
            } else {
                "!"
            };
            let (x_rel, _) = grid.write_string(
                glyph,
                attribute.fg,
                attribute.bg,
                attribute.attrs | Attr::BOLD,
                left_area.skip_cols(x),
                None,
                None,
            );
            x += x_rel;
        } else if self.progress_spinner.is_active() {
            self.progress_spinner.set_dirty(true);
            self.progress_spinner
                .draw(grid, left_area.skip_cols(x), context);
            x += self.progress_spinner.width;
        } else if self.focus.is_some() {
            let glyph = if context.settings.terminal.emoji_capable() {
                "📫"
            } else {
                "#"
            };
            let (x_rel, _) = grid.write_string(
                glyph,
                attribute.fg,
                attribute.bg,
                attribute.attrs,
                left_area.skip_cols(x),
                None,
                None,
            );
            x += x_rel;
        }
        // Mail counts for the focused mailbox: `📨new 📩unread 📧total`
        // (`📧` instead of `✉️`: the dingbat needs an emoji-presentation
        // variation selector that the cell-based grid cannot carry, so
        // it rendered text-style). `new` counts unseen mail that arrived
        // since the mailbox was focused; the floor follows the user
        // reading mail down, so only genuinely new arrivals surface.
        if let Some((new, unseen, total)) = self.focus_counts(context) {
            let counts = if context.settings.terminal.emoji_capable() {
                format!("📨{new} 📩{unseen} 📧{total}")
            } else if context.settings.terminal.ascii_drawing {
                format!("new:{new} unread:{unseen} total:{total}")
            } else {
                // Braille analogue: 📨 → `+`, 📩 → `~`, 📧 → `=`.
                format!("+{new} ~{unseen} ={total}")
            };
            let (x_rel, _) = grid.write_string(
                " | ",
                attribute.fg,
                attribute.bg,
                attribute.attrs,
                left_area.skip_cols(x),
                None,
                None,
            );
            x += x_rel;
            let (x_rel, _) = grid.write_string(
                &counts,
                attribute.fg,
                attribute.bg,
                attribute.attrs,
                left_area.skip_cols(x),
                None,
                None,
            );
            x += x_rel;
        }
        if !self.status_message.is_empty() || !self.substatus_message.is_empty() {
            // Written piecewise (leading space, first part, optional
            // separator + second part) instead of building a `String` and
            // `format!(" {message}")` on every redraw.
            let (x_rel, _) = grid.write_string(
                " ",
                attribute.fg,
                attribute.bg,
                attribute.attrs,
                left_area.skip_cols(x),
                None,
                None,
            );
            x += x_rel;
            if !self.status_message.is_empty() {
                let (x_rel, _) = grid.write_string(
                    &self.status_message,
                    attribute.fg,
                    attribute.bg,
                    attribute.attrs,
                    left_area.skip_cols(x),
                    None,
                    None,
                );
                x += x_rel;
            }
            if !self.substatus_message.is_empty() {
                if !self.status_message.is_empty() {
                    let (x_rel, _) = grid.write_string(
                        " | ",
                        attribute.fg,
                        attribute.bg,
                        attribute.attrs,
                        left_area.skip_cols(x),
                        None,
                        None,
                    );
                    x += x_rel;
                }
                let (x_rel, _) = grid.write_string(
                    &self.substatus_message,
                    attribute.fg,
                    attribute.bg,
                    attribute.attrs,
                    left_area.skip_cols(x),
                    None,
                    None,
                );
                x += x_rel;
            }
        }

        if gauge_w > 0 {
            let (x_rel, _) = grid.write_string(
                " |",
                attribute.fg,
                attribute.bg,
                attribute.attrs | Attr::BOLD,
                left_area.skip_cols(x),
                None,
                None,
            );
            x += x_rel;
            let gauge_area = left_area.skip_cols(x).take_cols(gauge_w);
            self.render_line_gauge(grid, gauge_area, context, &gauge_label);
            x += gauge_w;
        }

        if hints_w > 0 {
            let (x_rel, _) = grid.write_string(
                " |",
                attribute.fg,
                attribute.bg,
                attribute.attrs | Attr::BOLD,
                left_area.skip_cols(x),
                None,
                None,
            );
            x += x_rel;
            // Key glyphs render in the theme's highlight-selected
            // accent color (the `...highlighted_selected` background)
            // so the binding pops out of the descriptive label text
            // and follows the active theme.
            let selected = crate::conf::value(context, "mail.listing.compact.highlighted_selected");
            let key_fg = selected.bg;
            for span in &hints_spans {
                let (fg, attrs) = if span.key {
                    (key_fg, attribute.attrs | Attr::BOLD)
                } else {
                    (attribute.fg, attribute.attrs)
                };
                let (dx, _) = grid.write_string(
                    &span.text,
                    fg,
                    attribute.bg,
                    attrs,
                    left_area.skip_cols(x),
                    None,
                    None,
                );
                if dx == 0 {
                    break; // row exhausted; remaining spans are clipped
                }
                x += dx;
            }
        }
        let skip = left_area
            .width()
            .saturating_sub(self.display_buffer.len() + 1);
        grid.write_string(
            &self.display_buffer,
            attribute.fg,
            attribute.bg,
            attribute.attrs,
            left_area.skip_cols(skip),
            None,
            None,
        );

        context.dirty_areas.push_back(area);
    }

    /// Whether the focused account's connection is in an error state
    /// (drives the `✘` status icon).
    fn focus_offline(&self, context: &Context) -> bool {
        let Some((acc_hash, _)) = self.focus else {
            return false;
        };
        context
            .accounts
            .get_index_of(&acc_hash)
            .is_some_and(|i| context.accounts[i].is_online.is_err())
    }

    /// `(unseen, total)` for `mb_hash` from the account's collection of
    /// loaded envelopes — the same envelopes the listing renders. The
    /// backend's mailbox metadata (`ref_mailbox.count()`) can drift from
    /// what the user sees: IMAP only populates its unseen set when the
    /// server reports it, so the collection is the source of truth.
    /// Missing mailbox → `None`.
    fn collection_unseen_total(
        collection: &melib::Collection,
        mb_hash: MailboxHash,
    ) -> Option<(usize, usize)> {
        let mailboxes = collection.mailboxes.read().ok()?;
        // A mailbox not (yet) in the collection simply has no loaded
        // envelopes — that is (0, 0), not "no data".
        let Some(envs) = mailboxes.get(&mb_hash) else {
            return Some((0, 0));
        };
        let total = envs.len();
        // Count inside a single `envelopes` read guard. Calling
        // `Collection::get_env` per envelope took the same read lock once
        // per envelope, so every status-bar redraw (one per keystroke, and
        // once per spinner tick while a fetch runs) cost `len` lock round
        // trips on the UI thread. The lock order `mailboxes` → `envelopes`
        // matches `Collection`'s own (`threads` → `mailboxes` →
        // `envelopes`).
        let envelopes = collection.envelopes.read().ok()?;
        let unseen = envs
            .iter()
            .filter(|env_hash| envelopes.get(*env_hash).is_some_and(|env| !env.is_seen()))
            .count();
        Some((unseen, total))
    }

    /// Mail counts for the focused mailbox: `(new, unseen, total)`.
    /// `new` counts unseen mail that arrived since the mailbox was
    /// focused, EXCEPT arrivals during the mailbox's *initial* fetch:
    /// while a first-fetch `Parsing` session is active, unseen mail
    /// trickling in belongs to the initial sync, so the floor rises with
    /// it and `new` stays 0. An *incremental* sync (the mailbox was
    /// already populated when the parse session started) instead keeps
    /// the floor, so genuinely new arrivals surface immediately. Once the
    /// mailbox is settled, the floor only follows the user reading mail
    /// down. Returns `None` when no listing has reported a focus yet.
    fn focus_counts(&mut self, context: &Context) -> Option<(usize, usize, usize)> {
        let (acc_hash, mb_hash) = self.focus?;
        let account = &context.accounts[context.accounts.get_index_of(&acc_hash)?];
        let (unseen, total) = Self::collection_unseen_total(&account.collection, mb_hash)?;
        let parsing = account
            .mailbox_entries
            .get(&mb_hash)
            .is_some_and(|entry| entry.status.is_parsing());
        if parsing {
            let settled_total = self.settled_total;
            let first_fetch = *self
                .parse_first_fetch
                .get_or_insert_with(|| settled_total.unwrap_or(0) == 0);
            if first_fetch {
                // Initial fetch: absorb the arrivals.
                self.unseen_floor = self.unseen_floor.max(unseen);
            } else if unseen < self.unseen_floor {
                // Incremental sync: new arrivals surface, while the user
                // reading mail down still lowers the floor.
                self.unseen_floor = unseen;
            }
        } else {
            self.parse_first_fetch = None;
            self.settled_total = Some(total);
            if unseen < self.unseen_floor {
                self.unseen_floor = unseen;
            }
        }
        let new = unseen.saturating_sub(self.unseen_floor);
        Some((new, unseen, total))
    }

    /// Decide whether the centre `LineGauge` segment should be drawn and
    /// how wide it should be. Returns `(label, width)` where an empty
    /// label means "no gauge, width 0". The label carries the literal
    /// `Fetch {done}/{total}` text that the gauge renders to its left;
    /// `render_line_gauge` decides the bar fill from `MailboxStatus`.
    fn gauge_metrics(&self, context: &Context) -> (String, usize) {
        let Some((acc_hash, mb_hash)) = self.focus else {
            return (String::new(), 0);
        };
        let Some(account_index) = context.accounts.get_index_of(&acc_hash) else {
            return (String::new(), 0);
        };
        let account = &context.accounts[account_index];
        let entry = match account.mailbox_entries.get(&mb_hash) {
            Some(e) => e,
            None => return (String::new(), 0),
        };
        let (done, total) = match entry.status {
            MailboxStatus::Parsing(done, total) if total > 0 => (done, total),
            _ => return (String::new(), 0),
        };
        let label = format!("Fetch {done}/{total}");
        // Total width: 1-cell gutter + label + 1-cell spacer + at least
        // one bar cell + 1-cell gutter. Reserve `min(label+8, 30)` so
        // short bars don't waste a wide centre strip on tiny terminals.
        let width = label.grapheme_width() + 8;
        (label, width.min(30))
    }

    /// Render a `ratatui::widgets::LineGauge` for the focused mailbox's
    /// `Parsing(done, total)` ratio. The gauge runs in a temporary
    /// `RatatuiBuffer` and is then blitted back over `gauge_area` in the
    /// status bar grid. This mirrors the `draw_rounded_frame` helper
    /// pattern so the gauge is rendered through ratatui without leaking
    /// ratatui types into the rest of `StatusBar`.
    fn render_line_gauge(
        &self,
        grid: &mut CellBuffer,
        gauge_area: Area,
        context: &Context,
        label: &str,
    ) {
        if gauge_area.is_empty() {
            return;
        }
        let Some((acc_hash, mb_hash)) = self.focus else {
            return;
        };
        let Some(account_index) = context.accounts.get_index_of(&acc_hash) else {
            return;
        };
        let account = &context.accounts[account_index];
        let entry = match account.mailbox_entries.get(&mb_hash) {
            Some(e) => e,
            None => return,
        };
        let (done, total) = match entry.status {
            MailboxStatus::Parsing(done, total) if total > 0 => (done, total),
            _ => return,
        };
        let ratio = (done.min(total)) as f64 / total as f64;
        // Theme inversion for the filled portion: status.bar bg becomes
        // the gauge fg and vice versa, the same palette fork the Insert
        // mode indicator and the focus chip use.
        let mut base = crate::conf::value(context, "status.bar");
        if !context.settings.terminal.use_color() {
            base.attrs |= Attr::REVERSE;
        }
        let filled_symbol = if context.settings.terminal.ascii_drawing {
            "#"
        } else {
            "▰"
        };
        let unfilled_symbol = if context.settings.terminal.ascii_drawing {
            "."
        } else {
            "▱"
        };
        let buf_width = gauge_area.width() as u16;
        let buf_height = gauge_area.height() as u16;
        let mut buf =
            ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, buf_width, buf_height));
        let mut gauge = ratatui::widgets::LineGauge::default()
            .ratio(ratio)
            .label(ratatui::text::Line::from(label))
            .filled_symbol(filled_symbol)
            .unfilled_symbol(unfilled_symbol);
        gauge = gauge.filled_style(
            ratatui::style::Style::default()
                .fg(base.bg.into())
                .bg(base.fg.into())
                .add_modifier(ratatui::style::Modifier::BOLD),
        );
        ratatui::widgets::Widget::render(gauge, buf.area, &mut buf);
        crate::terminal::ratatui_bridge::blit_buffer_to_cellbuffer_at(&buf, grid, gauge_area);
    }

    /// Compute the right-edge hints text from `self.container.shortcuts()`.
    /// Priority list (in display order):
    ///
    /// 1. `general.toggle_help`         — label `Help`
    /// 2. `general.enter_command_mode`  — label `Command`
    /// 3. `scroll_up`                   — label `Scroll Up`
    /// 4. `scroll_down`                 — label `Scroll Down`
    /// 5. `focus_left`                  — label `Switch Left View`
    /// 6. `focus_right`                 — label `Switch Right View`
    /// 7. `close`                       — label `Close View` (only on
    ///    sub-views that expose a close binding, e.g. composing)
    /// 8. `general.quit`                — label `Quit`
    ///
    /// Bindings missing from the active view are skipped silently. Every
    /// key glyph comes from the *configured* binding (remapping a
    /// shortcut changes its hint); with the defaults the format is
    /// `⌨️ (?:Help)(:/<M-x>:Command)(Up:Scroll Up)(Down:Scroll
    /// Down)...(<Esc>/q:Quit)` — every hint is rendered as `(key:label)`
    /// with no separator between them, and the key glyph of each hint
    /// renders in the theme's highlight-selected color (see
    /// [`HintSpan`]) so the actionable binding stands out from the
    /// descriptive label. Truncates with `…` (or `...` in ASCII
    /// terminals) when the joined text would overflow the configured
    /// max width.
    #[allow(clippy::type_complexity)]
    fn hints_metrics(&self, context: &Context) -> (Vec<HintSpan>, usize) {
        let maps = self.container.shortcuts(context);
        let general = maps.get(crate::conf::Shortcuts::GENERAL);
        // Walk every non-general section first, then fall back to
        // general. This mirrors how `process_event` resolves bindings
        // across the codebase: each view checks its own section
        // (`listing`, `pager`, `composing`, ...) before reaching for
        // the catch-all fields defined in `GeneralShortcuts`. Picking
        // the deepest focused sub-view's binding first is what the
        // user sees in practice when they rebind e.g.
        // `shortcuts.listing.scroll_up`; the general `scroll_up`
        // catch-all only fills in when the focused view doesn't
        // expose the field at all.
        let pick_key = |name: &str| -> Option<&crate::terminal::ShortcutKeys> {
            for (section, map) in maps.iter() {
                if *section == crate::conf::Shortcuts::GENERAL {
                    continue;
                }
                if let Some(k) = map.get(name) {
                    return Some(k);
                }
            }
            if let Some(map) = general {
                if let Some(k) = map.get(name) {
                    return Some(k);
                }
            }
            None
        };
        // Eight entries in fixed display order: help first (so it
        // survives narrow ellipsis), then scroll, then focus switches,
        // then view close, then exit pinned last so it is the final
        // actionable hint.
        let pickers: [(
            &'static str,
            Option<&crate::terminal::ShortcutKeys>,
            &'static str,
        ); 8] = [
            ("help", general.and_then(|m| m.get("toggle_help")), "Help"),
            (
                "enter_command_mode",
                general.and_then(|m| m.get("enter_command_mode")),
                "Command",
            ),
            ("scroll_up", pick_key("scroll_up"), "Scroll Up"),
            ("scroll_down", pick_key("scroll_down"), "Scroll Down"),
            ("focus_left", pick_key("focus_left"), "Switch Left View"),
            ("focus_right", pick_key("focus_right"), "Switch Right View"),
            ("close", pick_key("close"), "Close View"),
            ("quit", general.and_then(|m| m.get("quit")), "Quit"),
        ];
        let entries: Vec<(&crate::terminal::ShortcutKeys, &'static str)> = pickers
            .iter()
            .filter_map(|(_, key, label)| key.as_ref().copied().map(|k| (k, *label)))
            .collect();
        if entries.is_empty() {
            return (Vec::new(), 0);
        }
        let ellipsis = if context.settings.terminal.ascii_drawing {
            "..."
        } else {
            "…"
        };
        // Build one colored run per hint: `(` plain, key glyph in the
        // theme's highlight-selected color (+bold), `:label)` plain. The hint form renders placeholder
        // keys in angle brackets (`<Up>/k`, `<Esc>/q`) so they read as
        // key descriptions rather than literal text (see
        // [`crate::terminal::ShortcutKeys::hint_display`]).
        let mut spans: Vec<HintSpan> = Vec::with_capacity(entries.len() * 3);
        for (key, label) in &entries {
            spans.push(HintSpan::plain("("));
            spans.push(HintSpan::key(key.hint_display()));
            spans.push(HintSpan::plain(format!(":{label})")));
        }
        // Use a generous maximum: the status bar clips the segment at
        // the row's remaining width anyway; this only governs the
        // ellipsis cutoff. Eight labelled hints run ~100–110 cells, so
        // the previous 80-column budget truncated too aggressively.
        let mut spans = truncate_spans_with_ellipsis(spans, 200, ellipsis);
        if spans.is_empty() {
            return (Vec::new(), 0);
        }
        // Segment head: keyboard glyph with breathing room after the `|`
        // separator; the emoji-presentation selector is carried to the
        // terminal by the `FORCE_EMOJI` cell attribute (see cells.rs).
        // On ASCII terminals only the space remains.
        let icon = if context.settings.terminal.emoji_capable() {
            // FORCE_EMOJI carries the FE0F selector to the terminal.
            " ⌨️ "
        } else if context.settings.terminal.ascii_drawing {
            // Plain-ASCII mode: leading space only.
            " "
        } else {
            // Unicode-but-not-emoji analogue. Literal word so it cannot
            // be mistaken for a Ctrl-binding.
            " Shortcut "
        };
        spans.insert(0, HintSpan::plain(icon));
        let width = spans.iter().map(|span| span.text.grapheme_width()).sum();
        (spans, width)
    }

    fn draw_command_bar(&self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        grid.clear_area(area, crate::conf::value(context, "theme_default"));
        let command_bar = crate::conf::value(context, "status.command_bar");
        let (_, y) = grid.write_string(
            self.ex_buffer.as_str(),
            command_bar.fg,
            command_bar.bg,
            command_bar.attrs,
            area,
            None,
            None,
        );
        grid.change_theme(area, command_bar);
        if let Some(c) = grid
            .row_iter(area, self.ex_buffer.cursor()..area.width(), y)
            .next()
        {
            grid[c].set_attrs(command_bar.attrs | Attr::UNDERLINE);
        }
        context.dirty_areas.push_back(area);
    }

    /// Per-mode attributes for the status bar's leading [`UIMode`]
    /// indicator, derived strictly from the existing theme vocabulary (no
    /// new keys): "status.bar" is the base; Insert inverts it (the classic
    /// editing emphasis), Command adopts the amber `status.command_bar`
    /// block shared with the command line above, and Embedded uses the
    /// bolded "status.notification" surface. Fork stays on the base.
    fn mode_indicator_attrs(mode: UIMode, context: &Context) -> ThemeAttribute {
        let mut base = crate::conf::value(context, "status.bar");
        if !context.settings.terminal.use_color() {
            base.attrs |= Attr::REVERSE;
        }
        match mode {
            UIMode::Normal | UIMode::Fork => base,
            UIMode::Insert => ThemeAttribute {
                fg: base.bg,
                bg: base.fg,
                attrs: base.attrs | Attr::BOLD,
            },
            UIMode::Command => {
                let mut attrs = crate::conf::value(context, "status.command_bar");
                if !context.settings.terminal.use_color() {
                    attrs.attrs |= Attr::REVERSE;
                }
                attrs
            }
            UIMode::Embedded => {
                let mut attrs = crate::conf::value(context, "status.notification");
                attrs.attrs |= Attr::BOLD;
                if !context.settings.terminal.use_color() {
                    attrs.attrs |= Attr::REVERSE;
                }
                attrs
            }
        }
    }
}

impl Component for StatusBar {
    fn draw(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        #[cfg(debug_assertions)]
        let __draw_span = crate::state::DrawSpan::enter("StatusBar");
        // The bottom strip is wrapped in a rounded frame (one ring of
        // border cells around the strip), so it occupies `self.height`
        // content rows plus one border row above and below — mirroring the
        // pane rings the container views draw (`draw_rounded_frame`).
        const FRAME_ROWS: usize = 2;
        let total_rows = area.height();
        if total_rows <= self.height + FRAME_ROWS {
            return;
        }

        /* Top-level vertical split via ratatui Layout: the container takes
         * every row but the framed bottom strip, which the status bar and
         * (in Command mode, where the strip is two rows) the command line
         * share. */
        let [body, bar] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length((self.height + FRAME_ROWS) as u16),
        ])
        .areas(crate::terminal::ratatui_bridge::area_to_rect(area));
        let bar_area = crate::terminal::ratatui_bridge::rect_to_area(bar, area);
        let bar_inner = crate::terminal::ratatui_bridge::draw_rounded_frame(grid, bar_area, {
            let mut attr = crate::conf::value(context, "status.bar");
            if !context.settings.terminal.use_color() {
                attr.attrs |= Attr::REVERSE;
            }
            attr
        });
        // The frame ring writes cells directly (blit), so push its strips
        // for the incremental flush — without this the ring only appears
        // on full repaints (mirrors Tabbed's own frame push).
        for frame_area in crate::terminal::ratatui_bridge::frame_flush_areas(grid, bar_area) {
            context.dirty_areas.push_back(frame_area);
        }
        let [command_line, status_row] = Layout::vertical([
            Constraint::Length(self.height.saturating_sub(1) as u16),
            Constraint::Length(1),
        ])
        .areas(crate::terminal::ratatui_bridge::area_to_rect(bar_inner));

        self.container.draw(
            grid,
            crate::terminal::ratatui_bridge::rect_to_area(body, area),
            context,
        );

        self.dirty = false;
        self.draw_status_bar(
            grid,
            crate::terminal::ratatui_bridge::rect_to_area(status_row, area),
            context,
        );

        if self.mode != UIMode::Command && !self.is_dirty() {
            return;
        }
        match self.mode {
            UIMode::Normal => {}
            UIMode::Command => {
                let command_line_area =
                    crate::terminal::ratatui_bridge::rect_to_area(command_line, area);
                self.draw_command_bar(grid, command_line_area, context);
                /* don't autocomplete for less than 3 characters */
                if self.ex_buffer.as_str().split_graphemes().len() <= 2 {
                    return;
                }

                let mut unique_suggestions: HashSet<&str> = HashSet::default();
                let mut suggestions: Vec<AutoCompleteEntry> = self
                    .cmd_history
                    .iter()
                    .rev()
                    .filter_map(|h| {
                        let sug = self.ex_buffer.as_str();
                        if h.starts_with(sug) && !unique_suggestions.contains(sug) {
                            unique_suggestions.insert(sug);
                            Some(h.clone().into())
                        } else {
                            None
                        }
                    })
                    .collect();
                let command_completion_suggestions =
                    crate::command::command_completion_suggestions(self.ex_buffer.as_str());

                suggestions.extend(command_completion_suggestions.iter().filter_map(|e| {
                    if unique_suggestions.insert(e.as_str()) {
                        Some(e.clone().into())
                    } else {
                        None
                    }
                }));
                /*
                suggestions.extend(crate::command::COMMAND_COMPLETION.iter().filter_map(|e| {
                    if e.0.starts_with(self.ex_buffer.as_str()) {
                        Some(e.into())
                    } else {
                        None
                    }
                }));
                */
                if let Some(p) = self.ex_buffer.as_str().split_whitespace().last() {
                    let path = std::path::Path::new(p);
                    suggestions.extend(
                        path.complete(true, p.ends_with('/'))
                            .into_iter()
                            .map(|m| format!("{}{}", self.ex_buffer.as_str(), m).into()),
                    );
                }
                if suggestions.is_empty() && !self.auto_complete.suggestions().is_empty() {
                    self.auto_complete.set_suggestions(suggestions);
                    /* redraw self.container because we have got ridden of an autocomplete
                     * box, and it must be drawn over */
                    self.container.set_dirty(true);
                    return;
                }
                /* redraw self.container because we have less suggestions than before */
                if suggestions.len() < self.auto_complete.suggestions().len() {
                    self.container.set_dirty(true);
                }

                suggestions.sort_by(|a, b| a.entry.cmp(&b.entry));
                suggestions.dedup_by(|a, b| a.entry == b.entry);
                if self.auto_complete.set_suggestions(suggestions) {
                    let len = self.auto_complete.suggestions().len() - 1;
                    self.auto_complete.set_cursor(len);

                    self.container.set_dirty(true);
                }
                /* Completion popup: a rounded floating panel anchored right
                 * above the status strip (the widget styles itself with the
                 * dialog vocabulary in `AutoComplete::draw`). */
                if !self.auto_complete.suggestions().is_empty() {
                    self.auto_complete.draw(
                        grid,
                        area.skip_rows_from_end(self.height + 2),
                        context,
                    );
                }
                /*
                let hist_height = std::cmp::min(15, self.auto_complete.suggestions().len());
                    let hist_area = if status_bar_height < self.auto_complete.suggestions().len() {
                        let hist_area = (
                            (
                                get_x(upper_left),
                                std::cmp::min(
                                    get_y(bottom_right) - status_bar_height - hist_height + 1,
                                    get_y(pos_dec(bottom_right, (0, status_bar_height))),
                                ),
                            ),
                            pos_dec(bottom_right, (0, status_bar_height)),
                        );
                        ScrollBar::default().set_show_arrows(false).draw(
                            grid,
                            hist_area,
                            context,
                            self.auto_complete.cursor(),
                            hist_height,
                            self.auto_complete.suggestions().len(),
                        );
                        grid.change_theme(hist_area, crate::conf::value(context, "status.history"));
                        context.dirty_areas.push_back(hist_area);
                        hist_area
                    } else {
                        (
                            get_x(upper_left),
                            std::cmp::min(
                                get_y(bottom_right) - status_bar_height - hist_height + 1,
                                get_y(pos_dec(bottom_right, (0, status_bar_height))),
                            ),
                        ),
                        pos_dec(bottom_right, (0, status_bar_height)),
                    )
                };
                let offset = if hist_height
                    > (self.auto_complete.suggestions().len() - self.auto_complete.cursor())
                {
                    self.auto_complete.suggestions().len() - hist_height
                } else {
                    self.auto_complete.cursor()
                };

                grid.clear_area(hist_area, crate::conf::value(context, "theme_default"));
                let history_hints = crate::conf::value(context, "status.history.hints");
                if hist_height > 0 {
                    grid.change_theme(hist_area, history_hints);
                }
                for (y_offset, s) in self
                    .auto_complete
                    .suggestions()
                    .iter()
                    .skip(offset)
                    .take(hist_height)
                    .enumerate()
                {
                    let (x, y) = grid.write_string(
                        s.as_str(),
                        history_hints.fg,
                        history_hints.bg,
                        history_hints.attrs,
                        (
                            set_y(
                                upper_left!(hist_area),
                                get_y(bottom_right!(hist_area)) - hist_height + y_offset + 1,
                            ),
                            bottom_right!(hist_area),
                        ),
                        Some(get_x(upper_left!(hist_area))),
                    );
                    grid.write_string(
                        &s.description,
                        history_hints.fg,
                        history_hints.bg,
                        history_hints.attrs,
                        ((x + 2, y), bottom_right!(hist_area)),
                        None,
                    );
                    if y_offset + offset == self.auto_complete.cursor() {
                        grid.change_theme(
                            (
                                get_x(upper_left),
                                std::cmp::min(
                                    get_y(bottom_right) - status_bar_height - hist_height + 1,
                                    get_y(pos_dec(bottom_right, (0, status_bar_height))),
                                ),
                            ),
                            pos_dec(bottom_right, (0, status_bar_height)),
                        )
                    };
                    let offset = if hist_height
                        > (self.auto_complete.suggestions().len() - self.auto_complete.cursor())
                    {
                        self.auto_complete.suggestions().len() - hist_height
                    } else {
                        self.auto_complete.cursor()
                    };
                    grid.clear_area(hist_area, crate::conf::value(context, "theme_default"));
                    let history_hints = crate::conf::value(context, "status.history.hints");
                    if hist_height > 0 {
                        grid.change_theme(hist_area, history_hints);
                    }
                    for (y_offset, s) in self
                        .auto_complete
                        .suggestions()
                        .iter()
                        .skip(offset)
                        .take(hist_height)
                        .enumerate()
                    {
                        let (x, y) = grid.write_string(
                            s.as_str(),
                            grid,
                            history_hints.fg,
                            history_hints.bg,
                            history_hints.attrs,
                            (
                                set_y(
                                    hist_area.upper_left(),
                                    get_y(hist_area.bottom_right()) - hist_height + y_offset + 1,
                                ),
                                hist_area.bottom_right(),
                            ),
                            Some(get_x(hist_area.upper_left())),
                        );
                        grid.write_string(
                            &s.description,
                            grid,
                            history_hints.fg,
                            history_hints.bg,
                            history_hints.attrs,
                            ((x + 2, y), hist_area.bottom_right()),
                            None,
                        );
                        if y_offset + offset == self.auto_complete.cursor() {
                            grid.change_theme(
                                (
                                    set_y(
                                        hist_area.upper_left(),
                                        get_y(hist_area.bottom_right()) - hist_height
                                            + y_offset
                                            + 1,
                                    ),
                                    set_y(
                                        hist_area.bottom_right(),
                                        get_y(hist_area.bottom_right()) - hist_height
                                            + y_offset
                                            + 1,
                                    ),
                                ),
                                history_hints,
                            );
                            grid.write_string(
                                &s.as_str()[self.ex_buffer.as_str().len()..],
                                history_hints.fg,
                                history_hints.bg,
                                history_hints.attrs,
                                (
                                    (
                                        get_x(upper_left)
                                            + self.ex_buffer.as_str().split_graphemes().len(),
                                        get_y(bottom_right) - status_bar_height + 1,
                                    ),
                                    set_y(bottom_right, get_y(bottom_right) - status_bar_height + 1),
                                ),
                                None,
                            );
                        }
                    }
                    context.dirty_areas.push_back(hist_area);
                    */
            }
            _ => {}
        }
    }

    fn process_event(&mut self, event: &mut UIEvent, context: &mut Context) -> bool {
        // In Normal mode the quit binding doubles as the top-level
        // "exit application" request, but only when no focused
        // component consumed it (layered quit: sub-views close first).
        // Snapshot the check before forwarding: the container may
        // mutate the event while processing it.
        let is_quit_request = self.mode == UIMode::Normal
            && matches!(event, UIEvent::Input(ref k) if context
                .settings
                .shortcuts
                .general
                .quit
                .contains(k));
        if self.container.process_event(event, context) {
            return true;
        }
        if is_quit_request {
            context.replies.push_back(UIEvent::Exit);
            return true;
        }

        match event {
            UIEvent::ConfigReload { old_settings: _ } => {
                let mut progress_spinner = Self::make_progress_spinner(context);
                if self.progress_spinner.is_active() {
                    progress_spinner.start();
                }
                self.progress_spinner = progress_spinner;
                self.mouse = context.settings.terminal.use_mouse.is_true();
                self.set_dirty(true);
                self.container.set_dirty(true);
            }
            UIEvent::ChangeMode(m) => {
                self.set_dirty(true);
                self.container.set_dirty(true);
                self.mode = *m;
                match m {
                    UIMode::Normal => {
                        self.height = 1;
                        if !self.ex_buffer.is_empty() {
                            context
                                .replies
                                .push_back(UIEvent::Command(self.ex_buffer.as_str().to_string()));
                        }
                        if parse_command(self.ex_buffer.as_str().as_bytes()).is_ok()
                            && self.cmd_history.last().map(String::as_str)
                                != Some(self.ex_buffer.as_str())
                        {
                            crate::command::history::log_cmd(self.ex_buffer.as_str().to_string());
                            self.cmd_history.push(self.ex_buffer.as_str().to_string());
                        }
                        self.ex_buffer.clear();
                        self.ex_buffer_cmd_history_pos.take();
                    }
                    UIMode::Command => {
                        self.height = 2;
                    }
                    _ => {
                        self.height = 1;
                    }
                };
            }
            UIEvent::CmdInput(Key::Char('\n')) => {
                if let Some(suggestion) = self.auto_complete.get_suggestion() {
                    self.ex_buffer.set_text(suggestion);
                }
                context
                    .replies
                    .push_back(UIEvent::ChangeMode(UIMode::Normal));
                self.dirty = true;
                return true;
            }
            UIEvent::CmdInput(Key::Char('\t')) => {
                if let Some(suggestion) = self.auto_complete.get_suggestion().or_else(|| {
                    if self.auto_complete.cursor() == 0 {
                        self.auto_complete
                            .suggestions()
                            .last()
                            .map(|e| e.entry.clone())
                    } else {
                        None
                    }
                }) {
                    self.container.set_dirty(true);
                    self.set_dirty(true);
                    self.ex_buffer.set_text(suggestion);
                }
            }
            UIEvent::CmdInput(Key::Char(c)) => {
                self.dirty = true;
                self.ex_buffer
                    .process_event(&mut UIEvent::InsertInput(Key::Char(*c)), context);
                return true;
            }
            UIEvent::CmdInput(Key::Paste(s)) => {
                self.dirty = true;
                self.ex_buffer
                    .process_event(&mut UIEvent::InsertInput(Key::Paste(s.clone())), context);
                return true;
            }
            UIEvent::CmdInput(Key::Ctrl('u')) => {
                self.dirty = true;
                self.ex_buffer.clear();
                self.ex_buffer_cmd_history_pos.take();
                return true;
            }
            UIEvent::CmdInput(Key::Up) => {
                self.auto_complete.dec_cursor();
                self.dirty = true;
            }
            UIEvent::CmdInput(Key::Down) => {
                self.auto_complete.inc_cursor();
                self.set_dirty(true);
            }
            UIEvent::CmdInput(Key::Left) => {
                self.ex_buffer.cursor_dec();
                self.set_dirty(true);
            }
            UIEvent::CmdInput(Key::Right) => {
                self.ex_buffer.cursor_inc();
                self.set_dirty(true);
            }
            UIEvent::CmdInput(Key::Ctrl('p')) => {
                if self.cmd_history.is_empty() {
                    return true;
                }
                let pos = self.ex_buffer_cmd_history_pos.map(|p| p + 1).unwrap_or(0);
                let pos = std::cmp::min(pos, self.cmd_history.len().saturating_sub(1));
                if Some(pos) != self.ex_buffer_cmd_history_pos {
                    let history_entry =
                        self.cmd_history[self.cmd_history.len().saturating_sub(1) - pos].clone();
                    self.container.set_dirty(true);
                    self.set_dirty(true);
                    self.ex_buffer.set_text(history_entry);
                    self.ex_buffer_cmd_history_pos = Some(pos);
                    self.dirty = true;
                }

                return true;
            }
            UIEvent::CmdInput(Key::Ctrl('n')) => {
                if self.cmd_history.is_empty() {
                    return true;
                }
                if Some(0) == self.ex_buffer_cmd_history_pos {
                    self.ex_buffer_cmd_history_pos = None;
                    self.ex_buffer.clear();
                    self.dirty = true;
                } else if let Some(pos) = self.ex_buffer_cmd_history_pos.map(|p| p - 1) {
                    let history_entry =
                        self.cmd_history[self.cmd_history.len().saturating_sub(1) - pos].clone();
                    self.container.set_dirty(true);
                    self.set_dirty(true);
                    self.ex_buffer.set_text(history_entry);
                    self.ex_buffer_cmd_history_pos = Some(pos);
                    self.dirty = true;
                }

                return true;
            }
            UIEvent::CmdInput(k @ Key::Backspace) | UIEvent::CmdInput(k @ Key::Ctrl(_)) => {
                self.dirty = true;
                self.ex_buffer
                    .process_event(&mut UIEvent::InsertInput(k.clone()), context);
                return true;
            }
            UIEvent::CmdInput(Key::Esc) => {
                self.ex_buffer.clear();
                context
                    .replies
                    .push_back(UIEvent::ChangeMode(UIMode::Normal));
                self.dirty = true;
                return true;
            }
            UIEvent::Resize => {
                self.dirty = true;
            }
            UIEvent::StatusEvent(StatusEvent::BufClear) => {
                self.display_buffer.clear();
                self.dirty = true;
            }
            UIEvent::StatusEvent(StatusEvent::BufSet(s)) => {
                self.display_buffer.clone_from(s);
                self.dirty = true;
            }
            UIEvent::StatusEvent(StatusEvent::UpdateStatus(ref mut s)) => {
                self.status_message.clear();
                self.status_message.push_str(s.as_str());
                self.substatus_message.clear();
                self.dirty = true;
            }
            UIEvent::StatusEvent(StatusEvent::UpdateSubStatus(ref mut s)) => {
                self.substatus_message.clear();
                self.substatus_message.push_str(s.as_str());
                self.dirty = true;
            }
            UIEvent::StatusEvent(StatusEvent::SetMouse(val)) => {
                self.mouse = *val;
                self.dirty = true;
            }
            UIEvent::StatusEvent(StatusEvent::JobCanceled(ref job_id))
            | UIEvent::StatusEvent(StatusEvent::JobFinished(ref job_id)) => {
                self.done_jobs.insert(*job_id);
                self.in_progress_jobs.remove(job_id);
                if self.in_progress_jobs.is_empty() {
                    self.progress_spinner.stop();
                }
                self.progress_spinner.set_dirty(true);
            }
            UIEvent::StatusEvent(StatusEvent::NewJob(ref job_id))
                if !self.done_jobs.contains(job_id) =>
            {
                if self.in_progress_jobs.is_empty() {
                    self.progress_spinner.start();
                }
                self.progress_spinner.set_dirty(true);
                self.in_progress_jobs.insert(*job_id);
            }
            UIEvent::Timer(_) => {
                if self.progress_spinner.process_event(event, context) {
                    return true;
                }
            }
            UIEvent::StatusEvent(StatusEvent::FocusMailbox(acc_hash, mb_hash)) => {
                // Idempotent: the listing co-emits this event on every
                // status refresh (not only cursor moves), so the floor
                // re-baselines only when the focus actually changes —
                // otherwise every mail arrival would reset the `📨 new`
                // counter to zero.
                if self.focus != Some((*acc_hash, *mb_hash)) {
                    self.focus = Some((*acc_hash, *mb_hash));
                    self.settled_total = None;
                    self.parse_first_fetch = None;
                    self.unseen_floor = context
                        .accounts
                        .get_index_of(acc_hash)
                        .and_then(|i| {
                            Self::collection_unseen_total(&context.accounts[i].collection, *mb_hash)
                        })
                        .map_or(0, |(unseen, _)| unseen);
                    self.dirty = true;
                }
            }
            UIEvent::MailboxUpdate((acc_hash, mb_hash)) => {
                if self.focus == Some((*acc_hash, *mb_hash)) {
                    self.set_dirty(true);
                }
            }
            UIEvent::AccountStatusChange(acc_hash, _msg) => {
                // `AccountStatusChange` payloads are an optional descriptive
                // string, not a `MailboxHash`: only the first element
                // `AccountHash` is matched. The whole status bar repaints
                // when the focused account's connection state flips.
                if self.focus.is_some_and(|(acc, _)| acc == *acc_hash) {
                    self.set_dirty(true);
                }
            }
            _ => {}
        }
        false
    }

    fn is_dirty(&self) -> bool {
        self.dirty
            || self.container.is_dirty()
            || self.ex_buffer.is_dirty()
            || self.progress_spinner.is_dirty()
    }

    fn set_dirty(&mut self, value: bool) {
        self.dirty = value;
        self.ex_buffer.set_dirty(value);
        self.progress_spinner.set_dirty(value);
    }

    fn shortcuts(&self, context: &Context) -> ShortcutMaps {
        self.container.shortcuts(context)
    }

    fn id(&self) -> ComponentId {
        self.id
    }

    fn can_quit_cleanly(&mut self, context: &Context) -> bool {
        self.container.can_quit_cleanly(context)
    }

    fn attributes(&self) -> &'static ComponentAttr {
        &ComponentAttr::CONTAINER
    }

    fn children(&self) -> IndexMap<ComponentId, &dyn Component> {
        let mut ret = IndexMap::default();
        ret.insert(self.container.id(), &self.container as &dyn Component);
        ret.insert(self.ex_buffer.id(), &self.ex_buffer as &dyn Component);
        ret.insert(
            self.progress_spinner.id(),
            &self.progress_spinner as &dyn Component,
        );
        ret
    }

    fn children_mut(&mut self) -> IndexMap<ComponentId, &mut dyn Component> {
        IndexMap::default()
    }

    fn realize(&self, parent: Option<ComponentId>, context: &mut Context) {
        context.realized.insert(self.id(), parent);
        self.container.realize(self.id().into(), context);
        self.progress_spinner.realize(self.id().into(), context);
        self.ex_buffer.realize(self.id().into(), context);
    }

    fn unrealize(&self, context: &mut Context) {
        context.unrealized.insert(self.id());
        self.container.unrealize(context);
        self.progress_spinner.unrealize(context);
        self.ex_buffer.unrealize(context);
    }
}

#[derive(Debug)]
struct HelpView {
    content: Screen<Virtual>,
    cursor: (usize, usize),
    curr_views: ShortcutMaps,
    search: Option<SearchPattern>,
}

#[derive(Debug)]
pub struct Tabbed {
    pinned: usize,
    children: Vec<Box<dyn Component>>,
    /// Per-child flag: dynamically added tab children (via `Tab(New)`) draw
    /// inside an area inset by one cell so their content stays clear of the
    /// rounded frame ring painted over the tab body's outermost cells. The
    /// pinned children (the mail listing and the contact list) manage the
    /// ring themselves.
    inset_children: Vec<bool>,
    cursor_pos: usize,

    show_shortcuts: bool,
    help_view: HelpView,
    theme_default: ThemeAttribute,

    dirty: bool,
    id: ComponentId,
}

impl Tabbed {
    pub fn new(children: Vec<Box<dyn Component>>, context: &Context) -> Self {
        let pinned = children.len();
        let theme_default = crate::conf::value(context, "theme_default");
        let mut ret = Self {
            help_view: HelpView {
                content: Screen::<Virtual>::new(theme_default),
                curr_views: children
                    .first()
                    .map(|c| c.shortcuts(context))
                    .unwrap_or_default(),
                cursor: (0, 0),
                search: None,
            },
            theme_default,
            pinned,
            inset_children: vec![false; children.len()],
            children,
            cursor_pos: 0,
            show_shortcuts: false,
            dirty: true,
            id: ComponentId::default(),
        };
        ret.help_view
            .curr_views
            .extend_shortcuts(ret.shortcuts(context));
        ret
    }

    fn draw_tabs(&self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        let tab_bar_attribute = crate::conf::value(context, "tab.bar");
        grid.clear_area(area, tab_bar_attribute);
        if self.children.is_empty() {
            return;
        }
        let tab_unfocused_attribute = crate::conf::value(context, "tab.unfocused");
        let mut tab_focused_attribute = crate::conf::value(context, "tab.focused");
        if !context.settings.terminal.use_color() {
            tab_focused_attribute.attrs |= Attr::REVERSE;
        }

        /* Modern tab spacing: a one-column gutter aligns labels with the
         * inner content of the rounded body frame drawn right below this
         * row; labels keep one blank padding column on each side and tabs
         * are separated by a two-column gap. The focused tab gains an
         * underline accent on top of the "tab.focused" vocabulary so the
         * active tab also reads on grayscale terminals. */
        let mut x = 1;
        for (idx, c) in self.children.iter().enumerate() {
            let focused = idx == self.cursor_pos;
            let ThemeAttribute { fg, bg, mut attrs } = if focused {
                tab_focused_attribute
            } else {
                tab_unfocused_attribute
            };
            if focused {
                attrs |= Attr::UNDERLINE;
            }
            let name = format!(" {c} ");
            grid.write_string(&name, fg, bg, attrs, area.skip_cols(x), None, None);
            x += name.len() + 2;
            if idx == self.pinned.saturating_sub(1) {
                x += 2;
            }
            if x > area.width() {
                break;
            }
        }
        context.dirty_areas.push_back(area);
    }

    pub fn add_component(&mut self, new: Box<dyn Component>, context: &mut Context) {
        new.realize(self.id().into(), context);
        self.inset_children.push(true);
        self.children.push(new);
    }

    fn update_help_curr_views(&mut self, context: &Context) {
        let mut children_maps = self.children[self.cursor_pos].shortcuts(context);
        children_maps.extend_shortcuts(self.shortcuts(context));
        if let Some(i) = children_maps
            .get_index_of(Shortcuts::GENERAL)
            .filter(|i| i + 1 != children_maps.len())
        {
            children_maps.move_index(i, children_maps.len().saturating_sub(1));
        }
        self.help_view.curr_views = children_maps;
    }

    /// Emit both the legacy `UpdateStatus` string and the structured
    /// `FocusMailbox` event for the active child, so the status bar stays
    /// in sync with the focused mailbox as the cursor moves between tabs.
    fn push_focus_updates(
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
}

impl std::fmt::Display for Tabbed {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "tabs")
    }
}

impl Component for Tabbed {
    fn draw(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        #[cfg(debug_assertions)]
        let __draw_span = crate::state::DrawSpan::enter("Tabbed");
        /* Top-level vertical split via ratatui Layout: one tab row, the rest
         * is the tab body. Identical to the previous nth_row/skip_rows math
         * for every size (including empty areas). */
        let [tab_row, below_tab_row] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(0)])
                .areas(crate::terminal::ratatui_bridge::area_to_rect(area));
        let below_tab_row = crate::terminal::ratatui_bridge::rect_to_area(below_tab_row, area);
        if self.dirty {
            grid.clear_area(
                crate::terminal::ratatui_bridge::rect_to_area(tab_row, area),
                crate::conf::value(context, "tab.bar"),
            );
            context
                .dirty_areas
                .push_back(crate::terminal::ratatui_bridge::rect_to_area(tab_row, area));
        }

        /* If children are dirty but self isn't and the shortcuts panel is visible,
         * it will get overwritten. */
        let must_redraw_shortcuts: bool = self.show_shortcuts && !self.dirty && self.is_dirty();

        /* children should be drawn after the shortcuts/help panel lest they
         * overwrite the panel on the grid. the drawing order is determined
         * by the dirty_areas queue which is LIFO */
        /* Dynamically added tab children draw inside an area inset by one
         * cell: the rounded frame below is painted over the tab body's
         * outermost cells, so a child drawing at the area's edges would get
         * its first column and header row overwritten. Pinned children (the
         * mail listing, the contact list) draw pane frames flush to the body
         * edges themselves and get the full area. */
        let inset_child = *self.inset_children.get(self.cursor_pos).unwrap_or(&false);
        let inset = |a: Area| a.skip(1, 1).skip_cols_from_end(1).skip_rows_from_end(1);
        if self.children.len() > 1 {
            self.draw_tabs(
                grid,
                crate::terminal::ratatui_bridge::rect_to_area(tab_row, area),
                context,
            );
            let child_area = if inset_child {
                inset(below_tab_row)
            } else {
                below_tab_row
            };
            self.children[self.cursor_pos].draw(grid, child_area, context);
        } else {
            let child_area = if inset_child { inset(area) } else { area };
            self.children[self.cursor_pos].draw(grid, child_area, context);
        }

        /* Rounded outer frame around the tab body (visual chrome only),
         * drawn only for inset children — they are laid out one cell
         * inside the body and rely on this frame for their border.
         * Pinned children (the mail listing, the contact list) draw
         * their own pane rings flush to the body edges; painting a
         * full-body frame over them would merge the pane tops into one
         * border. The visible tab is the focused pane, so its frame
         * uses "tab.focused". Drawn after the children so partial child
         * redraws cannot leave the border ring eaten; the ring cells
         * are pushed for flushing. The shortcuts overlay below draws
         * after the frame, so it layers on top. */
        let body_area = if self.children.len() > 1 {
            below_tab_row
        } else {
            area
        };
        if inset_child && self.is_dirty() && body_area.width() >= 2 && body_area.height() >= 2 {
            draw_rounded_frame(grid, body_area, crate::conf::value(context, "tab.focused"));
            for frame_area in frame_flush_areas(grid, body_area) {
                context.dirty_areas.push_back(frame_area);
            }
        }
        let area = below_tab_row;

        if (self.show_shortcuts && self.dirty) || must_redraw_shortcuts {
            let mut children_maps = self.children[self.cursor_pos].shortcuts(context);
            children_maps.extend_shortcuts(self.shortcuts(context));
            if children_maps.is_empty() {
                return;
            }
            if let Some(i) = children_maps
                .get_index_of(Shortcuts::GENERAL)
                .filter(|i| i + 1 != children_maps.len())
            {
                children_maps.move_index(i, children_maps.len().saturating_sub(1));
            }
            if (children_maps == self.help_view.curr_views) && must_redraw_shortcuts {
                let dialog_area = crate::terminal::ratatui_bridge::center_inside_via_layout(
                    // add box perimeter padding
                    area,
                    {
                        let (w, h) = self.help_view.content.area().size();
                        (w + 1, h + 1)
                    },
                );
                context.dirty_areas.push_back(dialog_area);
                grid.clear_area(dialog_area, self.theme_default);
                let inner_area = draw_rounded_frame(
                    grid,
                    dialog_area,
                    crate::conf::value(context, "tab.focused"),
                );
                for frame_area in frame_flush_areas(grid, dialog_area) {
                    context.dirty_areas.push_back(frame_area);
                }
                let (x, y) = grid.write_string(
                    "shortcuts",
                    crate::conf::value(context, "tab.focused").fg,
                    self.theme_default.bg,
                    self.theme_default.attrs | Attr::BOLD,
                    inner_area.skip_cols(2),
                    None,
                    None,
                );
                grid.write_string(
                    &format!(
                        "Press {} to close",
                        children_maps[Shortcuts::GENERAL]["toggle_help"]
                    ),
                    crate::conf::value(context, "tab.unfocused").fg,
                    self.theme_default.bg,
                    self.theme_default.attrs | Attr::ITALICS,
                    inner_area.skip(4 + x, y),
                    None,
                    None,
                );
                let inner_area = inner_area.skip_rows(y + 1).skip_rows_from_end(1);
                let (width, height) = self.help_view.content.grid().size();
                let (cols, rows) = inner_area.size();

                grid.copy_area(
                    self.help_view.content.grid(),
                    inner_area,
                    self.help_view
                        .content
                        .area()
                        .skip(
                            std::cmp::min(
                                (width - 1).saturating_sub(cols),
                                self.help_view.cursor.0,
                            ),
                            std::cmp::min(
                                (height - 1).saturating_sub(rows),
                                self.help_view.cursor.1,
                            ),
                        )
                        .take_rows(rows),
                );
                if height.wrapping_div(rows + 1) > 0 || width.wrapping_div(cols + 1) > 0 {
                    context
                        .replies
                        .push_back(UIEvent::StatusEvent(StatusEvent::ScrollUpdate(
                            ScrollUpdate::Update {
                                id: self.id,
                                context: ScrollContext {
                                    shown_lines: std::cmp::min(
                                        (height).saturating_sub(rows + 1),
                                        self.help_view.cursor.1,
                                    ) + rows,
                                    total_lines: height,
                                    has_more_lines: false,
                                },
                            },
                        )));
                    ScrollBar::default().set_show_arrows(true).draw(
                        grid,
                        inner_area.nth_col(inner_area.width().saturating_sub(1)),
                        context,
                        /* position */
                        std::cmp::min((height).saturating_sub(rows + 1), self.help_view.cursor.1),
                        /* visible_rows */
                        rows,
                        /* length */
                        height,
                    );
                } else {
                    context
                        .replies
                        .push_back(UIEvent::StatusEvent(StatusEvent::ScrollUpdate(
                            ScrollUpdate::End(self.id),
                        )));
                }
                self.dirty = false;
                return;
            }
            let mut max_length = 6;
            let mut max_width =
                "Press XXXX to close, use COMMAND \"search\" to find shortcuts".len() + 3;

            let mut max_first_column_width = 3;

            for (desc, shortcuts) in children_maps.iter() {
                max_length += shortcuts.len() + 3;
                // `Display for ShortcutKeys` renders the `/`-joined
                // bindings; format each entry once here instead of twice
                // per entry (the two `max` computations used to re-run it).
                let column_width = shortcuts
                    .values()
                    .map(|v| v.to_string().len() + 5)
                    .max()
                    .unwrap_or(0);
                max_width = std::cmp::max(max_width, std::cmp::max(desc.len(), column_width));
                max_first_column_width = std::cmp::max(max_first_column_width, column_width);
            }
            if !self
                .help_view
                .content
                .resize_with_context(max_width, max_length + 2, context)
            {
                self.dirty = false;
                return;
            }
            self.help_view.content.grid_mut().set_growable(true);
            let help_area = self.help_view.content.area();
            self.help_view.content.grid_mut().write_string(
                "use COMMAND \"search\" to find shortcuts",
                self.theme_default.fg,
                self.theme_default.bg,
                self.theme_default.attrs,
                help_area.skip(2, 1),
                None,
                None,
            );
            let mut idx = 2;
            for (desc, shortcuts) in children_maps.iter() {
                let help_area = self.help_view.content.area();
                self.help_view.content.grid_mut().write_string(
                    desc,
                    self.theme_default.fg,
                    self.theme_default.bg,
                    self.theme_default.attrs,
                    help_area.skip(2, 2 + idx),
                    None,
                    None,
                );
                idx += 2;
                for (k, v) in shortcuts {
                    let help_area = self.help_view.content.area();
                    let (x, _) = self.help_view.content.grid_mut().write_string(
                        &format!("{v: >max_first_column_width$}"),
                        self.theme_default.fg,
                        self.theme_default.bg,
                        self.theme_default.attrs | Attr::BOLD,
                        help_area.skip(2, 2 + idx),
                        None,
                        None,
                    );
                    let help_area = self.help_view.content.area();
                    self.help_view.content.grid_mut().write_string(
                        k,
                        self.theme_default.fg,
                        self.theme_default.bg,
                        self.theme_default.attrs,
                        help_area.skip(x + 4, 2 + idx),
                        None,
                        None,
                    );
                    idx += 1;
                }
                idx += 1;
            }
            self.help_view.curr_views = children_maps;
            let dialog_area = crate::terminal::ratatui_bridge::center_inside_via_layout(
                // add box perimeter padding
                area,
                {
                    let (w, h) = self.help_view.content.area().size();
                    (w + 1, h + 1)
                },
            );
            context.dirty_areas.push_back(dialog_area);
            grid.clear_area(dialog_area, self.theme_default);
            let inner_area = draw_rounded_frame(
                grid,
                dialog_area,
                crate::conf::value(context, "tab.focused"),
            );
            for frame_area in frame_flush_areas(grid, dialog_area) {
                context.dirty_areas.push_back(frame_area);
            }
            let (x, y) = grid.write_string(
                "shortcuts",
                crate::conf::value(context, "tab.focused").fg,
                self.theme_default.bg,
                self.theme_default.attrs | Attr::BOLD,
                inner_area.skip_cols(2),
                None,
                None,
            );
            grid.write_string(
                &format!(
                    "Press {} to close",
                    self.help_view.curr_views[Shortcuts::GENERAL]["toggle_help"]
                ),
                crate::conf::value(context, "tab.unfocused").fg,
                self.theme_default.bg,
                self.theme_default.attrs | Attr::ITALICS,
                inner_area.skip(4 + x, y),
                None,
                None,
            );
            let inner_area = inner_area.skip_rows(y + 1).skip_rows_from_end(1);
            let (width, height) = self.help_view.content.area().size();
            let (cols, rows) = inner_area.size();
            if let Some(ref mut search) = self.help_view.search {
                use crate::melib::text::search::KMP;
                search.positions = self
                    .help_view
                    .content
                    .grid()
                    .kmp_search(&search.pattern)
                    .into_iter()
                    .map(|offset| (offset / width, offset % width))
                    .collect::<Vec<(usize, usize)>>();
                let results_attr = crate::conf::value(context, "pager.highlight_search");
                let results_current_attr =
                    crate::conf::value(context, "pager.highlight_search_current");
                search.cursor =
                    std::cmp::min(search.positions.len().saturating_sub(1), search.cursor);
                for (i, (y, x)) in search.positions.iter().enumerate() {
                    let area = self.help_view.content.area();
                    for c in self.help_view.content.grid().row_iter(
                        area,
                        *x..*x + search.pattern.grapheme_len(),
                        *y,
                    ) {
                        if i == search.cursor {
                            self.help_view.content.grid_mut()[c]
                                .set_fg(results_current_attr.fg)
                                .set_bg(results_current_attr.bg)
                                .set_attrs(results_current_attr.attrs);
                        } else {
                            self.help_view.content.grid_mut()[c]
                                .set_fg(results_attr.fg)
                                .set_bg(results_attr.bg)
                                .set_attrs(results_attr.attrs);
                        }
                    }
                }
                if !search.positions.is_empty() {
                    if let Some(mvm) = search.movement.take() {
                        match mvm {
                            SearchMovement::First => {
                                if self.help_view.cursor.1 > search.positions[search.cursor].0 {
                                    self.help_view.cursor.1 = search.positions[search.cursor].0;
                                }
                                if self.help_view.cursor.1 + rows
                                    < search.positions[search.cursor].0
                                {
                                    self.help_view.cursor.1 = search.positions[search.cursor].0;
                                }
                            }
                            SearchMovement::Previous
                                if self.help_view.cursor.1 > search.positions[search.cursor].0 =>
                            {
                                self.help_view.cursor.1 = search.positions[search.cursor].0;
                            }
                            SearchMovement::Next
                                if self.help_view.cursor.1 + rows
                                    < search.positions[search.cursor].0 =>
                            {
                                self.help_view.cursor.1 = search.positions[search.cursor].0;
                            }
                            _ => {}
                        }
                    }
                }
            }
            /* trim cursor if it's bigger than the help screen */
            self.help_view.cursor = (
                std::cmp::min((width).saturating_sub(cols), self.help_view.cursor.0),
                std::cmp::min((height).saturating_sub(rows), self.help_view.cursor.1),
            );
            if cols == 0 || rows == 0 {
                return;
            }

            /* In this case we will be scrolling, so show the user how to do it */
            if height.wrapping_div(rows + 1) > 0 || width.wrapping_div(cols + 1) > 0 {
                let help_area = self.help_view.content.area();
                self.help_view.content.grid_mut().write_string(
                    &format!(
                        "Use {down}, {up}, {right}, {left} to scroll.",
                        down = self.help_view.curr_views[Shortcuts::GENERAL]["scroll_down"],
                        up = self.help_view.curr_views[Shortcuts::GENERAL]["scroll_up"],
                        right = self.help_view.curr_views[Shortcuts::GENERAL]["scroll_right"],
                        left = self.help_view.curr_views[Shortcuts::GENERAL]["scroll_left"],
                    ),
                    self.theme_default.fg,
                    self.theme_default.bg,
                    self.theme_default.attrs | Attr::ITALICS,
                    help_area.skip(2, 2),
                    None,
                    None,
                );
            }

            grid.copy_area(
                self.help_view.content.grid(),
                inner_area,
                self.help_view
                    .content
                    .area()
                    .skip(
                        std::cmp::min((width - 1).saturating_sub(cols), self.help_view.cursor.0),
                        std::cmp::min((height - 1).saturating_sub(rows), self.help_view.cursor.1),
                    )
                    .take_rows(std::cmp::min(rows, height - 1)),
            );
            if height.wrapping_div(rows + 1) > 0 || width.wrapping_div(cols + 1) > 0 {
                context
                    .replies
                    .push_back(UIEvent::StatusEvent(StatusEvent::ScrollUpdate(
                        ScrollUpdate::Update {
                            id: self.id,
                            context: ScrollContext {
                                shown_lines: std::cmp::min(
                                    (height).saturating_sub(rows),
                                    self.help_view.cursor.1,
                                ) + rows,
                                total_lines: height,
                                has_more_lines: false,
                            },
                        },
                    )));
                ScrollBar::default().set_show_arrows(true).draw(
                    grid,
                    inner_area.nth_col(inner_area.width().saturating_sub(1)),
                    context,
                    /* position */
                    std::cmp::min((height).saturating_sub(rows), self.help_view.cursor.1),
                    /* visible_rows */
                    rows,
                    /* length */
                    height,
                );
            } else {
                context
                    .replies
                    .push_back(UIEvent::StatusEvent(StatusEvent::ScrollUpdate(
                        ScrollUpdate::End(self.id),
                    )));
            }
        }
        self.dirty = false;
    }

    fn process_event(&mut self, mut event: &mut UIEvent, context: &mut Context) -> bool {
        let shortcuts = &self.help_view.curr_views;
        match &mut event {
            UIEvent::ConfigReload { old_settings: _ } => {
                self.theme_default = crate::conf::value(context, "theme_default");
                self.set_dirty(true);
            }
            UIEvent::Input(Key::Alt(no)) if *no >= '1' && *no <= '9' => {
                let no = *no as usize - '1' as usize;
                if no < self.children.len() && self.cursor_pos != no % self.children.len() {
                    self.children[self.cursor_pos]
                        .process_event(&mut UIEvent::VisibilityChange(false), context);
                    self.cursor_pos = no % self.children.len();
                    self.update_help_curr_views(context);
                    let status = self.children[self.cursor_pos].status(context);
                    let status_watch = self.children[self.cursor_pos].status_watch();
                    let replies = &mut context.replies;
                    self.push_focus_updates(status, status_watch, replies);
                    self.set_dirty(true);
                }
                return true;
            }
            UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::GENERAL]["next_tab"]) =>
            {
                self.children[self.cursor_pos]
                    .process_event(&mut UIEvent::VisibilityChange(false), context);
                self.cursor_pos = (self.cursor_pos + 1) % self.children.len();
                self.update_help_curr_views(context);
                self.push_focus_updates(
                    self.children[self.cursor_pos].status(context),
                    self.children[self.cursor_pos].status_watch(),
                    &mut context.replies,
                );
                self.set_dirty(true);
                return true;
            }
            UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::GENERAL]["toggle_help"])
                    || (self.show_shortcuts
                        && (key == Key::Esc
                            || shortcut!(key == shortcuts[Shortcuts::GENERAL]["quit"]))) =>
            {
                if self.show_shortcuts {
                    // Children below the shortcut overlay must be redrawn.
                    self.set_dirty(true);
                    context
                        .replies
                        .push_back(UIEvent::StatusEvent(StatusEvent::ScrollUpdate(
                            ScrollUpdate::End(self.id),
                        )));
                }
                self.show_shortcuts = !self.show_shortcuts;
                self.dirty = true;
                return true;
            }
            UIEvent::Action(Tab(New(ref mut e @ Some(_)))) => {
                self.add_component(e.take().unwrap(), context);
                self.children[self.cursor_pos]
                    .process_event(&mut UIEvent::VisibilityChange(false), context);
                self.cursor_pos = self.children.len() - 1;
                self.children[self.cursor_pos].set_dirty(true);
                self.update_help_curr_views(context);
                return true;
            }
            UIEvent::Input(ref key)
                if !self.show_shortcuts
                    && self.cursor_pos >= self.pinned
                    && shortcut!(key == shortcuts[Shortcuts::GENERAL]["quit"]) =>
            {
                // Layered quit: let the focused non-pinned child interpret
                // the quit binding first (a dirty composer turns it into
                // its unsaved-changes dialog); only an unconsumed quit
                // closes the tab. Pinned tabs do not consume it, letting
                // it bubble up to the application-level exit path in
                // `StatusBar`.
                if self.children[self.cursor_pos].process_event(event, context) {
                    return true;
                }
                // A child that cannot quit cleanly *vetoes* the kill: with
                // its unsaved-changes dialog already open the child does
                // not consume a second quit key, and killing the tab here
                // would silently discard the work the dialog was asking
                // about. The dialog owns the decision (its own `x`/`y`
                // choices kill the tab); the quit binding must not make it
                // for the user.
                if !self.children[self.cursor_pos].can_quit_cleanly(context) {
                    return true;
                }
                let id = self.children[self.cursor_pos].id();
                context.replies.push_back(UIEvent::Action(Tab(Kill(id))));
                return true;
            }
            UIEvent::Action(Tab(Close)) => {
                if self.pinned > self.cursor_pos {
                    return true;
                }
                let id = self.children[self.cursor_pos].id();
                self.children[self.cursor_pos].kill(id, context);
                self.update_help_curr_views(context);
                self.set_dirty(true);
                return true;
            }
            UIEvent::Action(Tab(Kill(id))) => {
                if self.pinned > self.cursor_pos {
                    return true;
                }
                if let Some(c_idx) = self.children.iter().position(|x| x.id() == *id) {
                    self.children[c_idx]
                        .process_event(&mut UIEvent::VisibilityChange(false), context);
                    self.children[c_idx].unrealize(context);
                    self.children.remove(c_idx);
                    self.inset_children.remove(c_idx);
                    self.cursor_pos = 0;
                    self.set_dirty(true);
                    self.update_help_curr_views(context);
                    return true;
                } else {
                    log::debug!(
                        "Child component with id {:?} not found.\nList: {:?}",
                        id,
                        self.children
                    );
                }
            }
            UIEvent::Action(Action::Listing(ListingAction::Search { term: pattern, .. }))
                if self.show_shortcuts =>
            {
                self.help_view.search = Some(SearchPattern {
                    pattern: pattern.to_string(),
                    positions: vec![],
                    cursor: 0,
                    movement: Some(SearchMovement::First),
                });
                self.dirty = true;
                return true;
            }
            UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::GENERAL]["next_search_result"])
                    && self.show_shortcuts
                    && self.help_view.search.is_some() =>
            {
                if let Some(ref mut search) = self.help_view.search {
                    search.movement = Some(SearchMovement::Next);
                    search.cursor += 1;
                } else {
                    // The match guard above proved `search` is `Some`, so
                    // this is unreachable; `unreachable!()` documents that
                    // and panics instead of invoking UB if the invariant
                    // is ever broken.
                    unreachable!("help-view search guard verified `search` is Some");
                }
                self.dirty = true;
                return true;
            }
            UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::GENERAL]["previous_search_result"])
                    && self.show_shortcuts
                    && self.help_view.search.is_some() =>
            {
                if let Some(ref mut search) = self.help_view.search {
                    search.movement = Some(SearchMovement::Previous);
                    search.cursor = search.cursor.saturating_sub(1);
                } else {
                    // The match guard above proved `search` is `Some`, so
                    // this is unreachable; `unreachable!()` documents that
                    // and panics instead of invoking UB if the invariant
                    // is ever broken.
                    unreachable!("help-view search guard verified `search` is Some");
                }
                self.dirty = true;
                return true;
            }
            UIEvent::Input(Key::Esc) if self.show_shortcuts && self.help_view.search.is_some() => {
                self.help_view.search = None;
                self.dirty = true;
                return true;
            }
            UIEvent::Resize => {
                self.dirty = true;
            }
            UIEvent::Input(ref key)
                if self.show_shortcuts
                    && shortcut!(key == shortcuts[Shortcuts::LISTING]["search"]) =>
            {
                context
                    .replies
                    .push_back(UIEvent::CmdInput(Key::Paste("search ".to_string())));
                context
                    .replies
                    .push_back(UIEvent::ChangeMode(UIMode::Command));
                return true;
            }
            UIEvent::Input(ref key) if self.show_shortcuts => {
                match key {
                    _ if shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_up"]) => {
                        self.help_view.cursor.1 = self.help_view.cursor.1.saturating_sub(1);
                    }
                    _ if shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_down"]) => {
                        self.help_view.cursor.1 += 1;
                    }
                    _ if shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_left"]) => {
                        self.help_view.cursor.0 = self.help_view.cursor.0.saturating_sub(1);
                    }
                    _ if shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_right"]) => {
                        self.help_view.cursor.0 += 1;
                    }
                    _ => {
                        /* ignore, don't pass to components below the shortcut panel */
                        return false;
                    }
                }
                self.dirty = true;
                return true;
            }
            _ => {}
        }
        let c = self.cursor_pos;
        if let UIEvent::Input(_) | UIEvent::CmdInput(_) | UIEvent::EmbeddedInput(_) = event {
            self.children[c].process_event(event, context)
        } else {
            self.children[c].process_event(event, context)
                || self.children.iter_mut().enumerate().any(|(idx, child)| {
                    if idx == c {
                        return false;
                    }
                    child.process_event(event, context)
                })
        }
    }

    fn is_dirty(&self) -> bool {
        self.dirty || self.children[self.cursor_pos].is_dirty()
    }

    fn set_dirty(&mut self, value: bool) {
        self.dirty = value;
        self.children[self.cursor_pos].set_dirty(value);
    }

    fn id(&self) -> ComponentId {
        self.id
    }

    fn shortcuts(&self, context: &Context) -> ShortcutMaps {
        // Aggregate the focused child's shortcuts under the active
        // section name (e.g. "listing", "pager", "contact_list") plus
        // the general section. The StatusBar reads this map for its
        // hints segment and needs both layers.
        let mut map = ShortcutMaps::default();
        map.insert(
            Shortcuts::GENERAL,
            context.settings.shortcuts.general.key_values(),
        );
        if let Some(child) = self.children.get(self.cursor_pos) {
            map.extend(child.shortcuts(context));
        }
        map
    }

    fn can_quit_cleanly(&mut self, context: &Context) -> bool {
        for (i, c) in self.children.iter_mut().enumerate() {
            if !c.can_quit_cleanly(context) {
                self.cursor_pos = i;
                self.set_dirty(true);
                return false;
            }
        }
        true
    }

    fn status_watch(&self) -> Option<(AccountHash, MailboxHash)> {
        self.children
            .get(self.cursor_pos)
            .and_then(|c| c.status_watch())
    }

    fn attributes(&self) -> &'static ComponentAttr {
        &ComponentAttr::CONTAINER
    }

    fn children(&self) -> IndexMap<ComponentId, &dyn Component> {
        let mut ret = IndexMap::default();
        for c in &self.children {
            ret.insert(c.id(), c as &dyn Component);
        }
        ret
    }

    fn children_mut(&mut self) -> IndexMap<ComponentId, &mut dyn Component> {
        IndexMap::default()
    }

    fn realize(&self, parent: Option<ComponentId>, context: &mut Context) {
        context.realized.insert(self.id(), parent);
        for c in &self.children {
            c.realize(self.id().into(), context);
        }
    }
}

/// One colored run of the status-bar hints segment. The key glyph of a
/// `(key:label)` hint renders in the theme's highlight-selected color
/// (+bold) so the actionable binding stands out from the descriptive
/// label text.
#[derive(Debug)]
struct HintSpan {
    text: String,
    /// Render with the theme's highlight-selected color and bold (the
    /// key glyph of a hint).
    key: bool,
}

impl HintSpan {
    fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            key: false,
        }
    }

    fn key(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            key: true,
        }
    }
}

/// Truncate `spans` from the tail until their joined width fits in
/// `max_width` cells. When at least one grapheme was dropped, the
/// result is terminated with `ellipsis` (which itself counts toward
/// the width budget). Returns the spans unchanged if they already
/// fit. Span-aware sibling of the former string-only truncation: the
/// cut can land mid-span, in which case the trimmed span keeps its
/// own coloring and the ellipsis is appended as a plain run.
fn truncate_spans_with_ellipsis(
    spans: Vec<HintSpan>,
    max_width: usize,
    ellipsis: &str,
) -> Vec<HintSpan> {
    let total: usize = spans.iter().map(|span| span.text.grapheme_width()).sum();
    if total <= max_width {
        return spans;
    }
    let ellipsis_w = ellipsis.grapheme_width();
    if max_width <= ellipsis_w {
        // Not enough room for any content + ellipsis; clip ellipsis.
        return vec![HintSpan::plain(
            ellipsis
                .split_graphemes()
                .into_iter()
                .take(max_width)
                .collect::<String>(),
        )];
    }
    let budget = max_width - ellipsis_w;
    let mut out = Vec::with_capacity(spans.len());
    let mut used = 0;
    for span in spans {
        let width = span.text.grapheme_width();
        if used + width <= budget {
            out.push(span);
            used += width;
            continue;
        }
        // Boundary span: trim it grapheme by grapheme, then stop —
        // everything after it is dropped.
        let mut text = String::new();
        for g in span.text.split_graphemes() {
            let gw = g.grapheme_width();
            if used + gw > budget {
                break;
            }
            text.push_str(g);
            used += gw;
        }
        if !text.is_empty() {
            out.push(HintSpan {
                text,
                key: span.key,
            });
        }
        out.push(HintSpan::plain(ellipsis));
        return out;
    }
    out
}

/*
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawBuffer {
    pub buf: CellBuffer,
    title: Option<String>,
    cursor: (usize, usize),
    id: ComponentId,
    dirty: bool,
}

impl std::fmt::Display for RawBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        std::fmt::Display::fmt("Raw buffer", f)
    }
}

impl Component for RawBuffer {
    fn draw(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        if self.dirty {
            let (width, height) = self.buf.size();
            let (cols, rows) = (area.width(), area.height());
            self.cursor = (
                std::cmp::min(width.saturating_sub(cols), self.cursor.0),
                std::cmp::min(height.saturating_sub(rows), self.cursor.1),
            );
            grid.clear_area(area, crate::conf::value(context, "theme_default"));

            grid.copy_area(
                &self.buf,
                area,
                (
                    (
                        std::cmp::min((width - 1).saturating_sub(cols), self.cursor.0),
                        std::cmp::min((height - 1).saturating_sub(rows), self.cursor.1),
                    ),
                    (
                        std::cmp::min(self.cursor.0 + cols, width - 1),
                        std::cmp::min(self.cursor.1 + rows, height - 1),
                    ),
                ),
            );
            context.dirty_areas.push_back(area);
            self.dirty = false;
        }
    }
    fn process_event(&mut self, event: &mut UIEvent, _context: &mut Context) -> bool {
        match *event {
            UIEvent::Input(Key::Left) => {
                self.cursor.0 = self.cursor.0.saturating_sub(1);
                self.dirty = true;
                true
            }
            UIEvent::Input(Key::Right) => {
                self.cursor.0 += 1;
                self.dirty = true;
                true
            }
            UIEvent::Input(Key::Up) => {
                self.cursor.1 = self.cursor.1.saturating_sub(1);
                self.dirty = true;
                true
            }
            UIEvent::Input(Key::Down) => {
                self.cursor.1 += 1;
                self.dirty = true;
                true
            }
            _ => false,
        }
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

impl RawBuffer {
    pub fn new(buf: CellBuffer, title: Option<String>) -> Self {
        RawBuffer {
            buf,
            title,
            cursor: (0, 0),
            dirty: true,
            id: ComponentId::default(),
        }
    }
    pub fn title(&self) -> &str {
        self.title.as_deref().unwrap_or("untitled")
    }
}
*/
