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

//! Fork release `v0.12.0`：<https://github.com/kylelee/hardened-meli>
//!
//! 承载上游 `ContactBackendRefactor` 迁移（上游 db9f7f93，位于其 `v0_9_0`）：
//! fork 的 `v0_9_0` 早于带类型的 `contacts` 后端映射。迁移把每个账号下的
//! `notmuch_address_book_query`、`mutt_alias_file`、`vcard_folder` 旧键搬进该
//! 账号的 `contacts` 后端映射，键名保持为原来的键名。

use crate::conf::preprocessing::get_included_configs;
use crate::version_migrations::*;

use toml_edit::DocumentMut;

/// Fork release `v0.12.0`.
pub const V0_12_0_ID: VersionIdentifier = VersionIdentifier {
    string: "0.12.0",
    major: 0,
    minor: 12,
    patch: 0,
    pre: "",
};

/// Fork release `v0.12.0`.
#[derive(Clone, Copy, Debug)]
#[allow(non_camel_case_types)]
pub struct V0_12_0;

impl Version for V0_12_0 {
    fn version(&self) -> &VersionIdentifier {
        &V0_12_0_ID
    }

    fn migrations(&self) -> Vec<Box<dyn Migration + Send + Sync + 'static>> {
        vec![Box::new(ContactBackendRefactor) as Box<dyn Migration + Send + Sync + 'static>]
    }
}

/// 新增联系人后端配置项
///
/// 把 `notmuch_address_book_query`、`mutt_alias_file`、`vcard_folder` 搬进单一的
/// `contacts` 配置项。
#[derive(Clone, Copy, Debug)]
struct ContactBackendRefactor;

impl ContactBackendRefactor {
    /// 把 `path` 复制为同目录下以 `extension` 为扩展名的兄弟文件；若已存在同名
    /// 备份，则依次尝试 `extension{N}`，内容相同则复用，避免重复迁移覆盖更早的
    /// 备份。
    ///
    /// 命名沿用上游 `AddressbookRefactor::create_backup_file` 的约定
    /// （`contacts_new.bkp`、`contacts_new.bkp1`……）。
    fn create_backup_file(path: &Path, contents: &str, extension: &str) -> Result<PathBuf> {
        let mut path = path.to_path_buf();
        path.set_extension(extension);
        let mut i: u8 = 0;
        while path.try_exists().unwrap_or(false) {
            if let Ok(c) = std::fs::read_to_string(&path) {
                if c == contents {
                    break;
                }
            }
            if i == u8::MAX {
                path.set_extension("");
                return Err(Error::new(format!(
                    "Something has gone horribly wrong, and there are too many (over {}) backup \
                     files of path {}. Please cleanup the folder and try again. meli is aborting \
                     out of caution.",
                    u8::MAX,
                    path.display()
                ))
                .set_related_path(Some(path)));
            }
            i += 1;
            path.set_extension(format!("{extension}{i}"));
        }
        Ok(path)
    }

    fn transform(doc: &mut DocumentMut, verbose: bool) -> Result<()> {
        use melib::conf::ContactBackendConf;

        if let Some(accs) = doc.get_mut("accounts") {
            let Some(accs) = accs.as_table_mut() else {
                return Err(Error::new(format!("invalid accounts value, got: {accs:?}")));
            };
            for (acc_name, acc) in accs.iter_mut() {
                let Some(acc) = acc.as_table_like_mut() else {
                    return Err(Error::new(format!("invalid account value, got: {acc:?}")));
                };
                let mut contacts = toml_edit::InlineTable::new();
                for (t, f) in [
                    (
                        ContactBackendConf::NotmuchAddress as fn(String) -> ContactBackendConf,
                        "notmuch_address_book_query",
                    ),
                    (ContactBackendConf::MuttAlias, "mutt_alias_file"),
                    (ContactBackendConf::VCard, "vcard_folder"),
                ] {
                    if let Some(val) = acc.get_mut(f) {
                        // fork 加固：旧键必须是字符串；非字符串值视为配置错误，
                        // 而不是静默跳过。
                        let val = val.as_str().ok_or_else(|| {
                            format!("Expected string value for {f} field, got: {val}")
                        })?;
                        let val = t(val.to_string());
                        let val = serde::Serialize::serialize(
                            &val,
                            toml_edit::ser::ValueSerializer::new(),
                        )
                        .expect("serialize ContactBackendConf");
                        contacts.insert(f, val);
                        acc.remove(f);
                    }
                }
                if !contacts.is_empty() {
                    let new = toml_edit::Item::Value(toml_edit::Value::InlineTable(contacts));
                    if verbose {
                        tracing::info!("Account {acc_name}: adding contacts = {new}.");
                    }
                    acc.insert("contacts", new);
                }
            }
        }
        Ok(())
    }

