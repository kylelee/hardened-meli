//
// melib - IMAP
//
// Copyright 2024 Emmanouil Pitsidianakis <manos@pitsidianak.is>
// Copyright 2026 Kyle Lee
//
// This file is part of melib.
//
// melib is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// melib is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with melib. If not, see <http://www.gnu.org/licenses/>.
//
// SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{
    backends::{EnvelopeHashBatch, FlagOp, MailboxHash, RefreshEvent, RefreshEventKind, TagHash},
    email::{headers::HeaderName, Address, Envelope, EnvelopeHash},
    error::{Error, ErrorKind, Result, ResultIntoError},
    imap::{
        mailbox::ImapMailbox,
        sync::cache::{
            CachedEnvelope, CachedImapMailbox, CachedState, CachedStatus, ImapCache, ImapCacheReset,
        },
        FetchResponse, ModSequence, SelectResponse, UIDStore, UID, UIDVALIDITY,
    },
    utils::datetime::timestamp_to_string_utc,
    utils::sqlite3::{
        self,
        rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, Value},
        Connection, DatabaseDescription,
    },
};

type Sqlite3UID = i32;

/// Display name of the placeholder `From` address served for quarantined
/// cached envelopes.
const QUARANTINE_PLACEHOLDER_FROM_DISPLAY: &str = "[meli] cached envelope decode error";
/// Address spec of the placeholder `From` address; the `.invalid` TLD is
/// reserved by RFC 2606 and can never resolve.
const QUARANTINE_PLACEHOLDER_FROM_SPEC: &str = "invalid@invalid.invalid";

/// Build the placeholder [`Envelope`] served for a row quarantined in the
/// `invalid_envelopes` table.
///
/// The placeholder keeps the *stored* `hash` column value as its identity,
/// so a later server refetch of the same message computes the same hash and
/// replaces the placeholder seamlessly. The `From` address is a
/// round-trip-safe literal (the cache serializes addresses via their
/// display form and strictly re-parses them on load). The conversion error
/// and the size of the preserved raw blob are stashed into pseudo-headers
/// (`X-Meli-Cache-Error`, `X-Meli-Cache-Raw-Bytes`) for inspectability.
fn quarantined_envelope_placeholder(
    uid: UID,
    hash: EnvelopeHash,
    error: &str,
    raw_len: Option<i64>,
    first_seen: i64,
) -> Envelope {
    let mut env = Envelope::default();
    env.set_hash(hash);
    env.set_subject(
        format!("[meli] 缓存邮件无法解码 / undecodable cached message (uid {uid})").into_bytes(),
    );
    env.set_from(
        vec![Address::new(
            Some(QUARANTINE_PLACEHOLDER_FROM_DISPLAY),
            QUARANTINE_PLACEHOLDER_FROM_SPEC,
        )]
        .into(),
    );
    let first_seen = u64::try_from(first_seen).unwrap_or(0);
    env.set_datetime(first_seen);
    env.set_date(timestamp_to_string_utc(first_seen, None, true).as_bytes());
    let error_header = HeaderName::from_bytes(b"X-Meli-Cache-Error")
        .expect("constant X-Meli-Cache-Error is a valid header name");
    env.other_headers_mut()
        .insert(error_header, error.to_string());
    if let Some(raw_len) = raw_len {
        let raw_header = HeaderName::from_bytes(b"X-Meli-Cache-Raw-Bytes")
            .expect("constant X-Meli-Cache-Raw-Bytes is a valid header name");
        env.other_headers_mut()
            .insert(raw_header, raw_len.to_string());
    }
    env
}

#[derive(Debug)]
pub struct Sqlite3Cache {
    connection: Connection,
    loaded_mailboxes: BTreeMap<MailboxHash, CachedState>,
    uid_store: Arc<UIDStore>,
    data_dir: Option<PathBuf>,
}

const DB_DESCRIPTION: DatabaseDescription = DatabaseDescription {
    name: "header_cache.db",
    identifier: None,
    application_prefix: "meli",
    directory: None,
    init_script: Some(
        "PRAGMA foreign_keys = true;
    PRAGMA encoding = 'UTF-8';

    CREATE TABLE IF NOT EXISTS envelopes (
                    hash             INTEGER NOT NULL,
                    mailbox_hash     INTEGER NOT NULL,
                    uid              INTEGER NOT NULL,
                    modsequence      INTEGER,
                    envelope         BLOB NOT NULL,
                    PRIMARY KEY (mailbox_hash, uid),
                    FOREIGN KEY (mailbox_hash) REFERENCES mailbox(mailbox_hash) ON DELETE CASCADE
                   );
    CREATE TABLE IF NOT EXISTS mailbox (
                mailbox_hash     INTEGER UNIQUE,
                uidvalidity      INTEGER,
                max_uid          INTEGER,
                flags            BLOB NOT NULL,
                highestmodseq    INTEGER,
                messages         INTEGER,
                unseen           INTEGER,
                uidnext          INTEGER,
                PRIMARY KEY (mailbox_hash)
               );
    CREATE TABLE IF NOT EXISTS msn_cache (
                mailbox_hash     INTEGER NOT NULL,
                uidvalidity      INTEGER NOT NULL,
                msn              INTEGER NOT NULL,
                uid              INTEGER NOT NULL,
                PRIMARY KEY (mailbox_hash, msn)
               ) WITHOUT ROWID;
    CREATE TABLE IF NOT EXISTS invalid_envelopes (
                mailbox_hash     INTEGER NOT NULL,
                uid              INTEGER NOT NULL,
                hash             INTEGER NOT NULL,
                error            TEXT NOT NULL,
                raw              BLOB,
                first_seen       INTEGER NOT NULL,
                PRIMARY KEY (mailbox_hash, uid)
               ) WITHOUT ROWID;
    CREATE TABLE IF NOT EXISTS mailbox_list (id INTEGER PRIMARY KEY CHECK (id = 0), payload BLOB NOT NULL);
    CREATE INDEX IF NOT EXISTS envelope_uid_idx ON envelopes(mailbox_hash, uid ASC);
    CREATE INDEX IF NOT EXISTS envelope_idx ON envelopes(hash);
    CREATE INDEX IF NOT EXISTS mailbox_idx ON mailbox(mailbox_hash);",
    ),
    version: 5,
};

