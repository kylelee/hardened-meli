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

//! Terminal grid cells, keys, colors, etc.
use std::io::{BufWriter, Write};
use std::os::fd::AsFd;
use std::time::{Duration, Instant};

use melib::{log, uuid};

use crossterm::{
    cursor::{Hide, MoveTo, MoveToColumn, Show},
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    queue,
    style::{
        Attribute, Color as CrosstermColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
    },
    terminal::{
        self, Clear, ClearType, DisableLineWrap, EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use nix::{
    poll::{poll, PollFd, PollFlags, PollTimeout},
    unistd::read,
};

use crate::{
    terminal::{
        cells::CellBuffer, Alignment, Cell, Color, DisableAlternateScrollMode,
        EnableAlternateScrollMode, EscapeSequenceQuery, Pos, QueryBackground, QueryForeground,
        RestoreWindowTitleIconFromStack, RestoreWraparoundMode, SaveWindowTitleIconToStack,
        SaveWraparoundMode,
    },
    Attr, Context, ThemeAttribute,
};

pub type StateStdout = BufWriter<Box<dyn Write + 'static>>;

type DrawHorizontalSegmentFn =
    fn(&mut CellBuffer, &mut StateStdout, std::ops::Range<usize>, usize) -> ();

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct ScreenGeneration((u64, u64));

impl ScreenGeneration {
    pub const NIL: Self = Self((0, 0));

    #[inline]
    #[must_use]
    pub fn next(self) -> Self {
        Self(uuid::Uuid::new_v4().as_u64_pair())
    }
}

impl Default for ScreenGeneration {
    fn default() -> Self {
        Self::NIL
    }
}

impl std::fmt::Display for ScreenGeneration {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        let pair = self.0;
        write!(
            fmt,
            "{}",
            uuid::Uuid::from_u64_pair(pair.0, pair.1).as_simple()
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Virtual;

pub struct Tty {
    stdout: Option<StateStdout>,
    background_query: Option<Color>,
    mouse: bool,
    draw_horizontal_segment_fn: DrawHorizontalSegmentFn,
}

impl Tty {
    #[inline]
    pub const fn mouse(&self) -> bool {
        self.mouse
    }

    pub fn set_mouse(&mut self, mouse: bool) -> &mut Self {
        self.mouse = mouse;
        let Some(stdout) = self.stdout.as_mut() else {
            return self;
        };
        if mouse {
            queue!(stdout, EnableMouseCapture).expect("Could not write to stdout");
        } else {
            queue!(stdout, DisableMouseCapture).expect("Could not write to stdout");
        }
        write!(
            stdout,
            "{}",
            if mouse {
                EnableAlternateScrollMode.as_ref()
            } else {
                DisableAlternateScrollMode.as_ref()
            }
        )
        .expect("Could not write to stdout");
        _ = stdout.flush();

        self
    }

    #[inline]
    pub fn set_draw_fn(
        &mut self,
        draw_horizontal_segment_fn: DrawHorizontalSegmentFn,
    ) -> &mut Self {
        self.draw_horizontal_segment_fn = draw_horizontal_segment_fn;
        self
    }
}

mod private {
    pub trait Sealed {}
}
impl private::Sealed for Virtual {}
impl private::Sealed for Tty {}

pub struct Screen<Display: private::Sealed> {
    cols: usize,
    rows: usize,
    grid: CellBuffer,
    overlay_grid: CellBuffer,
    display: Display,
    generation: ScreenGeneration,
}

impl<D: private::Sealed> std::fmt::Debug for Screen<D> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct(stringify!(Screen))
            .field("cols", &self.cols)
            .field("rows", &self.rows)
            .field("grid", &self.grid)
            .field("overlay_grid", &self.overlay_grid)
            .field("generation", &self.generation)
            .finish()
    }
}

impl<D: private::Sealed> Screen<D> {
    #[inline]
    pub fn init(display: D, theme_default: ThemeAttribute) -> Self {
        let area = Area {
            offset: (0, 0),
            upper_left: (0, 0),
            bottom_right: (0, 0),
            empty: true,
            canvas_cols: 0,
            canvas_rows: 0,
            generation: ScreenGeneration::NIL,
        };
        let mut retval = Self {
            cols: 0,
            rows: 0,
            grid: CellBuffer::nil(area),
            overlay_grid: CellBuffer::nil(area),
            display,
            generation: ScreenGeneration::NIL,
        };
        retval.grid_mut().default_cell = theme_default.into();
        retval.overlay_grid_mut().default_cell = theme_default.into();
        retval
    }

    #[inline]
    pub fn with_cols_and_rows(mut self, cols: usize, rows: usize) -> Self {
        self.generation = self.generation.next();
        self.cols = cols;
        self.rows = rows;
        Self {
            cols,
            rows,
            grid: CellBuffer::new(Cell::with_char(' '), self.area()),
            overlay_grid: CellBuffer::new(Cell::with_char(' '), self.area()),
            ..self
        }
    }

    pub const fn area(&self) -> Area {
        let upper_left = (0, 0);
        let bottom_right = (self.cols.saturating_sub(1), self.rows.saturating_sub(1));
        Area {
            offset: upper_left,
            upper_left,
            bottom_right,
            empty: matches!((self.cols, self.rows), (0, 0)),
            canvas_cols: self.cols,
            canvas_rows: self.rows,
            generation: self.generation,
        }
    }

    #[inline]
    pub fn grid(&self) -> &CellBuffer {
        &self.grid
    }

    #[inline]
    pub fn grid_mut(&mut self) -> &mut CellBuffer {
        &mut self.grid
    }

    #[inline]
    pub fn overlay_grid(&self) -> &CellBuffer {
        &self.overlay_grid
    }

    #[inline]
    pub fn overlay_grid_mut(&mut self) -> &mut CellBuffer {
        &mut self.overlay_grid
    }

    #[inline]
    pub fn grid_and_overlay_grid_mut(&mut self) -> (&mut CellBuffer, &mut CellBuffer) {
        (&mut self.grid, &mut self.overlay_grid)
    }

    #[inline(always)]
    pub const fn generation(&self) -> ScreenGeneration {
        self.generation
    }

    #[inline]
    pub const fn cols(&self) -> usize {
        self.cols
    }

    #[inline]
    pub const fn rows(&self) -> usize {
        self.rows
    }
}

impl Clone for Screen<Virtual> {
    fn clone(&self) -> Self {
        Self {
            grid: self.grid.clone(),
            overlay_grid: self.overlay_grid.clone(),
            ..*self
        }
    }
}

/// Translate a meli [`Color`] to its crossterm equivalent for the flush
/// layer.
///
/// Named colors and `Color::Byte(_)` go through the 256-color indexed forms
/// (`38;5;n` / `48;5;n`), `Color::Default` becomes `Reset` (`39`/`49`) and
/// `Color::Rgb` keeps the direct-color form, byte-identical to the
/// pre-migration writers.
#[inline]
fn crossterm_color(color: Color) -> CrosstermColor {
    match color {
        Color::Default => CrosstermColor::Reset,
        Color::Rgb(r, g, b) => CrosstermColor::Rgb { r, g, b },
        color => CrosstermColor::AnsiValue(color.as_byte().unwrap_or_default()),
    }
}

/// Emit the SGR attribute transitions needed to move the terminal from
/// `prev` to `next`.
///
/// Byte-compatible with the previous hand-written escape emitter: every
/// transition is queued as a crossterm [`SetAttribute`] command in the same
/// fixed attribute order (`BOLD`, `DIM`, `ITALICS`, `UNDERLINE`, `UNDERCURL`,
/// `BLINK`, `REVERSE`, `HIDDEN`), except the `UNDERCURL` reset which has no
/// crossterm equivalent (`Attribute` cannot express `CSI 4:0 m`) and keeps
/// its explicit escape.
///
/// `Attr::FORCE_TEXT` is intentionally not handled here; it is not an SGR
/// attribute and is rendered as a `U+FE0E` suffix after the cell symbol by
/// the callers.
fn write_attr_delta(next: Attr, prev: Attr, stdout: &mut StateStdout) {
    macro_rules! transition {
        ($bit:expr, $on:expr, $off:expr) => {
            match (next.intersects($bit), prev.intersects($bit)) {
                (true, true) | (false, false) => {}
                (false, true) => queue!(stdout, SetAttribute($off)).unwrap(),
                (true, false) => queue!(stdout, SetAttribute($on)).unwrap(),
            }
        };
    }
    transition!(Attr::BOLD, Attribute::Bold, Attribute::NormalIntensity);
    transition!(Attr::DIM, Attribute::Dim, Attribute::NormalIntensity);
    transition!(Attr::ITALICS, Attribute::Italic, Attribute::NoItalic);
    transition!(
        Attr::UNDERLINE,
        Attribute::Underlined,
        Attribute::NoUnderline
    );
    match (
        next.intersects(Attr::UNDERCURL),
        prev.intersects(Attr::UNDERCURL),
    ) {
        (true, true) | (false, false) => {}
        // No crossterm `Attribute` maps to SGR `4:0` (underline style none).
        (false, true) => write!(stdout, "\x1B[4:0m").unwrap(),
        (true, false) => queue!(stdout, SetAttribute(Attribute::Undercurled)).unwrap(),
    }
    transition!(Attr::BLINK, Attribute::SlowBlink, Attribute::NoBlink);
    transition!(Attr::REVERSE, Attribute::Reverse, Attribute::NoReverse);
    transition!(Attr::HIDDEN, Attribute::Hidden, Attribute::NoHidden);
}

/// What a horizontal segment emitter must do before writing the next cell to
/// keep it on its grid column. See
/// [`Screen::segment_needs_reanchor`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Reanchor {
    /// The natural cursor advance already matches the grid; write the cell as
    /// is.
    None,
    /// Pin the cursor to the grid column with `MoveToColumn(x)`; the walk
    /// back crossed no `empty` cell, so there is no skipped column to cover.
    PinOnly,
    /// Cover the skipped continuation column with a space, then pin the
    /// cursor with `MoveToColumn(x)`.
    PinAndCover,
}

impl Screen<Tty> {
    #[inline]
    pub fn new(theme_default: ThemeAttribute) -> Self {
        Self::init(
            Tty {
                stdout: None,
                mouse: false,
                background_query: None,
                draw_horizontal_segment_fn: Self::draw_horizontal_segment,
            },
            theme_default,
        )
    }

    #[inline]
    pub fn tty(&self) -> &Tty {
        &self.display
    }

    #[inline]
    pub fn tty_mut(&mut self) -> &mut Tty {
        &mut self.display
    }

    #[inline]
    pub fn draw(&mut self, xs: std::ops::Range<usize>, y: usize) {
        let Some(stdout) = self.display.stdout.as_mut() else {
            return;
        };
        (self.display.draw_horizontal_segment_fn)(&mut self.grid, stdout, xs, y);
    }

    #[inline]
    pub fn draw_overlay(&mut self, xs: std::ops::Range<usize>, y: usize) {
        let Some(stdout) = self.display.stdout.as_mut() else {
            return;
        };
        (self.display.draw_horizontal_segment_fn)(&mut self.overlay_grid, stdout, xs, y);
    }

    /// On `SIGWNICH` the `State` redraws itself according to the new
    /// terminal size.
    pub fn update_size(&mut self) {
        let termsize = terminal::size().ok();
        let termcols = termsize.map(|(w, _)| w);
        let termrows = termsize.map(|(_, h)| h);
        if termcols.unwrap_or(72) as usize != self.cols
            || termrows.unwrap_or(120) as usize != self.rows
        {
            log::trace!(
                "Size updated, from ({}, {}) -> ({:?}, {:?})",
                self.cols,
                self.rows,
                termcols,
                termrows
            );
        }
        let cols = termcols.unwrap_or(72) as usize;
        let rows = termrows.unwrap_or(120) as usize;
        if self.grid.resize(cols, rows, None) && self.overlay_grid.resize(cols, rows, None) {
            self.generation = self.generation.next();
            self.cols = cols;
            self.rows = rows;
            self.grid.area = self.area();
            self.overlay_grid.area = self.area();
        } else {
            log::warn!("Terminal size too big: ({} cols, {} rows)", cols, rows);
        }
    }

    /// Switch back to the terminal's main screen (The command line the user
    /// sees before opening the application)
    pub fn switch_to_main_screen(&mut self) {
        let Some(stdout) = self.display.stdout.as_mut() else {
            return;
        };
        let mouse = self.display.mouse;
        queue!(stdout, LeaveAlternateScreen, Show, DisableBracketedPaste,)
            .expect("Could not write to stdout");
        write!(
            stdout,
            "{restore_title}{restore_wraparound}",
            restore_title = RestoreWindowTitleIconFromStack,
            restore_wraparound = RestoreWraparoundMode,
        )
        .unwrap();
        if mouse {
            queue!(stdout, DisableMouseCapture).expect("Could not write to stdout");
            write!(stdout, "{}", DisableAlternateScrollMode).expect("Could not write to stdout");
        }
        self.flush();
        if let Err(err) = terminal::disable_raw_mode() {
            log::warn!("Error while disabling raw mode: {err}");
        }
        self.display.stdout = None;
    }

    pub fn switch_to_alternate_screen(&mut self, context: &crate::Context) {
        terminal::enable_raw_mode().expect("Could not enable raw mode");
        let mut stdout = BufWriter::with_capacity(
            240 * 80,
            Box::new(std::io::stdout()) as Box<dyn std::io::Write>,
        );

        write!(stdout, "{}", SaveWindowTitleIconToStack).unwrap();
        queue!(
            stdout,
            EnterAlternateScreen,
            Hide,
            Clear(ClearType::All),
            MoveTo(0, 0),
            EnableBracketedPaste
        )
        .unwrap();
        write!(
            stdout,
            "{save_wraparound}{window_title}",
            save_wraparound = SaveWraparoundMode,
            window_title = if let Some(ref title) = context.settings.terminal.window_title {
                format!("\x1b]2;{title}\x07")
            } else {
                String::new()
            }
        )
        .unwrap();
        queue!(stdout, DisableLineWrap).unwrap();
        if self.display.mouse {
            queue!(stdout, EnableMouseCapture).unwrap();
            write!(stdout, "{}", EnableAlternateScrollMode).unwrap();
        }

        self.display.stdout = Some(stdout);
        self.flush();
    }

    #[inline]
    pub fn flush(&mut self) {
        if let Some(stdout) = self.display.stdout.as_mut() {
            stdout.flush().unwrap();
        }
    }

    /// Classify how a horizontal segment emitter must fix the cursor before
    /// writing the cell at `(x, y)` so that it lands on its grid column.
    ///
    /// The emitters write cells consecutively and rely on the terminal cursor
    /// advancing naturally. Any multi-byte glyph can break that assumption,
    /// because the grid lays text out with `width_cjk` (East-Asian Ambiguous
    /// characters take two columns, with an `empty` continuation cell) while
    /// each terminal renders them as one OR two columns:
    ///
    /// - An Ambiguous character (`’`, `·`, `→` …) laid out two columns
    ///   wide: a terminal rendering it narrow advances the cursor one
    ///   column less than the grid expects, shifting every later glyph of
    ///   the row - including dialog borders - one column left, which
    ///   reads as underlying text inserted into the dialog.
    /// - A wide character whose continuation cell an overlay painted over
    ///   (dialog border over a CJK listing row): the wide char advances
    ///   the cursor by two, so the border glyph lands one column right.
    ///
    /// An explicit `MoveToColumn(x)` before each cell that follows a
    /// multi-byte glyph pins it to its grid column under either rendering.
    ///
    /// The cell at `xs_start` is never re-anchored, so a segment whose first
    /// cell lands on a continuation cell gets no pin. `empty` cells never
    /// trigger a re-anchor either, and they are the only cells the walk back
    /// skips. The walk returns [`Reanchor::PinAndCover`] only when it actually
    /// crossed at least one of them (`px < x - 1`): the terminal cursor may
    /// still sit on that skipped column, so it is covered with a space before
    /// the jump. When the multi-byte glyph sits immediately before `x`
    /// (`px == x - 1`) there is no skipped column, so only
    /// [`Reanchor::PinOnly`] is needed - a space written there would land one
    /// column past `x` and, when `x` is the segment's last cell, bleed into
    /// the neighbouring area.
    fn segment_needs_reanchor(grid: &CellBuffer, x: usize, y: usize, xs_start: usize) -> Reanchor {
        if x <= xs_start || grid[(x, y)].empty() {
            return Reanchor::None;
        }
        // The emitter never writes `empty` cells, so the terminal cursor
        // does not advance over them either; look past continuation cells
        // to the glyph that last moved the cursor. Multi-byte glyphs are
        // re-anchored unconditionally: East-Asian Ambiguous characters are
        // laid out two columns wide in the grid (`width_cjk` in
        // `CellBuffer::write_string`) but terminals render them as either
        // one or two columns, and either choice must leave every following
        // glyph on its grid column.
        let mut px = x - 1;
        while px > xs_start && grid[(px, y)].empty() {
            px -= 1;
        }
        if grid[(px, y)].empty() || grid[(px, y)].ch().len_utf8() <= 1 {
            return Reanchor::None;
        }
        if px < x - 1 {
            Reanchor::PinAndCover
        } else {
            Reanchor::PinOnly
        }
    }

    /// Draw only a specific `area` on the screen.
    pub fn draw_horizontal_segment(
        grid: &mut CellBuffer,
        stdout: &mut StateStdout,
        xs: std::ops::Range<usize>,
        y: usize,
    ) {
        queue!(stdout, MoveTo(xs.start as u16, y as u16)).unwrap();
        let mut current_fg = Color::Default;
        let mut current_bg = Color::Default;
        let mut current_attrs = Attr::DEFAULT;
        let mut current_uri = None;
        let draw_hyperlinks = grid.draw_hyperlinks;
        let xs_start = xs.start;
        write!(stdout, "\x1B[m").unwrap();
        for x in xs {
            let c = &grid[(x, y)];
            // A multi-byte glyph before this cell may have advanced the
            // terminal cursor differently than the grid expects; see
            // [`Screen::segment_needs_reanchor`].
            match Self::segment_needs_reanchor(grid, x, y, xs_start) {
                Reanchor::None => {}
                Reanchor::PinOnly => {
                    // The row never changes within a segment write, so a
                    // column-only jump is enough - and shorter than a full
                    // MoveTo on CJK-dense rows where this fires per glyph.
                    queue!(stdout, MoveToColumn(x as u16)).unwrap();
                }
                Reanchor::PinAndCover => {
                    // Cover the column the terminal cursor may still sit on
                    // (an Ambiguous glyph's continuation on narrow-rendering
                    // terminals) before jumping: on wide-rendering terminals
                    // the space lands under the next glyph and is immediately
                    // overwritten.
                    write!(stdout, " ").unwrap();
                    queue!(stdout, MoveToColumn(x as u16)).unwrap();
                }
            }
            if draw_hyperlinks {
                if let Some((uri, end)) = grid.hyperlinks_associations.get(&(x, y)) {
                    if current_uri.take().is_some() {
                        crate::terminal::Hyperlink::<str, str, str>::write_end(stdout).unwrap();
                    }
                    crate::terminal::Hyperlink::with_id(uri, "", &*grid.hyperlinks_table[uri])
                        .write_start(stdout)
                        .unwrap();
                    current_uri = Some(end);
                } else if current_uri == Some(&(x, y)) {
                    current_uri = None;
                    crate::terminal::Hyperlink::<str, str, str>::write_end(stdout).unwrap();
                }
            }
            if c.attrs() != current_attrs {
                write_attr_delta(c.attrs(), current_attrs, stdout);
                current_attrs = c.attrs();
            }
            if c.bg() != current_bg {
                queue!(stdout, SetBackgroundColor(crossterm_color(c.bg()))).unwrap();
                current_bg = c.bg();
            }
            if c.fg() != current_fg {
                queue!(stdout, SetForegroundColor(crossterm_color(c.fg()))).unwrap();
                current_fg = c.fg();
            }
            if !c.empty() {
                write!(stdout, "{}", c.ch()).unwrap();
                if c.attrs().intersects(Attr::FORCE_TEXT) {
                    _ = write!(stdout, "\u{FE0E}");
                }
                if c.attrs().intersects(Attr::FORCE_EMOJI) {
                    _ = write!(stdout, "\u{FE0F}");
                }
            }
        }
        if current_uri.take().is_some() {
            crate::terminal::Hyperlink::<str, str, str>::write_end(stdout).unwrap();
        }
    }

    pub fn draw_horizontal_segment_no_color(
        grid: &mut CellBuffer,
        stdout: &mut StateStdout,
        xs: std::ops::Range<usize>,
        y: usize,
    ) {
        queue!(stdout, MoveTo(xs.start as u16, y as u16)).unwrap();
        let mut current_attrs = Attr::DEFAULT;
        let xs_start = xs.start;
        write!(stdout, "\x1B[m").unwrap();
        for x in xs {
            let c = &grid[(x, y)];
            match Self::segment_needs_reanchor(grid, x, y, xs_start) {
                Reanchor::None => {}
                Reanchor::PinOnly => {
                    queue!(stdout, MoveToColumn(x as u16)).unwrap();
                }
                Reanchor::PinAndCover => {
                    write!(stdout, " ").unwrap();
                    queue!(stdout, MoveToColumn(x as u16)).unwrap();
                }
            }
            if c.attrs() != current_attrs {
                write_attr_delta(c.attrs(), current_attrs, stdout);
                current_attrs = c.attrs();
            }
            if !c.empty() {
                write!(stdout, "{}", c.ch()).unwrap();
                if c.attrs().intersects(Attr::FORCE_TEXT) {
                    _ = write!(stdout, "\u{FE0E}");
                }
                if c.attrs().intersects(Attr::FORCE_EMOJI) {
                    _ = write!(stdout, "\u{FE0F}");
                }
            }
        }
    }

    pub const fn background_query(&self) -> Option<Color> {
        self.display.background_query
    }

    /// Write the startup terminal queries to the tty and synchronously read
    /// the replies: default background color (OSC 11) and default foreground
    /// color (OSC 10).
    ///
    /// The replies are raw-read from stdin with a bounded total timeout of
    /// [`PALETTE_QUERY_TIMEOUT`], parsed with the same logic the input thread
    /// uses, and any leftover bytes are drained afterwards.
    ///
    /// This must run *before* the input thread is spawned (see
    /// `State::new`): it consumes stdin in-band responses that would
    /// otherwise be observed by the input parser as keystrokes. On silent or
    /// slow terminals the read times out, so startup is never blocked for
    /// longer than the query budget plus a bounded drain pass.
    pub fn do_background_query(&mut self) {
        let Some(stdout) = self.display.stdout.as_mut() else {
            return;
        };
        write_startup_queries(stdout);
        _ = stdout.flush();

        let mut palette = (None, None);
        query_terminal_palette(std::io::stdin(), &mut palette, PALETTE_QUERY_TIMEOUT);
        if let (Some(fg), Some(bg)) = palette {
            log::trace!(
                "compute_scheme_contrast(fg {fg:?}, bg {bg:?}) = {:?}",
                Color::compute_scheme_contrast(fg, bg)
            );
        }
        log::debug!(
            "Startup terminal palette query resolved: foreground = {:?}, background = {:?}",
            palette.0,
            palette.1
        );
        self.display.background_query = palette.1;
        drain_stdin();
    }
}

/// Write the startup terminal queries to `out`: first the default background
/// color (OSC 11) query, then the default foreground color (OSC 10) query.
fn write_startup_queries(out: &mut impl Write) {
    write!(out, "{}", QueryBackground.as_ref()).expect("Could not write to stdout");
    write!(out, "{}", QueryForeground.as_ref()).expect("Could not write to stdout");
}

/// Total time budget for the synchronous startup terminal palette queries.
const PALETTE_QUERY_TIMEOUT: Duration = Duration::from_millis(200);
/// Maximum size of a single reply before it is treated as garbage: replies
/// are a few dozen bytes.
const MAX_REPLY_LEN: usize = 1024;
/// Upper bound for the post-query stdin drain pass.
const DRAIN_TIMEOUT: Duration = Duration::from_millis(50);

/// Raw-read replies to the startup palette queries from `fd` for at most
/// `timeout` in total, and parse them into `palette`.
///
/// Returns as soon as both the foreground and background replies have been
/// parsed, on EOF/error, or when the timeout expires.
fn query_terminal_palette(
    fd: impl AsFd,
    palette: &mut (Option<Color>, Option<Color>),
    timeout: Duration,
) {
    let deadline = Instant::now() + timeout;
    let mut buf: Vec<u8> = Vec::with_capacity(3 * 32);
    let mut chunk = [0u8; 512];
    while palette.0.is_none() || palette.1.is_none() {
        let Some(poll_timeout) = deadline
            .checked_duration_since(Instant::now())
            .and_then(|remaining| PollTimeout::try_from(remaining).ok())
        else {
            break;
        };
        let mut fds = [PollFd::new(fd.as_fd(), PollFlags::POLLIN)];
        match poll(&mut fds, poll_timeout) {
            Ok(0) => break, // timed out
            Ok(_)
                if fds[0]
                    .revents()
                    .is_some_and(|revents| revents.contains(PollFlags::POLLIN)) =>
            {
                match read(fd.as_fd(), &mut chunk) {
                    Ok(0) | Err(_) => break, // EOF or error
                    Ok(n) => {
                        buf.extend_from_slice(&chunk[..n]);
                        parse_palette_replies(&mut buf, palette);
                    }
                }
            }
            _ => break, // poll error or unexpected revents
        }
    }
}

/// Scan `buf` for complete replies to the startup palette queries and update
/// `palette` with the parsed values, logging each parsed reply.
///
/// Complete replies are removed from `buf`; a trailing partial reply is kept
/// for the next read, and bytes that cannot belong to a reply are discarded.
fn parse_palette_replies(buf: &mut Vec<u8>, palette: &mut (Option<Color>, Option<Color>)) {
    loop {
        // Replies are escape sequences; drop anything before the first ESC.
        let Some(esc) = buf.iter().position(|&b| b == 0x1b) else {
            buf.clear();
            return;
        };
        buf.drain(..esc);
        if buf.len() < 2 {
            return; // might be a partial reply
        }
        match buf[1] {
            b']' => {
                let Some((idx, term_len)) = find_osc_terminator(buf) else {
                    if buf.len() > MAX_REPLY_LEN {
                        // Malformed, unterminated reply: drop the ESC byte and rescan.
                        buf.remove(0);
                    } else {
                        return; // partial reply, wait for more bytes
                    }
                    continue;
                };
                let reply = String::from_utf8_lossy(&buf[..idx]).into_owned();
                buf.drain(..idx + term_len);
                if let Some(bg) = QueryBackground::parse(&reply) {
                    log::trace!("EscapeSequence parsed bg {bg:?}");
                    palette.1 = Some(bg);
                } else if let Some(fg) = QueryForeground::parse(&reply) {
                    log::trace!("EscapeSequence parsed fg {fg:?}");
                    palette.0 = Some(fg);
                } else {
                    log::trace!("EscapeSequence unknown");
                }
            }
            b'[' => {
                if buf.len() < 3 {
                    return; // might be a partial reply
                }
                let Some(idx) = find_csi_final_byte(buf) else {
                    if buf.len() > MAX_REPLY_LEN {
                        buf.remove(0);
                    } else {
                        return; // partial reply, wait for more bytes
                    }
                    continue;
                };
                // Unknown CSI reply (no palette consumer): consume the whole
                // sequence so the scan can proceed to any palette replies
                // that follow it within the bounded read window.
                buf.drain(..idx + 1);
            }
            _ => {
                // Not an OSC/CSI sequence; skip the stray ESC byte and rescan.
                buf.remove(0);
            }
        }
    }
}

/// Find the terminator (BEL `\x07` or string terminator `ESC \\`) of an OSC
/// sequence in `buf` starting after the `ESC ]` prefix. Returns the index of
/// the first terminator byte and its length.
fn find_osc_terminator(buf: &[u8]) -> Option<(usize, usize)> {
    let mut i = 2;
    while i < buf.len() {
        match buf[i] {
            0x07 => return Some((i, 1)),
            0x1b if i + 1 == buf.len() => return None, // need one more byte to decide
            0x1b if buf.get(i + 1) == Some(&b'\\') => return Some((i, 2)),
            _ => {}
        }
        i += 1;
    }
    None
}

/// Find the final byte of a CSI sequence in `buf` (the first byte in
/// `0x40..=0x7E` after the `ESC [` prefix).
fn find_csi_final_byte(buf: &[u8]) -> Option<usize> {
    buf[2..]
        .iter()
        .position(|b| (0x40..=0x7E).contains(b))
        .map(|i| i + 2)
}

/// Non-blockingly read and discard any bytes still pending on stdin, so that
/// no leftover terminal-reply bytes are observed by the input thread as user
/// input.
fn drain_stdin() {
    let stdin = std::io::stdin();
    let deadline = Instant::now() + DRAIN_TIMEOUT;
    let mut chunk = [0u8; 4096];
    while Instant::now() < deadline {
        let mut fds = [PollFd::new(stdin.as_fd(), PollFlags::POLLIN)];
        match poll(&mut fds, PollTimeout::ZERO) {
            Ok(0) => return,
            Ok(_)
                if fds[0]
                    .revents()
                    .is_some_and(|revents| revents.contains(PollFlags::POLLIN)) =>
            {
                match read(stdin.as_fd(), &mut chunk) {
                    Ok(0) | Err(_) => return,
                    Ok(_) => {}
                }
            }
            _ => return,
        }
    }
}

impl Screen<Virtual> {
    #[inline]
    pub fn new(theme_default: ThemeAttribute) -> Self {
        Self::init(Virtual, theme_default)
    }

    #[must_use]
    pub fn resize(&mut self, cols: usize, rows: usize) -> bool {
        if (cols, rows) == (self.grid.cols, self.grid.rows)
            && (cols, rows) == (self.overlay_grid.cols, self.overlay_grid.rows)
        {
            return true;
        }
        if self.grid.resize(cols, rows, None) && self.overlay_grid.resize(cols, rows, None) {
            self.generation = self.generation.next();
            self.cols = cols;
            self.rows = rows;
            self.grid.area = self.area();
            self.overlay_grid.area = self.area();
            return true;
        }

        false
    }

    #[must_use]
    pub fn resize_with_context(&mut self, cols: usize, rows: usize, context: &Context) -> bool {
        if (cols, rows) == (self.grid.cols, self.grid.rows)
            && (cols, rows) == (self.overlay_grid.cols, self.overlay_grid.rows)
        {
            return true;
        }

        if self.grid.resize_with_context(cols, rows, context)
            && self.overlay_grid.resize_with_context(cols, rows, context)
        {
            self.generation = self.generation.next();
            self.cols = cols;
            self.rows = rows;
            self.grid.area = self.area();
            self.overlay_grid.area = self.area();
            return true;
        }

        false
    }
}

/// An `Area` consists of two points: the upper left and bottom right corners.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct Area {
    offset: Pos,
    upper_left: Pos,
    bottom_right: Pos,
    empty: bool,
    canvas_cols: usize,
    canvas_rows: usize,
    generation: ScreenGeneration,
}

