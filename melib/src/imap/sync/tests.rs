//
// meli
//
// Copyright 2024 Emmanouil Pitsidianakis <manos@pitsidianak.is>
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

#[cfg(feature = "sqlite3")]
#[test]
fn test_imap_sync_sqlite() {
    use crate::{backends::IsSubscribedFn, imap::*};

    let tempdir = tempfile::tempdir().unwrap();
    let account_hash = AccountHash::from_bytes(b"test".as_slice());
    let account_name = "test".to_string().into();
    let event_consumer = BackendEventConsumer::new(Arc::new(|_, _| {}));
    let uid_store: Arc<UIDStore> = Arc::new(UIDStore {
        offline_cache: Arc::new(Mutex::new(None)),
        ..UIDStore::new(
            IsSubscribedFn::default(),
            account_hash,
            account_name,
            event_consumer,
            None,
            true,
            true,
        )
    });
    let mut value =
        sync::sqlite3_cache::Sqlite3Cache::get(Arc::clone(&uid_store), Some(tempdir.path()))
            .unwrap();

    let mailbox_hash = MailboxHash::from(b"test".as_slice());
    let fetch = FetchResponse {
        uid: Some(1),
        message_sequence_number: 1,
        modseq: None,
        flags: None,
        body: None,
        references: None,
        envelope: Some(Envelope::default()),
        bodystructure: false,
        raw_fetch_value: &[],
    };
    let fetches = &[fetch];
    let err = value.insert_envelopes(mailbox_hash, fetches).unwrap_err();
    assert_eq!(
        err.inner
            .unwrap()
            .downcast_ref::<rusqlite::Error>()
            .unwrap()
            .sqlite_error_code(),
        Some(rusqlite::ErrorCode::ConstraintViolation)
    );
    assert_eq!(err.kind, ErrorKind::NotFound);
    value
        .init_mailbox(mailbox_hash, &SelectResponse::default())
        .unwrap();

    value.insert_envelopes(mailbox_hash, fetches).unwrap();
    _ = tempdir.close();
}

#[cfg(feature = "sqlite3")]
#[test]
fn test_imap_sync_sqlite3_status_roundtrip() {
    use crate::{
        backends::{IsSubscribedFn, MailboxHash},
        imap::*,
    };

    let tempdir = tempfile::tempdir().unwrap();
    let account_hash = AccountHash::from_bytes(b"test".as_slice());
    let account_name = "test".to_string().into();
    let event_consumer = BackendEventConsumer::new(Arc::new(|_, _| {}));
    let uid_store: Arc<UIDStore> = Arc::new(UIDStore {
        offline_cache: Arc::new(Mutex::new(None)),
        ..UIDStore::new(
            IsSubscribedFn::default(),
            account_hash,
            account_name,
            event_consumer,
            None,
            true,
            true,
        )
    });
    let mut value =
        sync::sqlite3_cache::Sqlite3Cache::get(Arc::clone(&uid_store), Some(tempdir.path()))
            .unwrap();
    let mailbox_hash = MailboxHash::from(b"test".as_slice());
    value
        .init_mailbox(mailbox_hash, &SelectResponse::default())
        .unwrap();

    assert_eq!(value.cached_status(mailbox_hash).unwrap(), None);
    value
        .record_status(mailbox_hash, Some(2), None, Some(4))
        .unwrap();
    assert_eq!(
        value.cached_status(mailbox_hash).unwrap(),
        Some((Some(2), None, Some(4)))
    );
    // Recording all-`None` counters clears the baseline.
    value.record_status(mailbox_hash, None, None, None).unwrap();
    assert_eq!(value.cached_status(mailbox_hash).unwrap(), None);
    // Unknown mailboxes have no cached status.
    assert_eq!(
        value
            .cached_status(MailboxHash::from(b"other".as_slice()))
            .unwrap(),
        None
    );
    _ = tempdir.close();
}

#[cfg(feature = "sqlite3")]
#[test]
fn test_imap_sync_sqlite3_status_migration() {
    use crate::{
        backends::{IsSubscribedFn, MailboxHash},
        imap::*,
        utils::sqlite3::rusqlite,
    };

    let tempdir = tempfile::tempdir().unwrap();
    let account_hash = AccountHash::from_bytes(b"test".as_slice());
    let account_name = "test".to_string().into();
    let event_consumer = BackendEventConsumer::new(Arc::new(|_, _| {}));
    let uid_store: Arc<UIDStore> = Arc::new(UIDStore {
        offline_cache: Arc::new(Mutex::new(None)),
        ..UIDStore::new(
            IsSubscribedFn::default(),
            account_hash,
            account_name,
            event_consumer,
            None,
            true,
            true,
        )
    });
    let mailbox_hash = MailboxHash::from(b"test".as_slice());
    // Simulate a database created by a previous version of `meli`: same
    // `user_version`, `mailbox` table without the `messages`/`unseen`/`uidnext`
    // columns, with an already cached mailbox row.
    let db_path = tempdir.path().join("test_header_cache.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "user_version", 5_u32).unwrap();
        conn.execute_batch(
            "CREATE TABLE envelopes (
                            hash             INTEGER NOT NULL,
                            mailbox_hash     INTEGER NOT NULL,
                            uid              INTEGER NOT NULL,
                            modsequence      INTEGER,
                            envelope         BLOB NOT NULL,
                            PRIMARY KEY (mailbox_hash, uid),
                            FOREIGN KEY (mailbox_hash) REFERENCES mailbox(mailbox_hash) ON DELETE CASCADE
                           );
            CREATE TABLE mailbox (
                        mailbox_hash     INTEGER UNIQUE,
                        uidvalidity      INTEGER,
                        max_uid          INTEGER,
                        flags            BLOB NOT NULL,
                        highestmodseq    INTEGER,
                        PRIMARY KEY (mailbox_hash)
                       );
            CREATE INDEX envelope_uid_idx ON envelopes(mailbox_hash, uid ASC);
            CREATE INDEX envelope_idx ON envelopes(hash);
            CREATE INDEX mailbox_idx ON mailbox(mailbox_hash);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO mailbox (mailbox_hash, uidvalidity, max_uid, flags) VALUES (?1, 1, 3, \
             X'');",
            rusqlite::params![mailbox_hash.0 as i64],
        )
        .unwrap();
    }
    // Opening the cache must migrate the schema additively, without errors and
    // without discarding the existing row.
    let mut value =
        sync::sqlite3_cache::Sqlite3Cache::get(Arc::clone(&uid_store), Some(tempdir.path()))
            .unwrap();
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let columns = conn
            .prepare("PRAGMA table_info(mailbox);")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        for column in ["messages", "unseen", "uidnext"] {
            assert!(
                columns.contains(&column.to_string()),
                "column {column} missing after migration: {columns:?}"
            );
        }
    }
    assert!(value.mailbox_state(mailbox_hash).unwrap().is_some());
    // No STATUS baseline was recorded yet, so the quick check cannot
    // spuriously short-circuit.
    assert_eq!(value.cached_status(mailbox_hash).unwrap(), None);
    value
        .record_status(mailbox_hash, Some(3), Some(3), Some(4))
        .unwrap();
    assert_eq!(
        value.cached_status(mailbox_hash).unwrap(),
        Some((Some(3), Some(3), Some(4)))
    );
    _ = tempdir.close();
}

