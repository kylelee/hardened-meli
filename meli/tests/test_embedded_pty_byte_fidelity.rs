//
// meli
//
// Copyright 2026 Emmanouil Pitsidianakis <manos@pitsidianak.is>
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
// along with meli.  If not, see <http://www.gnu.org/licenses/>.
//
// SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later

//! End-to-end byte-fidelity verification of the embedded-pty path
//! (`UIMode::Embedded`), now that the raw input bytes handed to the
//! embedded terminal are produced by
//! [`encode_key`](meli::terminal::ratatui_bridge::encode_key).
//!
//! This file covers the public-API layer against a deterministic scripted
//! child (a `printf`/`dd` POSIX shell "editor"): `create_pty` spawns the
//! child, which emits a fixed VT stream (SGR colors, cursor addressing, a
//! line wrap, plus a malformed-garbage burst). The rendered [`EmbeddedGrid`]
//! cells are asserted symbol/color-by-color. Keystrokes are then written
//! through [`Terminal`]'s `io::Write` impl (the same path
//! `UIEvent::EmbeddedInput` uses in `Composer`) while the child has
//! enabled DECCKM, proving the CSI-arrow to SS3 translation, and the
//! child's stdin log must equal the expected byte stream exactly.
//!
//! The `Composer`-level end-to-end (`UIEvent::EmbeddedInput` forwarding,
//! Ctrl-z stop/resume round-trip, sentinel body update) lives next to the
//! `Composer` tests in `meli/src/mail/compose.rs` because it needs the
//! crate-internal mock `Context`.
//!
//! Determinism: the child sets `stty raw -echo` before anything else, so
//! neither the tty line discipline (ERASE/ICRNL/ECHO) nor timing affects
//! the byte stream; every wait is a bounded poll on an observable state
//! change (a marker rendered in the grid), never a fixed sleep.

use std::{
    io::Write as _,
    os::unix::fs::PermissionsExt,
    path::Path,
    time::{Duration, Instant},
};

use meli::{
    terminal::{embedded::create_pty, ratatui_bridge::encode_key},
    Attr, Cell, CellBuffer, Color, Key,
};

