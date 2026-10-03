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

//! Fork release `v0.10.0`: <https://github.com/kylelee/hardened-meli>
//!
//! Carries upstream's `ServerPasswordCommand` migration (upstream 483f0629 in
//! `v0_9_0`) because the fork's `v0_9_0` predates the Secret-typed
//! `server_password = { command = "..." }` change. Already-installed
//! `v0.9.0` users are walked through the new form here.

use crate::conf::preprocessing::get_included_configs;
use crate::version_migrations::*;

use toml_edit::DocumentMut;

/// Fork release `v0.10.0`.
pub const V0_10_0_ID: VersionIdentifier = VersionIdentifier {
    string: "0.10.0",
    major: 0,
    minor: 10,
    patch: 0,
    pre: "",
};

/// Fork release `v0.10.0`.
#[derive(Clone, Copy, Debug)]
#[allow(non_camel_case_types)]
pub struct V0_10_0;

impl Version for V0_10_0 {
    fn version(&self) -> &VersionIdentifier {
        &V0_10_0_ID
    }

    fn migrations(&self) -> Vec<Box<dyn Migration + Send + Sync + 'static>> {
        vec![Box::new(ServerPasswordCommand) as Box<dyn Migration + Send + Sync + 'static>]
    }
}

/// Transform `server_password_command` to `server_password` secret
/// (`server_password = { command = "..." }`).
///
/// Ported from upstream commit 483f0629 (and the surrounding 254cee97).
/// Fork deviation: the description and question wording match the existing
/// fork migrations' style and the hardened fork's error messages do not
/// include the command string (which can be sensitive).
#[derive(Clone, Copy, Debug)]
struct ServerPasswordCommand;

impl ServerPasswordCommand {
    fn transform(doc: &mut DocumentMut, verbose: bool) -> Result<()> {
        let Some(accs) = doc.get_mut("accounts") else {
            return Ok(());
        };
        let Some(accs) = accs.as_table_mut() else {
            return Err(Error::new(format!("invalid accounts value, got: {accs:?}")));
        };
        for (acc_name, acc) in accs.iter_mut() {
            let Some(acc) = acc.as_table_like_mut() else {
                return Err(Error::new(format!("invalid account value, got: {acc:?}")));
            };
            let mut new_entry = toml_edit::InlineTable::new();
            if let Some(pass) = acc.get_mut("server_password_command") {
                // Fork hardening: only emit that a conversion occurred in
                // verbose mode; never echo the command (it can be sensitive)
                // back to the user.
                let pass = pass.as_str().ok_or_else(|| {
                    format!("Expected string value for server_password_command field, got: {pass}")
                })?;
                new_entry.insert("command", pass.into());
            }
            if let Some(_prev) = acc.remove("server_password_command") {
                let new = toml_edit::Item::Value(toml_edit::Value::from(new_entry));
                if verbose {
                    log::info!(
                        "Account {acc_name}: converting server_password_command to new \
                         `server_password = {{ command = \"...\" }}` syntax."
                    );
                }
                acc.insert("server_password", new);
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
                return Err(Error::new(format!("invalid accounts value, got: {acc:?}")));
            };
            if let Some(pass) = acc.get("server_password") {
                let Some(pass_table) = pass.as_table_like() else {
                    continue;
                };
                let command = pass_table
                    .get("command")
                    .ok_or_else(|| {
                        format!("Expected server_password command field, got: {pass:?}")
                    })?
                    .as_str()
                    .ok_or_else(|| {
                        format!(
                            "Expected string value for server_password command field, got: \
                             {pass:?}"
                        )
                    })?
                    .to_string();
                if let Some(_prev) = acc.remove("server_password") {
                    let new = toml_edit::Item::Value(toml_edit::Value::from(command));
                    if verbose {
                        log::info!(
                            "Account {acc_name}: reverting server_password inline table back to \
                             server_password_command."
                        );
                    }
                    acc.insert("server_password_command", new);
                }
            }
        }
        Ok(())
    }
}

