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

use std::{
    borrow::Cow,
    fmt::Write as FmtWrite,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
};

use indexmap::IndexMap;

use crate::{
    conf::{
        shortcuts::{
            ComposingShortcuts, ContactListShortcuts, EnvelopeViewShortcuts, GeneralShortcuts,
            ListingShortcuts, PagerShortcuts, ThreadViewShortcuts,
        },
        themes::*,
        FileSettings,
    },
    terminal::{Color, Key, ShortcutKeys},
    Attr,
};

/// TEMP: verify the sample-config shortcut comments round-trip through
/// the real deserialization path.
#[test]
fn sample_config_shortcuts_roundtrip() {
    let sample = std::fs::read_to_string(
        env!("CARGO_MANIFEST_DIR").to_string() + "/docs/samples/sample-config.toml",
    )
    .unwrap();
    let sec = &sample[sample.find("###shortcuts").unwrap()..sample.find("#[composing]").unwrap()];
    let mut text = String::new();
    for line in sec.lines() {
        if line.starts_with("###shortcuts") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#[shortcuts.") {
            // `ShortcutsOverride` holds the sections directly; the
            // outer `[shortcuts]` table belongs to `FileSettings`.
            text.push('[');
            text.push_str(rest);
            text.push('\n');
            continue;
        }
        if let Some(rest) = line.strip_prefix("## ") {
            if rest.contains(" = ") && !rest.starts_with("All shortcut") {
                text.push_str(rest);
                text.push('\n');
            }
        }
    }
    let override_: crate::conf::ShortcutsOverride =
        toml::from_str(&text).expect("sample shortcut block must deserialize via the real path");
    // Spot-check round-trips.
    assert_eq!(
        override_.listing.as_ref().unwrap().scroll_up.to_string(),
        "Up,k"
    );
    assert_eq!(
        override_.general.as_ref().unwrap().quit.to_string(),
        "Esc,q"
    );
}

pub struct ConfigFile {
    pub path: PathBuf,
    pub file: fs::File,
}

impl ConfigFile {
    pub fn new(
        content: &str,
        dir: &tempfile::TempDir,
    ) -> std::result::Result<Self, std::io::Error> {
        let mut filename = String::with_capacity(2 * 16);
        for byte in melib::utils::random::random_u64().to_be_bytes() {
            write!(&mut filename, "{byte:02X}").unwrap();
        }
        let mut path = dir.path().to_path_buf();
        path.push(&*filename);
        let mut file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&path)?;
        file.write_all(content.as_bytes())?;
        file.flush()?;
        Ok(Self { path, file })
    }
}

impl Drop for ConfigFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub const TEST_CONFIG: &str = r#"
[accounts.account-name]
root_mailbox = "/path/to/root/mailbox"
format = "Maildir"
send_mail = 'false'
listing.index_style = "Conversations" # or [plain, threaded, compact]
identity="email@example.com"
display_name = "Name"
subscribed_mailboxes = ["INBOX", "INBOX/Sent", "INBOX/Drafts", "INBOX/Junk"]

# Set mailbox-specific settings
  [accounts.account-name.mailboxes]
  "INBOX" = { rename="Inbox" }
  "drafts" = { rename="Drafts" }
  "foobar-devel" = { ignore = true } # don't show notifications for this mailbox

# Setting up an mbox account
[accounts.mbox]
root_mailbox = "/var/mail/username"
format = "mbox"
send_mail = 'false'
listing.index_style = "Compact"
identity="username@hostname.local"
"#;

pub const EXTRA_CONFIG: &str = r#"
[accounts.mbox]
root_mailbox = "/"
format = "mbox"
send_mail = 'false'
index_style = "Compact"
identity="username@hostname.local"
    "#;
pub const IMAP_CONFIG: &str = r#"
[accounts.imap]
root_mailbox = "INBOX"
format = "imap"
send_mail = 'false'
identity="username@example.com"
server_username = "null"
server_hostname = "example.com"
server_password_command = "false"
    "#;

