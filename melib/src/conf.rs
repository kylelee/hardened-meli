/*
 * meli - configuration module.
 *
 * Copyright 2017 Manos Pitsidianakis
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

//! Basic mail account configuration to use with
//! [`backends`](./backends/index.html)

use std::{borrow::Cow, path::Path};

use indexmap::IndexMap;

use crate::{
    backends::SpecialUsageMailbox,
    email::Address,
    error::{Error, ErrorKind, Result},
    ShellExpandTrait,
};

mod field_types;
#[cfg(test)]
mod tests;

pub use field_types::*;

/// Trait for types that can be deserialized from an `extra` configuration
/// value.
///
/// Each backend's `validate_config` method pulls values out of
/// [`AccountSettings::extra`] (a `serde_json::Value` map) and validates them
/// per-backend. The default implementation defers to `serde_json`'s
/// `Deserialize`, which is what enables the new Secret-typed configuration
/// fields (`server_password = "literal"` and `server_password = { command =
/// "..." }`).
pub trait ExtraSetting: serde::de::DeserializeOwned {
    fn deserialize_extra(value: &serde_json::Value) -> Result<Self> {
        serde::de::Deserialize::deserialize(value.clone()).map_err(|err| {
            Error::new(format!(
                "could not deserialize value as {}",
                std::any::type_name::<Self>()
            ))
            .set_source(Some(crate::src_err_arc_wrap! { err }))
            .set_kind(ErrorKind::Configuration)
        })
    }
}

impl<'a> ExtraSetting for Cow<'a, str> {}
impl ExtraSetting for String {}
impl ExtraSetting for field_types::Secret {}
impl ExtraSetting for bool {}

macro_rules! impl_extra_setting_from_str {
    ($($t:ty),*$(,)?) => {
        $(impl ExtraSetting for $t {
            fn deserialize_extra(v: &serde_json::Value) -> Result<Self> {
                serde::de::Deserialize::deserialize(v.clone())
                    .or_else(|err| {
                        if let Ok(s) = serde::de::Deserialize::deserialize(v.clone()) {
                            let s: Cow<'_, str> = s;
                            if let Ok(v) = <$t as std::str::FromStr>::from_str(s.as_ref()) {
                                return Ok(v);
                            }
                        }
                        Err(err)
                    })
                    .map_err(|err| {
                        Error::new(format!(
                            "could not deserialize value as {}",
                            std::any::type_name::<Self>()
                        ))
                        .set_source(Some(crate::src_err_arc_wrap! { err }))
                        .set_kind(ErrorKind::Configuration)
                    })
            }
        })*
    };
}

impl_extra_setting_from_str! { u16, u64 }

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AccountSettings {
    pub name: String,
    /// Name of mailbox that is the root of the mailbox hierarchy.
    ///
    /// Note that this may have special or no meaning depending on the e-mail
    /// backend.
    pub root_mailbox: String,
    pub format: String,
    pub identity: String,
    #[serde(default)]
    pub extra_identities: Vec<String>,
    #[serde(default = "false_val")]
    pub read_only: bool,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub subscribed_mailboxes: Vec<String>,
    #[serde(default)]
    pub mailboxes: IndexMap<String, MailboxConf>,
    #[serde(default)]
    pub manual_refresh: bool,
    #[serde(flatten)]
    pub extra: IndexMap<String, serde_json::Value>,
}

impl AccountSettings {
    /// Create the account's display name from fields
    /// [`AccountSettings::identity`] and [`AccountSettings::display_name`].
    #[deprecated(
        since = "0.8.5",
        note = "Use AccountSettings::main_identity_address instead."
    )]
    pub fn make_display_name(&self) -> Address {
        Address::new(self.display_name.clone(), self.identity.clone())
    }

    /// Return address associated with this account.
    /// It combines the values from [`AccountSettings::identity`] and
    /// [`AccountSettings::display_name`].
    pub fn main_identity_address(&self) -> Address {
        Address::new(self.display_name.clone(), self.identity.clone())
    }

    /// Return addresses of extra identities associated with this account,
    /// if any.
    pub fn extra_identity_addresses(&self) -> Vec<Address> {
        self.extra_identities
            .iter()
            .map(|i| Address::new(None::<&str>, i.clone()))
            .collect()
    }

    pub fn vcard_folder(&self) -> Option<&str> {
        self.extra.get("vcard_folder").and_then(|v| v.as_str())
    }

    pub fn notmuch_address_book_query(&self) -> Option<&str> {
        self.extra
            .get("notmuch_address_book_query")
            .and_then(|v| v.as_str())
    }

    /// Look up an `extra` value as a string slice.
    ///
    /// Returns `None` if the key is absent or the value is not a string.
    /// The upstream port stores non-string values (e.g. Secret inline
    /// tables) under `extra`, and callers that want a string must opt in
    /// via this helper.
    pub fn extra_str(&self, key: &str) -> Option<&str> {
        self.extra.get(key).and_then(|v| v.as_str())
    }

    /// Look up an `extra` value as a raw [`serde_json::Value`].
    pub fn extra_value(&self, key: &str) -> Option<&serde_json::Value> {
        self.extra.get(key)
    }

    /// Look up an `extra` value as an owned string, coercing non-string
    /// scalars (numbers, booleans) to their string form — mirroring the
    /// legacy `IndexMap<String, String>` behaviour where every TOML value
    /// was stringified before the `serde_json::Value` migration.
    pub fn extra_conf_string(&self, key: &str) -> Option<String> {
        match self.extra.get(key)? {
            serde_json::Value::String(s) => Some(s.clone()),
            v @ (serde_json::Value::Number(_) | serde_json::Value::Bool(_)) => Some(v.to_string()),
            _ => None,
        }
    }

    /// Resolve the server password to a [`Secret`] suitable for the
    /// connection layer.
    ///
    /// The legacy `server_password_command = "..."` syntax is detected and
    /// rejected with a migration hint pointing at
    /// `server_password = { command = "..." }`.
    pub fn server_password_field(&self) -> Result<field_types::Secret> {
        if self.extra.contains_key("server_password_command") {
            return Err(Error::new(format!(
                "({}) `server_password_command` is no longer supported; use `server_password \
                 = {{ command = \"...\" }}` instead. Run the version migration offered on \
                 startup to update existing configurations.",
                self.name,
            ))
            .set_kind(ErrorKind::Configuration));
        }
        match self.extra.get("server_password") {
            Some(serde_json::Value::String(s)) => Ok(field_types::Secret::Value(s.clone())),
            Some(v @ serde_json::Value::Object(_)) => {
                serde_json::from_value(v.clone()).map_err(|err| {
                    Error::new(format!(
                        "({}) could not parse `server_password` object as a Secret",
                        self.name,
                    ))
                    .set_source(Some(crate::src_err_arc_wrap! { err }))
                    .set_kind(ErrorKind::Configuration)
                })
            }
            Some(_) => Err(Error::new(format!(
                "({}) `server_password` must be a string or `{{ command = \"...\" }}` table",
                self.name,
            ))
            .set_kind(ErrorKind::Configuration)),
            None => Err(Error::new(
                "Configuration error: connection requires `server_password` (string or \
                 `{ command = \"...\" }` table)",
            )),
        }
    }

    pub fn validate_config(&mut self) -> Result<()> {
        {
            if let Some(folder) = self
                .extra
                .swap_remove("vcard_folder")
                .and_then(|v| match v {
                    serde_json::Value::String(s) => Some(s),
                    serde_json::Value::Bool(b) => Some(b.to_string()),
                    serde_json::Value::Number(n) => Some(n.to_string()),
                    _ => None,
                })
            {
                let path = Path::new(&folder).expand();

                if !matches!(path.try_exists(), Ok(true)) {
                    return Err(Error::new(format!(
                        "`vcard_folder` path {} does not exist",
                        path.display()
                    ))
                    .set_details("`vcard_folder` must be a path of a folder containing .vcf files")
                    .set_kind(ErrorKind::Configuration));
                }
                if !path.is_dir() {
                    return Err(Error::new(format!(
                        "`vcard_folder` path {} is not a directory",
                        path.display()
                    ))
                    .set_details("`vcard_folder` must be a path of a folder containing .vcf files")
                    .set_kind(ErrorKind::Configuration));
                }
            }
            _ = self.extra.swap_remove("notmuch_address_book_query");
        }
        {
            if let Some(mutt_alias_file) =
                self.extra
                    .swap_remove("mutt_alias_file")
                    .and_then(|v| match v {
                        serde_json::Value::String(s) => Some(s),
                        serde_json::Value::Bool(b) => Some(b.to_string()),
                        serde_json::Value::Number(n) => Some(n.to_string()),
                        _ => None,
                    })
            {
                let path = Path::new(&mutt_alias_file).expand();

                if !matches!(path.try_exists(), Ok(true)) {
                    return Err(Error::new(format!(
                        "`mutt_alias_file` path {} does not exist",
                        path.display()
                    ))
                    .set_details("`mutt_alias_file` must be an existing path of a mutt alias file")
                    .set_kind(ErrorKind::Configuration));
                }
                if !path.is_file() {
                    return Err(Error::new(format!(
                        "`mutt_alias_file` path {} is not a file",
                        path.display()
                    ))
                    .set_details("`mutt_alias_file` must be a path of a mutt alias file")
                    .set_kind(ErrorKind::Configuration));
                }
            }
        }

        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct MailboxConf {
    #[serde(alias = "rename")]
    pub alias: Option<String>,
    #[serde(default = "false_val")]
    pub autoload: bool,
    #[serde(default)]
    pub subscribe: ToggleFlag,
    #[serde(default)]
    pub ignore: ToggleFlag,
    #[serde(default = "none")]
    pub usage: Option<SpecialUsageMailbox>,
    #[serde(default = "none")]
    pub sort_order: Option<usize>,
    #[serde(default = "none")]
    pub encoding: Option<String>,
    #[serde(flatten)]
    pub extra: IndexMap<String, String>,
}

impl Default for MailboxConf {
    fn default() -> Self {
        Self {
            alias: None,
            autoload: false,
            subscribe: ToggleFlag::Unset,
            ignore: ToggleFlag::Unset,
            usage: None,
            sort_order: None,
            encoding: None,
            extra: IndexMap::default(),
        }
    }
}

impl MailboxConf {
    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }
}

pub const fn true_val() -> bool {
    true
}

pub const fn false_val() -> bool {
    false
}

pub const fn none<T>() -> Option<T> {
    None
}