#[cfg(feature = "sqlite3")]
#[test]
fn test_imap_sync_sqlite3_msn_index_roundtrip() {
    use crate::{
        backends::{IsSubscribedFn, MailboxHash},
        imap::*,
    };

    let tempdir = tempfile::tempdir().unwrap();
    let account_hash = AccountHash::from_bytes(b"test".as_slice());
    let account_name = "test".to_string().into();
    let event_consumer = BackendEventConsumer::new(Arc::new(|_, _| {}));
    let uid_store: Arc<UIDStore> = Arc::new(UIDStore {
        offline_cache: Arc::new(Mutex::new(None)),
        ..UIDStore::new(
            IsSubscribedFn::default(),
            account_hash,
            account_name,
            event_consumer,
            None,
            true,
            true,
        )
    });
    let mut value =
        sync::sqlite3_cache::Sqlite3Cache::get(Arc::clone(&uid_store), Some(tempdir.path()))
            .unwrap();
    let mailbox_hash = MailboxHash::from(b"test".as_slice());

    // Nothing stored yet.
    assert_eq!(value.load_msn_index(mailbox_hash, 1).unwrap(), None);
    // Gaps are stored and restored as `None` entries.
    let index = vec![Some(1 as UID), None, Some(3 as UID)];
    value.store_msn_index(mailbox_hash, 1, &index).unwrap();
    assert_eq!(value.load_msn_index(mailbox_hash, 1).unwrap(), Some(index));
    // A stale `UIDVALIDITY` must not return the stored rows.
    assert_eq!(value.load_msn_index(mailbox_hash, 2).unwrap(), None);
    // Re-storing under a new `UIDVALIDITY` replaces the stored index.
    let index_2 = vec![Some(7 as UID)];
    value.store_msn_index(mailbox_hash, 2, &index_2).unwrap();
    assert_eq!(
        value.load_msn_index(mailbox_hash, 2).unwrap(),
        Some(index_2)
    );
    assert_eq!(value.load_msn_index(mailbox_hash, 1).unwrap(), None);
    // Storing an empty index is equivalent to storing nothing: there is
    // no row to carry the `UIDVALIDITY`, so `load` returns `None` and
    // the caller falls back to the `UID SEARCH` (which is cheap for an
    // empty mailbox).
    value.store_msn_index(mailbox_hash, 2, &[]).unwrap();
    assert_eq!(value.load_msn_index(mailbox_hash, 2).unwrap(), None);
    // Other mailboxes are unaffected.
    assert_eq!(
        value
            .load_msn_index(MailboxHash::from(b"other".as_slice()), 2)
            .unwrap(),
        None
    );
    _ = tempdir.close();
}

#[cfg(feature = "sqlite3")]
#[test]
fn test_imap_sync_sqlite3_msn_index_migration() {
    use crate::{
        backends::{IsSubscribedFn, MailboxHash},
        imap::*,
        utils::sqlite3::rusqlite,
    };

    let tempdir = tempfile::tempdir().unwrap();
    let account_hash = AccountHash::from_bytes(b"test".as_slice());
    let account_name = "test".to_string().into();
    let event_consumer = BackendEventConsumer::new(Arc::new(|_, _| {}));
    let uid_store: Arc<UIDStore> = Arc::new(UIDStore {
        offline_cache: Arc::new(Mutex::new(None)),
        ..UIDStore::new(
            IsSubscribedFn::default(),
            account_hash,
            account_name,
            event_consumer,
            None,
            true,
            true,
        )
    });
    let mailbox_hash = MailboxHash::from(b"test".as_slice());
    // Simulate a database created by a previous version of `meli`: same
    // `user_version`, no `msn_cache` table.
    let db_path = tempdir.path().join("test_header_cache.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "user_version", 5_u32).unwrap();
        conn.execute_batch(
            "CREATE TABLE envelopes (
                            hash             INTEGER NOT NULL,
                            mailbox_hash     INTEGER NOT NULL,
                            uid              INTEGER NOT NULL,
                            modsequence      INTEGER,
                            envelope         BLOB NOT NULL,
                            PRIMARY KEY (mailbox_hash, uid),
                            FOREIGN KEY (mailbox_hash) REFERENCES mailbox(mailbox_hash) ON DELETE CASCADE
                           );
            CREATE TABLE mailbox (
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
            CREATE INDEX envelope_uid_idx ON envelopes(mailbox_hash, uid ASC);
            CREATE INDEX envelope_idx ON envelopes(hash);
            CREATE INDEX mailbox_idx ON mailbox(mailbox_hash);",
        )
        .unwrap();
    }
    // Opening the cache must create the `msn_cache` table additively
    // (`CREATE TABLE IF NOT EXISTS` in the init script runs on every
    // open), without errors and without touching the existing tables.
    let mut value =
        sync::sqlite3_cache::Sqlite3Cache::get(Arc::clone(&uid_store), Some(tempdir.path()))
            .unwrap();
    assert_eq!(value.load_msn_index(mailbox_hash, 1).unwrap(), None);
    let index = vec![Some(1 as UID), Some(2 as UID)];
    value.store_msn_index(mailbox_hash, 1, &index).unwrap();
    assert_eq!(value.load_msn_index(mailbox_hash, 1).unwrap(), Some(index));
    _ = tempdir.close();
}

/// The identity field set persisted by the mailbox list snapshot.
#[cfg(feature = "sqlite3")]
type MailboxIdentity = (
    crate::backends::MailboxHash,
    String,
    String,
    String,
    Option<crate::backends::MailboxHash>,
    Vec<crate::backends::MailboxHash>,
    u8,
    crate::backends::SpecialUsageMailbox,
    bool,
    bool,
);

/// A mailbox with a marker-derived identity and explicit tree links.
#[cfg(feature = "sqlite3")]
fn sample_mailbox(
    marker: &str,
    parent: Option<crate::backends::MailboxHash>,
    children: Vec<crate::backends::MailboxHash>,
    usage: crate::backends::SpecialUsageMailbox,
    separator: u8,
    no_select: bool,
    is_subscribed: bool,
) -> crate::imap::mailbox::ImapMailbox {
    use crate::imap::mailbox::ImapMailbox;
    use std::sync::RwLock;

    ImapMailbox {
        hash: crate::backends::MailboxHash::from_bytes(marker.as_bytes()),
        imap_path: format!("imap-{marker}"),
        path: format!("path-{marker}"),
        name: format!("name-{marker}"),
        parent,
        children,
        separator,
        usage: Arc::new(RwLock::new(usage)),
        no_select,
        is_subscribed,
        ..ImapMailbox::default()
    }
}

/// The identity of `m` as a comparable tuple.
#[cfg(feature = "sqlite3")]
fn mailbox_identity(m: &crate::imap::mailbox::ImapMailbox) -> MailboxIdentity {
    let usage = *m.usage.read().unwrap();
    (
        m.hash,
        m.imap_path.clone(),
        m.path.clone(),
        m.name.clone(),
        m.parent,
        m.children.clone(),
        m.separator,
        usage,
        m.no_select,
        m.is_subscribed,
    )
}