impl std::fmt::Debug for Area {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct(stringify!(Area))
            .field("width", &self.width())
            .field("height", &self.height())
            .field("offset", &self.offset)
            .field("upper_left", &self.upper_left)
            .field("bottom_right", &self.bottom_right)
            .field("empty flag", &self.empty)
            .field("is_empty", &self.is_empty())
            .field("canvas_cols", &self.canvas_cols)
            .field("canvas_rows", &self.canvas_rows)
            .field("generation", &self.generation)
            .finish()
    }
}

impl<D: private::Sealed> From<&Screen<D>> for Area {
    fn from(sc: &Screen<D>) -> Self {
        sc.area()
    }
}

/// Convenience trait to turn both single `usize` values and `(usize, _)`
/// positions to `x` coordinate.
pub trait IntoColumns: private::Sealed {
    #[must_use]
    fn into(self) -> usize;
}

impl private::Sealed for usize {}
impl private::Sealed for Pos {}

impl IntoColumns for usize {
    fn into(self) -> usize {
        self
    }
}

impl IntoColumns for Pos {
    fn into(self) -> usize {
        get_x(self)
    }
}

impl Area {
    #[inline]
    pub fn height(&self) -> usize {
        if self.is_empty() {
            return 0;
        }
        get_y(self.bottom_right).saturating_sub(get_y(self.upper_left)) + 1
    }