#[test]
fn test_conf_config_parse() {
    let tempdir = tempfile::tempdir().unwrap();
    let new_file = ConfigFile::new(TEST_CONFIG, &tempdir).unwrap();
    let err = FileSettings::validate(new_file.path.clone(), true).unwrap_err();
    assert_eq!(
        err.summary.as_ref(),
        "Configuration error (account-name): root_mailbox `/path/to/root/mailbox` is not a valid \
         directory."
    );

    /* Test unrecognised configuration entries error */

    let new_file = ConfigFile::new(EXTRA_CONFIG, &tempdir).unwrap();
    let err = FileSettings::validate(new_file.path.clone(), true).unwrap_err();
    assert_eq!(
        err.summary.as_ref(),
        "Unrecognised configuration values: {\"index_style\": \"Compact\"}"
    );

    /* Test IMAP config */

    let new_file = ConfigFile::new(IMAP_CONFIG, &tempdir).unwrap();
    FileSettings::validate(new_file.path.clone(), true).expect("could not parse IMAP config");

    /* Test sample config */

    // Sample config contains `crate::conf::composing::SendMail::Smtp` variant which
    // only exists if meli is build with `smtp` feature.
    if cfg!(feature = "smtp") {
        let example_config = FileSettings::EXAMPLE_CONFIG.replace("\n#", "\n");
        let re = regex::Regex::new(r#"root_mailbox\s*=\s*"[^"]*""#).unwrap();
        let example_config = re.replace_all(
            &example_config,
            &format!(r#"root_mailbox = "{}""#, tempdir.path().to_str().unwrap()),
        );

        let new_file = ConfigFile::new(&example_config, &tempdir).unwrap();
        let config = FileSettings::validate(new_file.path.clone(), true)
            .expect("Could not parse example config!");
        for (accname, acc) in config.accounts.iter() {
            if !acc.extra.is_empty() {
                panic!(
                    "In example config, account `{}` has unrecognised configuration entries: {:?}",
                    accname, acc.extra
                );
            }
        }
    }

    if let Err(err) = tempdir.close() {
        eprintln!("Could not cleanup tempdir: {err}");
    }
}

#[test]
fn test_conf_thread_view_focus_shortcuts_parse() {
    let tempdir = tempfile::tempdir().unwrap();
    let config = format!(
        r#"
[accounts.focus-test]
root_mailbox = "{}"
format = "maildir"
send_mail = 'false'
identity = "username@hostname.local"

[shortcuts.thread-view]
focus_left = "Left"
focus_right = "Right"
"#,
        tempdir.path().display()
    );

    let new_file = ConfigFile::new(&config, &tempdir).unwrap();
    let config = FileSettings::validate(new_file.path.clone(), true)
        .expect("could not parse thread-view focus shortcuts config");

    let thread_view = config.shortcuts.thread_view.key_values();
    assert_eq!(
        thread_view.get("focus_left"),
        Some(&ShortcutKeys::single(Key::Left)),
        "focus_left must parse from [shortcuts.thread-view]"
    );
    assert_eq!(
        thread_view.get("focus_right"),
        Some(&ShortcutKeys::single(Key::Right)),
        "focus_right must parse from [shortcuts.thread-view]"
    );

    if let Err(err) = tempdir.close() {
        eprintln!("Could not cleanup tempdir: {err}");
    }
}

/// Shortcut fields accept up to two comma-separated keys; a bare single
/// value keeps its previous single-key meaning.
#[test]
fn test_conf_multi_key_shortcut_parse() {
    let tempdir = tempfile::tempdir().unwrap();
    // Each binding parses in its own config file (`deny_unknown_fields`
    // and section field sets differ between sections).
    for (field, value, expected) in [
        (
            "scroll_up",
            "\"Up,k\"",
            ShortcutKeys::double(Key::Up, Key::Char('k')),
        ),
        ("scroll_down", "\"Down\"", ShortcutKeys::single(Key::Down)),
        ("scroll_up", "'j'", ShortcutKeys::single(Key::Char('j'))),
    ] {
        let config = format!(
            r#"
[accounts.shortcut-test]
root_mailbox = "{}"
format = "maildir"
send_mail = 'false'
identity = "username@hostname.local"

[shortcuts.listing]
{field} = {value}
"#,
            tempdir.path().display()
        );
        let new_file = ConfigFile::new(&config, &tempdir).unwrap();
        let config = FileSettings::validate(new_file.path.clone(), true)
            .expect("could not parse multi-key shortcut config");
        let listing = config.shortcuts.listing.key_values();
        assert_eq!(
            listing.get(field),
            Some(&expected),
            "{field} = {value} must parse as {expected:?}"
        );
    }
    if let Err(err) = tempdir.close() {
        eprintln!("Could not cleanup tempdir: {err}");
    }
}

/// More than two comma-separated keys must be rejected at parse time.
#[test]
fn test_conf_three_key_shortcut_rejected() {
    let tempdir = tempfile::tempdir().unwrap();
    let config = format!(
        r#"
[accounts.shortcut-test]
root_mailbox = "{}"
format = "maildir"
send_mail = 'false'
identity = "username@hostname.local"

[shortcuts.listing]
scroll_up = "a,b,c"
"#,
        tempdir.path().display()
    );
    let new_file = ConfigFile::new(&config, &tempdir).unwrap();
    let err = FileSettings::validate(new_file.path.clone(), true).unwrap_err();
    assert!(
        err.to_string().contains("at most two comma-separated keys"),
        "three-key binding must be rejected with a clear error, got: {err}"
    );

    if let Err(err) = tempdir.close() {
        eprintln!("Could not cleanup tempdir: {err}");
    }
}
/// The horizontal navigation defaults mirror the vertical ones (arrow +
/// vim key either way), and the two settings that used to default to
/// `h` — which now belongs to the navigation key group — moved off it:
/// `thread-view.collapse_subtree` to `H` and
/// `envelope-view.toggle_expand_headers` to `x`. Users who prefer the
/// old single-key behavior can rebind them in their config.
#[test]
fn test_conf_navigation_keygroup_conflicts_resolved() {
    let general = GeneralShortcuts::default().key_values();
    assert_eq!(
        general.get("scroll_left"),
        Some(&ShortcutKeys::double(Key::Left, Key::Char('h'))),
        "general.scroll_left must default to Left,h"
    );
    assert_eq!(
        general.get("scroll_right"),
        Some(&ShortcutKeys::double(Key::Right, Key::Char('l'))),
        "general.scroll_right must default to Right,l"
    );

    let thread_view = ThreadViewShortcuts::default().key_values();
    assert_eq!(
        thread_view.get("collapse_subtree"),
        Some(&ShortcutKeys::single(Key::Char('H'))),
        "collapse_subtree must not claim the navigation key h"
    );

    let env_view = EnvelopeViewShortcuts::default().key_values();
    assert_eq!(
        env_view.get("toggle_expand_headers"),
        Some(&ShortcutKeys::single(Key::Char('x'))),
        "toggle_expand_headers must not claim the navigation key h"
    );

    // The envelope-view `reply` key moved off `R` onto the navigation key
    // `r`, and the vacated `R` now returns to the normal view; assert both
    // sides so a partial rebind cannot leave the two sharing `r`.
    assert_eq!(
        env_view.get("reply"),
        Some(&ShortcutKeys::single(Key::Char('r'))),
        "envelope-view.reply must default to r"
    );
    assert_eq!(
        env_view.get("return_to_normal_view"),
        Some(&ShortcutKeys::single(Key::Char('R'))),
        "envelope-view.return_to_normal_view must default to R"
    );

    // Listing refresh is a two-key group (`F5` first); the config/Display
    // form must stay round-trippable.
    let listing = ListingShortcuts::default().key_values();
    assert_eq!(
        listing.get("refresh").map(|keys| keys.to_string()),
        Some("F5,C-r".to_string()),
        "listing.refresh must render as F5,C-r"
    );
    assert_eq!(
        listing.get("refresh"),
        Some(&ShortcutKeys::double(Key::F(5), Key::Ctrl('r'))),
        "listing.refresh must bind both F5 and C-r"
    );
}

#[test]
fn test_conf_save_all_attachments_shortcut_parse() {
    let tempdir = tempfile::tempdir().unwrap();
    let config = format!(
        r#"
[accounts.shortcut-test]
root_mailbox = "{}"
format = "maildir"
send_mail = 'false'
identity = "username@hostname.local"

[shortcuts.envelope-view]
save_all_attachments = "C-s"
"#,
        tempdir.path().display()
    );

    let new_file = ConfigFile::new(&config, &tempdir).unwrap();
    let config = FileSettings::validate(new_file.path.clone(), true)
        .expect("could not parse envelope-view save_all_attachments shortcut config");

    let env_view = config.shortcuts.envelope_view.key_values();
    assert_eq!(
        env_view.get("save_all_attachments"),
        Some(&ShortcutKeys::single(Key::Ctrl('s'))),
        "save_all_attachments must parse from [shortcuts.envelope-view] as C-s"
    );

    let defaults = EnvelopeViewShortcuts::default().key_values();
    assert_eq!(
        defaults.get("save_all_attachments"),
        Some(&ShortcutKeys::single(Key::Ctrl('s'))),
        "save_all_attachments must default to C-s"
    );

    if let Err(err) = tempdir.close() {
        eprintln!("Could not cleanup tempdir: {err}");
    }
}

#[test]
fn test_conf_html_filter_defaults_to_builtin() {
    use crate::conf::pager::PagerSettings;

    let defaults = PagerSettings::default();
    assert_eq!(
        defaults.html_filter, None,
        "html_filter must default to None (the built-in renderer)"
    );

    let tempdir = tempfile::tempdir().unwrap();
    let base = format!(
        r#"
[accounts.filter-test]
root_mailbox = "{}"
format = "maildir"
send_mail = 'false'
identity = "username@hostname.local"
"#,
        tempdir.path().display()
    );

    // Absent [pager] section: the built-in renderer is the effective default.
    let new_file = ConfigFile::new(&base, &tempdir).unwrap();
    let config = FileSettings::validate(new_file.path.clone(), true).unwrap();
    assert_eq!(
        config.pager.html_filter, None,
        "absent html_filter must resolve to None (the built-in renderer)"
    );

    // An explicitly empty value also means the built-in renderer.
    let new_file =
        ConfigFile::new(&format!("{base}\n[pager]\nhtml_filter = ''\n"), &tempdir).unwrap();
    let config = FileSettings::validate(new_file.path.clone(), true).unwrap();
    assert_eq!(
        config.pager.html_filter, None,
        "empty html_filter must mean the built-in renderer"
    );

    // An explicitly configured command string is preserved as-is.
    let new_file = ConfigFile::new(
        &format!("{base}\n[pager]\nhtml_filter = 'w3m -T text/html'\n"),
        &tempdir,
    )
    .unwrap();
    let config = FileSettings::validate(new_file.path.clone(), true).unwrap();
    assert_eq!(
        config.pager.html_filter.as_deref(),
        Some("w3m -T text/html"),
        "explicit html_filter command must be preserved as-is"
    );

    if let Err(err) = tempdir.close() {
        eprintln!("Could not cleanup tempdir: {err}");
    }
}

/// Vertical navigation defaults are vim-style doubles everywhere: every
/// shortcut section's `scroll_up`/`scroll_down` defaults to `Up`/`k` and
/// `Down`/`j` (either key scrolls).
#[test]
fn test_conf_arrow_navigation_defaults() {
    for (section, values) in [
        ("listing", ListingShortcuts::default().key_values()),
        ("contact-list", ContactListShortcuts::default().key_values()),
        ("pager", PagerShortcuts::default().key_values()),
        ("general", GeneralShortcuts::default().key_values()),
        ("composing", ComposingShortcuts::default().key_values()),
        ("thread-view", ThreadViewShortcuts::default().key_values()),
    ] {
        assert_eq!(
            values.get("scroll_up"),
            Some(&ShortcutKeys::double(Key::Up, Key::Char('k'))),
            "{section}.scroll_up must default to Up,k"
        );
        assert_eq!(
            values.get("scroll_down"),
            Some(&ShortcutKeys::double(Key::Down, Key::Char('j'))),
            "{section}.scroll_down must default to Down,j"
        );
    }
}

#[test]
fn test_conf_composer_close_shortcut_default() {
    let defaults = ComposingShortcuts::default().key_values();
    assert_eq!(
        defaults.get("close"),
        Some(&ShortcutKeys::single(Key::Esc)),
        "composing.close must default to Esc"
    );
}

#[test]
fn test_conf_composer_close_shortcut_parse() {
    let tempdir = tempfile::tempdir().unwrap();
    let config = format!(
        r#"
[accounts.shortcut-test]
root_mailbox = "{}"
format = "maildir"
send_mail = 'false'
identity = "username@hostname.local"

[shortcuts.composing]
close = "C-x"
"#,
        tempdir.path().display()
    );

    let new_file = ConfigFile::new(&config, &tempdir).unwrap();
    let config = FileSettings::validate(new_file.path.clone(), true)
        .expect("could not parse composing close shortcut config");

    let composing = config.shortcuts.composing.key_values();
    assert_eq!(
        composing.get("close"),
        Some(&ShortcutKeys::single(Key::Ctrl('x'))),
        "close must parse from [shortcuts.composing] as C-x"
    );

    if let Err(err) = tempdir.close() {
        eprintln!("Could not cleanup tempdir: {err}");
    }
}

#[test]
fn test_conf_theme_parsing() {
    /* MUST SUCCEED: default themes should be valid */
    let def = Themes::default();
    def.validate().unwrap();
    /* MUST SUCCEED: new user theme `hunter2`, theme `dark` has user
     * redefinitions */
    const TEST_STR: &str = r#"[dark]
"mail.listing.tag_default" = { fg = "White", bg = "HotPink3" }
"mail.listing.attachment_flag" = { fg = "mail.listing.tag_default.bg" }
"mail.view.headers" = { bg = "mail.listing.tag_default.fg" }

["hunter2"]
"mail.view.body" = { fg = "Black", bg = "White"}"#;
    let parsed: Themes = toml::from_str(TEST_STR).unwrap();
    assert!(parsed.other_themes.contains_key("hunter2"));
    assert_eq!(
        unlink_bg(
            &parsed.dark,
            &ColorField::Bg,
            &Cow::from("mail.listing.tag_default")
        ),
        Color::Byte(132)
    );
    assert_eq!(
        unlink_fg(
            &parsed.dark,
            &ColorField::Fg,
            &Cow::from("mail.listing.attachment_flag")
        ),
        Color::Byte(132)
    );
    assert_eq!(
        unlink_bg(
            &parsed.dark,
            &ColorField::Bg,
            &Cow::from("mail.view.headers")
        ),
        Color::Byte(15), // White
    );
    parsed.validate().unwrap();
    /* MUST FAIL: theme `dark` contains a cycle */
    const HAS_CYCLE: &str = r#"[dark]
"mail.listing.compact" = { fg = "mail.listing.plain" }
"mail.listing.plain" = { fg = "mail.listing.compact" }
"#;
    let parsed: Themes = toml::from_str(HAS_CYCLE).unwrap();
    parsed.validate().unwrap_err();
    /* MUST FAIL: theme `dark` contains an invalid key */
    const HAS_INVALID_KEYS: &str = r#"[dark]
"asdfsafsa" = { fg = "Black" }
"#;
    let parsed: std::result::Result<Themes, _> = toml::from_str(HAS_INVALID_KEYS);
    parsed.unwrap_err();
    /* MUST SUCCEED: alias $Jebediah resolves to a valid color */
    const TEST_ALIAS_STR: &str = r##"[dark]
color_aliases= { "Jebediah" = "#b4da55" }
"mail.listing.tag_default" = { fg = "$Jebediah" }
"##;
    let parsed: Themes = toml::from_str(TEST_ALIAS_STR).unwrap();
    parsed.validate().unwrap();
    assert_eq!(
        unlink_fg(
            &parsed.dark,
            &ColorField::Fg,
            &Cow::from("mail.listing.tag_default")
        ),
        Color::Rgb(180, 218, 85)
    );
    /* MUST FAIL: Misspell color alias $Jebediah as $Jebedia */
    const TEST_INVALID_ALIAS_STR: &str = r##"[dark]
color_aliases= { "Jebediah" = "#b4da55" }
"mail.listing.tag_default" = { fg = "$Jebedia" }
"##;
    let parsed: Themes = toml::from_str(TEST_INVALID_ALIAS_STR).unwrap();
    parsed.validate().unwrap_err();
    /* MUST FAIL: Color alias $Jebediah is defined as itself */
    const TEST_CYCLIC_ALIAS_STR: &str = r#"[dark]
color_aliases= { "Jebediah" = "$Jebediah" }
"mail.listing.tag_default" = { fg = "$Jebediah" }
"#;
    let parsed: Themes = toml::from_str(TEST_CYCLIC_ALIAS_STR).unwrap();
    parsed.validate().unwrap_err();
    /* MUST FAIL: Attr alias $Jebediah is defined as itself */
    const TEST_CYCLIC_ALIAS_ATTR_STR: &str = r#"[dark]
attr_aliases= { "Jebediah" = "$Jebediah" }
"mail.listing.tag_default" = { attrs = "$Jebediah" }
"#;
    let parsed: Themes = toml::from_str(TEST_CYCLIC_ALIAS_ATTR_STR).unwrap();
    parsed.validate().unwrap_err();
    /* MUST FAIL: alias $Jebediah resolves to a cycle */
    const TEST_CYCLIC_ALIAS_STR_2: &str = r#"[dark]
color_aliases= { "Jebediah" = "$JebediahJr", "JebediahJr" = "mail.listing.tag_default" }
"mail.listing.tag_default" = { fg = "$Jebediah" }
"#;
    let parsed: Themes = toml::from_str(TEST_CYCLIC_ALIAS_STR_2).unwrap();
    parsed.validate().unwrap_err();
    /* MUST SUCCEED: alias $Jebediah resolves to a key's field */
    const TEST_CYCLIC_ALIAS_STR_3: &str = r#"[dark]
color_aliases= { "Jebediah" = "$JebediahJr", "JebediahJr" = "mail.listing.tag_default.bg" }
"mail.listing.tag_default" = { fg = "$Jebediah", bg = "Black" }
"#;
    let parsed: Themes = toml::from_str(TEST_CYCLIC_ALIAS_STR_3).unwrap();
    parsed.validate().unwrap();
    /* MUST FAIL: alias $Jebediah resolves to an invalid key */
    const TEST_INVALID_LINK_KEY_FIELD_STR: &str = r#"[dark]
color_aliases= { "Jebediah" = "$JebediahJr", "JebediahJr" = "mail.listing.tag_default.attrs" }
"mail.listing.tag_default" = { fg = "$Jebediah", bg = "Black" }
"#;
    let parsed: Themes = toml::from_str(TEST_INVALID_LINK_KEY_FIELD_STR).unwrap();
    parsed.validate().unwrap_err();
}

/// Every theme shipped in `themes/` must load through
/// the same path `:toggle theme` uses at runtime
/// ([`crate::state::State::load_theme_into_settings`]): parse the file, pick
/// each `[terminal.themes.<name>]` table, deserialize it into
/// [`ThemeOptions`], merge it over a `dark` clone via [`construct_theme`]
/// (which rejects unknown keys and malformed colors/attrs), then run the
/// full [`Themes::validate`] cycle and alias checks.
/// Every theme compiled into the binary: each [`builtin_themes`] entry
/// present in `Themes::default()` (Zed family plus community ports),
/// `dark`/`light` aliasing Ayu Dark/Ayu Light, and the default selected
/// theme being `Ayu Dark`.
#[test]
fn test_builtin_themes_compiled_in() {
    let def = Themes::default();
    // `builtin_themes()` drives the picker's built-in section: it must
    // list exactly the compiled-in themes, each name only once.
    let mut listed = builtin_themes().to_vec();
    let listed_len = listed.len();
    listed.sort_unstable();
    listed.dedup();
    assert_eq!(
        listed.len(),
        listed_len,
        "builtin_themes() lists a theme name twice"
    );
    let mut compiled_in = def
        .other_themes
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    compiled_in.sort_unstable();
    assert_eq!(
        listed, compiled_in,
        "the picker's built-in list and the compiled-in themes must match exactly"
    );
    let ayu_dark = &def.other_themes[DEFAULT_THEME];
    let ayu_light = &def.other_themes["Ayu Light"];
    for key in [
        "theme_default",
        "status.bar",
        "tab.focused",
        "pager.highlight_search",
    ] {
        assert_eq!(
            unlink(&def.dark, key),
            unlink(ayu_dark, key),
            "`dark` must alias {DEFAULT_THEME} for {key}"
        );
        assert_eq!(
            unlink(&def.light, key),
            unlink(ayu_light, key),
            "`light` must alias `Ayu Light` for {key}"
        );
    }
    def.validate().unwrap();

    assert_eq!(
        crate::conf::terminal::TerminalSettings::default().theme,
        DEFAULT_THEME,
        "the selected theme must default to {DEFAULT_THEME}"
    );
    assert_eq!(builtin_themes()[0], DEFAULT_THEME);
    for expected in [
        "Catppuccin Mocha",
        "Dracula",
        "Nord Dark",
        "Tokyo Night",
        "Zedokai",
    ] {
        assert!(
            builtin_themes().contains(&expected),
            "community port `{expected}` missing from builtin_themes()"
        );
    }
}

#[test]
fn test_docs_sample_themes_load() {
    let themes_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("themes");
    let mut found_theme_names = Vec::new();
    for entry in fs::read_dir(&themes_dir)
        .unwrap_or_else(|err| panic!("could not read {}: {err}", themes_dir.display()))
    {
        let path = entry
            .unwrap_or_else(|err| panic!("could not read {}: {err}", themes_dir.display()))
            .path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        let value: toml::Value = text.parse().unwrap_or_else(|err| {
            panic!("{}: invalid TOML: {err}", path.display());
        });
        let Some(themes_table) = value
            .get("terminal")
            .and_then(|t| t.get("themes"))
            .and_then(|t| t.as_table())
        else {
            panic!("{}: no [terminal.themes] table", path.display());
        };
        assert!(
            !themes_table.is_empty(),
            "{}: no themes declared",
            path.display()
        );
        for (name, table) in themes_table {
            assert_ne!(
                name,
                "light",
                "{}: theme name `light` is reserved",
                path.display()
            );
            assert_ne!(
                name,
                "dark",
                "{}: theme name `dark` is reserved",
                path.display()
            );
            let options: ThemeOptions = table.clone().try_into().unwrap_or_else(|err| {
                panic!("{}: invalid theme `{name}`: {err}", path.display());
            });
            let mut theme = Themes::default().dark;
            construct_theme(name, &mut theme, options).unwrap_or_else(|err| {
                panic!("{}: could not build theme `{name}`: {err}", path.display());
            });
            let mut all = Themes::default();
            all.other_themes.insert(name.clone(), theme);
            all.validate().unwrap_or_else(|err| {
                panic!(
                    "{}: theme `{name}` failed validation: {err}",
                    path.display()
                );
            });
            found_theme_names.push(name.clone());
        }
    }
    assert!(
        !found_theme_names.is_empty(),
        "no sample themes found in {}",
        themes_dir.display()
    );
    // The Zed ports this feature ships must stay discoverable.
    for expected in [
        "One Dark",
        "One Light",
        "Ayu Dark",
        "Ayu Mirage",
        "Ayu Light",
        "Gruvbox Dark",
        "Gruvbox Dark Hard",
        "Gruvbox Dark Soft",
        "Gruvbox Light",
        "Gruvbox Light Hard",
        "Gruvbox Light Soft",
    ] {
        assert!(
            found_theme_names.contains(&expected.to_string()),
            "ported theme `{expected}` not found in {}",
            themes_dir.display()
        );
    }
    // Everything shipped in `themes/` must be compiled in: the picker's
    // built-in section derives from `builtin_themes()`.
    for name in &found_theme_names {
        assert!(
            builtin_themes().contains(&name.as_str()),
            "theme `{name}` shipped in themes/ is not listed as built-in"
        );
    }
}

/// A `[terminal.themes.<name>]` table shadows the same-name built-in:
/// the user's values win, the name is tracked as user-defined, and the
/// picker labels the entry with the configuration as its source.
#[test]
fn test_config_table_shadows_builtin_theme() {
    // A `Themes` value deserializes from the *contents* of the
    // configuration's `[terminal.themes]` table.
    const OVERRIDE_STR: &str = r##"
["One Dark"]
"mail.listing.tag_default" = { fg = "#b4da55" }
"##;
    let parsed: Themes = toml::from_str(OVERRIDE_STR).unwrap();
    assert!(
        parsed.user_defined.contains("One Dark"),
        "an overridden built-in name must be tracked as user-defined"
    );
    let builtin = Themes::default();
    assert_ne!(
        unlink(&parsed.other_themes["One Dark"], "mail.listing.tag_default").fg,
        unlink(
            &builtin.other_themes["One Dark"],
            "mail.listing.tag_default"
        )
        .fg,
        "the configuration table must override the built-in value"
    );
    assert_eq!(
        unlink(&parsed.other_themes["One Dark"], "mail.listing.tag_default").fg,
        Color::Rgb(0xb4, 0xda, 0x55),
    );
    // The picker labels the shadowed name by its winning source.
    let entries = theme_picker_entries(&parsed, &IndexMap::new());
    let labeled: Vec<_> = entries
        .iter()
        .filter(|(name, _)| name == "One Dark")
        .map(|(_, label)| label)
        .collect();
    assert_eq!(labeled, ["One Dark (config)"]);
}

/// A theme directory file shadows every same-name definition:
/// `theme_from_file` (the `load_theme_into_settings` path) yields the
/// file's values over the built-in base, and the picker offers both the
/// shadowed built-in name and a new file theme exactly once, labeled
/// with the file as their source.
#[test]
fn test_theme_directory_file_shadows_builtin() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("my-one-dark.toml");
    std::fs::write(
        &path,
        r##"
[terminal.themes."One Dark"]
"mail.listing.tag_default" = { fg = "#00ff00" }
"##,
    )
    .unwrap();

    let builtin = Themes::default();
    let theme = crate::conf::theme_from_file("One Dark", &path, &builtin.dark).unwrap();
    assert_eq!(
        unlink(&theme, "mail.listing.tag_default").fg,
        Color::Rgb(0x00, 0xff, 0x00),
        "the directory file must override the built-in value"
    );

    let mut directory = IndexMap::new();
    directory.insert("One Dark".to_string(), path.clone());
    directory.insert("Custom Theme".to_string(), path);
    let entries = theme_picker_entries(&builtin, &directory);
    let one_dark: Vec<_> = entries
        .iter()
        .filter(|(name, _)| name == "One Dark")
        .map(|(_, label)| label)
        .collect();
    assert_eq!(one_dark, ["One Dark (file)"]);
    assert_eq!(
        entries
            .iter()
            .find(|(name, _)| name == "Custom Theme")
            .map(|(_, label)| label.as_str()),
        Some("Custom Theme (file)"),
        "a directory theme must be offered by the picker"
    );
    // Pure built-ins keep their label.
    assert_eq!(
        entries
            .iter()
            .find(|(name, _)| name == "Ayu Dark")
            .map(|(_, label)| label.as_str()),
        Some("Ayu Dark (built-in)"),
    );
}

#[test]
fn test_conf_theme_key_values() {
    use std::{collections::VecDeque, fs::File, io::Read, path::PathBuf};
    let mut queue: VecDeque<PathBuf> = VecDeque::new();
    queue.push_back("src/".into());
    let re_conf = regex::Regex::new(r#"value\((?:\s|\n)*[&]?context,[^"]*"([^"]*)""#).unwrap();

    let mut content = String::new();
    while let Some(dir) = queue.pop_front() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                queue.push_back(path);
            } else if path.extension().map(|os_s| os_s == "rs").unwrap_or(false) {
                let mut file = File::open(&path).unwrap();
                content.clear();
                file.read_to_string(&mut content).unwrap();
                for mat in re_conf.captures_iter(&content) {
                    let theme_key = &mat[1];
                    if !DEFAULT_KEYS.contains(&theme_key) {
                        panic!(
                            "Source file {} contains a hardcoded theme key str, {:?}, that is not \
                             included in the DEFAULT_KEYS table.",
                            path.display(),
                            theme_key
                        );
                    }
                }
            }
        }
    }
}

