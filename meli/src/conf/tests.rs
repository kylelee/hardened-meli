//
// meli
//
// Copyright 2024 Emmanouil Pitsidianakis <manos@pitsidianak.is>
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
        "Up/k"
    );
    assert_eq!(
        override_.general.as_ref().unwrap().quit.to_string(),
        "Esc/q"
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
        "general.scroll_left must default to Left/h"
    );
    assert_eq!(
        general.get("scroll_right"),
        Some(&ShortcutKeys::double(Key::Right, Key::Char('l'))),
        "general.scroll_right must default to Right/l"
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
            "{section}.scroll_up must default to Up/k"
        );
        assert_eq!(
            values.get("scroll_down"),
            Some(&ShortcutKeys::double(Key::Down, Key::Char('j'))),
            "{section}.scroll_down must default to Down/j"
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
"mail.listing.compact.even" = { fg = "mail.listing.compact.odd" }
"mail.listing.compact.odd" = { fg = "mail.listing.compact.even" }
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
/// (todo: modern focus/selection defaults). If a value changes intentionally,
/// update this test alongside the golden re-record.
#[test]
fn test_conf_theme_default_focus_palette() {
    let def = Themes::default();
    let dark = |key: &str| unlink(&def.dark, key);
    let light = |key: &str| unlink(&def.light, key);
    // tab.focused = bold + accent fg; tab.unfocused = dim.
    assert_eq!(
        dark("tab.focused"),
        ThemeAttribute {
            fg: Color::Byte(123),
            bg: Color::Default,
            attrs: Attr::BOLD
        }
    );
    assert_eq!(
        light("tab.focused"),
        ThemeAttribute {
            fg: Color::Byte(31),
            bg: Color::Default,
            attrs: Attr::BOLD
        }
    );
    for theme in [dark("tab.unfocused"), light("tab.unfocused")] {
        assert_eq!(
            theme,
            ThemeAttribute {
                fg: Color::Byte(244),
                bg: Color::Default,
                attrs: Attr::DIM
            }
        );
    }
    // Status bars: normal = accent on subtle dark, command = amber.
    assert_eq!(
        dark("status.bar"),
        ThemeAttribute {
            fg: Color::Byte(123),
            bg: Color::Byte(235),
            attrs: Attr::DEFAULT
        }
    );
    assert_eq!(
        light("status.bar"),
        ThemeAttribute {
            fg: Color::Byte(31),
            bg: Color::Byte(254),
            attrs: Attr::DEFAULT
        }
    );
    for theme in [dark("status.command_bar"), light("status.command_bar")] {
        assert_eq!(
            theme,
            ThemeAttribute {
                fg: Color::Byte(16),
                bg: Color::Byte(214),
                attrs: Attr::DEFAULT
            }
        );
    }
    // Selection fill: steel blue (dark) / light blue (light);
    // cursor highlight: grey ramp.
    for key in [
        "mail.listing.compact.even_selected",
        "mail.listing.compact.odd_selected",
        "mail.listing.plain.even_selected",
        "mail.listing.plain.odd_selected",
        "mail.listing.conversations.selected",
    ] {
        assert_eq!(dark(key).bg, Color::Byte(24), "{key} dark bg");
        assert_eq!(light(key).bg, Color::Byte(153), "{key} light bg");
    }
    for key in [
        "mail.listing.compact.even_highlighted",
        "mail.listing.compact.odd_highlighted",
        "mail.listing.plain.even_highlighted",
        "mail.listing.plain.odd_highlighted",
    ] {
        assert_eq!(dark(key).bg, Color::Byte(240), "{key} dark bg");
        assert_eq!(light(key).bg, Color::Byte(189), "{key} light bg");
    }
    assert_eq!(
        dark("mail.listing.conversations.highlighted").bg,
        Color::Byte(240)
    );
    assert_eq!(
        light("mail.listing.conversations.highlighted").bg,
        Color::Byte(189)
    );
    assert_eq!(
        dark("mail.sidebar_highlighted"),
        ThemeAttribute {
            fg: Color::Byte(16),
            bg: Color::Byte(123),
            attrs: Attr::DEFAULT
        }
    );
    assert_eq!(
        light("mail.sidebar_highlighted"),
        ThemeAttribute {
            fg: Color::Byte(16),
            bg: Color::Byte(123),
            attrs: Attr::DEFAULT
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

/// `sidebar_ratio > 100` underflows the listing layout arithmetic later; it
/// must be clamped at load time and reported.
#[test]
fn test_sidebar_ratio_out_of_range_is_clamped_with_warning() {
    let root = tempfile::tempdir().unwrap();
    let config = minimal_config(root.path(), "[listing]\nsidebar_ratio = 200");
    let s = FileSettings::validate_string(config, false).unwrap();
    assert_eq!(s.listing.sidebar_ratio, 100);
    assert_eq!(s.config_warnings.len(), 1, "{:?}", s.config_warnings);
    assert!(
        s.config_warnings[0].contains("sidebar_ratio"),
        "{:?}",
        s.config_warnings
    );

    // Valid values are untouched and produce no warning.
    let config = minimal_config(root.path(), "[listing]\nsidebar_ratio = 50");
    let s = FileSettings::validate_string(config, false).unwrap();
    assert_eq!(s.listing.sidebar_ratio, 50);
    assert!(s.config_warnings.is_empty());
}

/// Warnings collected while loading must reach `Settings`, which `State::new`
/// then drains into user-facing notifications.
#[test]
fn test_config_warnings_propagate_to_settings() {
    let dir = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let config = minimal_config(root.path(), "[listing]\nsidebar_ratio = 200");
    let file = ConfigFile::new(&config, &dir).unwrap();
    let settings = crate::conf::Settings::from_path(file.path.clone()).unwrap();
    assert_eq!(settings.listing.sidebar_ratio, 100);
    assert_eq!(
        settings.config_warnings.len(),
        1,
        "{:?}",
        settings.config_warnings
    );
    assert!(settings.config_warnings[0].contains("sidebar_ratio"));
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
