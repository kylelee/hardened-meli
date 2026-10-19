//
// meli
//
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

//! Fork release `v0.9.0`: <https://github.com/kylelee/hardened-meli>
//!
//! Carries upstream's two `SmtpAuth` configuration migrations for `v0.9.0`
//! (upstream `6e5d5554` + `c2a70993` + the `53900afb` `is_applicable()`
//! fine-tuning, semantically ported): the deprecated SMTP authentication
//! password table syntax (`send_mail.auth.password = { type = "raw",
//! value = "..." }` and `{ type = "command_evaluation", value = "..." }`)
//! and token command syntax (`send_mail.auth.token_command = "..."`) are
//! rewritten in place to the `Secret` syntax the fork's `SmtpAuth` already
//! accepts (`password = "..."`, `password = { command = "..." }`,
//! `token = { command = "..." }`), so configurations predating the
//! Secret-typed auth fields keep validating without deprecation warnings.
//!
//! Fork deviations from upstream:
//!
//! - A `.bkp` backup of the original contents is written next to each
//!   configuration file before it is rewritten (upstream rewrites in place
//!   with no backup).
//! - `is_applicable()` parses each included configuration file as TOML and
//!   only reports applicability when a deprecated key is actually present.
//!   Upstream's first version matched the raw string `send_mail` anywhere
//!   in the file, and the token migration still matches `xoauth2` anywhere.
//! - Verbose logs and error messages never echo password/token values.
//! - `revert()` validates itself with a dry run of the *revert* transform;
//!   upstream (copy-paste) dry-ran the forward `perform` transform instead.

use crate::conf::preprocessing::get_included_configs;
use crate::version_migrations::*;

use toml_edit::{DocumentMut, TableLike};

/// Fork release `v0.9.0`.
pub const V0_9_0_ID: VersionIdentifier = VersionIdentifier {
    string: "0.9.0",
    major: 0,
    minor: 9,
    patch: 0,
    pre: "",
};

/// Fork release `v0.9.0`.
#[derive(Clone, Copy, Debug)]
#[allow(non_camel_case_types)]
pub struct V0_9_0;

impl Version for V0_9_0 {
    fn version(&self) -> &VersionIdentifier {
        &V0_9_0_ID
    }

    fn migrations(&self) -> Vec<Box<dyn Migration + Send + Sync + 'static>> {
        vec![
            Box::new(SmtpAuthPasswordCommand) as Box<dyn Migration + Send + Sync + 'static>,
            Box::new(SmtpAuthTokenCommand) as Box<dyn Migration + Send + Sync + 'static>,
        ]
    }
}

/// Return `path` with `suffix` appended to its file name
/// (`config.toml` + `.bkp` → `config.toml.bkp`).
fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut filename = path.file_name().map(ToOwned::to_owned).unwrap_or_default();
    filename.push(suffix);
    let mut ret = path.to_path_buf();
    ret.set_file_name(filename);
    ret
}

/// Return the path of a backup file for `path` that does not clobber
/// existing backups (`config.toml.bkp`, then `config.toml.bkp1`,
/// `config.toml.bkp2`, ...).
///
/// If a backup with identical `contents` already exists, its path is
/// returned as-is, so repeated migrations do not pile up duplicate backups.
fn create_backup_file(path: &Path, contents: &str, extension: &str) -> Result<PathBuf> {
    let mut path = append_suffix(path, &format!(".{extension}"));
    let mut i: u8 = 0;
    while path.try_exists().unwrap_or(false) {
        if let Ok(previous) = std::fs::read_to_string(&path) {
            if previous == contents {
                break;
            }
        }
        if i == u8::MAX {
            return Err(Error::new(format!(
                "Something has gone horribly wrong, and there are too many (over {}) backup \
                 files of path {}. Please cleanup the folder and try again. meli is aborting \
                 out of caution.",
                u8::MAX,
                path.display()
            ))
            .set_related_path(Some(&path)));
        }
        i += 1;
        path = append_suffix(&path, &i.to_string());
    }
    Ok(path)
}