/// A three-mailbox list covering parent/child links, different usages,
/// separators and `no_select`/`is_subscribed` combinations.
#[cfg(feature = "sqlite3")]
fn sample_mailbox_list(
) -> std::collections::HashMap<crate::backends::MailboxHash, crate::imap::mailbox::ImapMailbox> {
    let parent_hash = crate::backends::MailboxHash::from_bytes(b"mailbox-list-parent");
    let child = sample_mailbox(
        "mailbox-list-child",
        Some(parent_hash),
        vec![],
        crate::backends::SpecialUsageMailbox::Normal,
        b'/',
        false,
        true,
    );
    let parent = sample_mailbox(
        "mailbox-list-parent",
        None,
        vec![child.hash],
        crate::backends::SpecialUsageMailbox::Drafts,
        b'.',
        false,
        true,
    );
    let lone = sample_mailbox(
        "mailbox-list-lone",
        None,
        vec![],
        crate::backends::SpecialUsageMailbox::Normal,
        b'.',
        true,
        false,
    );
    std::iter::once((parent.hash, parent))
        .chain(std::iter::once((child.hash, child)))
        .chain(std::iter::once((lone.hash, lone)))
        .collect()
}

#[cfg(feature = "sqlite3")]
#[test]
fn mailbox_list_roundtrip() {
    use crate::{
        backends::{MailboxHash, MailboxPermissions, SpecialUsageMailbox},
        imap::*,
        utils::sqlite3::rusqlite,
    };
    use std::collections::HashMap;

    let (tempdir, _uid_store, mut value) = sqlite3_cache_handle();
    let mailboxes = sample_mailbox_list();
    let expected: HashMap<_, MailboxIdentity> = mailboxes
        .iter()
        .map(|(hash, m)| (*hash, mailbox_identity(m)))
        .collect();

    value.save_mailbox_list(&mailboxes).unwrap();
    let loaded = value.load_mailbox_list().unwrap().unwrap();
    assert_eq!(loaded.len(), expected.len());
    for (hash, mailbox) in &loaded {
        assert_eq!(
            mailbox_identity(mailbox),
            expected[hash],
            "identity of {hash}"
        );
    }

    // Counts, `select` state, `warm` and `permissions` are deliberately
    // not persisted; they are rebuilt by later SELECT/resync operations.
    for mailbox in loaded.values() {
        assert!(mailbox.select.read().unwrap().is_none());
        assert!(!mailbox.is_warm());
        assert_eq!(mailbox.exists.lock().unwrap().len(), 0);
        assert_eq!(mailbox.unseen.lock().unwrap().len(), 0);
        assert_eq!(
            *mailbox.permissions.lock().unwrap(),
            MailboxPermissions::default()
        );
    }
    assert!(loaded
        .values()
        .any(|m| m.special_usage() == SpecialUsageMailbox::Drafts));

    // Malformed payload probe: a corrupted blob must surface as an error
    // (the caller decides the degradation) without disturbing other
    // tables.
    let mailbox_hash = MailboxHash::from(b"test".as_slice());
    value
        .init_mailbox(mailbox_hash, &SelectResponse::default())
        .unwrap();
    value
        .record_status(mailbox_hash, Some(1), None, Some(2))
        .unwrap();
    {
        let db_path = tempdir.path().join("test_header_cache.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute(
            "UPDATE mailbox_list SET payload = ?1 WHERE id = 0;",
            rusqlite::params![b"{not json".as_slice()],
        )
        .unwrap();
    }
    value.load_mailbox_list().unwrap_err();
    // Other tables are unaffected by the corrupted `mailbox_list` blob.
    assert_eq!(
        value.cached_status(mailbox_hash).unwrap(),
        Some((Some(1), None, Some(2)))
    );
    _ = tempdir.close();
}

#[cfg(feature = "sqlite3")]
#[test]
fn mailbox_list_empty_db_returns_none() {
    use crate::{imap::*, utils::sqlite3::rusqlite};

    // A fresh cache has no mailbox list.
    let (tempdir, uid_store, mut value) = sqlite3_cache_handle();
    assert!(value.load_mailbox_list().unwrap().is_none());
    drop(value);
    _ = tempdir.close();

    // A database created before the `mailbox_list` table existed (same
    // `user_version`): the init script must create the missing table on
    // open without a reset, `load` returns `None`, and a subsequent
    // save/load roundtrip works on the migrated database.
    let tempdir = tempfile::tempdir().unwrap();
    let db_path = tempdir.path().join("test_header_cache.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "user_version", 5_u32).unwrap();
        conn.execute_batch(
            "CREATE TABLE envelopes (
                            hash             INTEGER NOT NULL,
                            mailbox_hash     INTEGER NOT NULL,
                            uid              INTEGER NOT NULL,
                            modsequence      INTEGER,
                            envelope         BLOB NOT NULL,
                            PRIMARY KEY (mailbox_hash, uid),
                            FOREIGN KEY (mailbox_hash) REFERENCES mailbox(mailbox_hash) ON DELETE CASCADE
                           );
            CREATE TABLE mailbox (
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
            CREATE TABLE msn_cache (
                        mailbox_hash     INTEGER NOT NULL,
                        uidvalidity      INTEGER NOT NULL,
                        msn              INTEGER NOT NULL,
                        uid              INTEGER NOT NULL,
                        PRIMARY KEY (mailbox_hash, msn)
                       ) WITHOUT ROWID;
            CREATE TABLE invalid_envelopes (
                        mailbox_hash     INTEGER NOT NULL,
                        uid              INTEGER NOT NULL,
                        hash             INTEGER NOT NULL,
                        error            TEXT NOT NULL,
                        raw              BLOB,
                        first_seen       INTEGER NOT NULL,
                        PRIMARY KEY (mailbox_hash, uid)
                       ) WITHOUT ROWID;
            CREATE INDEX envelope_uid_idx ON envelopes(mailbox_hash, uid ASC);
            CREATE INDEX envelope_idx ON envelopes(hash);
            CREATE INDEX mailbox_idx ON mailbox(mailbox_hash);",
        )
        .unwrap();
    }
    let mut value =
        sync::sqlite3_cache::Sqlite3Cache::get(Arc::clone(&uid_store), Some(tempdir.path()))
            .unwrap();
    assert!(value.load_mailbox_list().unwrap().is_none());
    let mailboxes = sample_mailbox_list();
    value.save_mailbox_list(&mailboxes).unwrap();
    let loaded = value.load_mailbox_list().unwrap().unwrap();
    assert_eq!(loaded.len(), mailboxes.len());
    for (hash, mailbox) in &loaded {
        assert_eq!(
            mailbox_identity(mailbox),
            mailbox_identity(&mailboxes[hash])
        );
    }
    _ = tempdir.close();
}

#[cfg(feature = "sqlite3")]
#[test]
fn mailbox_list_overwrite_replaces() {
    use crate::backends::MailboxHash;
    use std::collections::HashMap;

    let (tempdir, _uid_store, mut value) = sqlite3_cache_handle();
    let first = sample_mailbox_list();
    value.save_mailbox_list(&first).unwrap();

    // A second save replaces the whole list: only the new mailbox is
    // visible afterwards.
    let replacement = sample_mailbox(
        "mailbox-list-replacement",
        None,
        vec![],
        crate::backends::SpecialUsageMailbox::Archive,
        b'.',
        false,
        true,
    );
    let second: HashMap<_, _> = std::iter::once((replacement.hash, replacement)).collect();
    value.save_mailbox_list(&second).unwrap();
    let loaded = value.load_mailbox_list().unwrap().unwrap();
    assert_eq!(loaded.len(), 1);
    let (hash, mailbox) = loaded.iter().next().unwrap();
    assert_eq!(*hash, MailboxHash::from_bytes(b"mailbox-list-replacement"));
    assert_eq!(mailbox_identity(mailbox), mailbox_identity(&second[hash]));
    for old_hash in first.keys() {
        assert!(!loaded.contains_key(old_hash));
    }
    _ = tempdir.close();
}

