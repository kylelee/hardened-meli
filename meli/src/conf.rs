/*
 * meli - configuration module.
 *
 * Copyright 2017 Manos Pitsidianakis
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

//! Configuration logic and `config.toml` interfaces.

extern crate serde;
extern crate toml;
extern crate xdg;

use std::{
    env,
    fs::OpenOptions,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{conf::deserializers::non_empty_opt_string, terminal::Color};
use indexmap::IndexMap;
use melib::{
    backends::MailboxHash,
    conf::{ActionFlag, MailboxConf, Secret, ToggleFlag},
    error::*,
    search::Query,
    Logger, ShellExpandTrait, SortField, SortOrder,
};

pub mod default_values;
pub mod preprocessing;
use preprocessing as pp;

pub mod data_types;
#[cfg(test)]
pub mod tests;
#[rustfmt::skip]
mod overrides;
pub use overrides::*;
pub mod composing;
pub mod notifications;
pub mod pager;
pub mod pgp;
pub mod tags;
#[macro_use]
pub mod shortcuts;
mod listing;
pub mod terminal;
mod themes;
use default_values::*;
pub use themes::*;

pub use self::{composing::*, pgp::*, shortcuts::*, tags::*};

/// Utility macro to access an [`AccountConf`] setting field from
/// [`Context`](crate::Context) indexed by `$account_hash`
///
/// The value returned is the optionally overridden one in the
/// [`AccountConf::conf_override`] field, otherwise the global one.
///
/// See also the [`mailbox_settings`](crate::mailbox_settings) macro.
#[macro_export]
macro_rules! account_settings {
    ($context:ident[$account_hash:expr].$setting:ident.$field:ident) => {{
        $context.accounts[&$account_hash]
            .settings
            .conf_override
            .$setting
            .$field
            .as_ref()
            .unwrap_or(&$context.settings.$setting.$field)
    }};
    ($context:ident[$account_hash:expr].$field:ident) => {{
        &$context.accounts[&$account_hash].settings.$field
    }};
}

/// Utility macro to access an [`AccountConf`] setting field from
/// [`Context`](crate::Context) indexed by `$account_hash` and a mailbox.
///
/// The value returned is the optionally overridden one in the
/// [`FileMailboxConf::conf_override`] field, otherwise the
/// [`AccountConf::conf_override`] field, otherwise the global one.
///
/// See also the [`account_settings`] macro.
#[macro_export]
macro_rules! mailbox_settings {
    ($context:ident has [$account_hash:expr][$mailbox_path:expr]) => {{
        if let Some(ref acc) = $context.accounts.get(&$account_hash) {
            acc.mailbox_entries.contains_key($mailbox_path)
        } else {
            false
        }
    }};
    ($context:ident[$account_hash:expr][$mailbox_path:expr].$setting:ident.$field:ident) => {{
        $context.accounts[&$account_hash][$mailbox_path]
            .conf
            .conf_override
            .$setting
            .$field
            .as_ref()
            .or($context.accounts[&$account_hash]
                .settings
                .conf_override
                .$setting
                .$field
                .as_ref())
            .unwrap_or(&$context.settings.$setting.$field)
    }};
    ($context:ident[$account_hash:expr][$mailbox_path:expr].$setting:ident.$field:ident.$subfield:ident) => {{
        $context.accounts[&$account_hash][$mailbox_path]
            .conf
            .conf_override
            .$setting
            .$field
            .as_ref()
            .map(|f| f.$subfield.as_ref())
            .or($context.accounts[&$account_hash]
                .settings
                .conf_override
                .$setting
                .$field
                .as_ref()
                .map(|f| f.$subfield.as_ref()))
            .unwrap_or(&$context.settings.$setting.$field.$subfield)
    }};
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MailUIConf {
    pub send_mail: Option<SendMail>,
    #[serde(default)]
    pub pager: PagerSettingsOverride,
    #[serde(default)]
    pub listing: ListingSettingsOverride,
    #[serde(default)]
    pub notifications: NotificationsSettingsOverride,
    #[serde(default)]
    pub shortcuts: ShortcutsOverride,
    #[serde(default)]
    pub composing: ComposingSettingsOverride,
    #[serde(default)]
    pub identity: Option<String>,
    #[serde(default)]
    pub tags: TagsSettingsOverride,
    #[serde(default)]
    pub themes: Option<Themes>,
    #[serde(default)]
    pub pgp: PGPSettingsOverride,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct FileMailboxConf {
    #[serde(flatten)]
    pub conf_override: MailUIConf,
    #[serde(default = "false_val")]
    pub collapsed: bool,
    #[serde(flatten)]
    pub mailbox_conf: MailboxConf,
}

impl FileMailboxConf {
    pub fn conf_override(&self) -> &MailUIConf {
        &self.conf_override
    }

    pub fn mailbox_conf(&self) -> &MailboxConf {
        &self.mailbox_conf
    }
}

use crate::conf::deserializers::extra_settings;
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct FileAccount {
    pub root_mailbox: String,
    /// The mailbox that is the default to open / view for this account. Must be
    /// a valid mailbox path.
    ///
    /// If not specified, the default is [`Self::root_mailbox`].
    #[serde(default = "none", skip_serializing_if = "Option::is_none")]
    pub default_mailbox: Option<String>,
    pub format: String,
    pub send_mail: SendMail,
    pub identity: String,
    #[serde(default)]
    pub extra_identities: Vec<String>,
    #[serde(default = "none", skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default = "false_val")]
    pub read_only: bool,
    #[serde(default)]
    pub subscribed_mailboxes: Vec<String>,
    #[serde(default)]
    pub mailboxes: IndexMap<String, FileMailboxConf>,
    #[serde(default)]
    pub search_backend: data_types::SearchBackend,
    #[serde(default = "false_val")]
    pub manual_refresh: bool,
    #[serde(default = "none", skip_serializing_if = "Option::is_none")]
    pub refresh_command: Option<String>,
    #[serde(flatten)]
    pub conf_override: MailUIConf,
    #[serde(flatten)]
    #[serde(
        deserialize_with = "extra_settings",
        skip_serializing_if = "IndexMap::is_empty"
    )]
    /// Use custom deserializer to convert any given value (eg `bool`, number,
    /// etc) to `String`.
    pub extra: IndexMap<String, String>,
}

impl FileAccount {
    pub fn mailboxes(&self) -> &IndexMap<String, FileMailboxConf> {
        &self.mailboxes
    }

    pub fn search_backend(&self) -> &data_types::SearchBackend {
        &self.search_backend
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FileSettings {
    pub accounts: IndexMap<String, FileAccount>,
    #[serde(default)]
    pub pager: pager::PagerSettings,
    #[serde(default)]
    pub listing: listing::ListingSettings,
    #[serde(default)]
    pub notifications: notifications::NotificationsSettings,
    #[serde(default)]
    pub shortcuts: shortcuts::Shortcuts,
    #[serde(default)]
    pub composing: composing::ComposingSettings,
    #[serde(default)]
    pub tags: tags::TagsSettings,
    #[serde(default)]
    pub pgp: pgp::PGPSettings,
    #[serde(default)]
    pub terminal: terminal::TerminalSettings,
    #[serde(default)]
    pub log: LogSettings,
    /// Non-fatal configuration problems found while loading; the invalid value
    /// has been replaced with a safe default. Surfaced to the user as
    /// notifications once the UI starts.
    #[serde(skip)]
    pub config_warnings: Vec<String>,
}

impl FileSettings {
    /// Replace configuration values that would otherwise panic when used with
    /// a safe default, recording a user-facing warning for each.
    ///
    /// This runs at load time so that the problem is reported once, with the
    /// field name, instead of aborting the process later.
    fn fixup_non_fatal(&mut self) {
        let empty_custom_frames = matches!(
            self.terminal.progress_spinner_sequence.as_ref(),
            Some(terminal::ProgressSpinnerSequence::Custom { frames, .. }) if frames.is_empty()
        );
        if empty_custom_frames {
            let msg =
                "terminal.progress_spinner_sequence: a custom sequence must contain at least \
                       one frame; using the default spinner instead.";
            melib::log::error!("{msg}");
            self.config_warnings.push(msg.to_string());
            self.terminal.progress_spinner_sequence = None;
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct AccountConf {
    pub account: melib::AccountSettings,
    /// How to send e-mail for this account.
    /// Required
    pub send_mail: SendMail,
    pub default_mailbox: Option<MailboxHash>,
    pub sent_mailbox: Option<MailboxHash>,
    pub conf: FileAccount,
    pub conf_override: MailUIConf,
    pub mailbox_confs: IndexMap<String, FileMailboxConf>,
}

impl AccountConf {
    pub fn account(&self) -> &melib::AccountSettings {
        &self.account
    }
    pub fn conf(&self) -> &FileAccount {
        &self.conf
    }
}

impl From<melib::AccountSettings> for AccountConf {
    fn from(account: melib::AccountSettings) -> Self {
        Self {
            account,
            ..Self::default()
        }
    }
}
impl From<FileAccount> for AccountConf {
    fn from(x: FileAccount) -> Self {
        let format = x.format.to_lowercase();
        let root_mailbox = x.root_mailbox.clone();
        let identity = x.identity.clone();
        let display_name = x.display_name.clone();
        let mailboxes = x
            .mailboxes
            .iter()
            .map(|(k, v)| (k.clone(), v.mailbox_conf.clone()))
            .collect();

        let account = melib::AccountSettings {
            name: String::new(),
            root_mailbox,
            format,
            identity,
            extra_identities: x.extra_identities.clone(),
            read_only: x.read_only,
            display_name,
            subscribed_mailboxes: x.subscribed_mailboxes.clone(),
            mailboxes,
            manual_refresh: x.manual_refresh,
            extra: x.extra.clone().into_iter().collect(),
        };

        let mailbox_confs = x.mailboxes.clone();
        Self {
            send_mail: x.send_mail.clone(),
            default_mailbox: None,
            sent_mailbox: None,
            conf_override: x.conf_override.clone(),
            conf: x,
            mailbox_confs,
            ..Self::from(account)
        }
    }
}

pub fn get_config_file() -> Result<PathBuf> {
    if let Ok(path) = env::var("MELI_CONFIG") {
        return Ok(PathBuf::from(path).expand());
    }
    let xdg_dirs = xdg::BaseDirectories::with_prefix("meli")?;
    xdg_dirs
        .place_config_file("config.toml")
        .chain_err_summary(|| {
            format!(
                "Cannot create configuration directory in {}",
                xdg_dirs.get_config_home().display()
            )
        })
        .chain_err_kind(ErrorKind::Platform)
}

/// Read-only view of the user's theme directory
/// (`$XDG_CONFIG_HOME/meli/themes/`).
///
/// Consumed by the `toggle_theme` picker: every `*.toml` file
/// contributes the themes declared inside it
/// (`[terminal.themes.<name>]` tables). Files that fail to parse are
/// reported through the notification and skipped, so one bad file
/// cannot take the picker down. Returns `(themes, errors)` where
/// `themes` maps a theme name to the file that declares it (validated
/// lazily when the theme is actually applied).
pub fn get_user_themes() -> (IndexMap<String, PathBuf>, Vec<String>) {
    let mut themes = IndexMap::new();
    let mut errors = Vec::new();
    let Some(dir) = (|| {
        let xdg_dirs = xdg::BaseDirectories::with_prefix("meli").ok()?;
        let dir = xdg_dirs.get_config_home().join("themes");
        dir.is_dir().then_some(dir)
    })() else {
        return (themes, errors);
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return (themes, errors);
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"))
        })
        .collect();
    paths.sort();
    for path in paths {
        match std::fs::read_to_string(&path)
            .map_err(|err| err.to_string())
            .and_then(|text| toml::from_str::<toml::Value>(&text).map_err(|err| err.to_string()))
        {
            Ok(value) => {
                let Some(themes_table) = value
                    .get("terminal")
                    .and_then(|t| t.get("themes"))
                    .and_then(|t| t.as_table())
                else {
                    errors.push(format!("{}: no [terminal.themes] table", path.display()));
                    continue;
                };
                for (name, _) in themes_table.iter() {
                    if name == self::LIGHT || name == self::DARK {
                        errors.push(format!(
                            "{}: theme name `{name}` is reserved",
                            path.display()
                        ));
                        continue;
                    }
                    themes.insert(name.clone(), path.clone());
                }
            }
            Err(err) => errors.push(format!("{}: {err}", path.display())),
        }
    }
    (themes, errors)
}

/// Load the theme `name` from a theme directory file.
///
/// The file is parsed through the same path `:toggle theme` uses at
/// runtime: pick its `[terminal.themes.<name>]` table, deserialize it
/// into [`ThemeOptions`] and merge it over `base` with
/// [`construct_theme`]. A theme directory file shadows every same-name
/// definition, compiled-in or from the configuration (theme directory >
/// configuration > built-in), so callers insert the result over any
/// existing entry.
pub fn theme_from_file(name: &str, path: &Path, base: &themes::Theme) -> Result<themes::Theme> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| Error::new(format!("could not read {}: {err}", path.display())))?;
    let value: toml::Value = text.parse().map_err(|err: toml::de::Error| {
        Error::new(format!("could not parse {}: {err}", path.display()))
    })?;
    let table = value
        .get("terminal")
        .and_then(|t| t.get("themes"))
        .and_then(|t| t.get(name))
        .cloned()
        .ok_or_else(|| {
            Error::new(format!(
                "{} has no [terminal.themes.{name}] table",
                path.display()
            ))
        })?;
    let options: ThemeOptions = table
        .try_into()
        .map_err(|err| Error::new(format!("{}: invalid theme `{name}`: {err}", path.display())))?;
    let mut theme = base.clone();
    construct_theme(name, &mut theme, options)?;
    Ok(theme)
}

/// Configuration keys that used to be accepted in `[listing]` and
/// `[accounts.<name>.listing]` but were later removed from
/// [`ListingSettings`](crate::conf::listing::ListingSettings) (the listing
/// layout is now a fixed 30/70 split and cannot be customised).
///
/// Both `ListingSettings` and `ListingSettingsOverride` carry
/// `#[serde(deny_unknown_fields)]`, so leftover keys in old configuration
/// files would abort startup; they must be stripped before parsing (see
/// [`strip_legacy_listing_keys`]).
const LEGACY_LISTING_KEYS: [&str; 3] = ["thread_layout", "sidebar_ratio", "mail_view_divider"];

/// Remove the keys in [`LEGACY_LISTING_KEYS`] from `table` (a `listing`
/// table), logging a warning per removal; returns `true` if any was removed.
fn remove_legacy_listing_keys(table: &mut toml::value::Table, location: &str) -> bool {
    let mut removed = false;
    for key in LEGACY_LISTING_KEYS {
        if table.remove(key).is_some() {
            removed = true;
            melib::log::warn!(
                "`{key}` in `{location}` is no longer a valid setting and was ignored."
            );
        }
    }
    removed
}

/// Strip removed listing keys from the pp-expanded configuration text `s`:
/// the top-level `listing` table, every `accounts.<name>.listing` table and
/// every `accounts.<name>.mailboxes.<mailbox>.listing` table
/// ([`FileMailboxConf`](crate::conf::FileMailboxConf) flattens
/// [`MailUIConf`](crate::conf::MailUIConf), so the same
/// `deny_unknown_fields` override applies there).
///
/// If `s` is not valid TOML, return it untouched so the existing error path
/// reports the real parse error; likewise when no legacy key matched, to
/// avoid a needless re-serialization.
fn strip_legacy_listing_keys(s: String) -> String {
    let Ok(mut value) = toml::from_str::<toml::Value>(&s) else {
        return s;
    };
    let mut removed = false;
    if let Some(listing) = value.get_mut("listing").and_then(toml::Value::as_table_mut) {
        removed |= remove_legacy_listing_keys(listing, "listing");
    }
    if let Some(accounts) = value
        .get_mut("accounts")
        .and_then(toml::Value::as_table_mut)
    {
        for (name, account) in accounts.iter_mut() {
            let Some(account) = account.as_table_mut() else {
                continue;
            };
            if let Some(listing) = account
                .get_mut("listing")
                .and_then(toml::Value::as_table_mut)
            {
                removed |=
                    remove_legacy_listing_keys(listing, &format!("accounts.{name}.listing"));
            }
            let Some(mailboxes) = account
                .get_mut("mailboxes")
                .and_then(toml::Value::as_table_mut)
            else {
                continue;
            };
            for (mailbox, mailbox_conf) in mailboxes.iter_mut() {
                let Some(mailbox_conf) = mailbox_conf.as_table_mut() else {
                    continue;
                };
                let Some(listing) = mailbox_conf
                    .get_mut("listing")
                    .and_then(toml::Value::as_table_mut)
                else {
                    continue;
                };
                removed |= remove_legacy_listing_keys(
                    listing,
                    &format!("accounts.{name}.mailboxes.{mailbox}.listing"),
                );
            }
        }
    }
    if removed {
        // Fall back to the original text if serialization fails; that is no
        // worse than not stripping at all.
        toml::to_string(&value).unwrap_or(s)
    } else {
        s
    }
}

impl FileSettings {
    pub const EXAMPLE_CONFIG: &'static str = include_str!("../docs/samples/sample-config.toml");

    pub fn new() -> Result<Self> {
        Self::load(get_config_file()?)
    }

    /// Load and validate settings from an explicit configuration file
    /// `path`, without consulting `MELI_CONFIG` or XDG environment
    /// lookups.
    #[cfg(test)]
    pub fn from_path(path: PathBuf) -> Result<Self> {
        Self::load(path)
    }

    fn load(config_path: PathBuf) -> Result<Self> {
        if !config_path.exists() {
            let path_string = config_path.display().to_string();
            if path_string.is_empty() {
                return Err(Error::new("Given configuration path is empty.")
                    .set_kind(ErrorKind::Configuration));
            }
            #[cfg(not(test))]
            let ask = crate::terminal::Ask::new(format!(
                "No configuration found. Would you like to generate one in {path_string}?"
            ));
            #[cfg(not(test))]
            let mut stdout = std::io::stdout();
            #[cfg(not(test))]
            let stdin = std::io::stdin();
            #[cfg(not(test))]
            if ask.run(&mut stdout, &mut stdin.lock()) {
                create_config_file(&config_path)?;
                return Err(
                    Error::new("Edit the sample configuration and relaunch meli.")
                        .set_kind(ErrorKind::Configuration),
                );
            }
            #[cfg(test)]
            return Ok(Self::default());
            #[cfg(not(test))]
            return Err(
                Error::new("No configuration file found.").set_kind(ErrorKind::Configuration)
            );
        }

        let mut stdout = std::io::stdout();
        let stdin = std::io::stdin();
        if !cfg!(test) {
            crate::version_migrations::version_setup(&config_path, &mut stdout, &mut stdin.lock())?;
        }
        Self::validate(config_path, false)
    }

    /// Validate configuration from `input` string.
    pub fn validate_string(s: String, clear_extras: bool) -> Result<Self> {
        let s = strip_legacy_listing_keys(s);
        let _: toml::value::Table = melib::serde_path_to_error::deserialize(
            toml::Deserializer::new(&s),
        )
        .map_err(|err| {
            Error::new("Config file is invalid TOML")
                .set_source(Some(Arc::new(err)))
                .set_kind(ErrorKind::ValueError)
        })?;

        let mut s: Self = melib::serde_path_to_error::deserialize(toml::Deserializer::new(&s))
            .map_err(|err| {
                Error::new("Input contains errors")
                    .set_source(Some(Arc::new(err)))
                    .set_kind(ErrorKind::Configuration)
            })?;
        s.fixup_non_fatal();
        let backends = melib::backends::Backends::new();
        let Themes {
            light: default_light,
            dark: default_dark,
            ..
        } = Themes::default();
        for (k, v) in default_light.keys.into_iter() {
            if !s.terminal.themes.light.contains_key(&k) {
                s.terminal.themes.light.insert(k, v);
            }
        }
        for theme in s.terminal.themes.other_themes.values_mut() {
            for (k, v) in default_dark.keys.clone().into_iter() {
                if !theme.contains_key(&k) {
                    theme.insert(k, v);
                }
            }
        }
        for (k, v) in default_dark.keys.into_iter() {
            if !s.terminal.themes.dark.contains_key(&k) {
                s.terminal.themes.dark.insert(k, v);
            }
        }
        match s.terminal.theme.as_str() {
            themes::DARK | themes::LIGHT => {}
            t if s.terminal.themes.other_themes.contains_key(t) => {}
            t => {
                return Err(Error::new(format!("Theme `{t}` was not found."))
                    .set_kind(ErrorKind::Configuration));
            }
        }

        s.terminal.themes.validate()?;
        for (name, acc) in s.accounts.iter_mut() {
            let FileAccount {
                root_mailbox,
                format,
                send_mail: _,
                identity,
                extra_identities,
                read_only,
                display_name,
                subscribed_mailboxes,
                mailboxes,
                extra,
                manual_refresh,
                default_mailbox: _,
                refresh_command: _,
                search_backend: _,
                conf_override: _,
            } = acc.clone();

            let lowercase_format = format.to_lowercase();
            let mut s = melib::AccountSettings {
                name: name.to_string(),
                root_mailbox,
                format: format.clone(),
                identity,
                extra_identities,
                read_only,
                display_name,
                subscribed_mailboxes,
                manual_refresh,
                mailboxes: mailboxes
                    .into_iter()
                    .map(|(k, v)| (k, v.mailbox_conf))
                    .collect(),
                extra: extra.into_iter().collect(),
            };
            s.validate_config()?;
            backends.validate_config(&lowercase_format, &mut s)?;
            if !s.extra.is_empty() {
                return Err(Error::new(format!(
                    "Unrecognised configuration values: {:?}",
                    s.extra
                ))
                .set_kind(ErrorKind::Configuration));
            }
            if clear_extras {
                acc.extra.clear();
            }
        }

        Ok(s)
    }

    /// Validate `path` and print errors.
    pub fn validate(path: PathBuf, clear_extras: bool) -> Result<Self> {
        let s = pp::pp(&path)?;
        let s = strip_legacy_listing_keys(s);
        let _: toml::value::Table = toml::from_str(&s).map_err(|err| {
            Error::new(format!(
                "{}: Config file is invalid TOML; {}",
                path.display(),
                err
            ))
        })?;

        let mut s: Self = toml::from_str(&s).map_err(|err| {
            Error::new(format!("{}: Config file contains errors", path.display()))
                .set_source(Some(Arc::new(err)))
                .set_kind(ErrorKind::Configuration)
        })?;
        s.fixup_non_fatal();
        let backends = melib::backends::Backends::new();
        let Themes {
            light: default_light,
            dark: default_dark,
            ..
        } = Themes::default();
        for (k, v) in default_light.keys.into_iter() {
            if !s.terminal.themes.light.contains_key(&k) {
                s.terminal.themes.light.insert(k, v);
            }
        }
        for theme in s.terminal.themes.other_themes.values_mut() {
            for (k, v) in default_dark.keys.clone().into_iter() {
                if !theme.contains_key(&k) {
                    theme.insert(k, v);
                }
            }
        }
        for (k, v) in default_dark.keys.into_iter() {
            if !s.terminal.themes.dark.contains_key(&k) {
                s.terminal.themes.dark.insert(k, v);
            }
        }
        match s.terminal.theme.as_str() {
            themes::DARK | themes::LIGHT => {}
            t if s.terminal.themes.other_themes.contains_key(t) => {}
            t => {
                return Err(Error::new(format!("Theme `{t}` was not found."))
                    .set_kind(ErrorKind::Configuration));
            }
        }

        s.terminal.themes.validate()?;
        for (name, acc) in s.accounts.iter_mut() {
            let FileAccount {
                root_mailbox,
                format,
                send_mail: _,
                identity,
                extra_identities,
                read_only,
                display_name,
                subscribed_mailboxes,
                mailboxes,
                extra,
                manual_refresh,
                default_mailbox: _,
                refresh_command: _,
                search_backend: _,
                conf_override: _,
            } = acc.clone();

            let lowercase_format = format.to_lowercase();
            let mut s = melib::AccountSettings {
                name: name.to_string(),
                root_mailbox,
                format: format.clone(),
                identity,
                extra_identities,
                read_only,
                display_name,
                subscribed_mailboxes,
                manual_refresh,
                mailboxes: mailboxes
                    .into_iter()
                    .map(|(k, v)| (k, v.mailbox_conf))
                    .collect(),
                extra: extra.into_iter().collect(),
            };
            s.validate_config()?;
            backends.validate_config(&lowercase_format, &mut s)?;
            if !s.extra.is_empty() {
                return Err(Error::new(format!(
                    "Unrecognised configuration values: {:?}",
                    s.extra
                ))
                .set_kind(ErrorKind::Configuration));
            }
            if clear_extras {
                acc.extra.clear();
            }
        }

        Ok(s)
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Settings {
    pub accounts: IndexMap<String, AccountConf>,
    pub pager: pager::PagerSettings,
    pub listing: listing::ListingSettings,
    pub notifications: notifications::NotificationsSettings,
    pub shortcuts: shortcuts::Shortcuts,
    pub tags: tags::TagsSettings,
    pub composing: composing::ComposingSettings,
    pub pgp: pgp::PGPSettings,
    pub terminal: terminal::TerminalSettings,
    pub log: LogSettings,
    /// Non-fatal configuration problems found at load time; the UI surfaces
    /// these as notifications (see `State::new`).
    #[serde(skip)]
    pub config_warnings: Vec<String>,
    #[serde(skip)]
    pub _logger: Logger,
}

impl Settings {
    pub fn new() -> Result<Self> {
        Self::from_file_settings(FileSettings::new()?)
    }

    /// Create a new `Settings` value from an explicit configuration file
    /// `path`, without consulting `MELI_CONFIG` or XDG environment
    /// lookups.
    #[cfg(test)]
    pub fn from_path(path: PathBuf) -> Result<Self> {
        Self::from_file_settings(FileSettings::from_path(path)?)
    }

    fn from_file_settings(fs: FileSettings) -> Result<Self> {
        let mut _logger = Logger::new(melib::LogLevel::default());

        if _logger.log_level() != fs.log.maximum_level {
            _logger.change_log_level(fs.log.maximum_level)
        }

        #[cfg(debug_assertions)]
        apply_debug_default_logging(&_logger, &fs.log);

        if let Some(ref log_path) = fs.log.log_file {
            _logger.change_log_dest(log_path.into());
        }

        let mut s: IndexMap<String, AccountConf> = IndexMap::new();

        for (id, x) in fs.accounts {
            let mut ac = AccountConf::from(x);
            ac.account.name.clone_from(&id);

            s.insert(id, ac);
        }

        Ok(Self {
            accounts: s,
            pager: fs.pager,
            listing: fs.listing,
            notifications: fs.notifications,
            shortcuts: fs.shortcuts,
            tags: fs.tags,
            composing: fs.composing,
            pgp: fs.pgp,
            terminal: fs.terminal,
            log: fs.log,
            config_warnings: fs.config_warnings,
            _logger,
        })
    }

    pub fn without_accounts() -> Result<Self> {
        let mut _logger = Logger::new(melib::LogLevel::default());

        let fs = FileSettings::new()?;

        if _logger.log_level() != fs.log.maximum_level {
            _logger.change_log_level(fs.log.maximum_level)
        }

        #[cfg(debug_assertions)]
        apply_debug_default_logging(&_logger, &fs.log);

        if let Some(ref log_path) = fs.log.log_file {
            _logger.change_log_dest(log_path.into());
        }

        Ok(Self {
            accounts: IndexMap::new(),
            pager: fs.pager,
            listing: fs.listing,
            notifications: fs.notifications,
            shortcuts: fs.shortcuts,
            tags: fs.tags,
            composing: fs.composing,
            pgp: fs.pgp,
            terminal: fs.terminal,
            log: fs.log,
            config_warnings: fs.config_warnings,
            _logger,
        })
    }
}

mod deserializers {
    use serde::{de, Deserialize, Deserializer};

    pub(in crate::conf) fn non_empty_opt_string<'de, D, T: std::convert::From<Option<String>>>(
        deserializer: D,
    ) -> std::result::Result<T, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = <String>::deserialize(deserializer)?;
        if s.is_empty() {
            Ok(None.into())
        } else {
            Ok(Some(s).into())
        }
    }

    pub(in crate::conf) fn non_empty_string<'de, D, T: std::convert::From<String>>(
        deserializer: D,
    ) -> std::result::Result<T, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = <String>::deserialize(deserializer)?;
        if s.is_empty() {
            Err(de::Error::custom("This field value cannot be empty."))
        } else {
            Ok(s.into())
        }
    }

    use toml::Value;
    fn any_of<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v: Value = Deserialize::deserialize(deserializer)?;
        if let Some(s) = v.as_str() {
            return Ok(s.to_string());
        }
        let mut ret = v.to_string();
        if (ret.starts_with('"') && ret.ends_with('"'))
            || (ret.starts_with('\"') && ret.ends_with('\''))
        {
            ret.drain(0..1).count();
            ret.drain(ret.len() - 1..).count();
        }
        Ok(ret)
    }

    use indexmap::IndexMap;
    pub(in crate::conf) fn extra_settings<'de, D>(
        deserializer: D,
    ) -> std::result::Result<IndexMap<String, String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        /* Why is this needed? If the user gives a configuration value such as key =
         * true, the parsing will fail since it expects string values. We
         * want to accept key = true as well as key = "true". */
        #[derive(Deserialize)]
        struct Wrapper(#[serde(deserialize_with = "any_of")] String);

        let v = <IndexMap<String, Wrapper>>::deserialize(deserializer)?;
        Ok(v.into_iter().map(|(k, Wrapper(v))| (k, v)).collect())
    }
}