    #[inline]
    pub fn width(&self) -> usize {
        if self.is_empty() {
            return 0;
        }
        get_x(self.bottom_right).saturating_sub(get_x(self.upper_left)) + 1
    }

    #[inline]
    pub fn size(&self) -> (usize, usize) {
        (self.width(), self.height())
    }

    /// Get `n`th row of `area` or its last one.
    #[inline]
    #[must_use]
    pub fn nth_row(&self, n: usize) -> Self {
        let Self {
            offset,
            upper_left,
            bottom_right,
            empty,
            canvas_cols,
            canvas_rows,
            generation,
        } = *self;
        let (_, max_y) = bottom_right;
        let n = std::cmp::min(n, self.height());
        if self.is_empty() || max_y < (get_y(upper_left) + n) {
            return self.into_empty();
        }
        let y = std::cmp::min(max_y, get_y(upper_left) + n);
        Self {
            offset: pos_inc(offset, (0, n)),
            upper_left: set_y(upper_left, y),
            bottom_right: set_y(bottom_right, y),
            empty,
            canvas_cols,
            canvas_rows,
            generation,
        }
    }

    /// Get `n`th col of `area` or its last one.
    #[inline]
    #[must_use]
    pub fn nth_col(&self, n: usize) -> Self {
        let Self {
            offset,
            upper_left,
            bottom_right,
            empty,
            canvas_cols,
            canvas_rows,
            generation,
        } = *self;
        let (max_x, _) = bottom_right;
        let n = std::cmp::min(n, self.width());
        if self.is_empty() || max_x < (get_x(upper_left) + n) {
            return self.into_empty();
        }
        let x = std::cmp::min(max_x, get_x(upper_left) + n);
        Self {
            offset: pos_inc(offset, (x, 0)),
            upper_left: set_x(upper_left, x),
            bottom_right: set_x(bottom_right, x),
            empty,
            canvas_cols,
            canvas_rows,
            generation,
        }
    }

