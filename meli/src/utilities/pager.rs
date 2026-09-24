/*
 * meli - pager
 *
 * Copyright 2020 Manos Pitsidianakis
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

use melib::text::{Line, LineBreakText};

use super::*;
use crate::{
    jobs::{IsAsync, JoinHandle},
    terminal::embedded::EmbeddedGrid,
};

/// Collapse runs of more than two consecutive blank lines (lines with no
/// visible characters — empty or whitespace-only) down to exactly two in
/// the pager reading view. Runs of one or two blank lines are kept verbatim
/// and non-blank content is untouched; applied uniformly to ASCII, CJK and
/// mixed text. Returns the original string untouched when no run exceeds
/// two, so the common case allocates nothing.
fn collapse_blank_line_runs(text: &str) -> std::borrow::Cow<'_, str> {
    let mut blanks = 0usize;
    let mut excess = false;
    for line in text.split_inclusive('\n') {
        if line.trim().is_empty() {
            blanks += 1;
            if blanks > 2 {
                excess = true;
                break;
            }
        } else {
            blanks = 0;
        }
    }
    if !excess {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut ret = String::with_capacity(text.len());
    let mut blanks = 0usize;
    for line in text.split_inclusive('\n') {
        if line.trim().is_empty() {
            blanks += 1;
            if blanks > 2 {
                continue;
            }
        } else {
            blanks = 0;
        }
        ret.push_str(line);
    }
    std::borrow::Cow::Owned(ret)
}

/// Render a `ratatui::widgets::Scrollbar` over `area` into `grid`.
///
/// `position` is the pager's first visible row/column, `content_len` the
/// total wrapped lines/columns and `viewport_len` the visible window size,
/// so the thumb length and offset encode the reading-position ratio. The
/// widget renders into a temporary ratatui buffer pre-filled with
/// `theme_default` (so the gutter keeps the theme background) and is
/// blitted back onto the grid — the same bridge pattern
/// `StatusBar::render_line_gauge` uses.
pub fn draw_scrollbar(
    grid: &mut CellBuffer,
    area: Area,
    context: &Context,
    horizontal: bool,
    position: usize,
    viewport_len: usize,
    content_len: usize,
) {
    use ratatui::widgets::{
        Scrollbar as RatatuiScrollbar, ScrollbarOrientation, ScrollbarState,
        StatefulWidget as RatatuiStatefulWidget,
    };

    if area.is_empty() || content_len == 0 {
        return;
    }
    let theme_default = crate::conf::value(context, "theme_default");
    let mut thumb = crate::conf::value(context, "widgets.options.highlighted");
    if !context.settings.terminal.use_color() {
        thumb.attrs |= Attr::REVERSE;
    }
    // Same palette as the previous meli `ScrollBar`: thumb/arrow foreground
    // is the highlighted option's background color.
    let thumb_style = ratatui::style::Style::new()
        .fg(thumb.bg.into())
        .add_modifier(thumb.attrs.into());
    let track_style = ratatui::style::Style::from(theme_default);
    let orientation = if horizontal {
        ScrollbarOrientation::HorizontalBottom
    } else {
        ScrollbarOrientation::VerticalRight
    };
    let mut scrollbar = RatatuiScrollbar::new(orientation)
        .thumb_style(thumb_style)
        .track_style(track_style)
        .begin_style(thumb_style)
        .end_style(thumb_style);
    if grid.ascii_drawing {
        scrollbar = scrollbar
            .thumb_symbol("#")
            .track_symbol(Some(if horizontal { "-" } else { "|" }))
            .begin_symbol(Some(if horizontal { "<" } else { "^" }))
            .end_symbol(Some(if horizontal { ">" } else { "v" }));
    }
    // ratatui's `position` ranges over `0..content_length` (position =
    // content_length - 1 puts the thumb exactly at the track bottom) and
    // `viewport_content_length` only sizes the thumb. Map the pager's
    // scroll range (positions 0..=content_len - viewport_len) onto that
    // scale so the thumb both starts at the top and ends flush at the
    // bottom.
    let scrollable = content_len
        .saturating_sub(viewport_len)
        .saturating_add(1)
        .max(1);
    let mut state = ScrollbarState::new(scrollable)
        .position(position)
        .viewport_content_length(viewport_len);
    let rect = ratatui::layout::Rect::new(0, 0, area.width() as u16, area.height() as u16);
    let mut buf = ratatui::buffer::Buffer::empty(rect);
    // Pre-fill with the theme so untouched cells keep the theme background
    // after the blit (parity with the old clear_area-then-draw behaviour).
    // One call instead of a bounds-checked `cell_mut` per cell.
    buf.set_style(rect, track_style);
    RatatuiStatefulWidget::render(scrollbar, buf.area, &mut buf, &mut state);
    crate::terminal::ratatui_bridge::blit_buffer_to_cellbuffer_at(&buf, grid, area);
}

/// A pager for text.
/// `Pager` holds its own content in its own `CellBuffer` and when `draw` is
/// called, it draws the current view of the text. It is responsible for
/// scrolling etc.
#[derive(Debug, Default)]
pub struct Pager {
    text: String,
    cursor: (usize, usize),
    reflow: Reflow,
    height: usize,
    width: usize,
    minimum_width: usize,
    search: Option<SearchPattern>,
    dirty: bool,

    colors: ThemeAttribute,
    /// The hosting pane's background ("pane.focused"/"pane.unfocused"),
    /// set by the parent (e.g. the envelope view) so the body text and the
    /// blank cells around it follow the keyboard focus. `None` falls back
    /// to `theme_default`.
    pane_fill: Option<ThemeAttribute>,
    initialised: bool,
    show_scrollbar: bool,
    /// Reserve the last column for a scrollbar drawn by a parent
    /// component (e.g. the mail view's combined headers+body scrollbar),
    /// without the pager drawing its own. Only affects the wrap width;
    /// `show_scrollbar` still controls whether the pager draws one.
    reserve_scrollbar_column: bool,
    /// At the last draw, were the visible columns plus horizontal cursor less
    /// than total width? Used to decide whether to accept `scroll_right`
    /// key events.
    cols_lt_width: bool,
    /// At the last draw, were the visible rows plus vertical cursor less than
    /// total height? Used to decide whether to accept `scroll_down` key
    /// events.
    rows_lt_height: bool,
    filtered_content: Option<(String, EmbeddedGrid)>,
    filter_job: Option<(String, JoinHandle<Result<EmbeddedGrid>>)>,
    text_lines: Vec<Line>,
    line_breaker: LineBreakText,
    /// Cached `linkify` scan of [`Pager::text`], computed on first use and
    /// invalidated whenever the text changes.
    ///
    /// `draw_page` used to rescan the entire body (not just the visible
    /// lines) and allocate a fresh `Vec<Link>` on every draw, so a few
    /// hundred KB of mail cost a full scan per scroll step.
    links: Option<Vec<Link<'static>>>,
    /// Total wrapped line count of [`Pager::text`], computed on first use.
    ///
    /// [`Pager::size`] reports only the lines materialized so far, because
    /// the line breaker is consumed lazily while rendering. A scrollbar built
    /// from that count keeps growing as the user reads and cannot show
    /// progress over content that has not been wrapped yet, so the mail view
    /// asks for the real total instead. Invalidated whenever the text or the
    /// wrap width changes.
    total_height: Option<usize>,
    movement: Option<PageMovement>,
    id: ComponentId,
}

impl Clone for Pager {
    fn clone(&self) -> Self {
        Self {
            filter_job: None,
            text: self.text.clone(),
            cursor: self.cursor,
            reflow: self.reflow,
            height: self.height,
            width: self.width,
            minimum_width: self.minimum_width,
            search: self.search.clone(),
            dirty: true,
            colors: self.colors,
            pane_fill: self.pane_fill,
            initialised: false,
            show_scrollbar: self.show_scrollbar,
            reserve_scrollbar_column: self.reserve_scrollbar_column,
            cols_lt_width: self.cols_lt_width,
            rows_lt_height: self.rows_lt_height,
            filtered_content: self.filtered_content.clone(),
            text_lines: self.text_lines.clone(),
            line_breaker: self.line_breaker.clone(),
            links: self.links.clone(),
            total_height: self.total_height,
            movement: self.movement,
            id: ComponentId::default(),
        }
    }
}

impl std::fmt::Display for Pager {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "pager")
    }
}

impl Pager {
    const PAGES_AHEAD_TO_RENDER_NO: usize = 16;

    pub fn new(context: &Context) -> Self {
        let mut ret = Self {
            minimum_width: context.settings.pager.minimum_width,
            ..Self::default()
        };
        ret.set_colors(crate::conf::value(context, "theme_default"))
            .set_reflow(if context.settings.pager.split_long_lines {
                Reflow::All
            } else {
                Reflow::No
            });
        ret
    }

    pub fn set_show_scrollbar(&mut self, new_val: bool) -> &mut Self {
        self.show_scrollbar = new_val;
        self
    }

    /// Reserve the last column for a parent-drawn scrollbar (wrap width
    /// minus one) without the pager drawing its own scrollbar.
    pub fn set_reserve_scrollbar_column(&mut self, new_val: bool) -> &mut Self {
        self.reserve_scrollbar_column = new_val;
        self
    }
    pub fn set_colors(&mut self, new_val: ThemeAttribute) -> &mut Self {
        self.colors = new_val;
        self
    }

    pub fn set_reflow(&mut self, new_val: Reflow) -> &mut Self {
        self.reflow = new_val;
        self
    }

    pub fn set_initialised(&mut self, new_val: bool) -> &mut Self {
        self.initialised = new_val;
        self
    }

    pub fn reflow(&self) -> Reflow {
        self.reflow
    }

    pub fn update_from_str(&mut self, text: &str, mut width: Option<usize>) {
        if let Some(ref mut width) = width.as_mut() {
            if **width < self.minimum_width {
                **width = self.minimum_width;
            }
        }

        self.text = collapse_blank_line_runs(text).into_owned();
        self.links = None;
        self.total_height = None;
        self.text_lines.clear();
        self.line_breaker = LineBreakText::new(self.text.clone(), self.reflow, width);
        self.height = 0;
        self.width = 0;
        self.search = None;
        self.set_dirty(true);
        self.initialised = false;
        self.cursor = (0, 0);
    }

    /// One-shot `linkify` scan of `text`, cached in [`Pager::links`].
    fn scan_links(text: &str) -> Vec<Link<'static>> {
        let finder = linkify::LinkFinder::new();
        finder
            .links(text)
            .filter_map(|l| {
                Some(Link {
                    start: l.start(),
                    end: l.end(),
                    value: std::borrow::Cow::Owned(l.as_str().to_string()),
                    kind: match l.kind() {
                        linkify::LinkKind::Url => LinkKind::Url,
                        linkify::LinkKind::Email => LinkKind::Email,
                        _ => return None,
                    },
                })
            })
            .collect()
    }

    pub fn from_string(
        text: String,
        context: &Context,
        cursor_pos: Option<usize>,
        mut width: Option<usize>,
        colors: ThemeAttribute,
    ) -> Self {
        let pager_filter: Option<&String> = context.settings.pager.filter.as_ref();

        let pager_minimum_width: usize = context.settings.pager.minimum_width;

        let reflow: Reflow = if context.settings.pager.split_long_lines {
            Reflow::All
        } else {
            Reflow::No
        };

        if let Some(ref mut width) = width.as_mut() {
            if **width < pager_minimum_width {
                **width = pager_minimum_width;
            }
        }

        // Collapse runs of more than two consecutive blank lines down to
        // two for the reading view. Keep the original allocation when
        // nothing changes.
        let text = match collapse_blank_line_runs(&text) {
            std::borrow::Cow::Borrowed(_) => text,
            std::borrow::Cow::Owned(collapsed) => collapsed,
        };

        let mut ret = Self {
            text,
            text_lines: vec![],
            reflow,
            cursor: (0, cursor_pos.unwrap_or(0)),
            height: 1,
            width: 1,
            minimum_width: pager_minimum_width,
            initialised: false,
            dirty: true,
            id: ComponentId::default(),
            filtered_content: None,
            colors,
            ..Default::default()
        };

        if let Some(bin) = pager_filter {
            ret.filter(bin, context);
        }

        ret
    }

    /// Set the hosting pane's background fill; see the `pane_fill` field.
    pub fn set_pane_fill(&mut self, fill: Option<ThemeAttribute>) {
        self.pane_fill = fill;
    }

    /// The color the pager paints blank cells and plain body text with:
    /// the hosting pane's fill when set, `theme_default` otherwise.
    fn base_fill(&self, context: &Context) -> ThemeAttribute {
        self.pane_fill
            .unwrap_or_else(|| crate::conf::value(context, "theme_default"))
    }

    pub fn filter(&mut self, cmd: &str, context: &Context) {
        // Do not spawn a duplicate filter process for the same command: if a
        // filter job for this exact command is already in flight, keep it.
        // A different command still replaces the in-flight job.
        if self
            .filter_job
            .as_ref()
            .is_some_and(|(ongoing_cmd, _)| ongoing_cmd == cmd)
        {
            return;
        }
        async fn filter_fut(bin: String, text: String, tab_width: u8) -> Result<EmbeddedGrid> {
            use std::{
                io::Write,
                process::{Command, Stdio},
            };
            let mut filter_child = Command::new("sh")
                .args(["-c", &bin])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .chain_err_summary(|| "Failed to start pager filter process")?;
            let stdin = filter_child.stdin.as_mut().ok_or("failed to open stdin")?;
            stdin
                .write_all(text.as_bytes())
                .chain_err_summary(|| "Failed to write to stdin")?;
            let out = filter_child
                .wait_with_output()
                .chain_err_summary(|| "Failed to wait on filter")?;
            if !out.status.success() {
                let mut err = Error::new("Failed to wait on filter").set_kind(ErrorKind::External);
                if !out.stderr.is_empty() {
                    err = err.set_summary(String::from_utf8_lossy(&out.stderr).to_string());
                }
                return Err(err);
            }
            let stdout = out.stdout;
            let mut dev_null = std::fs::File::open("/dev/null")?;
            let mut embedded = EmbeddedGrid::new();
            embedded.set_tab_width(tab_width);
            embedded.set_terminal_size((80, 20));

            for b in stdout {
                embedded.process_byte(&mut dev_null, b);
            }
            Ok(embedded)
        }
        let tab_width = context.settings.terminal.tab_width;
        let fut = Box::pin(filter_fut(cmd.to_string(), self.text.clone(), tab_width));
        let handle = context.main_loop_handler.job_executor.spawn(
            format!("Running pager filter {cmd}").into(),
            fut,
            IsAsync::Blocking,
        );
        self.filter_job = Some((cmd.to_string(), handle));
    }

    pub fn cursor_pos(&self) -> usize {
        self.cursor.1
    }

    pub fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// The total number of wrapped lines in the current text (not just the
    /// prefix materialized so far), counted once and cached.
    ///
    /// Counting runs the line-breaking state machine over the remaining text
    /// on a cheap clone of the breaker (the text is shared through an
    /// `Arc<str>`), so it costs one pass over the body and no extra copy.
    pub fn total_height(&mut self) -> usize {
        if let Some(total) = self.total_height {
            return total;
        }
        let total = if self.filtered_content.is_some() {
            // Filtered content is an already-rendered grid: `height` is its
            // full height.
            self.height
        } else {
            self.height
                .saturating_add(self.line_breaker.remaining_lines())
        };
        self.total_height = Some(total);
        total
    }

    pub fn initialise(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        // Wrap at the *actual* area width. Clamping the wrap width up to
        // `pager.minimum_width` (default 80) made every wrapped line wider
        // than a narrow pane's pager area; `CellBuffer::write_string` then
        // clipped each line on draw, losing the clipped tail (it is not
        // reachable by scrolling), and a wide grapheme starting at the last
        // column spilled one cell past the pane edge.
        //
        // The only column that must be reserved is the vertical scrollbar
        // gutter (drawn over the last column when the text is taller than
        // the pane); the previous unconditional `area.width() - 4` slack
        // left up to four columns unused per line for no reason.
        let width = area.width().saturating_sub(usize::from(
            self.show_scrollbar || self.reserve_scrollbar_column,
        ));
        if self.filtered_content.is_none() {
            if self.line_breaker.width() != Some(width) {
                let line_breaker = LineBreakText::new(self.text.clone(), self.reflow, Some(width));

                self.line_breaker = line_breaker;
                self.text_lines.clear();
                self.total_height = None;
            };
            self.height = self.text_lines.len();
            self.width = width;
            if let Some(ref mut search) = self.search {
                use melib::text::search::KMP;
                search.positions.clear();
                for (y, l) in self.text_lines.iter().enumerate() {
                    search.positions.extend(
                        l.content
                            .kmp_search(&search.pattern)
                            .into_iter()
                            .map(|offset| (y, offset)),
                    );
                }
                if let Some(pos) = search.positions.get(search.cursor) {
                    if self.cursor.1 > pos.0 || self.cursor.1 + area.height() < pos.0 {
                        self.cursor.1 = pos.0.saturating_sub(3);
                    }
                }
            }
            self.draw_lines_up_to(
                grid,
                area,
                context,
                self.cursor.1 + Self::PAGES_AHEAD_TO_RENDER_NO * area.height(),
            );
        }
        self.draw_page(grid, area, context);

        self.initialised = true;
    }

    pub fn draw_lines_up_to(
        &mut self,
        _grid: &mut CellBuffer,
        area: Area,
        _context: &mut Context,
        up_to: usize,
    ) {
        if self.line_breaker.is_finished() || self.filtered_content.is_some() {
            return;
        }
        let old_lines_no = self.text_lines.len();
        if up_to == 0 {
            self.text_lines.extend(self.line_breaker.by_ref());
        } else {
            if old_lines_no >= up_to + area.height() {
                return;
            }
            let new_lines_no = (up_to + area.height()) - old_lines_no;
            self.text_lines
                .extend(self.line_breaker.by_ref().take(new_lines_no));
        };
        let new_lines_no = self.text_lines.len() - old_lines_no;
        if let Some(ref mut search) = self.search {
            use melib::text::search::KMP;
            for (y, l) in self.text_lines.iter().enumerate().skip(old_lines_no) {
                search.positions.extend(
                    l.content
                        .kmp_search(&search.pattern)
                        .into_iter()
                        .map(|offset| (y, offset)),
                );
            }
        }
        self.height += new_lines_no;
    }

    fn draw_page(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        if let Some((ref cmd, ref content)) = self.filtered_content {
            grid.copy_area(
                content.buffer(),
                area,
                content
                    .area()
                    .skip_cols(self.cursor.0)
                    .skip_rows(self.cursor.1)
                    .take_cols(content.terminal_size().0.min(area.width()))
                    .take_rows(content.terminal_size().1.min(area.height())),
            );
            context
                .replies
                .push_back(UIEvent::StatusEvent(StatusEvent::UpdateSubStatus(
                    cmd.to_string(),
                )));
            return;
        }

        {
            let mut area2 = area;

            // Scan for links once per text (not once per draw): the finder
            // walks the whole body, and `Link` owns its value so the cached
            // scan can outlive this call.
            if self.links.is_none() {
                self.links = Some(Self::scan_links(&self.text));
            }
            let links = self.links.as_ref().expect("filled just above");
            // Plain body text sits on the pane background when the pager
            // is hosted by a focus-aware pane: fg stays `colors.fg`, the
            // bg follows the pane fill.
            let text_colors = ThemeAttribute {
                bg: self
                    .pane_fill
                    .as_ref()
                    .map(|fill| fill.bg)
                    .unwrap_or(self.colors.bg),
                ..self.colors
            };
            let mut cur_link_idx = 0;
            for l in self
                .text_lines
                .iter()
                .skip(self.cursor.1)
                .take(area2.height())
            {
                if area2.is_empty() {
                    break;
                }
                // Perform a simple scan pass over `links`, by keeping current link index to
                // consider in `cur_link_idx`.
                //
                // There are 4 comparison cases for a link span and a line span:
                //
                // 1. Link starts after current line.
                //
                //    ```text
                //                                    Link.start.......Link.end
                //    Start.....................end
                //    ```
                //
                //    Stop loop.
                // 2. Link starts within current line.
                //
                //    ```text
                //            Link.start.......Link.end
                //    Start.....................end
                //    ```
                //
                //    Set link and if there are no other links in this line, break.
                // 3. Link starts before this line but ends within this line.
                //
                //    ```text
                //    Link.start.......Link.end
                //             Start.....................end
                //    ```
                //
                //    Same as 2.
                // 4. Link ends before this current line.
                //
                //    ```text
                //    Link.start.......Link.end
                //                       Start.....................end
                //    ```
                //
                //    Continue loop.
                // 5. Set link if link contains entire line and break.
                while let Some(link) = links.get(cur_link_idx) {
                    if link.start >= l.end {
                        // 1.
                        break;
                    } else if (l.start..l.end).contains(&link.start)
                        || (l.start..l.end).contains(&link.end)
                    {
                        // 2. or 3.
                        {
                            let skip_x = link.start.saturating_sub(l.start);
                            let start = area2.skip_cols(skip_x).upper_left();
                            let end = if link.end > l.end {
                                area2.skip_cols(skip_x + l.content.len()).upper_left()
                            } else {
                                let skip_x = skip_x + link.value.len();
                                area2.skip_cols(skip_x).upper_left()
                            };
                            let uri = grid.insert_uri(&link.value);
                            grid.set_uri(uri, start, end);
                        }
                        if link.end < l.end {
                            // In this case, there is more than one link in this line, so continue
                            // the scan.
                            cur_link_idx += 1;
                            continue;
                        }
                        break;
                    } else if l.start >= link.end {
                        // 4.
                        cur_link_idx += 1;
                        continue;
                    }
                    // 5.
                    if (link.start..link.end).contains(&l.start)
                        && (link.start..link.end).contains(&l.end)
                    {
                        let start = area2.upper_left();
                        let end = area2.skip_cols(l.content.len()).upper_left();
                        let uri = grid.insert_uri(&link.value);
                        grid.set_uri(uri, start, end);
                    }
                    break;
                }
                grid.write_string(
                    &l.content,
                    text_colors.fg,
                    text_colors.bg,
                    Attr::DEFAULT,
                    area2,
                    None,
                    None,
                );
                if l.content.starts_with('⤷') {
                    grid[area2.upper_left()]
                        .set_fg(crate::conf::value(context, "highlight").fg)
                        .set_attrs(crate::conf::value(context, "highlight").attrs);
                }
                area2 = area2.skip_rows(1);
            }

            if area2.height() <= 1 {
                grid.clear_area(area2, self.base_fill(context));
            }
        }

        {
            {
                let area3 = area;
                for text_formatter in
                    crate::conf::text_format_regexps(context, "pager.envelope.body")
                {
                    let t = grid.insert_tag(text_formatter.tag);
                    for (i, l) in self
                        .text_lines
                        .iter()
                        .skip(self.cursor.1)
                        .enumerate()
                        .take(area3.height() + 1)
                    {
                        let i = i + area3.upper_left().1;
                        for (start, end) in text_formatter.regexp.find_iter(&l.content) {
                            let start = start + area3.upper_left().0;
                            let end = end + area3.upper_left().0;
                            grid.set_tag(t, (start, i), (end, i));
                        }
                    }
                }
            }
            if let Some(ref mut search) = self.search {
                // Last row will be reserved for the "Results for ..." line.
                let area3 = area.skip_rows_from_end(1);
                let cursor_line = self.cursor.1;
                let results_attr = crate::conf::value(context, "pager.highlight_search");
                let results_current_attr =
                    crate::conf::value(context, "pager.highlight_search_current");
                search.cursor =
                    std::cmp::min(search.positions.len().saturating_sub(1), search.cursor);
                for (i, (y, offset)) in search
                    .positions
                    .iter()
                    .enumerate()
                    .filter(|(_, &(y, _))| y >= cursor_line && y < cursor_line + area3.height())
                {
                    let attr = if i == search.cursor {
                        results_current_attr
                    } else {
                        results_attr
                    };

                    let (y, x) = (*y, *offset);
                    let row_iter = grid.row_iter(
                        area3.nth_row(y - cursor_line),
                        x..x + search.pattern.grapheme_width(),
                        0,
                    );
                    debug_assert_eq!(row_iter.area().width(), search.pattern.grapheme_width());
                    for c in row_iter {
                        grid[c]
                            .set_fg(attr.fg)
                            .set_bg(attr.bg)
                            .set_attrs(attr.attrs);
                    }
                }
            }
        }
    }
}

impl Component for Pager {
    fn draw(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        #[cfg(debug_assertions)]
        let __draw_span = crate::state::DrawSpan::enter("Pager");
        if !self.is_dirty() {
            return;
        }

        if !self.initialised {
            self.initialise(grid, area, context);
        }

        self.dirty = false;

        if self.height == 0 || self.width == 0 {
            grid.clear_area(area, self.base_fill(context));
            return;
        }

        let (mut cols, mut rows) = (area.width(), area.height());
        let (has_more_lines, (width, height)) = if self.filtered_content.is_some() {
            (false, (self.width, self.height))
        } else {
            (
                !self.line_breaker.is_finished(),
                (self.line_breaker.width().unwrap_or(cols), self.height),
            )
        };
        if cols < 2 || rows < 2 {
            return;
        }

        if (self.show_scrollbar || self.reserve_scrollbar_column) && rows < height {
            cols -= 1;
            if self.show_scrollbar {
                rows -= 1;
            }
        } else if self.search.is_some() {
            rows -= 1;
        }

        if self.show_scrollbar && cols < width {
            rows -= 1;
        }

        if let Some(mvm) = self.movement.take() {
            match mvm {
                PageMovement::Up(amount) => {
                    self.cursor.1 = self.cursor.1.saturating_sub(amount);
                }
                PageMovement::PageUp(multiplier) => {
                    self.cursor.1 = self.cursor.1.saturating_sub(rows * multiplier);
                }
                PageMovement::Down(amount) => {
                    if self.cursor.1 + amount + 1 < self.height {
                        self.cursor.1 += amount;
                    } else {
                        self.cursor.1 = self.height.saturating_sub(1);
                    }
                    self.draw_lines_up_to(
                        grid,
                        area,
                        context,
                        self.cursor.1 + Self::PAGES_AHEAD_TO_RENDER_NO * rows,
                    );
                }
                PageMovement::PageDown(multiplier) => {
                    if self.cursor.1 + rows * multiplier + 1 < self.height {
                        self.cursor.1 += rows * multiplier;
                    } else if self.cursor.1 + rows * multiplier > self.height {
                        self.cursor.1 = self.height.saturating_sub(1);
                    } else {
                        self.cursor.1 = (self.height / rows) * rows;
                    }
                    self.draw_lines_up_to(
                        grid,
                        area,
                        context,
                        self.cursor.1 + Self::PAGES_AHEAD_TO_RENDER_NO * rows,
                    );
                }
                PageMovement::Right(amount) => {
                    if self.cursor.0 + amount + 1 < self.width {
                        self.cursor.0 += amount;
                    } else {
                        self.cursor.0 = self.width.saturating_sub(1);
                    }
                }
                PageMovement::Left(amount) => {
                    self.cursor.0 = self.cursor.0.saturating_sub(amount);
                }
                PageMovement::Home => {
                    self.cursor.1 = 0;
                }
                PageMovement::End => {
                    self.draw_lines_up_to(grid, area, context, 0);
                    self.cursor.1 = self.height.saturating_sub(1);
                }
            }
        }

        if let Some(ref mut search) = self.search {
            if !search.positions.is_empty() {
                if let Some(mvm) = search.movement.take() {
                    match mvm {
                        SearchMovement::First | SearchMovement::Last => {
                            self.cursor.1 = search.positions[search.cursor].0;
                        }
                        SearchMovement::Previous => {
                            if self.cursor.1 > search.positions[search.cursor].0 {
                                self.cursor.1 = search.positions[search.cursor].0;
                            }
                        }
                        SearchMovement::Next => {
                            if self.cursor.1 + rows < search.positions[search.cursor].0 {
                                self.cursor.1 = search.positions[search.cursor].0;
                            }
                        }
                    }
                }
            }
        }

        grid.clear_area(area, self.base_fill(context));

        self.cols_lt_width = cols + self.cursor.0 < width;
        self.rows_lt_height = rows + self.cursor.1 < height;

        self.cursor = (
            std::cmp::min(width.saturating_sub(cols), self.cursor.0),
            std::cmp::min(height.saturating_sub(rows), self.cursor.1),
        );
        self.draw_page(grid, area.take_cols(cols).take_rows(rows), context);
        if self.show_scrollbar && rows < height {
            draw_scrollbar(
                grid,
                area.nth_col(area.width().saturating_sub(1)),
                context,
                /* horizontal */ false,
                self.cursor.1,
                rows,
                height,
            );
        }
        if self.show_scrollbar && cols < width {
            draw_scrollbar(
                grid,
                area.nth_row(area.height().saturating_sub(1)),
                context,
                /* horizontal */ true,
                self.cursor.0,
                cols,
                width,
            );
        }
        if (rows < height) || self.search.is_some() {
            const RESULTS_STR: &str = "Results for ";
            let shown_lines = self.cursor.1 + rows;
            let total_lines = height;
            if rows < height {
                context
                    .replies
                    .push_back(UIEvent::StatusEvent(StatusEvent::ScrollUpdate(
                        ScrollUpdate::Update {
                            id: self.id,
                            context: ScrollContext {
                                shown_lines,
                                total_lines,
                                has_more_lines,
                            },
                        },
                    )));
            } else {
                context
                    .replies
                    .push_back(UIEvent::StatusEvent(StatusEvent::ScrollUpdate(
                        ScrollUpdate::End(self.id),
                    )));
            };
            if let Some(ref search) = self.search {
                let status_message = format!(
                    "{results_str}{search_pattern}: {current_pos}/{total_results}{has_more_lines}",
                    results_str = RESULTS_STR,
                    search_pattern = search.pattern,
                    current_pos = if search.positions.is_empty() {
                        0
                    } else {
                        search.cursor + 1
                    },
                    total_results = search.positions.len(),
                    has_more_lines = if !has_more_lines { "" } else { "(+)" }
                );
                let mut attribute = crate::conf::value(context, "status.bar");
                if !context.settings.terminal.use_color() {
                    attribute.attrs |= Attr::REVERSE;
                }
                grid.write_string(
                    &status_message,
                    attribute.fg,
                    attribute.bg,
                    attribute.attrs,
                    area.nth_row(area.height().saturating_sub(1)),
                    None,
                    None,
                );
                /* set search pattern to italics */
                let start_x = RESULTS_STR.len();
                let row_iter = grid.row_iter(
                    area.nth_row(area.height().saturating_sub(1)),
                    start_x..(start_x + search.pattern.grapheme_width()),
                    0,
                );
                debug_assert_eq!(row_iter.area().width(), search.pattern.grapheme_width());
                for c in row_iter {
                    grid[c].set_attrs(attribute.attrs | Attr::ITALICS);
                }
            }
        }
        context.dirty_areas.push_back(area);
    }

    fn process_event(&mut self, event: &mut UIEvent, context: &mut Context) -> bool {
        let shortcuts = self.shortcuts(context);
        match event {
            UIEvent::ConfigReload { old_settings: _ } => {
                self.set_colors(crate::conf::value(context, "theme_default"));
                self.set_dirty(true);
            }
            UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::PAGER]["scroll_up"])
                    && self.cursor.1 > 0 =>
            {
                self.movement = Some(PageMovement::Up(1));
                self.dirty = true;
                return true;
            }
            UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::PAGER]["scroll_down"])
                    && self.rows_lt_height =>
            {
                self.movement = Some(PageMovement::Down(1));
                self.dirty = true;
                return true;
            }
            UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_left"])
                    && self.cursor.0 > 0 =>
            {
                self.movement = Some(PageMovement::Left(1));
                self.dirty = true;
                return true;
            }
            UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::GENERAL]["scroll_right"])
                    && self.cols_lt_width =>
            {
                self.movement = Some(PageMovement::Right(1));
                self.dirty = true;
                return true;
            }
            UIEvent::Input(ref key) if shortcut!(key == shortcuts[Shortcuts::PAGER]["page_up"]) => {
                self.movement = Some(PageMovement::PageUp(1));
                self.dirty = true;
                return true;
            }
            UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::PAGER]["page_down"]) =>
            {
                self.movement = Some(PageMovement::PageDown(1));
                self.dirty = true;
                return true;
            }
            UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::GENERAL]["home_page"]) =>
            {
                self.movement = Some(PageMovement::Home);
                self.dirty = true;
                return true;
            }
            UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::GENERAL]["end_page"]) =>
            {
                self.movement = Some(PageMovement::End);
                self.dirty = true;
                return true;
            }
            UIEvent::ChangeMode(UIMode::Normal) => {
                self.dirty = true;
            }
            UIEvent::Action(View(Pipe(ref bin, ref args))) => {
                use std::{
                    io::Write,
                    process::{Command, Stdio},
                };
                let mut command_obj = match Command::new(bin)
                    .args(args.as_slice())
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .spawn()
                {
                    Ok(o) => o,
                    Err(err) => {
                        context.replies.push_back(UIEvent::Notification {
                            title: Some(format!("Could not pipe to {bin}").into()),
                            source: None,
                            body: err.to_string().into(),
                            kind: Some(NotificationType::Error(err.kind().into())),
                        });
                        return true;
                    }
                };
                let Some(stdin) = command_obj.stdin.as_mut() else {
                    context.replies.push_back(UIEvent::Notification {
                        title: Some(format!("Could not pipe to {bin}").into()),
                        source: None,
                        body: "the child process has no stdin pipe".into(),
                        kind: Some(NotificationType::Error(melib::error::ErrorKind::External)),
                    });
                    return true;
                };
                if let Err(err) = stdin.write_all(self.text.as_bytes()) {
                    // A filter command that exits without reading all of its
                    // stdin (e.g. `head -1`) closes the pipe: report the
                    // broken pipe instead of panicking.
                    context.replies.push_back(UIEvent::Notification {
                        title: Some(format!("Could not pipe to {bin}").into()),
                        source: None,
                        body: format!("could not write pager text to the child's stdin: {err}")
                            .into(),
                        kind: Some(NotificationType::Error(melib::error::ErrorKind::External)),
                    });
                    return true;
                }

                context.replies.push_back(UIEvent::Notification {
                    title: None,
                    source: None,
                    body: format!(
                        "Pager text piped to '{bin}{}{}'",
                        if args.is_empty() { "" } else { " " },
                        args.join(" ")
                    )
                    .into(),
                    kind: Some(NotificationType::Info),
                });
                return true;
            }
            ev if matches!(ev, UIEvent::Action(View(Filter(None))))
                || matches!(ev, UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::PAGER]["select_filter"])) =>
            {
                let filters = context
                    .settings
                    .pager
                    .named_filters
                    .iter()
                    .map(|(k, v)| (v.to_string(), k.to_string()))
                    .collect::<Vec<_>>();
                if filters.is_empty() {
                    context.replies.push_back(UIEvent::Notification {
                        title: None,
                        source: None,
                        body: "No filters set in [pager.named_filters].".into(),
                        kind: Some(NotificationType::Info),
                    });
                } else {
                    context.replies.push_back(UIEvent::GlobalUIDialog {
                        value: Box::new(UIDialog::new(
                            "select filter",
                            filters,
                            true,
                            Some(Box::new(move |_id: ComponentId, results: &[String]| {
                                Some(UIEvent::Action(View(Filter(Some(
                                    results.first().cloned()?,
                                )))))
                            })),
                            context,
                        )),
                        parent: Some(self.id()),
                    });
                }
                return true;
            }
            UIEvent::Action(View(Filter(Some(ref cmd)))) => {
                self.filter(cmd, context);
                self.initialised = false;
                self.dirty = true;
                return true;
            }
            UIEvent::StatusEvent(StatusEvent::JobFinished(ref job_id))
                if self
                    .filter_job
                    .as_ref()
                    .map(|(_, h)| h == job_id)
                    .unwrap_or(false) =>
            {
                let (cmd, mut handle) = self.filter_job.take().unwrap();
                match handle.chan.try_recv() {
                    Err(_) => { /* filter was canceled */ }
                    Ok(None) => { /* something happened, perhaps a worker thread panicked */ }
                    Ok(Some(Ok(buf))) => {
                        let (width, height) = buf.terminal_size();
                        self.width = width;
                        self.height = height;
                        self.filtered_content = Some((cmd, buf));
                        self.initialised = false;
                        self.set_dirty(true);
                    }
                    Ok(Some(Err(err))) => {
                        context.replies.push_back(UIEvent::Notification {
                            title: Some("Could not run filter".into()),
                            source: None,
                            body: err.to_string().into(),
                            kind: Some(NotificationType::Error(err.kind)),
                        });
                    }
                }
            }
            UIEvent::Action(Action::Listing(ListingAction::Search { term: pattern, .. })) => {
                self.search = Some(SearchPattern {
                    pattern: pattern.to_string(),
                    positions: vec![],
                    cursor: 0,
                    movement: Some(SearchMovement::First),
                });
                self.initialised = false;
                self.dirty = true;
                return true;
            }
            UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::GENERAL]["next_search_result"])
                    && self.search.is_some() =>
            {
                if let Some(ref mut search) = self.search {
                    search.movement = Some(SearchMovement::Next);
                    search.cursor += 1;
                    self.initialised = false;
                    self.dirty = true;
                    return true;
                }
            }
            UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::GENERAL]["previous_search_result"])
                    && self.search.is_some() =>
            {
                if let Some(ref mut search) = self.search {
                    search.movement = Some(SearchMovement::Previous);
                    search.cursor = search.cursor.saturating_sub(1);
                    self.initialised = false;
                    self.dirty = true;
                    return true;
                }
            }
            UIEvent::Input(Key::Esc) if self.search.is_some() => {
                self.search = None;
                self.initialised = false;
                self.dirty = true;
                return true;
            }
            UIEvent::Input(Key::Esc) if self.filtered_content.is_some() => {
                self.filtered_content = None;
                self.initialised = false;
                self.dirty = true;
                context
                    .replies
                    .push_back(UIEvent::StatusEvent(StatusEvent::UpdateSubStatus(
                        String::new(),
                    )));
                return true;
            }
            UIEvent::Resize => {
                self.initialised = false;
                self.set_dirty(true);
            }
            UIEvent::VisibilityChange(true) => {
                self.set_dirty(true);
            }
            UIEvent::VisibilityChange(false) => {
                context
                    .replies
                    .push_back(UIEvent::StatusEvent(StatusEvent::ScrollUpdate(
                        ScrollUpdate::End(self.id),
                    )));
                context
                    .replies
                    .push_back(UIEvent::StatusEvent(StatusEvent::UpdateSubStatus(
                        String::new(),
                    )));
            }
            UIEvent::Input(ref key)
                if context.settings.shortcuts.pager.commands.iter().any(|cmd| {
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
        false
    }

    fn is_dirty(&self) -> bool {
        self.dirty || !self.initialised
    }

    fn set_dirty(&mut self, value: bool) {
        self.dirty = value;
    }

    fn shortcuts(&self, context: &Context) -> ShortcutMaps {
        let mut ret: ShortcutMaps = Default::default();
        ret.insert(
            Shortcuts::PAGER,
            context.settings.shortcuts.pager.key_values(),
        );
        ret.insert(
            Shortcuts::GENERAL,
            context.settings.shortcuts.general.key_values(),
        );
        ret
    }

    fn id(&self) -> ComponentId {
        self.id
    }

    fn kill(&mut self, uuid: ComponentId, context: &mut Context) {
        if self.id != uuid {
            return;
        }

        context.replies.push_back(UIEvent::Action(Tab(Kill(uuid))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shared HOME for mock contexts. Environment variables are process-global,
    /// so parallel tests must not race each other by pointing them at tempdirs
    /// that get deleted while another test constructs its `Context` (which reads
    /// `MELI_CONFIG`/XDG vars). Pattern copied from `crate::mail::view::tests`.
    fn shared_test_home() -> &'static tempfile::TempDir {
        static HOME: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
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
        ctx.unwrap_or_else(|| Context::new_mock(shared_test_home()))
    }

    /// `from_string` spawns the configured filter command internally, and the
    /// envelope view historically called `filter()` again with the same command
    /// right after, spawning a second (duplicate) process and orphaning the
    /// first. `filter()` must not spawn a second job while one is already in
    /// flight for the same command string; a different command must still
    /// replace the in-flight job.
    #[test]
    fn test_pager_filter_same_command_does_not_respawn() {
        const FIRST_CMD: &str = "cat";
        const OTHER_CMD: &str = "tr a-z A-Z";

        let mut context = mock_context();
        context.settings.pager.filter = Some(FIRST_CMD.to_string());

        let mut pager = Pager::from_string(
            "hello\nworld\n".to_string(),
            &context,
            None,
            None,
            ThemeAttribute::default(),
        );
        let first_job_id = pager
            .filter_job
            .as_ref()
            .expect("from_string must have spawned the configured filter job")
            .1
            .job_id;

        // Duplicate call with the SAME command (what envelope view does today):
        // must be a no-op, keeping the first job id.
        pager.filter(FIRST_CMD, &context);
        assert_eq!(
            pager
                .filter_job
                .as_ref()
                .expect("filter job must still be in flight")
                .1
                .job_id,
            first_job_id,
            "filter() with the same command must not spawn a second job",
        );

        // A DIFFERENT command replaces the in-flight job.
        pager.filter(OTHER_CMD, &context);
        let (cmd, handle) = pager
            .filter_job
            .as_ref()
            .expect("different command must spawn a replacement job");
        assert_eq!(cmd, OTHER_CMD);
        assert_ne!(
            handle.job_id, first_job_id,
            "a different command must replace the in-flight job",
        );
    }

    /// Regression test for narrow-width CJK truncation: the pager must wrap
    /// its text at the *actual* area width, not at `pager.minimum_width`
    /// (default 80), otherwise in panes narrower than `minimum_width` every
    /// wrapped line is wider than the visible area, gets visually clipped by
    /// `CellBuffer::write_string`, and the clipped tail is unreachable
    /// (vertical scroll shows the same clipped rows; horizontal scroll is not
    /// applied at render time). A wide grapheme starting at the last column
    /// also spills one cell past the pane edge (the observed width+1).
    #[test]
    fn test_pager_narrow_width_cjk_integrity() {
        use melib::text::TextProcessing;

        let mut context = mock_context();
        let cjk_line = "横".repeat(72);
        let text = format!(
            "Date: Sat, 13 Sep 2026\nFrom: a@example.com\nSubject: S1\nMessage-ID: \
             <s1@example.com>\n\n{cjk_line}\n\n[-- #1 text/plain --]"
        );
        let mut pager = Pager::from_string(text, &context, None, None, ThemeAttribute::default());
        let mut screen =
            crate::terminal::Screen::<crate::terminal::Virtual>::new(Default::default());

        let mut draw_and_check =
            |screen: &mut crate::terminal::Screen<crate::terminal::Virtual>,
             pager: &mut Pager,
             pane_cols: usize,
             pane_rows: usize,
             phase: &str| {
                // Model the real resize flow: the terminal resize is
                // broadcast as `UIEvent::Resize`, which makes the pager
                // re-initialise (re-wrap from source) on the next draw.
                pager.process_event(&mut UIEvent::Resize, &mut context);
                assert!(screen.resize(pane_cols, pane_rows));
                let area = screen.area().skip_cols(13);
                pager.draw(screen.grid_mut(), area, &mut context);
                // Every wrapped line must fit the pager area without clipping.
                for (i, l) in pager.text_lines.iter().enumerate() {
                    assert!(
                        l.content.grapheme_width() <= area.width(),
                        "{phase}: pane {pane_cols}: text line {i} is {} columns wide but the \
                     pager area is only {} columns: {:?}",
                        l.content.grapheme_width(),
                        area.width(),
                        l.content
                    );
                }
                // And the whole body must actually be rendered: all 72 CJK chars
                // visible in the pager area.
                let grid = screen.grid();
                let visible = grid
                    .bounds_iter(area)
                    .flatten()
                    .filter(|&pos| grid[pos].ch() == '横')
                    .count();
                assert_eq!(
                    visible, 72,
                    "{phase}: pane {pane_cols}: only {visible}/72 CJK chars rendered in the \
                 pager area (content lost to clipping)"
                );
            };

        // Fresh open in a narrow pane.
        draw_and_check(&mut screen, &mut pager, 60, 20, "fresh open");
        // Wider: must re-wrap from source and keep all content.
        draw_and_check(&mut screen, &mut pager, 120, 36, "resize wider");
        // Narrower again: must re-wrap from source and keep all content.
        draw_and_check(&mut screen, &mut pager, 60, 20, "resize narrower");
        draw_and_check(&mut screen, &mut pager, 80, 24, "resize to 80");
        // Degenerate small panes must not panic (their wrap width is 0, which
        // keeps melib's width-0 early-return behaviour; content stays in
        // `Pager::text`).
        for pane_cols in [14, 17, 20] {
            pager.process_event(&mut UIEvent::Resize, &mut context);
            assert!(screen.resize(pane_cols, 10));
            let area = screen.area().skip_cols(13);
            pager.draw(screen.grid_mut(), area, &mut context);
        }
    }

    /// Every rendered line must fit the pager area, and greedy tightness
    /// must hold at the area width: appending the first grapheme of the next
    /// line to a line must overflow (no systematic one-word-early breaking).
    fn assert_pager_lines_fit_and_are_tight(pager: &Pager, area_width: usize, phase: &str) {
        use melib::text::TextProcessing;
        for (i, l) in pager.text_lines.iter().enumerate() {
            assert!(
                l.content.grapheme_width() <= area_width,
                "{phase}: text line {i} is {} columns wide but the pager area is only \
                 {area_width} columns: {:?}",
                l.content.grapheme_width(),
                l.content
            );
        }
        // Greedy tightness at the area width: appending the next break
        // *unit* of the source text (word / CJK char) to any line except the
        // last of a physical line must overflow. Whole-word moves are not
        // early breaks. Mid-word candidates (the iterator can emit spurious
        // `BreakAllowed` positions inside ASCII words) are ignored, exactly
        // like the wrapping code does.
        let word_splits_ascii = |offset: usize| {
            matches!(
                (
                    pager.text[..offset].chars().next_back(),
                    pager.text[offset..].chars().next(),
                ),
                (Some(p), Some(a)) if p.is_ascii_alphanumeric() && a.is_ascii_alphanumeric()
            )
        };
        let mut breaks: Vec<usize> = melib::text::LineBreakCandidateIter::new(&pager.text)
            .filter(|&(offset, _)| !word_splits_ascii(offset))
            .map(|(offset, _)| offset)
            .collect();
        // Soft points after '/' and word-internal '-' count as unit
        // boundaries too, mirroring the wrapping code.
        breaks.extend(pager.text.char_indices().filter_map(|(idx, ch)| {
            let after = idx + ch.len_utf8();
            match ch {
                '/' => Some(after),
                '-' if pager.text[..idx]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_alphanumeric)
                    && pager.text[after..]
                        .chars()
                        .next()
                        .is_some_and(char::is_alphanumeric) =>
                {
                    Some(after)
                }
                _ => None,
            }
        }));
        breaks.sort_unstable();
        breaks.dedup();
        let mut pos = 0usize;
        for pair in pager.text_lines.windows(2) {
            let (a, b) = (&pair[0].content, &pair[1].content);
            let _ = b;
            let a_content = a.strip_prefix('⤷').unwrap_or(a);
            let a_end = pos + a_content.len();
            pos = a_end;
            if !pager.text[a_end..].starts_with('\n') {
                let unit_end = breaks
                    .iter()
                    .find(|&&offset| offset > a_end)
                    .copied()
                    .unwrap_or(pager.text.len());
                let unit = &pager.text[a_end..unit_end];
                assert!(
                    a.grapheme_width() + unit.grapheme_width() > area_width,
                    "{phase}: line broke one unit early at {area_width} columns: {a:?} + {unit:?} \
                     would still fit"
                );
            }
        }
    }

    /// Mixed CJK+English: the trailing English word must move whole to the
    /// next line and every one of its characters must actually be rendered
    /// (the old overshooting break selection emitted lines wider than the
    /// pane, and `CellBuffer::write_string` clipped the overflowing tail of
    /// the trailing English word — reported as "the line end swallows 2
    /// English character widths").
    #[test]
    fn test_pager_mixed_cjk_english_no_loss_whole_word() {
        let mut context = mock_context();
        // 20 CJK chars = 40 columns, then a long English word.
        let cjk = "横".repeat(20);
        let text = format!("{cjk} englishwordtail");
        let mut pager = Pager::from_string(text, &context, None, None, ThemeAttribute::default());
        let mut screen =
            crate::terminal::Screen::<crate::terminal::Virtual>::new(Default::default());
        pager.process_event(&mut UIEvent::Resize, &mut context);
        assert!(screen.resize(60, 20));
        let area = screen.area().skip_cols(13);
        pager.draw(screen.grid_mut(), area, &mut context);

        assert_pager_lines_fit_and_are_tight(&pager, area.width(), "mixed");

        let grid = screen.grid();
        // All 20 CJK chars must be rendered in the pager area.
        let visible_cjk = grid
            .bounds_iter(area)
            .flatten()
            .filter(|&pos| grid[pos].ch() == '横')
            .count();
        assert_eq!(visible_cjk, 20, "mixed: CJK chars lost to clipping");

        // The whole English word must render intact on one row (never split
        // mid-word, never clipped).
        let mut word_row_found = false;
        for (_, y) in grid.bounds_iter(area).flatten() {
            let mut row = String::new();
            for x in 0..area.width() {
                row.push(grid[(area.upper_left().0 + x, y)].ch());
            }
            if row.contains("englishwordtail") {
                word_row_found = true;
                break;
            }
        }
        assert!(
            word_row_found,
            "mixed: the trailing English word is not rendered whole on one row \
             (split mid-word or clipped)"
        );
    }

    /// Pure English: lines must fit as many whole words as genuinely fit the
    /// area — no systematic one-word-early breaking with oversized EOL
    /// whitespace.
    #[test]
    fn test_pager_english_full_width_utilization() {
        let mut context = mock_context();
        let text = concat!(
            "alpha bravo charlie delta echo foxtrot golf hotel india juliet kilo lima ",
            "mike november oscar papa quebec romeo sierra tango uniform victor whiskey ",
            "xray yankee zulu one two three four five six seven eight nine ten"
        );
        let mut pager = Pager::from_string(
            text.to_string(),
            &context,
            None,
            None,
            ThemeAttribute::default(),
        );
        let mut screen =
            crate::terminal::Screen::<crate::terminal::Virtual>::new(Default::default());
        pager.process_event(&mut UIEvent::Resize, &mut context);
        assert!(screen.resize(120, 20));
        let area = screen.area().skip_cols(13);
        pager.draw(screen.grid_mut(), area, &mut context);

        assert!(pager.text_lines.len() >= 2, "text must wrap at this width");
        assert_pager_lines_fit_and_are_tight(&pager, area.width(), "english");
    }

    /// A URL longer than the pager area must break after a '/' soft point:
    /// every '/'-delimited segment fully rendered on one row, no `⤷` marker
    /// on the '/' continuation, no clipping, no loss.
    #[test]
    fn test_pager_url_soft_break_no_loss() {
        let mut context = mock_context();
        let text = "see https://example.com/a/very/long/path/with/segments end";
        let mut pager = Pager::from_string(
            text.to_string(),
            &context,
            None,
            None,
            ThemeAttribute::default(),
        );
        let mut screen =
            crate::terminal::Screen::<crate::terminal::Virtual>::new(Default::default());
        pager.process_event(&mut UIEvent::Resize, &mut context);
        assert!(screen.resize(60, 20));
        let area = screen.area().skip_cols(13);
        pager.draw(screen.grid_mut(), area, &mut context);

        assert_pager_lines_fit_and_are_tight(&pager, area.width(), "url");
        // Every URL segment must render whole on a single row of the grid.
        let grid = screen.grid();
        for segment in [
            "https:",
            "example.com",
            "very",
            "long",
            "path",
            "with",
            "segments",
        ] {
            let mut found_whole = false;
            for (_, y) in grid.bounds_iter(area).flatten() {
                let mut row = String::new();
                for x in 0..area.width() {
                    row.push(grid[(area.upper_left().0 + x, y)].ch());
                }
                if row.contains(segment) {
                    found_whole = true;
                    break;
                }
            }
            assert!(
                found_whole,
                "url: segment {segment:?} not rendered whole on one row (cut or lost)"
            );
        }
        // The '/' continuation is a plain wrap: no line after a '/'-ending
        // line starts with the `⤷` marker.
        for pair in pager.text_lines.windows(2) {
            if pair[0].content.ends_with('/') {
                assert!(
                    !pair[1].content.starts_with('⤷'),
                    "url: '/' continuation must be a plain wrap: {:?}",
                    pair[1].content
                );
            }
        }
    }

    /// Runs of more than two consecutive blank lines (empty or whitespace-
    /// only) are collapsed to exactly two in the pager reading view; runs of
    /// one or two blank lines are preserved verbatim.
    #[test]
    fn test_pager_blank_line_runs_collapsed() {
        let mut context = mock_context();
        let text = concat!(
            "para one\n\npara two\n\n\npara three\n\n\n\n\n",
            "para four\n   \n\t\n\n\npara five\n\n\n\n\n\npara six"
        );
        let mut pager = Pager::from_string(
            text.to_string(),
            &context,
            None,
            None,
            ThemeAttribute::default(),
        );
        let mut screen =
            crate::terminal::Screen::<crate::terminal::Virtual>::new(Default::default());
        pager.process_event(&mut UIEvent::Resize, &mut context);
        assert!(screen.resize(120, 20));
        let area = screen.area().skip_cols(13);
        pager.draw(screen.grid_mut(), area, &mut context);

        let contents: Vec<&str> = pager
            .text_lines
            .iter()
            .map(|l| l.content.as_str())
            .collect();
        // Expected: 1 blank kept; 2 kept; 3 -> 2; 5 -> 2; whitespace-only run
        // of 4 ("   ", "\t", "", "") -> first 2 kept; 5 -> 2.
        assert_eq!(
            contents,
            vec![
                "para one",
                "",
                "para two",
                "",
                "",
                "para three",
                "",
                "",
                "para four",
                "   ",
                "\t",
                "para five",
                "",
                "",
                "para six",
            ]
        );
    }

    /// The pager scrollbar (ratatui `Scrollbar` in the last column) must only
    /// appear when the text is taller than the viewport, and its thumb must
    /// track the scroll position: at the top of the track at cursor 0, lower
    /// after a page down, at the bottom after `End`.
    #[test]
    fn test_pager_scrollbar_tracks_scroll_position() {
        let mut context = mock_context();
        let text = (0..200)
            .map(|i| format!("line {i:03}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut pager = Pager::from_string(text, &context, None, None, ThemeAttribute::default());
        pager.set_show_scrollbar(true);
        let mut screen =
            crate::terminal::Screen::<crate::terminal::Virtual>::new(Default::default());
        pager.process_event(&mut UIEvent::Resize, &mut context);
        assert!(screen.resize(40, 10));
        let area = screen.area();

        // Glyphs of the last (gutter) column, top to bottom.
        fn gutter(
            screen: &crate::terminal::Screen<crate::terminal::Virtual>,
            area: Area,
        ) -> Vec<char> {
            let grid = screen.grid();
            grid.bounds_iter(area.nth_col(area.width() - 1))
                .flatten()
                .map(|(x, y)| grid[(x, y)].ch())
                .collect()
        }
        fn thumb_rows(gutter: &[char]) -> Vec<usize> {
            gutter
                .iter()
                .enumerate()
                .filter(|(_, c)| **c == '█')
                .map(|(i, _)| i)
                .collect()
        }

        pager.draw(screen.grid_mut(), area, &mut context);
        let top_gutter = gutter(&screen, area);
        assert_eq!(top_gutter.first(), Some(&'▲'), "top arrow missing");
        assert_eq!(top_gutter.last(), Some(&'▼'), "bottom arrow missing");
        let at_top = thumb_rows(&top_gutter);
        assert!(
            !at_top.is_empty() && at_top[0] <= 1,
            "thumb must start near the track top at cursor 0, got {at_top:?} in {top_gutter:?}"
        );

        pager.movement = Some(PageMovement::PageDown(10));
        pager.dirty = true;
        pager.draw(screen.grid_mut(), area, &mut context);
        let mid = thumb_rows(&gutter(&screen, area));
        assert!(
            !mid.is_empty() && mid[0] > at_top[0],
            "thumb must move down after paging, got {mid:?}"
        );

        pager.movement = Some(PageMovement::End);
        pager.dirty = true;
        pager.draw(screen.grid_mut(), area, &mut context);
        let end_gutter = gutter(&screen, area);
        let at_end = thumb_rows(&end_gutter);
        assert!(
            !at_end.is_empty() && at_end[at_end.len() - 1] >= end_gutter.len() - 2,
            "thumb must sit at the track bottom after End, got {at_end:?} in {end_gutter:?}"
        );

        // Short text that fits the viewport: no scrollbar glyphs; the gutter
        // column is ordinary (blank) content area.
        let mut short = Pager::from_string(
            "hi\nthere".to_string(),
            &context,
            None,
            None,
            ThemeAttribute::default(),
        );
        short.set_show_scrollbar(true);
        short.draw(screen.grid_mut(), area, &mut context);
        let short_gutter = gutter(&screen, area);
        assert!(
            !short_gutter.contains(&'█')
                && !short_gutter.contains(&'▲')
                && !short_gutter.contains(&'▼'),
            "no scrollbar must be drawn when content fits, got {short_gutter:?}"
        );
    }

    /// Horizontal scrolling follows the navigation key group: both the
    /// arrow keys and their vim counterparts (`h`/`l`) scroll the pager
    /// horizontally via the `general.scroll_left`/`scroll_right`
    /// defaults (`Left/h` and `Right/l`).
    #[test]
    fn test_pager_horizontal_scroll_keygroup() {
        use crate::terminal::{Screen, Virtual};

        let mut context = mock_context();
        let mut pager = Pager::from_string(
            "x".repeat(400),
            &context,
            None,
            None,
            ThemeAttribute::default(),
        );
        let theme_default = crate::conf::value(&context, "theme_default");
        let mut screen = Screen::<Virtual>::new(theme_default);
        assert!(screen.resize(80, 24));
        let area = screen.area();
        pager.draw(screen.grid_mut(), area, &mut context);
        // `initialise` wraps the text at the pane width, so drive the
        // content width wider than the pane directly (the state a
        // filtered/HTML pager ends up in) to exercise the horizontal
        // scrolling arms.
        pager.line_breaker.set_width(Some(400));
        pager.set_dirty(true);
        pager.draw(screen.grid_mut(), area, &mut context);
        assert!(
            pager.cols_lt_width,
            "precondition: content must overflow horizontally"
        );

        for key in [Key::Char('l'), Key::Right] {
            let mut event = UIEvent::Input(key.clone());
            assert!(
                pager.process_event(&mut event, &mut context),
                "{key:?} must scroll right"
            );
            pager.draw(screen.grid_mut(), area, &mut context);
            assert_eq!(pager.cursor.0, 1, "{key:?} must shift the cursor right");

            // Walk back with either scroll-left binding.
            let mut event = UIEvent::Input(Key::Char('h'));
            assert!(pager.process_event(&mut event, &mut context));
            pager.draw(screen.grid_mut(), area, &mut context);
            assert_eq!(pager.cursor.0, 0);
        }
        for key in [Key::Char('h'), Key::Left] {
            // Pre-scroll right so the left edge guard admits the key.
            let mut event = UIEvent::Input(Key::Char('l'));
            assert!(pager.process_event(&mut event, &mut context));
            pager.draw(screen.grid_mut(), area, &mut context);
            assert_eq!(pager.cursor.0, 1);

            let mut event = UIEvent::Input(key.clone());
            assert!(
                pager.process_event(&mut event, &mut context),
                "{key:?} must scroll left"
            );
            pager.draw(screen.grid_mut(), area, &mut context);
            assert_eq!(pager.cursor.0, 0, "{key:?} must shift the cursor back left");
        }
    }
}