#[cfg(feature = "sqlite3")]
#[test]
fn test_imap_sync_sqlite3_poison_row_isolated() {
    use crate::{
        backends::{IsSubscribedFn, MailboxHash},
        email::{Address, EnvelopeHash},
        imap::*,
        utils::sqlite3::rusqlite,
    };

    let tempdir = tempfile::tempdir().unwrap();
    let account_hash = AccountHash::from_bytes(b"test".as_slice());
    let account_name = "test".to_string().into();
    let event_consumer = BackendEventConsumer::new(Arc::new(|_, _| {}));
    let uid_store: Arc<UIDStore> = Arc::new(UIDStore {
        offline_cache: Arc::new(Mutex::new(None)),
        ..UIDStore::new(
            IsSubscribedFn::default(),
            account_hash,
            account_name,
            event_consumer,
            None,
            true,
            true,
        )
    });
    let mut value =
        sync::sqlite3_cache::Sqlite3Cache::get(Arc::clone(&uid_store), Some(tempdir.path()))
            .unwrap();
    let mailbox_hash = MailboxHash::from(b"test".as_slice());
    value
        .init_mailbox(mailbox_hash, &SelectResponse::default())
        .unwrap();
    // Pristine state that the malformed-row cleanup must not disturb.
    value
        .record_status(mailbox_hash, Some(3), Some(1), Some(6))
        .unwrap();
    let msn_index = vec![Some(1 as UID), Some(2 as UID)];
    value.store_msn_index(mailbox_hash, 1, &msn_index).unwrap();

    // Three healthy envelopes and two "poison" ones. The poison address
    // replicates what the lenient network ENVELOPE parser accepts (and
    // caches) but strict deserialization rejects on load: a display name
    // that needs quoting and an address-spec with an empty local part.
    let healthy_envs: Vec<Envelope> = (0..3)
        .map(|i| {
            let mut env = Envelope::default();
            env.set_hash(EnvelopeHash::from_bytes(format!("healthy-{i}").as_bytes()));
            env.set_from(
                vec![Address::new(
                    None::<String>,
                    format!("healthy{i}@example.com"),
                )]
                .into(),
            );
            env
        })
        .collect();
    let poison_envs: Vec<Envelope> = (0..2)
        .map(|i| {
            let mut env = Envelope::default();
            env.set_hash(EnvelopeHash::from_bytes(format!("poison-{i}").as_bytes()));
            env.set_from(vec![Address::new(Some("recipients:"), "recipients:@qq.com")].into());
            env
        })
        .collect();
    // Sanity: the poison envelope really is unloadable through the cache
    // serialization seam (the strict re-parse of the display string).
    let poison_blob = serde_json::to_vec(&poison_envs[0]).unwrap();
    serde_json::from_slice::<Envelope>(&poison_blob).unwrap_err();

    let mut fetches = Vec::new();
    for (i, env) in healthy_envs.iter().chain(poison_envs.iter()).enumerate() {
        fetches.push(FetchResponse {
            uid: Some(i as UID + 1),
            message_sequence_number: i + 1,
            modseq: None,
            flags: None,
            body: None,
            references: None,
            envelope: Some(env.clone()),
            bodystructure: false,
            raw_fetch_value: &[],
        });
    }
    // The write path stores the serialized envelope without re-parsing it,
    // so all five rows are cached before the load.
    value.insert_envelopes(mailbox_hash, &fetches).unwrap();

    let db_path = tempdir.path().join("test_header_cache.db");
    let envelope_rows = |condition: &str| -> i64 {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM envelopes WHERE mailbox_hash = {} {condition};",
                mailbox_hash.0 as i64
            ),
            [],
            |row| row.get(0),
        )
        .unwrap()
    };
    assert_eq!(envelope_rows(""), 5);

    // Loading must return the healthy envelopes instead of resetting the
    // cache: the poison rows are quarantined (moved into
    // `invalid_envelopes`) and served back as visible placeholders.
    let mut loaded = value.envelopes(mailbox_hash, 5, 100).unwrap().unwrap();
    let mut expected_hashes: Vec<EnvelopeHash> = healthy_envs
        .iter()
        .chain(poison_envs.iter())
        .map(|env| env.hash())
        .collect();
    expected_hashes.sort_unstable();
    loaded.sort_unstable();
    assert_eq!(loaded, expected_hashes);
    assert!(db_path.exists());
    // The poison rows moved out of `envelopes`; the healthy rows survive.
    assert_eq!(envelope_rows("AND uid >= 4"), 0);
    assert_eq!(envelope_rows(""), 3);
    // The quarantine preserved the stored hash, the conversion error, the
    // original raw blob and a first_seen timestamp per moved row.
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT uid, hash, error, raw, first_seen FROM invalid_envelopes WHERE \
                 mailbox_hash = ?1 ORDER BY uid ASC;",
            )
            .unwrap();
        let rows = stmt
            .query_map(rusqlite::params![mailbox_hash.0 as i64], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(rows.len(), 2);
        for (i, (uid, hash, error, raw, first_seen)) in rows.into_iter().enumerate() {
            assert_eq!(uid, i as i64 + 4);
            assert_eq!(hash, poison_envs[i].hash().0 as i64);
            assert!(!error.is_empty());
            assert_eq!(raw, serde_json::to_vec(&poison_envs[i]).unwrap());
            assert!(first_seen > 0);
        }
    }
    // The placeholders carry the stored hash as identity, name the uid in
    // their subject, use the round-trip-safe placeholder `From` address and
    // expose the conversion error via a pseudo-header.
    for (i, env) in poison_envs.iter().enumerate() {
        let uid = i as UID + 4;
        let entry = uid_store
            .envelopes
            .lock()
            .unwrap()
            .get(&env.hash())
            .map(|entry| {
                (
                    entry.uid,
                    entry.inner.subject().into_owned(),
                    entry.inner.from.first().cloned(),
                    entry
                        .inner
                        .other_headers()
                        .get("X-Meli-Cache-Error")
                        .map(str::to_string),
                    entry
                        .inner
                        .other_headers()
                        .get("X-Meli-Cache-Raw-Bytes")
                        .map(str::to_string),
                )
            });
        let (entry_uid, subject, from, cache_error, raw_bytes) = entry.unwrap();
        assert_eq!(entry_uid, uid);
        assert!(subject.contains(&format!("(uid {uid})")));
        assert_eq!(
            from.unwrap(),
            Address::new(
                Some("[meli] cached envelope decode error"),
                "invalid@invalid.invalid"
            )
        );
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let db_error: String = conn
            .query_row(
                "SELECT error FROM invalid_envelopes WHERE mailbox_hash = ?1 AND uid = ?2;",
                rusqlite::params![mailbox_hash.0 as i64, uid as i64],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cache_error.unwrap(), db_error);
        assert_eq!(
            raw_bytes.unwrap(),
            serde_json::to_vec(&poison_envs[i])
                .unwrap()
                .len()
                .to_string()
        );
    }
    // The quarantine must not disturb the rest of the cache state.
    assert!(value.mailbox_state(mailbox_hash).unwrap().is_some());
    assert_eq!(
        value.cached_status(mailbox_hash).unwrap(),
        Some((Some(3), Some(1), Some(6)))
    );
    assert_eq!(
        value.load_msn_index(mailbox_hash, 1).unwrap(),
        Some(msn_index)
    );
    assert_eq!(uid_store.envelopes.lock().unwrap().len(), 5);
    _ = tempdir.close();
}

