//
// meli
//
// Copyright 2024 Emmanouil Pitsidianakis <manos@pitsidianak.is>
// Copyright 2026 Kyle Lee
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

#[test]
fn test_terminal_osc8_print() {
    use crate::terminal::Hyperlink;

    const TEST_CASES: &[(Hyperlink<str, str, str>, &str)] = &[
        (
            Hyperlink::new("text", "url"),
            "\x1b]8;;url\x07text\x1b]8;;\x07",
        ),
        (
            Hyperlink::new("/tmp/", "file:///tmp/"),
            "\x1b]8;;file:///tmp/\x07/tmp/\x1b]8;;\x07",
        ),
        (
            Hyperlink::new("meli(1)", "man:meli(1)"),
            "\x1b]8;;man:meli(1)\x07meli(1)\x1b]8;;\x07",
        ),
        (
            Hyperlink::with_id("duplicated", "meli(1)", "man:meli(1)"),
            "\x1b]8;id=duplicated;man:meli(1)\x07meli(1)\x1b]8;;\x07",
        ),
        (
            Hyperlink::with_id("duplicated", "meli(1)", "man:meli(1)"),
            "\x1b]8;id=duplicated;man:meli(1)\x07meli(1)\x1b]8;;\x07",
        ),
    ];

    for (input, output) in TEST_CASES {
        println!("{input}");
        assert_eq!(&input.to_string(), output);
    }
}

/// Semantic-equivalence harness for the terminal flush byte layer.
///
/// The emitters `Screen::draw_horizontal_segment` and
/// `Screen::draw_horizontal_segment_no_color` translate a `CellBuffer` row
/// segment into an escape byte stream (cursor positioning, SGR color and
/// attribute transitions, OSC 8 hyperlink spans, wide-char continuation
/// skips, `FORCE_TEXT` `U+FE0E` suffixes).
///
/// This harness decodes the emitted byte stream back into per-cell terminal
/// state (character, fg/bg, attribute flags, hyperlink URI) and compares it
/// against pins recorded from the previous emitter
/// implementation. SGR sequence ordering may differ between implementations;
/// the decoded per-cell attribute state must not.
///
/// Pin recording: `MELI_UPDATE_FLUSH_PINS=1` re-writes the pin file from the
/// current emitter (the pins in-tree were recorded from the OLD emitter
/// before the crossterm swap).
mod flush_equivalence {
    use std::{
        cell::RefCell,
        collections::BTreeMap,
        io::{BufWriter, Write},
        path::PathBuf,
        rc::Rc,
    };

    use melib::text::wcwidth;

    use crate::terminal::{Attr, Cell, CellBuffer, Color, Screen, StateStdout, Tty, Virtual};

    struct Case {
        name: &'static str,
        y: usize,
        xs: std::ops::Range<usize>,
        no_color: bool,
    }

    impl Case {
        const fn row(name: &'static str, y: usize, xs: std::ops::Range<usize>) -> Self {
            Self {
                name,
                y,
                xs,
                no_color: false,
            }
        }

        const fn row_no_color(name: &'static str, y: usize, xs: std::ops::Range<usize>) -> Self {
            Self {
                name,
                y,
                xs,
                no_color: true,
            }
        }
    }

    const COLS: usize = 48;
    const ROWS: usize = 12;

    /// Colors cycled through by the `colors_named` row: default, the eight
    /// named colors and their bright (16-color) counterparts.
    const NAMED_CYCLE: [Color; 17] = [
        Color::Default,
        Color::Black,
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::White,
        Color::Byte(8),
        Color::Byte(9),
        Color::Byte(10),
        Color::Byte(11),
        Color::Byte(12),
        Color::Byte(13),
        Color::Byte(14),
        Color::Byte(15),
    ];

    /// Fill a fresh grid (all-default cells) for a case that does not need
    /// hyperlink state.
    fn plain_grid() -> CellBuffer {
        fresh_grid(false)
    }

    fn fresh_grid(draw_hyperlinks: bool) -> CellBuffer {
        let mut screen = Screen::<Virtual>::new(Default::default());
        assert!(screen.resize(COLS, ROWS));
        let mut grid = screen.grid().clone();
        grid.draw_hyperlinks = draw_hyperlinks;
        grid
    }

    fn styled(ch: char, fg: Color, bg: Color, attrs: Attr) -> Cell {
        Cell::new(ch, fg, bg, attrs)
    }

    /// Continuation cell after a wide char, mirroring what
    /// `CellBuffer::write_string` produces.
    fn continuation(fg: Color, bg: Color, attrs: Attr) -> Cell {
        *Cell::new(' ', fg, bg, attrs).set_empty(true)
    }

