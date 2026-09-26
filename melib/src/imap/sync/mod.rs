/*
 * melib - IMAP
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

use imap_codec::imap_types::{
    command::FetchModifier,
    fetch::{MacroOrMessageDataItemNames, MessageDataItemName},
    search::SearchKey,
    sequence::{SeqOrUid, SequenceSet},
    status::StatusDataItemName,
};
use indexmap::IndexSet;

use super::*;

pub mod cache;
#[cfg(feature = "sqlite3")]
pub mod sqlite3_cache;

#[cfg(test)]
mod tests;

/// Quick synchronization check of [RFC4549](https://datatracker.ietf.org/doc/rfc4549/)
/// Section 4.3.2: whether a `STATUS` response matches the cached
/// UIDVALIDITY and the `(MESSAGES, UNSEEN, UIDNEXT)` counters recorded with
/// [`ImapCache::record_status`].
///
/// Missing items (`None`) compare equal to each other; a missing UIDVALIDITY
/// never matches.
fn status_unchanged(
    status: &protocol_parser::StatusResponse,
    cached_uidvalidity: UIDVALIDITY,
    cached_status: cache::CachedStatus,
) -> bool {
    status.uidvalidity == Some(cached_uidvalidity)
        && (status.messages, status.unseen, status.uidnext) == cached_status
}

impl ImapConnection {
    pub async fn resync(&mut self, mailbox_hash: MailboxHash) -> Result<Option<Vec<Envelope>>> {
        if matches!(self.sync_policy, SyncPolicy::None) {
            return Ok(None);
        }

        if self.uid_store.mailbox_state(mailbox_hash)?.is_none() {
            return Ok(None);
        }

        // RFC 4549 §4.3 resync is incremental (`UID FETCH lastseenuid+1:*`):
        // it reconciles a populated cache but cannot backfill a mailbox whose
        // `envelopes` table is empty. That skeleton is the residue of an
        // interrupted sync — the `mailbox` row with its STATUS counters and
        // `max_uid` survived, the cached envelopes did not. Report "no
        // incremental data" so the caller rebuilds the mailbox with a full
        // fresh fetch instead of serving an empty listing forever.
        if !self.uid_store.has_envelopes(mailbox_hash)? {
            return Ok(None);
        }

        match self.sync_policy {
            SyncPolicy::Basic => self.resync_basic(mailbox_hash).await,
            SyncPolicy::Condstore => self.resync_condstore(mailbox_hash).await,
            SyncPolicy::CondstoreQresync => self.resync_condstoreqresync(mailbox_hash).await,
            // `SyncPolicy::None` returns early above.
            _ => Ok(None),
        }
    }

    /// Re-sync IMAP state by following the strategy described in
    /// [RFC4549](https://datatracker.ietf.org/doc/rfc4549/) "Synchronization Operations for
    /// Disconnected IMAP4 Clients" Section 4.3. Details of "Normal"
    /// Synchronization of a Single Mailbox.
    pub async fn resync_basic(
        &mut self,
        mailbox_hash: MailboxHash,
    ) -> Result<Option<Vec<Envelope>>> {
        // Plan:
        //
        // 1. Check UIDVALIDITY
        // 2. Discover New Messages and Changes to Old Messages (Section 4.3.1.) i. tag1
        //    UID FETCH <lastseenuid+1>:* <descriptors> (Create events are created from
        //    return value) ii. tag2 UID FETCH 1:<lastseenuid> FLAGS (NewFlags events
        //    must be manually added)
        // 3. Update seen/unseen sets for mailbox
        // 4. Check for removed messages (Remove events must be manually added)
        // 5. Add events
        // 6. Return new envelopes

        // Step 1: Check UIDVALIDITY
        let (Some(cached_uidvalidity), Some(lastseenuid)) = (
            self.uid_store
                .uidvalidity
                .lock()
                .unwrap()
                .get(&mailbox_hash)
                .cloned(),
            self.uid_store
                .lastseenuid
                .lock()
                .unwrap()
                .get(&mailbox_hash)
                .cloned(),
        ) else {
            return Ok(None);
        };
        let mailbox_path = {
            let f = &self.uid_store.mailboxes.lock().await[&mailbox_hash];
            f.imap_path().to_string()
        };
        let mut response = Vec::with_capacity(1024);

        // The RFC 4549 §4.3.2 quick check may only trust persisted cache
        // state. Counters held in memory by a same-session connection (e.g.
        // the watch connection) do not prove that the `envelopes` table is
        // usable: an empty table means the incremental `UID FETCH` below has
        // nothing to reconcile, so the mailbox must be rebuilt with a full
        // fresh fetch instead of being quick-skipped.
        if !self.uid_store.has_envelopes(mailbox_hash)? {
            return Ok(None);
        }

        // Quick synchronization check (RFC4549 Section 4.3.2): if the mailbox
        // STATUS counters are unchanged since the last recorded values, no
        // message was added, removed or had its flags changed, so the UID
        // FETCHes below can be skipped entirely.
        //
        // The item order matches the declaration order of the items in
        // `protocol_parser::status_response`'s `permutation` parsers, because
        // that parser only accepts this order; a response that can't be
        // parsed simply disables the quick check for this run.
        self.send_command(CommandBody::status(
            mailbox_path.as_str(),
            [
                StatusDataItemName::Messages,
                StatusDataItemName::UidNext,
                StatusDataItemName::UidValidity,
                StatusDataItemName::Unseen,
            ]
            .as_slice(),
        )?)
        .await?;
        self.read_response(&mut response, RequiredResponses::STATUS)
            .await?;
        let parsed_status = protocol_parser::status_response(response.as_slice())
            .ok()
            .map(|(_, s)| s);
        // The `STATUS` reply above is the most reliable `UIDNEXT` source
        // (unlike `SELECT`, servers such as Coremail often omit it there);
        // keep it for the cache-completeness check below.
        let status_uidnext = parsed_status.as_ref().and_then(|s| s.uidnext);
        let cached_status = self.uid_store.cached_status(mailbox_hash)?;
        // The recorded STATUS counters describe the server, not the cache:
        // only trust the quick-skip when the persisted envelope count
        // matches the recorded MESSAGES. A baseline recorded by a no-op
        // resync while the cache held fewer envelopes (e.g. an interrupted
        // sync, or a server-side fetch window that widened and revealed
        // history below `lastseenuid`) would otherwise quick-skip forever
        // and never rebuild the mailbox.
        let cache_covers_recorded_messages =
            match (cached_status, self.uid_store.count_envelopes(mailbox_hash)?) {
                (Some((Some(messages), _, _)), Some(count)) => count == messages,
                _ => true,
            };
        match (parsed_status, cached_status) {
            (Some(status), Some(cached_status))
                if status_unchanged(&status, cached_uidvalidity, cached_status)
                    && cache_covers_recorded_messages =>
            {
                if self.stream.as_ref().is_ok_and(|stream| {
                    !matches!(
                        stream.current_mailbox.is(&mailbox_hash),
                        MailboxSelection::None
                    )
                }) {
                    // RFC 4549 §4.3.2: a server may answer `STATUS` for the
                    // mailbox this connection currently has selected from
                    // the state at `SELECT` time instead of the live
                    // counters, so a matching reply does not prove the
                    // mailbox is unchanged. Flush any pending untagged
                    // updates for the selected mailbox with `NOOP` (RFC
                    // 3501 §6.1.2); if the server reports any change now,
                    // the full resync below must run. The reply is read with
                    // `RequiredResponses::UNTAGGED` so the untagged lines are
                    // returned in `response` instead of being consumed by
                    // `process_untagged`, which would only fetch the last
                    // new message by its message sequence number.
                    self.send_command(CommandBody::Noop).await?;
                    self.read_response(&mut response, RequiredResponses::UNTAGGED)
                        .await?;
                    if response.split_rn().any(|l| {
                        protocol_parser::untagged_responses(l)
                            .map(|(_, v, _)| v.is_some())
                            .unwrap_or(false)
                    }) {
                        log::trace!(
                            "resync_basic: NOOP reported pending updates for the selected \
                             mailbox {mailbox_path}; running full resync"
                        );
                    } else {
                        log::trace!(
                            "resync_basic: STATUS counters of mailbox {mailbox_path} are \
                             unchanged; skipping FLAGS resync"
                        );
                        return Ok(Some(vec![]));
                    }
                } else {
                    log::trace!(
                        "resync_basic: STATUS counters of mailbox {mailbox_path} are unchanged; \
                         skipping FLAGS resync"
                    );
                    return Ok(Some(vec![]));
                }
            }
            (None, _) => {
                log::trace!(
                    "resync_basic: could not parse STATUS response of mailbox {mailbox_path}: \
                     {}",
                    String::from_utf8_lossy(&response)
                );
            }
            _ => {}
        }

        let select_response = self
            .select_mailbox(mailbox_hash, &mut response, true)
            .await?;
        if select_response.uidvalidity != cached_uidvalidity {
            self.uid_store
                .init_mailbox(mailbox_hash, &select_response)?;
            return Ok(None);
        }

        self.uid_store
            .update_mailbox(mailbox_hash, &select_response)?;

        // Cache-completeness invariant: the persisted envelope count must
        // cover the server's `EXISTS`. If it does not (e.g. a server-side
        // fetch window widened and revealed UIDs below `lastseenuid`), the
        // incremental `UID FETCH lastseenuid+1:*` below cannot see the
        // missing history and would silently serve an incomplete mailbox.
        // A deficit that the incremental fetch *can* recover (newly
        // delivered UIDs above `lastseenuid`) must not force a full rebuild.
        // Report "no incremental data" so `examine_updates` backfills via
        // message sequence number and the next `fetch()` stream rebuilds the
        // mailbox.
        let cached_env_count = self.uid_store.count_envelopes(mailbox_hash)?;
        let observed_uidnext = status_uidnext.unwrap_or(select_response.uidnext);
        let deficit_unrecoverable =
            observed_uidnext == 0 || observed_uidnext <= lastseenuid.saturating_add(1);
        if cached_env_count.unwrap_or(0) < select_response.exists && deficit_unrecoverable {
            // Only after a full rebuild has been triggered once for this
            // mailbox this session may the guard accept the retrievable
            // set: some servers (Coremail / 网易 163) report an `EXISTS`
            // larger than the retrievable set, so a permanent deficit
            // would otherwise re-fetch the mailbox on every poll.
            // The rebuild itself is triggered by the fetch stream's
            // `cache_is_incomplete` (which records the mailbox); the guard
            // must not record it here, or it would suppress exactly the
            // fresh fetch that refills the cache after this fallback.
            let already_rebuilt = self
                .uid_store
                .completeness_rebuilt
                .lock()
                .unwrap()
                .contains(&mailbox_hash);
            if !already_rebuilt {
                // Never wipe the mailbox here (`init_mailbox` used to drop
                // the row and cascade-delete every envelope). Every refill
                // path only upserts and therefore needs no clean row:
                // `FetchStage::InitialFresh` writes through
                // `update_mailbox` + `insert_envelopes` (`INSERT OR
                // REPLACE`), and `examine_updates` backfills by message
                // sequence number with plain inserts. A wipe buys nothing
                // but destroys mail when the refill dies mid-way: live
                // Coremail / 网易 163 (frequent disconnects) left several
                // mailboxes with 0 envelopes and INBOX stuck at 61/825
                // after a rebuild was interrupted. The fetch stream's
                // resume+retry keeps refill progress monotonic, so keeping
                // the cached rows is always safe.
                log::trace!(
                    "resync_basic: cache holds {} envelopes but server reports {} for mailbox \
                     {mailbox_path}; falling back to full rebuild",
                    cached_env_count.unwrap_or(0),
                    select_response.exists
                );
                return Ok(None);
            }
            log::trace!(
                "resync_basic: cache holds {} envelopes but server reports {} for mailbox \
                 {mailbox_path}; already rebuilt this session, accepting the retrievable set",
                cached_env_count.unwrap_or(0),
                select_response.exists
            );
        }

        let (mailbox_exists, unseen) = {
            let f = &self.uid_store.mailboxes.lock().await[&mailbox_hash];
            (f.exists.clone(), f.unseen.clone())
        };
        let mut refresh_events = vec![];

        let mut new_envelopes = vec![];
        let mut new_unseen = BTreeSet::default();
        let mut new_seen = BTreeSet::default();
        let mut valid_envs = BTreeSet::default();

        // Step 2. Discover New Messages and Changes to Old Messages

        // Step 2i. Create events

        let (required_responses, attributes) = if self.uid_store.fetch_body_structure {
            crate::imap::email::common_attributes()
        } else {
            crate::imap::email::common_attributes_light()
        };
        self.send_command(CommandBody::fetch(
            lastseenuid.saturating_add(1)..,
            attributes,
            true,
        )?)
        .await?;
        self.read_response(&mut response, required_responses)
            .await?;
        let (_, mut v, _) = protocol_parser::fetch_responses(&response)?;
        //> Also note that a UID range of 559:* always includes the UID of the
        //> last message in the mailbox, even if 559 is higher than any
        //> assigned UID value. This is because the contents of a range are
        //> independent of the order of the range endpoints. Thus, any UID
        //> range with * as one of the endpoints indicates at least one
        //> message (the message with the highest numbered UID), unless the
        //> mailbox is empty.
        //- 6.4.9. UID Command - RFC9051
        v.retain(|f| f.uid != Some(lastseenuid));
        {
            {
                let mut tag_lck = self.uid_store.collection.tag_index.write().unwrap();
                for FetchResponse {
                    ref uid,
                    ref mut envelope,
                    ref mut flags,
                    ref references,
                    ..
                } in v.iter_mut()
                {
                    // RFC 3501 §6.4.8: a reply to a `UID FETCH` command
                    // must contain the UID data item. Without it the
                    // envelope cannot be associated with a message, so
                    // surface a protocol error instead of panicking.
                    let Some(uid) = *uid else {
                        return Err(Error::new(format!(
                            "IMAP server error: UID FETCH reply for mailbox {mailbox_path} is \
                             missing the UID data item (RFC 3501 6.4.8 violation)."
                        ))
                        .set_kind(ErrorKind::ProtocolError));
                    };
                    let env = envelope.as_mut().unwrap();
                    let env_hash = generate_envelope_hash(&mailbox_path, &uid);
                    valid_envs.insert(env_hash);
                    env.set_hash(env_hash);
                    if let Some(value) = references {
                        env.set_references(value);
                    }
                    if let Some((flags, keywords)) = flags {
                        env.set_flags(*flags);
                        if !env.is_seen() {
                            new_unseen.insert(env_hash);
                        } else {
                            new_seen.insert(env_hash);
                        }
                        for f in keywords {
                            let hash = TagHash::from_bytes(f.as_bytes());
                            tag_lck.entry(hash).or_insert_with(|| f.to_string());
                            env.tags_mut().insert(hash);
                        }
                    }
                }
            }

            self.uid_store
                .insert_envelopes(mailbox_hash, &v)
                .chain_err_summary(|| {
                    format!("Could not save envelopes in cache for mailbox {mailbox_path}")
                })?;
        }
        for FetchResponse { envelope, .. } in v {
            let Some(env) = envelope else {
                continue;
            };
            new_envelopes.push(env);
        }

        // Step 2ii. NewFlags events

        let sequence_set = if lastseenuid == 0 {
            SequenceSet::from(..)
        } else {
            SequenceSet::try_from(..=lastseenuid)?
        };
        self.send_command(CommandBody::Fetch {
            sequence_set,
            macro_or_item_names: MacroOrMessageDataItemNames::MessageDataItemNames(vec![
                MessageDataItemName::Flags,
            ]),
            uid: true,
            modifiers: vec![],
        })
        .await?;
        self.read_response(&mut response, RequiredResponses::FETCH_FLAGS)
            .await?;
        let (_, v, _) = protocol_parser::fetch_responses(&response)?;
        {
            let mut env_lck = self.uid_store.envelopes.lock().unwrap();
            let mut tag_lck = self.uid_store.collection.tag_index.write().unwrap();
            for FetchResponse { uid, flags, .. } in v {
                // RFC 3501 §6.4.8: a reply to a `UID FETCH` command must
                // contain the UID data item. Without it the flags cannot
                // be associated with a message, and skipping the reply
                // would (wrongly) mark the message as removed in Step 4
                // below, so surface a protocol error instead of
                // panicking.
                let Some(uid) = uid else {
                    return Err(Error::new(format!(
                        "IMAP server error: UID FETCH reply for mailbox {mailbox_path} is \
                         missing the UID data item (RFC 3501 6.4.8 violation)."
                    ))
                    .set_kind(ErrorKind::ProtocolError));
                };
                let env_hash = generate_envelope_hash(&mailbox_path, &uid);
                let Some(cenv) = env_lck.get_mut(&env_hash) else {
                    continue;
                };
                valid_envs.insert(env_hash);
                if let Some((flags, tags)) = flags {
                    let is_new_flags = !(flags == cenv.inner.flags()
                        && cenv.inner.tags()
                            == &tags
                                .iter()
                                .map(|t| TagHash::from_bytes(t.as_bytes()))
                                .collect::<IndexSet<TagHash>>());
                    cenv.inner.set_flags(flags);
                    if is_new_flags && !cenv.inner.is_seen() {
                        new_unseen.insert(env_hash);
                    } else if is_new_flags {
                        new_seen.insert(env_hash);
                    }
                    cenv.inner.tags_mut().clear();
                    for f in &tags {
                        let hash = TagHash::from_bytes(f.as_bytes());
                        tag_lck.entry(hash).or_insert_with(|| f.to_string());
                        cenv.inner.tags_mut().insert(hash);
                    }
                    if is_new_flags {
                        refresh_events.push((
                            uid,
                            RefreshEvent {
                                mailbox_hash,
                                account_hash: self.uid_store.account_hash,
                                kind: RefreshEventKind::NewFlags(env_hash, (flags, tags)),
                            },
                        ));
                    }
                }
            }
        }

        // Step 3. Update seen/unseen sets for mailbox
        let new_envelopes_hash_set: BTreeSet<_> =
            new_envelopes.iter().map(|env| env.hash()).collect::<_>();
        {
            let mut unseen_lck = unseen.lock().unwrap();
            for &seen_env_hash in new_envelopes_hash_set
                .difference(&new_unseen)
                .chain(new_seen.iter())
            {
                unseen_lck.remove(seen_env_hash);
            }

            unseen_lck.insert_set(new_unseen);
        }
        {
            let mut exists_lck = mailbox_exists.lock().unwrap();
            exists_lck.insert_set(new_envelopes_hash_set);
        }
        // Step 4. Remove events
        {
            let mut env_lck = self.uid_store.envelopes.lock().unwrap();
            let mut unseen_lck = unseen.lock().unwrap();
            let mut exists_lck = mailbox_exists.lock().unwrap();
            for env_hash in env_lck
                .iter()
                .filter_map(|(h, cenv)| {
                    if cenv.mailbox_hash == mailbox_hash {
                        Some(*h)
                    } else {
                        None
                    }
                })
                .collect::<BTreeSet<EnvelopeHash>>()
                .difference(&valid_envs)
            {
                refresh_events.push((
                    env_lck[env_hash].uid,
                    RefreshEvent {
                        mailbox_hash,
                        account_hash: self.uid_store.account_hash,
                        kind: RefreshEventKind::Remove(*env_hash),
                    },
                ));
                // A mail removed on the server must not survive as a
                // ghost in the seen/unseen sets either. (Semantic port
                // of upstream meli 4f2414a3.)
                unseen_lck.remove(*env_hash);
                exists_lck.remove(*env_hash);
                env_lck.remove(env_hash);
            }
        }
        // Step 5. Add events
        self.uid_store.update(mailbox_hash, &refresh_events)?;
        for (_uid, ev) in refresh_events {
            self.add_refresh_event(ev);
        }
        // Record the final STATUS counters as the baseline for the quick
        // synchronization check at the start of the next resync. If the
        // response can't be parsed, skip recording; the next resync will run
        // the full flow again.
        self.send_command(CommandBody::status(
            mailbox_path.as_str(),
            [
                StatusDataItemName::Messages,
                StatusDataItemName::UidNext,
                StatusDataItemName::Unseen,
            ]
            .as_slice(),
        )?)
        .await?;
        self.read_response(&mut response, RequiredResponses::STATUS)
            .await?;
        if let Ok((_, status)) = protocol_parser::status_response(response.as_slice()) {
            self.uid_store.record_status(
                mailbox_hash,
                status.messages,
                status.unseen,
                status.uidnext,
            )?;
        } else {
            log::trace!(
                "resync_basic: could not parse STATUS response of mailbox {mailbox_path}: {}",
                String::from_utf8_lossy(&response)
            );
        }
        // Step 6. Return new envelopes
        Ok(Some(new_envelopes))
    }

    /// Resync with `CONDSTORE` Extension
    ///
    /// Re-sync IMAP state by following the strategy described in
    /// [RFC4549](https://datatracker.ietf.org/doc/rfc4549/) "Synchronization Operations for
    /// Disconnected IMAP4 Clients", Section 6.1 "CONDSTORE Extension"
    pub async fn resync_condstore(
        &mut self,
        mailbox_hash: MailboxHash,
    ) -> Result<Option<Vec<Envelope>>> {
        let mut response = Vec::with_capacity(8 * 1024);

        let cached_uidvalidity = self
            .uid_store
            .uidvalidity
            .lock()
            .unwrap()
            .get(&mailbox_hash)
            .cloned();
        let lastseenuid = self
            .uid_store
            .lastseenuid
            .lock()
            .unwrap()
            .get(&mailbox_hash)
            .cloned();
        let cached_highestmodseq = self
            .uid_store
            .highestmodseqs
            .lock()
            .unwrap()
            .get(&mailbox_hash)
            .cloned();

        let (Some(cached_uidvalidity), Some(lastseenuid), Some(cached_highestmodseq)): (
            Option<UID>,
            Option<UID>,
            Option<std::result::Result<ModSequence, ()>>,
        ) = (cached_uidvalidity, lastseenuid, cached_highestmodseq) else {
            // This means the mailbox is not cached.
            return Ok(None);
        };
        let Ok(cached_highestmodseq) = cached_highestmodseq else {
            // No MODSEQ is available for __this__ mailbox, fallback to basic sync
            return self.resync_basic(mailbox_hash).await;
        };

        // 1. check UIDVALIDITY. If fail, discard cache and rebuild
        let select_response = self
            .select_mailbox(mailbox_hash, &mut response, true)
            .await?;
        if select_response.uidvalidity != cached_uidvalidity {
            self.uid_store
                .init_mailbox(mailbox_hash, &select_response)?;
            return Ok(None);
        }

        let new_highestmodseq = match select_response.highestmodseq {
            None => return self.resync_basic(mailbox_hash).await,
            Some(Err(_)) => {
                self.uid_store
                    .highestmodseqs
                    .lock()
                    .unwrap()
                    .insert(mailbox_hash, Err(()));
                return self.resync_basic(mailbox_hash).await;
            }
            Some(Ok(v)) => v,
        };

        self.uid_store
            .update_mailbox(mailbox_hash, &select_response)?;

        let (mailbox_path, mailbox_exists, unseen) = {
            let f = &self.uid_store.mailboxes.lock().await[&mailbox_hash];
            (
                f.imap_path().to_string(),
                f.exists.clone(),
                f.unseen.clone(),
            )
        };

        // Same cache-completeness invariant as `resync_basic`: the persisted
        // envelope count must cover the server's `EXISTS`, otherwise the
        // incremental `UID FETCH lastseenuid+1:*` below cannot see history
        // whose UIDs sit below `lastseenuid`. A deficit that the incremental
        // fetch *can* recover (newly delivered UIDs above `lastseenuid`) must
        // not force a full rebuild. Report "no incremental data" so the
        // caller rebuilds the mailbox.
        let cached_env_count = self.uid_store.count_envelopes(mailbox_hash)?;
        let deficit_unrecoverable = select_response.uidnext == 0
            || select_response.uidnext <= lastseenuid.saturating_add(1);
        if cached_env_count.unwrap_or(0) < select_response.exists && deficit_unrecoverable {
            // Session-scoped rebuild memo, same rationale as
            // `resync_basic`: some servers (Coremail / 网易 163) report an
            // `EXISTS` larger than the retrievable set. Only after the
            // fetch stream has triggered a full rebuild (recorded via
            // `cache_is_incomplete`) does the guard accept the retrievable
            // set; the guard itself must not record the mailbox, or it
            // would suppress the fresh fetch that refills the cache after
            // this fallback.
            let already_rebuilt = self
                .uid_store
                .completeness_rebuilt
                .lock()
                .unwrap()
                .contains(&mailbox_hash);
            if !already_rebuilt {
                // Same reason as `resync_basic`: the refill paths only
                // upsert (`FetchStage::InitialFresh` via `update_mailbox` +
                // `insert_envelopes`/`INSERT OR REPLACE`, `examine_updates`
                // via plain inserts), so the guard must never call
                // `init_mailbox` and cascade-delete the cached envelopes.
                // Live Coremail / 网易 163 proved the wipe destructive: an
                // interrupted rebuild left mailboxes at 0 envelopes and
                // INBOX at 61/825.
                log::trace!(
                    "resync_condstore: cache holds {} envelopes but server reports {} for \
                     mailbox {mailbox_path}; falling back to full rebuild",
                    cached_env_count.unwrap_or(0),
                    select_response.exists
                );
                return Ok(None);
            }
            log::trace!(
                "resync_condstore: cache holds {} envelopes but server reports {} for mailbox \
                 {mailbox_path}; already rebuilt this session, accepting the retrievable set",
                cached_env_count.unwrap_or(0),
                select_response.exists
            );
        }

        let mut refresh_events = vec![];
        let mut new_envelopes = vec![];
        let mut new_unseen = BTreeSet::default();
        let mut new_seen = BTreeSet::default();
        let mut valid_envs = BTreeSet::default();

        // 1b) Check the mailbox HIGHESTMODSEQ.
        //  If the cached value is the same as the one returned by the server, skip
        // fetching  message flags on step 2-II, i.e., the client only has to
        // find out which messages got  expunged.
        if cached_highestmodseq != new_highestmodseq {
            /* Cache is synced, only figure out which messages got expunged */

            // 2) Fetch the current "descriptors".
            //   I)  Discover new messages.

            //   II) Discover changes to old messages and flags for new messages
            //       using
            //       "FETCH 1:* (FLAGS) (CHANGEDSINCE <cached-value>)" or
            //       "SEARCH MODSEQ <cached-value>".

            // 2. tag1 UID FETCH <lastseenuid+1>:* <descriptors>
            let (required_responses, macro_or_item_names) = if self.uid_store.fetch_body_structure {
                crate::imap::email::common_attributes()
            } else {
                crate::imap::email::common_attributes_light()
            };
            self.send_command(CommandBody::Fetch {
                sequence_set: ((lastseenuid.saturating_add(1))..).try_into()?,
                macro_or_item_names,
                uid: true,
                modifiers: vec![FetchModifier::ChangedSince(cached_highestmodseq.into())],
            })
            .await?;
            self.read_response(
                &mut response,
                required_responses | RequiredResponses::FETCH_MODSEQ,
            )
            .await?;
            let (_, mut v, _) = protocol_parser::fetch_responses(&response)?;
            //> Also note that a UID range of 559:* always includes the UID of the
            //> last message in the mailbox, even if 559 is higher than any
            //> assigned UID value. This is because the contents of a range are
            //> independent of the order of the range endpoints. Thus, any UID
            //> range with * as one of the endpoints indicates at least one
            //> message (the message with the highest numbered UID), unless the
            //> mailbox is empty.
            //- 6.4.9. UID Command - RFC9051
            v.retain(|f| f.uid != Some(lastseenuid));
            {
                {
                    let mut tag_lck = self.uid_store.collection.tag_index.write().unwrap();
                    for FetchResponse {
                        ref uid,
                        ref mut envelope,
                        ref mut flags,
                        ref references,
                        ..
                    } in v.iter_mut()
                    {
                        // RFC 3501 §6.4.8: a reply to a `UID FETCH`
                        // command must contain the UID data item. Without
                        // it the envelope cannot be associated with a
                        // message, so surface a protocol error instead of
                        // panicking.
                        let Some(uid) = *uid else {
                            return Err(Error::new(format!(
                                "IMAP server error: UID FETCH reply for mailbox {mailbox_path} is \
                                 missing the UID data item (RFC 3501 6.4.8 violation)."
                            ))
                            .set_kind(ErrorKind::ProtocolError));
                        };
                        let env = envelope.as_mut().unwrap();
                        let env_hash = generate_envelope_hash(&mailbox_path, &uid);
                        env.set_hash(env_hash);
                        if let Some(value) = references {
                            env.set_references(value);
                        }
                        if let Some((flags, keywords)) = flags {
                            env.set_flags(*flags);
                            if !env.is_seen() {
                                new_unseen.insert(env.hash());
                            } else {
                                new_seen.insert(env.hash());
                            }
                            for f in keywords {
                                let hash = TagHash::from_bytes(f.as_bytes());
                                tag_lck.entry(hash).or_insert_with(|| f.to_string());
                                env.tags_mut().insert(hash);
                            }
                        }
                    }
                }
                self.uid_store
                    .insert_envelopes(mailbox_hash, &v)
                    .chain_err_summary(|| {
                        format!("Could not save envelopes in cache for mailbox {mailbox_path}")
                    })?;
            }

            for FetchResponse { envelope, .. } in v {
                let Some(env) = envelope else {
                    continue;
                };
                new_envelopes.push(env);
            }
            // 3. Fetch the bodies of any "interesting" messages that the client doesn't
            //    already have.
            // 3. tag2 UID FETCH 1:<lastseenuid> FLAGS

            let sequence_set = if lastseenuid == 0 {
                (1..).try_into()?
            } else {
                (1..lastseenuid).try_into()?
            };
            self.send_command(CommandBody::Fetch {
                sequence_set,
                macro_or_item_names: MacroOrMessageDataItemNames::MessageDataItemNames(vec![
                    MessageDataItemName::Uid,
                    MessageDataItemName::Flags,
                ]),
                uid: true,
                modifiers: vec![FetchModifier::ChangedSince(cached_highestmodseq.into())],
            })
            .await?;
            self.read_response(
                &mut response,
                RequiredResponses::FETCH_FLAGS | RequiredResponses::FETCH_MODSEQ,
            )
            .await?;
            // 1) update cached flags for old messages;
            let mut env_lck = self.uid_store.envelopes.lock().unwrap();
            let mut tag_lck = self.uid_store.collection.tag_index.write().unwrap();
            let (_, v, _) = protocol_parser::fetch_responses(&response)?;
            {
                for FetchResponse { uid, flags, .. } in v {
                    // RFC 3501 §6.4.8: a reply to a `UID FETCH` command
                    // must contain the UID data item. Without it the
                    // flags cannot be associated with a message, and
                    // skipping the reply would (wrongly) mark the
                    // message as removed in Step 4 below, so surface a
                    // protocol error instead of panicking.
                    let Some(uid) = uid else {
                        return Err(Error::new(format!(
                            "IMAP server error: UID FETCH reply for mailbox {mailbox_path} is \
                             missing the UID data item (RFC 3501 6.4.8 violation)."
                        ))
                        .set_kind(ErrorKind::ProtocolError));
                    };
                    let env_hash = generate_envelope_hash(&mailbox_path, &uid);
                    let Some(cenv) = env_lck.get_mut(&env_hash) else {
                        continue;
                    };
                    valid_envs.insert(env_hash);
                    if let Some((flags, tags)) = flags {
                        let is_new_flags = !(flags == cenv.inner.flags()
                            && cenv.inner.tags()
                                == &tags
                                    .iter()
                                    .map(|t| TagHash::from_bytes(t.as_bytes()))
                                    .collect::<IndexSet<TagHash>>());
                        cenv.inner.set_flags(flags);
                        if is_new_flags && !cenv.inner.is_seen() {
                            new_unseen.insert(env_hash);
                        } else if is_new_flags {
                            new_seen.insert(env_hash);
                        }
                        cenv.inner.tags_mut().clear();
                        for f in &tags {
                            let hash = TagHash::from_bytes(f.as_bytes());
                            tag_lck.entry(hash).or_insert_with(|| f.to_string());
                            cenv.inner.tags_mut().insert(hash);
                        }
                        if is_new_flags {
                            refresh_events.push((
                                uid,
                                RefreshEvent {
                                    mailbox_hash,
                                    account_hash: self.uid_store.account_hash,
                                    kind: RefreshEventKind::NewFlags(env_hash, (flags, tags)),
                                },
                            ));
                        }
                    }
                }
            }
            self.uid_store
                .highestmodseqs
                .lock()
                .unwrap()
                .insert(mailbox_hash, Ok(new_highestmodseq));
            // Step 3. Update seen/unseen sets for mailbox
            let new_envelopes_hash_set: BTreeSet<_> =
                new_envelopes.iter().map(|env| env.hash()).collect::<_>();
            {
                let mut unseen_lck = unseen.lock().unwrap();
                if unseen_lck.set.is_empty() {
                    let new_total = unseen_lck.len() + new_unseen.len();
                    unseen_lck.set_not_yet_seen(new_total);
                } else {
                    for &seen_env_hash in new_envelopes_hash_set
                        .difference(&new_unseen)
                        .chain(new_seen.iter())
                    {
                        unseen_lck.remove(seen_env_hash);
                    }

                    unseen_lck.insert_set(new_unseen);
                }
            }
            {
                let mut exists_lck = mailbox_exists.lock().unwrap();
                if exists_lck.set.is_empty() {
                    let new_total = exists_lck.len() + new_envelopes_hash_set.len();
                    exists_lck.set_not_yet_seen(new_total);
                } else {
                    exists_lck.insert_set(new_envelopes_hash_set);
                }
            }
        }
        {
            // EXPUNGE events.
            // This should be UID SEARCH 1:<maxuid> but it's difficult to compare to cached
            // UIDs at the point of calling this function
            self.send_command(CommandBody::search(None, SearchKey::All.into(), true))
                .await?;
            self.read_response(&mut response, RequiredResponses::SEARCH)
                .await?;
            // 1) update cached flags for old messages;
            let (_, v) = protocol_parser::search_results(response.as_slice())?;
            for uid in v {
                valid_envs.insert(generate_envelope_hash(&mailbox_path, &uid));
            }
            {
                let mut env_lck = self.uid_store.envelopes.lock().unwrap();
                let olds = env_lck
                    .iter()
                    .filter_map(|(h, cenv)| {
                        if cenv.mailbox_hash == mailbox_hash {
                            Some(*h)
                        } else {
                            None
                        }
                    })
                    .collect::<BTreeSet<EnvelopeHash>>();
                for env_hash in olds.difference(&valid_envs) {
                    refresh_events.push((
                        env_lck[env_hash].uid,
                        RefreshEvent {
                            mailbox_hash,
                            account_hash: self.uid_store.account_hash,
                            kind: RefreshEventKind::Remove(*env_hash),
                        },
                    ));
                    env_lck.remove(env_hash);
                }
                drop(env_lck);
            }
        }
        // Step 5. Add events
        self.uid_store.update(mailbox_hash, &refresh_events)?;
        for (_uid, ev) in refresh_events {
            self.add_refresh_event(ev);
        }
        // Step 6. Return new envelopes
        Ok(Some(new_envelopes))
    }

    /// Resync with `CONDSTORE` and `QRESYNC` Extension
    ///
    /// Not implemented yet: this currently delegates to the `CONDSTORE` resync
    /// strategy ([`Self::resync_condstore`]).
    pub async fn resync_condstoreqresync(
        &mut self,
        mailbox_hash: MailboxHash,
    ) -> Result<Option<Vec<Envelope>>> {
        log::trace!(
            "resync_condstoreqresync: mailbox_hash: {:?}, function unimplemented",
            mailbox_hash
        );
        self.resync_condstore(mailbox_hash).await
    }

    pub async fn init_mailbox(&mut self, mailbox_hash: MailboxHash) -> Result<SelectResponse> {
        let mut response = Vec::with_capacity(8 * 1024);
        let (mailbox_path, mailbox_exists, permissions) = {
            let f = &self.uid_store.mailboxes.lock().await[&mailbox_hash];
            (
                f.imap_path().to_string(),
                f.exists.clone(),
                f.permissions.clone(),
            )
        };

        /* first SELECT the mailbox to get READ/WRITE permissions (because EXAMINE
         * only returns READ-ONLY for both cases) */
        let mut select_response = self
            .select_mailbox(mailbox_hash, &mut response, true)
            .await?;
        {
            {
                let mut uidvalidities = self.uid_store.uidvalidity.lock().unwrap();

                let v = uidvalidities
                    .entry(mailbox_hash)
                    .or_insert(select_response.uidvalidity);
                *v = select_response.uidvalidity;
            }
            {
                if let Some(highestmodseq) = select_response.highestmodseq {
                    let mut highestmodseqs = self.uid_store.highestmodseqs.lock().unwrap();
                    let v = highestmodseqs.entry(mailbox_hash).or_insert(highestmodseq);
                    *v = highestmodseq;
                }
            }
            let mut permissions = permissions.lock().unwrap();
            permissions.create_messages = !select_response.read_only;
            permissions.remove_messages = !select_response.read_only;
            permissions.set_flags = !select_response.read_only;
            permissions.rename_messages = !select_response.read_only;
            permissions.delete_messages = !select_response.read_only;
            {
                let mut mailbox_exists_lck = mailbox_exists.lock().unwrap();
                mailbox_exists_lck.clear();
                mailbox_exists_lck.set_not_yet_seen(select_response.exists);
            }
        }
        if select_response.exists == 0 {
            return Ok(select_response);
        }
        /* reselecting the same mailbox with EXAMINE prevents expunging it */
        self.examine_mailbox(mailbox_hash, &mut response, true)
            .await?;
        if select_response.uidnext == 0 {
            /* UIDNEXT shouldn't be 0, since exists != 0 at this point */
            self.send_command(CommandBody::status(
                mailbox_path,
                [StatusDataItemName::UidNext].as_slice(),
            )?)
            .await?;
            self.read_response(&mut response, RequiredResponses::STATUS)
                .await?;
            let (_, status) = protocol_parser::status_response(response.as_slice())?;
            if let Some(uidnext) = status.uidnext {
                if uidnext == 0 {
                    return Err(Error::new(
                        "IMAP server error: zero UIDNEXT with nonzero exists.",
                    ));
                }
                select_response.uidnext = uidnext;
            } else {
                /* Third fallback layer: Coremail (Netease 163/126/188) answers
                 * `UIDNEXT` neither in `SELECT`/`EXAMINE` nor in `STATUS`
                 * (`STATUS (UIDNEXT)` comes back with an empty `()` item list),
                 * so both layers above leave `uidnext` unset. `UID SEARCH *` is
                 * answered however: in a UID command `*` denotes the highest UID
                 * in use (RFC 3501 §6.4.8), so the reply is just that UID and
                 * `max_uid + 1` is the UIDNEXT the server would have reported.
                 * It cannot skip mail: no message can hold a UID above
                 * `max_uid`. An empty mailbox never reaches this point, because
                 * `exists == 0` returns early above; should a broken server
                 * nevertheless answer `* SEARCH` with no UIDs, fall back to
                 * `uidnext = 1` rather than failing the whole mailbox select. */
                self.send_command(CommandBody::search(
                    None,
                    SearchKey::SequenceSet(SequenceSet::from(SeqOrUid::Asterisk)).into(),
                    true,
                ))
                .await?;
                self.read_response(&mut response, RequiredResponses::SEARCH)
                    .await?;
                let (_, search_uids) = protocol_parser::search_results(response.as_slice())?;
                let max_uid = search_uids.iter().copied().max().unwrap_or(0);
                select_response.uidnext = if max_uid == 0 { 1 } else { max_uid + 1 };
            }
        }
        Ok(select_response)
    }
}
