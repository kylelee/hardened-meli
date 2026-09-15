/*
 * meli
 *
 * Copyright 2026 Manos Pitsidianakis
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
 *
 * SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later
 */

//! Golden-snapshot characterization corpus.
//!
//! This module pins the pre-migration rendering of meli's major UI
//! surfaces to golden files so that the terminal I/O migration
//! (crossterm/ratatui) can prove that pixels did not change.
//!
//! Each test drives a component to draw into a [`Screen<Virtual>`] via the
//! mock `Context::new_mock` machinery (modeled on the focus tests in
//! `meli/src/mail/view/thread.rs` and `listing_menu_tests` in
//! `meli/src/mail/listing.rs`), serializes the resulting [`CellBuffer`]
//! per-cell (symbol/fg/bg/attrs/empty) with run-length encoding per row, and
//! asserts byte equality against a committed golden file.
//!
//! Environment switches:
//!
//! - `MELI_UPDATE_GOLDEN=1`: re-record mode; writes the serialized buffer to
//!   the golden directory instead of asserting.
//! - `MELI_GOLDEN_DIR=<path>`: overrides where goldens are read from/written
//!   to (default: `meli/tests/golden`), so a re-record can target a scratch
//!   directory and be diffed against the committed corpus.
//!
//! Determinism contract: no wall-clock, no randomness, no env-dependent
//! output. Timezone-dependent rendering is pinned by forcing `TZ=UTC` (see
//! [`init_determinism`]); fixed envelope dates are chosen older than the
//! `recent_dates` relative window so the listing date format is absolute.
//! The Composer's wall-clock `Date` draft header is pinned before its first
//! lazy `draw` initialization (see `meli/src/mail/compose.rs::tests`).

use std::{fmt::Write as _, path::PathBuf, sync::OnceLock};

use melib::{
    backends::{
        AccountHash, BackendMailbox, Mailbox, MailboxHash, MailboxPermissions, SpecialUsageMailbox,
    },
    Result,
};

use crate::{
    components::{Component, ComponentId, PageMovement},
    conf,
    contacts::list::ContactList,
    mail::listing::{
        CompactListing, ConversationsListing, Listing, ListingTrait, PlainListing, ThreadListing,
    },
    mail::view::{ThreadView, ThreadViewFocus},
    terminal::{Area, Attr, CellBuffer, Color, Screen, Virtual},
    types::UIEvent,
    utilities::{Pager, Selector, StatusBar, Tabbed, UIConfirmationDialog},
    Context, Envelope, Flag, IndexStyle, Key, Mail, ThemeAttribute,
};

// POSIX `tzset()`: re-read the `TZ` environment variable into libc's cached
// timezone state. The `libc` crate does not export this symbol, so it is
// declared here.
extern "C" {
    fn tzset();
}

/// Pin process-global time state to UTC once, before any golden rendering.
///
/// `CellBuffer` content includes dates rendered by
/// `melib::utils::datetime::timestamp_to_string`, which uses `localtime_r`.
/// `localtime_r` does not reliably re-read `TZ` on its own, so `tzset()` is
/// called explicitly after setting the variable.
pub fn init_determinism() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        // SAFETY: this only mutates process-global libc time state, which is
        // not thread-safe in general but is set to the same constant value by
        // every caller before any golden rendering happens.
        std::env::set_var("TZ", "UTC");
        unsafe {
            tzset();
        }
    });
}

/// Serialize a [`CellBuffer`] into the stable golden text format.
///
/// Format:
///
/// ```text
/// meli-golden v1
/// case=<name>
/// size=<cols>x<rows>
/// row<y> <count>x<ch>/<fg>/<bg>/0x<attr-bits>/<empty> ...
/// ```
///
/// Each `row` line contains run-length encoded runs of identical cells; the
/// cell attributes recorded are exactly the ones the terminal flush layer
/// reads: symbol, foreground, background, attribute bits and the wide-char
/// `empty` continuation flag.
pub fn serialize_buffer(name: &str, buf: &CellBuffer) -> String {
    init_determinism();
    let mut out = String::new();
    let _ = writeln!(out, "meli-golden v1");
    let _ = writeln!(out, "case={name}");
    let _ = writeln!(out, "size={}x{}", buf.cols, buf.rows);
    for y in 0..buf.rows {
        let mut line = format!("row{y}");
        let mut x = 0;
        while x < buf.cols {
            let cell = *buf.get(x, y).expect("in-bounds cell");
            let mut n = 1;
            while x + n < buf.cols && buf.get(x + n, y) == Some(&cell) {
                n += 1;
            }
            line.push(' ');
            let _ = write!(
                line,
                "{n}x{}/{}/{}/0x{:04x}/{}",
                ch_token(cell.ch()),
                color_token(cell.fg()),
                color_token(cell.bg()),
                cell.attrs().bits(),
                u8::from(cell.empty()),
            );
            x += n;
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Encode a cell symbol: printable ASCII except the `/` separator is emitted
/// literally, everything else as a `U+XXXX` escape (spaces included, so runs
/// are unambiguous).
fn ch_token(c: char) -> String {
    if c.is_ascii_graphic() && c != '/' && c != '\\' {
        c.to_string()
    } else {
        format!("U+{:04X}", u32::from(c))
    }
}

/// Encode a [`Color`] canonically.
fn color_token(c: Color) -> String {
    match c {
        Color::Default => "default".to_string(),
        Color::Black => "black".to_string(),
        Color::Red => "red".to_string(),
        Color::Green => "green".to_string(),
        Color::Yellow => "yellow".to_string(),
        Color::Blue => "blue".to_string(),
        Color::Magenta => "magenta".to_string(),
        Color::Cyan => "cyan".to_string(),
        Color::White => "white".to_string(),
        Color::Byte(b) => format!("byte{b}"),
        Color::Rgb(r, g, b) => format!("rgb({r},{g},{b})"),
    }
}

/// Where golden files live. `MELI_GOLDEN_DIR` overrides the default
/// in-tree `meli/tests/golden` directory.
fn golden_dir() -> PathBuf {
    match std::env::var_os("MELI_GOLDEN_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden")),
    }
}

/// Whether this run records goldens instead of asserting them.
fn update_mode() -> bool {
    std::env::var_os("MELI_UPDATE_GOLDEN").is_some_and(|v| v != "0" && !v.is_empty())
}

/// Record (in `MELI_UPDATE_GOLDEN=1` mode) or verify the golden for `name`.
///
/// Verification fails with the first differing serialized row so a regression
/// report points directly at the affected screen row.
pub fn record_or_assert(name: &str, grid: &CellBuffer) {
    let dir = golden_dir();
    let path = dir.join(format!("{name}.golden"));
    let actual = serialize_buffer(name, grid);
    if update_mode() {
        std::fs::create_dir_all(&dir)
            .unwrap_or_else(|err| panic!("could not create golden dir {}: {err}", dir.display()));
        std::fs::write(&path, actual)
            .unwrap_or_else(|err| panic!("could not write {}: {err}", path.display()));
        eprintln!("[golden] recorded {name} -> {}", path.display());
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "golden file {} could not be read ({err}); record it with \
             MELI_UPDATE_GOLDEN=1",
            path.display()
        )
    });
    if expected != actual {
        let mut msg = format!("golden mismatch for {name} ({})\n", path.display());
        let mut diff_found = false;
        for (i, (le, la)) in expected.lines().zip(actual.lines()).enumerate() {
            if le != la {
                msg.push_str(&format!(
                    "  first differing line {} (1-based):\n    expected: {le}\n    actual:   \
                     {la}\n",
                    i + 1
                ));
                diff_found = true;
                break;
            }
        }
        if !diff_found {
            msg.push_str(&format!(
                "  line counts differ: expected {} lines, actual {} lines\n",
                expected.lines().count(),
                actual.lines().count()
            ));
        }
        panic!("{}", msg.trim_end());
    }
    eprintln!("[golden] verified {name} ({})", path.display());
}