pub fn create_config_file(p: &Path) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(p)
        .chain_err_summary(|| format!("Cannot create configuration file in {}", p.display()))?;
    file.write_all(FileSettings::EXAMPLE_CONFIG.as_bytes())
        .and_then(|()| file.flush())
        .chain_err_summary(|| format!("Could not write to configuration file  {}", p.display()))?;
    println!("Written example configuration to {}", p.display());
    let set_permissions = |file: std::fs::File| -> Result<()> {
        let metadata = file.metadata()?;
        let mut permissions = metadata.permissions();

        permissions.set_mode(0o600); // Read/write for owner only.
        file.set_permissions(permissions)?;
        Ok(())
    };
    if let Err(err) = set_permissions(file) {
        println!(
            "Warning: Could not set permissions of {} to 0o600: {}",
            p.display(),
            err
        );
    }
    Ok(())
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LogSettings {
    #[serde(default)]
    pub log_file: Option<PathBuf>,
    #[serde(default)]
    pub maximum_level: melib::LogLevel,
}

pub use data_types::dotaddressable::*;

/// Debug builds default the logger to one file per run under `./log/` in
/// the current directory at `DEBUG` level, so a field bug report comes with
/// a trace of what the app was doing (the release build keeps the XDG
/// data-dir default).
///
/// An explicit `[logging] log_file` in the configuration wins, and setting
/// `MELI_DEBUG_LOG=0` opts out. Failures never abort startup: if `./log/`
/// cannot be created, the previous destination is kept.
#[cfg(debug_assertions)]
fn apply_debug_default_logging(logger: &Logger, log: &LogSettings) {
    // `cfg!(test)` (not `#[cfg(test)]`): unit tests construct `Settings`
    // too, and must not litter the crate's working directory with log
    // files.
    if cfg!(test) {
        return;
    }
    if std::env::var_os("MELI_DEBUG_LOG").is_some_and(|v| v == "0") {
        return;
    }
    // The caller applies an explicit `log_file` right after; do not fight it.
    if log.log_file.is_some() {
        return;
    }
    if logger.log_level() < melib::LogLevel::TRACE {
        logger.change_log_level(melib::LogLevel::TRACE);
    }
    let dir = Path::new("log");
    if let Err(err) = std::fs::create_dir_all(dir) {
        eprintln!(
            "debug logging disabled: could not create `{}`: {err}",
            dir.display()
        );
        return;
    }
    let ts = melib::utils::datetime::timestamp_to_string(
        melib::utils::datetime::now(),
        Some("%Y%m%d-%H%M%S"),
        false,
    );
    let path = dir.join(format!("meli-debug-{ts}.log"));
    logger.change_log_dest(path.clone());
    melib::log::info!("debug build: logging to {}", path.display());
}