impl From<EnvelopeHash> for Value {
    fn from(env_hash: EnvelopeHash) -> Self {
        (env_hash.0 as i64).into()
    }
}

impl ToSql for ModSequence {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.0.get() as i64))
    }
}

impl FromSql for ModSequence {
    fn column_result(value: rusqlite::types::ValueRef) -> FromSqlResult<Self> {
        let i: i64 = FromSql::column_result(value)?;
        if i == 0 {
            return Err(FromSqlError::OutOfRange(0));
        }
        Ok(Self::try_from(i).unwrap())
    }
}

impl Sqlite3Cache {
    pub fn get(uid_store: Arc<UIDStore>, data_dir: Option<&Path>) -> Result<Box<dyn ImapCache>> {
        let data_dir = data_dir.map(|p| p.to_path_buf());
        let db_desc = DatabaseDescription {
            identifier: Some(uid_store.account_name.to_string().into()),
            directory: data_dir.clone().map(|p| p.into()),
            ..DB_DESCRIPTION.clone()
        };
        let connection = match db_desc.open_or_create_db() {
            Ok(c) => Ok(c),
            Err(err) => {
                // try resetting database on error, but only one time.
                if db_desc.reset_db().is_ok() {
                    db_desc.open_or_create_db()
                } else {
                    Err(err)
                }
            }
        }?;
        Self::migrate(&connection)?;

        Ok(Box::new(Self {
            connection,
            loaded_mailboxes: BTreeMap::default(),
            uid_store,
            data_dir,
        }))
    }

    /// Adds the `messages`, `unseen` and `uidnext` columns to the `mailbox`
    /// table if they don't exist yet (databases created before these columns
    /// were introduced). Strictly additive: existing rows/columns are never
    /// dropped, renamed or modified.
    fn migrate(connection: &Connection) -> Result<()> {
        const STATUS_COLUMNS: [&str; 3] = ["messages", "unseen", "uidnext"];
        let mut existing_columns = BTreeSet::new();
        {
            let mut stmt = connection.prepare("PRAGMA table_info(mailbox);")?;
            let column_names = stmt
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            existing_columns.extend(column_names);
        }
        for column in STATUS_COLUMNS {
            if !existing_columns.contains(column) {
                connection
                    .execute_batch(&format!("ALTER TABLE mailbox ADD COLUMN {column} INTEGER;"))?;
            }
        }
        Ok(())
    }
}

impl ImapCacheReset for Sqlite3Cache {
    fn reset_db(uid_store: &UIDStore, data_dir: Option<&Path>) -> Result<()> {
        let db_desc = DatabaseDescription {
            identifier: Some(uid_store.account_name.to_string().into()),
            directory: data_dir.map(|p| p.to_path_buf().into()),
            ..DB_DESCRIPTION.clone()
        };
        db_desc.reset_db()
    }
}

impl ImapCache for Sqlite3Cache {
    fn reset(&mut self) -> Result<()> {
        Self::reset_db(&self.uid_store, self.data_dir.as_deref())
    }