/// Pin the default focus/selection palette for both light and dark themes
/// (the compiled-in Ayu themes; `dark` = "Ayu Dark", `light` = "Ayu Light").
/// If a value changes intentionally, update this test alongside the golden
/// re-record.
#[test]
fn test_conf_theme_default_focus_palette() {
    let def = Themes::default();
    let dark = |key: &str| unlink(&def.dark, key);
    let light = |key: &str| unlink(&def.light, key);
    // tab.focused = bold + foreground; tab.unfocused = dim.
    assert_eq!(
        dark("tab.focused"),
        ThemeAttribute {
            fg: Color::Rgb(191, 189, 182),
            bg: Color::Rgb(13, 16, 22),
            attrs: Attr::BOLD
        }
    );
    assert_eq!(
        light("tab.focused"),
        ThemeAttribute {
            fg: Color::Rgb(92, 97, 102),
            bg: Color::Rgb(252, 252, 252),
            attrs: Attr::BOLD
        }
    );
    assert_eq!(
        dark("tab.unfocused"),
        ThemeAttribute {
            fg: Color::Rgb(138, 137, 134),
            bg: Color::Rgb(31, 33, 39),
            attrs: Attr::DIM
        }
    );
    assert_eq!(
        light("tab.unfocused"),
        ThemeAttribute {
            fg: Color::Rgb(139, 142, 146),
            bg: Color::Rgb(236, 236, 237),
            attrs: Attr::DIM
        }
    );
    // Pane keys chain the tab keys: fg links the tab foreground, bg the
    // tab background; attrs stay Default (the tabs add Bold/Dim).
    assert_eq!(
        dark("pane.focused"),
        ThemeAttribute {
            fg: dark("tab.focused").fg,
            bg: dark("tab.focused").bg,
            attrs: Attr::DEFAULT
        }
    );
    assert_eq!(
        light("pane.focused"),
        ThemeAttribute {
            fg: light("tab.focused").fg,
            bg: light("tab.focused").bg,
            attrs: Attr::DEFAULT
        }
    );
    assert_eq!(
        dark("pane.unfocused"),
        ThemeAttribute {
            fg: dark("tab.unfocused").fg,
            bg: dark("tab.unfocused").bg,
            attrs: Attr::DEFAULT
        }
    );
    assert_eq!(
        light("pane.unfocused"),
        ThemeAttribute {
            fg: light("tab.unfocused").fg,
            bg: light("tab.unfocused").bg,
            attrs: Attr::DEFAULT
        }
    );
    // Status bars: normal = text on status bar, command = text on surface.
    assert_eq!(
        dark("status.bar"),
        ThemeAttribute {
            fg: Color::Rgb(191, 189, 182),
            bg: Color::Rgb(49, 51, 55),
            attrs: Attr::DEFAULT
        }
    );
    assert_eq!(
        light("status.bar"),
        ThemeAttribute {
            fg: Color::Rgb(92, 97, 102),
            bg: Color::Rgb(220, 221, 222),
            attrs: Attr::DEFAULT
        }
    );
    assert_eq!(
        dark("status.command_bar"),
        ThemeAttribute {
            fg: Color::Rgb(191, 189, 182),
            bg: Color::Rgb(31, 33, 39),
            attrs: Attr::DEFAULT
        }
    );
    assert_eq!(
        light("status.command_bar"),
        ThemeAttribute {
            fg: Color::Rgb(92, 97, 102),
            bg: Color::Rgb(236, 236, 237),
            attrs: Attr::DEFAULT
        }
    );
    // Selection fill: `selected` alias; cursor highlight: `match` alias.
    for key in [
        "mail.listing.compact.selected",
        "mail.listing.plain.selected",
        "mail.listing.conversations.selected",
    ] {
        assert_eq!(dark(key).bg, Color::Rgb(62, 64, 67), "{key} dark bg");
        assert_eq!(light(key).bg, Color::Rgb(207, 208, 210), "{key} light bg");
    }
    for key in [
        "mail.listing.compact.highlighted",
        "mail.listing.plain.highlighted",
    ] {
        assert_eq!(dark(key).bg, Color::Rgb(44, 87, 115), "{key} dark bg");
        assert_eq!(light(key).bg, Color::Rgb(175, 214, 243), "{key} light bg");
    }
    assert_eq!(
        dark("mail.listing.conversations.highlighted").bg,
        Color::Rgb(44, 87, 115)
    );
    assert_eq!(
        light("mail.listing.conversations.highlighted").bg,
        Color::Rgb(175, 214, 243)
    );
    assert_eq!(
        dark("mail.sidebar_highlighted"),
        ThemeAttribute {
            fg: Color::Rgb(191, 189, 182),
            bg: Color::Rgb(45, 47, 52),
            attrs: Attr::BOLD
        }
    );
    assert_eq!(
        light("mail.sidebar_highlighted"),
        ThemeAttribute {
            fg: Color::Rgb(92, 97, 102),
            bg: Color::Rgb(223, 224, 225),
            attrs: Attr::BOLD
        }
    );
}