/// Rewrite `theme = "<name>"` inside the `[terminal]` table of a
/// configuration file's text, preserving every other line verbatim.
///
/// - If `[terminal]` already contains a `theme = ...` entry (bare or
///   quoted, with optional trailing comment), it is replaced in place.
/// - If `[terminal]` exists but has no `theme`, the assignment is
///   inserted right after the table header.
/// - If no `[terminal]` table exists, one is appended with the
///   assignment.
///
/// The input is treated as text, not parsed as TOML and re-serialized:
/// user comments, key ordering and formatting of every other setting
/// survive untouched.
pub fn rewrite_terminal_theme(text: &str, name: &str) -> Result<String> {
    let name_escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
    let new_line = format!("theme = \"{name_escaped}\"");

    // Strategy: find the `[terminal]` table and whether it already has a
    // `theme = ...` key. If yes, replace that key's line in place; if the
    // table exists without one, insert right after the table header; if
    // there is no table at all, append one. Everything else is copied
    // verbatim.
    let lines: Vec<&str> = text.lines().collect();

    // Locate the `[terminal]` table boundaries (exclusive end = next
    // top-level table header or EOF).
    let mut terminal_start: Option<usize> = None;
    let mut terminal_end = lines.len();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if let Some(header) = trimmed.strip_prefix('[') {
            let is_terminal = header
                .split(']')
                .next()
                .is_some_and(|rest| rest.trim().eq_ignore_ascii_case("terminal"));
            if is_terminal && terminal_start.is_none() {
                terminal_start = Some(i);
            } else if terminal_start.is_some() {
                terminal_end = i;
                break;
            }
        }
    }

    let mut out = String::with_capacity(text.len() + new_line.len() + 16);
    match terminal_start {
        Some(ts) => {
            let mut replaced = false;
            for (i, line) in lines.iter().enumerate() {
                if i > ts && i < terminal_end && !replaced {
                    let trimmed = line.trim_start();
                    let is_theme_key = trimmed
                        .split_once('=')
                        .is_some_and(|(key, _)| key.trim().eq_ignore_ascii_case("theme"));
                    if is_theme_key {
                        out.push_str(&new_line);
                        out.push('\n');
                        replaced = true;
                        continue;
                    }
                }
                out.push_str(line);
                out.push('\n');
                // Right after the table header: if the table has no theme
                // key at all, insert it here.
                if i == ts
                    && lines[ts + 1..terminal_end].iter().all(|l| {
                        !l.trim_start()
                            .split_once('=')
                            .is_some_and(|(key, _)| key.trim().eq_ignore_ascii_case("theme"))
                    })
                {
                    out.push_str(&new_line);
                    out.push('\n');
                    replaced = true;
                }
            }
            if !replaced {
                // terminal table was the last table and empty; the header
                // path above already inserted.
            }
        }
        None => {
            out.push_str(text);
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("\n[terminal]\n");
            out.push_str(&new_line);
            out.push('\n');
        }
    }
    // Normalize trailing blank lines to exactly one newline.
    while out.ends_with("\n\n") {
        out.pop();
    }
    Ok(out)
}