/// Rewrite every configuration file `config` points at (including any
/// `include`d files) with `transform`.
///
/// Files that fail to parse as TOML are skipped. When `dry_run` is set,
/// nothing is written. When `backup` is set, a `.bkp` backup of the original
/// contents is written next to each file right before it is rewritten; files
/// whose contents the transform did not change are left fully untouched.
fn rewrite_included_configs(
    config: &Path,
    dry_run: bool,
    verbose: bool,
    backup: bool,
    version: &VersionIdentifier,
    id: &str,
    transform: fn(&mut DocumentMut, bool) -> Result<()>,
) -> Result<()> {
    for c in get_included_configs(config)? {
        let raw = std::fs::read_to_string(&c).chain_err_related_path(&c)?;
        let Ok(mut doc) = raw.parse::<DocumentMut>() else {
            if verbose {
                tracing::warn!("Could not parse {} as valid TOML, skipping.", c.display());
            }
            continue;
        };

        transform(&mut doc, verbose).chain_err_related_path(&c)?;

        let new = doc.to_string();
        if new == raw {
            continue;
        }

        if !dry_run {
            if backup {
                let bkp_filepath = create_backup_file(&c, &raw, "bkp")?;
                std::fs::write(&bkp_filepath, &raw).chain_err_related_path(&bkp_filepath)?;
                if verbose {
                    tracing::info!(
                        "Migration {version}/{id}: {} is backed up at {}.",
                        c.display(),
                        bkp_filepath.display()
                    );
                }
            }
            std::fs::write(&c, &new).chain_err_related_path(&c)?;
        }
    }
    Ok(())
}

/// Shared [`Migration::perform`] body: validate the whole migration with an
/// in-memory dry run first, then rewrite the files, backing each rewritten
/// file up.
fn perform_config_rewrite(
    migr: &dyn Migration,
    config: &Path,
    dry_run: bool,
    verbose: bool,
    transform: fn(&mut DocumentMut, bool) -> Result<()>,
) -> Result<()> {
    if !dry_run {
        rewrite_included_configs(
            config,
            true,
            false,
            false,
            migr.version(),
            migr.id(),
            transform,
        )
        .chain_err_summary(|| "No migration was performed.")?;
    }
    rewrite_included_configs(
        config,
        dry_run,
        verbose,
        true,
        migr.version(),
        migr.id(),
        transform,
    )
}

/// Shared [`Migration::revert`] body: like [`perform_config_rewrite`] but
/// without backups; the `.bkp` files left by the forward migration remain
/// the user's safety net.
fn revert_config_rewrite(
    migr: &dyn Migration,
    config: &Path,
    dry_run: bool,
    verbose: bool,
    transform: fn(&mut DocumentMut, bool) -> Result<()>,
) -> Result<()> {
    if !dry_run {
        rewrite_included_configs(
            config,
            true,
            false,
            false,
            migr.version(),
            migr.id(),
            transform,
        )
        .chain_err_summary(|| "No migration revert was performed.")?;
    }
    rewrite_included_configs(
        config,
        dry_run,
        verbose,
        false,
        migr.version(),
        migr.id(),
        transform,
    )
}

/// Whether any configuration file `config` points at (including any
/// `include`d files) parses as TOML and satisfies `predicate`.
fn any_included_config_is(
    config: &Path,
    mut predicate: impl FnMut(&DocumentMut) -> bool,
) -> Option<bool> {
    for c in get_included_configs(config).ok()? {
        let raw = std::fs::read_to_string(&c).ok()?;
        let Ok(doc) = raw.parse::<DocumentMut>() else {
            continue;
        };
        if predicate(&doc) {
            return Some(true);
        }
    }
    Some(false)
}

