/*
 * meli - imap module.
 *
 * Copyright 2019 Manos Pitsidianakis
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
    sync::Arc,
    time::{Duration, Instant},
};

use imap_codec::imap_types::search::SearchKey;

use super::*;
use crate::{
    backends::SpecialUsageMailbox,
    error::Result,
    imap::{email::common_attributes, sync::cache::ignore_not_found},
};

/// Arguments for IMAP watching functions
pub struct ImapWatchKit {
    pub conn: ImapConnection,
    pub main_conn: Arc<ConnectionMutex>,
    pub uid_store: Arc<UIDStore>,
}

/// Quote a mailbox path for interpolation into a raw IMAP command per
/// RFC 3501 quoted-string rules.
///
/// Returns `None` if the path contains CR or LF: it can never be legitimate
/// and would allow injecting arbitrary commands into the IMAP session.
fn quote_imap_mailbox_path(path: &str) -> Option<String> {
    if path.contains(['\r', '\n']) {
        return None;
    }
    let escaped = path.replace('\\', "\\\\").replace('"', "\\\"");
    Some(format!("\"{escaped}\""))
}

pub fn poll_with_examine(
    kit: ImapWatchKit,
) -> impl futures::stream::Stream<Item = Result<BackendEvent>> {
    try_fn_stream(|emitter| async move {
        log::trace!("poll with examine");
        let ImapWatchKit {
            mut conn,
            main_conn: _,
            uid_store,
        } = kit;
        conn.connect().await?;
        let mailboxes: HashMap<MailboxHash, ImapMailbox> = {
            let mailboxes_lck = timeout(uid_store.timeout, uid_store.mailboxes.lock()).await?;
            mailboxes_lck.clone()
        };
        loop {
            for (_, mailbox) in mailboxes.clone() {
                if let Some(ev) = examine_updates(mailbox, &mut conn).await? {
                    emitter.emit(ev).await;
                }
            }
            //[ref:FIXME]: make sleep duration configurable
            smol::Timer::after(Duration::from_secs(3 * 60)).await;
        }
    })
}

/// Whether `l` is IDLE keepalive/continuation noise: a continuation
/// request (`+ ...`, or a bare `+` without the trailing space — see
/// <https://github.com/modern-email/defects/issues/7>) or an untagged
/// `* OK` status line.
fn is_idling_noise(l: &[u8]) -> bool {
    is_continuation_request(l)
        || l.starts_with(b"* ok")
        || l.starts_with(b"* Ok")
        || l.starts_with(b"* OK")
}

/// Whether `l` is a continuation request line (`+ ...`, or a bare `+`
/// without the trailing space).
fn is_continuation_request(l: &[u8]) -> bool {
    l.starts_with(b"+ ") || l == b"+" || l == b"+\r\n"
}

pub fn idle(kit: ImapWatchKit) -> impl futures::stream::Stream<Item = Result<BackendEvent>> {
    // How long to wait for the tagged response to the IDLE terminator
    // `DONE`. A server that is going to answer does so promptly; anything
    // else is a wedged connection and the watch should restart with a new
    // one.
    const DONE_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
    // How long to wait for the `+` continuation of an IDLE command after
    // unexpected data has already arrived. Per RFC 2177 the server sends
    // the continuation immediately, so data racing ahead of it means the
    // continuation is in flight; only once it has arrived is it safe to
    // end the idle with `DONE` (a DONE that races the continuation can be
    // answered with a tagged BAD by servers whose idle state machine was
    // not ready yet).
    const IDLE_CONTINUATION_GRACE: Duration = Duration::from_secs(5);
    try_fn_stream(|emitter| async move {
        log::trace!("IDLE");
        /* IDLE only watches the connection's selected mailbox. We will IDLE on INBOX
         * and every `watch_sweep_interval` (5 minutes by default, see
         * `ImapServerConf::watch_sweep_interval`) wake up and poll the others */
        let ImapWatchKit {
            mut conn,
            main_conn,
            uid_store,
        } = kit;
        conn.connect().await?;
        let mailbox: ImapMailbox = {
            let mut retries = 0;
            loop {
                let inbox = uid_store
                    .mailboxes
                    .lock()
                    .await
                    .values()
                    .find(|f| {
                        f.parent.is_none() && (f.special_usage() == SpecialUsageMailbox::Inbox)
                    })
                    .cloned();
                match inbox {
                    Some(mailbox) => break mailbox,
                    None if retries >= 10 => {
                        return Err(Error::new(
                            "INBOX mailbox not found in local mailbox index. the connection might \
                             have not parsed the IMAP mailboxes correctly",
                        )
                        .set_kind(ErrorKind::TimedOut));
                    }
                    None => {
                        smol::Timer::after(Duration::from_millis(
                            retries * (4 * crate::utils::random::random_u8() as u64),
                        ))
                        .await;
                        retries += 1;
                    }
                }
            }
        };
        let mailbox_hash = mailbox.hash();
        // Single-session invariant: from this point on, the watched
        // mailbox must be persistently selected only by this (watch)
        // connection. Some servers stop pushing new-mail untagged `EXISTS`
        // updates to an IDLE session while more than one session holds
        // the mailbox selected, so the main connection (which the
        // warm-start initial fetch leaves holding the selection) and any
        // idle pooled connection must drop it. Transient selections by
        // user actions (a manual refresh, opening a mailbox) are
        // unaffected: they run on a connection and finish, and nothing
        // re-selects the watched mailbox on a second connection while the
        // watch is running.
        {
            let mut main_conn_lck = timeout(uid_store.timeout, main_conn.inner.lock()).await?;
            if main_conn_lck.stream.as_ref().is_ok_and(|s| {
                !matches!(s.current_mailbox.is(&mailbox_hash), MailboxSelection::None)
            }) {
                main_conn_lck.unselect().await?;
            }
        }
        // Pooled connections can carry the same stale selection; clear
        // those too. A busy slot belongs to an in-flight operation (a
        // transient selection by definition), so skipping it is correct;
        // a dead pooled connection has no server-side selection either,
        // so its errors are only logged.
        for slot in main_conn.pool.iter() {
            let Some(mut pooled_conn) = slot.try_lock() else {
                continue;
            };
            let Some(pooled_conn) = pooled_conn.as_mut() else {
                continue;
            };
            if pooled_conn.stream.as_ref().is_ok_and(|s| {
                !matches!(s.current_mailbox.is(&mailbox_hash), MailboxSelection::None)
            }) {
                if let Err(err) = pooled_conn.unselect().await {
                    log::trace!(
                        "{}: could not unselect the watched mailbox on a pooled connection: \
                         {}",
                        uid_store.account_name,
                        err
                    );
                }
            }
        }
        let mut response = Vec::with_capacity(8 * 1024);
        let select_response = conn
            .examine_mailbox(mailbox_hash, &mut response, true)
            .await?;
        {
            let mut mismatch = false;
            {
                let mut uidvalidities = uid_store.uidvalidity.lock().unwrap();

                if let Some(v) = uidvalidities.get(&mailbox_hash) {
                    mismatch = *v != select_response.uidvalidity;
                }
                uidvalidities.insert(mailbox_hash, select_response.uidvalidity);
            }
            if mismatch {
                emitter
                    .emit(
                        RefreshEvent {
                            account_hash: uid_store.account_hash,
                            mailbox_hash,
                            kind: RefreshEventKind::Rescan,
                        }
                        .into(),
                    )
                    .await;
            }
        }
        {
            // The server may have gained mail while no watcher was running
            // (or it may never deliver untagged EXISTS updates at all);
            // the EXAMINE response we just got may be the only signal.
            // Compensate by re-syncing before entering IDLE.
            let current_exists = mailbox.exists.lock().unwrap().len();
            if select_response.exists > current_exists {
                log::trace!(
                    "IDLE compensating resync: mailbox {} reports {} EXISTS but {} are known \
                     locally",
                    mailbox.path(),
                    select_response.exists,
                    current_exists
                );
                if let Some(ev) = examine_updates(Clone::clone(&mailbox), &mut conn).await? {
                    emitter.emit(ev).await;
                }
            }
        }
        let mailboxes: HashMap<MailboxHash, ImapMailbox> = {
            let mailboxes_lck = timeout(uid_store.timeout, uid_store.mailboxes.lock()).await?;
            mailboxes_lck.clone()
        };
        let heartbeat_interval = conn.server_conf.idle_heartbeat_interval;
        // Interval of the periodic sweep of the other mailboxes on the
        // main connection; read from the account configuration (default
        // 300 seconds) so tests can force frequent sweeps.
        let sweep_interval = conn.server_conf.watch_sweep_interval;
        // Cap the DONE-response wait so a wedged server fails fast: if the
        // user configured a shorter socket timeout, honor it too.
        let done_timeout = conn
            .server_conf
            .timeout
            .map(|t| t.min(DONE_RESPONSE_TIMEOUT))
            .unwrap_or(DONE_RESPONSE_TIMEOUT);
        // Cap the continuation grace the same way the DONE-response wait
        // is capped, so a short user-configured socket timeout bounds it
        // too.
        let continuation_grace = conn
            .server_conf
            .timeout
            .map(|t| t.min(IDLE_CONTINUATION_GRACE))
            .unwrap_or(IDLE_CONTINUATION_GRACE);
        conn.send_command(CommandBody::Idle).await?;
        let mut blockn = ImapBlockingConnection::from(conn);
        let mut watch = Instant::now();
        let mut events = vec![];
        // Whether the `+` continuation of the current IDLE session has been
        // seen (the greeting after entering IDLE, or any later
        // continuation/keepalive `+` line).
        let mut continuation_seen = false;
        loop {
            if !events.is_empty() {
                let events = BackendEvent::flatten(std::mem::take(&mut events));
                for ev in events {
                    emitter.emit(ev).await;
                }
            }
            let line = match timeout(Some(heartbeat_interval), blockn.read_line()).await {
                Ok(Some(line)) => line,
                Ok(None) => {
                    log::trace!("IDLE connection dropped: {:?}", blockn.err());
                    return Ok(());
                }
                Err(_) => {
                    /* Timeout */
                    log::trace!(
                        "IDLE heartbeat timed out after {heartbeat_interval:?}; unprocessed \
                         buffered bytes: {:?}",
                        String::from_utf8_lossy(
                            &blockn.buffered()[..blockn.buffered().len().min(200)]
                        )
                    );
                    blockn.conn.send_raw(b"DONE").await?;
                    if let Err(err) = timeout(
                        Some(done_timeout),
                        blockn
                            .conn
                            .read_response(&mut response, RequiredResponses::empty()),
                    )
                    .await
                    {
                        log::trace!("IDLE: no response to DONE within {done_timeout:?}: {err}");
                        return Err(err);
                    }
                    // The server may never deliver untagged updates during
                    // IDLE; the heartbeat wake-up may be the only chance to
                    // notice new mail, so re-sync the watched mailbox before
                    // going back to sleep.
                    if let Some(ev) =
                        examine_updates(Clone::clone(&mailbox), &mut blockn.conn).await?
                    {
                        emitter.emit(ev).await;
                    }
                    blockn.conn.send_command(CommandBody::Idle).await?;
                    continuation_seen = false;
                    let mut main_conn_lck = main_conn.lock().await?;
                    main_conn_lck.connect().await?;
                    continue;
                }
            };
            log::trace!(
                "IDLE received data: {:?}",
                String::from_utf8_lossy(&line[..line.len().min(300)])
            );
            let now = Instant::now();
            if now.duration_since(watch) >= sweep_interval {
                /* Time to poll all inboxes */
                let mut main_conn_lck = main_conn.lock().await?;
                for (h, mailbox) in mailboxes.clone() {
                    // Single-session invariant: the watched mailbox is
                    // persistently selected only on the watch connection,
                    // which covers it via IDLE plus the heartbeat
                    // compensation re-sync; examining it here would
                    // re-select it on a second connection and can make the
                    // server stop pushing new-mail updates to the IDLE
                    // session.
                    if h == mailbox_hash {
                        continue;
                    }
                    if let Some(ev) = examine_updates(mailbox, &mut main_conn_lck).await? {
                        events.push(ev);
                    }
                }
                watch = now;
            }
            if line.split_rn().filter(|l| !is_idling_noise(l)).count() == 0 {
                if line.split_rn().any(is_continuation_request) {
                    continuation_seen = true;
                }
                log::trace!("IDLE data was only keepalive/continuation lines, continuing");
                continue;
            }
            {
                log::trace!("IDLE push data received, sending DONE");
                let mut pending_lines: Vec<Vec<u8>> = vec![line];
                if !continuation_seen {
                    // The push data raced the `+` continuation of this
                    // IDLE session. Buffer it and only send DONE once the
                    // continuation arrived (or the bounded grace expired:
                    // servers answer IDLE with the continuation
                    // immediately, so it is in flight; a wedged server is
                    // then handled by the DONE-response read). No inline
                    // sleeps: every wait below is bounded by
                    // `continuation_grace`.
                    log::trace!(
                        "IDLE push data arrived before the `+` continuation; waiting up to \
                         {continuation_grace:?} for it before sending DONE"
                    );
                    let grace_deadline = Instant::now() + continuation_grace;
                    while !continuation_seen {
                        let remaining = grace_deadline.saturating_duration_since(Instant::now());
                        if remaining.is_zero() {
                            break;
                        }
                        match timeout(Some(remaining), blockn.read_line()).await {
                            Ok(Some(l)) => {
                                if l.split_rn().any(is_continuation_request) {
                                    log::trace!(
                                        "IDLE continuation arrived within the grace period"
                                    );
                                    continuation_seen = true;
                                } else if l.split_rn().any(|x| !is_idling_noise(x)) {
                                    pending_lines.push(l);
                                }
                            }
                            Ok(None) => {
                                log::trace!(
                                    "IDLE connection dropped while waiting for the \
                                     continuation: {:?}",
                                    blockn.err()
                                );
                                return Ok(());
                            }
                            Err(_) => {
                                /* Grace expired without the continuation; fall
                                 * through and send DONE anyway. */
                                break;
                            }
                        }
                    }
                }
                blockn.conn.send_raw(b"DONE").await?;
                let done_read = blockn
                    .conn
                    .read_response(&mut response, RequiredResponses::UNTAGGED)
                    .await;
                if let Err(err) = &done_read {
                    // Some servers answer the DONE terminator with a tagged
                    // BAD when it raced the `+` continuation (their idle
                    // state machine was not ready for it yet). That must not
                    // be fatal: restart the IDLE flow below (process the
                    // buffered push data and enter IDLE again) instead of
                    // surfacing a fatal error. `read_response` leaves the
                    // raw server reply in `response` when it converts a
                    // BAD/NO reply into an error, so the BAD can be
                    // recognized there.
                    if matches!(
                        super::protocol_parser::ImapResponse::try_from(response.as_slice()),
                        Ok(super::protocol_parser::ImapResponse::Bad(_))
                    ) {
                        log::trace!(
                            "IDLE: server answered DONE with a tagged BAD; restarting the \
                             IDLE flow: {err}"
                        );
                    } else {
                        return Err(err.clone());
                    }
                }
                for l in pending_lines
                    .iter()
                    .flat_map(|l| l.split_rn())
                    .chain(response.split_rn())
                {
                    log::trace!("process_untagged {:?}", String::from_utf8_lossy(l));
                    if is_idling_noise(l) {
                        continue;
                    }
                    if let Ok(Some(untagged_response)) =
                        super::protocol_parser::untagged_responses(l).map(|(_, v, _)| v)
                    {
                        if let Some(ev) = blockn.conn.process_untagged(untagged_response).await? {
                            events.push(ev);
                        }
                    }
                }
                blockn.conn.send_command(CommandBody::Idle).await?;
                continuation_seen = false;
            }
        }
    })
}