#[cfg(test)]
mod rewrite_theme_tests {
    use super::rewrite_terminal_theme;

    #[test]
    fn insert_into_existing_terminal_table() {
        let input = "[terminal]\nfoo = 1\n";
        let out = rewrite_terminal_theme(input, "nord").unwrap();
        assert_eq!(out, "[terminal]\ntheme = \"nord\"\nfoo = 1\n");
    }

    #[test]
    fn replace_existing_theme_line() {
        let input = "[terminal]\ntheme = \"dark\"\nfoo = 1\n";
        let out = rewrite_terminal_theme(input, "nord").unwrap();
        assert_eq!(out, "[terminal]\ntheme = \"nord\"\nfoo = 1\n");
    }

    #[test]
    fn append_terminal_table_when_missing() {
        let input = "[accounts.md]\nroot_mailbox = \"x\"\n";
        let out = rewrite_terminal_theme(input, "nord").unwrap();
        assert!(out.contains("[accounts.md]"), "other tables untouched");
        assert!(
            out.trim_end().ends_with("[terminal]\ntheme = \"nord\"")
                || out.contains("[terminal]\ntheme = \"nord\"")
        );
    }

    #[test]
    fn rewrite_twice_keeps_single_theme_line() {
        let input = "[terminal]\nfoo = 1\n";
        let once = rewrite_terminal_theme(input, "nord").unwrap();
        let twice = rewrite_terminal_theme(&once, "dark").unwrap();
        assert_eq!(twice.matches("theme =").count(), 1);
        assert!(twice.contains("theme = \"dark\""));
    }

    #[test]
    fn quoted_and_bare_existing_values_are_replaced() {
        for existing in ["theme = \"dark\"", "theme = 'dark'", "theme = dark"] {
            let input = format!("[terminal]\n{existing}\n");
            let out = rewrite_terminal_theme(&input, "nord").unwrap();
            assert_eq!(out.matches("theme").count(), 1, "for {existing}");
            assert!(out.contains("theme = \"nord\""), "for {existing}");
        }
    }
}