// ----------------------------------------------------------------------------
// Mock context helpers (modeled on `listing_menu_tests` in
// meli/src/mail/listing.rs).
// ----------------------------------------------------------------------------

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

    // Each mailbox must have a distinct path: `build_mailboxes_order`
    // sorts by path, so a shared hardcoded path would make both entries
    // compare equal and leave the INBOX-first detection ambiguous.
    fn path(&self) -> &str {
        &self.name
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

pub fn mock_context() -> Context {
    use crate::conf::composing::SendMail;

    init_determinism();
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

/// Register two mailboxes (`INBOX`, `Archive`) on the mock account and
/// rebuild the account's mailbox tree/order.
///
/// `Listing::new` snapshots the account entries via `list_mailboxes`, so
/// the mailboxes must be registered before it is called. The
/// INBOX-first comparator in `build_mailboxes_order` guarantees the order
/// `INBOX` (index 0), `Archive` (index 1).
fn register_two_mailboxes(context: &mut Context) -> (AccountHash, MailboxHash, MailboxHash) {
    use crate::{
        accounts::{build_mailboxes_order, MailboxEntry, MailboxStatus},
        conf::FileMailboxConf,
    };

    let account_hash = *context.accounts.iter().next().unwrap().0;
    let account = context.accounts.get_mut(&account_hash).unwrap();
    for name in ["INBOX", "Archive"] {
        let mailbox_hash = MailboxHash::from_bytes(name.as_bytes());
        account.mailbox_entries.insert(
            mailbox_hash,
            MailboxEntry::new(
                MailboxStatus::Available,
                name.to_string(),
                Box::new(TestMailbox {
                    hash: mailbox_hash,
                    name: name.to_string(),
                }),
                FileMailboxConf::default(),
            ),
        );
    }
    build_mailboxes_order(
        &mut account.tree,
        &account.mailbox_entries,
        &mut account.mailboxes_order,
    );
    (
        account_hash,
        MailboxHash::from_bytes(b"INBOX"),
        MailboxHash::from_bytes(b"Archive"),
    )
}

/// Raw bytes of the fixed golden-corpus mails. Dates are fixed and older
/// than the `recent_dates` relative window, so listings render them with the
/// absolute `%Y-%m-%d %T` format regardless of when the corpus runs.
const GOLDEN_ROOT_MAIL: &[u8] = b"From: Alice Example <alice@example.org>\r\n\
To: Bob Example <bob@example.org>\r\n\
Subject: golden thread root\r\n\
Message-ID: <golden-root@x.example>\r\n\
Date: Wed, 1 Jan 2025 10:00:00 +0000\r\n\
\r\n\
root body line\r\n";

const GOLDEN_REPLY_MAIL: &[u8] = b"From: Bob Example <bob@example.org>\r\n\
To: Alice Example <alice@example.org>\r\n\
Subject: Re: golden thread root\r\n\
Message-ID: <golden-reply@x.example>\r\n\
In-Reply-To: <golden-root@x.example>\r\n\
Date: Wed, 1 Jan 2025 11:30:00 +0000\r\n\
\r\n\
reply body line\r\n";

const GOLDEN_SOLO_MAIL: &[u8] = b"From: Carol Example <carol@example.org>\r\n\
To: Bob Example <bob@example.org>\r\n\
Subject: golden standalone mail\r\n\
Message-ID: <golden-solo@x.example>\r\n\
Date: Thu, 2 Jan 2025 09:30:00 +0000\r\n\
\r\n\
standalone body line\r\n";

/// Insert the three fixed golden-corpus mails (a two-mail thread plus a
/// standalone mail, the last one unseen) into `mailbox_hash`.
fn insert_golden_mails(context: &Context, mailbox_hash: MailboxHash) {
    let account_hash = *context.accounts.iter().next().unwrap().0;
    for (bytes, flags) in [
        (GOLDEN_ROOT_MAIL, Flag::SEEN),
        (GOLDEN_REPLY_MAIL, Flag::SEEN | Flag::REPLIED),
        // Unseen: exercises the bold/unseen row styling in listings.
        (GOLDEN_SOLO_MAIL, Flag::empty()),
    ] {
        let mut envelope = Envelope::from_bytes(bytes, None).expect("could not parse test mail");
        envelope.set_flags(flags);
        context.accounts[&account_hash]
            .collection
            .insert(envelope, mailbox_hash);
    }
}

/// Three more standalone SEEN mails on distinct dates, so the selection
/// golden cases render enough rows/blocks for zebra parity to be visible
/// below the cursor row.
const GOLDEN_SELECTION_MAILS: [&[u8]; 3] = [
    b"From: Dave Example <dave@example.org>\r\n\
To: Bob Example <bob@example.org>\r\n\
Subject: golden selection mail three\r\n\
Message-ID: <golden-sel-three@x.example>\r\n\
Date: Fri, 3 Jan 2025 08:15:00 +0000\r\n\
\r\n\
selection body three\r\n",
    b"From: Eve Example <eve@example.org>\r\n\
To: Bob Example <bob@example.org>\r\n\
Subject: golden selection mail four\r\n\
Message-ID: <golden-sel-four@x.example>\r\n\
Date: Sat, 4 Jan 2025 12:45:00 +0000\r\n\
\r\n\
selection body four\r\n",
    b"From: Frank Example <frank@example.org>\r\n\
To: Bob Example <bob@example.org>\r\n\
Subject: golden selection mail five\r\n\
Message-ID: <golden-sel-five@x.example>\r\n\
Date: Sun, 5 Jan 2025 18:20:00 +0000\r\n\
\r\n\
selection body five\r\n",
];

/// Insert [`GOLDEN_SELECTION_MAILS`] into `mailbox_hash`.
fn insert_selection_mails(context: &Context, mailbox_hash: MailboxHash) {
    let account_hash = *context.accounts.iter().next().unwrap().0;
    for bytes in GOLDEN_SELECTION_MAILS {
        let mut envelope = Envelope::from_bytes(bytes, None).expect("could not parse test mail");
        envelope.set_flags(Flag::SEEN);
        context.accounts[&account_hash]
            .collection
            .insert(envelope, mailbox_hash);
    }
}

/// Draw once, move the cursor one row down (drawing again so the movement
/// is applied), then toggle the entry selection under the new cursor
/// position and redraw. The final grid pins the selected+highlighted row
/// fill, the zebra parity base colors and the bold unseen-row styling in
/// one frame.
fn draw_selection_row_batch(
    listing: &mut dyn ListingTrait,
    grid: &mut CellBuffer,
    area: Area,
    context: &mut Context,
) {
    context.settings.shortcuts.listing.select_entry = Key::Char('V');
    context.settings.shortcuts.listing.scroll_down = Key::Down;
    listing.draw(grid, area, context);
    listing.set_movement(PageMovement::Down(1));
    listing.draw(grid, area, context);
    let mut event = UIEvent::Input(Key::Char('V'));
    assert!(
        listing.process_event(&mut event, context),
        "select_entry must be handled by the listing"
    );
    listing.draw(grid, area, context);
}

/// A fresh 80x24 virtual screen with the mock theme as the default cell,
/// matching how `State` initializes the real grid.
pub fn golden_screen(context: &Context, cols: usize, rows: usize) -> Screen<Virtual> {
    let mut screen = Screen::<Virtual>::new(conf::value(context, "theme_default"));
    assert!(
        screen.resize(cols, rows),
        "virtual screen resize to {cols}x{rows}"
    );
    screen
}

/// Feed all pending context replies into `component`, like the main event
/// loop does, until the reply queue settles.
fn pump_replies(component: &mut dyn Component, context: &mut Context) {
    for _ in 0..8 {
        let replies = context.replies();
        if replies.is_empty() {
            break;
        }
        for mut event in replies {
            let _ = component.process_event(&mut event, context);
        }
    }
}

// ----------------------------------------------------------------------------
// Corpus.
// ----------------------------------------------------------------------------

/// Full top-level frame as the real app builds it: `StatusBar` wrapping
/// `Tabbed(listing, contacts)` (see `State` wiring in `meli/src/main.rs`),
/// with the tab bar row, the listing surface (sidebar + compact rows) and
/// the status bar row all pinned in one frame.
#[test]
fn golden_tabbed_statusbar_full_frame() {
    let mut ctx = mock_context();
    let (_account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
    insert_golden_mails(&ctx, inbox_hash);

    let listing = Listing::new(&mut ctx);
    let tabbed = Tabbed::new(
        vec![Box::new(listing), Box::new(ContactList::new(&ctx))],
        &ctx,
    );
    let mut status_bar = StatusBar::new(&ctx, Box::new(tabbed));
    status_bar.realize(None, &mut ctx);
    pump_replies(&mut status_bar, &mut ctx);

    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    status_bar.draw(screen.grid_mut(), area, &mut ctx);
    record_or_assert("tabbed_statusbar_full_frame", screen.grid());
}

/// Insert only [`GOLDEN_SOLO_MAIL`] (seen) so the listing cursor rests on
/// the single standalone mail deterministically.
fn insert_solo_mail(context: &Context, mailbox_hash: MailboxHash) {
    let account_hash = *context.accounts.iter().next().unwrap().0;
    let mut envelope =
        Envelope::from_bytes(GOLDEN_SOLO_MAIL, None).expect("could not parse test mail");
    envelope.set_flags(Flag::SEEN);
    context.accounts[&account_hash]
        .collection
        .insert(envelope, mailbox_hash);
}

/// Insert only the two-mail golden thread (root + reply), so the listing
/// cursor rests on the thread row deterministically.
fn insert_thread_mails(context: &Context, mailbox_hash: MailboxHash) {
    let account_hash = *context.accounts.iter().next().unwrap().0;
    for (bytes, flags) in [
        (GOLDEN_ROOT_MAIL, Flag::SEEN),
        (GOLDEN_REPLY_MAIL, Flag::SEEN | Flag::REPLIED),
    ] {
        let mut envelope = Envelope::from_bytes(bytes, None).expect("could not parse test mail");
        envelope.set_flags(flags);
        context.accounts[&account_hash]
            .collection
            .insert(envelope, mailbox_hash);
    }
}

/// Drive `listing` to open the entry under the cursor via the pinned
/// `open_entry` shortcut, routing the `OpenEntryUnderCursor` `IntraComm`
/// back in like the main loop does so `Listing.view` is created, then
/// redraw.
fn open_entry_under_cursor(
    listing: &mut Listing,
    context: &mut Context,
    grid: &mut CellBuffer,
    area: Area,
) {
    context.settings.shortcuts.listing.open_entry = Key::Char('\n');
    listing.draw(grid, area, context);
    let mut event = UIEvent::Input(Key::Char('\n'));
    assert!(
        listing.process_event(&mut event, context),
        "open_entry must be handled by the listing"
    );
    pump_replies(listing, context);
    listing.draw(grid, area, context);
}

/// A standalone (single-mail thread) mail opened from the listing. While a
/// view is open the listing skips its own list-pane frame (it trusts the
/// `ThreadView` to frame itself), so this frame pins the embedded
/// `ThreadView` pane's rounded frame + focused border — the single-mail
/// fast path that historically drew the `mailview` frameless.
#[test]
fn golden_listing_open_solo_mail() {
    let mut ctx = mock_context();
    let (_account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
    insert_solo_mail(&ctx, inbox_hash);

    let mut listing = Listing::new(&mut ctx);
    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    open_entry_under_cursor(&mut listing, &mut ctx, screen.grid_mut(), area);
    record_or_assert("listing_open_solo_mail", screen.grid());
}

/// A two-mail thread opened from the listing: the split `ThreadView` draws
/// both panes' rounded frames itself (green guard for the divider column
/// and the listing's view-open frame skip).
#[test]
fn golden_listing_open_thread() {
    let mut ctx = mock_context();
    let (_account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
    insert_thread_mails(&ctx, inbox_hash);

    let mut listing = Listing::new(&mut ctx);
    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    open_entry_under_cursor(&mut listing, &mut ctx, screen.grid_mut(), area);
    record_or_assert("listing_open_thread", screen.grid());
}

/// Conversations style with the entry open (Right/Enter): while a view is
/// open the listing skips its own list-pane frame, and the conversation
/// list keeps rendering as a subpane in the left third — so that subpane
/// must draw its own rounded frame, unfocused-styled because the keyboard
/// focus sits in the `ThreadView` (the convention `ThreadView`'s internal
/// panes follow). This case pins the three-frame layout: sidebar frame,
/// conversation-subpane frame, solo-mail `ThreadView` frame, plus the
/// subject rendering inside the subpane's inner area.
#[test]
fn golden_conversations_entry_split_frame() {
    let mut ctx = mock_context();
    ctx.settings.listing.index_style = IndexStyle::Conversations;
    let (_account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
    insert_solo_mail(&ctx, inbox_hash);

    let mut listing = Listing::new(&mut ctx);
    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    open_entry_under_cursor(&mut listing, &mut ctx, screen.grid_mut(), area);
    let grid = screen.grid();
    let tab_unfocused = conf::value(&ctx, "tab.unfocused");
    let tab_focused = conf::value(&ctx, "tab.focused");
    // Sidebar (8 cols) + divider at x=8, so the list pane starts at x=9
    // (71 cols). The conversation subpane takes the left third
    // (x=9..=31): its ring owns columns 9 and 31 and rows 0 and 23.
    assert_eq!(grid[(9, 0)].ch(), '╭', "subpane top-left ring corner");
    assert_eq!(grid[(31, 0)].ch(), '╮', "subpane top-right ring corner");
    assert_eq!(grid[(9, 23)].ch(), '╰', "subpane bottom-left ring corner");
    assert_eq!(grid[(31, 23)].ch(), '╯', "subpane bottom-right ring corner");
    assert_eq!(grid[(31, 5)].ch(), '│', "subpane right ring column");
    assert_eq!(
        grid[(31, 5)].fg(),
        tab_unfocused.fg,
        "subpane ring must use the unfocused attr (focus is in the ThreadView)"
    );
    // The solo-mail ThreadView frames the remaining columns (x=33..=79).
    assert_eq!(grid[(33, 0)].ch(), '╭', "ThreadView top-left ring corner");
    assert_eq!(grid[(79, 0)].ch(), '╮', "ThreadView top-right ring corner");
    assert_eq!(
        grid[(33, 0)].fg(),
        tab_focused.fg,
        "ThreadView ring keeps the focused attr"
    );
    // The first conversation row renders inside the subpane's inner
    // area, one row below the ring and one column right of it.
    let first_row = grid_row_text(grid, 1);
    assert!(
        first_row.contains("golden standalone"),
        "first conversation row must render inside the subpane frame; row 1 was {first_row:?}"
    );
    assert!(
        !RING_GLYPHS.contains(&grid[(10, 1)].ch()),
        "subpane inner first column must be content, not ring"
    );
    record_or_assert("conversations_entry_split_frame", grid);
}

/// Conversations style, a two-mail thread opened: same three panes, but
/// the split `ThreadView` draws two frames of its own inside the view
/// area (at 80 columns the Auto layout takes the horizontal split:
/// thread list above, mail view below), so the screen carries four
/// frames — sidebar, conversation subpane, thread list (focused) and
/// mail view (unfocused).
#[test]
fn golden_conversations_entry_thread_split() {
    let mut ctx = mock_context();
    ctx.settings.listing.index_style = IndexStyle::Conversations;
    let (_account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
    insert_thread_mails(&ctx, inbox_hash);

    let mut listing = Listing::new(&mut ctx);
    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    open_entry_under_cursor(&mut listing, &mut ctx, screen.grid_mut(), area);
    let grid = screen.grid();
    let tab_unfocused = conf::value(&ctx, "tab.unfocused");
    let tab_focused = conf::value(&ctx, "tab.focused");
    assert_eq!(grid[(9, 0)].ch(), '╭', "subpane top-left ring corner");
    assert_eq!(grid[(31, 0)].ch(), '╮', "subpane top-right ring corner");
    assert_eq!(grid[(9, 23)].ch(), '╰', "subpane bottom-left ring corner");
    assert_eq!(grid[(31, 23)].ch(), '╯', "subpane bottom-right ring corner");
    assert_eq!(grid[(31, 5)].ch(), '│', "subpane right ring column");
    assert_eq!(
        grid[(31, 5)].fg(),
        tab_unfocused.fg,
        "subpane ring must use the unfocused attr"
    );
    let first_row = grid_row_text(grid, 1);
    assert!(
        first_row.contains("golden thread root"),
        "first conversation row must render inside the subpane frame; row 1 was {first_row:?}"
    );
    // The ThreadView's two stacked frames in x=33..=79: it reserves the
    // two top rows before its first frame (the same offset the Compact
    // `listing_open_thread` golden pins), so the thread-list frame
    // opens at row 2 with the focused attr and the mail-view frame
    // below it with the unfocused attr.
    assert_eq!(grid[(33, 2)].ch(), '╭', "thread-list frame top-left corner");
    assert_eq!(
        grid[(33, 2)].fg(),
        tab_focused.fg,
        "thread-list frame must use the focused attr"
    );
    assert_eq!(
        grid[(79, 2)].ch(),
        '╮',
        "thread-list frame top-right corner"
    );
    let split_row = (1..23)
        .find(|&y| grid[(33, y)].ch() == '╰')
        .unwrap_or_else(|| {
            panic!(
                "thread-list frame bottom corner missing; column 33 is {}",
                (0..24).map(|y| grid[(33, y)].ch()).collect::<String>()
            )
        });
    assert_eq!(
        grid[(79, split_row)].ch(),
        '╯',
        "thread-list frame bottom-right corner"
    );
    let mail_top_row = (split_row + 1..23)
        .find(|&y| grid[(33, y)].ch() == '╭')
        .unwrap_or_else(|| {
            panic!(
                "mail-view frame top corner missing; column 33 is {}",
                (0..24).map(|y| grid[(33, y)].ch()).collect::<String>()
            )
        });
    assert_eq!(
        grid[(33, mail_top_row)].fg(),
        tab_unfocused.fg,
        "mail-view frame must use the unfocused attr"
    );
    assert_eq!(
        grid[(33, 23)].ch(),
        '╰',
        "mail-view frame bottom-left corner"
    );
    assert_eq!(
        grid[(79, 23)].ch(),
        '╯',
        "mail-view frame bottom-right corner"
    );
    record_or_assert("conversations_entry_thread_split", grid);
}

/// Conversations style, entry opened then closed via the real
/// `exit_entry` key: the pane returns to the single full-width listing
/// frame and no subpane ring glyphs may remain inside the pane.
#[test]
fn golden_conversations_entry_close_no_residue() {
    let mut ctx = mock_context();
    ctx.settings.listing.index_style = IndexStyle::Conversations;
    ctx.settings.shortcuts.listing.exit_entry = Key::Char('i');
    let (_account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
    insert_solo_mail(&ctx, inbox_hash);

    let mut listing = Listing::new(&mut ctx);
    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    open_entry_under_cursor(&mut listing, &mut ctx, screen.grid_mut(), area);
    let mut event = UIEvent::Input(Key::Char('i'));
    assert!(
        listing.process_event(&mut event, &mut ctx),
        "exit_entry must be handled by the listing"
    );
    pump_replies(&mut listing, &mut ctx);
    listing.draw(screen.grid_mut(), area, &mut ctx);
    let grid = screen.grid();

    // The pane is back to the full-width listing frame (ring at columns
    // 9 and 79); only list content may appear inside it.
    assert_eq!(grid[(9, 0)].ch(), '╭', "pane ring top-left");
    assert_eq!(grid[(79, 0)].ch(), '╮', "pane ring top-right");
    assert_eq!(grid[(9, 23)].ch(), '╰', "pane ring bottom-left");
    assert_eq!(grid[(79, 23)].ch(), '╯', "pane ring bottom-right");
    for y in 1..23 {
        for x in [31, 32, 33] {
            assert!(
                !RING_GLYPHS.contains(&grid[(x, y)].ch()),
                "subpane ring residue at ({x},{y}): {:?}",
                grid[(x, y)].ch()
            );
        }
    }
    let first_row = grid_row_text(grid, 1);
    assert!(
        first_row.contains("golden standalone"),
        "full-width list content must be back; row 1 was {first_row:?}"
    );
    record_or_assert("conversations_entry_close_no_residue", grid);
}

/// One row-batch render of `CompactListing`.
#[test]
fn golden_compact_listing_row_batch() {
    let mut ctx = mock_context();
    let (account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
    insert_golden_mails(&ctx, inbox_hash);

    let mut listing = CompactListing::new(ComponentId::default(), (account_hash, inbox_hash), &ctx);
    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    listing.draw(screen.grid_mut(), area, &mut ctx);
    record_or_assert("listing_compact_row_batch", screen.grid());
}

/// One row-batch render of `ConversationsListing`.
#[test]
fn golden_conversations_listing_row_batch() {
    let mut ctx = mock_context();
    let (account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
    insert_golden_mails(&ctx, inbox_hash);

    let mut listing =
        ConversationsListing::new(ComponentId::default(), (account_hash, inbox_hash), &ctx);
    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    listing.draw(screen.grid_mut(), area, &mut ctx);
    record_or_assert("listing_conversations_row_batch", screen.grid());
}

/// One row-batch render of `ThreadListing`.
#[test]
fn golden_thread_listing_row_batch() {
    let mut ctx = mock_context();
    let (account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
    insert_golden_mails(&ctx, inbox_hash);

    let mut listing = ThreadListing::new(ComponentId::default(), (account_hash, inbox_hash), &ctx);
    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    listing.draw(screen.grid_mut(), area, &mut ctx);
    record_or_assert("listing_thread_row_batch", screen.grid());
}

/// One row-batch render of `PlainListing`.
#[test]
fn golden_plain_listing_row_batch() {
    let mut ctx = mock_context();
    let (account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
    insert_golden_mails(&ctx, inbox_hash);

    let mut listing = PlainListing::new(ComponentId::default(), (account_hash, inbox_hash), &ctx);
    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    listing.draw(screen.grid_mut(), area, &mut ctx);
    record_or_assert("listing_plain_row_batch", screen.grid());
}

/// `CompactListing` with a selected row under the cursor.
#[test]
fn golden_compact_listing_selection_row_batch() {
    let mut ctx = mock_context();
    let (account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
    insert_golden_mails(&ctx, inbox_hash);

    let mut listing = CompactListing::new(ComponentId::default(), (account_hash, inbox_hash), &ctx);
    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    draw_selection_row_batch(&mut *listing, screen.grid_mut(), area, &mut ctx);
    record_or_assert("listing_compact_selection_row_batch", screen.grid());
}

/// `ConversationsListing` with a selected block under the cursor; the extra
/// mails make the zebra parity and bold unseen blocks visible below it.
#[test]
fn golden_conversations_listing_selection_row_batch() {
    let mut ctx = mock_context();
    let (account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
    insert_golden_mails(&ctx, inbox_hash);
    insert_selection_mails(&ctx, inbox_hash);

    let mut listing =
        ConversationsListing::new(ComponentId::default(), (account_hash, inbox_hash), &ctx);
    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    draw_selection_row_batch(&mut *listing, screen.grid_mut(), area, &mut ctx);
    record_or_assert("listing_conversations_selection_row_batch", screen.grid());
}

/// `ThreadListing` with a selected row under the cursor.
#[test]
fn golden_thread_listing_selection_row_batch() {
    let mut ctx = mock_context();
    let (account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
    insert_golden_mails(&ctx, inbox_hash);

    let mut listing = ThreadListing::new(ComponentId::default(), (account_hash, inbox_hash), &ctx);
    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    draw_selection_row_batch(&mut *listing, screen.grid_mut(), area, &mut ctx);
    record_or_assert("listing_thread_selection_row_batch", screen.grid());
}

/// `PlainListing` with a selected row under the cursor.
#[test]
fn golden_plain_listing_selection_row_batch() {
    let mut ctx = mock_context();
    let (account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
    insert_golden_mails(&ctx, inbox_hash);

    let mut listing = PlainListing::new(ComponentId::default(), (account_hash, inbox_hash), &ctx);
    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    draw_selection_row_batch(&mut *listing, screen.grid_mut(), area, &mut ctx);
    record_or_assert("listing_plain_selection_row_batch", screen.grid());
}

/// Pager rendering: wrapped long lines, unicode content and collapsed blank
/// line runs.
#[test]
fn golden_pager() {
    let mut ctx = mock_context();
    let text = "\
The quick brown fox jumps over the lazy dog. Pack my box with five dozen liquor \
jugs; a line longer than eighty columns to exercise reflow wrapping.\n\
Second paragraph with unicode content: αβγδε ζηθικ λμν ξοπ ρστυφ χψω.\n\
\n\
\n\
\n\
Third paragraph after a blank-line run.\n\
";
    let mut pager = Pager::from_string(
        text.to_string(),
        &ctx,
        None,
        Some(80),
        conf::value(&ctx, "mail.view.body"),
    );
    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    pager.draw(screen.grid_mut(), area, &mut ctx);
    record_or_assert("pager", screen.grid());
}

/// `EnvelopeView` header block (Date/From/To/Subject/Message-ID) plus the
/// body pager underneath.
#[test]
fn golden_envelope_view_headers() {
    let mut ctx = mock_context();
    let mail = Mail::new(GOLDEN_ROOT_MAIL.to_vec(), None).expect("could not parse test mail");
    let mut view =
        crate::mail::view::EnvelopeView::new(mail, None, None, None, ctx.main_loop_handler.clone());
    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    view.draw(screen.grid_mut(), area, &mut ctx);
    record_or_assert("envelope_view_headers", screen.grid());
}

/// `UIConfirmationDialog` (single-choice selector) drawn as an overlay over
/// a filled background, pinning dialog centering and box chrome.
#[test]
fn golden_uiconfirmation_dialog() {
    let mut ctx = mock_context();
    let mut dialog: UIConfirmationDialog = Selector::new(
        "Are you sure?",
        vec![(true, "Yes".to_string()), (false, "No".to_string())],
        true,
        None,
        &ctx,
    );
    let mut screen = golden_screen(&ctx, 80, 24);
    // Background pattern so the golden proves the overlay leaves surrounding
    // cells untouched.
    {
        let area = screen.area();
        let grid = screen.grid_mut();
        for y in 0..area.height() {
            grid.write_string(
                &"x".repeat(80),
                Color::Default,
                Color::Default,
                Attr::DEFAULT,
                area.nth_row(y),
                None,
                None,
            );
        }
    }
    let area = screen.area();
    dialog.draw(screen.grid_mut(), area, &mut ctx);
    record_or_assert("uiconfirmation_dialog", screen.grid());

    // Second case: the cursor moved onto the first entry (single-choice
    // selector), pinning the options-list selection highlight.
    let mut event = UIEvent::Input(Key::Down);
    assert!(
        dialog.process_event(&mut event, &mut ctx),
        "scroll_down must move the selector cursor"
    );
    dialog.draw(screen.grid_mut(), area, &mut ctx);
    record_or_assert("uiconfirmation_dialog_cursor", screen.grid());
}

/// Shortcuts-help overlay (`?` toggle) over the live listing.
#[test]
fn golden_shortcuts_help_overlay() {
    let mut ctx = mock_context();
    // Pin the shortcut this test drives so that a `MELI_CONFIG` template
    // drift cannot change what the key means.
    ctx.settings.shortcuts.general.toggle_help = Key::Char('?');
    let (_account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
    insert_golden_mails(&ctx, inbox_hash);

    let listing = Listing::new(&mut ctx);
    let mut tabbed = Tabbed::new(
        vec![Box::new(listing), Box::new(ContactList::new(&ctx))],
        &ctx,
    );
    tabbed.realize(None, &mut ctx);
    pump_replies(&mut tabbed, &mut ctx);
    let mut event = UIEvent::Input(Key::Char('?'));
    assert!(
        tabbed.process_event(&mut event, &mut ctx),
        "toggle_help key must be consumed"
    );

    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    tabbed.draw(screen.grid_mut(), area, &mut ctx);
    record_or_assert("shortcuts_help_overlay", screen.grid());
}

/// Listing with the sidebar pane focused (`focus_left`): the sidebar frame
/// uses "tab.focused" and the list frame "tab.unfocused" — the mirror of
/// [`golden_tabbed_statusbar_full_frame`] (list focused) for the focus
/// highlight machine check.
///
/// The focus flip itself is asserted by requiring this render to differ
/// from the list-focused one: `Listing::process_event`'s `focus_left` arm
/// applies the focus change but does not consume the key (it falls through
/// to the component dispatch), so the return value is meaningless here.
#[test]
fn golden_listing_sidebar_focus_frame() {
    let draw_listed = |sidebar_focused: bool| -> CellBuffer {
        let mut ctx = mock_context();
        // Pin the shortcut this test drives so that a `MELI_CONFIG` template
        // drift cannot change what the key means.
        ctx.settings.shortcuts.listing.focus_left = Key::Left;
        let (_account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
        insert_golden_mails(&ctx, inbox_hash);

        let listing = Listing::new(&mut ctx);
        let mut tabbed = Tabbed::new(
            vec![Box::new(listing), Box::new(ContactList::new(&ctx))],
            &ctx,
        );
        tabbed.realize(None, &mut ctx);
        pump_replies(&mut tabbed, &mut ctx);
        if sidebar_focused {
            let mut event = UIEvent::Input(Key::Left);
            tabbed.process_event(&mut event, &mut ctx);
        }

        let mut screen = golden_screen(&ctx, 80, 24);
        let area = screen.area();
        tabbed.draw(screen.grid_mut(), area, &mut ctx);
        screen.grid().clone()
    };

    let sidebar_focused = draw_listed(true);
    let list_focused = draw_listed(false);
    assert_ne!(
        serialize_buffer("listing_sidebar_focus_frame", &sidebar_focused),
        serialize_buffer("listing_list_focus_baseline", &list_focused),
        "sidebar-focused frame must render differently from the \
         list-focused one"
    );
    record_or_assert("listing_sidebar_focus_frame", &sidebar_focused);
}

/// Read one screen row as its plain text (symbols only), for content
/// assertions inside pane frames.
fn grid_row_text(grid: &CellBuffer, y: usize) -> String {
    (0..grid.cols).map(|x| grid[(x, y)].ch()).collect()
}

/// Locate the top-left corner glyph of the rightmost pane ring on the top
/// screen row — with the sidebar visible this is the list pane's frame, so
/// its inner area starts one row below and one column right of it. Scanning
/// instead of hardcoding the column keeps this robust to `sidebar_ratio`
/// changes.
fn list_pane_ring_x(grid: &CellBuffer) -> usize {
    (0..grid.cols)
        .rev()
        .find(|&x| grid[(x, 0)].ch() == '╭')
        .unwrap_or_else(|| {
            panic!("no top-row ring corner found; row 0 is {}", {
                let mut s = String::new();
                for x in 0..grid.cols.min(40) {
                    s.push(grid[(x, 0)].ch());
                }
                s
            })
        })
}

const RING_GLYPHS: [char; 6] = ['─', '│', '╭', '╮', '╰', '╯'];

/// The listing pane's rounded-frame ring must own its own cells: pane
/// content renders inside the frame's inner area, so the first list row
/// (the unseen "golden standalone mail") is fully visible on the row below
/// the top border line and the ring never covers it. Drives all four
/// online index styles through `Listing::draw` (the path that paints the
/// pane rings) and asserts the invariant per style; the final (compact)
/// frame is pinned as a golden.
#[test]
fn golden_listing_frame_inner_content() {
    use crate::command::actions::{Action, ListingAction};

    for style in [
        ListingAction::SetPlain,
        ListingAction::SetThreaded,
        ListingAction::SetConversations,
        ListingAction::SetCompact,
    ] {
        let mut ctx = mock_context();
        let (_account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
        insert_golden_mails(&ctx, inbox_hash);

        let mut listing = Listing::new(&mut ctx);
        listing.realize(None, &mut ctx);
        pump_replies(&mut listing, &mut ctx);
        let is_compact = matches!(style, ListingAction::SetCompact);
        let style_label = format!("{style:?}");
        let mut event = UIEvent::Action(Action::Listing(style));
        assert!(
            listing.process_event(&mut event, &mut ctx),
            "index style switch must be handled"
        );

        // 80 columns cannot fit the full first row inside the ring (inner
        // width 69 < 79 columns of content); 140 columns leaves the inner
        // area ~123 columns wide, enough for the whole subject.
        let mut screen = golden_screen(&ctx, 140, 40);
        let area = screen.area();
        listing.draw(screen.grid_mut(), area, &mut ctx);
        let grid = screen.grid();
        let ring_x = list_pane_ring_x(grid);
        assert_eq!(
            grid[(ring_x + 1, 0)].ch(),
            '─',
            "{style_label}: ring must own the pane's top row"
        );
        let first_inner_row = grid_row_text(grid, 1);
        assert!(
            first_inner_row.contains("golden standalone mail"),
            "{style_label}: first list row must show the latest mail subject \
             inside the ring; row 1 was {first_inner_row:?}"
        );
        assert!(
            !RING_GLYPHS.contains(&grid[(ring_x + 1, 1)].ch()),
            "{style_label}: inner first column must be content, not ring"
        );
        if is_compact {
            record_or_assert("listing_frame_inner_content", grid);
        }
    }
}

/// The status page (`AccountStatus`) shares the list pane's ring: its first
/// content line ("Account <name>") must render on the row below the top
/// border instead of being covered by it. Opened via key navigation only
/// (`Left` to focus the menu, `Up` from INBOX to the account's Status
/// entry, `Right` to open it), since golden tests cannot set private
/// fields the way `listing_menu_tests` does.
#[test]
fn golden_listing_status_page_inside_frame() {
    let mut ctx = mock_context();
    ctx.settings.shortcuts.listing.focus_left = Key::Left;
    ctx.settings.shortcuts.listing.focus_right = Key::Right;
    ctx.settings.shortcuts.listing.scroll_up = Key::Up;
    let (_account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
    insert_golden_mails(&ctx, inbox_hash);

    let mut listing = Listing::new(&mut ctx);
    listing.realize(None, &mut ctx);
    pump_replies(&mut listing, &mut ctx);
    for key in [Key::Left, Key::Up, Key::Right] {
        let mut event = UIEvent::Input(key);
        listing.process_event(&mut event, &mut ctx);
    }

    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    listing.draw(screen.grid_mut(), area, &mut ctx);
    let grid = screen.grid();
    let ring_x = list_pane_ring_x(grid);
    let first_inner_row = grid_row_text(grid, 1);
    assert!(
        first_inner_row.contains("Account"),
        "status page first content row must be visible inside the ring; \
         row 1 was {first_inner_row:?}"
    );
    assert_eq!(grid[(ring_x + 1, 0)].ch(), '─');
}

/// Degenerate terminal sizes must not panic `Listing::draw` now that pane
/// content renders in the ring's inner area: a pane shorter than the ring
/// yields zero inner rows, which used to divide by zero in the listings'
/// row-update paths and in `draw_menu`'s scroll offset. For each index
/// style, draw at tiny sizes, then deliver an `EnvelopeUpdate` (which
/// queues a row update) and draw again.
#[test]
fn golden_listing_tiny_sizes_no_panic() {
    use crate::command::actions::{Action, ListingAction};

    for style in [
        ListingAction::SetPlain,
        ListingAction::SetThreaded,
        ListingAction::SetConversations,
        ListingAction::SetCompact,
    ] {
        let mut ctx = mock_context();
        let (_account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
        insert_golden_mails(&ctx, inbox_hash);

        let mut listing = Listing::new(&mut ctx);
        listing.realize(None, &mut ctx);
        pump_replies(&mut listing, &mut ctx);
        let mut event = UIEvent::Action(Action::Listing(style));
        listing.process_event(&mut event, &mut ctx);

        let env_hash = Envelope::from_bytes(GOLDEN_SOLO_MAIL, None)
            .expect("could not parse test mail")
            .hash();
        for (cols, rows) in [(80, 2), (80, 3), (80, 4), (6, 3), (3, 3), (2, 2)] {
            let mut screen = golden_screen(&ctx, cols, rows);
            let area = screen.area();
            listing.set_dirty(true);
            listing.draw(screen.grid_mut(), area, &mut ctx);
            let mut event = UIEvent::EnvelopeUpdate(env_hash);
            listing.process_event(&mut event, &mut ctx);
            listing.set_dirty(true);
            listing.draw(screen.grid_mut(), area, &mut ctx);
        }
    }
}

/// `ThreadView` split layout (two-mail thread, split focus): the thread
/// list pane frame uses "tab.focused" (it owns the cursor), the mail pane
/// frame "tab.unfocused".
#[test]
fn golden_threadview_split_frames() {
    let mut ctx = mock_context();
    let (account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
    insert_golden_mails(&ctx, inbox_hash);
    let mut view =
        golden_two_mail_thread_view(&mut ctx, account_hash, inbox_hash, ThreadViewFocus::None);

    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    view.draw(screen.grid_mut(), area, &mut ctx);
    record_or_assert("threadview_split_frames", screen.grid());
}

/// `ThreadView` with the mail pane focused: the full-area frame uses
/// "tab.focused" — the border rows must differ from the split capture.
#[test]
fn golden_threadview_mailview_focus_frame() {
    let mut ctx = mock_context();
    let (account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
    insert_golden_mails(&ctx, inbox_hash);
    let mut view = golden_two_mail_thread_view(
        &mut ctx,
        account_hash,
        inbox_hash,
        ThreadViewFocus::MailView,
    );

    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    view.draw(screen.grid_mut(), area, &mut ctx);
    record_or_assert("threadview_mailview_focus_frame", screen.grid());
}

/// Build a `ThreadView` over the two-mail golden thread (root + reply),
/// modeled on `thread.rs::focus_tests::make_two_mail_thread_view`.
fn golden_two_mail_thread_view(
    context: &mut Context,
    account_hash: melib::backends::AccountHash,
    mailbox_hash: melib::backends::MailboxHash,
    focus: ThreadViewFocus,
) -> ThreadView {
    let root_hash = Envelope::from_bytes(GOLDEN_ROOT_MAIL, None)
        .expect("could not parse root test envelope")
        .hash();
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

// ----------------------------------------------------------------------------
// Detector self-tests (no golden files).
// ----------------------------------------------------------------------------

/// Build a small [`CellBuffer`] with varied cell content for the detector
/// self-test: text with distinct fg/bg/attr regions.
fn detector_buffer() -> CellBuffer {
    let mut screen = Screen::<Virtual>::new(ThemeAttribute::default());
    assert!(screen.resize(30, 6), "virtual screen resize");
    let area = screen.area();
    let grid = screen.grid_mut();
    grid.write_string(
        "hello golden",
        Color::Default,
        Color::Default,
        Attr::DEFAULT,
        area,
        None,
        None,
    );
    grid.write_string(
        "BOLDROW",
        Color::Byte(4),
        Color::Default,
        Attr::BOLD,
        area.skip_rows(2),
        None,
        None,
    );
    grid.write_string(
        "rgb",
        Color::Rgb(1, 2, 3),
        Color::Byte(244),
        Attr::UNDERLINE | Attr::ITALICS,
        area.skip_rows(4),
        None,
        None,
    );
    screen.grid().clone()
}

/// Mutation proof: the serialization must detect a single-cell change in any
/// recorded attribute (symbol, fg, bg, attrs, empty flag). If the format ever
/// drops an attribute, the corresponding arm fails, which would let real
/// rendering regressions through the golden corpus undetected.
#[test]
fn golden_detector_detects_single_cell_mutation() {
    let original = detector_buffer();

    // No mutation: identical buffers must serialize identically.
    let clean = original.clone();
    assert_eq!(
        serialize_buffer("detector", &original),
        serialize_buffer("detector", &clean),
        "identical buffers must serialize identically"
    );

    // Single-cell symbol change.
    let mut mutated = original.clone();
    mutated[(2, 2)].set_ch('Z');
    assert_ne!(
        serialize_buffer("detector", &original),
        serialize_buffer("detector", &mutated),
        "detector must catch a single-cell symbol change"
    );

    // Single-cell foreground change.
    let mut mutated = original.clone();
    mutated[(3, 2)].set_fg(Color::Green);
    assert_ne!(
        serialize_buffer("detector", &original),
        serialize_buffer("detector", &mutated),
        "detector must catch a single-cell fg change"
    );

    // Single-cell background change.
    let mut mutated = original.clone();
    mutated[(4, 4)].set_bg(Color::Red);
    assert_ne!(
        serialize_buffer("detector", &original),
        serialize_buffer("detector", &mutated),
        "detector must catch a single-cell bg change"
    );

    // Single-cell attrs change.
    let mut mutated = original.clone();
    mutated[(5, 4)].set_attrs(Attr::REVERSE);
    assert_ne!(
        serialize_buffer("detector", &original),
        serialize_buffer("detector", &mutated),
        "detector must catch a single-cell attrs change"
    );

    // Wide-char continuation (empty) flag change.
    let mut mutated = original.clone();
    mutated[(0, 0)].set_empty(true);
    assert_ne!(
        serialize_buffer("detector", &original),
        serialize_buffer("detector", &mutated),
        "detector must catch a single-cell empty-flag change"
    );

    // The first difference must be reported in the mutated row.
    let mut mutated = original.clone();
    mutated[(7, 4)].set_ch('Q');
    let a = serialize_buffer("detector", &original);
    let b = serialize_buffer("detector", &mutated);
    let first_diff_row = a
        .lines()
        .zip(b.lines())
        .find_map(|(la, lb)| (la != lb).then_some(la))
        .expect("a differing row must exist");
    assert!(
        first_diff_row.starts_with("row4 "),
        "diff diagnostics must point at row 4, got: {first_diff_row}"
    );
}

/// Pin the UTC determinism mechanism itself: after [`init_determinism`],
/// local-time rendering must equal UTC rendering for a fixed timestamp
/// (2026-01-01T00:00:00Z).
#[test]
fn golden_tz_determinism_pin() {
    init_determinism();
    assert_eq!(
        melib::utils::datetime::timestamp_to_string(1_767_225_600, Some("%Y-%m-%d %T"), false),
        "2026-01-01 00:00:00",
        "local-time rendering must be pinned to UTC for the golden corpus"
    );
}
