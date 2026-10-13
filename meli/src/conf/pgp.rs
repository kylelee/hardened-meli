/*
 * meli - configuration module.
 *
 * Copyright 2019 Manos Pitsidianakis
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

use melib::{conf::ActionFlag, Error, Result};
use serde::{
    de::{self, Deserializer, MapAccess, Visitor},
    ser::SerializeStruct,
    Deserialize, Serialize, Serializer,
};

use crate::conf::{default_values::*, DotAddressable};

const GPGME_KEY: &str = "gpgme";

/// Settings for digital signing and encryption
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PGPSettings {
    /// auto verify signed e-mail according to RFC3156
    /// Default: true
    #[serde(default = "true_val", alias = "auto-verify-signatures")]
    pub auto_verify_signatures: ActionFlag,

    /// auto decrypt encrypted e-mail
    /// Default: true
    #[serde(default = "true_val", alias = "auto-decrypt")]
    pub auto_decrypt: ActionFlag,

    /// always sign sent e-mail
    /// Default: false
    #[serde(default = "false_val", alias = "auto-sign")]
    pub auto_sign: ActionFlag,

    /// Auto encrypt sent e-mail
    /// Default: false
    #[serde(default = "false_val", alias = "auto-encrypt")]
    pub auto_encrypt: ActionFlag,

    // https://tools.ietf.org/html/rfc4880#section-12.2
    /// Default: None
    #[serde(default = "none", alias = "sign-key")]
    pub sign_key: Option<String>,

    /// Default: None
    #[serde(default = "none", alias = "decrypt-key")]
    pub decrypt_key: Option<String>,

    /// Default: None
    #[serde(default = "none", alias = "encrypt-key")]
    pub encrypt_key: Option<String>,

    /// Default: true
    #[serde(default = "true_val", alias = "encrypt-for-self")]
    pub encrypt_for_self: bool,

    /// Allow remote lookups
    /// Default: False
    #[serde(
        default = "action_internal_value_false",
        alias = "allow-remote-lookups"
    )]
    pub allow_remote_lookup: ActionFlag,

    /// Remote lookup mechanisms.
    /// Default: "local,wkd"
    #[cfg_attr(
        feature = "gpgme",
        serde(
            default = "default_lookup_mechanism",
            alias = "remote-lookup-mechanisms"
        )
    )]
    #[cfg(feature = "gpgme")]
    pub remote_lookup_mechanisms: melib::email::pgp::LocateKey,
    #[cfg(not(feature = "gpgme"))]
    #[cfg_attr(
        not(feature = "gpgme"),
        serde(default, alias = "remote-lookup-mechanisms")
    )]
    pub remote_lookup_mechanisms: String,

    /// PGP backend to use for sign/verify/encrypt/decrypt.
    /// Default: `gpgme` if compiled with libgpgme, otherwise a CLI backend.
    #[serde(default)]
    pub backend: PGPBackendChoice,
}

#[cfg(feature = "gpgme")]
fn default_lookup_mechanism() -> melib::email::pgp::LocateKey {
    melib::email::pgp::LocateKey::LOCAL | melib::email::pgp::LocateKey::WKD
}

impl Default for PGPSettings {
    fn default() -> Self {
        Self {
            auto_verify_signatures: true.into(),
            auto_decrypt: true.into(),
            auto_sign: false.into(),
            auto_encrypt: false.into(),
            encrypt_for_self: true,
            sign_key: None,
            decrypt_key: None,
            encrypt_key: None,
            allow_remote_lookup: action_internal_value_false::<ActionFlag>(),
            #[cfg(feature = "gpgme")]
            remote_lookup_mechanisms: default_lookup_mechanism(),
            #[cfg(not(feature = "gpgme"))]
            remote_lookup_mechanisms: String::new(),
            backend: PGPBackendChoice::default(),
        }
    }
}

impl DotAddressable for melib::email::pgp::LocateKey {}

impl DotAddressable for PGPBackendChoice {
    fn lookup(&self, parent_field: &str, path: &[&str]) -> Result<String> {
        if path.is_empty() {
            return Ok(toml::Value::try_from(self)
                .map_err(|err| err.to_string())?
                .to_string());
        }
        let field = path[0];
        let tail = &path[1..];
        match (self, field) {
            #[cfg(feature = "gpgme")]
            (Self::GpgME, _) => Err(Error::new(format!(
                "{parent_field}.{field}: gpgme backend has no sub-fields"
            ))),
            #[cfg(not(feature = "gpgme"))]
            (Self::GpgME, _) => Err(Error::new(format!(
                "{parent_field}.{field}: gpgme backend has no sub-fields"
            ))),
            (Self::CLI(cli), "display_name") => cli.display_name.lookup(field, tail),
            (Self::CLI(cli), "scan_command") => cli.scan_command.lookup(field, tail),
            (Self::CLI(cli), "auto_key_locate") => cli.auto_key_locate.lookup(field, tail),
            _ => Err(Error::new(format!(
                "{parent_field} has no field named {field}"
            ))),
        }
    }
}

impl DotAddressable for PGPSettings {
    fn lookup(&self, parent_field: &str, path: &[&str]) -> Result<String> {
        match path.first() {
            Some(field) => {
                let tail = &path[1..];
                match *field {
                    "auto_verify_signatures" => self.auto_verify_signatures.lookup(field, tail),
                    "auto_decrypt" => self.auto_decrypt.lookup(field, tail),
                    "auto_sign" => self.auto_sign.lookup(field, tail),
                    "auto_encrypt" => self.auto_encrypt.lookup(field, tail),
                    "encrypt_for_self" => self.encrypt_for_self.lookup(field, tail),
                    "sign_key" => self.sign_key.lookup(field, tail),
                    "decrypt_key" => self.decrypt_key.lookup(field, tail),
                    "encrypt_key" => self.encrypt_key.lookup(field, tail),
                    "allow_remote_lookup" => self.allow_remote_lookup.lookup(field, tail),
                    #[cfg(feature = "gpgme")]
                    "remote_lookup_mechanisms" => self.remote_lookup_mechanisms.lookup(field, tail),
                    #[cfg(not(feature = "gpgme"))]
                    "remote_lookup_mechanisms" => self.remote_lookup_mechanisms.lookup(field, tail),
                    "backend" => self.backend.lookup(field, tail),
                    other => Err(Error::new(format!(
                        "{parent_field} has no field named {other}"
                    ))),
                }
            }
            None => Ok(toml::Value::try_from(self)
                .map_err(|err| err.to_string())?
                .to_string()),
        }
    }
}

/// Available PGP backends to use for signing, verifying, encrypting and
/// decrypting e-mail.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PGPBackendChoice {
    #[cfg_attr(not(feature = "gpgme"), allow(dead_code))]
    /// The default GPGME backend (libgpgme).
    GpgME,
    /// A user-defined command-line script backend.
    CLI(PGPBackendCLI),
}

// The default depends on the `gpgme` feature; cannot derive.
#[allow(clippy::derivable_impls)]
impl Default for PGPBackendChoice {
    fn default() -> Self {
        #[cfg(feature = "gpgme")]
        {
            Self::GpgME
        }
        #[cfg(not(feature = "gpgme"))]
        {
            Self::CLI(PGPBackendCLI::default())
        }
    }
}

#[cfg(feature = "gpgme")]
impl From<PGPBackendCLI> for PGPBackendChoice {
    fn from(cli: PGPBackendCLI) -> Self {
        Self::CLI(cli)
    }
}

/// A user-defined command-line script backend.
///
/// Each operation (key lookup, listing, signing, encrypting, etc.) is
/// delegated to a separate script that communicates with the rest of meli via
/// JSON in/out on its standard streams.
///
/// See the `contrib/pgp-cli-backends/gpg/*.py` scripts for a ready-to-use
/// `GnuPG` reference implementation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PGPBackendCLI {
    pub display_name: String,
    pub scan_command: String,
    #[serde(skip_serializing)]
    pub keylist_command: String,
    #[serde(skip_serializing)]
    pub get_key_command: String,
    #[serde(skip_serializing)]
    pub sign_command: String,
    #[serde(skip_serializing)]
    pub verify_command: String,
    #[serde(skip_serializing)]
    pub encrypt_command: String,
    #[serde(skip_serializing)]
    pub decrypt_command: String,
    pub auto_key_locate: melib::email::pgp::LocateKey,
}

impl Default for PGPBackendCLI {
    fn default() -> Self {
        Self {
            display_name: "gpg".into(),
            scan_command: "gpg --list-keys --json --no-default-keyring".into(),
            keylist_command: "gpg_keylist.py".into(),
            get_key_command: "gpg_get_key.py".into(),
            sign_command: "gpg_sign.py".into(),
            verify_command: "gpg_verify.py".into(),
            encrypt_command: "gpg_encrypt.py".into(),
            decrypt_command: "gpg_decrypt.py".into(),
            auto_key_locate: melib::email::pgp::LocateKey::LOCAL,
        }
    }
}

impl Serialize for PGPBackendChoice {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            #[cfg(feature = "gpgme")]
            Self::GpgME => {
                let mut s = serializer.serialize_struct("PGPBackendChoice", 1)?;
                s.serialize_field(GPGME_KEY, &true)?;
                s.end()
            }
            #[cfg(not(feature = "gpgme"))]
            Self::GpgME => {
                let mut s = serializer.serialize_struct("PGPBackendChoice", 1)?;
                s.serialize_field(GPGME_KEY, &true)?;
                s.end()
            }
            Self::CLI(cli) => cli.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for PGPBackendChoice {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PGPBackendChoiceVisitor;

        impl<'de> Visitor<'de> for PGPBackendChoiceVisitor {
            type Value = PGPBackendChoice;

            fn expecting(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                fmt.write_str(r#"either string "gpgme" or a CLI backend table"#)
            }

            fn visit_str<E: de::Error>(self, value: &str) -> std::result::Result<Self::Value, E> {
                self.visit_string(value.to_string())
            }

            fn visit_string<E: de::Error>(
                self,
                value: String,
            ) -> std::result::Result<Self::Value, E> {
                if value.eq_ignore_ascii_case(GPGME_KEY) {
                    #[cfg(feature = "gpgme")]
                    return Ok(PGPBackendChoice::GpgME);
                    #[cfg(not(feature = "gpgme"))]
                    return Err(de::Error::custom(
                        "value `gpgme` requires the `gpgme` feature to be enabled at compile \
                         time; configure a CLI backend instead",
                    ));
                }
                Err(de::Error::invalid_value(
                    de::Unexpected::Str(&value),
                    &"`gpgme`",
                ))
            }

            fn visit_borrowed_str<E: de::Error>(
                self,
                value: &'de str,
            ) -> std::result::Result<Self::Value, E> {
                self.visit_str(value)
            }

            fn visit_map<V>(self, map: V) -> std::result::Result<Self::Value, V::Error>
            where
                V: MapAccess<'de>,
            {
                let cli: PGPBackendCLI =
                    Deserialize::deserialize(de::value::MapAccessDeserializer::new(map))?;
                Ok(PGPBackendChoice::CLI(cli))
            }
        }

        deserializer.deserialize_any(PGPBackendChoiceVisitor)
    }
}

impl<'de> Deserialize<'de> for PGPBackendCLI {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "snake_case")]
        enum Field {
            DisplayName,
            ScanCommand,
            KeylistCommand,
            GetKeyCommand,
            SignCommand,
            VerifyCommand,
            EncryptCommand,
            DecryptCommand,
            AutoKeyLocate,
        }

        struct CliVisitor;
        impl<'de> Visitor<'de> for CliVisitor {
            type Value = PGPBackendCLI;

            fn expecting(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                fmt.write_str("PGP CLI backend table")
            }

            fn visit_map<V>(self, mut map: V) -> std::result::Result<Self::Value, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut result = PGPBackendCLI::default();
                while let Some(key) = map.next_key::<Field>()? {
                    match key {
                        Field::DisplayName => {
                            result.display_name = map.next_value()?;
                        }
                        Field::ScanCommand => {
                            result.scan_command = map.next_value()?;
                        }
                        Field::KeylistCommand => {
                            result.keylist_command = map.next_value()?;
                        }
                        Field::GetKeyCommand => {
                            result.get_key_command = map.next_value()?;
                        }
                        Field::SignCommand => {
                            result.sign_command = map.next_value()?;
                        }
                        Field::VerifyCommand => {
                            result.verify_command = map.next_value()?;
                        }
                        Field::EncryptCommand => {
                            result.encrypt_command = map.next_value()?;
                        }
                        Field::DecryptCommand => {
                            result.decrypt_command = map.next_value()?;
                        }
                        Field::AutoKeyLocate => {
                            result.auto_key_locate = map.next_value()?;
                        }
                    }
                }
                Ok(result)
            }
        }
        deserializer.deserialize_map(CliVisitor)
    }
}