pub async fn examine_updates(
    mailbox: ImapMailbox,
    conn: &mut ImapConnection,
) -> Result<Option<BackendEvent>> {
    if mailbox.no_select {
        return Ok(None);
    }
    let mailbox_hash = mailbox.hash();
    log::trace!("examining mailbox {} {}", mailbox_hash, mailbox.path());
    if let Some(new_envelopes) = conn.resync(mailbox_hash).await? {
        Ok(new_envelopes
            .into_iter()
            .map(|env| RefreshEvent {
                mailbox_hash,
                account_hash: conn.uid_store.account_hash,
                kind: RefreshEventKind::Create(Box::new(env)),
            })
            .collect::<Vec<_>>()
            .try_into()
            .ok())
    } else {
        let mut response = Vec::with_capacity(8 * 1024);
        let select_response = conn
            .examine_mailbox(mailbox_hash, &mut response, true)
            .await?;
        {
            let mut uidvalidities = conn.uid_store.uidvalidity.lock().unwrap();

            if let Some(v) = uidvalidities.get(&mailbox_hash) {
                if *v != select_response.uidvalidity {
                    return Ok(Some(
                        RefreshEvent {
                            account_hash: conn.uid_store.account_hash,
                            mailbox_hash,
                            kind: RefreshEventKind::Rescan,
                        }
                        .into(),
                    ));
                }
            } else {
                uidvalidities.insert(mailbox_hash, select_response.uidvalidity);
            }
        }

        let current_exists = mailbox.exists.lock().unwrap().len();
        if mailbox.is_cold() {
            /* Mailbox hasn't been loaded yet */
            let has_list_status: bool = conn
                .uid_store
                .capabilities
                .lock()
                .unwrap()
                .iter()
                .any(|cap| cap.eq_ignore_ascii_case(b"LIST-STATUS"));
            if has_list_status {
                // [ref:TODO]: (#222) imap-codec does not support "LIST Command Extensions" currently.
                let Some(quoted_mailbox_path) = quote_imap_mailbox_path(mailbox.imap_path()) else {
                    log::warn!(
                        "Could not safely quote IMAP mailbox path {:?}; skipping LIST-STATUS \
                         update.",
                        mailbox.imap_path()
                    );
                    mailbox.set_warm(true);
                    return Ok(None);
                };
                conn.send_command_raw(
                    format!(
                        "LIST {} \"\" RETURN (STATUS (MESSAGES UNSEEN))",
                        quoted_mailbox_path
                    )
                    .as_bytes(),
                )
                .await?;
                conn.read_response(
                    &mut response,
                    RequiredResponses::LIST | RequiredResponses::STATUS,
                )
                .await?;
                log::trace!(
                    "list return status out: {}",
                    String::from_utf8_lossy(&response)
                );
                for l in response.split_rn() {
                    if !l.starts_with(b"*") {
                        continue;
                    }
                    if let Ok(status) = protocol_parser::status_response(l).map(|(_, v)| v) {
                        if Some(mailbox_hash) == status.mailbox {
                            if let Some(total) = status.messages {
                                if let Ok(mut exists_lck) = mailbox.exists.lock() {
                                    exists_lck.clear();
                                    exists_lck.set_not_yet_seen(total);
                                }
                            }
                            if let Some(total) = status.unseen {
                                if let Ok(mut unseen_lck) = mailbox.unseen.lock() {
                                    unseen_lck.clear();
                                    unseen_lck.set_not_yet_seen(total);
                                }
                            }
                            break;
                        }
                    }
                }
            } else {
                conn.send_command(CommandBody::search(None, SearchKey::Unseen.into(), false))
                    .await?;
                conn.read_response(&mut response, RequiredResponses::SEARCH)
                    .await?;
                let unseen_count = protocol_parser::search_results(&response)?.1.len();
                if let Ok(mut exists_lck) = mailbox.exists.lock() {
                    exists_lck.clear();
                    exists_lck.set_not_yet_seen(select_response.exists);
                }
                if let Ok(mut unseen_lck) = mailbox.unseen.lock() {
                    unseen_lck.clear();
                    unseen_lck.set_not_yet_seen(unseen_count);
                }
            }
            mailbox.set_warm(true);
            return Ok(None);
        }

        if select_response.recent > 0 {
            /* UID SEARCH RECENT */
            conn.send_command(CommandBody::search(None, SearchKey::Recent.into(), true))
                .await?;
            conn.read_response(&mut response, RequiredResponses::SEARCH)
                .await?;
            let v = protocol_parser::search_results(response.as_slice()).map(|(_, v)| v)?;
            if v.is_empty() {
                log::trace!(
                    "search response was empty: {}",
                    String::from_utf8_lossy(&response)
                );
                return Ok(None);
            }
            let (required_responses, attributes) = common_attributes();
            conn.send_command(CommandBody::fetch(v.as_slice(), attributes, true)?)
                .await?;
            conn.read_response(&mut response, required_responses)
                .await?;
        } else if select_response.exists > current_exists {
            let min = current_exists.max(1);

            let (required_responses, attributes) = common_attributes();
            conn.send_command(CommandBody::fetch(min.., attributes, false)?)
                .await?;
            conn.read_response(&mut response, required_responses)
                .await?;
        } else {
            return Ok(None);
        }
        log::trace!(
            "fetch response is {} bytes and {} lines",
            response.len(),
            String::from_utf8_lossy(&response).lines().count()
        );
        let (_, mut v, _) = protocol_parser::fetch_responses(&response)?;
        log::trace!("responses len is {}", v.len());
        if v.is_empty() {
            return Ok(None);
        }
        for FetchResponse {
            ref uid,
            ref mut envelope,
            ref mut flags,
            ref references,
            ..
        } in v.iter_mut()
        {
            // A FETCH response parsed across literal-continuation lines can
            // be missing its UID or ENVELOPE item; skip such malformed
            // entries instead of panicking on the `unwrap()`s below.
            if uid.is_none() || envelope.is_none() {
                continue;
            }
            let uid = uid.unwrap();
            let env = envelope.as_mut().unwrap();
            env.set_hash(generate_envelope_hash(mailbox.imap_path(), &uid));
            if let Some(value) = references {
                env.set_references(value);
            }
            let mut tag_lck = conn.uid_store.collection.tag_index.write().unwrap();
            if let Some((flags, keywords)) = flags {
                env.set_flags(*flags);
                if !env.is_seen() {
                    mailbox.unseen.lock().unwrap().insert_new(env.hash());
                }
                mailbox.exists.lock().unwrap().insert_new(env.hash());
                for f in keywords {
                    let hash = TagHash::from_bytes(f.as_bytes());
                    tag_lck.entry(hash).or_insert_with(|| f.to_string());
                    env.tags_mut().insert(hash);
                }
            }
        }
        {
            conn.uid_store
                .insert_envelopes(mailbox_hash, &v)
                .or_else(ignore_not_found)
                .chain_err_summary(|| {
                    format!(
                        "Could not save envelopes in cache for mailbox {}",
                        mailbox.imap_path()
                    )
                })?;
        }

        let mut events = Vec::with_capacity(v.len());

        for FetchResponse {
            uid,
            envelope,
            message_sequence_number,
            ..
        } in v
        {
            if uid.is_none() || envelope.is_none() {
                continue;
            }
            let uid = uid.unwrap();
            if conn
                .uid_store
                .uid_index
                .lock()
                .unwrap()
                .contains_key(&(mailbox_hash, uid))
            {
                continue;
            }
            let env = envelope.unwrap();
            log::trace!(
                "Create event {} {} {}",
                env.hash(),
                env.subject(),
                mailbox.path(),
            );
            conn.uid_store
                .msn_index
                .lock()
                .unwrap()
                .entry(mailbox_hash)
                .or_default()
                .insert(message_sequence_number, uid);
            conn.uid_store
                .hash_index
                .lock()
                .unwrap()
                .insert(env.hash(), (uid, mailbox_hash));
            conn.uid_store
                .uid_index
                .lock()
                .unwrap()
                .insert((mailbox_hash, uid), env.hash());
            events.push(RefreshEvent {
                account_hash: conn.uid_store.account_hash,
                mailbox_hash,
                kind: Create(Box::new(env)),
            });
        }
        Ok(events.try_into().ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quote_imap_mailbox_path_clean_path_is_byte_identical() {
        assert_eq!(
            quote_imap_mailbox_path("INBOX").as_deref(),
            Some("\"INBOX\"")
        );
        assert_eq!(
            quote_imap_mailbox_path("Archive/2024").as_deref(),
            Some("\"Archive/2024\"")
        );
    }

    #[test]
    fn test_quote_imap_mailbox_path_escapes_quote_and_backslash() {
        assert_eq!(
            quote_imap_mailbox_path("we\"ird").as_deref(),
            Some("\"we\\\"ird\"")
        );
        assert_eq!(
            quote_imap_mailbox_path("back\\slash").as_deref(),
            Some("\"back\\\\slash\"")
        );
        assert_eq!(
            quote_imap_mailbox_path("a\"b\\c").as_deref(),
            Some("\"a\\\"b\\\\c\"")
        );
    }

    #[test]
    fn test_quote_imap_mailbox_path_rejects_crlf() {
        assert_eq!(quote_imap_mailbox_path("foo\r\nbar"), None);
        assert_eq!(quote_imap_mailbox_path("foo\rbar"), None);
        assert_eq!(quote_imap_mailbox_path("foo\nbar"), None);
        assert_eq!(quote_imap_mailbox_path("\r\n"), None);
    }

    #[test]
    fn test_full_list_command_with_escaped_path() {
        let quoted = quote_imap_mailbox_path("a\"b\\c").unwrap();
        assert_eq!(
            format!("LIST {} \"\" RETURN (STATUS (MESSAGES UNSEEN))", quoted),
            "LIST \"a\\\"b\\\\c\" \"\" RETURN (STATUS (MESSAGES UNSEEN))"
        );
    }

    #[test]
    fn test_raw_interpolation_of_hostile_path_is_rejected() {
        // Pre-fix behavior interpolated the path raw into the command,
        // letting a hostile path (from server LIST responses) terminate the
        // quoted string early and inject commands. Such paths must be
        // rejected outright.
        let evil = "foo\" RETURN (STATUS (MESSAGES 1))\r\nA002 EXPUNGE";
        assert_eq!(quote_imap_mailbox_path(evil), None);
    }
}