#[cfg(feature = "sqlite3")]
#[test]
fn test_imap_sync_sqlite3_all_rows_poison_no_reset() {
    use crate::{
        backends::{IsSubscribedFn, MailboxHash},
        email::{Address, EnvelopeHash},
        imap::*,
        utils::sqlite3::rusqlite,
    };

    let tempdir = tempfile::tempdir().unwrap();
    let account_hash = AccountHash::from_bytes(b"test".as_slice());
    let account_name = "test".to_string().into();
    let event_consumer = BackendEventConsumer::new(Arc::new(|_, _| {}));
    let uid_store: Arc<UIDStore> = Arc::new(UIDStore {
        offline_cache: Arc::new(Mutex::new(None)),
        ..UIDStore::new(
            IsSubscribedFn::default(),
            account_hash,
            account_name,
            event_consumer,
            None,
            true,
            true,
        )
    });
    let mut value =
        sync::sqlite3_cache::Sqlite3Cache::get(Arc::clone(&uid_store), Some(tempdir.path()))
            .unwrap();
    let mailbox_hash = MailboxHash::from(b"test".as_slice());
    value
        .init_mailbox(mailbox_hash, &SelectResponse::default())
        .unwrap();

    let poison_envs: Vec<Envelope> = (0..2)
        .map(|i| {
            let mut env = Envelope::default();
            env.set_hash(EnvelopeHash::from_bytes(format!("poison-{i}").as_bytes()));
            env.set_from(vec![Address::new(Some("recipients:"), "recipients:@qq.com")].into());
            env
        })
        .collect();
    let mut fetches = Vec::new();
    for (i, env) in poison_envs.iter().enumerate() {
        fetches.push(FetchResponse {
            uid: Some(i as UID + 1),
            message_sequence_number: i + 1,
            modseq: None,
            flags: None,
            body: None,
            references: None,
            envelope: Some(env.clone()),
            bodystructure: false,
            raw_fetch_value: &[],
        });
    }
    value.insert_envelopes(mailbox_hash, &fetches).unwrap();

    let db_path = tempdir.path().join("test_header_cache.db");
    // Even when every cached row is malformed, the load must not fail and
    // must not reset the cache: it returns a placeholder per quarantined
    // row and the rows are moved out of `envelopes` individually.
    let mut loaded = value.envelopes(mailbox_hash, 2, 100).unwrap().unwrap();
    let mut poison_hashes: Vec<EnvelopeHash> = poison_envs.iter().map(|env| env.hash()).collect();
    poison_hashes.sort_unstable();
    loaded.sort_unstable();
    assert_eq!(loaded, poison_hashes);
    assert!(db_path.exists());
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM envelopes WHERE mailbox_hash = ?1;",
            rusqlite::params![mailbox_hash.0 as i64],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rows, 0);
    let quarantined: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM invalid_envelopes WHERE mailbox_hash = ?1;",
            rusqlite::params![mailbox_hash.0 as i64],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(quarantined, 2);
    // The mailbox itself is still cached (only its envelopes are gone).
    assert!(value.mailbox_state(mailbox_hash).unwrap().is_some());
    assert_eq!(uid_store.envelopes.lock().unwrap().len(), 2);
    _ = tempdir.close();
}

#[cfg(feature = "sqlite3")]
#[test]
fn test_imap_sync_sqlite3_healthy_rows_unchanged() {
    use crate::{
        backends::{IsSubscribedFn, MailboxHash},
        email::{Address, EnvelopeHash},
        imap::*,
        utils::sqlite3::rusqlite,
    };

    let tempdir = tempfile::tempdir().unwrap();
    let account_hash = AccountHash::from_bytes(b"test".as_slice());
    let account_name = "test".to_string().into();
    let event_consumer = BackendEventConsumer::new(Arc::new(|_, _| {}));
    let uid_store: Arc<UIDStore> = Arc::new(UIDStore {
        offline_cache: Arc::new(Mutex::new(None)),
        ..UIDStore::new(
            IsSubscribedFn::default(),
            account_hash,
            account_name,
            event_consumer,
            None,
            true,
            true,
        )
    });
    let mut value =
        sync::sqlite3_cache::Sqlite3Cache::get(Arc::clone(&uid_store), Some(tempdir.path()))
            .unwrap();
    let mailbox_hash = MailboxHash::from(b"test".as_slice());
    value
        .init_mailbox(mailbox_hash, &SelectResponse::default())
        .unwrap();

    // Zero poison: a cache of healthy envelopes loads unchanged.
    let healthy_envs: Vec<Envelope> = (0..3)
        .map(|i| {
            let mut env = Envelope::default();
            env.set_hash(EnvelopeHash::from_bytes(format!("healthy-{i}").as_bytes()));
            env.set_from(
                vec![Address::new(
                    None::<String>,
                    format!("healthy{i}@example.com"),
                )]
                .into(),
            );
            env
        })
        .collect();
    let mut fetches = Vec::new();
    for (i, env) in healthy_envs.iter().enumerate() {
        fetches.push(FetchResponse {
            uid: Some(i as UID + 1),
            message_sequence_number: i + 1,
            modseq: None,
            flags: None,
            body: None,
            references: None,
            envelope: Some(env.clone()),
            bodystructure: false,
            raw_fetch_value: &[],
        });
    }
    value.insert_envelopes(mailbox_hash, &fetches).unwrap();

    let mut loaded = value.envelopes(mailbox_hash, 3, 100).unwrap().unwrap();
    let mut healthy_hashes: Vec<EnvelopeHash> = healthy_envs.iter().map(|env| env.hash()).collect();
    healthy_hashes.sort_unstable();
    loaded.sort_unstable();
    assert_eq!(loaded, healthy_hashes);
    assert_eq!(uid_store.envelopes.lock().unwrap().len(), 3);
    // No quarantine happened and none was needed.
    let db_path = tempdir.path().join("test_header_cache.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let quarantined: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM invalid_envelopes WHERE mailbox_hash = ?1;",
            rusqlite::params![mailbox_hash.0 as i64],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(quarantined, 0);
    _ = tempdir.close();
}

use std::sync::Arc;

/// Boilerplate shared by the sqlite3 cache tests: a `Sqlite3Cache` over a
/// fresh temporary directory plus its `UIDStore`.
#[cfg(feature = "sqlite3")]
fn sqlite3_cache_handle() -> (
    tempfile::TempDir,
    Arc<crate::imap::UIDStore>,
    Box<dyn crate::imap::sync::cache::ImapCache>,
) {
    use crate::{backends::IsSubscribedFn, imap::*};

    let tempdir = tempfile::tempdir().unwrap();
    let account_hash = AccountHash::from_bytes(b"test".as_slice());
    let account_name = "test".to_string().into();
    let event_consumer = BackendEventConsumer::new(Arc::new(|_, _| {}));
    let uid_store: Arc<UIDStore> = Arc::new(UIDStore {
        offline_cache: Arc::new(Mutex::new(None)),
        ..UIDStore::new(
            IsSubscribedFn::default(),
            account_hash,
            account_name,
            event_consumer,
            None,
            true,
            true,
        )
    });
    let value =
        sync::sqlite3_cache::Sqlite3Cache::get(Arc::clone(&uid_store), Some(tempdir.path()))
            .unwrap();
    (tempdir, uid_store, value)
}

/// An envelope that the lenient network ENVELOPE parser accepts (and the
/// write path stores) but strict cache deserialization rejects on load.
#[cfg(feature = "sqlite3")]
fn poison_envelope(marker: &str) -> crate::email::Envelope {
    use crate::email::{Address, EnvelopeHash};

    let mut env = crate::email::Envelope::default();
    env.set_hash(EnvelopeHash::from_bytes(marker.as_bytes()));
    env.set_from(vec![Address::new(Some("recipients:"), "recipients:@qq.com")].into());
    env
}

/// A cache-healthy envelope with a stable, marker-derived identity.
#[cfg(feature = "sqlite3")]
fn healthy_envelope(marker: &str) -> crate::email::Envelope {
    use crate::email::{Address, EnvelopeHash};

    let mut env = crate::email::Envelope::default();
    env.set_hash(EnvelopeHash::from_bytes(marker.as_bytes()));
    env.set_from(
        vec![Address::new(
            None::<String>,
            format!("{marker}@example.com"),
        )]
        .into(),
    );
    env
}

#[cfg(feature = "sqlite3")]
fn fetch_response_for(
    uid: usize,
    env: &crate::email::Envelope,
) -> crate::imap::FetchResponse<'static> {
    crate::imap::FetchResponse {
        uid: Some(uid as crate::imap::UID),
        message_sequence_number: uid,
        modseq: None,
        flags: None,
        body: None,
        references: None,
        envelope: Some(env.clone()),
        bodystructure: false,
        raw_fetch_value: &[],
    }
}