    /// Place box given by `(width, height)` in corner of `area`
    #[must_use]
    pub fn place_inside(&self, (width, height): (usize, usize), upper: bool, left: bool) -> Self {
        if self.is_empty() || width < 3 || height < 3 {
            return *self;
        }
        let (upper_x, upper_y) = self.upper_left;
        let (max_x, max_y) = self.bottom_right;
        let x = if upper {
            upper_x + 2
        } else {
            max_x.saturating_sub(2).saturating_sub(width)
        };

        let y = if left {
            upper_y + 2
        } else {
            max_y.saturating_sub(2).saturating_sub(height)
        };
        let upper_left = (std::cmp::min(x, max_x), std::cmp::min(y, max_y));
        let bottom_right = (
            std::cmp::min(x + width, max_x),
            std::cmp::min(y + height, max_y),
        );

        Self {
            offset: pos_inc(
                self.offset,
                (
                    (get_x(upper_left) - get_x(self.upper_left)),
                    (get_y(upper_left) - get_y(self.upper_left)),
                ),
            ),
            upper_left,
            bottom_right,
            empty: self.empty,
            canvas_cols: self.canvas_cols,
            canvas_rows: self.canvas_rows,
            generation: self.generation,
        }
    }

    /// Place given area of dimensions `(width, height)` inside `area` according
    /// to given alignment
    #[must_use]
    pub fn align_inside(
        &self,
        (width, height): (usize, usize),
        horizontal_alignment: Alignment,
        vertical_alignment: Alignment,
    ) -> Self {
        if self.is_empty() || width == 0 || height == 0 {
            return *self;
        }
        let (top_x, width) = match horizontal_alignment {
            Alignment::Center => (
                { std::cmp::max(self.width() / 2, width / 2) - width / 2 },
                width,
            ),
            Alignment::Start => (0, self.width().min(width)),
            Alignment::End => (self.width().saturating_sub(width), self.width().min(width)),
            Alignment::Fill => (0, self.width()),
        };
        let (top_y, height) = match vertical_alignment {
            Alignment::Center => (
                { std::cmp::max(self.height() / 2, height / 2) - height / 2 },
                self.height().min(height),
            ),
            Alignment::Start => (0, self.height().min(height)),
            Alignment::End => (self.height().saturating_sub(height), self.height()),
            Alignment::Fill => (0, self.height()),
        };

        self.skip(top_x, top_y).take(width, height)
    }