    /// Populate the plain grid rows used by the corpus.
    fn fill_plain_rows(grid: &mut CellBuffer) {
        // Row 0: named/default/16-color fg cycle over a fixed bg.
        for (i, &fg) in NAMED_CYCLE.iter().enumerate() {
            grid[(i, 0)] = styled(
                char::from(b'a' + i as u8),
                fg,
                Color::Byte(234),
                Attr::DEFAULT,
            );
        }
        // Row 1: 256-color and RGB palette, fg and bg deltas interleaved.
        let cycle_256: [(Color, Color); 12] = [
            (Color::Byte(0), Color::Byte(255)),
            (Color::Byte(1), Color::Byte(16)),
            (Color::Byte(16), Color::Rgb(255, 0, 128)),
            (Color::Byte(231), Color::Default),
            (Color::Byte(255), Color::Byte(1)),
            (Color::Rgb(255, 0, 128), Color::Rgb(0, 0, 0)),
            (Color::Rgb(0, 0, 0), Color::Rgb(255, 255, 255)),
            (Color::Rgb(255, 255, 255), Color::Byte(240)),
            (Color::Default, Color::Byte(15)),
            (Color::Byte(15), Color::Rgb(1, 2, 3)),
            (Color::Rgb(1, 2, 3), Color::Byte(231)),
            (Color::Byte(240), Color::Default),
        ];
        for (i, &(fg, bg)) in cycle_256.iter().enumerate() {
            grid[(i, 1)] = styled(char::from(b'A' + i as u8), fg, bg, Attr::DEFAULT);
        }
        // Row 2: attribute combos (constant colors to isolate attr deltas).
        let attrs_cycle: [Attr; 13] = [
            Attr::DEFAULT,
            Attr::BOLD,
            Attr::BOLD | Attr::UNDERLINE,
            Attr::BOLD | Attr::UNDERLINE | Attr::REVERSE,
            Attr::REVERSE,
            Attr::DEFAULT,
            Attr::DIM,
            Attr::ITALICS,
            Attr::BLINK,
            Attr::HIDDEN,
            Attr::UNDERCURL,
            Attr::UNDERLINE | Attr::UNDERCURL,
            Attr::DEFAULT,
        ];
        for (i, &attrs) in attrs_cycle.iter().enumerate() {
            grid[(i, 2)] = styled(
                char::from(b'0' + i as u8),
                Color::White,
                Color::Black,
                attrs,
            );
        }
        // Row 3: undercurl on/off transitions.
        let undercurl_cycle: [Attr; 8] = [
            Attr::UNDERCURL,
            Attr::DEFAULT,
            Attr::UNDERCURL,
            Attr::UNDERLINE,
            Attr::UNDERLINE | Attr::UNDERCURL,
            Attr::DEFAULT,
            Attr::UNDERLINE,
            Attr::DEFAULT,
        ];
        for (i, &attrs) in undercurl_cycle.iter().enumerate() {
            grid[(i, 3)] = styled(
                char::from(b'm' + i as u8),
                Color::Green,
                Color::Default,
                attrs,
            );
        }
        // Row 4: FORCE_TEXT cells with color churn between them.
        grid[(0, 4)] = styled('\u{2602}', Color::Red, Color::Default, Attr::FORCE_TEXT);
        grid[(1, 4)] = styled('x', Color::Default, Color::Default, Attr::DEFAULT);
        grid[(2, 4)] = styled(
            '\u{26a1}',
            Color::Byte(9),
            Color::Byte(234),
            Attr::FORCE_TEXT | Attr::UNDERLINE,
        );
        grid[(3, 4)] = styled('y', Color::Default, Color::Default, Attr::DEFAULT);
        grid[(4, 4)] = styled(
            '\u{3012}',
            Color::Rgb(9, 8, 7),
            Color::Default,
            Attr::FORCE_TEXT | Attr::BOLD,
        );
        // Row 5: wide chars + continuation skips (incl. FORCE_TEXT on a wide
        // char) followed by narrow chars.
        grid[(0, 5)] = styled('\u{4e2d}', Color::Red, Color::Default, Attr::BOLD);
        grid[(1, 5)] = continuation(Color::Red, Color::Default, Attr::BOLD);
        grid[(2, 5)] = styled('\u{6587}', Color::Red, Color::Default, Attr::BOLD);
        grid[(3, 5)] = continuation(Color::Red, Color::Default, Attr::BOLD);
        grid[(4, 5)] = styled('a', Color::Red, Color::Default, Attr::BOLD);
        grid[(5, 5)] = styled('\u{ff57}', Color::Blue, Color::Default, Attr::DEFAULT);
        grid[(6, 5)] = continuation(Color::Blue, Color::Default, Attr::DEFAULT);
        grid[(7, 5)] = styled('Z', Color::Blue, Color::Default, Attr::DEFAULT);
        grid[(8, 5)] = styled(
            '\u{4e2d}',
            Color::Green,
            Color::Default,
            Attr::BOLD | Attr::FORCE_TEXT,
        );
        grid[(9, 5)] = continuation(Color::Green, Color::Default, Attr::BOLD | Attr::FORCE_TEXT);
        grid[(10, 5)] = styled('q', Color::Green, Color::Default, Attr::DEFAULT);
        // Row 6: scattered `empty` flags that are NOT wide-char continuations
        // (the emitter skips them without advancing the terminal cursor, so
        // subsequent printed cells shift left).
        grid[(0, 6)] = styled('A', Color::Default, Color::Default, Attr::DEFAULT);
        grid[(1, 6)] = continuation(Color::Default, Color::Default, Attr::DEFAULT);
        grid[(2, 6)] = styled('B', Color::Cyan, Color::Default, Attr::REVERSE);
        grid[(3, 6)] = continuation(Color::Cyan, Color::Default, Attr::REVERSE);
        grid[(4, 6)] = styled('C', Color::Default, Color::Default, Attr::DEFAULT);
        // Row 7: segment starting at a non-zero offset with color churn.
        for i in 5..13usize {
            let fg = if i % 2 == 0 {
                Color::Byte(1 + i as u8)
            } else {
                Color::Rgb(i as u8, 10, 20)
            };
            grid[(i, 7)] = styled(
                char::from(b'k' + (i - 5) as u8),
                fg,
                Color::Byte(237),
                Attr::BOLD,
            );
        }
        // Row 8: single-cell segment with heavy styling.
        grid[(40, 8)] = styled(
            'Q',
            Color::Rgb(10, 200, 30),
            Color::Rgb(4, 5, 6),
            Attr::BOLD | Attr::REVERSE,
        );
        // Row 9: left untouched (empty-segment case points here).
        // Row 10: all cells flagged empty.
        for x in 0..5usize {
            grid[(x, 10)] = continuation(Color::Default, Color::Default, Attr::DEFAULT);
        }
        // Row 11: non-default attrs on the first and last cells of the
        // segment (no trailing reset exists in the emitter).
        for x in 0..5usize {
            grid[(x, 11)] = styled(
                char::from(b'v' + x as u8),
                Color::Yellow,
                Color::Default,
                Attr::BOLD | Attr::REVERSE,
            );
        }
    }