/// Call `predicate` on every `accounts.*.send_mail.auth` table in `doc`,
/// returning `true` as soon as one call returns `true`.
fn any_send_mail_auth(
    doc: &DocumentMut,
    mut predicate: impl FnMut(&dyn TableLike) -> bool,
) -> bool {
    let Some(accs) = doc.get("accounts") else {
        return false;
    };
    let Some(accs) = accs.as_table_like() else {
        return false;
    };
    for (_, acc) in accs.iter() {
        let Some(auth) = acc
            .as_table_like()
            .and_then(|acc| acc.get("send_mail"))
            .and_then(|send_mail| send_mail.as_table_like())
            .and_then(|send_mail| send_mail.get("auth"))
            .and_then(|auth| auth.as_table_like())
        else {
            continue;
        };
        if predicate(auth) {
            return true;
        }
    }
    false
}

/// Transform the deprecated `send_mail` SMTP authentication password table
/// syntax (`password = { type = "raw", value = "..." }` and
/// `password = { type = "command_evaluation" | "command_eval" |
/// "CommandEval", value = "..." }`) to the `Secret` syntax
/// (`password = "..."` and `password = { command = "..." }`).
///
/// Ported from upstream `6e5d5554` (with the `53900afb` `is_applicable()`
/// fine-tuning). Fork deviation: verbose logging never echoes the password
/// value.
#[derive(Clone, Copy, Debug)]
struct SmtpAuthPasswordCommand;

impl SmtpAuthPasswordCommand {
    fn transform(doc: &mut DocumentMut, verbose: bool) -> Result<()> {
        let Some(accs) = doc.get_mut("accounts") else {
            return Ok(());
        };
        let Some(accs) = accs.as_table_mut() else {
            return Err(Error::new(format!("invalid accounts value, got: {accs:?}")));
        };
        for (acc_name, acc) in accs.iter_mut() {
            let Some(acc) = acc.as_table_like_mut() else {
                continue;
            };
            let Some(send_mail) = acc.get_mut("send_mail") else {
                continue;
            };
            let Some(send_mail) = send_mail.as_table_like_mut() else {
                continue;
            };
            let Some(auth) = send_mail.get_mut("auth") else {
                continue;
            };
            let Some(auth) = auth.as_table_like_mut() else {
                continue;
            };
            let Some(password) = auth.get_mut("password") else {
                continue;
            };
            let Some(type_) = password.get("type") else {
                continue;
            };
            let value = password
                .get("value")
                .and_then(|v| v.as_str())
                .map(str::to_owned);
            let new = if matches!(type_.as_str(), Some(raw) if raw.eq_ignore_ascii_case("raw")) {
                value.map(|value| toml_edit::Item::Value(toml_edit::Value::from(value)))
            } else if matches!(
                type_.as_str(),
                Some(command) if command.eq_ignore_ascii_case("command_eval")
                    || command.eq_ignore_ascii_case("command_evaluation")
                    || command.eq_ignore_ascii_case("CommandEval")
            ) {
                value.map(|value| {
                    let mut new_entry = toml_edit::InlineTable::new();
                    new_entry.insert("command", toml_edit::Value::from(value));
                    toml_edit::Item::Value(toml_edit::Value::from(new_entry))
                })
            } else {
                None
            };
            let Some(new) = new else {
                continue;
            };
            if auth.remove("password").is_some() {
                if verbose {
                    tracing::info!(
                        "Account {acc_name}: converting the deprecated \
                         `send_mail.auth.password` table syntax to the `Secret` syntax."
                    );
                }
                auth.insert("password", new);
            }
        }
        Ok(())
    }