    /// Place box given by `dimensions` in center of `area`
    #[inline]
    #[must_use]
    pub fn center_inside(&self, dimensions: (usize, usize)) -> Self {
        self.align_inside(dimensions, Alignment::Center, Alignment::Center)
    }

    #[inline]
    pub fn contains(&self, other: Self) -> bool {
        debug_assert_eq!(self.generation, other.generation);
        if self.is_empty() {
            return false;
        } else if other.is_empty() {
            return true;
        }
        get_y(other.bottom_right) <= get_y(self.bottom_right)
            && get_x(other.upper_left) >= get_x(self.upper_left)
            && get_y(other.upper_left) >= get_y(self.upper_left)
            && get_x(other.bottom_right) <= get_x(self.bottom_right)
    }

    /// Skip `n` rows and return the remaining area.
    /// Return value will be an empty area if `n` is more than the height.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use meli::terminal::{Screen, Virtual, Area};
    /// # let mut screen = Screen::<Virtual>::new(Default::default());
    /// # assert!(screen.resize(120, 20));
    /// # let area = screen.area();
    /// assert_eq!(area.width(), 120);
    /// assert_eq!(area.height(), 20);
    /// // Skip first two rows:
    /// let body = area.skip_rows(2);
    /// assert_eq!(body.height(), 18);
    /// ```
    #[inline]
    #[must_use]
    pub fn skip_rows(&self, n: usize) -> Self {
        let n = std::cmp::min(n, self.height());
        if self.is_empty() || self.upper_left.1 + n > self.bottom_right.1 {
            return self.into_empty();
        }

        Self {
            offset: pos_inc(self.offset, (0, n)),
            upper_left: pos_inc(self.upper_left, (0, n)),
            ..*self
        }
    }