/// Bounded spin on `pred` until it holds or the deadline passes.
///
/// Yields between attempts (and parks briefly after the first thousand
/// spins) so the pty reader thread and the child process are never
/// starved on a busy single core. Returns whether the condition was
/// eventually observed.
fn wait_until(deadline: Duration, mut pred: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    let mut spins = 0;
    loop {
        if pred() {
            return true;
        }
        assert!(
            start.elapsed() < deadline,
            "condition not reached within {deadline:?} (wait started at {start:?})"
        );
        spins += 1;
        if spins < 1_000 {
            std::thread::yield_now();
        } else {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

/// Write `contents` to `path` and mark it executable.
fn write_exec(path: &Path, contents: &str) {
    std::fs::write(path, contents).unwrap_or_else(|err| {
        panic!("could not write {}: {err}", path.display());
    });
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap_or_else(|err| {
        panic!("could not chmod {}: {err}", path.display());
    });
}

/// Does `grid` contain `needle` as a horizontal run of symbols?
fn grid_contains(grid: &CellBuffer, needle: &str) -> bool {
    let needle: Vec<char> = needle.chars().collect();
    'rows: for y in 0..grid.rows {
        let mut window: Vec<char> = Vec::with_capacity(needle.len());
        for x in 0..grid.cols {
            let Some(cell) = grid.get(x, y) else {
                continue 'rows;
            };
            window.push(cell.ch());
            if window.len() > needle.len() {
                window.remove(0);
            }
            if window == needle {
                return true;
            }
        }
    }
    false
}

fn cell(grid: &CellBuffer, x: usize, y: usize) -> Cell {
    *grid
        .get(x, y)
        .unwrap_or_else(|| panic!("cell ({x},{y}) out of bounds"))
}

fn cell_ch_is(grid: &CellBuffer, x: usize, y: usize, ch: char) -> bool {
    cell(grid, x, y).ch() == ch
}

// ----------------------------------------------------------------------------
// Scripted children.
// ----------------------------------------------------------------------------

/// Direct-child shim for layer 1: emits a fixed VT stream (garbage burst,
/// reset/clear, pinned content with colors, cursor addressing, a 90-column
/// wrap line), enables DECCKM (application cursor keys), prints `READY`,
/// logs exactly `count` stdin bytes to `$1`, then prints `DONE2` and exits.
fn vt_script(count: usize) -> String {
    format!(
        r#"#!/bin/sh
# meli embedded-pty byte-fidelity child (generated by a test; do not edit).
LOG="$1"
COUNT={count}
stty raw -echo
# -- malformed/garbage burst: unknown DEC private modes, unknown CSI final,
#    an unknown escape, a partial OSC, stray continuation bytes, a valid
#    2-byte UTF-8 char, BEL, a grouped SGR. None of these may panic the
#    grid parser, and all output below is cleared before the pinned content.
printf '\033[?777h\033[?9999z\033[12~\033Q\033[=p\033]0;osc-partial'
printf '\200\237\303\251\007\033[1;3;4;5;7m'
# -- reset, clear, pinned content --
printf '\033[0m\033[2J\033[H'
printf 'SIM-EDITOR v1\r\n'
printf '\033[31mRED\033[0m \033[39m\033[1;44mBOLD-ON-BLUE\033[39;49m\r\n'
printf '\033[38;5;196mIDX196\033[39m\r\n'
printf '\033[4;10HCUP-MARK\r\n'
printf '{w90}\r\n'
printf '\033[?1hREADY'
dd bs=1 count="$COUNT" of="$LOG" 2>/dev/null
printf '\033[?1lDONE2'
exit 0
"#,
        w90 = "W".repeat(90),
    )
}

// ----------------------------------------------------------------------------
// Layer 1: create_pty + EmbeddedGrid rendering + Terminal::write bytes.
// ----------------------------------------------------------------------------

#[test]
fn embedded_pty_grid_renders_scripted_vt_output_and_forwards_keys() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("vt-byte.log");
    let script = tmp.path().join("vt-child.sh");

    // `create_pty` parses the raw stdout of `getconf PATH` without
    // stripping its trailing `\n`. On hosts where that output is a single
    // component (Fedora/Arch: `/usr/bin`), every candidate `sh` path
    // contains the newline and fails `exists()`, so `create_pty` refuses
    // to spawn. Pre-existing upstream defect in `terminal/embedded.rs`
    // (zero diff vs main; see the task-6 evidence log), out of this
    // todo's no-product-code mandate. Prepend a shim `getconf` printing
    // the same PATH without the newline - the equivalent of a
    // multi-component `getconf PATH` host (Debian), where the lookup
    // succeeds. Only the `sh` lookup is affected; no bytes are altered.
    let shimbin = tmp.path().join("bin");
    std::fs::create_dir_all(&shimbin).unwrap();
    write_exec(&shimbin.join("getconf"), "#!/bin/sh\nprintf '/usr/bin'\n");
    let previous_path = std::env::var_os("PATH").unwrap_or_default();
    std::env::set_var(
        "PATH",
        format!("{}:{}", shimbin.display(), previous_path.to_string_lossy()),
    );

    // Key matrix with the byte stream each key must produce on the child's
    // stdin. DECCKM is on (`CSI ? 1 h`), so the CSI cursor keys are
    // translated to their SS3 application forms by `Terminal::write`
    // (meli/src/terminal/embedded/terminal.rs); plain keys must match
    // `encode_key` verbatim.
    let matrix: &[(Key, &[u8])] = &[
        (Key::Char('h'), b"h"),
        (Key::Char('j'), b"j"),
        (Key::Char('k'), b"k"),
        (Key::Char('l'), b"l"),
        (Key::Left, b"\x1bOD"),
        (Key::Down, b"\x1bOB"),
        (Key::Up, b"\x1bOA"),
        (Key::Right, b"\x1bOC"),
        (Key::Home, b"\x1bOH"),
        (Key::End, b"\x1bOF"),
        (Key::PageUp, b"\x1b[5~"),
        (Key::PageDown, b"\x1b[6~"),
        (Key::Delete, b"\x1b[3~"),
        (Key::Insert, b"\x1b[2~"),
        (Key::F(1), b"\x1bOP"),
        (Key::F(5), b"\x1b[15~"),
        (Key::Ctrl('['), b"\x1b"),
        (Key::Alt('x'), b"\x1bx"),
        (Key::Char('\n'), b"\r"),
        // Exit trigger; dd stops after consuming it.
        (Key::Ctrl('x'), b"\x18"),
    ];
    let expected: Vec<u8> = matrix
        .iter()
        .flat_map(|(_, bytes)| bytes.iter().copied())
        .collect();
    // Independent cross-check: everything that is not a DECCKM-translated
    // cursor key must equal the `encode_key` table byte for byte.
    for (key, bytes) in matrix {
        if !matches!(
            key,
            Key::Left | Key::Down | Key::Up | Key::Right | Key::Home | Key::End
        ) {
            assert_eq!(&encode_key(key), bytes, "encode_key({key:?}) drifted");
        }
    }

    write_exec(&script, &vt_script(expected.len()));
    let command = format!("{} {}", script.display(), log.display());
    let pty = create_pty(80, 20, &command).expect("create_pty spawns the scripted child");

    // The grid renders the pinned content; poll for the READY marker (the
    // child only prints it after `stty raw -echo` took effect, i.e. after
    // it is safe to write keystrokes without the line discipline
    // mangling them).
    assert!(
        wait_until(Duration::from_secs(30), || {
            let guard = pty.lock().unwrap();
            grid_contains(guard.grid.buffer(), "READY")
        }),
        "READY marker never rendered; grid dump:\n{}",
        {
            let guard = pty.lock().unwrap();
            dump_grid(guard.grid.buffer())
        }
    );
    {
        let guard = pty.lock().unwrap();
        let grid = guard.grid.buffer();
        assert_eq!(guard.grid.terminal_size(), (80, 20));

        // Row 0: plain text, default colors.
        for (x, ch) in "SIM-EDITOR v1".chars().enumerate() {
            let c = cell(grid, x, 0);
            assert_eq!(c.ch(), ch, "plain text at ({x},0)");
            assert_eq!(c.fg(), Color::Default);
            assert_eq!(c.bg(), Color::Default);
        }
        // Row 1: SGR fg color, then bold-on-blue background run.
        // meli's embedded VT parser treats SGR 0 as an ATTRIBUTE-only
        // reset (colors need SGR 39/49), so the space after `RED\e[0m`
        // still carries the red foreground - pinned here as-is.
        for (x, ch) in "RED".chars().enumerate() {
            let c = cell(grid, x, 1);
            assert_eq!(c.ch(), ch, "RED run at ({x},1)");
            assert_eq!(c.fg(), Color::Red, "RED fg at ({x},1)");
            assert_eq!(c.bg(), Color::Default);
        }
        assert!(cell_ch_is(grid, 3, 1, ' '));
        assert_eq!(
            cell(grid, 3, 1).fg(),
            Color::Red,
            "SGR 0 is an attrs-only reset in this parser; fg stays Red"
        );
        for (x, ch) in "BOLD-ON-BLUE".chars().enumerate() {
            let c = cell(grid, x + 4, 1);
            assert_eq!(c.ch(), ch, "BOLD-ON-BLUE at ({},1)", x + 4);
            assert_eq!(c.fg(), Color::Default);
            assert_eq!(c.bg(), Color::Blue, "BOLD-ON-BLUE bg at ({},1)", x + 4);
            assert!(c.attrs().contains(Attr::BOLD), "bold at ({},1)", x + 4);
        }
        // Row 2: 256-color SGR (38;5;196).
        for (x, ch) in "IDX196".chars().enumerate() {
            let c = cell(grid, x, 2);
            assert_eq!(c.ch(), ch, "IDX196 at ({x},2)");
            assert_eq!(c.fg(), Color::Byte(196), "IDX196 fg at ({x},2)");
        }
        // Row 3: CUP (CSI 4 ; 10 H) positions the run at column 9.
        for x in 0..9 {
            assert!(cell_ch_is(grid, x, 3, ' '), "blank before CUP at ({x},3)");
        }
        for (x, ch) in "CUP-MARK".chars().enumerate() {
            assert!(cell_ch_is(grid, x + 9, 3, ch), "CUP-MARK at ({},3)", x + 9);
        }
        // Rows 4/5: a 90-column run wraps at the 80-column boundary.
        for x in 0..80 {
            assert!(cell_ch_is(grid, x, 4, 'W'), "wrap row0 at ({x},4)");
        }
        for x in 0..10 {
            assert!(cell_ch_is(grid, x, 5, 'W'), "wrap row1 at ({x},5)");
        }
        assert!(cell_ch_is(grid, 10, 5, ' '), "wrap stops after 10 cells");
    }

    // Feed the whole matrix through the `io::Write` impl on `Terminal`
    // (the exact call `Composer` makes for `UIEvent::EmbeddedInput`).
    for (key, _) in matrix {
        let bytes = encode_key(key);
        pty.lock()
            .unwrap()
            .write_all(&bytes)
            .unwrap_or_else(|err| panic!("write_all({key:?}) failed: {err}"));
    }

    // The child logs exactly `expected.len()` bytes, then prints DONE2.
    assert!(
        wait_until(Duration::from_secs(30), || {
            let guard = pty.lock().unwrap();
            grid_contains(guard.grid.buffer(), "DONE2")
        }),
        "DONE2 marker never rendered; grid dump:\n{}",
        {
            let guard = pty.lock().unwrap();
            dump_grid(guard.grid.buffer())
        }
    );

    let got = std::fs::read(&log).unwrap_or_else(|err| panic!("read {}: {err}", log.display()));
    assert_eq!(
        got, expected,
        "child stdin log differs from the expected byte stream"
    );
}

// ----------------------------------------------------------------------------
// Shared helpers.
// ----------------------------------------------------------------------------

/// Human-readable dump of a grid for failure messages.
fn dump_grid(grid: &CellBuffer) -> String {
    let mut out = String::new();
    for y in 0..grid.rows {
        for x in 0..grid.cols {
            out.push(cell(grid, x, y).ch());
        }
        out.push('\n');
    }
    out
}
