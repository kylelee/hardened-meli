/*
 * meli - nntp module.
 *
 * Copyright 2023 Manos Pitsidianakis
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

//! Store article seen/read flags in an sqlite3 database, since NNTP has no
//! concept of server-side flag bookkeeping.

pub use inner::*;

#[cfg(feature = "sqlite3")]
mod inner {
    use crate::{
        email::Flag,
        nntp::UID,
        utils::sqlite3::{self, Connection, DatabaseDescription},
        EnvelopeHash, MailboxHash, Result,
    };

    pub const DB_DESCRIPTION: DatabaseDescription = DatabaseDescription {
        name: "nntp_store.db",
        application_prefix: "meli",
        identifier: None,
        directory: None,
        init_script: Some(
            "PRAGMA foreign_keys = true;
PRAGMA encoding = 'UTF-8';

CREATE TABLE IF NOT EXISTS article (
    hash             INTEGER NOT NULL,
    mailbox_hash     INTEGER NOT NULL,
    uid              INTEGER NOT NULL,
    flags            INTEGER NOT NULL DEFAULT 0,
    tags             TEXT,
    PRIMARY KEY (mailbox_hash, uid)
);
    CREATE INDEX IF NOT EXISTS article_uid_idx ON article(mailbox_hash, uid);
    CREATE INDEX IF NOT EXISTS article_idx ON article(hash);",
        ),
        version: 1,
    };

    #[derive(Debug)]
    pub struct Store {
        pub connection: Connection,
    }

    impl Store {
        pub fn new(id: &str) -> Result<Self> {
            let db_desc = DatabaseDescription {
                identifier: Some(id.to_string().into()),
                ..DB_DESCRIPTION
            };
            Ok(Self {
                connection: db_desc.open_or_create_db()?,
            })
        }

        pub fn set_flags(
            &self,
            envelope_hash: EnvelopeHash,
            mailbox_hash: MailboxHash,
            uid: UID,
            new_value: Flag,
        ) -> Result<()> {
            self.connection.execute(
                "INSERT OR REPLACE INTO article(hash, mailbox_hash, uid, flags) VALUES (?, ?, ?, \
                 ?)",
                sqlite3::params![&envelope_hash, &mailbox_hash, uid.get(), &new_value.bits()],
            )?;

            Ok(())
        }

        pub fn flags(
            &self,
            envelope_hash: EnvelopeHash,
            mailbox_hash: MailboxHash,
            uid: UID,
        ) -> Result<Flag> {
            self.connection.execute(
                "INSERT INTO article(hash,mailbox_hash,uid) VALUES(?1,?2,?3) ON \
                 CONFLICT(mailbox_hash, uid) DO UPDATE SET hash = ?1",
                sqlite3::params![&envelope_hash, &mailbox_hash, uid.get()],
            )?;
            let mut stmt = self
                .connection
                .prepare("SELECT flags FROM article WHERE mailbox_hash = ?1 AND uid = ?2;")?;
            Ok(Flag::from_bits({
                stmt.query_row(sqlite3::params![&mailbox_hash, uid.get()], |row| {
                    let flag: u8 = row.get(0)?;
                    Ok(flag)
                })?
            })
            .unwrap_or_default())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn in_memory_store() -> Store {
            let connection = Connection::open_in_memory().unwrap();
            connection
                .execute_batch(DB_DESCRIPTION.init_script.unwrap())
                .unwrap();
            Store { connection }
        }

        /// Regression: when an article's message-id changes, its
        /// `EnvelopeHash` changes with it. The uid store must keep exactly
        /// one row per `(mailbox_hash, uid)` and always point that row's
        /// `hash` at the newest value, instead of silently ignoring the
        /// insert and then failing to find the flags row keyed by the old
        /// hash.
        #[test]
        fn test_flags_upsert_keeps_single_row_and_updates_hash() {
            let store = in_memory_store();
            let mailbox_hash = MailboxHash(1);
            let uid = UID::new(7).unwrap();
            let hash_a = EnvelopeHash(100);
            let hash_b = EnvelopeHash(200);

            assert_eq!(
                store.flags(hash_a, mailbox_hash, uid).unwrap(),
                Flag::default()
            );
            assert_eq!(
                store.flags(hash_b, mailbox_hash, uid).unwrap(),
                Flag::default()
            );

            let count: i64 = store
                .connection
                .query_row(
                    "SELECT COUNT(*) FROM article WHERE mailbox_hash = ?1 AND uid = ?2;",
                    sqlite3::params![mailbox_hash, uid.get()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "same (mailbox_hash, uid) must map to one row");

            let stored_hash: i64 = store
                .connection
                .query_row(
                    "SELECT hash FROM article WHERE mailbox_hash = ?1 AND uid = ?2;",
                    sqlite3::params![mailbox_hash, uid.get()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(stored_hash, hash_b.0 as i64, "hash must track newest");

            let stale: i64 = store
                .connection
                .query_row(
                    "SELECT COUNT(*) FROM article WHERE hash = ?1;",
                    sqlite3::params![hash_a],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(stale, 0, "old hash must no longer match any row");
        }

        /// Flags set on the old hash must stay readable after the hash
        /// changes, because the flags lookup is keyed by
        /// `(mailbox_hash, uid)` alone.
        #[test]
        fn test_flags_survive_hash_change() {
            let store = in_memory_store();
            let mailbox_hash = MailboxHash(2);
            let uid = UID::new(9).unwrap();
            let hash_a = EnvelopeHash(300);
            let hash_b = EnvelopeHash(400);

            store.flags(hash_a, mailbox_hash, uid).unwrap();
            store
                .set_flags(hash_a, mailbox_hash, uid, Flag::SEEN)
                .unwrap();

            assert_eq!(
                store.flags(hash_b, mailbox_hash, uid).unwrap(),
                Flag::SEEN
            );

            let count: i64 = store
                .connection
                .query_row(
                    "SELECT COUNT(*) FROM article WHERE mailbox_hash = ?1 AND uid = ?2;",
                    sqlite3::params![mailbox_hash, uid.get()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "hash change must not duplicate rows");
        }
    }
}

#[cfg(not(feature = "sqlite3"))]
mod inner {
    use crate::{email::Flag, nntp::UID, EnvelopeHash, Error, ErrorKind, MailboxHash, Result};

    #[derive(Debug)]
    pub struct Store;

    impl Store {
        pub fn new(_: &str) -> Result<Self> {
            Ok(Self)
        }

        pub fn set_flags(&self, _: EnvelopeHash, _: MailboxHash, _: UID, _: Flag) -> Result<()> {
            Ok(())
        }

        pub fn flags(&self, _: EnvelopeHash, _: MailboxHash, _: UID) -> Result<Flag> {
            Err(Error::new(
                "NNTP store flag cache accessed but this copy of melib isn't built with sqlite3 \
                 support. This is a bug and should be reported.",
            )
            .set_kind(ErrorKind::Bug))
        }
    }
}