    /// Skip the last `n` rows and return the remaining area.
    /// Return value will be an empty area if `n` is more than the height.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use meli::terminal::{Screen, Virtual, Area};
    /// # let mut screen = Screen::<Virtual>::new(Default::default());
    /// # assert!(screen.resize(120, 20));
    /// # let area = screen.area();
    /// assert_eq!(area.width(), 120);
    /// assert_eq!(area.height(), 20);
    /// // Take only first two rows (equivalent to area.take_rows(2))
    /// let header = area.skip_rows_from_end(18);
    /// assert_eq!(header.height(), 2);
    /// assert_eq!(header, area.take_rows(2));
    /// ```
    #[inline]
    #[must_use]
    pub fn skip_rows_from_end(&self, n: usize) -> Self {
        let n = std::cmp::min(n, self.height());
        if self.is_empty() || self.bottom_right.1 < n {
            return self.into_empty();
        }

        Self {
            bottom_right: (self.bottom_right.0, self.bottom_right.1 - n),
            ..*self
        }
    }

    #[inline]
    #[must_use]
    fn _skip_cols_inner(&self, n: usize) -> Self {
        let n = std::cmp::min(n, self.width());
        if self.is_empty() || self.bottom_right.0 < self.upper_left.0 + n {
            return self.into_empty();
        }

        Self {
            offset: pos_inc(self.offset, (n, 0)),
            upper_left: pos_inc(self.upper_left, (n, 0)),
            ..*self
        }
    }

    /// Skip the first `n` rows and return the remaining area.
    /// Return value will be an empty area if `n` is more than the width.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use meli::terminal::{Screen, Virtual, Area};
    /// # let mut screen = Screen::<Virtual>::new(Default::default());
    /// # assert!(screen.resize(120, 20));
    /// # let area = screen.area();
    /// assert_eq!(area.width(), 120);
    /// assert_eq!(area.height(), 20);
    /// // Skip first two columns
    /// let indent = area.skip_cols(2);
    /// assert_eq!(indent.width(), 118);
    /// ```
    #[inline]
    #[must_use]
    pub fn skip_cols(&self, n: impl IntoColumns) -> Self {
        let n: usize = n.into();
        self._skip_cols_inner(n)
    }

    /// Skip the last `n` rows and return the remaining area.
    /// Return value will be an empty area if `n` is more than the width.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use meli::terminal::{Screen, Virtual, Area};
    /// # let mut screen = Screen::<Virtual>::new(Default::default());
    /// # assert!(screen.resize(120, 20));
    /// # let area = screen.area();
    /// assert_eq!(area.width(), 120);
    /// assert_eq!(area.height(), 20);
    /// // Skip last two columns
    /// let indent = area.skip_cols_from_end(2);
    /// assert_eq!(indent.width(), 118);
    /// assert_eq!(indent, area.take_cols(118));
    /// ```
    #[inline]
    #[must_use]
    pub fn skip_cols_from_end(&self, n: usize) -> Self {
        let n = std::cmp::min(n, self.width());
        if self.is_empty() || self.bottom_right.0 < n {
            return self.into_empty();
        }
        Self {
            bottom_right: (self.bottom_right.0 - n, self.bottom_right.1),
            ..*self
        }
    }

    /// Shortcut for using `Area::skip_cols` and `Area::skip_rows` together.
    #[inline]
    #[must_use]
    pub fn skip(&self, n_cols: usize, n_rows: usize) -> Self {
        self.skip_cols(n_cols).skip_rows(n_rows)
    }

    /// Take the first `n` rows and return the remaining area.
    /// Return value will be an empty area if `n` is more than the height.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use meli::terminal::{Screen, Virtual, Area};
    /// # let mut screen = Screen::<Virtual>::new(Default::default());
    /// # assert!(screen.resize(120, 20));
    /// # let area = screen.area();
    /// assert_eq!(area.width(), 120);
    /// assert_eq!(area.height(), 20);
    /// // Take only first two rows
    /// let header = area.take_rows(2);
    /// assert_eq!(header.height(), 2);
    /// ```
    #[inline]
    #[must_use]
    pub fn take_rows(&self, n: usize) -> Self {
        let n = std::cmp::min(n, self.height());
        if self.is_empty() || self.bottom_right.1 < (self.height() - n) {
            return self.into_empty();
        }

        Self {
            bottom_right: (
                self.bottom_right.0,
                self.bottom_right.1 - (self.height() - n),
            ),
            ..*self
        }
    }

    /// Take the first `n` columns and return the remaining area.
    /// Return value will be an empty area if `n` is more than the width.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use meli::terminal::{Screen, Virtual, Area};
    /// # let mut screen = Screen::<Virtual>::new(Default::default());
    /// # assert!(screen.resize(120, 20));
    /// # let area = screen.area();
    /// assert_eq!(area.width(), 120);
    /// assert_eq!(area.height(), 20);
    /// // Take only first two columns
    /// let header = area.take_cols(2);
    /// assert_eq!(header.width(), 2);
    /// ```
    #[inline]
    #[must_use]
    pub fn take_cols(&self, n: usize) -> Self {
        let n = std::cmp::min(n, self.width());
        if self.is_empty() || self.bottom_right.0 < (self.width() - n) {
            return self.into_empty();
        }

        Self {
            bottom_right: (
                self.bottom_right.0 - (self.width() - n),
                self.bottom_right.1,
            ),
            ..*self
        }
    }