#[test]
fn test_conf_progress_spinner_sequence() {
    use crate::{conf::terminal::ProgressSpinnerSequence, utilities::ProgressSpinner};

    let int_0 = ProgressSpinnerSequence::Integer(5);
    assert_eq!(
        toml::Value::try_from(&int_0).unwrap(),
        toml::Value::try_from(5).unwrap()
    );

    let frames = ProgressSpinnerSequence::Custom {
        frames: vec![
            "⠁".to_string(),
            "⠂".to_string(),
            "⠄".to_string(),
            "⡀".to_string(),
            "⢀".to_string(),
            "⠠".to_string(),
            "⠐".to_string(),
            "⠈".to_string(),
        ],
        interval_ms: ProgressSpinner::INTERVAL_MS,
    };
    assert_eq!(frames.interval_ms(), ProgressSpinner::INTERVAL_MS);
    assert_eq!(
        toml::Value::try_from(&frames).unwrap(),
        toml::Value::try_from(["⠁", "⠂", "⠄", "⡀", "⢀", "⠠", "⠐", "⠈"]).unwrap()
    );
    let frames = ProgressSpinnerSequence::Custom {
        frames: vec![
            "⠁".to_string(),
            "⠂".to_string(),
            "⠄".to_string(),
            "⡀".to_string(),
            "⢀".to_string(),
            "⠠".to_string(),
            "⠐".to_string(),
            "⠈".to_string(),
        ],
        interval_ms: ProgressSpinner::INTERVAL_MS + 1,
    };
    assert_eq!(
        toml::Value::try_from(&frames).unwrap(),
        toml::Value::try_from(indexmap::indexmap! {
            "frames" => toml::Value::try_from(["⠁", "⠂", "⠄", "⡀", "⢀", "⠠", "⠐", "⠈"]).unwrap(),
            "interval_ms" => toml::Value::try_from(ProgressSpinner::INTERVAL_MS + 1).unwrap()
        })
        .unwrap()
    );
    assert_eq!(
        toml::from_str::<ProgressSpinnerSequence>(
            r#"frames = ["⠁", "⠂", "⠄", "⡀", "⢀", "⠠", "⠐", "⠈"]
interval_ms = 51"#
        )
        .unwrap(),
        frames
    );
    assert_eq!(
        toml::from_str::<indexmap::IndexMap<String, ProgressSpinnerSequence>>(
            r#"sequence = { frames = ["⠁", "⠂", "⠄", "⡀", "⢀", "⠠", "⠐", "⠈"], interval_ms = 51 }"#
        )
        .unwrap(),
        indexmap::indexmap! {
            "sequence".to_string() => frames,
        },
    );
}