    /// Hyperlink URL table ids used by the hyperlink grid.
    const HL_A: u64 = 1000;
    const HL_B: u64 = 1001;
    const HL_C: u64 = 1002;
    const HL_D: u64 = 1003;
    const HL_E: u64 = 1004;
    const HL_F: u64 = 1005;

    fn fill_hyperlink_rows(grid: &mut CellBuffer) {
        for (id, url) in [
            (HL_A, "https://example.com/a"),
            (HL_B, "https://example.com/b"),
            (HL_C, "https://example.com/c"),
            (HL_D, "https://example.com/d"),
            (HL_E, "https://example.com/e"),
            (HL_F, "https://example.com/f"),
        ] {
            grid.hyperlinks_table.insert(id, url.into());
        }
        // Row 0: single span (5..9 exclusive of 9, the end cell closes the
        // span before its own char is printed).
        for x in 0..20usize {
            grid[(x, 0)] = styled('p', Color::Default, Color::Default, Attr::DEFAULT);
        }
        grid.hyperlinks_associations.insert((5, 0), (HL_A, (9, 0)));
        // Row 1: adjacent spans: B starts exactly on A's end cell.
        for x in 0..14usize {
            grid[(x, 1)] = styled('r', Color::Default, Color::Default, Attr::DEFAULT);
        }
        grid.hyperlinks_associations.insert((2, 1), (HL_B, (6, 1)));
        grid.hyperlinks_associations.insert((6, 1), (HL_C, (10, 1)));
        // Row 2: nested spans: B starts inside A's range.
        for x in 0..14usize {
            grid[(x, 2)] = styled('n', Color::Default, Color::Default, Attr::DEFAULT);
        }
        grid.hyperlinks_associations.insert((2, 2), (HL_D, (8, 2)));
        grid.hyperlinks_associations.insert((4, 2), (HL_E, (6, 2)));
        // Row 3: span whose end lies outside the segment (closes at row end).
        for x in 0..COLS {
            grid[(x, 3)] = styled('t', Color::Default, Color::Default, Attr::DEFAULT);
        }
        grid.hyperlinks_associations
            .insert((10, 3), (HL_F, (COLS + 12, 3)));
        // Row 4: zero-length span (start == end; never matches the end-cell
        // close check, so it closes at row end).
        for x in 0..12usize {
            grid[(x, 4)] = styled('z', Color::Default, Color::Default, Attr::DEFAULT);
        }
        grid.hyperlinks_associations.insert((5, 4), (HL_A, (5, 4)));
        // Row 5: multiple spans with color churn inside them.
        for x in 0..16usize {
            grid[(x, 5)] = styled(
                char::from(b's'),
                if x % 4 == 2 {
                    Color::Byte(9)
                } else {
                    Color::Default
                },
                Color::Default,
                if x % 4 == 3 {
                    Attr::UNDERLINE
                } else {
                    Attr::DEFAULT
                },
            );
        }
        grid.hyperlinks_associations.insert((1, 5), (HL_B, (4, 5)));
        grid.hyperlinks_associations.insert((4, 5), (HL_C, (8, 5)));
        grid.hyperlinks_associations.insert((8, 5), (HL_D, (12, 5)));
    }

    /// Corpus: every case is emitted through the CURRENT production emitter
    /// (color or no-color variant) and its decode compared to the pin.
    fn corpus() -> Vec<Case> {
        vec![
            Case::row("colors_named_default_16", 0, 0..17),
            Case::row("colors_256_rgb", 1, 0..12),
            Case::row("attrs_combos", 2, 0..13),
            Case::row("undercurl_transitions", 3, 0..8),
            Case::row("force_text", 4, 0..5),
            Case::row("wide_char_skip", 5, 0..11),
            Case::row("empty_scatter", 6, 0..5),
            Case::row("segment_offset", 7, 5..13),
            Case::row("single_cell", 8, 40..41),
            Case::row("empty_segment", 9, 10..10),
            Case::row("all_empty", 10, 0..5),
            Case::row("attrs_at_edges", 11, 0..5),
            Case::row("hl_single", 0, 0..20),
            Case::row("hl_adjacent", 1, 0..14),
            Case::row("hl_nested", 2, 0..14),
            Case::row("hl_to_row_end", 3, 0..COLS),
            Case::row("hl_zero_len", 4, 0..12),
            Case::row("hl_multiple_color_churn", 5, 0..16),
            Case::row("hl_disabled", 0, 0..20),
            Case::row_no_color("nc_colors_named", 0, 0..17),
            Case::row_no_color("nc_colors_256_rgb", 1, 0..12),
            Case::row_no_color("nc_attrs_combos", 2, 0..13),
            Case::row_no_color("nc_undercurl", 3, 0..8),
            Case::row_no_color("nc_force_text", 4, 0..5),
            Case::row_no_color("nc_wide_char", 5, 0..11),
            Case::row_no_color("nc_hl_single", 0, 0..20),
        ]
    }