    /// Shortcut for using `Area::take_cols` and `Area::take_rows` together.
    #[inline]
    #[must_use]
    pub fn take(&self, n_cols: usize, n_rows: usize) -> Self {
        self.take_cols(n_cols).take_rows(n_rows)
    }

    #[inline]
    #[must_use]
    pub const fn upper_left(&self) -> Pos {
        self.upper_left
    }

    #[inline]
    #[must_use]
    pub const fn bottom_right(&self) -> Pos {
        self.bottom_right
    }

    #[inline]
    #[must_use]
    pub const fn upper_right(&self) -> Pos {
        set_x(self.upper_left, get_x(self.bottom_right))
    }

    #[inline]
    #[must_use]
    pub const fn bottom_left(&self) -> Pos {
        set_y(self.upper_left, get_y(self.bottom_right))
    }

    #[inline]
    #[must_use]
    pub const fn offset(&self) -> Pos {
        self.offset
    }

    #[inline]
    #[must_use]
    pub const fn generation(&self) -> ScreenGeneration {
        self.generation
    }

    #[inline]
    #[must_use]
    pub const fn new_empty(generation: ScreenGeneration) -> Self {
        Self {
            offset: (0, 0),
            upper_left: (0, 0),
            bottom_right: (0, 0),
            canvas_rows: 0,
            canvas_cols: 0,
            empty: true,
            generation,
        }
    }

    #[inline]
    #[must_use]
    pub const fn into_empty(self) -> Self {
        Self {
            offset: (0, 0),
            upper_left: (0, 0),
            bottom_right: (0, 0),
            empty: true,
            ..self
        }
    }

    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.empty
            || (self.upper_left.0 > self.bottom_right.0 || self.upper_left.1 > self.bottom_right.1)
    }
}

#[inline(always)]
#[must_use]
const fn pos_inc(p: Pos, inc: (usize, usize)) -> Pos {
    (p.0 + inc.0, p.1 + inc.1)
}

#[inline(always)]
#[must_use]
const fn get_x(p: Pos) -> usize {
    p.0
}

#[inline(always)]
#[must_use]
const fn get_y(p: Pos) -> usize {
    p.1
}

#[inline(always)]
#[must_use]
const fn set_x(p: Pos, new_x: usize) -> Pos {
    (new_x, p.1)
}