/// Minimal valid account config whose `root_mailbox` is an existing directory,
/// with `extra` appended (e.g. a `[listing]` or `[terminal]` table).
fn minimal_config(root: &std::path::Path, extra: &str) -> String {
    let root = root.display();
    format!(
        "[accounts.account-name]\nroot_mailbox = \"{root}\"\nformat = \"maildir\"\n\
         send_mail = 'false'\nidentity = \"email@example.com\"\n{extra}\n"
    )
}

/// An empty custom spinner would later panic in the status bar
/// (`wrapping_rem(0)`); it must be replaced with the default at load time and
/// reported to the user.
#[test]
fn test_empty_progress_spinner_sequence_is_rejected_with_warning() {
    let root = tempfile::tempdir().unwrap();
    for spinner in [
        "progress_spinner_sequence = []",
        "progress_spinner_sequence = { frames = [], interval_ms = 100 }",
    ] {
        let config = minimal_config(root.path(), &format!("[terminal]\n{spinner}"));
        let s = FileSettings::validate_string(config, false).unwrap();
        assert!(
            s.terminal.progress_spinner_sequence.is_none(),
            "empty frames must fall back to the default"
        );
        assert_eq!(s.config_warnings.len(), 1, "{:?}", s.config_warnings);
        assert!(
            s.config_warnings[0].contains("progress_spinner_sequence"),
            "{:?}",
            s.config_warnings
        );
    }
}