#[cfg(feature = "sqlite3")]
#[test]
fn test_imap_sync_sqlite3_quarantine_placeholder_visible() {
    use crate::{backends::MailboxHash, imap::*};

    let (tempdir, uid_store, mut value) = sqlite3_cache_handle();
    let mailbox_hash = MailboxHash::from(b"test".as_slice());
    value
        .init_mailbox(mailbox_hash, &SelectResponse::default())
        .unwrap();

    let healthy_env = healthy_envelope("healthy-marker-0");
    let healthy_hash = healthy_env.hash();
    let healthy = fetch_response_for(1, &healthy_env);
    let poison = poison_envelope("poison-marker-0");
    let poison_hash = poison.hash();
    let fetches = &[healthy, fetch_response_for(2, &poison)];
    value.insert_envelopes(mailbox_hash, fetches).unwrap();

    // The quarantined row must stay visible: the load returns the healthy
    // envelope AND a placeholder entry for the undecodable one.
    let mut loaded = value.envelopes(mailbox_hash, 2, 100).unwrap().unwrap();
    loaded.sort_unstable();
    let mut expected = vec![healthy_hash, poison_hash];
    expected.sort_unstable();
    assert_eq!(
        loaded, expected,
        "quarantined uid must be served as a placeholder"
    );
    let placeholder = uid_store
        .envelopes
        .lock()
        .unwrap()
        .get(&poison_hash)
        .map(|entry| (entry.uid, entry.inner.subject().into_owned()))
        .unwrap();
    assert_eq!(placeholder.0, 2);
    assert!(placeholder.1.contains("uid 2"));
    _ = tempdir.close();
}

#[cfg(feature = "sqlite3")]
#[test]
fn test_imap_sync_sqlite3_quarantine_second_load_idempotent() {
    use crate::{backends::MailboxHash, imap::*, utils::sqlite3::rusqlite};

    let (tempdir, uid_store, mut value) = sqlite3_cache_handle();
    let mailbox_hash = MailboxHash::from(b"test".as_slice());
    value
        .init_mailbox(mailbox_hash, &SelectResponse::default())
        .unwrap();

    let healthy_env = healthy_envelope("idempotent-healthy");
    let poison = poison_envelope("idempotent-poison");
    let fetches = &[
        fetch_response_for(1, &healthy_env),
        fetch_response_for(2, &poison),
    ];
    value.insert_envelopes(mailbox_hash, fetches).unwrap();

    let db_path = tempdir.path().join("test_header_cache.db");
    let quarantine_rows = || -> Vec<(i64, i64)> {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let rows = conn
            .prepare(
                "SELECT uid, first_seen FROM invalid_envelopes WHERE mailbox_hash = ?1 ORDER BY \
                 uid ASC;",
            )
            .unwrap()
            .query_map(rusqlite::params![mailbox_hash.0 as i64], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        rows
    };

    let mut first = value.envelopes(mailbox_hash, 2, 100).unwrap().unwrap();
    first.sort_unstable();
    let rows_after_first = quarantine_rows();
    assert_eq!(rows_after_first.len(), 1);
    assert_eq!(rows_after_first[0].0, 2);

    // Second load is idempotent: the same healthy + placeholder set is
    // served, no duplicate quarantine row appears and `first_seen` is
    // untouched. `first_seen` is written only by the quarantine move,
    // which is the same code path that emits the per-row `log::error!`,
    // so an unchanged `first_seen` and row count also prove that the
    // error-level log did not fire again on this load. (Direct log
    // capture cannot be shared across test modules: melib's lib tests
    // run in one process, only one global logger may be installed, and
    // the ENVELOPE parser tests already install one.)
    let mut second = value.envelopes(mailbox_hash, 2, 100).unwrap().unwrap();
    second.sort_unstable();
    assert_eq!(second, first);
    assert_eq!(quarantine_rows(), rows_after_first);
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let healthy_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM envelopes WHERE mailbox_hash = ?1;",
            rusqlite::params![mailbox_hash.0 as i64],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(healthy_rows, 1);
    assert_eq!(uid_store.envelopes.lock().unwrap().len(), 2);
    _ = tempdir.close();
}

#[cfg(feature = "sqlite3")]
#[test]
fn test_imap_sync_sqlite3_quarantine_uid_window_batching() {
    use crate::{backends::MailboxHash, imap::*, utils::sqlite3::rusqlite};

    let (tempdir, _uid_store, mut value) = sqlite3_cache_handle();
    let mailbox_hash = MailboxHash::from(b"test".as_slice());
    value
        .init_mailbox(mailbox_hash, &SelectResponse::default())
        .unwrap();

    let poison_in_window = poison_envelope("window-poison-in");
    let poison_out_of_window = poison_envelope("window-poison-out");
    let mut fetches = Vec::new();
    for uid in [1_usize, 2, 6, 7, 9, 10] {
        let healthy = healthy_envelope(&format!("window-healthy-{uid}"));
        fetches.push(fetch_response_for(uid, &healthy));
    }
    fetches.push(fetch_response_for(3, &poison_out_of_window));
    fetches.push(fetch_response_for(8, &poison_in_window));
    value.insert_envelopes(mailbox_hash, &fetches).unwrap();

    let db_path = tempdir.path().join("test_header_cache.db");
    let table_count = |table: &str| -> i64 {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE mailbox_hash = ?1;"),
            rusqlite::params![mailbox_hash.0 as i64],
            |row| row.get(0),
        )
        .unwrap()
    };

    // Window [6, 10] (lastseenuid 10, batch 4): the four healthy rows
    // plus a placeholder for the quarantined uid 8; uid 3 has not been
    // read yet, so it is neither quarantined nor served.
    let loaded = value.envelopes(mailbox_hash, 10, 4).unwrap().unwrap();
    assert_eq!(loaded.len(), 5);
    assert!(loaded.contains(&poison_in_window.hash()));
    assert!(!loaded.contains(&poison_out_of_window.hash()));
    assert_eq!(table_count("invalid_envelopes"), 1);
    assert_eq!(table_count("envelopes"), 7);

    // A batch whose window covers uid 3 quarantines and serves it too.
    let loaded = value.envelopes(mailbox_hash, 3, 10).unwrap().unwrap();
    assert_eq!(loaded.len(), 3);
    assert!(loaded.contains(&poison_out_of_window.hash()));
    assert_eq!(table_count("invalid_envelopes"), 2);
    assert_eq!(table_count("envelopes"), 6);
    _ = tempdir.close();
}

