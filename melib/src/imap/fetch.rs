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

use super::*;
use imap_codec::imap_types::search::SearchKey;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FetchStage {
    #[default]
    InitialFresh,
    InitialCache,
    FromCache {
        max_uid: UID,
        batch: usize,
    },
    /// Serve the offline cache before any network round-trip
    /// (stale-while-revalidate); `max_uid` is a placeholder until the
    /// first `chunk()` call corrects it with `lastseenuid()`.
    CacheFirst {
        max_uid: UID,
        batch: usize,
    },
    ResyncCache,
    /// Fresh-fetch from the server using a real UID list.
    ///
    /// Walking `UID FETCH min..max` downward from `uidnext` is
    /// `O(uidnext / batch_size)` round-trips and stalls on long-lived
    /// mailboxes whose UID space is sparse but reaches
    /// `uidnext ≈ 1.3×10^9` (Coremail / Netease 163 with
    /// `UIDVALIDITY = 1`): ≈528 000 round-trips at `batch_size = 2500`,
    /// i.e. effectively a hang. Issuing one `UID SEARCH ALL` in the
    /// [`FetchStage::InitialFresh`] stage instead lets us batch by the
    /// **actual** mail count, not by the UID value. The UID list and
    /// cursor live on [`FetchState`] (not in this enum) so this variant
    /// stays `Copy`.
    FreshFetch,
    Finished,
}

#[derive(Debug)]
pub struct FetchState {
    pub stage: FetchStage,
    pub connection: Arc<ConnectionMutex>,
    pub mailbox_hash: MailboxHash,
    pub uid_store: Arc<UIDStore>,
    pub batch_size: usize,
    pub cache_batch_size: usize,
    pub response: Vec<u8>,
    /// Whether the `CacheFirst` stage already emitted the cached
    /// envelopes to the stream. Later stages must not serve them again.
    pub cache_served_offline: bool,
    /// Server-reported message count (`EXISTS`) of the most recent
    /// `SELECT`/`EXAMINE`. The cache-completeness check in
    /// [`Self::cache_is_incomplete`] uses it to decide whether a cache
    /// whose envelope count is below it must fall back to a full fresh
    /// fetch, while a genuinely empty server mailbox must not.
    pub last_select_exists: UID,
    /// `UIDVALIDITY` of the most recent `SELECT`/`EXAMINE`, paired with
    /// [`Self::last_select_exists`]. A cached envelope count may only be
    /// compared against the server `EXISTS` when both describe the same
    /// cache generation; a stale `UIDVALIDITY` must not trigger a
    /// rebuild.
    pub last_select_uidvalidity: UIDVALIDITY,
    /// `UIDNEXT` of the most recent `SELECT`/`EXAMINE`. `0` means the
    /// server omitted it (unknown). Used by
    /// [`Self::cache_is_incomplete`] to tell a deficit the incremental
    /// `UID FETCH lastseenuid+1:*` can recover from one that sits below
    /// `lastseenuid` and needs a full rebuild.
    pub last_select_uidnext: UID,
    /// Real UID list collected by `UID SEARCH ALL` in
    /// [`FetchStage::InitialFresh`]. Consumed by the
    /// [`FetchStage::FreshFetch`] stage in batches of `batch_size`.
    pub fresh_fetch_uids: Vec<UID>,
    /// Cursor into [`Self::fresh_fetch_uids`]. Advanced by every
    /// `UID FETCH` batch the [`FetchStage::FreshFetch`] stage emits.
    pub fresh_fetch_next: usize,
}