impl Migration for ServerPasswordCommand {
    fn id(&self) -> &'static str {
        melib::identify! { ServerPasswordCommand }
    }

    fn version(&self) -> &VersionIdentifier {
        &V0_10_0_ID
    }

    fn description(&self) -> &str {
        "Transform `server_password_command` to new syntax: `server_password = { command = \
         \"...\" }`"
    }

    fn question(&self) -> &str {
        "Transform `server_password_command` to new syntax?"
    }

    fn is_applicable(&self, config: &Path) -> Option<bool> {
        for c in get_included_configs(config).ok()? {
            let raw = std::fs::read_to_string(&c).ok()?;
            if raw.contains("server_password_command") {
                return Some(true);
            }
        }

        Some(false)
    }

    fn perform(&self, config: &Path, dry_run: bool, verbose: bool) -> Result<()> {
        for c in get_included_configs(config)? {
            if !dry_run {
                self.perform(config, true, false)
                    .chain_err_summary(|| "No migration was performed.")?;
            }
            let raw = std::fs::read_to_string(&c).chain_err_related_path(&c)?;
            let Ok(mut doc) = raw.parse::<DocumentMut>() else {
                continue;
            };

            Self::transform(&mut doc, verbose)?;

            if !dry_run {
                std::fs::write(&c, doc.to_string()).chain_err_related_path(&c)?;
            }
        }
        Ok(())
    }

    fn revert(&self, config: &Path, dry_run: bool, verbose: bool) -> Result<()> {
        for c in get_included_configs(config)? {
            if !dry_run {
                self.perform(config, true, false)
                    .chain_err_summary(|| "No migration revert was performed.")?;
            }
            let raw = std::fs::read_to_string(&c).chain_err_related_path(&c)?;
            let Ok(mut doc) = raw.parse::<DocumentMut>() else {
                continue;
            };

            Self::revert_transform(&mut doc, verbose)?;

            if !dry_run {
                std::fs::write(&c, doc.to_string()).chain_err_related_path(&c)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use melib::utils::logging::{LogLevel, Logger};

    #[test]
    fn test_version_migration_server_password_command() {
        tokio_test::block_on(async {
            run_version_migration_server_password_command()
        });
    }

    fn run_version_migration_server_password_command() {
        let _logger = Logger::new_with(LogLevel::TRACE, true);
        let input = r#"
accounts.imap2 = { format = "imap", server_password_command = "echo hunter2" }

[accounts.imap]
root_mailbox = "INBOX"
format = "imap"
send_mail = 'false'
identity="username@example.com"
server_username = "null"
server_hostname = "example.com"
server_password_command = "false"


[accounts.jmap]
root_mailbox = "INBOX"
format = ".map"
send_mail = 'false'
identity="username@example.com"
server_username = "null"
server_hostname = "example.com"
server_password = "hunter2"
"#;
        let mut doc = input.parse::<DocumentMut>().unwrap();

        ServerPasswordCommand::transform(&mut doc, true).unwrap();
        // Both imap and imap2 should now carry `server_password = { command
        // = "..." }`, and jmap should be left alone since it never had a
        // server_password_command entry.
        let out = doc.to_string();
        assert!(
            out.contains("server_password = { command = \"echo hunter2\" }")
                || out.contains("server_password = {command = \"echo hunter2\"}"),
            "expected the imap2 entry to be converted; got:\n{out}"
        );
        // All `server_password_command` entries must be gone after the
        // transform: a partial conversion that leaves any behind (even
        // exactly one) is a failure.
        assert_eq!(
            out.matches("server_password_command").count(),
            0,
            "expected every server_password_command entry to be transformed; got:\n{out}"
        );

        // Revert and check that server_password_command is restored.
        let mut doc = out.parse::<DocumentMut>().unwrap();
        ServerPasswordCommand::revert_transform(&mut doc, true).unwrap();
        let out = doc.to_string();
        assert!(
            out.contains("server_password_command = \"echo hunter2\""),
            "expected revert to restore server_password_command; got:\n{out}"
        );
    }
}