#[cfg(feature = "sqlite3")]
#[test]
fn test_imap_sync_sqlite3_quarantine_legacy_schema_migration() {
    use crate::{
        backends::{IsSubscribedFn, MailboxHash},
        imap::*,
        utils::sqlite3::rusqlite,
    };

    let tempdir = tempfile::tempdir().unwrap();
    let account_hash = AccountHash::from_bytes(b"test".as_slice());
    let account_name = "test".to_string().into();
    let event_consumer = BackendEventConsumer::new(Arc::new(|_, _| {}));
    let uid_store: Arc<UIDStore> = Arc::new(UIDStore {
        offline_cache: Arc::new(Mutex::new(None)),
        ..UIDStore::new(
            IsSubscribedFn::default(),
            account_hash,
            account_name,
            event_consumer,
            None,
            true,
            true,
        )
    });
    let mailbox_hash = MailboxHash::from(b"test".as_slice());
    // Simulate a database created before the `invalid_envelopes` table
    // existed (same `user_version`): quarantine must work on it without a
    // reset, the table being created by the init script on open.
    let db_path = tempdir.path().join("test_header_cache.db");
    {
        let healthy_env = healthy_envelope("legacy-healthy");
        let poison = poison_envelope("legacy-poison");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.pragma_update(None, "user_version", 5_u32).unwrap();
        conn.execute_batch(
            "CREATE TABLE envelopes (
                            hash             INTEGER NOT NULL,
                            mailbox_hash     INTEGER NOT NULL,
                            uid              INTEGER NOT NULL,
                            modsequence      INTEGER,
                            envelope         BLOB NOT NULL,
                            PRIMARY KEY (mailbox_hash, uid),
                            FOREIGN KEY (mailbox_hash) REFERENCES mailbox(mailbox_hash) ON DELETE CASCADE
                           );
            CREATE TABLE mailbox (
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
            CREATE TABLE msn_cache (
                        mailbox_hash     INTEGER NOT NULL,
                        uidvalidity      INTEGER NOT NULL,
                        msn              INTEGER NOT NULL,
                        uid              INTEGER NOT NULL,
                        PRIMARY KEY (mailbox_hash, msn)
                       ) WITHOUT ROWID;
            CREATE INDEX envelope_uid_idx ON envelopes(mailbox_hash, uid ASC);
            CREATE INDEX envelope_idx ON envelopes(hash);
            CREATE INDEX mailbox_idx ON mailbox(mailbox_hash);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO mailbox (mailbox_hash, uidvalidity, max_uid, flags) VALUES (?1, 1, 2, \
             X'');",
            rusqlite::params![mailbox_hash.0 as i64],
        )
        .unwrap();
        for (uid, hash, blob) in [
            (
                1_i64,
                healthy_env.hash().0 as i64,
                serde_json::to_vec(&healthy_env).unwrap(),
            ),
            (
                2_i64,
                poison.hash().0 as i64,
                serde_json::to_vec(&poison).unwrap(),
            ),
        ] {
            conn.execute(
                "INSERT INTO envelopes (hash, uid, mailbox_hash, envelope) VALUES (?1, ?2, ?3, \
                 ?4);",
                rusqlite::params![hash, uid, mailbox_hash.0 as i64, blob],
            )
            .unwrap();
        }
    }
    let mut value =
        sync::sqlite3_cache::Sqlite3Cache::get(Arc::clone(&uid_store), Some(tempdir.path()))
            .unwrap();
    let mut loaded = value.envelopes(mailbox_hash, 2, 100).unwrap().unwrap();
    loaded.sort_unstable();
    let mut expected = vec![
        EnvelopeHash::from_bytes(b"legacy-healthy"),
        EnvelopeHash::from_bytes(b"legacy-poison"),
    ];
    expected.sort_unstable();
    assert_eq!(loaded, expected);
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let quarantined: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM invalid_envelopes WHERE mailbox_hash = ?1;",
            rusqlite::params![mailbox_hash.0 as i64],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(quarantined, 1);
    _ = tempdir.close();
}

#[cfg(feature = "sqlite3")]
#[test]
fn test_imap_sync_sqlite3_quarantine_garbage_blob() {
    use crate::{backends::MailboxHash, email::EnvelopeHash, imap::*, utils::sqlite3::rusqlite};

    // A second class of conversion failure: the envelope column holds a
    // BLOB that is not valid JSON at all (e.g. a torn write). It is
    // quarantined and served as a placeholder just like an address
    // round-trip failure.
    let (tempdir, uid_store, mut value) = sqlite3_cache_handle();
    let mailbox_hash = MailboxHash::from(b"test".as_slice());
    value
        .init_mailbox(mailbox_hash, &SelectResponse::default())
        .unwrap();

    let garbage_hash = EnvelopeHash::from_bytes(b"garbage-marker");
    let garbage_healthy = healthy_envelope("garbage-healthy");
    let fetches = &[fetch_response_for(1, &garbage_healthy)];
    value.insert_envelopes(mailbox_hash, fetches).unwrap();
    {
        // The garbage row cannot come from `insert_envelopes` (it only
        // takes parsed `Envelope` values); inject it with direct SQL.
        let db_path = tempdir.path().join("test_header_cache.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute(
            "INSERT INTO envelopes (hash, uid, mailbox_hash, envelope) VALUES (?1, 2, ?2, ?3);",
            rusqlite::params![
                garbage_hash.0 as i64,
                mailbox_hash.0 as i64,
                b"{not json".as_slice()
            ],
        )
        .unwrap();
    }

    let mut loaded = value.envelopes(mailbox_hash, 2, 100).unwrap().unwrap();
    loaded.sort_unstable();
    let mut expected = vec![healthy_envelope("garbage-healthy").hash(), garbage_hash];
    expected.sort_unstable();
    assert_eq!(loaded, expected);
    let entry = uid_store
        .envelopes
        .lock()
        .unwrap()
        .get(&garbage_hash)
        .map(|entry| entry.inner.subject().into_owned())
        .unwrap();
    assert!(entry.contains("uid 2"));
    _ = tempdir.close();
}