    fn revert_transform(doc: &mut DocumentMut, verbose: bool) -> Result<()> {
        use melib::conf::ContactBackendConf;

        use serde::de::IntoDeserializer as _;

        if let Some(accs) = doc.get_mut("accounts") {
            let Some(accs) = accs.as_table_mut() else {
                return Err(Error::new(format!("invalid accounts value, got: {accs:?}")));
            };
            for (acc_name, acc) in accs.iter_mut() {
                let Some(acc) = acc.as_table_like_mut() else {
                    return Err(Error::new(format!("invalid account value, got: {acc:?}")));
                };
                if let Some(contacts) = acc.get_mut("contacts") {
                    let Some(contacts) = contacts.as_table_like_mut() else {
                        continue;
                    };
                    let mut notmuch_address_book_query: Option<String> = None;
                    let mut mutt_alias_file = None;
                    let mut vcard_folder = None;
                    for (opt, variant, f) in [
                        (
                            &mut notmuch_address_book_query,
                            ContactBackendConf::NotmuchAddress(String::new()),
                            "notmuch_address_book_query",
                        ),
                        (
                            &mut mutt_alias_file,
                            ContactBackendConf::MuttAlias(String::new()),
                            "mutt_alias_file",
                        ),
                        (
                            &mut vcard_folder,
                            ContactBackendConf::VCard(String::new()),
                            "vcard_folder",
                        ),
                    ] {
                        let Some(prev) = contacts.remove(f) else {
                            continue;
                        };
                        let toml_edit::Item::Value(prev) = prev else {
                            return Err(Error::new(format!(
                                "{acc_name}: invalid {f} value, got: {prev:?}"
                            )));
                        };
                        let prev = <ContactBackendConf as serde::Deserialize>::deserialize(
                            prev.into_deserializer(),
                        )
                        .unwrap();

                        let val = match (prev, variant) {
                            (
                                ContactBackendConf::MuttAlias(val),
                                ContactBackendConf::MuttAlias(_),
                            )
                            | (ContactBackendConf::VCard(val), ContactBackendConf::VCard(_))
                            | (
                                ContactBackendConf::NotmuchAddress(val),
                                ContactBackendConf::NotmuchAddress(_),
                            ) => val,
                            (prev, _) => {
                                return Err(Error::new(format!(
                                    "{acc_name}: invalid {f} value, got: {prev:?}"
                                )));
                            }
                        };
                        *opt = Some(val);
                    }
                    if !contacts.is_empty() {
                        return Err(Error::new(format!(
                            "{acc_name}: could not revert migration because we would have to \
                             remove these unmigrateable items: {:?}",
                            acc.get("contacts").map(ToString::to_string)
                        )));
                    }
                    acc.remove("contacts");
                    if let Some(n) = notmuch_address_book_query {
                        let new = toml_edit::Item::Value(toml_edit::Value::from(n));
                        if verbose {
                            tracing::info!(
                                "Account {acc_name}: adding notmuch_address_book_query = {new}."
                            );
                        }
                        acc.insert("notmuch_address_book_query", new);
                    }
                    if let Some(n) = mutt_alias_file {
                        let new = toml_edit::Item::Value(toml_edit::Value::from(n));
                        if verbose {
                            tracing::info!("Account {acc_name}: adding mutt_alias_file = {new}.");
                        }
                        acc.insert("mutt_alias_file", new);
                    }
                    if let Some(n) = vcard_folder {
                        let new = toml_edit::Item::Value(toml_edit::Value::from(n));
                        if verbose {
                            tracing::info!("Account {acc_name}: adding vcard_folder = {new}.");
                        }
                        acc.insert("vcard_folder", new);
                    }
                }
            }
        }
        Ok(())
    }
}