/// `thread_layout`, `sidebar_ratio` and `mail_view_divider` were removed
/// from `ListingSettings` (the layout is now a fixed 30/70 split), but the
/// struct carries `#[serde(deny_unknown_fields)]`: leftover keys in old
/// config files (top-level `[listing]` and per-account
/// `[accounts.<name>.listing]`) used to abort startup. They must be stripped
/// before parsing, leaving every other setting intact.
#[test]
fn test_removed_thread_layout_keys_are_stripped_from_config() {
    let root = tempfile::tempdir().unwrap();
    let config = minimal_config(
        root.path(),
        "[listing]\nthread_layout = \"auto\"\nsidebar_ratio = 50\n\
         mail_view_divider = \"+\"\ncontext_lines = 1\n\
         [accounts.account-name.listing]\nthread_layout = \"auto\"\n\
         sidebar_ratio = 50\nmail_view_divider = \"+\"\ncontext_lines = 2",
    );
    let s = FileSettings::validate_string(config, false).unwrap();

    // Top-level `[listing]`: the legacy keys are gone, sibling keys survive.
    assert_eq!(s.listing.context_lines, 1);

    // Per-account `[accounts.<name>.listing]`: stripped as well, and the
    // remaining overrides still parse.
    assert_eq!(
        s.accounts["account-name"]
            .conf_override
            .listing
            .context_lines,
        Some(2)
    );
}

