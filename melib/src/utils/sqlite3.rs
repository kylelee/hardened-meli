/*
 * meli - melib
 *
 * Copyright 2020 Manos Pitsidianakis
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

use std::{
    borrow::Cow,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Arc,
};

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput};
pub use rusqlite::{self, config::DbConfig, params, Connection};

use crate::{error::*, tracing, Envelope};

/// A description for creating, opening and handling application databases.
#[derive(Clone, Debug)]
pub struct DatabaseDescription {
    /// A name that represents the function of this database, e.g.
    /// `headers_cache`, `contacts`, `settings`, etc.
    pub name: &'static str,
    /// An optional identifier string that along with
    /// [`DatabaseDescription::name`] makes a specialized identifier for the
    /// database. E.g. an account name, a date, etc.
    pub identifier: Option<Cow<'static, str>>,
    /// The name of the application to use when storing the database in `XDG`
    /// directories, used for when the consumer application is not `meli`
    /// itself.
    pub application_prefix: &'static str,
    /// Optionally override file system location instead of saving at `XDG` data
    /// directory.
    pub directory: Option<Cow<'static, Path>>,
    /// A script that initializes the schema of the database.
    pub init_script: Option<&'static str>,
    /// The current value of the `user_version` `PRAGMA` of the `sqlite3`
    /// database, used for schema versioning.
    pub version: u32,
}

/// Characters that may never appear in a [`DatabaseDescription`] component
/// that becomes part of a filesystem path.
///
/// Both `/` and `\` are directory separators on Windows, while
/// [`std::path::MAIN_SEPARATOR_STR`] is only `\`; a `/`-carrying identifier
/// could therefore still escape the configured `directory` through
/// [`PathBuf::join`] there. An interior NUL byte can never form a valid path
/// on any platform. Rejecting the whole set unconditionally keeps
/// [`DatabaseDescription::db_path`] fail-closed regardless of the host
/// separator (CVE-2018-14362 defense in depth, issue #156).
const FORBIDDEN_PATH_CHARS: [char; 3] = ['/', '\\', '\0'];

impl DatabaseDescription {
    /// Returns whether the computed database path for this description exist.
    pub fn exists(&self) -> Result<bool> {
        let path = self.db_path()?;
        Ok(path.exists())
    }

    /// Returns the computed database path for this description.
    pub fn db_path(&self) -> Result<PathBuf> {
        let name: Cow<'static, str> = self.identifier.as_ref().map_or_else(
            || self.name.into(),
            |id| format!("{}_{}", id, self.name).into(),
        );

        for (field_name, field_value) in [
            ("name", self.name),
            ("identifier", self.identifier.as_deref().unwrap_or_default()),
            ("application_prefix", self.application_prefix),
        ] {
            if field_value.contains(FORBIDDEN_PATH_CHARS) {
                return Err(Error::new(format!(
                    "Database description for `{}{}{}` field {} cannot contain path separators \
                     (`/`, `\\`) or NUL. Got: {}.",
                    self.identifier.as_deref().unwrap_or_default(),
                    if self.identifier.is_none() { "" } else { ":" },
                    self.name,
                    field_name,
                    field_value,
                ))
                .set_kind(ErrorKind::ValueError));
            }
        }

        if let Some(directory) = self.directory.as_deref() {
            if !directory.is_dir() {
                return Err(Error::new(format!(
                    "Database description for `{}{}{}` expects a valid directory path value. Got: \
                     {}.",
                    self.identifier.as_deref().unwrap_or_default(),
                    if self.identifier.is_none() { "" } else { ":" },
                    self.name,
                    directory.display()
                ))
                .set_kind(ErrorKind::ValueError));
            }
            return Ok(directory.join(name.as_ref()));
        }
        let data_dir = xdg::BaseDirectories::with_prefix(self.application_prefix);
        data_dir.place_data_file(name.as_ref()).map_err(|err| {
            Error::new(format!(
                "Could not create sqlite3 database file for `{}{}{}` in XDG data directory.",
                self.identifier.as_deref().unwrap_or_default(),
                if self.identifier.is_none() { "" } else { ":" },
                self.name,
            ))
            .set_kind(ErrorKind::Platform)
            .set_source(Some(Arc::new(err)))
        })
    }

    /// Returns an [`rusqlite::Connection`] for this description.
    pub fn open_or_create_db(&self) -> Result<Connection> {
        let mut second_try: bool = false;
        let db_path = self.db_path()?;
        let set_mode = !db_path.exists();
        if set_mode {
            tracing::info!("Creating {} database in {}", self.name, db_path.display());
        }
        loop {
            let mut inner_fn = || {
                let conn = Connection::open(&db_path)?;
                conn.busy_timeout(std::time::Duration::new(10, 0))?;
                for conf_flag in [
                    DbConfig::SQLITE_DBCONFIG_ENABLE_FKEY,
                    DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,
                ]
                .into_iter()
                {
                    conn.set_db_config(conf_flag, true)?;
                }
                rusqlite::vtab::array::load_module(&conn)?;
                if set_mode {
                    let file = std::fs::File::open(&db_path)?;
                    let metadata = file.metadata()?;
                    let mut permissions = metadata.permissions();

                    permissions.set_mode(0o600); // Read/write for owner only.
                    file.set_permissions(permissions)?;
                }
                let _: String =
                    conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))?;
                let version: i32 =
                    conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
                if version != 0_i32 && version as u32 != self.version {
                    tracing::info!(
                        "Database version mismatch, is {} but expected {}. Attempting to recreate \
                         database.",
                        version,
                        self.version
                    );
                    if second_try {
                        return Err(Error::new(format!(
                            "Database version mismatch, is {} but expected {}. Could not recreate \
                             database.",
                            version, self.version
                        )));
                    }
                    self.reset_db()?;
                    second_try = true;
                    return Ok(None);
                }

                if version == 0 {
                    conn.pragma_update(None, "user_version", self.version)?;
                }
                if let Some(s) = self.init_script {
                    conn.execute_batch(s)
                        .map_err(|err| Error::new(err.to_string()))?;
                }

                Ok(Some(conn))
            };
            match inner_fn() {
                Ok(None) => continue,
                Ok(Some(conn)) => return Ok(conn),
                Err(err) => {
                    return Err(Error::new(format!(
                        "{}: Could not open or create database",
                        db_path.display()
                    ))
                    .set_source(Some(Arc::new(err))))
                }
            }
        }
    }

    /// Reset database to a clean slate.
    pub fn reset_db(&self) -> Result<()> {
        let db_path = self.db_path()?;
        if !db_path.exists() {
            return Ok(());
        }
        tracing::info!("Resetting {} database in {}", self.name, db_path.display());
        std::fs::remove_file(&db_path).map_err(|err| {
            Error::new(format!("{}: could not remove file", db_path.display()))
                .set_kind(ErrorKind::from(err.kind()))
                .set_source(Some(Arc::new(err)))
        })?;
        tracing::info!(
            "{} {} database reset successful",
            self.name,
            db_path.display()
        );
        Ok(())
    }
}

impl ToSql for Envelope {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        let v: Vec<u8> = serde_json::to_vec(self).map_err(|e| {
            rusqlite::Error::ToSqlConversionFailure(Box::new(Error::new(e.to_string())))
        })?;
        Ok(ToSqlOutput::from(v))
    }
}

impl FromSql for Envelope {
    fn column_result(value: rusqlite::types::ValueRef) -> FromSqlResult<Self> {
        let b: Vec<u8> = FromSql::column_result(value)?;

        serde_json::from_slice(&b).map_err(|e| FromSqlError::Other(Box::new(e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CVE-2018-14362 (issue #156): `db_path` is the one place a
    /// `DatabaseDescription` string becomes a filesystem path, so every
    /// path separator (`/` and `\` on every platform) and NUL must be
    /// rejected unconditionally, with no file created.
    #[test]
    fn test_db_path_rejects_path_separators_and_nul() {
        fn desc(
            name: &'static str,
            identifier: Option<Cow<'static, str>>,
            application_prefix: &'static str,
            directory: &Path,
        ) -> DatabaseDescription {
            DatabaseDescription {
                name,
                identifier,
                application_prefix,
                directory: Some(directory.to_path_buf().into()),
                init_script: None,
                version: 1,
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let legal = desc(
            "header_cache.db",
            Some("cve-14362".into()),
            "meli",
            dir.path(),
        );
        assert_eq!(
            legal.db_path().unwrap(),
            dir.path().join("cve-14362_header_cache.db"),
            "honest descriptions must still compute their path"
        );

        let cases = [
            ("name", desc("../evil", None, "meli", dir.path())),
            ("name", desc("a/b", None, "meli", dir.path())),
            ("name", desc("a\\b", None, "meli", dir.path())),
            ("name", desc("a\0b", None, "meli", dir.path())),
            (
                "identifier",
                desc(
                    "header_cache.db",
                    Some("../evil".into()),
                    "meli",
                    dir.path(),
                ),
            ),
            (
                "identifier",
                desc("header_cache.db", Some("a/b".into()), "meli", dir.path()),
            ),
            (
                "identifier",
                desc("header_cache.db", Some("a\\b".into()), "meli", dir.path()),
            ),
            (
                "identifier",
                desc("header_cache.db", Some("a\0b".into()), "meli", dir.path()),
            ),
            (
                "application_prefix",
                desc("header_cache.db", None, "../evil", dir.path()),
            ),
            (
                "application_prefix",
                desc("header_cache.db", None, "a/b", dir.path()),
            ),
            (
                "application_prefix",
                desc("header_cache.db", None, "a\\b", dir.path()),
            ),
            (
                "application_prefix",
                desc("header_cache.db", None, "a\0b", dir.path()),
            ),
        ];
        for (field, d) in cases {
            let err = d
                .db_path()
                .expect_err(&format!("{field}: injected description must be rejected"));
            assert_eq!(err.kind, ErrorKind::ValueError, "{field}: wrong error kind");
        }

        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "rejected descriptions must not create any file"
        );
    }

    #[test]
    fn test_open_or_create_db_runs_init_script_exactly_once() {
        const INIT_SCRIPT: &str =
            "CREATE TABLE IF NOT EXISTS x (id INTEGER); INSERT INTO x VALUES (42);";
        let dir = tempfile::tempdir().unwrap();
        let db = DatabaseDescription {
            name: "test_init_once",
            identifier: None,
            application_prefix: "meli_test",
            directory: Some(dir.path().to_path_buf().into()),
            init_script: Some(INIT_SCRIPT),
            version: 1,
        };
        let conn = db.open_or_create_db().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM x", params![], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1, "init_script must execute exactly once");
    }

    #[test]
    fn test_open_or_create_db_error_is_graceful() {
        let dir = tempfile::tempdir().unwrap();
        // A directory at the database path makes `Connection::open` fail
        // without any prior side effect; the error must surface as `Err`,
        // not a panic, and the path must not be replaced by a file.
        std::fs::create_dir(dir.path().join("test_err_db")).unwrap();
        let db = DatabaseDescription {
            name: "test_err_db",
            identifier: None,
            application_prefix: "meli_test",
            directory: Some(dir.path().to_path_buf().into()),
            init_script: None,
            version: 1,
        };
        let err = db.open_or_create_db().unwrap_err();
        assert!(
            err.to_string()
                .contains("Could not open or create database"),
            "expected graceful open error, got: {err}"
        );
        assert!(dir.path().join("test_err_db").is_dir());
    }
}