    /// Which grid a case draws from.
    fn grid_for(case: &Case) -> CellBuffer {
        match case.name {
            "hl_disabled" => {
                let mut grid = fresh_grid(false);
                fill_hyperlink_rows(&mut grid);
                grid
            }
            name if name.starts_with("hl_") => {
                let mut grid = fresh_grid(true);
                fill_hyperlink_rows(&mut grid);
                grid
            }
            name if name.starts_with("nc_hl") => {
                let mut grid = fresh_grid(true);
                fill_hyperlink_rows(&mut grid);
                grid
            }
            _ => {
                let mut grid = plain_grid();
                fill_plain_rows(&mut grid);
                grid
            }
        }
    }

    /// Writer that shares its buffer with the test so the emitted bytes can
    /// be inspected after the `StateStdout` `BufWriter` is flushed.
    #[derive(Clone, Default)]
    struct SharedBuf(Rc<RefCell<Vec<u8>>>);

    impl SharedBuf {
        fn into_inner(self) -> Vec<u8> {
            Rc::try_unwrap(self.0)
                .map(|inner| inner.into_inner())
                .unwrap_or_default()
        }
    }

    impl std::io::Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Run one corpus case through the production emitter and return the raw
    /// byte stream.
    fn emit_case(case: &Case) -> Vec<u8> {
        isolate_from_no_color_env();
        let mut grid = grid_for(case);
        let sink = SharedBuf::default();
        let mut stdout: StateStdout =
            BufWriter::with_capacity(8192, Box::new(sink.clone()) as Box<dyn std::io::Write>);
        if case.no_color {
            Screen::<Tty>::draw_horizontal_segment_no_color(
                &mut grid,
                &mut stdout,
                case.xs.clone(),
                case.y,
            );
        } else {
            Screen::<Tty>::draw_horizontal_segment(&mut grid, &mut stdout, case.xs.clone(), case.y);
        }
        stdout.flush().unwrap();
        drop(stdout);
        sink.into_inner()
    }

    /// The pinned byte-level spec asserts full SGR color output, but the
    /// vendored crossterm honors the `NO_COLOR` convention
    /// (no-color.org): a `NO_COLOR` set in the host environment makes
    /// `SetForegroundColor`/`SetBackgroundColor` render as an empty SGR,
    /// silently stripping every color byte. Force the emitter's color
    /// output on regardless of the host environment so the spec this
    /// module pins stays observable. `set_ansi_color_disabled` overrides
    /// the memoized env snapshot, so this also repairs a poisoned
    /// process-global state.
    fn isolate_from_no_color_env() {
        crossterm::style::Colored::set_ansi_color_disabled(false);
    }

    #[derive(Clone, Debug, PartialEq)]
    enum DecColor {
        Default,
        Index(u8),
        Rgb(u8, u8, u8),
    }

    /// Per-cell terminal state decoded from an escape byte stream.
    #[derive(Clone, Debug, PartialEq)]
    struct DecCell {
        ch: char,
        fg: DecColor,
        bg: DecColor,
        bold: bool,
        dim: bool,
        italics: bool,
        underline: bool,
        undercurl: bool,
        blink: bool,
        reverse: bool,
        hidden: bool,
        force_text: bool,
        uri: Option<String>,
    }

    #[derive(Clone)]
    struct VtState {
        col: usize,
        fg: DecColor,
        bg: DecColor,
        bold: bool,
        dim: bool,
        italics: bool,
        underline: bool,
        underline_curly: bool,
        blink: bool,
        reverse: bool,
        hidden: bool,
        uri: Option<String>,
    }

    impl VtState {
        fn new() -> Self {
            Self {
                col: 0,
                fg: DecColor::Default,
                bg: DecColor::Default,
                bold: false,
                dim: false,
                italics: false,
                underline: false,
                underline_curly: false,
                blink: false,
                reverse: false,
                hidden: false,
                uri: None,
            }
        }
    }