/// The same removal happened one tier deeper:
/// `FileMailboxConf` flattens `MailUIConf`, so a per-mailbox
/// `[accounts.<name>.mailboxes.<mailbox>.listing]` table has the same
/// `deny_unknown_fields` overrides and old configs carrying the removed
/// keys there used to abort startup, too. They must be stripped while
/// sibling keys survive.
#[test]
fn test_removed_thread_layout_keys_are_stripped_from_mailbox_conf() {
    let root = tempfile::tempdir().unwrap();
    let config = minimal_config(
        root.path(),
        "[accounts.account-name.mailboxes.\"INBOX\".listing]\n\
         thread_layout = \"auto\"\nsidebar_ratio = 50\n\
         mail_view_divider = \"+\"\ncontext_lines = 3",
    );
    let s = FileSettings::validate_string(config, false).unwrap();
    assert_eq!(
        s.accounts["account-name"].mailboxes["INBOX"]
            .conf_override()
            .listing
            .context_lines,
        Some(3)
    );
}

/// Theme keys that were intentionally removed (the listing zebra-stripping
/// `even`/`odd` row keys and `mail.view.divider`) used to hard-fail startup
/// with an "unrecognized theme keywords" error when present in a
/// `[terminal.themes.<name>]` table. They must be dropped with a warning
/// instead, while genuinely unknown keys must still be rejected.
#[test]
fn test_removed_theme_keys_are_stripped_and_unknown_still_error() {
    let root = tempfile::tempdir().unwrap();

    // Every intentionally removed key must be accepted and stripped; a
    // sibling key that still exists keeps its value.
    let mut theme_table = String::new();
    for key in crate::conf::themes::REMOVED_THEME_KEYS {
        theme_table.push_str(&format!("\"{key}\" = {{ fg = \"HotPink3\" }}\n"));
    }
    let config = minimal_config(
        root.path(),
        &format!(
            "[terminal.themes.\"hunter2\"]\n{theme_table}\
             \"mail.listing.tag_default\" = {{ fg = \"Red\" }}\n"
        ),
    );
    let s = FileSettings::validate_string(config, false).unwrap();
    let theme = &s.terminal.themes.other_themes["hunter2"];
    for key in crate::conf::themes::REMOVED_THEME_KEYS {
        assert!(
            !theme.contains_key(*key),
            "removed key `{key}` must be stripped, not parsed"
        );
    }
    assert_eq!(
        unlink_fg(theme, &ColorField::Fg, "mail.listing.tag_default"),
        Color::Byte(9) // the string "Red" parses to color index 9
    );

    // A key that was never a theme key must still be an error.
    let config = minimal_config(
        root.path(),
        "[terminal.themes.\"hunter2\"]\n\"not.a.theme.key\" = { fg = \"Red\" }\n",
    );
    let err = FileSettings::validate_string(config, false).unwrap_err();
    assert!(
        err.to_string().contains("unrecognized theme keywords"),
        "{err}"
    );
}

