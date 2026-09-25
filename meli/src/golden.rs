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
    Card, Result, ToggleFlag,
};

use crate::{
    components::{Component, ComponentId, PageMovement},
    conf,
    contacts::list::ContactList,
    mail::listing::{
        CompactListing, ConversationsListing, Listing, ListingTrait, PlainListing, ThreadListing,
    },
    mail::view::{ThreadView, ThreadViewFocus},
    mail::Composer,
    terminal::{Area, Attr, CellBuffer, Color, Screen, Virtual},
    types::UIEvent,
    utilities::{Pager, Selector, StatusBar, Tabbed, UIConfirmationDialog, UIDialog},
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
///
/// Other test modules (e.g. `accounts::tests`) reuse this accessor for the
/// same reason: one process-wide static home means no env flipping between
/// parallel tests.
#[cfg(test)]
pub(crate) fn shared_test_home() -> &'static tempfile::TempDir {
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
    context.settings.shortcuts.listing.select_entry = Key::Char('V').into();
    context.settings.shortcuts.listing.scroll_down = Key::Down.into();
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

/// Regression: the contacts tab is a pinned `Tabbed` child, so `Tabbed`
/// draws no body frame over it. `ContactList` must therefore paint its own
/// rounded pane ring flush to the tab body. It used to inset its content to
/// dodge a frame that is only drawn for inset children, which left the
/// Contacts tab borderless once that frame was scoped to inset children.
/// Switching to the Contacts tab with `Alt-2` must yield a full-body ring
/// under the focused tab attribute, with the contact table header on the
/// first inner row.
#[test]
fn golden_tabbed_contacts_full_frame() {
    let mut ctx = mock_context();
    let (account_hash, _inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
    let mut card = Card::new();
    card.set_name("Golden Contact".to_string())
        .set_email("golden@example.com".to_string())
        .set_url("https://example.com/golden".to_string());
    ctx.accounts[&account_hash].contacts.add_card(card);

    let mut tabbed = Tabbed::new(
        vec![
            Box::new(Listing::new(&mut ctx)),
            Box::new(ContactList::new(&ctx)),
        ],
        &ctx,
    );
    tabbed.realize(None, &mut ctx);
    pump_replies(&mut tabbed, &mut ctx);

    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    tabbed.draw(screen.grid_mut(), area, &mut ctx);

    let mut event = UIEvent::Input(Key::Alt('2'));
    assert!(
        tabbed.process_event(&mut event, &mut ctx),
        "Alt-2 must switch to the Contacts tab"
    );
    pump_replies(&mut tabbed, &mut ctx);
    tabbed.draw(screen.grid_mut(), area, &mut ctx);

    let grid = screen.grid();
    let tab_focused = conf::value(&ctx, "tab.focused");
    /* The tab bar owns row 0, so the contact body spans rows 1..=23 and its
     * ring corners sit on the body's outermost cells. */
    assert_eq!(grid[(0, 1)].ch(), '╭', "contacts tab top-left ring corner");
    assert_eq!(
        grid[(0, 1)].fg(),
        tab_focused.fg,
        "contacts tab ring must use the focused tab attribute"
    );
    assert_eq!(
        grid[(79, 1)].ch(),
        '╮',
        "contacts tab top-right ring corner"
    );
    assert_eq!(
        grid[(0, 23)].ch(),
        '╰',
        "contacts tab bottom-left ring corner"
    );
    assert_eq!(
        grid[(79, 23)].ch(),
        '╯',
        "contacts tab bottom-right ring corner"
    );
    let header_row = grid_row_text(grid, 2);
    assert!(
        header_row.contains("NAME") && header_row.contains("E-MAIL"),
        "contact table header must render on the first inner row; row 2 was {header_row:?}"
    );
    record_or_assert("tabbed_contacts_full_frame", grid);
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

/// Hand the keyboard from the launch-time sidebar focus to the mail list
/// grid without opening an entry. At `Menu` focus the `open_mailbox`
/// shortcut only adopts the sidebar-selected mailbox and moves the focus
/// to the grid; the entry is opened by the separate `open_entry` shortcut
/// once the grid owns the keyboard.
fn focus_mail_list_grid(listing: &mut Listing, context: &mut Context) {
    context.settings.shortcuts.listing.open_mailbox = Key::Char('\n').into();
    let mut event = UIEvent::Input(Key::Char('\n'));
    assert!(
        listing.process_event(&mut event, context),
        "open_mailbox must move the focus from the sidebar to the grid"
    );
    pump_replies(listing, context);
}

/// Drive `listing` to open the entry under the cursor via the pinned
/// `open_entry` shortcut, routing the `OpenEntryUnderCursor` `IntraComm`
/// back in like the main loop does so `Listing.view` is created, then
/// redraw.
///
/// A fresh `Listing` now starts with the keyboard on the visible sidebar,
/// so the grid focus is established first — otherwise the `open_entry`
/// key would be consumed by the sidebar's `open_mailbox` arm and only
/// switch mailboxes.
fn open_entry_under_cursor(
    listing: &mut Listing,
    context: &mut Context,
    grid: &mut CellBuffer,
    area: Area,
) {
    focus_mail_list_grid(listing, context);
    context.settings.shortcuts.listing.open_entry = Key::Char('\n').into();
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

/// Conversations style, a single-mail entry opened: layout2 — the grid
/// keeps 30% of the width (focused ring: the keyboard stays on the grid
/// after opening) and the mail view takes 70% (dimmed), equal in height
/// (rows 0..=23); the mailbox list is hidden.
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
    // The grid subpane keeps 30% (x=0..=23, ring focused — the grid holds
    // the keyboard), the mail view frames the rest (x=25..=79, ring
    // dimmed), both full height.
    assert_eq!(grid[(0, 0)].ch(), '╭', "subpane top-left ring corner");
    assert_eq!(
        grid[(0, 0)].fg(),
        tab_focused.fg,
        "subpane ring must use the focused attr (the grid holds the keyboard)"
    );
    assert_eq!(grid[(23, 0)].ch(), '╮', "subpane top-right ring corner");
    assert_eq!(grid[(0, 23)].ch(), '╰', "subpane bottom-left ring corner");
    assert_eq!(grid[(23, 23)].ch(), '╯', "subpane bottom-right ring corner");
    let view_left = (24..area.width())
        .find(|&x| grid[(x, 0)].ch() == '╭')
        .expect("the view frame must open on row 0");
    assert_eq!(view_left, 25, "the view takes 70% of the width");
    assert_eq!(
        grid[(view_left, 0)].fg(),
        tab_unfocused.fg,
        "mail-view frame must use the dimmed attr"
    );
    assert_eq!(grid[(79, 0)].ch(), '╮', "mail-view frame top-right corner");
    assert_eq!(
        grid[(79, 23)].ch(),
        '╯',
        "mail-view frame bottom-right corner"
    );
    // The conversation row renders inside the subpane's inner area.
    let first_row = grid_row_text(grid, 1);
    assert!(
        first_row.contains("golden standalone"),
        "first conversation row must render inside the subpane frame; row 1 was {first_row:?}"
    );
    record_or_assert("conversations_entry_split_frame", grid);
}

/// Conversations style, a two-mail thread opened: layout3 — the grid
/// keeps 30% (focused ring: the keyboard stays on the grid) and the
/// thread list takes 70% (dimmed, whole-list render); the mailbox list is
/// hidden.
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
    assert_eq!(grid[(0, 0)].ch(), '╭', "subpane top-left ring corner");
    assert_eq!(
        grid[(0, 0)].fg(),
        tab_focused.fg,
        "subpane ring must use the focused attr (the grid holds the keyboard)"
    );
    assert_eq!(grid[(23, 0)].ch(), '╮', "subpane top-right ring corner");
    assert_eq!(grid[(23, 23)].ch(), '╯', "subpane bottom-right ring corner");
    let first_row = grid_row_text(grid, 1);
    assert!(
        first_row.contains("golden thread root"),
        "first conversation row must render inside the subpane frame; row 1 was {first_row:?}"
    );
    // The thread-list frame in x=25..=79, full height, dimmed.
    assert_eq!(grid[(25, 0)].ch(), '╭', "thread-list frame top-left corner");
    assert_eq!(
        grid[(25, 0)].fg(),
        tab_unfocused.fg,
        "thread-list frame must use the dimmed attr (the grid holds the keyboard)"
    );
    assert_eq!(
        grid[(79, 0)].ch(),
        '╮',
        "thread-list frame top-right corner"
    );
    assert_eq!(
        grid[(25, 23)].ch(),
        '╰',
        "thread-list frame bottom-left corner"
    );
    assert_eq!(
        grid[(79, 23)].ch(),
        '╯',
        "thread-list frame bottom-right corner"
    );
    assert!(
        (1..23).any(|y| grid_row_text(grid, y).contains("golden thread root")),
        "the thread-list rows must render inside the view frame"
    );
    assert!(
        (1..23).any(|y| grid_row_text(grid, y).contains("Bob")),
        "the reply row must render inside the view frame"
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
    ctx.settings.shortcuts.listing.exit_entry = Key::Char('i').into();
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

    // Back to layout1: mailbox list (30%, ring at columns 0 and 23) |
    // divider | grid (70%, ring at columns 25 and 79); only list content
    // may appear inside.
    assert_eq!(grid[(0, 0)].ch(), '╭', "mailbox ring top-left");
    assert_eq!(grid[(23, 0)].ch(), '╮', "mailbox ring top-right");
    assert_eq!(grid[(25, 0)].ch(), '╭', "grid ring top-left");
    assert_eq!(grid[(79, 0)].ch(), '╮', "grid ring top-right");
    assert_eq!(grid[(25, 23)].ch(), '╰', "grid ring bottom-left");
    assert_eq!(grid[(79, 23)].ch(), '╯', "grid ring bottom-right");
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

/// A cached listing row whose envelope has since been removed from the
/// collection is exactly the stale-hash race `Collection::get_env` now reports
/// as `None`. Highlighting such a row (which resolves the cursor to a cached
/// envelope hash and then looks it up) must return early instead of
/// dereferencing a dead guard, and must not paint a fabricated envelope in its
/// place.
#[test]
fn stale_listing_row_is_skipped_without_fabricating_content() {
    let mut ctx = mock_context();
    let (account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
    insert_golden_mails(&ctx, inbox_hash);

    // Each listing gets its own screen: the rows of one listing must not be
    // compared against a grid another listing drew over.
    let mut plain = PlainListing::new(ComponentId::default(), (account_hash, inbox_hash), &ctx);
    let mut thread = ThreadListing::new(ComponentId::default(), (account_hash, inbox_hash), &ctx);
    let mut plain_screen = golden_screen(&ctx, 80, 24);
    let mut thread_screen = golden_screen(&ctx, 80, 24);
    let plain_area = plain_screen.area();
    let thread_area = thread_screen.area();
    plain.draw(plain_screen.grid_mut(), plain_area, &mut ctx);
    thread.draw(thread_screen.grid_mut(), thread_area, &mut ctx);

    let non_empty_rows = |grid: &CellBuffer| {
        (0..grid.rows)
            .filter(|&y| !grid_row_text(grid, y).trim().is_empty())
            .count()
    };
    let plain_rows_before = non_empty_rows(plain_screen.grid());
    let thread_rows_before = non_empty_rows(thread_screen.grid());

    // Drop one envelope straight from the collection, leaving both listings'
    // cached rows pointing at a hash that no longer resolves.
    let stale_hash = {
        let account = ctx.accounts.get(&account_hash).unwrap();
        *account
            .collection
            .get_mailbox(inbox_hash)
            .iter()
            .next()
            .unwrap()
    };
    ctx.accounts[&account_hash]
        .collection
        .remove(stale_hash, inbox_hash);
    assert!(!ctx.accounts[&account_hash].contains_key(stale_hash));

    // `highlight_line` for every row, including the stale one: it must not
    // panic, and must not add or drop a drawn row (only cell attributes may
    // change, e.g. the flagged column of a live row).
    for idx in 0..8 {
        plain.highlight_line(plain_screen.grid_mut(), plain_area, idx, &ctx);
        thread.highlight_line(thread_screen.grid_mut(), thread_area, idx, &ctx);
    }
    assert_eq!(
        non_empty_rows(plain_screen.grid()),
        plain_rows_before,
        "highlighting a stale plain-listing row must not fabricate a row"
    );
    assert_eq!(
        non_empty_rows(thread_screen.grid()),
        thread_rows_before,
        "highlighting a stale thread-listing row must not fabricate a row"
    );
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
    ctx.settings.shortcuts.general.toggle_help = Key::Char('?').into();
    let (_account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
    insert_golden_mails(&ctx, inbox_hash);

    let mut listing = Listing::new(&mut ctx);
    // The help overlay lists the focused pane's shortcuts; a fresh listing
    // now starts on the sidebar, whose `open_mailbox` binding would change
    // the rendered text. Keep the original grid-focused capture.
    focus_mail_list_grid(&mut listing, &mut ctx);
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
/// uses "tab.focused" and the list frame "tab.unfocused" — the counterpart
/// of the list-focused capture the other listing goldens pin, for the focus
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
        ctx.settings.shortcuts.listing.focus_left = Key::Left.into();
        let (_account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
        insert_golden_mails(&ctx, inbox_hash);

        let mut listing = Listing::new(&mut ctx);
        // A fresh listing now starts with the keyboard on the visible
        // sidebar; start both captures from the grid so `focus_left` (not
        // the launch default) is what selects the sidebar-focused frame.
        focus_mail_list_grid(&mut listing, &mut ctx);
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
/// instead of hardcoding the column keeps this robust to layout changes.
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
        // The index-style switch action is handled by the grid-focused
        // listing only (a fresh listing now starts on the sidebar), so hand
        // the keyboard to the grid before driving the styles.
        focus_mail_list_grid(&mut listing, &mut ctx);
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
    ctx.settings.shortcuts.listing.focus_left = Key::Left.into();
    ctx.settings.shortcuts.listing.focus_right = Key::Right.into();
    ctx.settings.shortcuts.listing.scroll_up = Key::Up.into();
    let (_account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
    insert_golden_mails(&ctx, inbox_hash);

    let mut listing = Listing::new(&mut ctx);
    listing.realize(None, &mut ctx);
    pump_replies(&mut listing, &mut ctx);
    for key in [Key::Left, Key::Up, Key::Right] {
        let mut event = UIEvent::Input(key.clone());
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

/// `ThreadView` with the mail pane focused: the split stays, and the
/// rings swap — the mail pane frame uses "tab.focused" and the thread
/// list frame "tab.unfocused" — the mirror of the split capture.
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
// Pane background focus tests (no golden files): every pane's empty
// background follows the keyboard — the keyboard-holding pane fills with
// "pane.focused", the others with "pane.unfocused". Ring colors are covered
// by the frame goldens above; these pin the background fills.
// ----------------------------------------------------------------------------

/// Assert the background of an empty cell inside `pane`'s area.
fn assert_pane_bg(
    grid: &CellBuffer,
    (x, y): (usize, usize),
    expected: &ThemeAttribute,
    what: &str,
) {
    assert_eq!(
        grid[(x, y)].bg(),
        expected.bg,
        "{what}: cell ({x},{y}) background must be {expected:?}, got {:?} (row: {:?})",
        grid[(x, y)].bg(),
        grid_row_text(grid, y),
    );
}

/// Assert the background of a *text* cell (a non-blank glyph): the row and
/// heading layer that must ride the pane background while keeping its own
/// fg/attrs.
fn assert_text_cell_bg(grid: &CellBuffer, (x, y): (usize, usize), expected_bg: Color, what: &str) {
    assert_ne!(
        grid[(x, y)].ch(),
        ' ',
        "{what}: cell ({x},{y}) must hold a glyph (row: {:?})",
        grid_row_text(grid, y),
    );
    assert_eq!(
        grid[(x, y)].bg(),
        expected_bg,
        "{what}: text cell ({x},{y}) background must be {expected_bg:?}, got {:?} (row: {:?})",
        grid[(x, y)].bg(),
        grid_row_text(grid, y),
    );
}

/// Layout1: the sidebar owns the keyboard on startup, so the sidebar pane
/// background is the highlight and the mail-list grid background the dimmed
/// one; handing the keyboard to the grid swaps both.
#[test]
fn pane_background_layout1_sidebar_and_grid() {
    let mut ctx = mock_context();
    let (_account_hash, _inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
    insert_golden_mails(&ctx, _inbox_hash);
    let pane_focused = conf::value(&ctx, "pane.focused");
    let pane_unfocused = conf::value(&ctx, "pane.unfocused");
    let sidebar_highlighted = conf::value(&ctx, "mail.sidebar_highlighted");
    let cursor_row_highlighted = conf::value(&ctx, "mail.listing.compact.highlighted");

    let mut listing = Listing::new(&mut ctx);
    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    listing.draw(screen.grid_mut(), area, &mut ctx);
    let grid = screen.grid();
    // Startup focus is the sidebar (b8f74d69): sidebar bright, grid dim.
    // The tail cells sit below/right of the menu entries and the (empty)
    // grid rows, inside the pane rings.
    assert_pane_bg(grid, (20, 21), &pane_focused, "focused sidebar pane");
    assert_pane_bg(grid, (70, 21), &pane_unfocused, "unfocused grid pane");
    // Text layer: base rows ride the pane background; the sidebar cursor
    // entry and the grid cursor row keep their own highlight fills.
    assert_text_cell_bg(
        grid,
        (1, 1),
        pane_focused.bg,
        "active-account name rides the focused sidebar pane",
    );
    assert_eq!(
        grid[(2, 2)].bg(),
        sidebar_highlighted.bg,
        "sidebar cursor entry keeps its highlight background"
    );
    assert_text_cell_bg(
        grid,
        (3, 3),
        pane_focused.bg,
        "base sidebar entry rides the focused sidebar pane",
    );
    assert_eq!(
        grid[(26, 1)].bg(),
        cursor_row_highlighted.bg,
        "grid cursor row keeps its highlight background"
    );
    assert_text_cell_bg(
        grid,
        (26, 2),
        pane_unfocused.bg,
        "grid base row rides the unfocused grid pane",
    );

    // Hand the keyboard to the grid: both backgrounds swap.
    focus_mail_list_grid(&mut listing, &mut ctx);
    listing.set_dirty(true);
    listing.draw(screen.grid_mut(), area, &mut ctx);
    let grid = screen.grid();
    assert_pane_bg(grid, (20, 21), &pane_unfocused, "unfocused sidebar pane");
    assert_pane_bg(grid, (70, 21), &pane_focused, "focused grid pane");
    assert_text_cell_bg(
        grid,
        (1, 1),
        pane_unfocused.bg,
        "active-account name rides the unfocused sidebar pane",
    );
    assert_eq!(
        grid[(2, 2)].bg(),
        sidebar_highlighted.bg,
        "sidebar cursor entry keeps its highlight background"
    );
    assert_text_cell_bg(
        grid,
        (3, 3),
        pane_unfocused.bg,
        "base sidebar entry rides the unfocused sidebar pane",
    );
    assert_eq!(
        grid[(26, 1)].bg(),
        cursor_row_highlighted.bg,
        "grid cursor row keeps its highlight background"
    );
    assert_text_cell_bg(
        grid,
        (26, 2),
        pane_focused.bg,
        "grid base row rides the focused grid pane",
    );
}

/// Layout2 (single mail open): with the keyboard on the grid the mail
/// content pane background is dimmed; moving the keyboard onto the view
/// dims the grid instead.
#[test]
fn pane_background_layout2_grid_and_mailview() {
    let mut ctx = mock_context();
    let (_account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
    insert_solo_mail(&ctx, inbox_hash);
    let pane_focused = conf::value(&ctx, "pane.focused");
    let pane_unfocused = conf::value(&ctx, "pane.unfocused");

    let cursor_row_highlighted = conf::value(&ctx, "mail.listing.compact.highlighted");
    let mut listing = Listing::new(&mut ctx);
    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    // Opening lands the keyboard on the grid (layout2): the grid subpane
    // (x0..=23) is bright, the mail view pane (x25..=79) dimmed.
    open_entry_under_cursor(&mut listing, &mut ctx, screen.grid_mut(), area);
    let grid = screen.grid();
    assert_pane_bg(grid, (10, 21), &pane_focused, "focused grid subpane");
    assert_pane_bg(grid, (70, 21), &pane_unfocused, "dimmed mail view pane");
    // The grid's cursor row keeps its highlight fill; the mail pane has no
    // text cells yet (the view is still loading), its text layer is pinned
    // by `pane_background_envelope_text_follows_pane_fill` below.
    assert_eq!(
        grid[(2, 1)].bg(),
        cursor_row_highlighted.bg,
        "grid cursor row keeps its highlight background"
    );

    // Move the keyboard onto the view (Right): the grid dims, the mail
    // view lights up.
    let mut event = UIEvent::Input(Key::Right);
    assert!(
        listing.process_event(&mut event, &mut ctx),
        "focus_right must move the keyboard onto the open view"
    );
    pump_replies(&mut listing, &mut ctx);
    listing.set_dirty(true);
    listing.draw(screen.grid_mut(), area, &mut ctx);
    let grid = screen.grid();
    assert_pane_bg(grid, (10, 21), &pane_unfocused, "dimmed grid subpane");
    assert_pane_bg(grid, (70, 21), &pane_focused, "focused mail view pane");
    assert_eq!(
        grid[(2, 1)].bg(),
        cursor_row_highlighted.bg,
        "grid cursor row keeps its highlight background"
    );
}

/// Layout3 (thread open): the thread-list pane (the conversations/thread
/// list of the view) stays dimmed while the grid holds the keyboard.
#[test]
fn pane_background_layout3_thread_list() {
    let mut ctx = mock_context();
    let (_account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
    // A two-mail thread so the opened entry is a thread (layout3): the
    // view splits into [thread list | mail view] like
    // `golden_listing_open_thread`.
    insert_thread_mails(&ctx, inbox_hash);
    let pane_focused = conf::value(&ctx, "pane.focused");
    let pane_unfocused = conf::value(&ctx, "pane.unfocused");

    let mut listing = Listing::new(&mut ctx);
    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    open_entry_under_cursor(&mut listing, &mut ctx, screen.grid_mut(), area);
    let grid = screen.grid();
    // Keyboard on the grid (x0..=23): bright grid, dimmed thread list
    // (x25..=79).
    assert_pane_bg(grid, (10, 21), &pane_focused, "focused grid subpane");
    assert_pane_bg(grid, (70, 21), &pane_unfocused, "dimmed thread-list pane");
    // Text layer: the grid cursor row and the thread-list cursor entry
    // keep their highlight fills; a base thread entry rides the dimmed
    // thread-list pane background.
    let cursor_row_highlighted = conf::value(&ctx, "mail.listing.compact.highlighted");
    let highlight = conf::value(&ctx, "highlight");
    assert_eq!(
        grid[(10, 1)].bg(),
        cursor_row_highlighted.bg,
        "grid cursor row keeps its highlight background"
    );
    assert_eq!(
        grid[(30, 1)].bg(),
        highlight.bg,
        "thread-list cursor entry keeps its highlight background"
    );
    assert_text_cell_bg(
        grid,
        (30, 2),
        pane_unfocused.bg,
        "base thread entry rides the dimmed thread-list pane",
    );
}

/// Mail view text (headers + body) rides the hosting pane's fill: with
/// `pane.unfocused` handed to the view, header and body *text* cells take
/// the dim fill and keep their theme fg, and blank filler takes the fill.
#[test]
fn pane_background_envelope_text_follows_pane_fill() {
    let mut ctx = mock_context();
    let pane_unfocused = conf::value(&ctx, "pane.unfocused");
    // Even a stale user config that still defines `mail.view.headers_area`
    // must not leak into the band: every cell rides the pane fill. Install a
    // sentinel fill so the blank-cell assertions below cannot pass vacuously
    // (the shipped theme's headers_area bg may happen to equal the pane fill).
    let sentinel: crate::conf::ThemeAttributeInner =
        toml::from_str("fg = \"theme_default\"\nbg = \"#ff00ff\"\nattrs = \"theme_default\"\n")
            .expect("valid sentinel theme attribute");
    ctx.settings
        .terminal
        .themes
        .other_themes
        .get_mut(crate::conf::DEFAULT_THEME)
        .unwrap()
        .keys
        .insert("mail.view.headers_area".into(), sentinel);

    let mail = Mail::new(GOLDEN_ROOT_MAIL.to_vec(), None).expect("could not parse test mail");
    let mut view =
        crate::mail::view::EnvelopeView::new(mail, None, None, None, ctx.main_loop_handler.clone());
    view.set_pane_fill(Some(pane_unfocused));

    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    view.draw(screen.grid_mut(), area, &mut ctx);
    let grid = screen.grid();
    assert_text_cell_bg(
        grid,
        (0, 0),
        pane_unfocused.bg,
        "header text rides the pane fill",
    );
    assert_text_cell_bg(
        grid,
        (0, 6),
        pane_unfocused.bg,
        "body text rides the pane fill",
    );
    assert_pane_bg(
        grid,
        (60, 6),
        &pane_unfocused,
        "body blank cells take the pane fill",
    );
    assert_text_cell_bg(
        grid,
        (6, 0),
        pane_unfocused.bg,
        "header value text rides the pane fill",
    );
    assert_pane_bg(
        grid,
        (5, 0),
        &pane_unfocused,
        "gap between header name and value takes the pane fill",
    );
    assert_pane_bg(
        grid,
        (79, 0),
        &pane_unfocused,
        "trailing blank after the header value takes the pane fill",
    );
    assert_pane_bg(
        grid,
        (79, 4),
        &pane_unfocused,
        "trailing blank on the last header row takes the pane fill",
    );
}

/// Selected and highlighted rows keep their own fills while base rows ride
/// the pane background (plain style; the selection goldens pin the same
/// split for every index style).
#[test]
fn pane_background_selection_rows_keep_own_fill() {
    let mut ctx = mock_context();
    let (account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
    insert_golden_mails(&ctx, inbox_hash);
    let pane_unfocused = conf::value(&ctx, "pane.unfocused");
    let highlighted_selected = conf::value(&ctx, "mail.listing.plain.highlighted_selected");

    let mut listing = PlainListing::new(ComponentId::default(), (account_hash, inbox_hash), &ctx);
    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    draw_selection_row_batch(&mut *listing, screen.grid_mut(), area, &mut ctx);
    let grid = screen.grid();
    // Row 1 is the cursor row with the selection toggled: its highlighted +
    // selected fill stays; rows 0 and 2 are base rows on the pane fill.
    assert_eq!(
        grid[(5, 1)].bg(),
        highlighted_selected.bg,
        "selected+highlighted row keeps its own background"
    );
    assert_text_cell_bg(
        grid,
        (5, 0),
        pane_unfocused.bg,
        "base row rides the pane fill",
    );
    assert_text_cell_bg(
        grid,
        (5, 2),
        pane_unfocused.bg,
        "base row rides the pane fill",
    );
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

// ----------------------------------------------------------------------------
// statusbar-gauge-spinner corpus (T5).
//
// Behaviour assertions are made on the rendered row text instead of the
// StatusBar's dirty flag — events that do not match the focused mailbox
// can bubble back from the StatusBar without ever flipping it.
// ----------------------------------------------------------------------------

use crate::{accounts::MailboxStatus, jobs::JobId};

/// Read the bottom row of a freshly drawn status-bar screen as plain
/// text. The status bar is always the last row of the frame in
/// `State`'s real layout, and the T5 corpus draws directly into the
/// same area to assert layout decisions.
/// Read the status-bar content row of a freshly drawn status-bar screen.
/// The strip is framed (rounded border ring), so its last row is the
/// frame's lower border — the content sits one row above it.
fn statusbar_row_text(grid: &CellBuffer) -> String {
    let content_row = grid.rows.saturating_sub(2);
    grid_row_text(grid, content_row)
}

/// UT1: focus injection. Feeding `FocusMailbox(acc, mb)` followed by a
/// matching `MailboxUpdate`/`AccountStatusChange` must update the
/// rendered row (positive); a non-matching event after a focus must
/// leave the row text unchanged (negative).
#[test]
fn statusbar_focus_injection() {
    let mut ctx = mock_context();
    let (account_hash, inbox_hash, _archive_hash) = register_two_mailboxes(&mut ctx);
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
    // Baseline: the tabbed listing auto-reports focus during the initial
    // reply pump (Tabbed's `push_focus_updates` fires on the initial
    // cursor sync), so the row already carries the status icon and the
    // mail counts.
    status_bar.draw(screen.grid_mut(), area, &mut ctx);
    let baseline = statusbar_row_text(screen.grid());
    assert!(
        baseline.contains('\u{1F4E7}'),
        "auto-reported focus must render the mail counts, got {baseline:?}"
    );
    assert!(
        baseline.contains('\u{1F4EB}'),
        "idle focused mailbox must render the fixed 📫 icon, got {baseline:?}"
    );

    // No-focus negative: a container without a listing child reports no
    // `status_watch`, so its row must carry no chip/label.
    let mut bare = StatusBar::new(
        &ctx,
        Box::new(Tabbed::new(vec![Box::new(ContactList::new(&ctx))], &ctx)),
    );
    bare.realize(None, &mut ctx);
    pump_replies(&mut bare, &mut ctx);
    let mut bare_screen = golden_screen(&ctx, 80, 24);
    let bare_area = bare_screen.area();
    bare.draw(bare_screen.grid_mut(), bare_area, &mut ctx);
    let bare_row = statusbar_row_text(bare_screen.grid());
    assert!(
        !bare_row.contains('\u{1F4E7}')
            && !bare_row.contains('\u{1F4E9}')
            && !bare_row.contains('\u{1F4EB}'),
        "no-listing container must not render a status icon or counts, got {bare_row:?}"
    );

    // Inject focus on INBOX.
    status_bar.process_event(
        &mut UIEvent::StatusEvent(crate::types::StatusEvent::FocusMailbox(
            account_hash,
            inbox_hash,
        )),
        &mut ctx,
    );
    status_bar.set_dirty(true);
    status_bar.draw(screen.grid_mut(), area, &mut ctx);
    let focused_row = statusbar_row_text(screen.grid());
    assert!(
        focused_row.contains('\u{1F4E7}'),
        "focused row should carry the mail counts, got {focused_row:?}"
    );
    assert!(
        focused_row.contains('\u{1F4E9}'),
        "focused row should carry the unread-mail glyph, got {focused_row:?}"
    );

    // A MailboxUpdate for a different mailbox must not change the row.
    let (_account_hash_2, _inbox_hash_2, archive_hash_2) = register_two_mailboxes(&mut ctx);
    status_bar.process_event(
        &mut UIEvent::MailboxUpdate((account_hash, archive_hash_2)),
        &mut ctx,
    );
    status_bar.set_dirty(true);
    status_bar.draw(screen.grid_mut(), area, &mut ctx);
    let negative_row = statusbar_row_text(screen.grid());
    assert_eq!(
        negative_row, focused_row,
        "non-matching MailboxUpdate must not change the status bar row"
    );

    // AccountStatusChange for the second (non-focused) account must also
    // be a no-op. The bogus-account variant crashes upstream callers
    // (the listing rejects unknown hashes), so the negative case uses
    // a second valid account hash.
    let (_account_hash_3, _, _) = register_two_mailboxes(&mut ctx);
    status_bar.process_event(
        &mut UIEvent::AccountStatusChange(_account_hash_3, None),
        &mut ctx,
    );
    status_bar.set_dirty(true);
    status_bar.draw(screen.grid_mut(), area, &mut ctx);
    assert_eq!(
        statusbar_row_text(screen.grid()),
        focused_row,
        "non-matching AccountStatusChange must not change the status bar row"
    );
}

/// UT8: status-icon branch. An online focused account renders the fixed
/// idle `📫`; an account in an error state renders `✘` instead (the
/// mock's default `is_online` is `Uninit`, which also renders `📫`).
#[test]
fn statusbar_chip_branch() {
    let mut ctx = mock_context();
    let (account_hash, inbox_hash, _) = register_two_mailboxes(&mut ctx);
    ctx.accounts.get_mut(&account_hash).unwrap().is_online = crate::accounts::IsOnline::True;
    let listing = Listing::new(&mut ctx);
    let tabbed = Tabbed::new(
        vec![Box::new(listing), Box::new(ContactList::new(&ctx))],
        &ctx,
    );
    let mut status_bar = StatusBar::new(&ctx, Box::new(tabbed));
    status_bar.realize(None, &mut ctx);
    pump_replies(&mut status_bar, &mut ctx);
    status_bar.process_event(
        &mut UIEvent::StatusEvent(crate::types::StatusEvent::FocusMailbox(
            account_hash,
            inbox_hash,
        )),
        &mut ctx,
    );
    status_bar.set_dirty(true);
    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    status_bar.draw(screen.grid_mut(), area, &mut ctx);
    let row = statusbar_row_text(screen.grid());
    assert!(
        row.contains('\u{1F4EB}'),
        "online idle mailbox must render the fixed 📫 icon, got {row:?}"
    );

    // Offline (Err): the icon must carry the ✘ glyph instead.
    ctx.accounts.get_mut(&account_hash).unwrap().is_online = crate::accounts::IsOnline::Err {
        value: melib::error::Error::new("offline"),
        retries: 1,
    };
    status_bar.set_dirty(true);
    status_bar.draw(screen.grid_mut(), area, &mut ctx);
    let row = statusbar_row_text(screen.grid());
    assert!(
        row.contains('\u{2718}'),
        "offline mailbox must render the ✘ icon, got {row:?}"
    );
    assert!(
        !row.contains('\u{1F4EB}'),
        "offline mailbox must not render the idle 📫 icon, got {row:?}"
    );
}

/// UT9: mailbox-status carousel. While refresh work is in flight the
/// status icon keeps cycling even with the centre gauge active (the
/// carousel is the activity indicator, the gauge the progress bar);
/// once the last job finishes and the mailbox is `Available`, the icon
/// settles on the fixed idle `📫`.
#[test]
fn statusbar_carousel_runs_with_gauge_and_stops() {
    let mut ctx = mock_context();
    let (account_hash, inbox_hash, _) = register_two_mailboxes(&mut ctx);
    // A custom single-frame carousel keeps the frame glyph (`◑`)
    // distinct from every other status-bar glyph, so its presence and
    // absence can be asserted by row text without ambiguity.
    ctx.settings.terminal.progress_spinner_sequence =
        Some(crate::conf::terminal::ProgressSpinnerSequence::Custom {
            frames: vec!["◑".to_string()],
            interval_ms: 80,
        });
    // Provide a non-zero total so the gauge is active.
    ctx.accounts
        .get_mut(&account_hash)
        .unwrap()
        .mailbox_entries
        .get_mut(&inbox_hash)
        .unwrap()
        .status = MailboxStatus::Parsing(12, 250);
    let listing = Listing::new(&mut ctx);
    let tabbed = Tabbed::new(
        vec![Box::new(listing), Box::new(ContactList::new(&ctx))],
        &ctx,
    );
    let mut status_bar = StatusBar::new(&ctx, Box::new(tabbed));
    status_bar.realize(None, &mut ctx);
    pump_replies(&mut status_bar, &mut ctx);
    status_bar.process_event(
        &mut UIEvent::StatusEvent(crate::types::StatusEvent::FocusMailbox(
            account_hash,
            inbox_hash,
        )),
        &mut ctx,
    );
    let job_id = JobId::new();
    status_bar.process_event(
        &mut UIEvent::StatusEvent(crate::types::StatusEvent::NewJob(job_id)),
        &mut ctx,
    );
    status_bar.set_dirty(true);
    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    status_bar.draw(screen.grid_mut(), area, &mut ctx);
    let row = statusbar_row_text(screen.grid());
    assert!(
        row.contains('◑'),
        "carousel must keep cycling while the gauge is active, got {row:?}"
    );

    // Finish the job and flip the mailbox to Available: the carousel
    // stops and the icon settles on the fixed idle glyph.
    ctx.accounts
        .get_mut(&account_hash)
        .unwrap()
        .mailbox_entries
        .get_mut(&inbox_hash)
        .unwrap()
        .status = MailboxStatus::Available;
    status_bar.process_event(
        &mut UIEvent::StatusEvent(crate::types::StatusEvent::JobFinished(job_id)),
        &mut ctx,
    );
    status_bar.set_dirty(true);
    status_bar.draw(screen.grid_mut(), area, &mut ctx);
    let row = statusbar_row_text(screen.grid());
    assert!(
        row.contains('\u{1F4EB}') && !row.contains('◑'),
        "idle mailbox must settle on the fixed 📫 icon once work finishes, \
         got {row:?}"
    );
}

/// UT6: `ascii_drawing=true`. The whole rendered row must be ASCII; no
/// emoji envelope, no ✓/✘ glyphs.
#[test]
fn statusbar_ascii_drawing_is_all_ascii() {
    let mut ctx = mock_context();
    ctx.settings.terminal.ascii_drawing = true;
    // ascii_drawing disables emoji rendering entirely.
    ctx.settings.terminal.emoji_capable = ToggleFlag::InternalVal(false);
    let (account_hash, inbox_hash, _) = register_two_mailboxes(&mut ctx);
    let account = ctx.accounts.get_mut(&account_hash).unwrap();
    account.settings.account.format = "imap".to_string();
    account.is_online = crate::accounts::IsOnline::True;
    let listing = Listing::new(&mut ctx);
    let tabbed = Tabbed::new(
        vec![Box::new(listing), Box::new(ContactList::new(&ctx))],
        &ctx,
    );
    let mut status_bar = StatusBar::new(&ctx, Box::new(tabbed));
    status_bar.realize(None, &mut ctx);
    pump_replies(&mut status_bar, &mut ctx);
    status_bar.process_event(
        &mut UIEvent::StatusEvent(crate::types::StatusEvent::FocusMailbox(
            account_hash,
            inbox_hash,
        )),
        &mut ctx,
    );
    status_bar.set_dirty(true);
    let mut screen = golden_screen(&ctx, 80, 24);
    // The rounded frame picks its border set from the grid flag (mirrors
    // the runtime init that copies the setting into the buffer).
    screen.grid_mut().ascii_drawing = true;
    let area = screen.area();
    status_bar.draw(screen.grid_mut(), area, &mut ctx);
    let row = statusbar_row_text(screen.grid());
    assert!(
        row.is_ascii(),
        "ascii_drawing row must contain only ASCII chars, got {row:?}"
    );
    // ASCII substitutes: idle status icon `#` (offline `!`), counts as
    // `new:N unread:N total:N`, envelope/carousel emoji dropped.
    assert!(
        row.contains("#"),
        "idle status icon must fall back to '#' in ascii_drawing, got {row:?}"
    );
    assert!(
        row.contains("new:"),
        "mail counts must fall back to ASCII labels in ascii_drawing, got {row:?}"
    );
    assert!(
        !row.contains('\u{2713}') && !row.contains('\u{1F4E9}'),
        "ascii_drawing must drop the ✓ and envelope glyph, got {row:?}"
    );
}

/// UT7: hint keys are taken from the user's shortcut binding. Changing
/// `shortcuts.listing.scroll_up` (which also lives in the `general`
/// catch-all as a fallback) changes the rendered `(key:label)` segment
/// accordingly — the focused view's section wins, with `general` only
/// filling in for fields the focused view doesn't expose.
#[test]
fn statusbar_hints_follow_keybinding() {
    let mut ctx = mock_context();
    let (_account_hash, inbox_hash, _) = register_two_mailboxes(&mut ctx);
    insert_golden_mails(&ctx, inbox_hash);
    let listing = Listing::new(&mut ctx);
    let tabbed = Tabbed::new(
        vec![Box::new(listing), Box::new(ContactList::new(&ctx))],
        &ctx,
    );
    let mut status_bar = StatusBar::new(&ctx, Box::new(tabbed));
    status_bar.realize(None, &mut ctx);
    pump_replies(&mut status_bar, &mut ctx);
    let mut screen = golden_screen(&ctx, 200, 24);
    let area = screen.area();
    status_bar.draw(screen.grid_mut(), area, &mut ctx);
    let row = statusbar_row_text(screen.grid());
    // The key glyph must render in the theme's highlight-selected
    // accent color (`mail.listing.compact.highlighted_selected`
    // bg; with the default Ayu Dark theme that is `#5ac1fe`): scan the
    // content row's cells for the `?` of `(?:Help)` (the only `?` in the
    // strip) — cell scanning avoids char-index/column drift from wide
    // glyphs and the frame's border columns.
    let y = screen.grid().rows.saturating_sub(2);
    let help_col = screen
        .grid()
        .bounds_iter(area.nth_row(y))
        .flatten()
        .find(|(x, yy)| screen.grid()[(*x, *yy)].ch() == '?')
        .map(|(x, _)| x)
        .expect("help key glyph '?' must be on the status row");
    assert_eq!(
        screen.grid()[(help_col, y)].fg(),
        crate::conf::value(&ctx, "mail.listing.compact.highlighted_selected").bg,
        "hint key glyphs must render in the theme highlight-selected color, got row {row:?}"
    );
    // The mock context is emoji-capable, so the up-arrow binding renders
    // as the `⬆️` emoji icon rather than `<Up>`: `(⬆️|k:Scroll Up)`. The
    // in-group `|` of that group must be a plain span: normal status-bar
    // font color, no bold — only the key glyphs carry the highlight.
    // Locate the `|` immediately followed by `k` so the row's other `|`
    // cells (the segment head and the count separators) do not qualify.
    let row_cells: Vec<(usize, usize)> = screen
        .grid()
        .bounds_iter(area.nth_row(y))
        .flatten()
        .collect();
    let sep_idx = row_cells
        .iter()
        .position(|p| {
            screen.grid()[*p].ch() == '|'
                && p.0 + 1 < screen.grid().cols
                && screen.grid()[(p.0 + 1, p.1)].ch() == 'k'
        })
        .expect("the `(⬆️|k:Scroll Up)` separator must be on the status row");
    let (sep_col, sep_row) = row_cells[sep_idx];
    let k_col = (sep_col + 1, sep_row);
    // A `base + VS16` emoji-presentation cluster occupies the leading
    // glyph cell plus an empty continuation cell (see
    // `CellBuffer::write_string`), so the `⬆` glyph sits two cells before
    // the separator.
    let cont_col = sep_col - 1;
    assert!(
        screen.grid()[(cont_col, sep_row)].empty(),
        "the `⬆️` cluster must reserve an empty continuation cell before the `|`, got row {row:?}"
    );
    let arrow_col = sep_col - 2;
    let arrow = &screen.grid()[(arrow_col, sep_row)];
    assert_eq!(
        arrow.ch(),
        '\u{2B06}',
        "the up-arrow emoji base glyph must precede the in-group separator, got row {row:?}"
    );
    let sep = &screen.grid()[(sep_col, sep_row)];
    assert_eq!(
        sep.fg(),
        crate::conf::value(&ctx, "status.bar").fg,
        "the in-group `|` separator must use the normal status-bar font color, got row {row:?}"
    );
    assert!(
        !sep.attrs().contains(Attr::BOLD),
        "the in-group `|` separator must not be bold, got row {row:?}"
    );
    let k_cell = &screen.grid()[k_col];
    assert_eq!(
        k_cell.fg(),
        crate::conf::value(&ctx, "mail.listing.compact.highlighted_selected").bg,
        "the key glyph `k` must render in the theme highlight-selected color, got row {row:?}"
    );
    assert!(
        k_cell.attrs().contains(Attr::BOLD),
        "the key glyph `k` must be bold, got row {row:?}"
    );
    // The arrow glyph IS the highlighted key text: highlight-selected
    // color, bold, and carrying the emoji-presentation attribute.
    assert_eq!(
        arrow.fg(),
        crate::conf::value(&ctx, "mail.listing.compact.highlighted_selected").bg,
        "the `⬆` icon must render in the theme highlight-selected color, got row {row:?}"
    );
    assert!(
        arrow.attrs().contains(Attr::BOLD) && arrow.attrs().contains(Attr::FORCE_EMOJI),
        "the `⬆` icon must be bold and carry FORCE_EMOJI, got row {row:?}"
    );
    // `grid_row_text` concatenates each cell's `ch()`, so the VS16
    // cluster contributes the `⬆` base char followed by the empty
    // continuation cell's blank `ch()`; assert on the base char and the
    // `|k:Scroll Up` tail rather than the exact joined string.
    assert!(
        row.contains('⬆') && row.contains("|k:Scroll Up") && !row.contains("<Up>"),
        "default hints must render the arrow as an emoji icon (`⬆` … `|k:Scroll Up`) and drop \
         `<Up>`, got {row:?}"
    );
    assert!(row.contains("?:Help"), "got {row:?}");
    // The default `listing.search` binding is a two-key group
    // (`/` first, `F3` second), so the listing hint must surface the
    // literal group text `(/|<F3>:Search)`, all ASCII.
    assert!(
        row.contains(":Search)") && row.contains("(/|<F3>:Search)"),
        "listing view must surface the default search group `(/|<F3>:Search)`, got {row:?}"
    );
    // Display order: search sits before the trailing quit hint.
    let search_idx = row
        .find(":Search)")
        .expect("the search hint must be present on the listing status row");
    let quit_idx = row
        .find(":Quit)")
        .expect("the quit hint must be present on the listing status row");
    assert!(
        search_idx < quit_idx,
        "the search hint (`:Search)`) must precede quit (`:Quit)`), got {row:?}"
    );
    // Rebind the listing section — the focused view's section wins
    // over the `general` catch-all even though both define `scroll_up`.
    ctx.settings.shortcuts.listing.scroll_up = Key::Char('j').into();
    status_bar.set_dirty(true);
    status_bar.draw(screen.grid_mut(), area, &mut ctx);
    let row = statusbar_row_text(screen.grid());
    assert!(
        row.contains("(j:Scroll Up)") && !row.contains("Up:Scroll Up"),
        "rebound listing.scroll_up must surface as `(j:Scroll Up)`, got {row:?}"
    );
}

/// `UT7b`: hint pickers only render for fields the focused view exposes.
/// On the composing view, `composing.close` exists, so the hint must
/// include `(Esc:Close View)`; on the contact-list view, `focus_left`
/// is not in `ContactListShortcuts`, so `Focus Left` must not
/// appear in the hint.
#[test]
fn statusbar_hints_follow_view_section() {
    let mut ctx = mock_context();
    let (account_hash, inbox_hash, _) = register_two_mailboxes(&mut ctx);
    insert_golden_mails(&ctx, inbox_hash);
    let composer = Composer::with_account(account_hash, &ctx);
    let tabbed = Tabbed::new(vec![Box::new(composer)], &ctx);
    let mut status_bar = StatusBar::new(&ctx, Box::new(tabbed));
    status_bar.realize(None, &mut ctx);
    pump_replies(&mut status_bar, &mut ctx);
    let mut screen = golden_screen(&ctx, 200, 24);
    let area = screen.area();
    status_bar.draw(screen.grid_mut(), area, &mut ctx);
    let row = statusbar_row_text(screen.grid());
    assert!(
        row.contains("(<Esc>:Close View)"),
        "composing view must surface `composing.close` as `(<Esc>:Close View)`, got {row:?}"
    );
    // Composing exposes no `search` binding and `general` has none
    // either, so the Search hint must be silently skipped.
    assert!(
        !row.contains(":Search)"),
        "composing view must drop the Search hint (no binding), got {row:?}"
    );

    // ContactList has no `focus_left` / `focus_right` — the
    // `Focus Left/Right` hints must not appear.
    let bare = StatusBar::new(
        &ctx,
        Box::new(Tabbed::new(vec![Box::new(ContactList::new(&ctx))], &ctx)),
    );
    let mut bare = bare;
    bare.realize(None, &mut ctx);
    pump_replies(&mut bare, &mut ctx);
    let mut screen2 = golden_screen(&ctx, 200, 24);
    let area2 = screen2.area();
    bare.draw(screen2.grid_mut(), area2, &mut ctx);
    let row2 = statusbar_row_text(screen2.grid());
    assert!(
        !row2.contains("Focus Left") && !row2.contains("Focus Right"),
        "contact-list view must drop Focus Left/Right hints (no binding), got {row2:?}"
    );
    // ContactList exposes no `search` binding either, so the Search
    // hint must be silently skipped here too.
    assert!(
        !row2.contains(":Search)"),
        "contact-list view must drop the Search hint (no binding), got {row2:?}"
    );
}

/// Layered quit: on a non-pinned tab (a composer opened via `Tab(New)`),
/// the quit binding must close that tab through the `Tab(Kill)` reply
/// path instead of reaching the application-level exit.
#[test]
fn quit_key_closes_unpinned_tab() {
    use crate::command::{actions::Action, TabAction};

    let mut ctx = mock_context();
    let (account_hash, inbox_hash, _) = register_two_mailboxes(&mut ctx);
    insert_golden_mails(&ctx, inbox_hash);
    let mut tabbed = Tabbed::new(
        vec![
            Box::new(Listing::new(&mut ctx)),
            Box::new(ContactList::new(&ctx)),
        ],
        &ctx,
    );
    tabbed.realize(None, &mut ctx);
    pump_replies(&mut tabbed, &mut ctx);
    let composer = Composer::with_account(account_hash, &ctx);
    let mut event = UIEvent::Action(Action::Tab(TabAction::New(Some(Box::new(composer)))));
    assert!(tabbed.process_event(&mut event, &mut ctx));
    pump_replies(&mut tabbed, &mut ctx);
    let before = tabbed.children().len();
    assert_eq!(before, 3, "precondition: composer tab added");

    let mut event = UIEvent::Input(Key::Char('q'));
    assert!(
        tabbed.process_event(&mut event, &mut ctx),
        "quit on a non-pinned tab must be consumed by Tabbed"
    );
    pump_replies(&mut tabbed, &mut ctx);
    assert_eq!(
        tabbed.children().len(),
        before - 1,
        "quit must close the non-pinned composer tab"
    );
}

/// Layered quit, dirty draft: the quit binding must never discard unsaved
/// work. The first quit key is consumed by the composer (it opens the
/// unsaved-changes dialog), and a second quit key — which the dialog does
/// not consume — must be vetoed by the child's `can_quit_cleanly` instead
/// of closing the tab and losing the draft. Regression: `Tabbed`'s quit
/// arm used to treat "child returned false" as "close the tab", so
/// pressing `q` twice (or `Esc` twice, or `Esc` from the recipient /
/// attachment sub-views) silently discarded the draft.
#[test]
fn quit_key_never_discards_dirty_draft() {
    use crate::command::{actions::Action, TabAction};

    for key in [Key::Char('q'), Key::Esc] {
        let mut ctx = mock_context();
        let (account_hash, inbox_hash, _) = register_two_mailboxes(&mut ctx);
        insert_golden_mails(&ctx, inbox_hash);
        let mut tabbed = Tabbed::new(
            vec![
                Box::new(Listing::new(&mut ctx)),
                Box::new(ContactList::new(&ctx)),
            ],
            &ctx,
        );
        tabbed.realize(None, &mut ctx);
        pump_replies(&mut tabbed, &mut ctx);
        let mut composer = Composer::with_account(account_hash, &ctx);
        composer.set_has_changes_for_tests(true);
        let mut event = UIEvent::Action(Action::Tab(TabAction::New(Some(Box::new(composer)))));
        assert!(tabbed.process_event(&mut event, &mut ctx));
        pump_replies(&mut tabbed, &mut ctx);
        let before = tabbed.children().len();
        assert_eq!(before, 3, "precondition: dirty composer tab added");

        // Every quit key press, however many, must leave the tab (and the
        // draft) alone: only the dialog's own x/y choices may close it.
        for round in 0..3 {
            let mut event = UIEvent::Input(key.clone());
            let _ = tabbed.process_event(&mut event, &mut ctx);
            pump_replies(&mut tabbed, &mut ctx);
            assert_eq!(
                tabbed.children().len(),
                before,
                "{key:?} on round {round}: quit must not discard the dirty draft"
            );
        }
    }
}

/// Layered quit, top level: in Normal mode, when the focused pinned view
/// does not consume the quit binding, `StatusBar` must turn it into a
/// `UIEvent::Exit` reply for the main loop (the app-level exit path)
/// instead of dropping the key.
#[test]
fn statusbar_quit_unconsumed_requests_exit() {
    let mut ctx = mock_context();
    let (_account_hash, inbox_hash, _) = register_two_mailboxes(&mut ctx);
    insert_golden_mails(&ctx, inbox_hash);
    let tabbed = Tabbed::new(
        vec![
            Box::new(Listing::new(&mut ctx)),
            Box::new(ContactList::new(&ctx)),
        ],
        &ctx,
    );
    let mut status_bar = StatusBar::new(&ctx, Box::new(tabbed));
    status_bar.realize(None, &mut ctx);
    pump_replies(&mut status_bar, &mut ctx);

    for key in [Key::Esc, Key::Char('q')] {
        let mut event = UIEvent::Input(key.clone());
        assert!(status_bar.process_event(&mut event, &mut ctx));
        assert!(
            ctx.replies().iter().any(|r| matches!(r, UIEvent::Exit)),
            "{key:?} must surface as a UIEvent::Exit request when no view consumes it"
        );
    }
}

/// The help overlay closes on any quit-group key (not just `Esc`):
/// with the overlay open, `q` must toggle it off and be consumed, and
/// no `UIEvent::Exit` may surface — the app-level exit path only sees
/// quit keys once the overlay (and every sub-view) is gone.
#[test]
fn quit_key_closes_help_overlay() {
    let mut ctx = mock_context();
    let (_account_hash, inbox_hash, _) = register_two_mailboxes(&mut ctx);
    insert_golden_mails(&ctx, inbox_hash);
    let tabbed = Tabbed::new(vec![Box::new(Listing::new(&mut ctx))], &ctx);
    let mut status_bar = StatusBar::new(&ctx, Box::new(tabbed));
    status_bar.realize(None, &mut ctx);
    pump_replies(&mut status_bar, &mut ctx);

    for key in [Key::Char('q'), Key::Esc] {
        // Open the help overlay.
        let mut event = UIEvent::Input(Key::Char('?'));
        assert!(
            status_bar.process_event(&mut event, &mut ctx),
            "toggle_help must open the overlay"
        );
        // Either quit key closes the overlay instead of leaking to the
        // top-level exit path.
        let mut event = UIEvent::Input(key.clone());
        assert!(
            status_bar.process_event(&mut event, &mut ctx),
            "{key:?} must be consumed by the help overlay"
        );
        assert!(
            !ctx.replies().iter().any(|r| matches!(r, UIEvent::Exit)),
            "{key:?} with the help overlay open must not request app exit"
        );
    }
}

/// Layered quit for the theme picker overlay (`:toggle theme`): the picker
/// is a `UIDialog` built like `State::open_theme_picker` does. Every
/// quit-group key (`q`/`Esc` by default) must close the picker and be
/// consumed by it — `State::rcv_event` dispatches to overlays first, so a
/// consumed key never reaches the components below, where `StatusBar` would
/// turn it into a `UIEvent::Exit` app-exit request. Closing is pure exit:
/// no theme event may fire (selection takes effect only via the arrow-key
/// live preview and `Enter`'s persist).
#[test]
fn quit_key_closes_theme_picker() {
    let mut ctx = mock_context();
    let (_account_hash, inbox_hash, _) = register_two_mailboxes(&mut ctx);
    insert_golden_mails(&ctx, inbox_hash);
    let tabbed = Tabbed::new(vec![Box::new(Listing::new(&mut ctx))], &ctx);
    let mut status_bar = StatusBar::new(&ctx, Box::new(tabbed));
    status_bar.realize(None, &mut ctx);
    pump_replies(&mut status_bar, &mut ctx);

    for key in [Key::Char('q'), Key::Esc] {
        let current = ctx.settings.terminal.theme.clone();
        let mut picker: UIDialog<String> = UIDialog::new(
            "theme",
            vec![
                ("Ayu Dark".to_string(), "Ayu Dark (built-in)".to_string()),
                ("light".to_string(), "light (config)".to_string()),
            ],
            /* single_only */ true,
            /* done_fn */ None,
            &ctx,
        );
        // Build the picker the way `State::open_theme_picker` does.
        picker.set_cursor_to(&current);
        let restore = current.clone();
        picker.set_done_fn(Some(Box::new(
            move |_id, selection: &[String]| match selection.first() {
                Some(name) => Some(UIEvent::ChangeTheme {
                    name: name.clone(),
                    persist: true,
                }),
                None => Some(UIEvent::ChangeTheme {
                    name: restore,
                    persist: false,
                }),
            },
        )));
        picker.realize(None, &mut ctx);

        // The overlay consumes the quit key (`rcv_event` stops at the
        // first component returning true, so the StatusBar never sees it
        // and no `UIEvent::Exit` may appear).
        let mut event = UIEvent::Input(key.clone());
        assert!(
            picker.process_event(&mut event, &mut ctx),
            "{key:?} must be consumed by the theme picker"
        );
        assert!(picker.is_done(), "{key:?} must close the theme picker");
        // `Context::replies` drains, so snapshot once for the check
        // below. An `Exit` assertion would be vacuous here: the picker
        // alone never emits `UIEvent::Exit` - that would take the
        // `StatusBar` dispatch this test bypasses. The effective
        // assertions are the two above (the picker consumes the quit
        // key and closes) and the one below (closing is pure exit, no
        // theme event fires).
        let replies = ctx.replies();
        // Design: closing is pure exit - no persist/apply event fires.
        assert!(
            !replies
                .iter()
                .any(|r| matches!(r, UIEvent::ChangeTheme { .. })),
            "{key:?} must not fire any theme event on close"
        );
    }
}

/// The framed status strip must keep both side border columns intact at
/// every terminal width (the emoji-presentation cluster fix keeps the
/// grid's column accounting in sync with the terminal, so the content
/// never visually overruns the right frame border).
#[test]
fn statusbar_frame_columns_intact_at_all_widths() {
    for cols in (114usize..=200).chain([79, 80, 100]) {
        let mut ctx = mock_context();
        let (_account_hash, inbox_hash, _) = register_two_mailboxes(&mut ctx);
        insert_golden_mails(&ctx, inbox_hash);
        ctx.settings.shortcuts.listing.scroll_up = Key::Char('j').into();
        ctx.settings.shortcuts.listing.scroll_down = Key::Char('k').into();
        let listing = Listing::new(&mut ctx);
        let tabbed = Tabbed::new(
            vec![Box::new(listing), Box::new(ContactList::new(&ctx))],
            &ctx,
        );
        let mut status_bar = StatusBar::new(&ctx, Box::new(tabbed));
        status_bar.realize(None, &mut ctx);
        pump_replies(&mut status_bar, &mut ctx);
        let mut screen = golden_screen(&ctx, cols, 10);
        let area = screen.area();
        status_bar.draw(screen.grid_mut(), area, &mut ctx);
        let y = screen.grid().rows - 2;
        let last = screen.grid()[(cols - 1, y)].ch();
        let second_last = screen.grid()[(cols - 2, y)].ch();
        let first = screen.grid()[(0, y)].ch();
        println!("cols={cols} first={first:?} second_last={second_last:?} last={last:?}");
        assert_eq!(first, '\u{2502}', "left border missing at cols={cols}");
        assert_eq!(last, '\u{2502}', "right border missing at cols={cols}");
    }
}

/// `UT7c`: with a thread open, the scroll hints must come from the
/// `thread-view` section — the section `ThreadView::process_event`
/// actually dispatches on — not from the embedded mail view's `pager`
/// section, which also defines `scroll_up`/`scroll_down` and used to
/// shadow thread-view rebinds in the hints.
#[test]
fn statusbar_hints_follow_thread_view_section() {
    let mut ctx = mock_context();
    let (account_hash, inbox_hash, _) = register_two_mailboxes(&mut ctx);
    insert_thread_mails(&ctx, inbox_hash);
    let mut view =
        golden_two_mail_thread_view(&mut ctx, account_hash, inbox_hash, ThreadViewFocus::None);
    // Drive the expanded mail view to `Loaded`: `MailViewState::shortcuts`
    // is empty until the body bytes arrive, and the pager section (which
    // also defines `scroll_up`) only exists once the EnvelopeView does —
    // this is the state where the pager used to shadow thread-view
    // rebinds in the hints.
    view.load_expanded_entry_for_tests(GOLDEN_REPLY_MAIL.to_vec(), &mut ctx);
    let mut status_bar = StatusBar::new(&ctx, Box::new(view));
    status_bar.realize(None, &mut ctx);
    pump_replies(&mut status_bar, &mut ctx);
    let mut screen = golden_screen(&ctx, 200, 24);
    let area = screen.area();
    status_bar.draw(screen.grid_mut(), area, &mut ctx);
    // Rebind both contenders: the thread-view section (the dispatcher at
    // split focus) and the pager section (the former shadow). The
    // rendered hint must follow the thread-view binding.
    ctx.settings.shortcuts.thread_view.scroll_up = Key::Char('k').into();
    ctx.settings.shortcuts.pager.scroll_up = Key::Char('p').into();
    status_bar.set_dirty(true);
    status_bar.draw(screen.grid_mut(), area, &mut ctx);
    let row = statusbar_row_text(screen.grid());
    assert!(
        row.contains("(k:Scroll Up)") && !row.contains("p:Scroll Up"),
        "thread view must surface thread_view.scroll_up, not the pager's, got {row:?}"
    );
}

/// UT10: status-bar counts come from the account's collection (the same
/// envelopes the listing renders), not the backend's mailbox metadata.
/// After inserting the golden corpus (3 mails, 1 unseen) the row must
/// show `📨0 📩1 ✉️3`, and a further unseen arrival delivered via
/// `MailboxUpdate` must surface as `📨1 📩2` — the repeated
/// `FocusMailbox` co-emission that rides along every status refresh
/// must not re-baseline the floor.
#[test]
fn statusbar_counts_from_collection() {
    let mut ctx = mock_context();
    let (account_hash, inbox_hash, _) = register_two_mailboxes(&mut ctx);
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
    let row = statusbar_row_text(screen.grid());
    assert!(
        row.contains("📨 0") && row.contains("📩 1") && row.contains("📧 3"),
        "counts must mirror the collection (1 unseen of 3), got {row:?}"
    );
    // A new unseen arrival: flip a seen envelope and deliver the same
    // MailboxUpdate the backend would send (the listing co-emits another
    // FocusMailbox for the same mailbox while handling it).
    let seen_hash = {
        let account = ctx.accounts.get(&account_hash).unwrap();
        account
            .collection
            .get_mailbox(inbox_hash)
            .iter()
            .copied()
            .find(|h| {
                account
                    .collection
                    .get_env(*h)
                    .is_some_and(|env| env.is_seen())
            })
            .expect("golden corpus contains seen mails")
    };
    if let Some(mut env) = ctx
        .accounts
        .get_mut(&account_hash)
        .unwrap()
        .collection
        .get_env_mut(seen_hash)
    {
        env.set_unseen();
    } else {
        panic!("seen_hash {seen_hash} vanished from the golden collection");
    }
    status_bar.process_event(
        &mut UIEvent::MailboxUpdate((account_hash, inbox_hash)),
        &mut ctx,
    );
    status_bar.process_event(
        &mut UIEvent::StatusEvent(crate::types::StatusEvent::FocusMailbox(
            account_hash,
            inbox_hash,
        )),
        &mut ctx,
    );
    status_bar.set_dirty(true);
    status_bar.draw(screen.grid_mut(), area, &mut ctx);
    let row = statusbar_row_text(screen.grid());
    assert!(
        row.contains("📨 1") && row.contains("📩 2"),
        "a new unseen arrival must surface in the 📨 counter even with a \
         repeated FocusMailbox, got {row:?}"
    );

    // An *incremental* sync (the mailbox was already populated when the
    // parse session started) must not absorb arrivals: further unseen
    // mail surfaces in 📨 even while `Parsing`. Only a first fetch
    // (empty mailbox) absorbs — see `statusbar_counts_initial_fetch_absorbed`.
    let last_seen_hash = {
        let account = ctx.accounts.get(&account_hash).unwrap();
        account
            .collection
            .get_mailbox(inbox_hash)
            .iter()
            .copied()
            .find(|h| {
                account
                    .collection
                    .get_env(*h)
                    .is_some_and(|env| env.is_seen())
            })
            .expect("one seen mail remains in the corpus")
    };
    ctx.accounts
        .get_mut(&account_hash)
        .unwrap()
        .mailbox_entries
        .get_mut(&inbox_hash)
        .unwrap()
        .status = MailboxStatus::Parsing(3, 3);
    if let Some(mut env) = ctx
        .accounts
        .get_mut(&account_hash)
        .unwrap()
        .collection
        .get_env_mut(last_seen_hash)
    {
        env.set_unseen();
    } else {
        panic!("last_seen_hash {last_seen_hash} vanished from the golden collection");
    }
    status_bar.process_event(
        &mut UIEvent::MailboxUpdate((account_hash, inbox_hash)),
        &mut ctx,
    );
    status_bar.set_dirty(true);
    status_bar.draw(screen.grid_mut(), area, &mut ctx);
    let row = statusbar_row_text(screen.grid());
    assert!(
        row.contains("📨 2") && row.contains("📩 3"),
        "unseen arrivals during an incremental sync must count as new \
         mail, got {row:?}"
    );
}

/// A mailbox that was empty when its first parse session started absorbs
/// the synced-in mail: those arrivals belong to the initial fetch and
/// must not surface as `📨 new`.
#[test]
fn statusbar_counts_initial_fetch_absorbed() {
    let mut ctx = mock_context();
    let (account_hash, inbox_hash, _) = register_two_mailboxes(&mut ctx);
    // The initial fetch starts on an empty mailbox.
    ctx.accounts
        .get_mut(&account_hash)
        .unwrap()
        .mailbox_entries
        .get_mut(&inbox_hash)
        .unwrap()
        .status = MailboxStatus::Parsing(0, 0);
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
    // First observation during the parse session: settled total is 0, so
    // the session is classified as the initial fetch.
    status_bar.draw(screen.grid_mut(), area, &mut ctx);
    // Mail synced in by the initial fetch.
    insert_golden_mails(&ctx, inbox_hash);
    status_bar.process_event(
        &mut UIEvent::MailboxUpdate((account_hash, inbox_hash)),
        &mut ctx,
    );
    status_bar.set_dirty(true);
    status_bar.draw(screen.grid_mut(), area, &mut ctx);
    let row = statusbar_row_text(screen.grid());
    assert!(
        row.contains("📨 0") && row.contains("📩 1") && row.contains("📧 3"),
        "initial-fetch arrivals must be absorbed, got {row:?}"
    );
    // Once the fetch settles, a further arrival surfaces again.
    ctx.accounts
        .get_mut(&account_hash)
        .unwrap()
        .mailbox_entries
        .get_mut(&inbox_hash)
        .unwrap()
        .status = MailboxStatus::Available;
    let seen_hash = {
        let account = ctx.accounts.get(&account_hash).unwrap();
        account
            .collection
            .get_mailbox(inbox_hash)
            .iter()
            .copied()
            .find(|h| {
                account
                    .collection
                    .get_env(*h)
                    .is_some_and(|env| env.is_seen())
            })
            .expect("golden corpus contains seen mails")
    };
    if let Some(mut env) = ctx
        .accounts
        .get_mut(&account_hash)
        .unwrap()
        .collection
        .get_env_mut(seen_hash)
    {
        env.set_unseen();
    } else {
        panic!("seen_hash {seen_hash} vanished from the golden collection");
    }
    status_bar.process_event(
        &mut UIEvent::MailboxUpdate((account_hash, inbox_hash)),
        &mut ctx,
    );
    status_bar.set_dirty(true);
    status_bar.draw(screen.grid_mut(), area, &mut ctx);
    let row = statusbar_row_text(screen.grid());
    assert!(
        row.contains("📨 1") && row.contains("📩 2"),
        "arrivals after the initial fetch settle must surface, got {row:?}"
    );
}

/// UT11: status-bar falls back to non-emoji analogues when the terminal
/// can't render them. With `emoji_capable = false` the row contains
/// only ASCII or braille (no envelope / mailbox / keyboard glyphs).
#[test]
fn statusbar_no_emoji_fallback() {
    let mut ctx = mock_context();
    ctx.settings.terminal.emoji_capable = ToggleFlag::InternalVal(false);
    let (account_hash, inbox_hash, _) = register_two_mailboxes(&mut ctx);
    ctx.accounts.get_mut(&account_hash).unwrap().is_online = crate::accounts::IsOnline::True;
    let listing = Listing::new(&mut ctx);
    let tabbed = Tabbed::new(
        vec![Box::new(listing), Box::new(ContactList::new(&ctx))],
        &ctx,
    );
    let mut status_bar = StatusBar::new(&ctx, Box::new(tabbed));
    status_bar.realize(None, &mut ctx);
    pump_replies(&mut status_bar, &mut ctx);
    status_bar.process_event(
        &mut UIEvent::StatusEvent(crate::types::StatusEvent::FocusMailbox(
            account_hash,
            inbox_hash,
        )),
        &mut ctx,
    );
    status_bar.set_dirty(true);
    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    status_bar.draw(screen.grid_mut(), area, &mut ctx);
    let row = statusbar_row_text(screen.grid());
    assert!(
        !row.contains('📫')
            && !row.contains('📩')
            && !row.contains('📨')
            && !row.contains('📧')
            && !row.contains('⌨'),
        "emoji glyphs must be replaced when emoji_capable is false, got {row:?}"
    );
    // Idle online icon is the ASCII fallback `#` (ascii_drawing still
    // defaults to false so we get the braille-analogue hierarchy).
    assert!(
        row.contains('#'),
        "idle icon should fall back to '#', got {row:?}"
    );

    // With NewJob fired, the carousel runs. With emoji off + ascii off
    // (braille mode), the carousel frames are the braille analogues.
    let job_id = JobId::new();
    status_bar.process_event(
        &mut UIEvent::StatusEvent(crate::types::StatusEvent::NewJob(job_id)),
        &mut ctx,
    );
    status_bar.set_dirty(true);
    status_bar.draw(screen.grid_mut(), area, &mut ctx);
    let row = statusbar_row_text(screen.grid());
    assert!(
        ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏']
            .iter()
            .any(|g| row.contains(*g)),
        "a braille carousel frame must be rendered, got {row:?}"
    );
}

/// GT3: Parsing(12, 250) + active job. The centre segment renders a
/// `LineGauge` with label `Fetch 12/250` in the status row (anchored to
/// the gauge segment, not the grid origin), with the carousel cycling
/// at the left edge.
#[test]
fn golden_statusbar_gauge_active() {
    let mut ctx = mock_context();
    let (account_hash, inbox_hash, _) = register_two_mailboxes(&mut ctx);
    ctx.accounts
        .get_mut(&account_hash)
        .unwrap()
        .mailbox_entries
        .get_mut(&inbox_hash)
        .unwrap()
        .status = MailboxStatus::Parsing(12, 250);
    let listing = Listing::new(&mut ctx);
    let tabbed = Tabbed::new(
        vec![Box::new(listing), Box::new(ContactList::new(&ctx))],
        &ctx,
    );
    let mut status_bar = StatusBar::new(&ctx, Box::new(tabbed));
    status_bar.realize(None, &mut ctx);
    pump_replies(&mut status_bar, &mut ctx);
    status_bar.process_event(
        &mut UIEvent::StatusEvent(crate::types::StatusEvent::FocusMailbox(
            account_hash,
            inbox_hash,
        )),
        &mut ctx,
    );
    status_bar.process_event(
        &mut UIEvent::StatusEvent(crate::types::StatusEvent::NewJob(JobId::new())),
        &mut ctx,
    );
    status_bar.set_dirty(true);
    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    status_bar.draw(screen.grid_mut(), area, &mut ctx);
    let row = statusbar_row_text(screen.grid());
    assert!(
        row.contains("Fetch 12/250"),
        "the gauge label must render in the status row, got {row:?}"
    );
    assert!(
        row.contains('▱'),
        "the gauge bar must render unfilled cells for ratio 12/250, got {row:?}"
    );
    record_or_assert("statusbar_gauge_active", screen.grid());
}

/// GT4: Parsing(5, 0) (incremental batch, total unknown) + active job.
/// No gauge segment; the mailbox-status carousel cycles at the left
/// edge of the status row.
#[test]
fn golden_statusbar_spinner_only() {
    let mut ctx = mock_context();
    let (account_hash, inbox_hash, _) = register_two_mailboxes(&mut ctx);
    ctx.accounts
        .get_mut(&account_hash)
        .unwrap()
        .mailbox_entries
        .get_mut(&inbox_hash)
        .unwrap()
        .status = MailboxStatus::Parsing(5, 0);
    let listing = Listing::new(&mut ctx);
    let tabbed = Tabbed::new(
        vec![Box::new(listing), Box::new(ContactList::new(&ctx))],
        &ctx,
    );
    let mut status_bar = StatusBar::new(&ctx, Box::new(tabbed));
    status_bar.realize(None, &mut ctx);
    pump_replies(&mut status_bar, &mut ctx);
    status_bar.process_event(
        &mut UIEvent::StatusEvent(crate::types::StatusEvent::FocusMailbox(
            account_hash,
            inbox_hash,
        )),
        &mut ctx,
    );
    status_bar.process_event(
        &mut UIEvent::StatusEvent(crate::types::StatusEvent::NewJob(JobId::new())),
        &mut ctx,
    );
    status_bar.set_dirty(true);
    let mut screen = golden_screen(&ctx, 80, 24);
    let area = screen.area();
    status_bar.draw(screen.grid_mut(), area, &mut ctx);
    record_or_assert("statusbar_spinner_only", screen.grid());
}

/// GT5: status bar must not panic at degenerate widths or tiny screens.
/// Mirrors `golden_listing_tiny_sizes_no_panic`. The (40, 6) frame is
/// recorded as a golden so a regression on the layout's truncation
/// logic (hint `…`, gauge `#/.`, focus chip) is caught by `make test`.
#[test]
fn golden_statusbar_tiny_sizes_no_panic() {
    let mut ctx = mock_context();
    let (account_hash, inbox_hash, _) = register_two_mailboxes(&mut ctx);
    insert_golden_mails(&ctx, inbox_hash);
    let listing = Listing::new(&mut ctx);
    let tabbed = Tabbed::new(
        vec![Box::new(listing), Box::new(ContactList::new(&ctx))],
        &ctx,
    );
    let mut status_bar = StatusBar::new(&ctx, Box::new(tabbed));
    status_bar.realize(None, &mut ctx);
    pump_replies(&mut status_bar, &mut ctx);
    status_bar.process_event(
        &mut UIEvent::StatusEvent(crate::types::StatusEvent::FocusMailbox(
            account_hash,
            inbox_hash,
        )),
        &mut ctx,
    );
    status_bar.process_event(
        &mut UIEvent::StatusEvent(crate::types::StatusEvent::NewJob(JobId::new())),
        &mut ctx,
    );
    for (cols, rows) in [(40, 6), (60, 6), (20, 3), (10, 4)] {
        let mut screen = golden_screen(&ctx, cols, rows);
        let area = screen.area();
        status_bar.set_dirty(true);
        status_bar.draw(screen.grid_mut(), area, &mut ctx);
    }
    // Record the (40, 6) frame as the GT5 golden.
    let mut screen = golden_screen(&ctx, 40, 6);
    let area = screen.area();
    status_bar.set_dirty(true);
    status_bar.draw(screen.grid_mut(), area, &mut ctx);
    record_or_assert("statusbar_tiny_sizes", screen.grid());
}