    /// Decode a single-row escape stream into ordered per-column cell state.
    /// Columns not covered by a printed character (e.g. wide-char
    /// continuation columns) are simply absent from the map.
    fn decode(bytes: &[u8]) -> BTreeMap<usize, DecCell> {
        let mut st = VtState::new();
        let mut cells: BTreeMap<usize, DecCell> = BTreeMap::new();
        let mut last_col: Option<usize> = None;
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            if b == 0x1b {
                if i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                    // CSI sequence: scan to the final byte.
                    let mut j = i + 2;
                    while j < bytes.len() && !(0x40..=0x7e).contains(&bytes[j]) {
                        j += 1;
                    }
                    if j >= bytes.len() {
                        break;
                    }
                    let body = String::from_utf8_lossy(&bytes[i + 2..j]).into_owned();
                    match bytes[j] {
                        b'H' => {
                            let mut parts = body.split(';');
                            let row: usize = parts.next().and_then(|p| p.parse().ok()).unwrap_or(1);
                            let col: usize = parts.next().and_then(|p| p.parse().ok()).unwrap_or(1);
                            assert!(row >= 1 && col >= 1, "CUP is 1-based");
                            st.col = col - 1;
                        }
                        b'G' => {
                            // CHA (cursor horizontal absolute): 1-based column.
                            let col: usize = body.parse().unwrap_or(1);
                            assert!(col >= 1, "CHA is 1-based");
                            st.col = col - 1;
                        }
                        b'm' => apply_sgr(&mut st, &body),
                        _ => {}
                    }
                    i = j + 1;
                } else if i + 1 < bytes.len() && bytes[i + 1] == b']' {
                    // OSC: scan to BEL or ST.
                    let mut j = i + 2;
                    let mut st_terminated = false;
                    while j < bytes.len() {
                        if bytes[j] == 0x07 {
                            break;
                        }
                        if bytes[j] == 0x1b && j + 1 < bytes.len() && bytes[j + 1] == b'\\' {
                            st_terminated = true;
                            break;
                        }
                        j += 1;
                    }
                    let body = String::from_utf8_lossy(&bytes[i + 2..j]).into_owned();
                    if let Some(rest) = body.strip_prefix("8;") {
                        if let Some(pos) = rest.find(';') {
                            let uri = &rest[pos + 1..];
                            st.uri = if uri.is_empty() {
                                None
                            } else {
                                Some(uri.to_string())
                            };
                        }
                    }
                    i = j + if st_terminated { 2 } else { 1 };
                } else {
                    // Other two-byte escape; skip.
                    i += 2;
                }
            } else if b < 0x20 || b == 0x7f {
                // Control bytes are not printed.
                i += 1;
            } else {
                // Decode one UTF-8 scalar and print it at the cursor.
                let (ch, len) = next_char(&bytes[i..]);
                if ch == '\u{fe0e}' {
                    if let Some(col) = last_col {
                        if let Some(cell) = cells.get_mut(&col) {
                            cell.force_text = true;
                        }
                    }
                } else {
                    let dc = DecCell {
                        ch,
                        fg: st.fg.clone(),
                        bg: st.bg.clone(),
                        bold: st.bold,
                        dim: st.dim,
                        italics: st.italics,
                        underline: st.underline,
                        undercurl: st.underline && st.underline_curly,
                        blink: st.blink,
                        reverse: st.reverse,
                        hidden: st.hidden,
                        force_text: false,
                        uri: st.uri.clone(),
                    };
                    cells.insert(st.col, dc);
                    last_col = Some(st.col);
                    let w = wcwidth(ch).filter(|w| *w > 0).unwrap_or(0);
                    st.col += w;
                }
                i += len;
            }
        }
        cells
    }

    fn next_char(bytes: &[u8]) -> (char, usize) {
        let len = match bytes[0] {
            0x00..=0x7f => 1,
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf7 => 4,
            _ => 1,
        };
        let len = len.min(bytes.len());
        let s = String::from_utf8_lossy(&bytes[..len]).into_owned();
        (s.chars().next().unwrap_or('\u{fffd}'), len)
    }

    fn apply_sgr(st: &mut VtState, body: &str) {
        let params: Vec<&str> = body.split(';').collect();
        let mut k = 0;
        while k < params.len() {
            let raw = params[k];
            let (main, sub) = match raw.split_once(':') {
                Some((m, s)) => (m.to_string(), Some(s.to_string())),
                None => (raw.to_string(), None),
            };
            let code: u16 = main.trim().parse().unwrap_or(0);
            match code {
                // SGR 0 resets rendition attributes and colors only; it does
                // NOT move the cursor and does NOT close an OSC 8 span.
                0 => {
                    st.fg = DecColor::Default;
                    st.bg = DecColor::Default;
                    st.bold = false;
                    st.dim = false;
                    st.italics = false;
                    st.underline = false;
                    st.underline_curly = false;
                    st.blink = false;
                    st.reverse = false;
                    st.hidden = false;
                }
                1 => st.bold = true,
                2 => st.dim = true,
                3 => st.italics = true,
                4 => match sub.as_deref() {
                    Some("0") => {
                        st.underline = false;
                        st.underline_curly = false;
                    }
                    Some("3") => {
                        st.underline = true;
                        st.underline_curly = true;
                    }
                    Some(_) => {
                        st.underline = true;
                        st.underline_curly = false;
                    }
                    None => {
                        st.underline = true;
                        st.underline_curly = false;
                    }
                },
                5 => st.blink = true,
                7 => st.reverse = true,
                8 => st.hidden = true,
                22 => {
                    st.bold = false;
                    st.dim = false;
                }
                23 => st.italics = false,
                24 => {
                    st.underline = false;
                    st.underline_curly = false;
                }
                25 => st.blink = false,
                27 => st.reverse = false,
                28 => st.hidden = false,
                39 => st.fg = DecColor::Default,
                49 => st.bg = DecColor::Default,
                38 | 48 => {
                    let is_fg = code == 38;
                    if let Some(color) = extended_color(&params, k) {
                        let consumed = match color {
                            DecColor::Index(_) => 2,
                            DecColor::Rgb(_, _, _) => 4,
                            DecColor::Default => 0,
                        };
                        if is_fg {
                            st.fg = color;
                        } else {
                            st.bg = color;
                        }
                        k += consumed;
                    }
                }
                _ => {}
            }
            k += 1;
        }
    }

    fn extended_color(params: &[&str], k: usize) -> Option<DecColor> {
        let kind = *params.get(k + 1)?;
        match kind {
            "5" => params
                .get(k + 2)
                .and_then(|p| p.parse().ok())
                .map(DecColor::Index),
            "2" => {
                let r: u8 = params.get(k + 2).and_then(|p| p.parse().ok())?;
                let g: u8 = params.get(k + 3).and_then(|p| p.parse().ok())?;
                let b: u8 = params.get(k + 4).and_then(|p| p.parse().ok())?;
                Some(DecColor::Rgb(r, g, b))
            }
            _ => None,
        }
    }

    fn color_token(c: &DecColor) -> String {
        match c {
            DecColor::Default => "d".to_string(),
            DecColor::Index(n) => format!("i{n}"),
            DecColor::Rgb(r, g, b) => format!("r{r}.{g}.{b}"),
        }
    }

    /// Stable one-line serialization of a decoded cell.
    fn cell_token(col: usize, c: &DecCell) -> String {
        let mask = format!(
            "{}{}{}{}{}{}{}{}",
            u8::from(c.bold),
            u8::from(c.dim),
            u8::from(c.italics),
            u8::from(c.underline),
            u8::from(c.undercurl),
            u8::from(c.blink),
            u8::from(c.reverse),
            u8::from(c.hidden),
        );
        format!(
            "{} {} {} {} {} {} {}",
            col,
            c.ch.escape_unicode().collect::<String>(),
            color_token(&c.fg),
            color_token(&c.bg),
            mask,
            u8::from(c.force_text),
            c.uri.as_deref().unwrap_or("-"),
        )
    }

    fn serialize_decode(cells: &BTreeMap<usize, DecCell>) -> String {
        cells
            .iter()
            .map(|(col, c)| cell_token(*col, c))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn pins_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/flush_equivalence_pins.txt")
    }

    /// Parse the pin file into (case name -> serialized decode) pairs.
    fn parse_pins(content: &str) -> BTreeMap<String, String> {
        let mut pins = BTreeMap::new();
        let mut current: Option<(String, Vec<String>)> = None;
        for line in content.lines() {
            let line = line.trim_end();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("case ") {
                if let Some((name, lines)) = current.take() {
                    pins.insert(name, lines.join("\n"));
                }
                current = Some((rest.to_string(), Vec::new()));
            } else if line == "end" {
                if let Some((name, lines)) = current.take() {
                    pins.insert(name, lines.join("\n"));
                }
            } else if let Some((_, lines)) = current.as_mut() {
                lines.push(line.to_string());
            }
        }
        if let Some((name, lines)) = current.take() {
            pins.insert(name, lines.join("\n"));
        }
        pins
    }

    fn run_corpus() -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        for case in corpus() {
            let bytes = emit_case(&case);
            let decoded = decode(&bytes);
            out.insert(case.name.to_string(), serialize_decode(&decoded));
        }
        out
    }

    /// Decoder self-test on hand-crafted streams: proves the decoder actually
    /// decodes (the pin comparison is not vacuous).
    #[test]
    fn test_flush_decoder_self_test() {
        // Plain text at the origin.
        let cells = decode(b"\x1b[1;1H\x1b[mABC");
        assert_eq!(cells.len(), 3);
        assert_eq!(cells[&0].ch, 'A');
        assert_eq!(cells[&0].fg, DecColor::Default);
        assert!(!cells[&2].bold);

        // CUP with explicit row/col (1-based) and default SGR reset.
        let cells = decode(b"\x1b[5;8H\x1b[mX");
        assert_eq!(cells[&7].ch, 'X');

        // Indexed and RGB colors, fg and bg.
        let cells = decode(b"\x1b[m\x1b[38;5;196mA\x1b[48;2;10;20;30mB\x1b[49mC");
        assert_eq!(cells[&0].fg, DecColor::Index(196));
        assert_eq!(cells[&0].bg, DecColor::Default);
        assert_eq!(cells[&1].fg, DecColor::Index(196));
        assert_eq!(cells[&1].bg, DecColor::Rgb(10, 20, 30));
        assert_eq!(cells[&2].bg, DecColor::Default);

        // Attribute transitions, bold off via 22, italic off via 23.
        let cells = decode(b"\x1b[m\x1b[1m\x1b[3mA\x1b[22m\x1b[23mB");
        assert!(cells[&0].bold && cells[&0].italics);
        assert!(!cells[&1].bold && !cells[&1].italics);

        // Underline styles: 4, 4:3, 4:0, 24.
        let cells = decode(b"\x1b[m\x1b[4mA\x1b[4:3mB\x1b[4:0mC\x1b[4:3m\x1b[24mD");
        assert!(cells[&0].underline && !cells[&0].undercurl);
        assert!(cells[&1].underline && cells[&1].undercurl);
        assert!(!cells[&2].underline && !cells[&2].undercurl);
        assert!(!cells[&3].underline && !cells[&3].undercurl);

        // OSC 8 spans, id form and close form.
        let cells = decode(b"\x1b[m\x1b]8;id=7;https://x/y\x07L\x1b]8;;\x07R");
        assert_eq!(cells[&0].uri.as_deref(), Some("https://x/y"));
        assert_eq!(cells[&1].uri, None);

        // U+FE0E marks the previous cell as force_text.
        let cells = decode("\x1b[mE\u{fe0e}F".as_bytes());
        assert!(cells[&0].force_text);
        assert!(!cells[&1].force_text);

        // Wide char advances the cursor by two columns.
        let cells = decode("\x1b[m\u{4e2d}z".as_bytes());
        assert_eq!(cells[&0].ch, '\u{4e2d}');
        assert!(!cells.contains_key(&1));
        assert_eq!(cells[&2].ch, 'z');
    }

    /// Regression (theme-picker left-edge bleed): an overlay paints a
    /// non-empty cell (a dialog's left frame border) over what the
    /// underlying grid had marked as a wide char's continuation cell. The
    /// emitter must re-anchor the cursor before that cell - the preceding
    /// wide char advances the terminal cursor by two, so without an
    /// explicit `MoveTo` the border glyph lands one column right and the
    /// wide char's right half stays visible at the dialog's edge.
    #[test]
    fn overlay_cell_after_wide_char_reanchors() {
        let mut grid = plain_grid();
        grid[(0, 0)] = styled('A', Color::Default, Color::Default, Attr::DEFAULT);
        grid[(1, 0)] = styled('\u{4e2d}', Color::Red, Color::Default, Attr::DEFAULT);
        // The dialog border painted over 中's continuation cell.
        grid[(2, 0)] = styled('|', Color::Green, Color::Default, Attr::DEFAULT);
        grid[(3, 0)] = styled('B', Color::Default, Color::Default, Attr::DEFAULT);

        for no_color in [false, true] {
            let sink = SharedBuf::default();
            let mut stdout: StateStdout =
                BufWriter::with_capacity(1024, Box::new(sink.clone()) as Box<dyn std::io::Write>);
            if no_color {
                Screen::<Tty>::draw_horizontal_segment_no_color(&mut grid, &mut stdout, 0..4, 0);
            } else {
                Screen::<Tty>::draw_horizontal_segment(&mut grid, &mut stdout, 0..4, 0);
            }
            drop(stdout);
            let cells = decode(&sink.into_inner());
            assert_eq!(cells[&0].ch, 'A');
            assert_eq!(cells[&1].ch, '\u{4e2d}');
            assert_eq!(
                cells[&2].ch, '|',
                "overlay cell must land on its grid column (no_color={no_color})"
            );
            assert_eq!(cells[&3].ch, 'B', "(no_color={no_color})");
        }
    }

    /// East-Asian Ambiguous characters (here `’`, U+2019) are laid out
    /// two columns wide by `CellBuffer::write_string` (`width_cjk`) with
    /// an `empty` continuation cell, but terminals render them as one OR
    /// two columns. The emitter must pin every glyph following such a
    /// cluster to its grid column: on a narrow-rendering terminal the
    /// natural cursor advance comes up one short and the rest of the row -
    /// dialog borders included - shifts one column left, which reads as
    /// underlying text inserted into the dialog.
    #[test]
    fn glyph_after_ambiguous_cluster_reanchors() {
        let mut grid = plain_grid();
        grid[(0, 0)] = styled('A', Color::Default, Color::Default, Attr::DEFAULT);
        // `’` plus the continuation cell write_string would produce.
        grid[(1, 0)] = styled('\u{2019}', Color::Red, Color::Default, Attr::DEFAULT);
        grid[(2, 0)] = continuation(Color::Red, Color::Default, Attr::DEFAULT);
        grid[(3, 0)] = styled('r', Color::Default, Color::Default, Attr::DEFAULT);
        // The dialog border painted over the cluster's continuation.
        grid[(4, 0)] = styled('|', Color::Green, Color::Default, Attr::DEFAULT);

        for no_color in [false, true] {
            let sink = SharedBuf::default();
            let mut stdout: StateStdout =
                BufWriter::with_capacity(1024, Box::new(sink.clone()) as Box<dyn std::io::Write>);
            if no_color {
                Screen::<Tty>::draw_horizontal_segment_no_color(&mut grid, &mut stdout, 0..5, 0);
            } else {
                Screen::<Tty>::draw_horizontal_segment(&mut grid, &mut stdout, 0..5, 0);
            }
            stdout.flush().unwrap();
            drop(stdout);
            let cells = decode(&sink.into_inner());

            assert_eq!(cells[&0].ch, 'A');
            assert_eq!(cells[&1].ch, '\u{2019}');
            // The continuation column is explicitly covered with a space
            // (no stale glyph can persist there on narrow terminals).
            assert_eq!(cells[&2].ch, ' ', "(no_color={no_color})");
            // `r` and the border must sit on their grid columns even when
            // the terminal rendered `’` one column narrow.
            assert_eq!(cells[&3].ch, 'r', "(no_color={no_color})");
            assert_eq!(cells[&4].ch, '|', "(no_color={no_color})");
        }
    }

    /// Regression: a wide char whose continuation cell an overlay painted
    /// over, with that overlay cell as the *last* cell of the segment. The
    /// walk back does not cross any `empty` cell (`px == x - 1`), so the
    /// emitter must only `MoveToColumn` to pin the overlay. Writing the
    /// covering space would land one column *past* the segment, leaving a
    /// space in the wide char's color over the neighbouring pane and eating
    /// one of its columns.
    #[test]
    fn overlay_last_cell_after_wide_char_does_not_spill_past_segment() {
        let mut grid = plain_grid();
        grid[(0, 0)] = styled('A', Color::Default, Color::Default, Attr::DEFAULT);
        grid[(1, 0)] = styled('\u{4e2d}', Color::Red, Color::Default, Attr::DEFAULT);
        // The dialog border painted directly over 中's continuation cell.
        grid[(2, 0)] = styled('|', Color::Red, Color::Default, Attr::DEFAULT);

        for no_color in [false, true] {
            let sink = SharedBuf::default();
            let mut stdout: StateStdout =
                BufWriter::with_capacity(1024, Box::new(sink.clone()) as Box<dyn std::io::Write>);
            if no_color {
                Screen::<Tty>::draw_horizontal_segment_no_color(&mut grid, &mut stdout, 0..3, 0);
            } else {
                Screen::<Tty>::draw_horizontal_segment(&mut grid, &mut stdout, 0..3, 0);
            }
            stdout.flush().unwrap();
            drop(stdout);
            let cells = decode(&sink.into_inner());
            assert_eq!(cells[&0].ch, 'A', "(no_color={no_color})");
            assert_eq!(cells[&1].ch, '\u{4e2d}', "(no_color={no_color})");
            assert_eq!(cells[&2].ch, '|', "(no_color={no_color})");
            // The column just past the segment (the wide char's second half
            // as the terminal cursor sees it) must not receive a covering
            // space.
            assert!(
                !cells.contains_key(&3),
                "covering space spilled past the segment (no_color={no_color}): {cells:?}"
            );
        }
    }

    /// Regression guard for the original fix: when the walk back really does
    /// cross a skipped `empty` continuation cell, the covering space must
    /// still be written before the overlay border is pinned.
    #[test]
    fn overlay_after_skipped_continuation_still_covers() {
        let mut grid = plain_grid();
        grid[(0, 0)] = styled('A', Color::Default, Color::Default, Attr::DEFAULT);
        // East-Asian Ambiguous `’` plus the continuation cell
        // `write_string` would produce.
        grid[(1, 0)] = styled('\u{2019}', Color::Red, Color::Default, Attr::DEFAULT);
        grid[(2, 0)] = continuation(Color::Red, Color::Default, Attr::DEFAULT);
        // The dialog border painted after the skipped continuation.
        grid[(3, 0)] = styled('|', Color::Green, Color::Default, Attr::DEFAULT);
        grid[(4, 0)] = styled('B', Color::Default, Color::Default, Attr::DEFAULT);

        for no_color in [false, true] {
            let sink = SharedBuf::default();
            let mut stdout: StateStdout =
                BufWriter::with_capacity(1024, Box::new(sink.clone()) as Box<dyn std::io::Write>);
            if no_color {
                Screen::<Tty>::draw_horizontal_segment_no_color(&mut grid, &mut stdout, 0..5, 0);
            } else {
                Screen::<Tty>::draw_horizontal_segment(&mut grid, &mut stdout, 0..5, 0);
            }
            stdout.flush().unwrap();
            drop(stdout);
            let cells = decode(&sink.into_inner());
            assert_eq!(cells[&0].ch, 'A', "(no_color={no_color})");
            assert_eq!(cells[&1].ch, '\u{2019}', "(no_color={no_color})");
            // The skipped continuation column is explicitly covered.
            assert_eq!(cells[&2].ch, ' ', "(no_color={no_color})");
            assert_eq!(cells[&3].ch, '|', "(no_color={no_color})");
            assert_eq!(cells[&4].ch, 'B', "(no_color={no_color})");
        }
    }

    /// Compare the CURRENT emitter's decode against the pins recorded from
    /// the OLD emitter. With `MELI_UPDATE_FLUSH_PINS=1` the
    /// pins file is (re)written from the current emitter instead.
    #[test]
    fn test_flush_semantic_equivalence_pinned() {
        let current = run_corpus();
        assert!(!current.is_empty());

        if std::env::var_os("MELI_UPDATE_FLUSH_PINS").is_some_and(|v| !v.is_empty()) {
            let mut content = String::new();
            content.push_str(
                "# meli flush byte-layer semantic-equivalence pins (decoded per-cell state).\n",
            );
            content.push_str("# Recorded from the emitter in use before the crossterm swap.\n");
            content.push_str(
                "# Regenerate (ONLY with deliberate intent) via MELI_UPDATE_FLUSH_PINS=1.\n",
            );
            for (name, ser) in &current {
                content.push_str(&format!("case {name}\n"));
                if !ser.is_empty() {
                    content.push_str(ser);
                    content.push('\n');
                }
                content.push_str("end\n");
            }
            std::fs::write(pins_path(), content).expect("write flush equivalence pins");
            println!(
                "recorded {} pins to {}",
                current.len(),
                pins_path().display()
            );
            return;
        }

        let pins_raw = std::fs::read_to_string(pins_path()).unwrap_or_else(|_| {
            panic!(
                "pins file missing at {}; record it with MELI_UPDATE_FLUSH_PINS=1",
                pins_path().display()
            )
        });
        let pins = parse_pins(&pins_raw);
        assert_eq!(
            pins.len(),
            current.len(),
            "pin file must cover the whole corpus"
        );
        let mut failures = Vec::new();
        for (name, expected) in &pins {
            let got = current
                .get(name)
                .unwrap_or_else(|| panic!("case {name} missing from corpus"));
            if expected != got {
                failures.push(format!(
                    "case {name} decode diverged:\n--- pinned (old emitter) ---\n{expected}\n--- current emitter ---\n{got}\n",
                ));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
