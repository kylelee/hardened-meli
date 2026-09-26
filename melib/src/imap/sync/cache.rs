/*
 * meli - imap melib
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
    collections::HashMap,
    convert::TryFrom,
    path::Path,
    sync::{Arc, RwLock},
};

use super::*;
use crate::{
    backends::MailboxHash,
    email::{Envelope, EnvelopeHash},
    error::*,
};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ModSequence(pub std::num::NonZeroU64);

impl From<ModSequence> for std::num::NonZeroU64 {
    #[inline]
    fn from(m: ModSequence) -> Self {
        m.0
    }
}

impl TryFrom<i64> for ModSequence {
    type Error = ();
    fn try_from(val: i64) -> std::result::Result<Self, ()> {
        std::num::NonZeroU64::new(val as u64)
            .map(|u| Ok(Self(u)))
            .unwrap_or(Err(()))
    }
}

impl std::fmt::Display for ModSequence {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(fmt, "{}", self.0)
    }
}

#[derive(Debug)]
pub struct CachedEnvelope {
    pub inner: Envelope,
    pub uid: UID,
    pub mailbox_hash: MailboxHash,
    pub modsequence: Option<ModSequence>,
}

#[derive(Clone, Copy, Debug)]
pub struct CachedState {
    pub uidvalidity: UID,
    pub highestmodseq: Option<ModSequence>,
}

/// `(MESSAGES, UNSEEN, UIDNEXT)` counters of a mailbox `STATUS` response.
pub type CachedStatus = (Option<UID>, Option<UID>, Option<UID>);

/// Cached snapshot of an `ImapMailbox`'s identity.
///
/// Message counts (`exists`/`unseen`), the `select` state, the `warm` flag
/// and `permissions` are deliberately not part of the snapshot; they are
/// rebuilt by later `SELECT`/resync operations.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CachedImapMailbox {
    pub hash: MailboxHash,
    pub imap_path: String,
    pub path: String,
    pub name: String,
    pub parent: Option<MailboxHash>,
    pub children: Vec<MailboxHash>,
    pub separator: u8,
    pub usage: SpecialUsageMailbox,
    pub no_select: bool,
    pub is_subscribed: bool,
}

impl From<&ImapMailbox> for CachedImapMailbox {
    fn from(m: &ImapMailbox) -> Self {
        Self {
            hash: m.hash,
            imap_path: m.imap_path.clone(),
            path: m.path.clone(),
            name: m.name.clone(),
            parent: m.parent,
            children: m.children.clone(),
            separator: m.separator,
            usage: *m.usage.read().unwrap_or_else(|e| e.into_inner()),
            no_select: m.no_select,
            is_subscribed: m.is_subscribed,
        }
    }
}

impl From<CachedImapMailbox> for ImapMailbox {
    fn from(m: CachedImapMailbox) -> Self {
        Self {
            hash: m.hash,
            imap_path: m.imap_path,
            path: m.path,
            name: m.name,
            parent: m.parent,
            children: m.children,
            separator: m.separator,
            usage: Arc::new(RwLock::new(m.usage)),
            no_select: m.no_select,
            is_subscribed: m.is_subscribed,
            ..Self::default()
        }
    }
}

/// Helper function for ignoring cache misses with
/// `.or_else(ignore_not_found)?`.
#[inline(always)]
pub fn ignore_not_found(err: Error) -> Result<()> {
    if matches!(err.kind, ErrorKind::NotFound) {
        return Ok(());
    }
    Err(err)
}

pub trait ImapCache: Send + std::fmt::Debug {
    fn reset(&mut self) -> Result<()>;
    fn mailbox_state(&mut self, mailbox_hash: MailboxHash) -> Result<Option<CachedState>>;

    /// Returns the persisted list of mailboxes of the account, if any.
    ///
    /// Message counts, `select` state, the `warm` flag and `permissions`
    /// are not part of the persisted snapshot; they are rebuilt by later
    /// `SELECT`/resync operations.
    fn load_mailbox_list(&mut self) -> Result<Option<HashMap<MailboxHash, ImapMailbox>>>;

    /// Persists the list of mailboxes of the account, replacing any
    /// previously stored list.
    fn save_mailbox_list(&mut self, mailboxes: &HashMap<MailboxHash, ImapMailbox>) -> Result<()>;

    fn lastseenuid(&mut self, mailbox_hash: MailboxHash) -> Result<Option<UID>>;

    /// Returns whether any envelope is persisted for `mailbox_hash`.
    ///
    /// A `mailbox` row (UIDVALIDITY, `max_uid`, STATUS counters) can survive
    /// an interrupted sync while the `envelopes` table is empty; such a
    /// skeleton must not be mistaken for a synchronized mailbox.
    fn has_envelopes(&mut self, mailbox_hash: MailboxHash) -> Result<bool>;

    /// Returns the persisted envelope count of `mailbox_hash`.
    ///
    /// Returns `Ok(None)` when the offline cache is disabled or no mailbox
    /// row exists; `Ok(Some(n))` is the number of persisted envelopes of the
    /// mailbox. A `mailbox` row can survive an interrupted sync while the
    /// `envelopes` table is empty, so this count — not the mere presence of
    /// a row — is the completeness baseline.
    fn count_envelopes(&mut self, mailbox_hash: MailboxHash) -> Result<Option<usize>>;

    /// Records the `(MESSAGES, UNSEEN, UIDNEXT)` counters of a mailbox as
    /// returned by a `STATUS` command, to be used as the baseline of the next
    /// quick synchronization check.
    fn record_status(
        &mut self,
        mailbox_hash: MailboxHash,
        messages: Option<UID>,
        unseen: Option<UID>,
        uidnext: Option<UID>,
    ) -> Result<()>;

    /// Returns the `(MESSAGES, UNSEEN, UIDNEXT)` counters recorded with
    /// [`ImapCache::record_status`], if any.
    fn cached_status(&mut self, mailbox_hash: MailboxHash) -> Result<Option<CachedStatus>>;

    /// Returns the persisted `msn_index` of a mailbox, if it was stored
    /// with the given `UIDVALIDITY`.
    ///
    /// The returned vector is indexed by zero-based message sequence
    /// number, with `None` for missing entries. Storing an empty index
    /// is equivalent to storing nothing (`load` returns `None`).
    fn load_msn_index(
        &mut self,
        mailbox_hash: MailboxHash,
        uidvalidity: UIDVALIDITY,
    ) -> Result<Option<Vec<Option<UID>>>>;

    /// Persists the `msn_index` of a mailbox for the given
    /// `UIDVALIDITY`, replacing any previously stored index of the
    /// mailbox.
    fn store_msn_index(
        &mut self,
        mailbox_hash: MailboxHash,
        uidvalidity: UIDVALIDITY,
        msn_index: &[Option<UID>],
    ) -> Result<()>;

    fn find_envelope(
        &mut self,
        identifier: std::result::Result<UID, EnvelopeHash>,
        mailbox_hash: MailboxHash,
    ) -> Result<Option<CachedEnvelope>>;

    fn update(
        &mut self,
        mailbox_hash: MailboxHash,
        refresh_events: &[(UID, RefreshEvent)],
    ) -> Result<()>;

    fn update_mailbox(
        &mut self,
        mailbox_hash: MailboxHash,
        select_response: &SelectResponse,
    ) -> Result<()>;

    fn insert_envelopes(
        &mut self,
        mailbox_hash: MailboxHash,
        fetches: &[FetchResponse<'_>],
    ) -> Result<()>;

    /// Returns the newest `batch_size` cached envelopes with
    /// `uid <= lastseenuid`, in descending UID order, together with the
    /// lowest UID of the returned page (`None` when the page is empty).
    ///
    /// The lowest-page-UID lets callers page by actual rows instead of by
    /// UID-window arithmetic: long-lived IMAP servers (Coremail / 网易 163,
    /// `UIDVALIDITY = 1`, `uidnext` in the 10^8-10^9 range) have extremely
    /// sparse UID spaces where stepping `max_uid -= batch_size` iterates
    /// hundreds of thousands of (near-)empty windows before terminating.
    fn envelopes(
        &mut self,
        mailbox_hash: MailboxHash,
        lastseenuid: UID,
        batch_size: usize,
    ) -> Result<Option<(Vec<EnvelopeHash>, Option<UID>)>>;

    fn init_mailbox(
        &mut self,
        mailbox_hash: MailboxHash,
        select_response: &SelectResponse,
    ) -> Result<()>;

    fn update_flags(
        &mut self,
        env_hashes: EnvelopeHashBatch,
        mailbox_hash: MailboxHash,
        flags: Vec<FlagOp>,
    ) -> Result<()>;
}

pub trait ImapCacheReset: Send + std::fmt::Debug {
    fn reset_db(uid_store: &UIDStore, data_dir: Option<&Path>) -> Result<()>
    where
        Self: Sized;
}

impl ImapCache for Arc<UIDStore> {
    fn reset(&mut self) -> Result<()> {
        if !self.keep_offline_cache.load(Ordering::SeqCst) {
            return Ok(());
        }
        #[cfg(feature = "sqlite3")]
        {
            sync::sqlite3_cache::Sqlite3Cache::reset_db(self, None)?;
        }
        Ok(())
    }

    fn mailbox_state(&mut self, mailbox_hash: MailboxHash) -> Result<Option<CachedState>> {
        if !self.keep_offline_cache.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let mut mutex = self.offline_cache.lock().unwrap();
        self.init_cache(&mut mutex)?;

        if let Some(ref mut cache_handle) = *mutex {
            return cache_handle.mailbox_state(mailbox_hash);
        }
        Ok(None)
    }

    fn load_mailbox_list(&mut self) -> Result<Option<HashMap<MailboxHash, ImapMailbox>>> {
        if !self.keep_offline_cache.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let mut mutex = self.offline_cache.lock().unwrap();
        self.init_cache(&mut mutex)?;

        if let Some(ref mut cache_handle) = *mutex {
            return cache_handle.load_mailbox_list();
        }
        Ok(None)
    }

    fn save_mailbox_list(&mut self, mailboxes: &HashMap<MailboxHash, ImapMailbox>) -> Result<()> {
        if !self.keep_offline_cache.load(Ordering::SeqCst) {
            return Ok(());
        }
        let mut mutex = self.offline_cache.lock().unwrap();
        self.init_cache(&mut mutex)?;

        if let Some(ref mut cache_handle) = *mutex {
            return cache_handle.save_mailbox_list(mailboxes);
        }
        Ok(())
    }

    fn lastseenuid(&mut self, mailbox_hash: MailboxHash) -> Result<Option<UID>> {
        if !self.keep_offline_cache.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let mut mutex = self.offline_cache.lock().unwrap();
        self.init_cache(&mut mutex)?;

        if let Some(ref mut cache_handle) = *mutex {
            return cache_handle.lastseenuid(mailbox_hash);
        }
        Ok(None)
    }

    fn has_envelopes(&mut self, mailbox_hash: MailboxHash) -> Result<bool> {
        if !self.keep_offline_cache.load(Ordering::SeqCst) {
            return Ok(false);
        }
        let mut mutex = self.offline_cache.lock().unwrap();
        self.init_cache(&mut mutex)?;

        if let Some(ref mut cache_handle) = *mutex {
            return cache_handle.has_envelopes(mailbox_hash);
        }
        Ok(false)
    }

    fn count_envelopes(&mut self, mailbox_hash: MailboxHash) -> Result<Option<usize>> {
        if !self.keep_offline_cache.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let mut mutex = self.offline_cache.lock().unwrap();
        self.init_cache(&mut mutex)?;

        if let Some(ref mut cache_handle) = *mutex {
            return cache_handle.count_envelopes(mailbox_hash);
        }
        Ok(None)
    }

    fn record_status(
        &mut self,
        mailbox_hash: MailboxHash,
        messages: Option<UID>,
        unseen: Option<UID>,
        uidnext: Option<UID>,
    ) -> Result<()> {
        if !self.keep_offline_cache.load(Ordering::SeqCst) {
            return Ok(());
        }
        let mut mutex = self.offline_cache.lock().unwrap();
        self.init_cache(&mut mutex)?;

        if let Some(ref mut cache_handle) = *mutex {
            return cache_handle.record_status(mailbox_hash, messages, unseen, uidnext);
        }
        Ok(())
    }

    fn cached_status(&mut self, mailbox_hash: MailboxHash) -> Result<Option<CachedStatus>> {
        if !self.keep_offline_cache.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let mut mutex = self.offline_cache.lock().unwrap();
        self.init_cache(&mut mutex)?;

        if let Some(ref mut cache_handle) = *mutex {
            return cache_handle.cached_status(mailbox_hash);
        }
        Ok(None)
    }

    fn load_msn_index(
        &mut self,
        mailbox_hash: MailboxHash,
        uidvalidity: UIDVALIDITY,
    ) -> Result<Option<Vec<Option<UID>>>> {
        if !self.keep_offline_cache.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let mut mutex = self.offline_cache.lock().unwrap();
        self.init_cache(&mut mutex)?;

        if let Some(ref mut cache_handle) = *mutex {
            return cache_handle.load_msn_index(mailbox_hash, uidvalidity);
        }
        Ok(None)
    }

    fn store_msn_index(
        &mut self,
        mailbox_hash: MailboxHash,
        uidvalidity: UIDVALIDITY,
        msn_index: &[Option<UID>],
    ) -> Result<()> {
        if !self.keep_offline_cache.load(Ordering::SeqCst) {
            return Ok(());
        }
        let mut mutex = self.offline_cache.lock().unwrap();
        self.init_cache(&mut mutex)?;

        if let Some(ref mut cache_handle) = *mutex {
            return cache_handle.store_msn_index(mailbox_hash, uidvalidity, msn_index);
        }
        Ok(())
    }

    fn find_envelope(
        &mut self,
        identifier: std::result::Result<UID, EnvelopeHash>,
        mailbox_hash: MailboxHash,
    ) -> Result<Option<CachedEnvelope>> {
        if !self.keep_offline_cache.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let mut mutex = self.offline_cache.lock().unwrap();
        self.init_cache(&mut mutex)?;

        if let Some(ref mut cache_handle) = *mutex {
            return cache_handle.find_envelope(identifier, mailbox_hash);
        }
        Ok(None)
    }

    fn update(
        &mut self,
        mailbox_hash: MailboxHash,
        refresh_events: &[(UID, RefreshEvent)],
    ) -> Result<()> {
        if !self.keep_offline_cache.load(Ordering::SeqCst) {
            return Ok(());
        }
        let mut mutex = self.offline_cache.lock().unwrap();
        self.init_cache(&mut mutex)?;

        if let Some(ref mut cache_handle) = *mutex {
            return cache_handle.update(mailbox_hash, refresh_events);
        }
        Ok(())
    }

    fn update_mailbox(
        &mut self,
        mailbox_hash: MailboxHash,
        select_response: &SelectResponse,
    ) -> Result<()> {
        if !self.keep_offline_cache.load(Ordering::SeqCst) {
            return Ok(());
        }
        let mut mutex = self.offline_cache.lock().unwrap();
        self.init_cache(&mut mutex)?;

        if let Some(ref mut cache_handle) = *mutex {
            return cache_handle.update_mailbox(mailbox_hash, select_response);
        }
        Ok(())
    }

    fn insert_envelopes(
        &mut self,
        mailbox_hash: MailboxHash,
        fetches: &[FetchResponse<'_>],
    ) -> Result<()> {
        if !self.keep_offline_cache.load(Ordering::SeqCst) {
            return Ok(());
        }
        let mut mutex = self.offline_cache.lock().unwrap();
        self.init_cache(&mut mutex)?;

        if let Some(ref mut cache_handle) = *mutex {
            cache_handle.insert_envelopes(mailbox_hash, fetches)?;
        }
        let mut env_lck = self.envelopes.lock().unwrap();
        let mut hash_index_lck = self.hash_index.lock().unwrap();
        let mut uid_index_lck = self.uid_index.lock().unwrap();
        let mut msn_index_lck = self.msn_index.lock().unwrap();
        for item in fetches {
            if let FetchResponse {
                uid: Some(uid),
                message_sequence_number,
                modseq,
                flags: _,
                body: _,
                references: _,
                envelope: Some(env),
                raw_fetch_value: _,
                bodystructure: _,
            } = item
            {
                let uid = *uid;
                let modseq = *modseq;
                msn_index_lck
                    .entry(mailbox_hash)
                    .or_default()
                    .insert(message_sequence_number.saturating_sub(1), uid);
                hash_index_lck.insert(env.hash(), (uid, mailbox_hash));
                uid_index_lck.insert((mailbox_hash, uid), env.hash());
                env_lck.insert(
                    env.hash(),
                    CachedEnvelope {
                        inner: env.clone(),
                        uid,
                        mailbox_hash,
                        modsequence: modseq,
                    },
                );
            }
        }
        Ok(())
    }

    fn envelopes(
        &mut self,
        mailbox_hash: MailboxHash,
        lastseenuid: UID,
        batch_size: usize,
    ) -> Result<Option<(Vec<EnvelopeHash>, Option<UID>)>> {
        if !self.keep_offline_cache.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let mut mutex = self.offline_cache.lock().unwrap();
        self.init_cache(&mut mutex)?;

        if let Some(ref mut cache_handle) = *mutex {
            return cache_handle.envelopes(mailbox_hash, lastseenuid, batch_size);
        }
        Ok(None)
    }

    fn init_mailbox(
        &mut self,
        mailbox_hash: MailboxHash,
        select_response: &SelectResponse,
    ) -> Result<()> {
        if !self.keep_offline_cache.load(Ordering::SeqCst) {
            return Ok(());
        }
        let mut mutex = self.offline_cache.lock().unwrap();
        self.init_cache(&mut mutex)?;

        if let Some(ref mut cache_handle) = *mutex {
            cache_handle.init_mailbox(mailbox_hash, select_response)?;
        }
        // The mailbox's persisted rows were just dropped, so the in-memory
        // envelope map must not outlive them: otherwise `FreshFetch`'s
        // resume filter (which skips UIDs already present in this map)
        // would skip messages that no longer exist in the cache, leaving
        // the rebuild empty. `uid_index` is deliberately left intact so
        // that already-known UIDs are still deduplicated when Create
        // events are emitted after the rebuild.
        self.envelopes
            .lock()
            .unwrap()
            .retain(|_, cached| cached.mailbox_hash != mailbox_hash);
        Ok(())
    }

    fn update_flags(
        &mut self,
        env_hashes: EnvelopeHashBatch,
        mailbox_hash: MailboxHash,
        flags: Vec<FlagOp>,
    ) -> Result<()> {
        if !self.keep_offline_cache.load(Ordering::SeqCst) {
            return Ok(());
        }
        let mut mutex = self.offline_cache.lock().unwrap();
        self.init_cache(&mut mutex)?;

        if let Some(ref mut cache_handle) = *mutex {
            return cache_handle.update_flags(env_hashes, mailbox_hash, flags);
        }
        Ok(())
    }
}