    fn lastseenuid(&mut self, mailbox_hash: MailboxHash) -> Result<Option<UID>> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let env_lastseenuid: Option<UID> = {
            let mut stmt = tx.prepare("SELECT MAX(uid) FROM envelopes WHERE mailbox_hash = ?1;")?;

            let ret: Option<UID> = stmt.query_row(sqlite3::params![mailbox_hash], |row| {
                row.get(0).map(|i: Option<Sqlite3UID>| i.map(|i| i as UID))
            })?;
            drop(stmt);
            ret
        };
        let mut lastseenuid = {
            let mut stmt = tx.prepare("SELECT max_uid FROM mailbox WHERE mailbox_hash = ?1;")?;

            let ret: Option<UID> = stmt.query_row(sqlite3::params![mailbox_hash], |row| {
                row.get(0).map(|i: Option<Sqlite3UID>| i.map(|i| i as UID))
            })?;
            drop(stmt);
            ret
        };
        if let (Some(env_lastseenuid), true) = (env_lastseenuid, lastseenuid != env_lastseenuid) {
            lastseenuid = Some(env_lastseenuid);
            tx.execute(
                "UPDATE mailbox SET max_uid=?1 where mailbox_hash = ?2;",
                sqlite3::params![env_lastseenuid, mailbox_hash],
            )?;
            tx.commit()?;
        }
        if let Some(lastseenuid) = lastseenuid {
            self.uid_store
                .lastseenuid
                .lock()
                .unwrap()
                .insert(mailbox_hash, lastseenuid);
        }
        Ok(lastseenuid)
    }

    fn has_envelopes(&mut self, mailbox_hash: MailboxHash) -> Result<bool> {
        let mut stmt = self
            .connection
            .prepare("SELECT EXISTS(SELECT 1 FROM envelopes WHERE mailbox_hash = ?1);")?;
        let exists: bool = stmt.query_row(sqlite3::params![mailbox_hash], |row| row.get(0))?;
        Ok(exists)
    }

    fn count_envelopes(&mut self, mailbox_hash: MailboxHash) -> Result<Option<usize>> {
        let mut stmt = self
            .connection
            .prepare("SELECT COUNT(*) FROM envelopes WHERE mailbox_hash = ?1;")?;
        let count: usize = stmt.query_row(sqlite3::params![mailbox_hash], |row| row.get(0))?;
        Ok(Some(count))
    }

    fn record_status(
        &mut self,
        mailbox_hash: MailboxHash,
        messages: Option<UID>,
        unseen: Option<UID>,
        uidnext: Option<UID>,
    ) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE mailbox SET messages = ?1, unseen = ?2, uidnext = ?3 WHERE mailbox_hash = ?4;",
            sqlite3::params![
                messages.map(|i| i as Sqlite3UID),
                unseen.map(|i| i as Sqlite3UID),
                uidnext.map(|i| i as Sqlite3UID),
                mailbox_hash
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn cached_status(&mut self, mailbox_hash: MailboxHash) -> Result<Option<CachedStatus>> {
        let mut stmt = self
            .connection
            .prepare("SELECT messages, unseen, uidnext FROM mailbox WHERE mailbox_hash = ?1;")?;
        let mut ret = stmt.query_map(sqlite3::params![mailbox_hash], |row| {
            Ok((
                row.get(0)
                    .map(|i: Option<Sqlite3UID>| i.map(|i| i as UID))?,
                row.get(1)
                    .map(|i: Option<Sqlite3UID>| i.map(|i| i as UID))?,
                row.get(2)
                    .map(|i: Option<Sqlite3UID>| i.map(|i| i as UID))?,
            ))
        })?;
        let Some(v) = ret.next() else {
            return Ok(None);
        };
        let status = v?;
        // All-`NULL` status columns (e.g. a mailbox row from a database that
        // predates these columns) mean no baseline was ever recorded.
        Ok(match status {
            (None, None, None) => None,
            status => Some(status),
        })
    }

    fn load_msn_index(
        &mut self,
        mailbox_hash: MailboxHash,
        uidvalidity: UIDVALIDITY,
    ) -> Result<Option<Vec<Option<UID>>>> {
        let mut stmt = self.connection.prepare(
            "SELECT msn, uid, uidvalidity FROM msn_cache WHERE mailbox_hash = ?1 ORDER BY msn \
             ASC;",
        )?;
        let rows = stmt
            .query_map(sqlite3::params![mailbox_hash], |row| {
                Ok((
                    row.get::<_, Sqlite3UID>(0)?,
                    row.get::<_, Sqlite3UID>(1)?,
                    row.get::<_, Sqlite3UID>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let Some(&(_, _, stored_uidvalidity)) = rows.first() else {
            return Ok(None);
        };
        // Rows of a stale `UIDVALIDITY` must not be reused; the caller
        // rebuilds the index with a `UID SEARCH`.
        if stored_uidvalidity as UIDVALIDITY != uidvalidity {
            return Ok(None);
        }
        // `msn` is stored 1-based; the returned vector is indexed 0-based,
        // with `None` for missing message sequence numbers.
        //
        // Bound on the dense index built here: `msn` is an `i32` read from a
        // (possibly corrupted) local cache, so a single forged row with
        // `msn = 2_000_000_000` would otherwise try to allocate ~32 GiB.
        // Rows past the bound are ignored, which is a safe degradation on an
        // already-corrupt cache.
        const MAX_MSN_INDEX_LEN: usize = 5_000_000;
        let len = rows
            .last()
            .and_then(|&(msn, _, _)| usize::try_from(msn).ok())
            .unwrap_or(0)
            .min(MAX_MSN_INDEX_LEN);
        let mut ret = vec![None; len];
        for &(msn, uid, _) in &rows {
            if let Some(slot) = msn
                .checked_sub(1)
                .and_then(|msn| usize::try_from(msn).ok())
                .and_then(|msn| ret.get_mut(msn))
            {
                *slot = Some(uid as UID);
            }
        }
        Ok(Some(ret))
    }

    fn store_msn_index(
        &mut self,
        mailbox_hash: MailboxHash,
        uidvalidity: UIDVALIDITY,
        msn_index: &[Option<UID>],
    ) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM msn_cache WHERE mailbox_hash = ?1;",
            sqlite3::params![mailbox_hash],
        )?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO msn_cache (mailbox_hash, uidvalidity, msn, uid) VALUES (?1, ?2, ?3, \
                 ?4);",
            )?;
            for (i, uid) in msn_index.iter().enumerate() {
                let Some(uid) = uid else {
                    continue;
                };
                stmt.execute(sqlite3::params![
                    mailbox_hash,
                    uidvalidity as Sqlite3UID,
                    (i + 1) as Sqlite3UID,
                    *uid as Sqlite3UID
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn mailbox_state(&mut self, mailbox_hash: MailboxHash) -> Result<Option<CachedState>> {
        if let Some(s) = self.loaded_mailboxes.get(&mailbox_hash) {
            return Ok(Some(*s));
        }
        let tx = self.connection.transaction()?;
        let mut stmt = tx.prepare(
            "SELECT uidvalidity, max_uid, flags, highestmodseq FROM mailbox WHERE mailbox_hash = \
             ?1;",
        )?;

        let mut ret = stmt.query_map(sqlite3::params![mailbox_hash], |row| {
            Ok((
                row.get(0).map(|u: Sqlite3UID| u as UID)?,
                row.get(1)
                    .map(|u: Option<Sqlite3UID>| u.map(|u| u as UID))?,
                row.get(2)?,
                row.get(3)?,
            ))
        })?;
        if let Some(v) = ret.next() {
            let (uidvalidity, lastseenuid, flags, highestmodseq): (
                UIDVALIDITY,
                Option<UID>,
                Vec<u8>,
                Option<ModSequence>,
            ) = v?;
            drop(ret);
            drop(stmt);
            self.uid_store
                .highestmodseqs
                .lock()
                .unwrap()
                .entry(mailbox_hash)
                .and_modify(|entry| *entry = highestmodseq.ok_or(()))
                .or_insert_with(|| highestmodseq.ok_or(()));
            self.uid_store
                .uidvalidity
                .lock()
                .unwrap()
                .entry(mailbox_hash)
                .and_modify(|entry| *entry = uidvalidity)
                .or_insert(uidvalidity);
            let mut tag_lck = self.uid_store.collection.tag_index.write().unwrap();
            // The flags blob comes from the local (possibly corrupted) cache.
            // Validate it as UTF-8 instead of using `from_utf8_unchecked`,
            // which would be UB on arbitrary bytes.
            let flags_str = std::str::from_utf8(&flags).map_err(|_| {
                Error::new("Cached mailbox flags are not valid UTF-8")
                    .set_kind(ErrorKind::ProtocolError)
            })?;
            for f in flags_str.split('\0') {
                let hash = TagHash::from_bytes(f.as_bytes());
                tag_lck.entry(hash).or_insert_with(|| f.to_string());
            }
            if let Some(lastseenuid) = lastseenuid {
                self.uid_store
                    .lastseenuid
                    .lock()
                    .unwrap()
                    .insert(mailbox_hash, lastseenuid);
            };
            let retval = CachedState {
                highestmodseq,
                uidvalidity,
            };
            self.loaded_mailboxes.insert(mailbox_hash, retval);
            Ok(Some(retval))
        } else {
            Ok(None)
        }
    }

    fn load_mailbox_list(&mut self) -> Result<Option<HashMap<MailboxHash, ImapMailbox>>> {
        let mut stmt = self
            .connection
            .prepare("SELECT payload FROM mailbox_list WHERE id = 0;")?;
        let mut rows = stmt.query([])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let payload: Vec<u8> = row.get(0)?;
        // A corrupted blob surfaces as an error; the caller decides the
        // degradation.
        let mailboxes: Vec<CachedImapMailbox> = serde_json::from_slice(&payload)?;
        Ok(Some(
            mailboxes
                .into_iter()
                .map(|mailbox| (mailbox.hash, ImapMailbox::from(mailbox)))
                .collect(),
        ))
    }

    fn save_mailbox_list(&mut self, mailboxes: &HashMap<MailboxHash, ImapMailbox>) -> Result<()> {
        let payload: Vec<u8> = serde_json::to_vec(
            &mailboxes
                .values()
                .map(CachedImapMailbox::from)
                .collect::<Vec<CachedImapMailbox>>(),
        )?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT OR REPLACE INTO mailbox_list (id, payload) VALUES (0, ?1);",
            sqlite3::params![payload],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn init_mailbox(
        &mut self,
        mailbox_hash: MailboxHash,
        select_response: &SelectResponse,
    ) -> Result<()> {
        self.loaded_mailboxes.remove(&mailbox_hash);
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM mailbox WHERE mailbox_hash = ?1",
            sqlite3::params![mailbox_hash],
        )
        .chain_err_summary(|| {
            format!(
                "Could not clear cache of mailbox {} account {}",
                mailbox_hash, self.uid_store.account_name
            )
        })?;
        // A full mailbox rebuild (e.g. after a `UIDVALIDITY` change) ends
        // the quarantine of this mailbox: placeholder rows must not
        // outlive the cache generation they were recorded in. The table
        // carries no foreign key on purpose (mirroring `msn_cache`), so
        // this cleanup is explicit.
        tx.execute(
            "DELETE FROM invalid_envelopes WHERE mailbox_hash = ?1",
            sqlite3::params![mailbox_hash],
        )
        .chain_err_summary(|| {
            format!(
                "Could not clear quarantined envelopes of mailbox {} account {}",
                mailbox_hash, self.uid_store.account_name
            )
        })?;

        let highestmodseq: Option<ModSequence> =
            select_response.highestmodseq.transpose().unwrap_or(None);
        tx.execute(
            "INSERT OR IGNORE INTO mailbox (uidvalidity, flags, highestmodseq, mailbox_hash) \
             VALUES (?1, ?2, ?3, ?4)",
            sqlite3::params![
                select_response.uidvalidity as Sqlite3UID,
                select_response
                    .flags
                    .1
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<&str>>()
                    .join("\0")
                    .as_bytes(),
                highestmodseq,
                mailbox_hash
            ],
        )
        .chain_err_summary(|| {
            format!(
                "Could not insert uidvalidity {} in header_cache of account {}",
                select_response.uidvalidity, self.uid_store.account_name
            )
        })?;
        tx.commit()?;
        let val = CachedState {
            highestmodseq,
            uidvalidity: select_response.uidvalidity,
        };
        self.loaded_mailboxes.insert(mailbox_hash, val);
        Ok(())
    }

    fn update_mailbox(
        &mut self,
        mailbox_hash: MailboxHash,
        select_response: &SelectResponse,
    ) -> Result<()> {
        if self.mailbox_state(mailbox_hash)?.is_none() {
            return Err(Error::new("Mailbox is not in cache").set_kind(ErrorKind::NotFound));
        }

        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let highestmodseq: Option<ModSequence> =
            select_response.highestmodseq.transpose().unwrap_or(None);
        tx.execute(
            "UPDATE mailbox SET flags = ?1, highestmodseq = ?2, uidvalidity = ?3 where \
             mailbox_hash = ?4;",
            sqlite3::params![
                select_response
                    .flags
                    .1
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<&str>>()
                    .join("\0")
                    .as_bytes(),
                highestmodseq,
                select_response.uidvalidity,
                mailbox_hash,
            ],
        )
        .chain_err_summary(|| {
            format!(
                "Could not update mailbox {} in header_cache of account {}",
                mailbox_hash, self.uid_store.account_name
            )
        })?;
        tx.commit()?;
        let val = CachedState {
            highestmodseq,
            uidvalidity: select_response.uidvalidity,
        };
        self.loaded_mailboxes.insert(mailbox_hash, val);
        Ok(())
    }

    fn envelopes(
        &mut self,
        mailbox_hash: MailboxHash,
        lastseenuid: UID,
        batch_size: usize,
    ) -> Result<Option<(Vec<EnvelopeHash>, Option<UID>)>> {
        if self.mailbox_state(mailbox_hash)?.is_none() {
            return Ok(None);
        }

        let max = lastseenuid;
        let (ret, quarantined) = {
            let mut stmt = self.connection.prepare(
                "SELECT uid, hash, envelope, modsequence FROM envelopes WHERE mailbox_hash = \
                 ?1 AND uid <= ?2 ORDER BY uid DESC LIMIT ?3;",
            )?;
            let mut rows = stmt.query(sqlite3::params![
                mailbox_hash,
                max,
                batch_size as Sqlite3UID
            ])?;
            let mut ret: Vec<(UID, EnvelopeHash, Envelope, Option<ModSequence>)> = Vec::new();
            // A row whose envelope blob fails conversion (e.g. an address
            // that the lenient network ENVELOPE parser accepted but strict
            // deserialization rejects) must not render the whole cache
            // unusable; it is collected here and quarantined below so that
            // it stays visible (as a placeholder) and inspectable instead
            // of the entire cache being reset.
            let mut quarantined: Vec<(UID, EnvelopeHash, String, Vec<u8>)> = Vec::new();
            while let Some(row) = rows.next()? {
                let uid: UID = row.get(0)?;
                let hash: EnvelopeHash = row.get(1)?;
                let modsequence: Option<ModSequence> = row.get(3)?;
                match row.get::<_, Envelope>(2) {
                    Ok(envelope) => ret.push((uid, hash, envelope, modsequence)),
                    Err(rusqlite::Error::FromSqlConversionFailure(_, _, ref err)) => {
                        // The blob itself is still readable even though the
                        // `Envelope` conversion failed; keep the original
                        // bytes as forensic evidence.
                        let raw: Vec<u8> = row.get(2)?;
                        quarantined.push((uid, hash, err.to_string(), raw));
                    }
                    Err(err) => return Err(err.into()),
                }
            }
            (ret, quarantined)
        };
        // Quarantine each malformed row in a single transaction: move it
        // from `envelopes` to `invalid_envelopes`, preserving the original
        // blob, the stored hash and the conversion error.
        //
        // Logging discipline: the full `log::error!` fires only here, i.e.
        // when a row is actually moved (its first occurrence). Loads that
        // merely serve already-quarantined rows emit only the single
        // `log::debug!` summary below, so a permanently poisoned cache does
        // not produce error-level noise on every startup while the first
        // occurrence stays visible.
        if !quarantined.is_empty() {
            let first_seen = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs() as i64)
                .unwrap_or(0);
            match self
                .connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            {
                Ok(tx) => {
                    for (uid, hash, error, raw) in &quarantined {
                        log::error!(
                            "IMAP cache: quarantining malformed cached envelope of account {} in \
                             mailbox {} uid {}: it will be shown as a placeholder until the \
                             mailbox is resynced. Reason: {}",
                            self.uid_store.account_name,
                            mailbox_hash,
                            uid,
                            error
                        );
                        // Best-effort per row: a failed move is logged and
                        // retried on the next load (the row stays in
                        // `envelopes`).
                        if let Err(err) = tx.execute(
                            "DELETE FROM envelopes WHERE mailbox_hash = ?1 AND uid = ?2;",
                            sqlite3::params![mailbox_hash, *uid as Sqlite3UID],
                        ) {
                            log::error!(
                                "IMAP cache: could not delete malformed cached envelope of \
                                 account {} in mailbox {} uid {}: {}",
                                self.uid_store.account_name,
                                mailbox_hash,
                                uid,
                                err
                            );
                            continue;
                        }
                        if let Err(err) = tx.execute(
                            "INSERT OR REPLACE INTO invalid_envelopes (mailbox_hash, uid, hash, \
                             error, raw, first_seen) VALUES (?1, ?2, ?3, ?4, ?5, ?6);",
                            sqlite3::params![
                                mailbox_hash,
                                *uid as Sqlite3UID,
                                hash,
                                error,
                                raw,
                                first_seen
                            ],
                        ) {
                            log::error!(
                                "IMAP cache: could not quarantine malformed cached envelope of \
                                 account {} in mailbox {} uid {}: {}",
                                self.uid_store.account_name,
                                mailbox_hash,
                                uid,
                                err
                            );
                        }
                    }
                    if let Err(err) = tx.commit() {
                        log::error!(
                            "IMAP cache: could not commit quarantine of {} malformed cached \
                             envelopes of account {} in mailbox {}: {}",
                            quarantined.len(),
                            self.uid_store.account_name,
                            mailbox_hash,
                            err
                        );
                    }
                }
                Err(err) => log::error!(
                    "IMAP cache: could not open transaction to quarantine {} malformed cached \
                     envelopes of account {} in mailbox {}: {}",
                    quarantined.len(),
                    self.uid_store.account_name,
                    mailbox_hash,
                    err
                ),
            }
        }
        // Serve already-quarantined rows from the served page's uid window
        // as visible placeholders, so that undecodable cached messages
        // remain inspectable in the listing instead of being hidden. A uid
        // that is (again) present in `envelopes` takes precedence over its
        // stale quarantine entry. The window's lower bound is the lowest
        // uid of the envelope page just served (`ret` is DESC-ordered): a
        // row-based walk must not re-serve placeholders the next page will
        // cover, nor skip those inside this page.
        let page_lowest = ret.last().map(|(uid, ..)| *uid);
        let mut placeholders: Vec<(UID, EnvelopeHash, Envelope, Option<ModSequence>)> = Vec::new();
        {
            let mut stmt = self.connection.prepare(
                "SELECT uid, hash, error, length(raw), first_seen FROM invalid_envelopes WHERE \
                 mailbox_hash = ?1 AND uid <= ?2 AND uid >= ?3 AND uid NOT IN (SELECT uid FROM \
                 envelopes WHERE mailbox_hash = ?1) ORDER BY uid DESC LIMIT ?4;",
            )?;
            let mut rows = stmt.query(sqlite3::params![
                mailbox_hash,
                max,
                page_lowest.unwrap_or(1),
                batch_size as Sqlite3UID
            ])?;
            while let Some(row) = rows.next()? {
                let uid: UID = row.get(0)?;
                let hash: EnvelopeHash = row.get(1)?;
                let error: String = row.get(2)?;
                let raw_len: Option<i64> = row.get(3)?;
                let first_seen: i64 = row.get(4)?;
                placeholders.push((
                    uid,
                    hash,
                    quarantined_envelope_placeholder(uid, hash, &error, raw_len, first_seen),
                    None,
                ));
            }
        }
        if !placeholders.is_empty() {
            log::debug!(
                "IMAP cache: {} quarantined cached envelopes of account {} in mailbox {} shown \
                 as placeholders",
                placeholders.len(),
                self.uid_store.account_name,
                mailbox_hash
            );
        }
        let mut lowest_served: Option<UID> = None;
        let mut env_lck = self.uid_store.envelopes.lock().unwrap();
        let mut hash_index_lck = self.uid_store.hash_index.lock().unwrap();
        let mut uid_index_lck = self.uid_store.uid_index.lock().unwrap();
        let mut env_hashes = Vec::with_capacity(ret.len() + placeholders.len());
        for (uid, hash, env, modseq) in ret.into_iter().chain(placeholders) {
            if hash != env.hash() {
                return Ok(None);
            }
            env_hashes.push(env.hash());
            lowest_served = Some(match lowest_served {
                None => uid,
                Some(lowest) => lowest.min(uid),
            });
            hash_index_lck.insert(env.hash(), (uid, mailbox_hash));
            uid_index_lck.insert((mailbox_hash, uid), env.hash());
            env_lck.insert(
                env.hash(),
                CachedEnvelope {
                    inner: env,
                    uid,
                    mailbox_hash,
                    modsequence: modseq,
                },
            );
        }
        Ok(Some((env_hashes, lowest_served)))
    }

    fn insert_envelopes(
        &mut self,
        mailbox_hash: MailboxHash,
        fetches: &[FetchResponse<'_>],
    ) -> Result<()> {
        let mut lastseenuid = self
            .uid_store
            .lastseenuid
            .lock()
            .unwrap()
            .get(&mailbox_hash)
            .cloned()
            .unwrap_or_default();
        let Self {
            ref mut connection,
            ref uid_store,
            loaded_mailboxes: _,
            data_dir: _,
        } = self;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for item in fetches {
            if let FetchResponse {
                uid: Some(uid),
                message_sequence_number: _,
                modseq,
                flags: _,
                body: _,
                references: _,
                envelope: Some(envelope),
                raw_fetch_value: _,
                bodystructure: _,
            } = item
            {
                lastseenuid = lastseenuid.max(*uid);
                // A successful (re)fetch of a previously quarantined uid
                // heals it: the fresh row takes precedence and the stale
                // quarantine entry is removed so that no placeholder is
                // served for it anymore.
                let result = tx
                    .execute(
                        "INSERT OR REPLACE INTO envelopes (hash, uid, mailbox_hash, modsequence, \
                         envelope) VALUES (?1, ?2, ?3, ?4, ?5)",
                        sqlite3::params![
                            envelope.hash(),
                            *uid as Sqlite3UID,
                            mailbox_hash,
                            modseq,
                            &envelope
                        ],
                    )
                    .and_then(|_| {
                        tx.execute(
                            "DELETE FROM invalid_envelopes WHERE mailbox_hash = ?1 AND uid = ?2;",
                            sqlite3::params![mailbox_hash, *uid as Sqlite3UID],
                        )
                    });
                if let Err(err) = result {
                    let summary = format!(
                        "Could not insert envelope {} {} in header_cache of account {}",
                        envelope.message_id(),
                        envelope.hash(),
                        uid_store.account_name
                    );
                    if err.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation) {
                        // Our only constraint is the mailbox_hash foreign key.
                        return Err(Error::from(err)
                            .set_summary(summary)
                            .set_kind(ErrorKind::NotFound));
                    }
                    return Err(Error::from(err).set_summary(summary));
                }
            }
        }
        tx.commit()?;
        if let Ok(Some(new_lastseenuid)) = self.lastseenuid(mailbox_hash) {
            self.uid_store
                .lastseenuid
                .lock()
                .unwrap()
                .insert(mailbox_hash, new_lastseenuid);
        }
        Ok(())
    }

    fn update_flags(
        &mut self,
        env_hashes: EnvelopeHashBatch,
        mailbox_hash: MailboxHash,
        flags: Vec<FlagOp>,
    ) -> Result<()> {
        let Self {
            ref mut connection,
            ref uid_store,
            loaded_mailboxes: _,
            data_dir: _,
        } = self;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let values = std::rc::Rc::new(env_hashes.iter().map(Value::from).collect::<Vec<Value>>());

        let rows = tx
            .prepare("SELECT uid, envelope FROM envelopes WHERE hash IN rarray(?1);")?
            .query_map([values], |row| Ok((row.get(0)?, row.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect::<Vec<(UID, Envelope)>>();
        let mut stmt =
            tx.prepare("UPDATE envelopes SET envelope = ?1 WHERE mailbox_hash = ?2 AND uid = ?3;")?;
        for (uid, mut env) in rows {
            for op in flags.iter() {
                match op {
                    FlagOp::UnSet(flag) | FlagOp::Set(flag) => {
                        let mut f = env.flags();
                        f.set(*flag, op.as_bool());
                        env.set_flags(f);
                    }
                    FlagOp::UnSetTag(tag) | FlagOp::SetTag(tag) => {
                        let hash = TagHash::from_bytes(tag.as_bytes());
                        if op.as_bool() {
                            env.tags_mut().insert(hash);
                        } else {
                            env.tags_mut().shift_remove(&hash);
                        }
                    }
                }
            }
            stmt.execute(sqlite3::params![&env, mailbox_hash, uid as Sqlite3UID])?;
            uid_store
                .envelopes
                .lock()
                .unwrap()
                .entry(env.hash())
                .and_modify(|entry| {
                    entry.inner = env;
                });
        }
        drop(stmt);
        tx.commit()?;
        if let Ok(Some(new_lastseenuid)) = self.lastseenuid(mailbox_hash) {
            self.uid_store
                .lastseenuid
                .lock()
                .unwrap()
                .insert(mailbox_hash, new_lastseenuid);
        }
        Ok(())
    }

    fn update(
        &mut self,
        mailbox_hash: MailboxHash,
        refresh_events: &[(UID, RefreshEvent)],
    ) -> Result<()> {
        {
            let Self {
                ref mut connection,
                ref uid_store,
                loaded_mailboxes: _,
                data_dir: _,
            } = self;
            let tx =
                connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let mut hash_index_lck = uid_store.hash_index.lock().unwrap();
            // Hoist the statements out of the loop: preparing the same SQL for
            // every flag-change event is wasteful.
            let mut select_stmt =
                tx.prepare("SELECT envelope FROM envelopes WHERE mailbox_hash = ?1 AND uid = ?2;")?;
            let mut update_stmt = tx.prepare(
                "UPDATE envelopes SET envelope = ?1 WHERE mailbox_hash = ?2 AND uid = ?3;",
            )?;
            for (uid, event) in refresh_events {
                match &event.kind {
                    RefreshEventKind::Remove(env_hash) => {
                        hash_index_lck.remove(env_hash);
                        tx.execute(
                            "DELETE FROM envelopes WHERE mailbox_hash = ?1 AND uid = ?2;",
                            sqlite3::params![mailbox_hash, *uid as Sqlite3UID],
                        )
                        .chain_err_summary(|| {
                            format!(
                                "Could not remove envelope {} uid {} from  mailbox {} account {}",
                                env_hash, *uid, mailbox_hash, uid_store.account_name
                            )
                        })?;
                        // An expunged message must not leave a placeholder
                        // behind in the quarantine table.
                        tx.execute(
                            "DELETE FROM invalid_envelopes WHERE mailbox_hash = ?1 AND uid = ?2;",
                            sqlite3::params![mailbox_hash, *uid as Sqlite3UID],
                        )
                        .chain_err_summary(|| {
                            format!(
                                "Could not remove quarantined envelope uid {} from mailbox {} \
                                 account {}",
                                *uid, mailbox_hash, uid_store.account_name
                            )
                        })?;
                    }
                    RefreshEventKind::NewFlags(env_hash, (flags, tags)) => {
                        let mut ret: Vec<Envelope> = select_stmt
                            .query_map(sqlite3::params![mailbox_hash, *uid as Sqlite3UID], |row| {
                                row.get(0)
                            })?
                            .collect::<std::result::Result<_, _>>()?;
                        if let Some(mut env) = ret.pop() {
                            env.set_flags(*flags);
                            env.tags_mut().clear();
                            env.tags_mut()
                                .extend(tags.iter().map(|t| TagHash::from_bytes(t.as_bytes())));
                            update_stmt
                                .execute(sqlite3::params![&env, mailbox_hash, *uid as Sqlite3UID])
                                .chain_err_summary(|| {
                                    format!(
                                        "Could not update envelope {} uid {} from  mailbox {} \
                                         account {}",
                                        env_hash, *uid, mailbox_hash, uid_store.account_name
                                    )
                                })?;
                            uid_store
                                .envelopes
                                .lock()
                                .unwrap()
                                .entry(*env_hash)
                                .and_modify(|entry| {
                                    entry.inner = env;
                                });
                        }
                    }
                    _ => {}
                }
            }
            drop(select_stmt);
            drop(update_stmt);
            tx.commit()?;
        }
        if let Ok(Some(new_lastseenuid)) = self.lastseenuid(mailbox_hash) {
            self.uid_store
                .lastseenuid
                .lock()
                .unwrap()
                .insert(mailbox_hash, new_lastseenuid);
        }
        Ok(())
    }

    fn find_envelope(
        &mut self,
        identifier: std::result::Result<UID, EnvelopeHash>,
        mailbox_hash: MailboxHash,
    ) -> Result<Option<CachedEnvelope>> {
        let mut ret: Vec<(UID, Envelope, Option<ModSequence>)> = match identifier {
            Ok(uid) => self
                .connection
                .prepare(
                    "SELECT uid, envelope, modsequence FROM envelopes WHERE mailbox_hash = ?1 AND \
                     uid = ?2;",
                )?
                .query_map(sqlite3::params![mailbox_hash, uid as Sqlite3UID], |row| {
                    Ok((
                        row.get(0).map(|u: Sqlite3UID| u as UID)?,
                        row.get(1)?,
                        row.get(2)?,
                    ))
                })?
                .collect::<std::result::Result<_, _>>()?,
            Err(env_hash) => self
                .connection
                .prepare(
                    "SELECT uid, envelope, modsequence FROM envelopes WHERE mailbox_hash = ?1 AND \
                     hash = ?2;",
                )?
                .query_map(sqlite3::params![mailbox_hash, env_hash], |row| {
                    Ok((
                        row.get(0).map(|u: Sqlite3UID| u as UID)?,
                        row.get(1)?,
                        row.get(2)?,
                    ))
                })?
                .collect::<std::result::Result<_, _>>()?,
        };
        if ret.len() != 1 {
            return Ok(None);
        }
        let (uid, inner, modsequence) = ret.pop().unwrap();
        Ok(Some(CachedEnvelope {
            inner,
            uid,
            mailbox_hash,
            modsequence,
        }))
    }
}