    fn revert_transform(doc: &mut DocumentMut, verbose: bool) -> Result<()> {
        let Some(accs) = doc.get_mut("accounts") else {
            return Ok(());
        };
        let Some(accs) = accs.as_table_mut() else {
            return Err(Error::new(format!("invalid accounts value, got: {accs:?}")));
        };
        for (acc_name, acc) in accs.iter_mut() {
            let Some(acc) = acc.as_table_like_mut() else {
                continue;
            };
            let Some(send_mail) = acc.get_mut("send_mail") else {
                continue;
            };
            let Some(send_mail) = send_mail.as_table_like_mut() else {
                continue;
            };
            let Some(auth) = send_mail.get_mut("auth") else {
                continue;
            };
            let Some(auth) = auth.as_table_like_mut() else {
                continue;
            };
            let Some(password) = auth.get_mut("password") else {
                continue;
            };
            let new = if let Some(value) = password.as_str().map(str::to_owned) {
                let mut new = toml_edit::InlineTable::new();
                new.insert("type", "raw".into());
                new.insert("value", toml_edit::Value::from(value));
                new
            } else if let Some(command) = password
                .as_table_like()
                .and_then(|password| password.get("command"))
                .and_then(|v| v.as_str())
                .map(str::to_owned)
            {
                let mut new = toml_edit::InlineTable::new();
                new.insert("type", "command_evaluation".into());
                new.insert("value", toml_edit::Value::from(command));
                new
            } else {
                continue;
            };
            if auth.remove("password").is_some() {
                if verbose {
                    tracing::info!(
                        "Account {acc_name}: reverting `send_mail.auth.password` to the \
                         deprecated table syntax."
                    );
                }
                auth.insert(
                    "password",
                    toml_edit::Item::Value(toml_edit::Value::InlineTable(new)),
                );
            }
        }
        Ok(())
    }

    /// Whether `doc` contains any deprecated `send_mail.auth.password`
    /// table value (a `password` entry with a legacy `type` field).
    fn is_applicable_doc(doc: &DocumentMut) -> bool {
        any_send_mail_auth(doc, |auth| {
            auth.get("password")
                .is_some_and(|password| password.get("type").is_some())
        })
    }
}

impl Migration for SmtpAuthPasswordCommand {
    fn id(&self) -> &'static str {
        melib::identify! { SmtpAuthPasswordCommand }
    }

    fn version(&self) -> &VersionIdentifier {
        &V0_9_0_ID
    }

    fn description(&self) -> &str {
        "Transform `send_mail = { password = { type = \"raw\", value = \"...\" } }` to new syntax: `send_mail = { password = \"...\" }`"
    }

    fn question(&self) -> &str {
        "Transform `send_mail = { password = { type = \"raw\", value = \"...\" } }` to new syntax?"
    }

    fn is_applicable(&self, config: &Path) -> Option<bool> {
        any_included_config_is(config, Self::is_applicable_doc)
    }

    fn perform(&self, config: &Path, dry_run: bool, verbose: bool) -> Result<()> {
        perform_config_rewrite(self, config, dry_run, verbose, Self::transform)
    }

    fn revert(&self, config: &Path, dry_run: bool, verbose: bool) -> Result<()> {
        revert_config_rewrite(self, config, dry_run, verbose, Self::revert_transform)
    }
}

/// Transform the deprecated `send_mail` SMTP authentication token command
/// syntax (`token_command = "..."` in `auth` tables of `xoauth2` or
/// `oauthbearer` type) to the `Secret` syntax
/// (`token = { command = "..." }`).
///
/// Ported from upstream `c2a70993`. Fork deviations: `is_applicable()`
/// looks for the deprecated `token_command` key itself instead of matching
/// the string `xoauth2` anywhere in the file, and verbose logging never
/// echoes the command.
#[derive(Clone, Copy, Debug)]
struct SmtpAuthTokenCommand;

impl SmtpAuthTokenCommand {
    fn transform(doc: &mut DocumentMut, verbose: bool) -> Result<()> {
        let Some(accs) = doc.get_mut("accounts") else {
            return Ok(());
        };
        let Some(accs) = accs.as_table_mut() else {
            return Err(Error::new(format!("invalid accounts value, got: {accs:?}")));
        };
        for (acc_name, acc) in accs.iter_mut() {
            let Some(acc) = acc.as_table_like_mut() else {
                continue;
            };
            let Some(send_mail) = acc.get_mut("send_mail") else {
                continue;
            };
            let Some(send_mail) = send_mail.as_table_like_mut() else {
                continue;
            };
            let Some(auth) = send_mail.get_mut("auth") else {
                continue;
            };
            let Some(auth) = auth.as_table_like_mut() else {
                continue;
            };
            let Some(value) = auth
                .get("token_command")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
            else {
                continue;
            };
            if auth.remove("token_command").is_some() {
                let mut new_entry = toml_edit::InlineTable::new();
                new_entry.insert("command", toml_edit::Value::from(value));
                let new = toml_edit::Item::Value(toml_edit::Value::from(new_entry));
                if verbose {
                    tracing::info!(
                        "Account {acc_name}: converting `send_mail.auth.token_command` to \
                         `token = {{ command = \"...\" }}`."
                    );
                }
                auth.insert("token", new);
            }
        }
        Ok(())
    }