impl FetchState {
    pub async fn chunk(&mut self) -> Result<Vec<Envelope>> {
        loop {
            match self.stage {
                FetchStage::InitialFresh => {
                    let select_response = self
                        .connection
                        .lock()
                        .await?
                        .init_mailbox(self.mailbox_hash)
                        .await?;
                    _ = self
                        .uid_store
                        .update_mailbox(self.mailbox_hash, &select_response);
                    self.last_select_exists = select_response.exists;
                    self.last_select_uidvalidity = select_response.uidvalidity;
                    self.last_select_uidnext = select_response.uidnext;

                    if select_response.exists == 0 {
                        self.stage = FetchStage::Finished;
                        return Ok(Vec::new());
                    }

                    // Long-lived IMAP servers (e.g. Coremail / Netease 163
                    // with `UIDVALIDITY = 1` and `uidnext ≈ 1.3×10^9`
                    // but only a handful of real messages) are sparse:
                    // walking `UID FETCH min..max` downward from
                    // `uidnext` in `batch_size` steps would take roughly
                    // `uidnext / batch_size` round-trips (≈528 000 at
                    // `batch_size = 2500`) — effectively a hang.
                    //
                    // Issue one `UID SEARCH ALL` to enumerate the real
                    // messages once and then fetch them in batches by
                    // explicit UID list. The cost is now proportional to
                    // the real mail count, not to the UID value.
                    let mut conn = self.connection.lock().await?;
                    let mut search_resp = Vec::new();
                    conn.send_command(CommandBody::search(None, SearchKey::All.into(), true))
                        .await?;
                    conn.read_response(&mut search_resp, RequiredResponses::SEARCH)
                        .await?;
                    drop(conn);
                    let (_, uids) = protocol_parser::search_results(search_resp.as_slice())?;
                    // Resume instead of restarting: a previous attempt may
                    // have fetched and cached part of this mailbox before
                    // the connection died, in which case the retry would
                    // otherwise start over from the first UID. Filter by
                    // the in-memory envelope map (the persisted cache
                    // mirror, populated by cache reads and
                    // `insert_envelopes` and cleared when the mailbox is
                    // wiped), not by `uid_index`: the latter also holds
                    // known-but-not-cached UIDs used for Create-event
                    // dedup, so it would skip messages a rebuild still
                    // needs to fetch. This only applies when the offline
                    // cache is active: the cache stages then serve those
                    // envelopes to the caller. Without an offline cache
                    // the fresh fetch is the only source of envelopes, so
                    // nothing may be skipped.
                    let offline_cache_active =
                        self.uid_store.keep_offline_cache.load(Ordering::SeqCst);
                    let uids: Vec<UID> = if offline_cache_active {
                        let cached_uids: std::collections::HashSet<UID> = self
                            .uid_store
                            .envelopes
                            .lock()
                            .unwrap()
                            .values()
                            .filter(|cached| cached.mailbox_hash == self.mailbox_hash)
                            .map(|cached| cached.uid)
                            .collect();
                        uids.into_iter()
                            .filter(|uid| !cached_uids.contains(uid))
                            .collect()
                    } else {
                        uids
                    };
                    if uids.is_empty() {
                        self.stage = FetchStage::Finished;
                        return Ok(Vec::new());
                    }
                    self.fresh_fetch_uids = uids;
                    self.fresh_fetch_next = 0;
                    self.stage = FetchStage::FreshFetch;
                    continue;
                }
                FetchStage::InitialCache => {
                    let select_response = self
                        .connection
                        .lock()
                        .await?
                        .select_mailbox(self.mailbox_hash, &mut self.response, false)
                        .await?;
                    self.last_select_exists = select_response.exists;
                    self.last_select_uidvalidity = select_response.uidvalidity;
                    self.last_select_uidnext = select_response.uidnext;
                    if let Err(err) = self
                        .uid_store
                        .update_mailbox(self.mailbox_hash, &select_response)
                        .chain_err_summary(|| {
                            format!("Could not update cache for mailbox {}.", self.mailbox_hash)
                        })
                    {
                        (self.uid_store.event_consumer)(self.uid_store.account_hash, err.into());
                    }
                    match self.lastseenuid() {
                        Ok(Some(max_uid)) => {
                            self.stage = FetchStage::FromCache { max_uid, batch: 0 };
                            continue;
                        }
                        Ok(None) => {}
                        Err(err) => {
                            imap_log!(
                                error,
                                self.connection.lock().await?,
                                "IMAP cache error: could not fetch cache for {}. Reason: {}",
                                self.uid_store.account_name,
                                err
                            );
                            // Try resetting the database
                            if let Err(err) = self.uid_store.reset() {
                                imap_log!(
                                    error,
                                    self.connection.lock().await?,
                                    "IMAP cache error: could not reset cache for {}. Reason: {}",
                                    self.uid_store.account_name,
                                    err
                                );
                            }
                        }
                    }
                    self.stage = FetchStage::InitialFresh;
                    continue;
                }
                FetchStage::FromCache { max_uid, batch } => {
                    let cache_batch_size = if batch == 0 {
                        500
                    } else {
                        self.cache_batch_size
                    };
                    let res = self.cached_envs(max_uid, cache_batch_size).await;
                    match res {
                        Ok(Some((mut cached_payload, lowest_served))) => {
                            // Page-based advance: the next page starts just
                            // below this page's lowest served UID. Walking
                            // by UID-window subtraction instead would need
                            // O(uidnext / batch_size) iterations on the
                            // sparse UID spaces of long-lived servers
                            // (Coremail / 网易 163, uidnext in the 10^8-10^9
                            // range), nearly all of them empty.
                            self.stage = match lowest_served {
                                Some(lowest) if lowest > 1 => FetchStage::FromCache {
                                    max_uid: lowest - 1,
                                    batch: batch + 1,
                                },
                                _ => FetchStage::Finished,
                            };
                            // Cache-completeness invariant: once the online
                            // `InitialCache` -> `FromCache` walk is
                            // exhausted, the persisted envelope count must
                            // cover the server's `EXISTS`. If it does not
                            // (nothing or only part of the history served),
                            // ending here would render a silently
                            // incomplete listing. Rebuild the mailbox with a
                            // full fresh fetch instead. A genuinely complete
                            // cache and an empty server mailbox
                            // (`last_select_exists == 0`) still finish
                            // normally.
                            if self.stage == FetchStage::Finished && self.cache_is_incomplete() {
                                self.stage = FetchStage::InitialFresh;
                            }
                            let (mailbox_exists, unseen) = {
                                let f = &self.uid_store.mailboxes.lock().await[&self.mailbox_hash];
                                (Arc::clone(&f.exists), Arc::clone(&f.unseen))
                            };
                            unseen.lock().unwrap().insert_existing_set(
                                cached_payload
                                    .iter()
                                    .filter_map(|env| {
                                        if !env.is_seen() {
                                            Some(env.hash())
                                        } else {
                                            None
                                        }
                                    })
                                    .collect(),
                            );
                            mailbox_exists.lock().unwrap().insert_existing_set(
                                cached_payload.iter().map(|env| env.hash()).collect::<_>(),
                            );
                            // The cache served first for fast UX; when the
                            // cache batches are exhausted, one final resync
                            // reconciles the payload and the unseen/exists
                            // sets with the server's truth, so a mail that
                            // was deleted on the server does not survive
                            // as a ghost. (Semantic port of upstream meli
                            // 4f2414a3 "fetch from cache then resync".)
                            if self.stage == FetchStage::Finished {
                                let mut conn = self.connection.lock().await?;
                                match conn.resync(self.mailbox_hash).await {
                                    Ok(Some(payload)) => {
                                        unseen.lock().unwrap().insert_existing_set(
                                            payload
                                                .iter()
                                                .filter_map(|env| {
                                                    if !env.is_seen() {
                                                        Some(env.hash())
                                                    } else {
                                                        None
                                                    }
                                                })
                                                .collect(),
                                        );
                                        mailbox_exists.lock().unwrap().insert_existing_set(
                                            payload.iter().map(|env| env.hash()).collect::<_>(),
                                        );
                                        cached_payload.extend(payload);
                                    }
                                    Ok(None) => {}
                                    Err(err) => {
                                        // Keep the graceful degradation of
                                        // the cached payload: log and
                                        // finish without failing the
                                        // stream.
                                        imap_log!(
                                            error,
                                            conn,
                                            "IMAP error: could not resync {} after serving \
                                             the cache. Reason: {}",
                                            self.uid_store.account_name,
                                            err
                                        );
                                    }
                                }
                            }
                            return Ok(cached_payload);
                        }
                        Err(err) => {
                            imap_log!(
                                error,
                                self.connection.lock().await?,
                                "IMAP cache error: could not fetch cache for {}. Reason: {}",
                                self.uid_store.account_name,
                                err
                            );
                            // Try resetting the database
                            if let Err(err) = self.uid_store.reset() {
                                imap_log!(
                                    error,
                                    self.connection.lock().await?,
                                    "IMAP cache error: could not reset cache for {}. Reason: {}",
                                    self.uid_store.account_name,
                                    err
                                );
                            }
                            self.stage = FetchStage::InitialFresh;
                            continue;
                        }
                        Ok(None) => {
                            self.stage = FetchStage::InitialFresh;
                            continue;
                        }
                    }
                }
                FetchStage::CacheFirst { max_uid, batch } => {
                    // stale-while-revalidate: serve the offline cache
                    // before any network round-trip so the UI can render
                    // immediately; the `ResyncCache` stage that follows
                    // reconciles with the server.
                    let max_uid = if batch == 0 {
                        match self.lastseenuid() {
                            Ok(Some(max_uid)) => max_uid,
                            Ok(None) => {
                                // A persisted `mailbox` skeleton (`mailbox_state`
                                // is `Ok(Some)`) with no `max_uid` and no
                                // envelopes is the residue of an interrupted
                                // sync: there is no incremental resync baseline
                                // and nothing for the online cache stages to
                                // serve. Rebuild the mailbox with a full fresh
                                // fetch. `InitialFresh` re-initializes this
                                // mailbox's own row, so no whole-store `reset()`
                                // is needed (it would needlessly drop every
                                // other mailbox's offline cache).
                                if matches!(
                                    self.uid_store.mailbox_state(self.mailbox_hash),
                                    Ok(Some(_))
                                ) && !self.uid_store.has_envelopes(self.mailbox_hash)?
                                {
                                    self.stage = FetchStage::InitialFresh;
                                    continue;
                                }
                                self.stage = FetchStage::ResyncCache;
                                continue;
                            }
                            Err(err) => {
                                log::error!(
                                    "{} IMAP cache error: could not fetch cache. Reason: {}",
                                    self.uid_store.account_name,
                                    err
                                );
                                // A failed cache read can leave a poisoned
                                // skeleton behind: a `mailbox` row with
                                // STATUS counters/`max_uid` but no envelopes.
                                // Reset it and rebuild from the server. A
                                // mailbox that is merely not cached yet
                                // (`mailbox_state` is `Ok(None)`) is not an
                                // error: it falls through to `ResyncCache`,
                                // which walks the cache stages and ends at
                                // `InitialFresh`.
                                match self.uid_store.mailbox_state(self.mailbox_hash) {
                                    Ok(None) => {}
                                    _ => {
                                        if let Err(err) = self.uid_store.reset() {
                                            log::error!(
                                                "{} IMAP cache error: could not reset cache. \
                                                 Reason: {}",
                                                self.uid_store.account_name,
                                                err
                                            );
                                        }
                                        self.stage = FetchStage::InitialFresh;
                                        continue;
                                    }
                                }
                                self.stage = FetchStage::ResyncCache;
                                continue;
                            }
                        }
                    } else {
                        max_uid
                    };
                    let cache_batch_size = if batch == 0 {
                        500
                    } else {
                        self.cache_batch_size
                    };
                    let res = self.cached_envs_offline(max_uid, cache_batch_size).await;
                    match res {
                        Ok(Some((cached_payload, lowest_served))) => {
                            if batch == 0 && cached_payload.is_empty() {
                                // Poisoned cache: the persisted `envelopes`
                                // table is empty while the `mailbox` skeleton
                                // (row, STATUS counters, `max_uid`) survived
                                // an interrupted sync. Serving this empty
                                // snapshot and letting the incremental resync
                                // follow would short-circuit to an empty
                                // mailbox forever; only the fresh fetch can
                                // backfill the missing history. Nothing was
                                // served, so there is no duplicate-emission
                                // risk.
                                self.stage = FetchStage::InitialFresh;
                                continue;
                            }
                            // Page-based advance (see the `FromCache` stage):
                            // the next page starts just below this page's
                            // lowest served UID, so the walk takes
                            // O(cached_rows / batch_size) queries however
                            // sparse the UID space is.
                            self.stage = match lowest_served {
                                Some(lowest) if lowest > 1 => FetchStage::CacheFirst {
                                    max_uid: lowest - 1,
                                    batch: batch + 1,
                                },
                                _ => FetchStage::ResyncCache,
                            };
                            self.cache_served_offline = true;
                            let (mailbox_exists, unseen) = {
                                let f = &self.uid_store.mailboxes.lock().await[&self.mailbox_hash];
                                (Arc::clone(&f.exists), Arc::clone(&f.unseen))
                            };
                            unseen.lock().unwrap().insert_existing_set(
                                cached_payload
                                    .iter()
                                    .filter_map(|env| {
                                        if !env.is_seen() {
                                            Some(env.hash())
                                        } else {
                                            None
                                        }
                                    })
                                    .collect(),
                            );
                            mailbox_exists.lock().unwrap().insert_existing_set(
                                cached_payload.iter().map(|env| env.hash()).collect::<_>(),
                            );
                            return Ok(cached_payload);
                        }
                        Ok(None) => {
                            self.stage = FetchStage::ResyncCache;
                            continue;
                        }
                        Err(err) => {
                            log::error!(
                                "{} IMAP cache error: could not fetch cache. Reason: {}",
                                self.uid_store.account_name,
                                err
                            );
                            // A failed offline cache read can leave a
                            // poisoned skeleton behind; reset it and rebuild
                            // from the server instead of letting the
                            // incremental resync short-circuit to an empty
                            // listing.
                            if let Err(err) = self.uid_store.reset() {
                                log::error!(
                                    "{} IMAP cache error: could not reset cache. Reason: {}",
                                    self.uid_store.account_name,
                                    err
                                );
                            }
                            self.stage = FetchStage::InitialFresh;
                            continue;
                        }
                    }
                }
                FetchStage::ResyncCache => {
                    let mut conn = self.connection.lock().await?;
                    let select_response = match conn.init_mailbox(self.mailbox_hash).await {
                        Ok(select_response) => select_response,
                        Err(err)
                            if self.cache_served_offline
                                && (err.kind.is_network()
                                    || err.kind.is_timeout()
                                    || err.kind.is_oserror()) =>
                        {
                            // The offline cache was already emitted by the
                            // `CacheFirst` stage; a network-ish failure to
                            // reach the server ends the stream gracefully
                            // instead of failing the mailbox. All other
                            // errors (authentication, protocol, etc.)
                            // propagate.
                            imap_log!(
                                error,
                                conn,
                                "IMAP error: could not connect to resync {} after serving the \
                             offline cache. Reason: {}",
                                self.uid_store.account_name,
                                err
                            );
                            self.stage = FetchStage::Finished;
                            return Ok(Vec::new());
                        }
                        Err(err) => return Err(err),
                    };
                    self.last_select_exists = select_response.exists;
                    self.last_select_uidvalidity = select_response.uidvalidity;
                    self.last_select_uidnext = select_response.uidnext;
                    // Cache-completeness invariant: if the persisted
                    // envelope count does not cover this `SELECT`'s `EXISTS`
                    // and the incremental `UID FETCH lastseenuid+1:*` cannot
                    // recover the deficit (e.g. the server-side fetch window
                    // widened and revealed UIDs below `lastseenuid`), the
                    // cache walk above served an incomplete snapshot.
                    // Rebuild with a full fresh fetch instead of letting the
                    // subsequent resync short-circuit to a partial listing.
                    // The account layer dedups re-emitted envelopes by hash
                    // (`Collection::merge`). This must run before the resync
                    // below, which wipes the mailbox row (and, via the
                    // cascading foreign key, its envelopes), leaving nothing
                    // for a later check to measure.
                    if self.cache_is_incomplete() {
                        self.stage = FetchStage::InitialFresh;
                        continue;
                    }
                    let mut resync_res: Option<Vec<Envelope>> = None;
                    match self
                        .uid_store
                        .update_mailbox(self.mailbox_hash, &select_response)
                    {
                        Err(err) if err.kind.is_not_found() => {
                            _ = self
                                .uid_store
                                .init_mailbox(self.mailbox_hash, &select_response);
                        }
                        Err(err) => {
                            (self.uid_store.event_consumer)(
                                self.uid_store.account_hash,
                                err.set_summary(format!(
                                    "Could not update cache for mailbox {}.",
                                    self.mailbox_hash
                                ))
                                .into(),
                            );
                        }
                        Ok(()) => {
                            // Only the offline-served path consumes a
                            // resync result here (it emits what changed
                            // since the served cache snapshot). The online
                            // path must not resync before the cache is
                            // walked: it re-walks the cache stages below
                            // and performs one final resync when the cache
                            // batches finish, so the cache serves first
                            // and the server's truth corrects the
                            // payload/sets last. (Semantic port of
                            // upstream meli 4f2414a3.)
                            if self.cache_served_offline {
                                let mailbox_hash = self.mailbox_hash;
                                match conn.resync(mailbox_hash).await {
                                    Ok(Some(payload)) => resync_res = Some(payload),
                                    Ok(None) => {}
                                    Err(err) => {
                                        imap_log!(
                                            error,
                                            conn,
                                            "IMAP error: could not resync {} after serving \
                                             the offline cache. Reason: {}",
                                            self.uid_store.account_name,
                                            err
                                        );
                                    }
                                }
                            }
                        }
                    }
                    if self.cache_served_offline {
                        // The cached envelopes were already emitted by the
                        // `CacheFirst` stage: emit only what the resync
                        // fetched (if anything) and finish. Falling
                        // through to `InitialCache`/`FromCache` (or the
                        // `InitialFresh` fresh fetch fallback) would emit
                        // the same envelopes a second time.
                        self.stage = FetchStage::Finished;
                        return Ok(resync_res.unwrap_or_default());
                    }
                    // Online: re-walk the cache stages (cache serves
                    // first); the final resync at the end of the cache
                    // batches reconciles with the server.
                    self.stage = FetchStage::InitialCache;
                    continue;
                }
                FetchStage::FreshFetch => {
                    // The UID list and cursor live on `self` (see
                    // [`FetchState::fresh_fetch_uids`] /
                    // [`FetchState::fresh_fetch_next`]) so this
                    // variant can stay `Copy`. We avoid
                    // `let Self { ... } = self` here because the
                    // mix of `ref mut` fields and Copy fields on
                    // `&mut Self` confuses partial-move analysis.
                    let mailbox_hash = self.mailbox_hash;
                    let batch_size = self.batch_size;
                    let connection = Arc::clone(&self.connection);
                    let uid_store = Arc::clone(&self.uid_store);
                    let uids = &mut self.fresh_fetch_uids;
                    let next = &mut self.fresh_fetch_next;
                    let response = &mut self.response;
                    let mut our_unseen: BTreeSet<EnvelopeHash> = BTreeSet::default();
                    let (mailbox_path, mailbox_exists, no_select, unseen) = {
                        let f = &uid_store.mailboxes.lock().await[&mailbox_hash];
                        (
                            f.imap_path().to_string(),
                            Arc::clone(&f.exists),
                            f.no_select,
                            Arc::clone(&f.unseen),
                        )
                    };
                    if no_select {
                        self.stage = FetchStage::Finished;
                        return Ok(Vec::new());
                    }

                    // Slice the next `batch_size` real UIDs from the
                    // SEARCH-driven list. A server reply that references
                    // a non-existent UID is legal — the server just
                    // returns no FETCH for it — so we don't need to
                    // guard against stale UIDs.
                    let (take, finished) = {
                        let remaining = uids.len().saturating_sub(*next);
                        if remaining == 0 {
                            unseen.lock().unwrap().set_not_yet_seen(0);
                            mailbox_exists.lock().unwrap().set_not_yet_seen(0);
                            self.stage = FetchStage::Finished;
                            return Ok(Vec::new());
                        }
                        (
                            remaining.min(batch_size),
                            *next + remaining.min(batch_size) >= uids.len(),
                        )
                    };
                    let to_fetch: &[UID] = &uids[*next..*next + take];

                    // Build an explicit UID SequenceSet (`5,1320000000`).
                    // `NonZeroU32` rejects 0 (RFC 3501 §2.3.1.1:
                    // uid ranges are positive) and the `UID SEARCH`
                    // parser only yields positive values, so the only
                    // failure modes are an unexpected 0 (server bug) or
                    // a UID above `u32::MAX` (also a server bug); both
                    // are surfaced as `ErrorKind::Bug`.
                    let nz: Vec<NonZeroU32> = to_fetch
                        .iter()
                        .map(|&uid| {
                            let v = u32::try_from(uid).map_err(|_| {
                                Error::new(format!(
                                    "IMAP UID {uid} does not fit in u32 (u32::MAX); server \
                                     returned an out-of-range UID"
                                ))
                                .set_kind(ErrorKind::Bug)
                            })?;
                            NonZeroU32::new(v).ok_or_else(|| {
                                Error::new("IMAP UID 0 returned by server").set_kind(ErrorKind::Bug)
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;
                    let mut envelopes = Vec::with_capacity(batch_size);
                    let mut conn = connection.lock().await?;
                    // Bounded single retry for a connection drop mid-batch.
                    // The read below surfaces a dead stream as a
                    // network-ish error; reconnect, clear the partial
                    // response buffer and retry the SAME batch without
                    // advancing the cursor. The re-`EXAMINE` is forced so a
                    // stale `current_mailbox` belonging to the previous
                    // stream cannot skip the SELECT on the new one. A
                    // second failure or any non-network error propagates
                    // unchanged.
                    let mut fetch_attempt = 0_usize;
                    loop {
                        response.clear();
                        let sequence_set = SequenceSet::try_from(nz.clone())?;
                        let (required_responses, macro_or_item_names) =
                            if uid_store.fetch_body_structure {
                                crate::imap::email::common_attributes()
                            } else {
                                crate::imap::email::common_attributes_light()
                            };
                        let exchange = async {
                            // Force the re-`EXAMINE` only on the retry: on a
                            // fresh stream `current_mailbox` is always empty,
                            // while forcing it on the first attempt would add
                            // a redundant `EXAMINE` to the steady-state
                            // command sequence (the preceding `InitialFresh`
                            // `init_mailbox` already selected the mailbox).
                            conn.examine_mailbox(mailbox_hash, response, fetch_attempt > 0)
                                .await?;
                            conn.send_command(CommandBody::Fetch {
                                sequence_set,
                                macro_or_item_names,
                                uid: true,
                                modifiers: vec![],
                            })
                            .await?;
                            conn.read_response(response, required_responses)
                                .await
                                .chain_err_summary(|| {
                                    format!(
                                        "Could not parse fetch response for mailbox {mailbox_path}"
                                    )
                                })?;
                            Ok::<(), Error>(())
                        }
                        .await;
                        match exchange {
                            Ok(()) => break,
                            Err(err)
                                if fetch_attempt == 0
                                    && (err.kind.is_network()
                                        || err.kind.is_timeout()
                                        || err.kind.is_oserror()) =>
                            {
                                fetch_attempt += 1;
                                log::trace!(
                                    "FreshFetch: connection lost mid-batch ({}); reconnecting and \
                                     retrying the batch",
                                    err
                                );
                                conn.connect().await?;
                            }
                            Err(err) => return Err(err),
                        }
                    }
                    let (_, mut v, _) = protocol_parser::fetch_responses(response)?;
                    for FetchResponse {
                        ref uid,
                        ref mut envelope,
                        ref mut flags,
                        raw_fetch_value,
                        ref references,
                        ..
                    } in v.iter_mut()
                    {
                        if uid.is_none() || envelope.is_none() || flags.is_none() {
                            imap_log!(
                                trace,
                                conn,
                                "BUG? something in fetch is none. UID: {:?}, envelope: {:?} \
                                 flags: {:?}",
                                uid,
                                envelope,
                                flags
                            );
                            imap_log!(
                                trace,
                                conn,
                                "response was: {}",
                                String::from_utf8_lossy(response)
                            );
                            if let Ok(Some(untagged_response)) =
                                super::protocol_parser::untagged_responses(raw_fetch_value)
                                    .map(|(_, v, _)| v)
                            {
                                if let Some(ev) = conn.process_untagged(untagged_response).await? {
                                    conn.add_backend_event(ev);
                                }
                            }
                            continue;
                        }
                        let uid = uid.unwrap();
                        let env = envelope.as_mut().unwrap();
                        env.set_hash(generate_envelope_hash(&mailbox_path, &uid));
                        if let Some(value) = references {
                            env.set_references(value);
                        }
                        let mut tag_lck = uid_store.collection.tag_index.write().unwrap();
                        if let Some((flags, keywords)) = flags {
                            env.set_flags(*flags);
                            if !env.is_seen() {
                                our_unseen.insert(env.hash());
                            }
                            for f in keywords {
                                let hash = TagHash::from_bytes(f.as_bytes());
                                tag_lck.entry(hash).or_insert_with(|| f.to_string());
                                env.tags_mut().insert(hash);
                            }
                        }
                    }
                    {
                        let mut uid_store = Arc::clone(&self.uid_store);

                        if let Err(err) = uid_store
                            .insert_envelopes(mailbox_hash, &v)
                            .chain_err_summary(|| {
                                format!(
                                    "Could not save envelopes in cache for mailbox \
                                     {mailbox_path}"
                                )
                            })
                        {
                            (uid_store.event_consumer)(uid_store.account_hash, err.into());
                        }
                    }

                    for f in v {
                        let FetchResponse {
                            uid: Some(uid),
                            message_sequence_number,
                            envelope: Some(env),
                            ..
                        } = f
                        else {
                            continue;
                        };
                        // The MSN is untrusted wire data and the parser
                        // accepts a leading `0`; saturate instead of
                        // underflowing (`0 - 1`) on such a response.
                        uid_store
                            .msn_index
                            .lock()
                            .unwrap()
                            .entry(mailbox_hash)
                            .or_default()
                            .insert(message_sequence_number.saturating_sub(1), uid);
                        uid_store
                            .hash_index
                            .lock()
                            .unwrap()
                            .insert(env.hash(), (uid, mailbox_hash));
                        uid_store
                            .uid_index
                            .lock()
                            .unwrap()
                            .insert((mailbox_hash, uid), env.hash());
                        envelopes.push(env);
                    }
                    unseen.lock().unwrap().insert_existing_set(our_unseen);
                    mailbox_exists
                        .lock()
                        .unwrap()
                        .insert_existing_set(envelopes.iter().map(|env| env.hash()).collect::<_>());
                    drop(conn);

                    // Advance the cursor; if the list is exhausted,
                    // finalize unseen/exists accounting and finish.
                    // Take ownership of `uids` for the next iteration so
                    // the unused trailing slots are freed (otherwise
                    // every chunk re-uses the full Vec).
                    if finished {
                        unseen.lock().unwrap().set_not_yet_seen(0);
                        mailbox_exists.lock().unwrap().set_not_yet_seen(0);
                        // Free the unused tail of the UID list: every
                        // `chunk()` call would otherwise retain the
                        // whole `Vec` even though we already served
                        // every message.
                        *uids = Vec::new();
                        self.stage = FetchStage::Finished;
                    } else {
                        // Advance the cursor for the next chunk.
                        *next += take;
                    }
                    return Ok(envelopes);
                }
                FetchStage::Finished => {
                    return Ok(vec![]);
                }
            }
        }
    }

    /// Whether the persisted cache for this mailbox is incomplete with
    /// respect to the `SELECT`/`EXAMINE` this fetch is walking.
    ///
    /// True only when a mailbox row exists for the same cache generation
    /// ([`Self::last_select_uidvalidity`]), the persisted envelope count is
    /// below the server-reported [`Self::last_select_exists`], and the
    /// deficit is **not** recoverable by the incremental
    /// `UID FETCH lastseenuid+1:*` that the resync stages issue: a cache
    /// that is merely missing newly delivered UIDs above `lastseenuid`
    /// catches up incrementally and must not force a full rebuild. A
    /// complete cache and a genuinely empty server mailbox are both
    /// complete. Called at stage transitions only, so the extra `COUNT(*)`
    /// / `lastseenuid` queries per call are not on the hot path.
    ///
    /// This is also one of the cache-completeness rebuild trigger sites:
    /// the first call that finds the cache incomplete for this mailbox
    /// records the mailbox in
    /// [`UIDStore::completeness_rebuilt`](super::UIDStore) and returns
    /// `true`; every later call for the same mailbox returns `false`,
    /// because a server may report `EXISTS` larger than its retrievable
    /// set (Coremail / 网易 163) and a full rebuild can never reach it.
    fn cache_is_incomplete(&self) -> bool {
        let mailbox_hash = self.mailbox_hash;
        let mut uid_store = Arc::clone(&self.uid_store);
        let Ok(Some(state)) = uid_store.mailbox_state(mailbox_hash) else {
            return false;
        };
        if state.uidvalidity != self.last_select_uidvalidity {
            return false;
        }
        let Ok(cached_env_count) = uid_store.count_envelopes(mailbox_hash) else {
            return false;
        };
        if cached_env_count.unwrap_or(0) >= self.last_select_exists {
            return false;
        }
        // The deficit is only actionable when it sits below `lastseenuid`:
        // if the server reports UIDs above it (`UIDNEXT > lastseenuid + 1`)
        // the incremental resync can fetch the missing history, so a full
        // rebuild would be redundant. An unknown `UIDNEXT` (`0`) is treated
        // as unrecoverable so that the cache is rebuilt rather than trusted.
        let Ok(Some(lastseenuid)) = uid_store.lastseenuid(mailbox_hash) else {
            return false;
        };
        let recoverable = self.last_select_uidnext != 0
            && self.last_select_uidnext <= lastseenuid.saturating_add(1);
        if !recoverable {
            return false;
        }
        // Session-scoped rebuild memo: the first incompleteness per mailbox
        // per process triggers the rebuild (marking it here means "a
        // rebuild has been triggered this session"), later checks accept
        // the retrievable set as complete. Without this, a server whose
        // `EXISTS` exceeds the retrievable set (Coremail / 网易 163) would
        // re-wipe and re-fetch the mailbox on every poll.
        self.uid_store
            .completeness_rebuilt
            .lock()
            .unwrap()
            .insert(mailbox_hash)
    }

    fn load_cache(
        conn: &ImapConnection,
        mailbox_hash: MailboxHash,
        max_uid: UID,
        batch_size: usize,
        select_response: SelectResponse,
    ) -> Option<Result<(Vec<EnvelopeHash>, Option<UID>)>> {
        let mut uid_store = conn.uid_store.clone();
        if let Err(err) = uid_store
            .update_mailbox(mailbox_hash, &select_response)
            .chain_err_summary(|| format!("Could not update cache for mailbox {mailbox_hash}."))
        {
            (uid_store.event_consumer)(uid_store.account_hash, err.into());
        }
        match uid_store.mailbox_state(mailbox_hash) {
            Err(err) => return Some(Err(err)),
            Ok(Some(_)) => {}
            Ok(None) => {
                return None;
            }
        };
        match uid_store.envelopes(mailbox_hash, max_uid, batch_size) {
            Ok(Some(envs)) => Some(Ok(envs)),
            Ok(None) => None,
            Err(err) => Some(Err(err)),
        }
    }

    async fn cached_envs(
        &mut self,
        max_uid: UID,
        batch_size: usize,
    ) -> Result<Option<(Vec<Envelope>, Option<UID>)>> {
        let Self {
            stage: _,
            ref mut connection,
            mailbox_hash,
            ref uid_store,
            batch_size: _,
            cache_batch_size: _,
            ref mut response,
            cache_served_offline: _,
            last_select_exists: _,
            last_select_uidvalidity: _,
            last_select_uidnext: _,
            fresh_fetch_uids: _,
            fresh_fetch_next: _,
        } = self;
        let mailbox_hash = *mailbox_hash;
        if !uid_store.keep_offline_cache.load(Ordering::SeqCst) {
            return Ok(None);
        }
        {
            let mut conn = connection.lock().await?;
            let select_response = conn.select_mailbox(mailbox_hash, response, false).await?;
            match Self::load_cache(&conn, mailbox_hash, max_uid, batch_size, select_response) {
                None => Ok(None),
                Some(Ok((env_hashes, lowest_served))) => {
                    let env_lck = uid_store.envelopes.lock().unwrap();
                    Ok(Some((
                        env_hashes
                            .into_iter()
                            .filter_map(|env_hash| {
                                env_lck.get(&env_hash).map(|c_env| c_env.inner.clone())
                            })
                            .collect::<Vec<Envelope>>(),
                        lowest_served,
                    )))
                }
                Some(Err(err)) => Err(err),
            }
        }
    }

    /// Offline variant of [`Self::cached_envs`]: reads a batch of
    /// envelopes from the cache without any network round-trip and
    /// without updating cache metadata (no SELECT, no `update_mailbox`)
    /// — the metadata refresh is left to the subsequent resync.
    async fn cached_envs_offline(
        &self,
        max_uid: UID,
        batch_size: usize,
    ) -> Result<Option<(Vec<Envelope>, Option<UID>)>> {
        let mut uid_store = Arc::clone(&self.uid_store);
        let mailbox_hash = self.mailbox_hash;
        if !uid_store.keep_offline_cache.load(Ordering::SeqCst) {
            return Ok(None);
        }
        match uid_store.mailbox_state(mailbox_hash) {
            Err(err) => return Err(err),
            Ok(Some(_)) => {}
            Ok(None) => return Ok(None),
        }
        match uid_store.envelopes(mailbox_hash, max_uid, batch_size) {
            Ok(Some((env_hashes, lowest_served))) => {
                let env_lck = uid_store.envelopes.lock().unwrap();
                Ok(Some((
                    env_hashes
                        .into_iter()
                        .filter_map(|env_hash| {
                            env_lck.get(&env_hash).map(|c_env| c_env.inner.clone())
                        })
                        .collect::<Vec<Envelope>>(),
                    lowest_served,
                )))
            }
            Ok(None) => Ok(None),
            Err(err) => Err(err),
        }
    }

    fn lastseenuid(&mut self) -> Result<Option<UID>> {
        let mailbox_hash = self.mailbox_hash;
        match self.uid_store.lastseenuid(mailbox_hash)? {
            None => Ok(None),
            Some(lastseenuid) => Ok(Some(lastseenuid)),
        }
    }
}