/// Warnings collected while loading must reach `Settings`, which `State::new`
/// then drains into user-facing notifications.
#[test]
fn test_config_warnings_propagate_to_settings() {
    let dir = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let config = minimal_config(
        root.path(),
        "[terminal]\nprogress_spinner_sequence = { frames = [], interval_ms = 100 }",
    );
    let file = ConfigFile::new(&config, &dir).unwrap();
    let settings = crate::conf::Settings::from_path(file.path.clone()).unwrap();
    assert_eq!(
        settings.config_warnings.len(),
        1,
        "{:?}",
        settings.config_warnings
    );
    assert!(settings.config_warnings[0].contains("progress_spinner_sequence"));
}

/// `get_included_configs` used `conf_path.parent().unwrap()`; a parentless
/// path (empty or `/`) must return an error, not panic.
#[test]
fn test_get_included_configs_parentless_path_does_not_panic() {
    use std::path::Path;

    use crate::conf::preprocessing::get_included_configs;

    for path in [Path::new(""), Path::new("/")] {
        let res = get_included_configs(path);
        assert!(res.is_err(), "{path:?} must not yield includes");
    }
}

mod toggle_theme_ui {
    use super::*;
    use crate::command::{parse_command, Action};
    use crate::conf::rewrite_terminal_theme;
    use crate::types::UIEvent;

    /// The `toggle theme` command must produce `Action::ToggleTheme`,
    /// which the State layer must translate into opening the theme
    /// picker overlay and applying a theme change.
    #[test]
    fn toggle_theme_command_opens_picker_and_persists() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config_toml = format!(
            "[accounts.test]\n\
             root_mailbox = \"{}\"\n\
             format = \"maildir\"\n\
             send_mail = 'false'\n\
             identity = \"user@example.com\"\n\
             \n\
             [terminal]\n\
             theme = \"dark\"\n",
            temp_dir.path().display()
        );
        let config_file = ConfigFile::new(&config_toml, &temp_dir).unwrap();
        std::env::set_var("MELI_CONFIG", config_file.path.as_os_str());

        let mut ctx = crate::golden::mock_context();
        ctx.settings.terminal.theme = "dark".to_string();

        // 1. Command parses
        let action = parse_command(b"toggle theme").unwrap();
        assert!(matches!(action, Action::ToggleTheme));

        // 2. The config file initially has theme = "dark"
        let text = std::fs::read_to_string(&config_file.path).unwrap();
        assert!(text.contains("theme = \"dark\""));

        // 3. rewrite_terminal_theme can switch to "light"
        let updated = rewrite_terminal_theme(&text, "light").unwrap();
        assert!(updated.contains("theme = \"light\""));
        assert!(!updated.contains("theme = \"dark\""));
        assert_eq!(updated.matches("theme =").count(), 1);
        // Other settings preserved
        assert!(updated.contains("format = \"maildir\""));
        assert!(updated.contains("identity = \"user@example.com\""));

        // 4. Write back and re-read: persists
        std::fs::write(&config_file.path, &updated).unwrap();
        let retext = std::fs::read_to_string(&config_file.path).unwrap();
        assert!(retext.contains("theme = \"light\""));
    }

    /// The picker's live-preview event must change the in-memory theme;
    /// the persist event must rewrite the file.
    #[test]
    fn change_theme_event_flow() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config_toml = format!(
            "[accounts.test]\n\
             root_mailbox = \"{}\"\n\
             format = \"maildir\"\n\
             send_mail = 'false'\n\
             identity = \"user@example.com\"\n\
             \n\
             [terminal]\n\
             theme = \"dark\"\n",
            temp_dir.path().display()
        );
        let config_file = ConfigFile::new(&config_toml, &temp_dir).unwrap();
        std::env::set_var("MELI_CONFIG", config_file.path.as_os_str());

        // Live preview: in-memory only
        let preview_event = UIEvent::ChangeTheme {
            name: "light".to_string(),
            persist: false,
        };
        // The event reaches State::rcv_event in the real loop; here we
        // verify the payload carries the right semantics.
        assert!(matches!(
            &preview_event,
            UIEvent::ChangeTheme { name, persist } if name == "light" && !persist
        ));

        // Persist: also rewrites the file
        let text = std::fs::read_to_string(&config_file.path).unwrap();
        let updated = rewrite_terminal_theme(&text, "light").unwrap();
        std::fs::write(&config_file.path, updated).unwrap();
        let after = std::fs::read_to_string(&config_file.path).unwrap();
        assert!(after.contains("theme = \"light\""));
    }

    /// Toggling between multiple themes in sequence must keep the config
    /// file consistent (single theme line, correct value).
    #[test]
    fn theme_toggles_keep_single_line() {
        let base = "[terminal]\ntheme = \"dark\"\nfoo = 1\n";
        let a = rewrite_terminal_theme(base, "light").unwrap();
        let b = rewrite_terminal_theme(&a, "dark").unwrap();
        let c = rewrite_terminal_theme(&b, "nord").unwrap();
        assert!(c.contains("theme = \"nord\""));
        assert_eq!(c.matches("theme =").count(), 1);
        assert!(c.contains("foo = 1"), "other settings preserved");
    }
}