    fn revert_transform(doc: &mut DocumentMut, verbose: bool) -> Result<()> {
        let Some(accs) = doc.get_mut("accounts") else {
            return Ok(());
        };
        let Some(accs) = accs.as_table_mut() else {
            return Err(Error::new(format!("invalid accounts value, got: {accs:?}")));
        };
        for (acc_name, acc) in accs.iter_mut() {
            let Some(acc) = acc.as_table_like_mut() else {
                continue;
            };
            let Some(send_mail) = acc.get_mut("send_mail") else {
                continue;
            };
            let Some(send_mail) = send_mail.as_table_like_mut() else {
                continue;
            };
            let Some(auth) = send_mail.get_mut("auth") else {
                continue;
            };
            let Some(auth) = auth.as_table_like_mut() else {
                continue;
            };
            let Some(value) = auth
                .get("token")
                .and_then(|v| v.as_table_like())
                .and_then(|token| token.get("command"))
                .and_then(|v| v.as_str())
                .map(str::to_owned)
            else {
                continue;
            };
            if auth.remove("token").is_some() {
                let new = toml_edit::Item::Value(toml_edit::Value::from(value));
                if verbose {
                    tracing::info!(
                        "Account {acc_name}: reverting `send_mail.auth.token` back to \
                         `token_command`."
                    );
                }
                auth.insert("token_command", new);
            }
        }
        Ok(())
    }

    /// Whether `doc` contains any deprecated `send_mail.auth.token_command`
    /// key.
    fn is_applicable_doc(doc: &DocumentMut) -> bool {
        any_send_mail_auth(doc, |auth| auth.contains_key("token_command"))
    }
}