#[inline(always)]
#[must_use]
const fn set_y(p: Pos, new_y: usize) -> Pos {
    (p.0, new_y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skip_rows() {
        let mut screen = Screen::<Virtual>::new(Default::default());
        assert!(screen.resize(120, 20));
        let area = screen.area();
        assert_eq!(area.width(), 120);
        assert_eq!(area.height(), 20);

        for i in 1..=area.height() {
            assert_eq!(area.skip_rows(i).height(), area.height() - i);
            assert!(area.contains(area.skip_rows(i)));
            if i < area.height() {
                assert!(!area.take_rows(i).contains(area.skip_rows(i)));
            } else {
                assert!(area.take_rows(i).contains(area.skip_rows(i)));
            }
        }

        assert!(area.skip_rows(area.height()).is_empty());
        assert_eq!(area.skip_rows(0), area);
    }

    #[test]
    fn test_skip_rows_from_end() {
        let mut screen = Screen::<Virtual>::new(Default::default());
        assert!(screen.resize(120, 20));
        let area = screen.area();
        assert_eq!(area.width(), 120);
        assert_eq!(area.height(), 20);

        for i in 1..=area.height() {
            assert_eq!(area.skip_rows_from_end(i).height(), area.height() - i);
            assert!(area.contains(area.skip_rows_from_end(i)));
        }

        assert!(area.skip_rows_from_end(area.height()).is_empty());
        assert_eq!(area.skip_rows_from_end(0), area);
    }

    #[test]
    fn test_skip_cols() {
        let mut screen = Screen::<Virtual>::new(Default::default());
        assert!(screen.resize(120, 20));
        let area = screen.area();
        assert_eq!(area.width(), 120);
        assert_eq!(area.height(), 20);

        for i in 1..=area.width() {
            assert_eq!(area.skip_cols(i).width(), area.width() - i);
            assert!(area.contains(area.skip_cols(i)));
            if i < area.width() {
                assert!(!area.take_cols(i).contains(area.skip_cols(i)));
            } else {
                assert!(area.take_cols(i).contains(area.skip_cols(i)));
            }
        }

        assert!(area.skip_cols(area.width()).is_empty());
        assert_eq!(area.skip_cols(0), area);
    }

    #[test]
    fn test_skip_cols_from_end() {
        let mut screen = Screen::<Virtual>::new(Default::default());
        assert!(screen.resize(120, 20));
        let area = screen.area();
        assert_eq!(area.width(), 120);
        assert_eq!(area.height(), 20);

        for i in 1..=area.width() {
            assert_eq!(area.skip_cols_from_end(i).width(), area.width() - i);
            assert!(area.contains(area.skip_cols_from_end(i)));
        }

        assert!(area.skip_cols_from_end(area.width()).is_empty());
        assert_eq!(area.skip_cols_from_end(0), area);
    }

    #[test]
    fn test_take_rows() {
        let mut screen = Screen::<Virtual>::new(Default::default());
        assert!(screen.resize(120, 20));
        let area = screen.area();
        assert_eq!(area.width(), 120);
        assert_eq!(area.height(), 20);

        for i in 1..=area.height() {
            assert_eq!(area.take_rows(i).height(), i);
            assert!(area.contains(area.take_rows(i)));
        }

        assert!(area.take_rows(0).is_empty());
        assert_eq!(area.take_rows(area.height()), area);
    }

    #[test]
    fn test_take_cols() {
        let mut screen = Screen::<Virtual>::new(Default::default());
        assert!(screen.resize(120, 20));
        let area = screen.area();
        assert_eq!(area.width(), 120);
        assert_eq!(area.height(), 20);

        for i in 1..=area.width() {
            assert_eq!(area.take_cols(i).width(), i);
            assert!(area.contains(area.take_cols(i)));
        }

        assert!(area.take_cols(0).is_empty());
        assert_eq!(area.take_cols(area.width()), area);
    }

    #[test]
    fn test_nth_area() {
        let mut screen = Screen::<Virtual>::new(Default::default());
        assert!(screen.resize(120, 20));
        let area = screen.area();
        assert_eq!(area.width(), 120);
        assert_eq!(area.height(), 20);

        for i in 0..area.width() {
            assert_eq!(area.nth_col(i).width(), 1);
            assert!(area.contains(area.nth_col(i)));
            if i + 1 == area.width() {
                assert!(area.nth_col(i).contains(area.nth_col(i + 1)));
            } else {
                assert!(!area.nth_col(i).contains(area.nth_col(i + 1)));
            }
        }

        for i in 0..area.height() {
            assert_eq!(area.nth_row(i).height(), 1);
            assert!(area.contains(area.nth_row(i)));
            if i + 1 == area.height() {
                assert!(area.nth_row(i).contains(area.nth_row(i + 1)));
            } else {
                assert!(!area.nth_row(i).contains(area.nth_row(i + 1)));
            }
        }
    }

    #[test]
    fn test_place_inside_area() {
        let mut screen = Screen::<Virtual>::new(Default::default());
        assert!(screen.resize(120, 20));
        let area = screen.area();
        assert_eq!(area.width(), 120);
        assert_eq!(area.height(), 20);

        for width in 0..area.width() {
            for height in 0..area.height() {
                for upper in [true, false] {
                    for left in [true, false] {
                        let inner = area.place_inside((width, height), upper, left);
                        assert!(area.contains(inner));
                        if (3..area.height() - 2).contains(&height)
                            && (3..area.width() - 2).contains(&width)
                        {
                            assert_ne!(area, inner);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn test_align_inside_area() {
        use Alignment::{Center, End, Fill, Start};

        const ALIGNMENTS: [Alignment; 4] = [Fill, Start, End, Center];

        let mut screen = Screen::<Virtual>::new(Default::default());
        assert!(screen.resize(120, 20));
        let area = screen.area();
        assert_eq!(area.width(), 120);
        assert_eq!(area.height(), 20);

        // Ask for all subsets that area has:
        for width in 1..=area.width() {
            for height in 1..=area.height() {
                for horz in ALIGNMENTS {
                    for vert in ALIGNMENTS {
                        let inner = area.align_inside((width, height), horz, vert);
                        assert!(area.contains(inner));
                        assert!(!inner.is_empty());
                        match (horz, vert) {
                            (Fill, Fill) => {
                                assert_eq!(area, inner);
                            }
                            (Fill, _) => {
                                assert_eq!(inner.width(), area.width());
                                assert_eq!(inner.height(), height);
                            }
                            (_, Fill) => {
                                assert_eq!(inner.height(), area.height());
                                assert_eq!(inner.width(), width);
                            }
                            _ => {
                                assert_eq!((width, height), inner.size());
                                if (width, height) != area.size() {
                                    assert_ne!(area, inner);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Ask for more width/height than area has:
        for width in 1..=(2 * area.width()) {
            for height in 1..=(2 * area.height()) {
                for horz in ALIGNMENTS {
                    for vert in ALIGNMENTS {
                        let inner = area.align_inside((width, height), horz, vert);
                        assert!(area.contains(inner));
                        assert!(!inner.is_empty());
                        match (horz, vert) {
                            (Fill, Fill) => {
                                assert_eq!(area, inner);
                            }
                            (Fill, _) => {
                                assert_eq!(inner.width(), area.width());
                                assert!(
                                    height >= inner.height() && area.height() >= inner.height()
                                );
                            }
                            (_, Fill) => {
                                assert_eq!(inner.height(), area.height());
                                assert!(width >= inner.width() && area.width() >= inner.width());
                            }
                            _ => {
                                assert!(
                                    height >= inner.height() && area.height() >= inner.height()
                                );
                                assert!(width >= inner.width() && area.width() >= inner.width());
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn test_parse_palette_replies_osc_replies() {
        let mut palette = (None, None);
        let mut buf = b"\x1b]11;rgb:ffff/0000/0000\x1b\\\x1b]10;rgb:0000/ffff/0000\x07".to_vec();
        parse_palette_replies(&mut buf, &mut palette);
        assert_eq!(palette.1, Some(Color::Rgb(255, 0, 0)));
        assert_eq!(palette.0, Some(Color::Rgb(0, 255, 0)));
        assert!(buf.is_empty());
    }

    #[test]
    fn test_parse_palette_replies_split_across_reads() {
        let mut palette = (None, None);
        let mut buf = b"\x1b]11;rgb:ffff/".to_vec();
        parse_palette_replies(&mut buf, &mut palette);
        assert_eq!(palette, (None, None));
        buf.extend_from_slice(b"ffff/ffff\x1b\\");
        parse_palette_replies(&mut buf, &mut palette);
        assert_eq!(palette.1, Some(Color::Rgb(255, 255, 255)));
        assert!(buf.is_empty());
    }

    #[test]
    fn test_parse_palette_replies_rejects_garbage() {
        let mut palette = (None, None);
        // Invalid UTF-8 and sequences with missing/invalid payload: parsing
        // must reject them without panicking and keep the default colors.
        let mut buf = b"garbage \xff\xfe \x1b[?9999z\x1b]10;notacolor\x07\x1b]11\x1b\\".to_vec();
        parse_palette_replies(&mut buf, &mut palette);
        assert_eq!(palette, (None, None));
        assert!(buf.is_empty());
    }

    #[test]
    fn test_parse_palette_replies_mixed_with_unknown_csi() {
        let mut palette = (None, None);
        // A late DECRPM-style reply interleaved with the palette replies:
        // it must be consumed without disturbing the palette parsing.
        let mut buf =
            b"\x1b[?2026;2$y\x1b]10;rgb:ffff/ffff/ffff\x1b\\\x1b]11;rgb:1c1c/1b1b/1919\x07"
                .to_vec();
        parse_palette_replies(&mut buf, &mut palette);
        assert_eq!(palette.0, Some(Color::Rgb(255, 255, 255)));
        assert_eq!(palette.1, Some(Color::Rgb(28, 27, 25)));
        assert!(buf.is_empty());
    }

    #[test]
    fn test_write_startup_queries() {
        let mut out = Vec::new();
        write_startup_queries(&mut out);
        // Background (OSC 11) query first, then foreground (OSC 10).
        assert_eq!(out, b"\x1b]11;?\x1b\\\x1b]10;?\x1b\\");
        // The Synchronized-Output support probe (`CSI ? 2026 $ p`) must not
        // be revived: its late replies stall the input parser.
        let needle = b"\x1b[?2026$p";
        assert!(!out.windows(needle.len()).any(|w| w == needle));
    }

    #[test]
    fn test_query_terminal_palette_mocked_reply() {
        use std::io::Write as _;
        use std::os::unix::net::UnixStream;

        let (reader, mut writer) = UnixStream::pair().unwrap();
        writer
            .write_all(b"\x1b]11;rgb:ffff/0000/0000\x1b\\")
            .unwrap();
        writer.write_all(b"\x1b]10;rgb:0000/0000/ffff\x07").unwrap();
        drop(writer); // EOF after the replies

        let mut palette = (None, None);
        query_terminal_palette(&reader, &mut palette, Duration::from_secs(2));
        assert_eq!(palette.1, Some(Color::Rgb(255, 0, 0)));
        assert_eq!(palette.0, Some(Color::Rgb(0, 0, 255)));
    }

    #[test]
    fn test_query_terminal_palette_silent_timeout() {
        use std::os::unix::net::UnixStream;

        // A terminal that never replies must not block the startup read loop
        // for longer than the query budget.
        let (reader, _writer) = UnixStream::pair().unwrap();
        let mut palette = (None, None);
        let start = Instant::now();
        query_terminal_palette(&reader, &mut palette, PALETTE_QUERY_TIMEOUT);
        let elapsed = start.elapsed();
        assert_eq!(palette, (None, None));
        assert!(
            elapsed >= PALETTE_QUERY_TIMEOUT - Duration::from_millis(10),
            "query returned too early: {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(1000),
            "query blocked too long: {elapsed:?}"
        );
    }
}