impl Migration for ContactBackendRefactor {
    fn id(&self) -> &'static str {
        melib::identify! { ContactBackendRefactor }
    }

    fn version(&self) -> &VersionIdentifier {
        &V0_12_0_ID
    }

    fn description(&self) -> &str {
        "`notmuch_address_book_query`, `mutt_alias_file`, `vcard_folder` are moved into a single \
         `contacts` configuration value"
    }

    fn question(&self) -> &str {
        "Consolidate `notmuch_address_book_query`, `mutt_alias_file`, `vcard_folder` options into \
         the new `contacts` field?"
    }

    fn is_applicable(&self, config: &Path) -> Option<bool> {
        if !config.try_exists().unwrap_or(false) {
            return Some(false);
        }
        for c in get_included_configs(config).ok()? {
            let raw = std::fs::read_to_string(&c).ok()?;
            if raw.contains("notmuch_address_book_query")
                || raw.contains("mutt_alias_file")
                || raw.contains("vcard_folder")
            {
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
                if verbose {
                    tracing::warn!("Could not parse {} as valid TOML, skipping.", c.display());
                }
                continue;
            };

            Self::transform(&mut doc, verbose).chain_err_related_path(&c)?;

            // 幂等：变换未产生改动时，配置文件和备份都保持原样。
            let new = doc.to_string();
            if !dry_run && new != raw {
                let bkp_filepath = Self::create_backup_file(&c, &raw, "bkp")?;
                std::fs::write(&bkp_filepath, &raw).chain_err_related_path(&bkp_filepath)?;
                if verbose {
                    tracing::info!(
                        "Migration {}/{}: {} is backed up at {}.",
                        self.version().as_str(),
                        self.id(),
                        c.display(),
                        bkp_filepath.display()
                    );
                }
                std::fs::write(&c, new).chain_err_related_path(&c)?;
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
                if verbose {
                    tracing::warn!("Could not parse {} as valid TOML, skipping.", c.display());
                }
                continue;
            };

            Self::revert_transform(&mut doc, verbose).chain_err_related_path(&c)?;

            let new = doc.to_string();
            if !dry_run && new != raw {
                std::fs::write(&c, new).chain_err_related_path(&c)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_migration_contact_backend_refactor() {
        run_contact_backend_refactor()
    }

    #[test]
    fn test_contact_backend_refactor_backup_file() {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("meli.toml");
        std::fs::write(&path, "first contents").unwrap();
        let bkp =
            ContactBackendRefactor::create_backup_file(&path, "first contents", "bkp").unwrap();
        assert_eq!(
            bkp.file_name().unwrap().to_string_lossy().as_ref(),
            // `set_extension` 会替换掉原来的 `.toml` 扩展名。
            "meli.bkp"
        );
        // 已存在且内容相同的备份会被复用。
        std::fs::write(&bkp, "first contents").unwrap();
        let bkp_again =
            ContactBackendRefactor::create_backup_file(&path, "first contents", "bkp").unwrap();
        assert_eq!(
            bkp_again.file_name().unwrap().to_string_lossy().as_ref(),
            "meli.bkp"
        );
        // 内容不同的备份不能覆盖既有备份。
        let bkp2 =
            ContactBackendRefactor::create_backup_file(&path, "second contents", "bkp").unwrap();
        assert_eq!(
            bkp2.file_name().unwrap().to_string_lossy().as_ref(),
            "meli.bkp1"
        );
    }

    fn run_contact_backend_refactor() {
        let input = r#"
[accounts.imap]
root_mailbox = "INBOX"
format = "imap"
send_mail = 'false'
identity="username@example.com"
server_username = "null"
server_hostname = "example.com"
server_password = "false"
notmuch_address_book_query = "notmuch"
mutt_alias_file = "mutt_alias"
vcard_folder = "vcard_folder"

"#;
        let mut doc = input.parse::<DocumentMut>().unwrap();

        ContactBackendRefactor::transform(&mut doc, true).unwrap();
        let migrated = doc.to_string();
        assert_eq!(
            migrated,
            r#"
[accounts.imap]
root_mailbox = "INBOX"
format = "imap"
send_mail = 'false'
identity="username@example.com"
server_username = "null"
server_hostname = "example.com"
server_password = "false"
contacts = { notmuch_address_book_query = { type = "NotmuchAddress", value = "notmuch" }, mutt_alias_file = { type = "MuttAlias", value = "mutt_alias" }, vcard_folder = { type = "VCard", value = "vcard_folder" } }

"#
        );
        // 三个旧的账号顶层键都已消失：剩下的出现位置只有新 `contacts` 映射内
        // 作为后端名的那些。
        let stripped = migrated.replace(
            "contacts = { notmuch_address_book_query = { type = \"NotmuchAddress\", value = \
             \"notmuch\" }, mutt_alias_file = { type = \"MuttAlias\", value = \"mutt_alias\" }, \
             vcard_folder = { type = \"VCard\", value = \"vcard_folder\" } }",
            "contacts = {}",
        );
        for key in [
            "notmuch_address_book_query",
            "mutt_alias_file",
            "vcard_folder",
        ] {
            assert!(
                !stripped.contains(key),
                "old account key `{key}` must be removed; got:\n{migrated}"
            );
        }

        // 幂等：再跑一次不能改变文档。
        ContactBackendRefactor::transform(&mut doc, true).unwrap();
        assert_eq!(doc.to_string(), migrated);

        // revert 恢复原始文档。
        ContactBackendRefactor::revert_transform(&mut doc, true).unwrap();
        assert_eq!(doc.to_string(), input);
    }

    /// 真实文件上的端到端迁移：旧键被搬走，同目录的 `.bkp` 文件保留原始内容，
    /// 重复执行迁移时配置文件与备份集合都保持不变。
    #[test]
    fn test_contact_backend_refactor_perform_is_idempotent() {
        let tempdir = tempfile::tempdir().unwrap();
        let config = tempdir.path().join("meli.toml");
        let input = r#"
[accounts.imap]
root_mailbox = "INBOX"
format = "imap"
send_mail = 'false'
identity="username@example.com"
server_username = "null"
server_hostname = "example.com"
server_password = "false"
notmuch_address_book_query = "notmuch"
mutt_alias_file = "mutt_alias"
vcard_folder = "vcard_folder"
"#;
        std::fs::write(&config, input).unwrap();

        ContactBackendRefactor.perform(&config, false, true).unwrap();
        let migrated = std::fs::read_to_string(&config).unwrap();
        assert!(
            migrated.contains("contacts = { notmuch_address_book_query = { type = \
                               \"NotmuchAddress\", value = \"notmuch\" }"),
            "expected the `contacts` map to be generated; got:\n{migrated}"
        );
        let bkp = tempdir.path().join("meli.bkp");
        assert!(bkp.try_exists().unwrap(), "expected a backup file");
        assert_eq!(std::fs::read_to_string(&bkp).unwrap(), input);

        // 重跑：已无旧键，配置文件与备份集合都不变。
        ContactBackendRefactor.perform(&config, false, true).unwrap();
        assert_eq!(std::fs::read_to_string(&config).unwrap(), migrated);
        assert!(
            !tempdir.path().join("meli.bkp1").try_exists().unwrap(),
            "an idempotent re-run must not create another backup"
        );

        // revert 恢复账号原有的顶层旧键。
        ContactBackendRefactor.revert(&config, false, true).unwrap();
        assert_eq!(std::fs::read_to_string(&config).unwrap(), input);
    }
}