impl Migration for SmtpAuthTokenCommand {
    fn id(&self) -> &'static str {
        melib::identify! { SmtpAuthTokenCommand }
    }

    fn version(&self) -> &VersionIdentifier {
        &V0_9_0_ID
    }

    fn description(&self) -> &str {
        "Transform `send_mail = { auth = { type = \"xoauth2\", token_command = \"...\" } }` to Secret"
    }

    fn question(&self) -> &str {
        "Transform `send_mail = { auth = { type = \"xoauth2\", token_command = \"...\" } }` to new syntax?"
    }

    fn is_applicable(&self, config: &Path) -> Option<bool> {
        any_included_config_is(config, Self::is_applicable_doc)
    }

    fn perform(&self, config: &Path, dry_run: bool, verbose: bool) -> Result<()> {
        perform_config_rewrite(self, config, dry_run, verbose, Self::transform)
    }

    fn revert(&self, config: &Path, dry_run: bool, verbose: bool) -> Result<()> {
        revert_config_rewrite(self, config, dry_run, verbose, Self::revert_transform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deprecated password table syntax is rewritten to the `Secret` syntax
    /// and revert restores it (ported from upstream, aligned to fork key
    /// names).
    #[test]
    fn test_smtp_auth_password_command_transform() {
        let input = r#"
accounts.imap2 = { send_mail = { auth = { type = "auto", username = "user@example.com", password = { type = "raw", value = "hunter2" } }, security = { type = "STARTTLS" } } }

[accounts.imap]
send_mail = { auth = { type = "auto", username = "user@example.com", password = { type = "raw", value = "hunter2" } }, security = { type = "STARTTLS" } }

[accounts.imap3]
send_mail = { auth = { type = "auto", username = "user@example.com", password = { type = "command_evaluation", value = "echo hunter2" } }, security = { type = "STARTTLS" } }
"#;
        let mut doc = input.parse::<DocumentMut>().unwrap();

        SmtpAuthPasswordCommand::transform(&mut doc, true).unwrap();
        assert_eq!(
            doc.to_string(),
            r#"
accounts.imap2 = { send_mail = { auth = { type = "auto", username = "user@example.com", password = "hunter2" }, security = { type = "STARTTLS" } } }

[accounts.imap]
send_mail = { auth = { type = "auto", username = "user@example.com", password = "hunter2" }, security = { type = "STARTTLS" } }

[accounts.imap3]
send_mail = { auth = { type = "auto", username = "user@example.com", password = { command = "echo hunter2" } }, security = { type = "STARTTLS" } }
"#
        );
        SmtpAuthPasswordCommand::revert_transform(&mut doc, true).unwrap();
        assert_eq!(doc.to_string(), input);
    }

    /// Deprecated `token_command` is rewritten to `token = { command = ... }`
    /// and revert restores it (ported from upstream).
    #[test]
    fn test_smtp_auth_token_command_transform() {
        let input = r#"
accounts.imap2 = { send_mail = { auth = { type = "xoauth2", token_command = "echo hunter2" } } }

[accounts.imap]
send_mail = { auth = { type = "xoauth2", token_command = "echo hunter2" } }
"#;
        let mut doc = input.parse::<DocumentMut>().unwrap();

        SmtpAuthTokenCommand::transform(&mut doc, true).unwrap();
        assert_eq!(
            doc.to_string(),
            r#"
accounts.imap2 = { send_mail = { auth = { type = "xoauth2", token = { command = "echo hunter2" } } } }

[accounts.imap]
send_mail = { auth = { type = "xoauth2", token = { command = "echo hunter2" } } }
"#
        );
        SmtpAuthTokenCommand::revert_transform(&mut doc, true).unwrap();
        assert_eq!(doc.to_string(), input);
    }

    /// `is_applicable()` must only trigger on the deprecated keys, not on
    /// any `send_mail` value (the `53900afb` fine-tuning).
    #[test]
    fn test_smtp_auth_migrations_is_applicable() {
        let tempdir = tempfile::tempdir().unwrap();
        let config = tempdir.path().join("meli.toml");

        // New `Secret` syntax only: no migration must trigger.
        std::fs::write(
            &config,
            r#"
[accounts.imap]
format = "imap"
send_mail = { hostname = "smtp.example.com", auth = { type = "auto", username = "user@example.com", password = { command = "echo hunter2" } }, security = { type = "STARTTLS" } }

[accounts.gmail]
format = "imap"
send_mail = { hostname = "smtp.gmail.com", auth = { type = "xoauth2", token = { command = "echo token" } }, security = { type = "TLS" } }
"#,
        )
        .unwrap();
        assert_eq!(SmtpAuthPasswordCommand.is_applicable(&config), Some(false));
        assert_eq!(SmtpAuthTokenCommand.is_applicable(&config), Some(false));

        // `send_mail = false` (no auth table at all): not applicable either.
        std::fs::write(
            &config,
            r#"
[accounts.imap]
format = "imap"
send_mail = 'false'
"#,
        )
        .unwrap();
        assert_eq!(SmtpAuthPasswordCommand.is_applicable(&config), Some(false));
        assert_eq!(SmtpAuthTokenCommand.is_applicable(&config), Some(false));

        // Deprecated password table syntax: only the password migration
        // applies.
        std::fs::write(
            &config,
            r#"
[accounts.imap]
format = "imap"
send_mail = { auth = { type = "auto", username = "user@example.com", password = { type = "raw", value = "hunter2" } } }
"#,
        )
        .unwrap();
        assert_eq!(SmtpAuthPasswordCommand.is_applicable(&config), Some(true));
        assert_eq!(SmtpAuthTokenCommand.is_applicable(&config), Some(false));

        // Deprecated `token_command`: only the token migration applies.
        std::fs::write(
            &config,
            r#"
[accounts.gmail]
format = "imap"
send_mail = { auth = { type = "xoauth2", token_command = "echo token" } }
"#,
        )
        .unwrap();
        assert_eq!(SmtpAuthPasswordCommand.is_applicable(&config), Some(false));
        assert_eq!(SmtpAuthTokenCommand.is_applicable(&config), Some(true));
    }

    /// `perform()` on a real file: dry run touches nothing; the real run
    /// rewrites deprecated keys and leaves a `.bkp` backup behind; a second
    /// migration gets a `.bkp1` backup; re-running is a no-op without new
    /// backups; `revert()` restores the deprecated syntax.
    #[test]
    fn test_smtp_auth_migrations_perform_backup() {
        let tempdir = tempfile::tempdir().unwrap();
        let config = tempdir.path().join("meli.toml");
        let original = r#"
[accounts.imap]
format = "imap"
identity = "username@example.com"
send_mail = { hostname = "smtp.example.com", auth = { type = "auto", username = "user@example.com", password = { type = "command_eval", value = "echo hunter2" } }, security = { type = "STARTTLS" } }

[accounts.gmail]
format = "imap"
identity = "username@example.com"
send_mail = { hostname = "smtp.gmail.com", auth = { type = "xoauth2", token_command = "echo token" }, security = { type = "TLS" } }
"#;
        std::fs::write(&config, original).unwrap();

        // Dry run: file untouched, no backup.
        SmtpAuthPasswordCommand
            .perform(&config, true, false)
            .unwrap();
        SmtpAuthTokenCommand.perform(&config, true, false).unwrap();
        assert_eq!(std::fs::read_to_string(&config).unwrap(), original);
        assert!(!tempdir.path().join("meli.toml.bkp").try_exists().unwrap());

        // Password migration: rewrite + `.bkp` backup of the original.
        SmtpAuthPasswordCommand
            .perform(&config, false, false)
            .unwrap();
        let migrated = std::fs::read_to_string(&config).unwrap();
        assert!(
            migrated.contains("password = { command = \"echo hunter2\" }"),
            "expected the password entry to be converted; got:\n{migrated}"
        );
        assert!(!migrated.contains("command_eval"));
        // The password migration must not touch the token syntax; the
        // separate token migration handles that below.
        assert!(migrated.contains("token_command = \"echo token\""));
        assert_eq!(
            std::fs::read_to_string(tempdir.path().join("meli.toml.bkp")).unwrap(),
            original
        );

        // Token migration: rewrite + `.bkp1` backup (the `.bkp` contents
        // differ, so it must not be clobbered).
        SmtpAuthTokenCommand.perform(&config, false, false).unwrap();
        let migrated2 = std::fs::read_to_string(&config).unwrap();
        assert!(
            migrated2.contains("token = { command = \"echo token\" }"),
            "expected the token entry to be converted; got:\n{migrated2}"
        );
        assert!(!migrated2.contains("token_command"));
        assert_eq!(
            std::fs::read_to_string(tempdir.path().join("meli.toml.bkp1")).unwrap(),
            migrated
        );

        // Re-running on the already-migrated file changes nothing and does
        // not pile up new backups.
        SmtpAuthPasswordCommand
            .perform(&config, false, false)
            .unwrap();
        SmtpAuthTokenCommand.perform(&config, false, false).unwrap();
        assert_eq!(std::fs::read_to_string(&config).unwrap(), migrated2);
        assert!(!tempdir.path().join("meli.toml.bkp2").try_exists().unwrap());

        // Revert restores the deprecated syntax.
        SmtpAuthTokenCommand.revert(&config, false, false).unwrap();
        SmtpAuthPasswordCommand
            .revert(&config, false, false)
            .unwrap();
        let reverted = std::fs::read_to_string(&config).unwrap();
        assert!(
            reverted
                .contains("password = { type = \"command_evaluation\", value = \"echo hunter2\" }"),
            "expected revert to restore the password table syntax; got:\n{reverted}"
        );
        assert!(
            reverted.contains("token_command = \"echo token\""),
            "expected revert to restore token_command; got:\n{reverted}"
        );
    }
}