#[cfg(feature = "sqlite3")]
#[test]
fn test_imap_sync_sqlite3_quarantine_refetch_heals() {
    use crate::{backends::MailboxHash, imap::*, utils::sqlite3::rusqlite};

    // A later server refetch of a quarantined uid stores a healthy row
    // with the same hash and heals the quarantine: the placeholder is
    // replaced by the real envelope, exactly once, with no stale
    // quarantine row left behind.
    let (tempdir, uid_store, mut value) = sqlite3_cache_handle();
    let mailbox_hash = MailboxHash::from(b"test".as_slice());
    value
        .init_mailbox(mailbox_hash, &SelectResponse::default())
        .unwrap();

    let healthy_one = healthy_envelope("heal-one");
    // Same marker => same `EnvelopeHash` as the poison row stored, i.e.
    // the hash a server refetch would compute again.
    let poison = poison_envelope("heal-marker");
    let healed = healthy_envelope("heal-marker");
    assert_eq!(poison.hash(), healed.hash());
    let fetches = &[
        fetch_response_for(1, &healthy_one),
        fetch_response_for(2, &poison),
    ];
    value.insert_envelopes(mailbox_hash, fetches).unwrap();

    let mut loaded = value.envelopes(mailbox_hash, 2, 100).unwrap().unwrap();
    loaded.sort_unstable();
    assert_eq!(loaded.len(), 2);
    assert!(loaded.contains(&healed.hash()));

    value
        .insert_envelopes(mailbox_hash, &[fetch_response_for(2, &healed)])
        .unwrap();
    let mut loaded = value.envelopes(mailbox_hash, 2, 100).unwrap().unwrap();
    loaded.sort_unstable();
    assert_eq!(loaded.len(), 2);
    // The uid 2 entry is the real envelope now, not the placeholder.
    let from = uid_store
        .envelopes
        .lock()
        .unwrap()
        .get(&healed.hash())
        .map(|entry| entry.inner.from.first().cloned())
        .unwrap()
        .unwrap();
    assert_eq!(from.get_display_name(), None);
    let db_path = tempdir.path().join("test_header_cache.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let quarantined: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM invalid_envelopes WHERE mailbox_hash = ?1;",
            rusqlite::params![mailbox_hash.0 as i64],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(quarantined, 0);
    let healthy_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM envelopes WHERE mailbox_hash = ?1;",
            rusqlite::params![mailbox_hash.0 as i64],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(healthy_rows, 2);
    _ = tempdir.close();
}

#[cfg(feature = "sqlite3")]
#[test]
fn test_imap_sync_sqlite3_quarantine_cleared_by_rebuild_and_expunge() {
    use crate::{
        backends::{MailboxHash, RefreshEvent, RefreshEventKind},
        imap::*,
        utils::sqlite3::rusqlite,
    };

    let (tempdir, _uid_store, mut value) = sqlite3_cache_handle();
    let mailbox_hash = MailboxHash::from(b"test".as_slice());
    value
        .init_mailbox(mailbox_hash, &SelectResponse::default())
        .unwrap();

    let poison = poison_envelope("clear-poison");
    let clear_healthy = healthy_envelope("clear-healthy");
    let fetches = &[
        fetch_response_for(1, &clear_healthy),
        fetch_response_for(2, &poison),
    ];
    value.insert_envelopes(mailbox_hash, fetches).unwrap();
    let db_path = tempdir.path().join("test_header_cache.db");
    let quarantined_count = || -> i64 {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM invalid_envelopes WHERE mailbox_hash = ?1;",
            rusqlite::params![mailbox_hash.0 as i64],
            |row| row.get(0),
        )
        .unwrap()
    };

    let loaded = value.envelopes(mailbox_hash, 2, 100).unwrap().unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(quarantined_count(), 1);

    // A server-side expunge of the quarantined uid removes its placeholder
    // row as well.
    value
        .update(
            mailbox_hash,
            &[(
                2,
                RefreshEvent {
                    account_hash: AccountHash::from_bytes(b"test".as_slice()),
                    mailbox_hash,
                    kind: RefreshEventKind::Remove(poison.hash()),
                },
            )],
        )
        .unwrap();
    assert_eq!(quarantined_count(), 0);

    // A full mailbox rebuild (e.g. a `UIDVALIDITY` change) ends the
    // quarantine too: re-poisoned rows do not outlive the rebuild.
    value
        .insert_envelopes(mailbox_hash, &[fetch_response_for(2, &poison)])
        .unwrap();
    let loaded = value.envelopes(mailbox_hash, 2, 100).unwrap().unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(quarantined_count(), 1);
    value
        .init_mailbox(mailbox_hash, &SelectResponse::default())
        .unwrap();
    assert_eq!(quarantined_count(), 0);
    let loaded = value.envelopes(mailbox_hash, 2, 100).unwrap().unwrap();
    assert!(loaded.is_empty());
    _ = tempdir.close();
}

#[test]
fn test_imap_sync_status_unchanged() {
    use super::*;

    fn make_status(
        messages: Option<UID>,
        unseen: Option<UID>,
        uidnext: Option<UID>,
        uidvalidity: Option<UID>,
    ) -> protocol_parser::StatusResponse {
        protocol_parser::StatusResponse {
            mailbox: None,
            messages,
            recent: None,
            uidnext,
            uidvalidity,
            unseen,
        }
    }

    let status = make_status(Some(3), Some(3), Some(4), Some(1));
    assert!(status_unchanged(&status, 1, (Some(3), Some(3), Some(4))));
    // UIDVALIDITY mismatch is always a change.
    assert!(!status_unchanged(&status, 2, (Some(3), Some(3), Some(4))));
    assert!(!status_unchanged(
        &make_status(Some(3), Some(3), Some(4), None),
        1,
        (Some(3), Some(3), Some(4))
    ));
    // Any counter mismatch is a change.
    assert!(!status_unchanged(&status, 1, (Some(2), Some(3), Some(4))));
    assert!(!status_unchanged(&status, 1, (Some(3), Some(2), Some(4))));
    assert!(!status_unchanged(&status, 1, (Some(3), Some(3), Some(5))));
    // Missing items (`None`) compare equal to each other.
    assert!(status_unchanged(
        &make_status(Some(3), None, Some(4), Some(1)),
        1,
        (Some(3), None, Some(4))
    ));
    assert!(status_unchanged(
        &make_status(None, None, None, Some(1)),
        1,
        (None, None, None)
    ));
    assert!(!status_unchanged(
        &make_status(Some(3), None, Some(4), Some(1)),
        1,
        (Some(3), Some(3), Some(4))
    ));
}

#[test]
fn test_imap_sync_status_response_item_order() {
    use super::*;

    // `status_response` only accepts items in the order its `permutation`
    // parsers are declared (MESSAGES, RECENT, UIDNEXT, UIDVALIDITY, UNSEEN);
    // this is why the STATUS commands in `resync_basic` request the items in
    // this order.
    let (_, status) = protocol_parser::status_response(
        b"* STATUS INBOX (MESSAGES 3 UIDNEXT 4 UIDVALIDITY 1 UNSEEN 3)\r\n",
    )
    .unwrap();
    assert_eq!(status.messages, Some(3));
    assert_eq!(status.unseen, Some(3));
    assert_eq!(status.uidnext, Some(4));
    assert_eq!(status.uidvalidity, Some(1));

    protocol_parser::status_response(
        b"* STATUS INBOX (MESSAGES 3 UNSEEN 3 UIDNEXT 4 UIDVALIDITY 1)\r\n",
    )
    .unwrap_err();
}
