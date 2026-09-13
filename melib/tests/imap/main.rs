//
// meli
//
// Copyright 2025 Emmanouil Pitsidianakis <manos@pitsidianak.is>
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

#![cfg(feature = "imap")]

use rusty_fork::rusty_fork_test;

rusty_fork_test! {
    #[test]
    fn test_imap_watch() {
        tests::run_imap_watch();
    }

    #[test]
    fn test_imap_refresh_after_initial_fetch_new_mail() {
        tests::run_imap_refresh_after_initial_fetch();
    }

    #[test]
    fn test_imap_watch_after_initial_fetch_new_mail() {
        tests::run_imap_watch_after_initial_fetch();
    }

    #[test]
    fn test_imap_refresh_status_stale_when_selected() {
        tests::run_imap_refresh_status_stale_when_selected();
    }

    #[test]
    fn test_imap_watch_idle_no_push() {
        tests::run_imap_watch_idle_no_push();
    }

    #[test]
    fn test_imap_watch_push_glued_to_idling_greeting() {
        tests::run_imap_watch_push_glued_to_idling_greeting();
    }

    /// Replay fixture: the IDLE-arm glued payload uses the byte shape
    /// captured from the real server's push path. See
    /// `tests::run_imap_watch_replay_real_push_bytes`.
    #[test]
    fn test_imap_watch_replay_real_push_bytes() {
        tests::run_imap_watch_replay_real_push_bytes();
    }

    #[test]
    fn test_imap_watch_startup_compensation() {
        tests::run_imap_watch_startup_compensation();
    }

    #[test]
    fn test_imap_watch_push_exists_multi_mail() {
        tests::run_imap_watch_push_exists_multi_mail();
    }

    #[test]
    fn test_imap_watch_uidvalidity_change_rescan() {
        tests::run_imap_watch_uidvalidity_change_rescan();
    }

    #[test]
    fn test_imap_watch_done_no_response_errors() {
        tests::run_imap_watch_done_no_response_errors();
    }

    #[test]
    fn test_imap_watch_sweep_interval_conf_default() {
        tests::run_imap_watch_sweep_interval_conf_default();
    }

    #[test]
    fn test_imap_watch_sweep_interval_short_sweep_fires() {
        tests::run_imap_watch_sweep_interval_short_sweep_fires();
    }

    /// Pins the sweep's main-connection re-selection of the watched
    /// mailbox under the multi-session push suppression mock mode,
    /// plus the startup UNSELECT half of the single-session invariant.
    /// Was RED pre-T6 (archived in
    /// `.omo/evidence/fix-imap-idle-push/task-4-gated-mocks.txt`). See
    /// `tests::run_imap_watch_multi_session_no_push`.
    #[test]
    fn test_imap_watch_multi_session_no_push() {
        tests::run_imap_watch_multi_session_no_push();
    }

    /// Failing-first pin for the watch-startup half of the single-session
    /// invariant (T6): the main connection must UNSELECT the watched
    /// mailbox at watch startup. See `tests::run_imap_watch_startup_unselect`.
    #[test]
    fn test_imap_watch_startup_unselect() {
        tests::run_imap_watch_startup_unselect();
    }

    #[test]
    fn test_imap_watch_id_gated_push() {
        tests::run_imap_watch_id_gated_push();
    }

    /// Failing-first pin for the tag-not-last DONE reply framing: the
    /// tagged reply and a trailing `* EXISTS` push arrive in one TCP
    /// write; `read_lines` must stop at the tag line and the trailing
    /// push must still be processed.
    #[test]
    fn test_imap_watch_tag_not_last() {
        tests::run_imap_watch_tag_not_last();
    }

    /// Failing-first pin for the DONE-before-continuation hazard: a push
    /// arrives before the `+ idling` continuation and the premature DONE
    /// is answered with a tagged BAD; the push must still be delivered
    /// and the session must recover.
    #[test]
    fn test_imap_watch_push_before_continuation() {
        tests::run_imap_watch_push_before_continuation();
    }

    /// Failing-first pin for the bare `+` keepalive line: it must be
    /// filtered as keepalive noise (no DONE sent for it) and the watch
    /// must keep working afterwards.
    #[test]
    fn test_imap_watch_bare_plus_keepalive() {
        tests::run_imap_watch_bare_plus_keepalive();
    }

    /// Regression pin (expected GREEN): a `* BYE` arriving mid-IDLE must
    /// surface as a disconnect, never a hang.
    #[test]
    fn test_imap_watch_bye_mid_idle_unselected() {
        tests::run_imap_watch_bye_mid_idle_unselected();
    }

    #[cfg(feature = "sqlite3")]
    #[test]
    fn test_imap_resync_status_shortcircuit_hit() {
        tests::run_imap_resync_status_shortcircuit(true);
    }

    #[cfg(feature = "sqlite3")]
    #[test]
    fn test_imap_resync_status_shortcircuit_miss() {
        tests::run_imap_resync_status_shortcircuit(false);
    }

    /// Failing-first pin for the UID-less `UID FETCH` reply regression:
    /// a server that omits the mandatory `UID` data item from a `UID
    /// FETCH` reply (RFC 3501 §6.4.8 violation) must make the FLAGS
    /// resync fail with a protocol error; it used to panic on an
    /// `Option::unwrap()` in `resync_basic`. The RED run's panic output
    /// is archived in
    /// `.omo/evidence/fix-imap-idle-push/task-d2-uid-panic.txt`. See
    /// `tests::run_imap_uid_fetch_reply_without_uid_item`.
    #[cfg(feature = "sqlite3")]
    #[test]
    fn test_imap_uid_fetch_reply_without_uid_item() {
        tests::run_imap_uid_fetch_reply_without_uid_item();
    }

    /// Regression pin for the UID-less `UID FETCH` reply regression on
    /// the envelope-fetch path (resync Step 2i): a server that omits
    /// the mandatory `UID` data item from *every* `UID FETCH` reply
    /// (RFC 3501 §6.4.8 violation, via the general
    /// `uid_fetch_drop_uid_all` mock flag) must never panic the
    /// client; the resync must fail cleanly with a protocol error. The
    /// protocol error surfaces at the FLAGS resync step (Step 2ii):
    /// the UID-less envelope reply of Step 2i is absorbed by the
    /// client's untagged FETCH handling, which resolves the sequence
    /// number with a `UID SEARCH` (so the Step 2i `uid.unwrap()` guard
    /// is not reached; see the task evidence file for the reachability
    /// analysis). See
    /// `tests::run_imap_uid_fetch_reply_without_uid_item_envelope`.
    #[cfg(feature = "sqlite3")]
    #[test]
    fn test_imap_uid_fetch_reply_without_uid_item_envelope() {
        tests::run_imap_uid_fetch_reply_without_uid_item_envelope();
    }

    #[cfg(feature = "sqlite3")]
    #[test]
    fn test_imap_fetch_cache_first_before_select() {
        tests::run_imap_fetch_cache_first(true);
    }

    #[cfg(feature = "sqlite3")]
    #[test]
    fn test_imap_fetch_cache_first_empty_cache_commands_unchanged() {
        tests::run_imap_fetch_cache_first(false);
    }

    #[cfg(feature = "sqlite3")]
    #[test]
    fn test_imap_fetch_msn_index_persisted() {
        tests::run_imap_fetch_msn_index_persisted(true);
    }

    #[cfg(feature = "sqlite3")]
    #[test]
    fn test_imap_fetch_msn_index_uidvalidity_change() {
        tests::run_imap_fetch_msn_index_persisted(false);
    }

    #[cfg(feature = "sqlite3")]
    #[test]
    fn test_imap_fetch_body_structure_disabled() {
        tests::run_imap_fetch_body_structure_flag(false);
    }

    #[cfg(feature = "sqlite3")]
    #[test]
    fn test_imap_fetch_body_structure_default_commands_unchanged() {
        tests::run_imap_fetch_body_structure_flag(true);
    }

    #[cfg(feature = "sqlite3")]
    #[test]
    fn test_imap_fetch_no_cache_offline_errors() {
        tests::run_imap_fetch_no_cache_offline_errors();
    }

    #[cfg(feature = "sqlite3")]
    #[test]
    fn test_imap_mailboxes_cache_first() {
        tests::run_imap_mailboxes_cache_first();
    }

    #[cfg(feature = "sqlite3")]
    #[test]
    fn test_imap_refresh_mailboxes_live() {
        tests::run_imap_refresh_mailboxes_live();
    }

    #[cfg(feature = "sqlite3")]
    #[test]
    fn test_imap_offline_startup_uses_cache() {
        tests::run_imap_offline_startup_uses_cache();
    }
}

pub mod server {
    use std::{
        convert::TryInto,
        net::{TcpListener, TcpStream},
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc, Mutex,
        },
        time::Duration,
    };

    use futures::{
        channel::mpsc::{UnboundedReceiver, UnboundedSender},
        executor::block_on,
        future::{self, Either},
        io::{AsyncReadExt, AsyncWriteExt},
        pin_mut, Future, StreamExt,
    };
    use imap_codec::{
        encode::{Encoder, Fragment},
        imap_types, ResponseCodec,
    };
    use imap_types::{
        core::{IString, Literal, LiteralMode, NString},
        fetch::MessageDataItem,
        response::{Data, Response},
    };
    use melib::{backends::prelude::*, imap::*, parser::BytesExt, smol::Async, Mail};

    pub enum SessionState {
        // Unauthenticated,
        Authenticated,
        SelectedMailbox,
    }

    /// Server state with only one mailbox (INBOX).
    #[derive(Default)]
    pub struct ServerState {
        pub envelopes: IndexMap<UID, Mail>,
        pub next_uid: UID,
        pub uidvalidity: UID,
        /// RFC 4549 §4.3.2 permits servers to answer `STATUS` for the
        /// connection's currently selected mailbox from the state at
        /// `SELECT` time. When this is `true`, the mock emulates such a
        /// server: `STATUS` replies with the counters captured when the
        /// mailbox was selected on this connection, not the live ones.
        pub stale_status_when_selected: bool,
        /// When `true` (together with `push_glued_at_idle_start`), the
        /// glued IDLE-greeting payload is written in the byte shape
        /// captured from the real server's push path (see
        /// `tests::run_imap_watch_replay_real_push_bytes` for the capture
        /// provenance): every glued `* n EXISTS` line is paired with a
        /// `* 0 RECENT` line, matching how the captured server pairs
        /// `EXISTS` with `RECENT` in every EXISTS-bearing response.
        pub replay_real_push_bytes: bool,
        /// Some servers never deliver untagged updates during IDLE. When
        /// this is `true`, the mock emulates such a server: `ServerEvent::New`
        /// updates the state but does not write `* EXISTS` on the IDLE
        /// connection.
        pub idle_no_push: bool,
        /// When `true`, mail delivered via `ServerEvent::New` before the IDLE
        /// command is buffered and then written glued to the `+ idling`
        /// greeting as a single write (`"+ idling\r\n* n EXISTS\r\n"`),
        /// emulating a server push that arrives in the same TCP segment as
        /// the IDLE continuation response.
        pub push_glued_at_idle_start: bool,
        /// When `true`, the mock never answers the IDLE terminator `DONE`
        /// with a tagged OK (it stays silent), emulating servers that hang
        /// on DONE. The client must fail the DONE response read after its
        /// timeout and restart the watch.
        pub ignore_done: bool,
        /// Some servers do not push new-mail untagged updates to an IDLE
        /// session while more than one session has the mailbox selected
        /// (e.g. they only push to the single most recent session, or
        /// suppress fan-out under multi-session load). When this is `true`,
        /// the mock emulates such a server: `ServerEvent::New` updates the
        /// state but does not write `* EXISTS` on the IDLE connection
        /// while `selected_sessions` counts more than one session holding
        /// a selection on the watched mailbox. With exactly one session
        /// selected, pushes behave normally.
        pub suppress_push_when_multi_selected: bool,
        /// Some servers only push new-mail untagged updates during IDLE to
        /// sessions that identified themselves with the RFC 2971 `ID`
        /// command. When this is `true`, the mock emulates such a server:
        /// a connection that never sent `ID` does not receive `* EXISTS`
        /// pushes.
        pub id_gated_push: bool,
        /// When `true`, the mock answers the IDLE terminator `DONE` with a
        /// single TCP write that glues an `* n EXISTS` untagged line after
        /// the tagged OK reply
        /// (`"M{k} OK IDLE terminated\r\n* n EXISTS\r\n"`), emulating a
        /// server whose new-mail push races the tagged completion and
        /// lands in the same segment after it (tag-not-last framing).
        pub glue_exists_after_done: bool,
        /// When `true`, the mock emulates a server that pushes untagged
        /// data before the IDLE continuation: mail delivered via
        /// `ServerEvent::New` while not idling is buffered, and when the
        /// client next sends `IDLE` the buffered `* n EXISTS` lines are
        /// written *before* the `+ idling` continuation. Additionally,
        /// the first `DONE` terminator is answered with a tagged BAD (the
        /// server's idle state machine was not ready for a DONE that
        /// raced the continuation).
        pub push_before_continuation: bool,
        /// Per-mailbox count of sessions currently holding a selection on
        /// that mailbox: `SELECT`/`EXAMINE` increment it for this
        /// connection (only on the transition from not-selected),
        /// `UNSELECT`/`CLOSE`/connection drop decrement it. Used by
        /// `suppress_push_when_multi_selected`.
        pub selected_sessions: std::collections::HashMap<String, usize>,
        /// When `Some(name)`, `LIST \"\" *` additionally advertises this
        /// second mailbox and the server answers its `EXAMINE`/`SELECT`.
        /// Tests that need more than one mailbox (e.g. the watch sweep
        /// seam test, which must observe the sweep firing on a mailbox
        /// other than the watched one) set it; the default `None` keeps
        /// the historical single-`inbox` LIST replies for every other
        /// test.
        pub extra_mailbox: Option<String>,
        /// When `true`, the mock emulates a server that violates RFC 3501
        /// §6.4.8: replies to the `UID FETCH .. FLAGS` command omit the
        /// mandatory `UID` data item (the `* n FETCH` sequence number is
        /// still sent). The client must surface a protocol error, not
        /// panic.
        pub uid_fetch_flags_drop_uid: bool,
        /// When `true`, the mock emulates a server that violates RFC 3501
        /// §6.4.8 across the board: replies to *every* `UID FETCH`
        /// command (the envelope-fetch variants as well as the
        /// FLAGS-only variant) omit the mandatory `UID` data item (the
        /// `* n FETCH` sequence number is still sent). The client must
        /// surface a protocol error, not panic.
        pub uid_fetch_drop_uid_all: bool,
    }

    impl ServerState {
        pub fn insert(&mut self, new: Box<Mail>) -> (usize, UID) {
            let uid = self.next_uid;
            self.envelopes.insert(uid, *new);
            let msn = self.envelopes.len();
            self.next_uid += 1;
            (msn, uid)
        }

        /// Record that a session just selected `mailbox` (see
        /// `selected_sessions`).
        pub fn session_selected(&mut self, mailbox: &str) {
            *self
                .selected_sessions
                .entry(mailbox.to_string())
                .or_insert(0) += 1;
        }

        /// Record that a session stopped holding a selection on
        /// `mailbox` (see `selected_sessions`).
        pub fn session_unselected(&mut self, mailbox: &str) {
            if let Some(count) = self.selected_sessions.get_mut(mailbox) {
                *count = count.saturating_sub(1);
            }
        }

        /// How many sessions currently hold a selection on `mailbox` (see
        /// `selected_sessions`).
        pub fn selected_session_count(&self, mailbox: &str) -> usize {
            self.selected_sessions.get(mailbox).copied().unwrap_or(0)
        }
    }

    trait AsImapResponseItem {
        fn as_envelope(&'_ self) -> imap_types::envelope::Envelope<'_>;
        fn as_flags(&'_ self) -> Vec<imap_types::flag::FlagFetch<'_>>;
        fn as_bodystructure(&'_ self) -> imap_types::body::BodyStructure<'_>;
        fn as_body_peek_references(&self) -> imap_types::fetch::MessageDataItem<'_>;
    }

    impl AsImapResponseItem for Mail {
        fn as_envelope(&'_ self) -> imap_types::envelope::Envelope<'_> {
            macro_rules! address {
                ($a:expr) => {{
                    imap_types::envelope::Address {
                        name: $a.display_name().to_string().try_into().unwrap(),
                        adl: NString(None),
                        mailbox: $a
                            .get_email()
                            .split_once('@')
                            .unwrap()
                            .0
                            .to_string()
                            .try_into()
                            .unwrap(),
                        host: $a
                            .get_email()
                            .split_once('@')
                            .unwrap()
                            .1
                            .to_string()
                            .try_into()
                            .unwrap(),
                    }
                }};
            }
            imap_types::envelope::Envelope {
                date: self.date_as_str().try_into().unwrap(),
                subject: self.subject().as_ref().to_string().try_into().unwrap(),
                from: self.from().iter().map(|a| address! {a}).collect(),
                sender: self.from().iter().map(|a| address! {a}).collect(),
                reply_to: vec![],
                to: self.to().iter().map(|a| address! {a}).collect(),
                cc: self.cc().iter().map(|a| address! {a}).collect(),
                bcc: self.bcc().iter().map(|a| address! {a}).collect(),
                in_reply_to: NString(None),
                message_id: self.message_id().to_string().try_into().unwrap(),
            }
        }

        fn as_flags(&'_ self) -> Vec<imap_types::flag::FlagFetch<'_>> {
            let flags: Vec<imap_types::flag::Flag<'static>> = self.flags().into();
            flags
                .into_iter()
                .map(imap_types::flag::FlagFetch::Flag)
                .collect()
        }

        fn as_bodystructure(&'_ self) -> imap_types::body::BodyStructure<'_> {
            imap_types::body::BodyStructure::Single {
                body: imap_types::body::Body {
                    basic: imap_types::body::BasicFields {
                        parameter_list: vec![],
                        id: NString(None),
                        description: NString(None),
                        content_transfer_encoding: "7BIT".try_into().unwrap(),
                        size: self.bytes.len() as u32,
                    },
                    specific: imap_types::body::SpecificFields::Text {
                        subtype: "plain".try_into().unwrap(),
                        number_of_lines: 1,
                    },
                },
                extension_data: None,
            }
        }

        fn as_body_peek_references(&self) -> imap_types::fetch::MessageDataItem<'_> {
            imap_types::fetch::MessageDataItem::BodyExt {
                section: Some(imap_types::fetch::Section::HeaderFields(
                    None,
                    vec!["REFERENCES".try_into().unwrap()].try_into().unwrap(),
                )),
                origin: None,
                data: self
                    .other_headers()
                    .get(HeaderName::REFERENCES)
                    .map(|s| {
                        // Real servers usually answer `BODY[HEADER.FIELDS
                        // ..]` with a literal (the value can be long and
                        // is not required to be quotable); emit a literal
                        // so the reply framing matches production servers
                        // instead of a quoted string.
                        NString(Some(IString::Literal(Literal::unvalidated(
                            s.as_bytes().to_vec(),
                        ))))
                    })
                    .unwrap_or(NString(None)),
            }
        }
    }

    #[derive(Debug)]
    pub enum ServerEvent {
        New(Box<Mail>),
        Delete(UID),
        /// Write raw bytes to the client as-is (no framing added). Used to
        /// inject protocol edge-case lines (e.g. a bare `+\r\n`)
        /// independently of the mock's command machinery.
        RawLine(Vec<u8>),
        Quit,
    }

    pub struct ImapServerStream {
        pub tcp_stream: Async<TcpStream>,
        pub command_receiver: UnboundedReceiver<ServerEvent>,
        pub command_sender: UnboundedSender<ServerEvent>,
        pub state: Arc<Mutex<ServerState>>,
        pub session_state: SessionState,
        pub buf: Vec<u8>,
        /// Log of every command line received while not idling.
        pub received_commands: Arc<Mutex<Vec<String>>>,
        /// Whether the client sent an RFC 2971 `ID` command during the
        /// connection handshake (observed at the `M4` stage). With `use_id`
        /// disabled the client never sends `ID` at all.
        pub sent_id: bool,
        /// Number of bytes already read into `buf` before the loop handler
        /// starts: bytes of a client command that arrived at the `M4`
        /// handshake stage instead of `ID NIL` (a client with `use_id`
        /// disabled proceeds to its next command immediately). The loop
        /// handler must consume them before reading more from the socket.
        pub preread_len: usize,
        /// Whether the `M4` handshake stage observed the caller's driving
        /// future completing (possible with `use_id` disabled: the connect
        /// future is done after the `M3` capability reply). Callers must not
        /// await a future that already completed.
        pub driver_completed_in_handshake: bool,
        /// How many new-mail `* EXISTS` pushes this server connection has
        /// written on an IDLE session (see `ServerEvent::New` in the idle
        /// loop). Ground truth for asserting that a push was (not)
        /// delivered.
        pub idle_exists_pushes: Arc<AtomicUsize>,
        /// Log of every line received from the client while the connection
        /// is in the IDLE state. The `received_commands` log only covers
        /// the non-idling `'main` loop; this one records what the client
        /// sent while idling (e.g. a stray `DONE` terminator), which is
        /// ground truth for asserting that keepalive noise did not
        /// trigger a DONE.
        pub idle_received_lines: Arc<Mutex<Vec<String>>>,
        /// How many IDLE sessions this connection has entered; incremented
        /// each time the `+ idling` greeting is written. Ground truth for
        /// asserting the connection has entered (or re-entered) IDLE
        /// without racing the greeting against a client command.
        pub idle_sessions: Arc<AtomicUsize>,
        /// When `Some`, sleep before answering SELECT/EXAMINE commands.
        pub select_reply_delay: Option<Duration>,
        /// Set to `true` once the server has written a SELECT/EXAMINE
        /// reply. Used as ground truth for asserting that a client
        /// action happened before the server answered a SELECT.
        pub select_reply_sent: Arc<AtomicBool>,
        /// Ground truth of the exact bytes written at the IDLE-arm
        /// greeting seam (the `+ idling` continuation write, including
        /// any glued push lines). The replay fixture test asserts byte
        /// equality against the expected real-shape payload: fixture
        /// bytes are capture data, never interpreted.
        pub idle_greeting_bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl ImapServerStream {
        pub fn new<T: 'static, F: Future<Output = T> + std::marker::Unpin>(
            listener: &Async<TcpListener>,
            fut: F,
            (command_sender, command_receiver): (
                UnboundedSender<ServerEvent>,
                UnboundedReceiver<ServerEvent>,
            ),
            state: Arc<Mutex<ServerState>>,
        ) -> Self {
            let mut buf = vec![0; 64 * 1024];
            // Set by the `M4` handshake stage below (see the `sent_id`,
            // `preread_len` and `driver_completed_in_handshake` field
            // docs).
            let mut sent_id = false;
            let mut preread_len = 0_usize;
            let mut driver_completed_in_handshake = false;
            let tcp_stream = {
                let (mut tcp_stream, next_fut) = {
                    let accept_fut = listener.accept();
                    pin_mut!(accept_fut);

                    match block_on(future::select(fut, accept_fut)) {
                        Either::Left((_, _)) => {
                            unreachable!();
                        }
                        Either::Right((value2, fut)) => (value2.unwrap().0, fut),
                    }
                };
                {
                    let read_fut = tcp_stream.read(&mut buf);
                    pin_mut!(read_fut);
                    let next_fut = match block_on(future::select(next_fut, read_fut)) {
                        Either::Left((_, _)) => {
                            unreachable!();
                        }
                        Either::Right((value2, fut)) => {
                            let read_bytes = value2.unwrap();
                            assert_eq!(&buf[..read_bytes], b"M1 CAPABILITY\r\n");
                            fut
                        }
                    };
                    block_on(tcp_stream.write_all(
                            b"* CAPABILITY IMAP4rev1 AUTH=PLAIN SASL-IR\r\nM1 OK CAPABILITY completed\r\n",
                        ))
                        .unwrap();
                    let read_fut = tcp_stream.read(&mut buf);
                    pin_mut!(read_fut);
                    let next_fut = match block_on(future::select(next_fut, read_fut)) {
                        Either::Left((_, _)) => {
                            unreachable!();
                        }
                        Either::Right((value2, fut)) => {
                            let read_bytes = value2.unwrap();
                            assert_eq!(
                                &buf[..read_bytes],
                                b"M2 AUTHENTICATE PLAIN AHVzZXIAcGFzc3dvcmQ=\r\n"
                            );
                            fut
                        }
                    };
                    block_on(tcp_stream.write_all(b"M2 OK Success\r\n")).unwrap();
                    let read_fut = tcp_stream.read(&mut buf);
                    pin_mut!(read_fut);
                    let next_fut = match block_on(future::select(next_fut, read_fut)) {
                        Either::Left((_, _)) => {
                            unreachable!();
                        }
                        Either::Right((value2, next_fut)) => {
                            let read_bytes = value2.unwrap();
                            assert_eq!(&buf[..read_bytes], b"M3 CAPABILITY\r\n");
                            next_fut
                        }
                    };
                    block_on(tcp_stream.write_all(
                        b"* CAPABILITY IMAP4rev1 ID IDLE UNSELECT ENABLE\r\nM3 OK Success\r\n",
                    ))
                    .unwrap();
                    // The capability list above advertises `ID`, so a
                    // client with `use_id` enabled (melib's default) sends
                    // `M4 ID NIL` and waits for its reply before the connect
                    // future completes. Answer it here, while the caller is
                    // still driving the client future alone: the loop
                    // handler is not polled yet, and leaving ID unanswered
                    // would time the connection out.
                    //
                    // A client with `use_id` disabled sends no `ID` at
                    // all: its connect future completes on the `M3`
                    // capability reply alone (Either::Left — the pending
                    // read has consumed no bytes), while a longer driver
                    // future such as the watch stream stays pending and
                    // the client's next command (e.g. `M4 EXAMINE INBOX`)
                    // arrives instead (Either::Right, not `ID`). Record
                    // whether `ID` was seen (`sent_id`) and keep any
                    // other pre-read bytes for the loop handler
                    // (`preread_len`) so they are not lost. Callers must
                    // not blindly await their future afterwards when it
                    // may have completed here (see the `now_or_never`
                    // guards in the tests).
                    let read_fut = tcp_stream.read(&mut buf);
                    pin_mut!(read_fut);
                    match block_on(future::select(next_fut, read_fut)) {
                        Either::Left((_, _)) => {
                            // The driving future completed without any
                            // further client command: `use_id` is disabled
                            // and no `ID` was sent. Record that the
                            // caller's future is done.
                            driver_completed_in_handshake = true;
                        }
                        Either::Right((value2, _)) => {
                            let read_bytes = value2.unwrap();
                            if &buf[..read_bytes] == b"M4 ID NIL\r\n" {
                                block_on(tcp_stream.write_all(
                                    b"* ID (\"name\" \"mock\" \"version\" \"1.0\")\r\nM4 OK ID \
                                     completed\r\n",
                                ))
                                .unwrap();
                                sent_id = true;
                            } else {
                                // Not the `ID` command: the client has
                                // `use_id` disabled and already sent its
                                // next command. Hand the bytes to the
                                // loop handler instead of dropping them.
                                preread_len = read_bytes;
                            }
                        }
                    };
                    tcp_stream
                }
            };
            Self {
                tcp_stream,
                command_receiver,
                command_sender,
                state,
                session_state: SessionState::Authenticated,
                buf,
                received_commands: Arc::default(),
                sent_id,
                preread_len,
                driver_completed_in_handshake,
                idle_exists_pushes: Arc::default(),
                idle_received_lines: Arc::default(),
                idle_sessions: Arc::default(),
                select_reply_delay: None,
                select_reply_sent: Arc::default(),
                idle_greeting_bytes: Arc::default(),
            }
        }

        pub async fn loop_handler(self, name: &'static str) {
            let Self {
                mut tcp_stream,
                mut command_receiver,
                command_sender,
                state,
                mut session_state,
                mut buf,
                received_commands,
                sent_id,
                preread_len,
                idle_exists_pushes,
                idle_received_lines,
                idle_sessions,
                select_reply_delay,
                select_reply_sent,
                idle_greeting_bytes,
                driver_completed_in_handshake: _,
            } = self;
            let mut buf_start = 0;
            // Whether the first `DONE` of this connection was answered with
            // a tagged BAD (see `ServerState::push_before_continuation`).
            let mut done_bad_answered = false;
            // Bytes pre-read at the handshake `M4` stage (a `use_id`
            // disabled client's first command) must be consumed before
            // reading more from the socket.
            let mut buf_end = preread_len;
            // RFC 4549 §4.3.2 stale-STATUS emulation (see `ServerState` docs).
            let mut status_snapshot: Option<(usize, usize, UID, UID)> = None;
            // EXISTS count at the time of the connection's last SELECT/EXAMINE;
            // a NOOP must report the live count when it differs.
            let mut select_time_exists = 0_usize;
            // Buffered mail for `push_glued_at_idle_start` (see `ServerState`
            // docs).
            let mut pending_glued: Vec<Box<Mail>> = Vec::new();
            /// Outcome of a single `read_line` attempt: a complete
            /// CRLF-terminated line, bytes that do not yet form a line, or
            /// the peer having closed the connection. EOF must be
            /// distinguishable from "no line yet": a read on a closed
            /// socket is instantly ready with zero bytes, and looping on it
            /// starves the command channel (a `select` that sees the read
            /// future ready never polls the other future), busy-looping the
            /// loop handler forever.
            enum ReadOutcome<'a> {
                Line(&'a [u8]),
                Incomplete,
                Eof,
            }

            async fn read_line<'a>(
                tcp_stream: &mut Async<TcpStream>,
                buf: &'a mut [u8],
                start: &mut usize,
                end: &mut usize,
            ) -> ReadOutcome<'a> {
                // log::trace!(
                //     "read_line: buf={:?} start = {start:?} end = {end:?}",
                //     String::from_utf8_lossy(&buf[..*end])
                // );
                // Only wait for more socket bytes when no complete line is
                // buffered. A complete line can already sit at the front of
                // the buffer when the caller handed pre-read handshake
                // bytes to the loop handler (`preread_len`, a `use_id`
                // disabled client's first command); blocking on another
                // read there would deadlock the protocol.
                if !buf[*start..*end].contains_subsequence(b"\r\n") {
                    let read_bytes = tcp_stream.read(&mut buf[*end..]).await.unwrap();
                    if read_bytes == 0 {
                        return ReadOutcome::Eof;
                    }
                    *end += read_bytes;
                    // log::trace!(
                    //     "read_line: read_bytes = {read_bytes:?} buf = {:?}",
                    //     String::from_utf8_lossy(&buf[..*end])
                    // );
                    if !buf[*start..*end].contains_subsequence(b"\r\n") {
                        // log::trace!("read_line: returning None");
                        return ReadOutcome::Incomplete;
                    }
                    let Some(input) = buf[*start..*end].split_rn().next() else {
                        // log::trace!("read_line: returning None");
                        return ReadOutcome::Incomplete;
                    };
                    *start += input.len();
                    if *start == *end {
                        *start = 0;
                        *end = 0;
                    }
                    // log::trace!("read_line: returning {:?}", String::from_utf8_lossy(input));
                    ReadOutcome::Line(input)
                } else {
                    let rest = &buf[*start..*end];
                    let input = rest.split_rn().next().unwrap();
                    *start += input.len();
                    if *start == *end {
                        *start = 0;
                        *end = 0;
                    }
                    // log::trace!("read_line: returning {:?}", String::from_utf8_lossy(input));
                    ReadOutcome::Line(input)
                }
            }
            'outer: loop {
                let idle_cmd_id = 'main: loop {
                    let mut read_fut = Box::pin(read_line(
                        &mut tcp_stream,
                        &mut buf,
                        &mut buf_start,
                        &mut buf_end,
                    ));
                    let input = match future::select(&mut read_fut, command_receiver.next()).await {
                        Either::Left((value1, _)) => match value1 {
                            ReadOutcome::Eof => {
                                eprintln!(
                                    "{name} loop_handler: connection closed by peer in 'main"
                                );
                                break 'outer;
                            }
                            ReadOutcome::Incomplete => {
                                continue 'main;
                            }
                            ReadOutcome::Line(input) => {
                                drop(read_fut);
                                input
                            }
                        },
                        Either::Right((command, _)) => {
                            drop(read_fut);
                            let command = command.unwrap();
                            if matches!(command, ServerEvent::Quit) {
                                tcp_stream.write_all(b"* BYE world\r\n").await.unwrap();
                                tcp_stream.flush().await.unwrap();
                                eprintln!(
                                    "{name} loop_handler received ServerEvent::Quit from 'main"
                                );
                                break 'outer;
                            }
                            if let ServerEvent::RawLine(bytes) = command {
                                tcp_stream.write_all(&bytes).await.unwrap();
                                tcp_stream.flush().await.unwrap();
                                eprintln!(
                                    "{name} loop_handler wrote RawLine from 'main: {:?}",
                                    String::from_utf8_lossy(&bytes)
                                );
                                continue 'main;
                            }
                            if matches!(command, ServerEvent::New(_))
                                && (state.lock().unwrap().push_glued_at_idle_start
                                    || state.lock().unwrap().push_before_continuation)
                            {
                                // Buffer it: it will be glued to the IDLE
                                // greeting below instead of being written
                                // as a standalone push.
                                let ServerEvent::New(m) = command else {
                                    unreachable!();
                                };
                                pending_glued.push(m);
                                continue 'main;
                            }
                            command_sender.unbounded_send(command).unwrap();
                            continue 'main;
                        }
                    };
                    let line = String::from_utf8_lossy(input).to_string();
                    eprintln!("{name} loop_handler 'main received: {line:?}");
                    received_commands.lock().unwrap().push(line.clone());
                    let (id, cmd) = line.split_once(' ').unwrap();
                    match cmd {
                        "IDLE\r\n" => {
                            break 'main id.to_string();
                        }
                        "NOOP\r\n" => {
                            // RFC 3501 §6.1.2: NOOP flushes pending untagged
                            // updates for the selected mailbox.
                            let exists_now = state.lock().unwrap().envelopes.len();
                            if exists_now != select_time_exists {
                                tcp_stream
                                    .write_all(format!("* {exists_now} EXISTS\r\n").as_bytes())
                                    .await
                                    .unwrap();
                                select_time_exists = exists_now;
                            }
                            tcp_stream.write_all(id.as_bytes()).await.unwrap();
                            tcp_stream
                                .write_all(b" OK NOOP completed\r\n")
                                .await
                                .unwrap();
                            tcp_stream.flush().await.unwrap();
                        }
                        cmd if cmd.starts_with("ID ") => {
                            // RFC 2971 ID command; meli sends it by default
                            // since `use_id` now defaults to true.
                            tcp_stream
                                .write_all(b"* ID (\"name\" \"mock\" \"version\" \"1.0\")\r\n")
                                .await
                                .unwrap();
                            tcp_stream.write_all(id.as_bytes()).await.unwrap();
                            tcp_stream.write_all(b" OK ID completed\r\n").await.unwrap();
                            tcp_stream.flush().await.unwrap();
                        }
                        "LOGOUT\r\n" => {
                            tcp_stream.write_all(b"* BYE world\r\n").await.unwrap();
                            tcp_stream.write_all(id.as_bytes()).await.unwrap();
                            tcp_stream
                                .write_all(b" OK LOGOUT completed\r\n")
                                .await
                                .unwrap();
                            tcp_stream.flush().await.unwrap();
                            eprintln!("{name} loop_handler received LOGOUT from 'main");
                            break 'outer;
                        }
                        "LIST \"\" *\r\n" => {
                            tcp_stream
                                .write_all(b"* LIST () \"/\" \"inbox\"\r\n")
                                .await
                                .unwrap();
                            let extra_mailbox = state.lock().unwrap().extra_mailbox.clone();
                            if let Some(extra) = extra_mailbox {
                                tcp_stream
                                    .write_all(
                                        format!("* LIST () \"/\" \"{extra}\"\r\n").as_bytes(),
                                    )
                                    .await
                                    .unwrap();
                            }
                            tcp_stream.write_all(id.as_bytes()).await.unwrap();
                            tcp_stream
                                .write_all(b" OK LIST completed\r\n")
                                .await
                                .unwrap();
                            tcp_stream.flush().await.unwrap();
                        }
                        "LSUB \"\" *\r\n" => {
                            tcp_stream
                                .write_all(b"* LSUB () \".\" \"inbox\"\r\n")
                                .await
                                .unwrap();
                            tcp_stream.write_all(id.as_bytes()).await.unwrap();
                            tcp_stream
                                .write_all(b" OK LSUB completed\r\n")
                                .await
                                .unwrap();
                            tcp_stream.flush().await.unwrap();
                        }
                        "EXAMINE INBOX\r\n" => {
                            if let Some(delay) = select_reply_delay {
                                std::thread::sleep(delay);
                            }
                            if !matches!(session_state, SessionState::SelectedMailbox) {
                                // This session now also holds a selection
                                // on INBOX (see
                                // `ServerState::selected_sessions`).
                                state.lock().unwrap().session_selected("INBOX");
                            }
                            session_state = SessionState::SelectedMailbox;
                            let (exists, recent, uidvalidity, unseen, next_uid) = {
                                let state_lck = state.lock().unwrap();
                                let uidvalidity = state_lck.uidvalidity;
                                let exists = state_lck.envelopes.len();
                                let unseen = state_lck
                                    .envelopes
                                    .values()
                                    .filter(|env| !env.is_seen())
                                    .count();
                                let next_uid = state_lck.next_uid;
                                let recent = 0;
                                (exists, recent, uidvalidity, unseen, next_uid)
                            };
                            if state.lock().unwrap().stale_status_when_selected {
                                status_snapshot = Some((exists, unseen, next_uid, uidvalidity));
                            }
                            select_time_exists = exists;
                            tcp_stream
                                .write_all(
                                    format!(
                                        "* {exists} EXISTS\r\n* {recent} RECENT\r\n* OK \
                                         [UIDVALIDITY {uidvalidity}] UIDs valid\r\n* FLAGS \
                                         (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n* OK \
                                         [PERMANENTFLAGS ()] No permanent flags permitted\r\n{id} \
                                         OK [READ-ONLY] EXAMINE completed\r\n"
                                    )
                                    .as_bytes(),
                                )
                                .await
                                .unwrap();
                            tcp_stream.flush().await.unwrap();
                            select_reply_sent.store(true, Ordering::SeqCst);
                        }
                        "SELECT INBOX\r\n" => {
                            if let Some(delay) = select_reply_delay {
                                std::thread::sleep(delay);
                            }
                            if !matches!(session_state, SessionState::SelectedMailbox) {
                                // This session now also holds a selection
                                // on INBOX (see
                                // `ServerState::selected_sessions`).
                                state.lock().unwrap().session_selected("INBOX");
                            }
                            session_state = SessionState::SelectedMailbox;
                            let (exists, recent, uidvalidity, unseen, next_uid) = {
                                let state_lck = state.lock().unwrap();
                                let uidvalidity = state_lck.uidvalidity;
                                let exists = state_lck.envelopes.len();
                                let unseen = state_lck
                                    .envelopes
                                    .values()
                                    .filter(|env| !env.is_seen())
                                    .count();
                                let next_uid = state_lck.next_uid;
                                let recent = 0;
                                (exists, recent, uidvalidity, unseen, next_uid)
                            };
                            if state.lock().unwrap().stale_status_when_selected {
                                status_snapshot = Some((exists, unseen, next_uid, uidvalidity));
                            }
                            select_time_exists = exists;
                            tcp_stream
                                .write_all(
                                    format!(
                                        "* {exists} EXISTS\r\n* {recent} RECENT\r\n* OK \
                                         [UIDVALIDITY {uidvalidity}] UIDs valid\r\n* FLAGS \
                                         (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n* OK \
                                         [PERMANENTFLAGS ()] No permanent flags permitted\r\n{id} \
                                         OK SELECT completed\r\n"
                                    )
                                    .as_bytes(),
                                )
                                .await
                                .unwrap();
                            tcp_stream.flush().await.unwrap();
                            select_reply_sent.store(true, Ordering::SeqCst);
                        }
                        examine if examine.starts_with("EXAMINE ") => {
                            // A non-INBOX mailbox (see
                            // `ServerState::extra_mailbox`): the mailbox is
                            // always empty. INBOX-only session counting
                            // (`selected_sessions`) is unaffected by these.
                            session_state = SessionState::SelectedMailbox;
                            tcp_stream
                                .write_all(
                                    b"* 0 EXISTS\r\n* 0 RECENT\r\n* OK [UIDVALIDITY 1] \
                                     UIDs valid\r\n* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \
                                     \\Draft)\r\n* OK [PERMANENTFLAGS ()] No permanent \
                                     flags permitted\r\n",
                                )
                                .await
                                .unwrap();
                            tcp_stream.write_all(id.as_bytes()).await.unwrap();
                            tcp_stream
                                .write_all(b" OK [READ-ONLY] EXAMINE completed\r\n")
                                .await
                                .unwrap();
                            tcp_stream.flush().await.unwrap();
                        }
                        select if select.starts_with("SELECT ") => {
                            // A non-INBOX mailbox (see
                            // `ServerState::extra_mailbox`): the mailbox is
                            // always empty. INBOX-only session counting
                            // (`selected_sessions`) is unaffected by these.
                            session_state = SessionState::SelectedMailbox;
                            tcp_stream
                                .write_all(
                                    b"* 0 EXISTS\r\n* 0 RECENT\r\n* OK [UIDVALIDITY 1] \
                                     UIDs valid\r\n* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \
                                     \\Draft)\r\n* OK [PERMANENTFLAGS ()] No permanent \
                                     flags permitted\r\n",
                                )
                                .await
                                .unwrap();
                            tcp_stream.write_all(id.as_bytes()).await.unwrap();
                            tcp_stream
                                .write_all(b" OK SELECT completed\r\n")
                                .await
                                .unwrap();
                            tcp_stream.flush().await.unwrap();
                        }
                        "UNSELECT\r\n" => {
                            if !matches!(session_state, SessionState::SelectedMailbox) {
                                tcp_stream.write_all(id.as_bytes()).await.unwrap();
                                tcp_stream
                                    .write_all(b" BAD no mailbox is selected\r\n")
                                    .await
                                    .unwrap();
                                tcp_stream.flush().await.unwrap();
                                continue 'main;
                            }
                            session_state = SessionState::Authenticated;
                            status_snapshot = None;
                            state.lock().unwrap().session_unselected("INBOX");
                            tcp_stream.write_all(id.as_bytes()).await.unwrap();
                            tcp_stream
                                .write_all(b" OK UNSELECT succeeded\r\n")
                                .await
                                .unwrap();
                            tcp_stream.flush().await.unwrap();
                        }
                        "CLOSE\r\n" => {
                            if matches!(session_state, SessionState::SelectedMailbox) {
                                state.lock().unwrap().session_unselected("INBOX");
                            }
                            session_state = SessionState::Authenticated;
                            status_snapshot = None;
                            tcp_stream.write_all(id.as_bytes()).await.unwrap();
                            tcp_stream
                                .write_all(b" OK CLOSE succeeded\r\n")
                                .await
                                .unwrap();
                            tcp_stream.flush().await.unwrap();
                        }
                        "EXPUNGE\r\n" => {
                            if !matches!(session_state, SessionState::SelectedMailbox) {
                                tcp_stream.write_all(id.as_bytes()).await.unwrap();
                                tcp_stream
                                    .write_all(b" BAD no mailbox is selected\r\n")
                                    .await
                                    .unwrap();
                                tcp_stream.flush().await.unwrap();
                                continue 'main;
                            }
                            tcp_stream.write_all(id.as_bytes()).await.unwrap();
                            tcp_stream
                                .write_all(b" OK EXPUNGE succeeded\r\n")
                                .await
                                .unwrap();
                            tcp_stream.flush().await.unwrap();
                        }
                        "UID SEARCH 1:*\r\n" => {
                            if !matches!(session_state, SessionState::SelectedMailbox) {
                                tcp_stream.write_all(id.as_bytes()).await.unwrap();
                                tcp_stream
                                    .write_all(b" BAD no mailbox is selected\r\n")
                                    .await
                                    .unwrap();
                                tcp_stream.flush().await.unwrap();
                                continue 'main;
                            }
                            let uids = state
                                .lock()
                                .unwrap()
                                .envelopes
                                .iter()
                                .map(|(u, _)| *u)
                                .collect::<Vec<_>>();
                            if uids.is_empty() {
                                tcp_stream.write_all(b"* SEARCH\r\n").await.unwrap();
                            } else {
                                tcp_stream.write_all(b"* SEARCH ").await.unwrap();
                                for uid in uids {
                                    tcp_stream
                                        .write_all(format!("{uid}").as_bytes())
                                        .await
                                        .unwrap();
                                }
                                tcp_stream.write_all(b"\r\n").await.unwrap();
                            }
                            tcp_stream.write_all(id.as_bytes()).await.unwrap();
                            tcp_stream
                                .write_all(b" OK SEARCH completed\r\n")
                                .await
                                .unwrap();
                            tcp_stream.flush().await.unwrap();
                        }
                        uid_search_msn
                            if uid_search_msn.starts_with("UID SEARCH ")
                                && uid_search_msn.ends_with("\r\n") =>
                        {
                            // Single message-sequence-number search (e.g.
                            // `UID SEARCH 3`): the recovery path the
                            // client's untagged FETCH handler uses to
                            // resolve a UID-less FETCH reply's sequence
                            // number to its UID.
                            let msn_str =
                                &uid_search_msn["UID SEARCH ".len()..uid_search_msn.len() - 2];
                            if !matches!(session_state, SessionState::SelectedMailbox) {
                                tcp_stream.write_all(id.as_bytes()).await.unwrap();
                                tcp_stream
                                    .write_all(b" BAD no mailbox is selected\r\n")
                                    .await
                                    .unwrap();
                                tcp_stream.flush().await.unwrap();
                                continue 'main;
                            }
                            let uid = msn_str.parse::<usize>().ok().and_then(|msn| {
                                state
                                    .lock()
                                    .unwrap()
                                    .envelopes
                                    .get_index(msn.saturating_sub(1))
                                    .map(|(u, _)| *u)
                            });
                            match uid {
                                Some(uid) => {
                                    tcp_stream
                                        .write_all(format!("* SEARCH {uid}\r\n").as_bytes())
                                        .await
                                        .unwrap();
                                }
                                None => {
                                    tcp_stream.write_all(b"* SEARCH\r\n").await.unwrap();
                                }
                            }
                            tcp_stream.write_all(id.as_bytes()).await.unwrap();
                            tcp_stream
                                .write_all(b" OK SEARCH completed\r\n")
                                .await
                                .unwrap();
                            tcp_stream.flush().await.unwrap();
                        }
                        "SEARCH UNSEEN\r\n" => {
                            if !matches!(session_state, SessionState::SelectedMailbox) {
                                tcp_stream.write_all(id.as_bytes()).await.unwrap();
                                tcp_stream
                                    .write_all(b" BAD no mailbox is selected\r\n")
                                    .await
                                    .unwrap();
                                tcp_stream.flush().await.unwrap();
                                continue 'main;
                            }
                            let msns = state
                                .lock()
                                .unwrap()
                                .envelopes
                                .values()
                                .enumerate()
                                .filter(|(_, env)| !env.is_seen())
                                .map(|(i, _)| i + 1)
                                .collect::<Vec<_>>();
                            if msns.is_empty() {
                                tcp_stream.write_all(b"* SEARCH\r\n").await.unwrap();
                            } else {
                                tcp_stream.write_all(b"* SEARCH ").await.unwrap();
                                for msn in msns {
                                    tcp_stream
                                        .write_all(format!("{msn}").as_bytes())
                                        .await
                                        .unwrap();
                                }
                                tcp_stream.write_all(b"\r\n").await.unwrap();
                            }
                            tcp_stream.write_all(id.as_bytes()).await.unwrap();
                            tcp_stream
                                .write_all(b" OK SEARCH completed\r\n")
                                .await
                                .unwrap();
                            tcp_stream.flush().await.unwrap();
                        }
                        "STATUS INBOX (UIDNEXT)\r\n" => {
                            let uidnext = state.lock().unwrap().next_uid;
                            tcp_stream
                                .write_all(
                                    format!("* STATUS INBOX (UIDNEXT {uidnext})\r\n").as_bytes(),
                                )
                                .await
                                .unwrap();
                            tcp_stream.write_all(id.as_bytes()).await.unwrap();
                            tcp_stream
                                .write_all(b" OK STATUS completed\r\n")
                                .await
                                .unwrap();
                            tcp_stream.flush().await.unwrap();
                        }
                        status
                            if status.starts_with("STATUS INBOX (MESSAGES")
                                && status.ends_with(")\r\n") =>
                        {
                            let (messages, unseen, uidnext, uidvalidity) = status_snapshot
                                .unwrap_or_else(|| {
                                    let state_lck = state.lock().unwrap();
                                    (
                                        state_lck.envelopes.len(),
                                        state_lck
                                            .envelopes
                                            .values()
                                            .filter(|env| !env.is_seen())
                                            .count(),
                                        state_lck.next_uid,
                                        state_lck.uidvalidity,
                                    )
                                });
                            tcp_stream
                                .write_all(
                                    // The item order matches what melib's
                                    // `status_response` parser accepts.
                                    format!(
                                        "* STATUS INBOX (MESSAGES {messages} UIDNEXT {uidnext} \
                                         UIDVALIDITY {uidvalidity} UNSEEN {unseen})\r\n"
                                    )
                                    .as_bytes(),
                                )
                                .await
                                .unwrap();
                            tcp_stream.write_all(id.as_bytes()).await.unwrap();
                            tcp_stream
                                .write_all(b" OK STATUS completed\r\n")
                                .await
                                .unwrap();
                            tcp_stream.flush().await.unwrap();
                        }
                        fetch
                            if fetch.starts_with("FETCH ")
                                && fetch.ends_with(
                                    " (UID FLAGS ENVELOPE BODY.PEEK[HEADER.FIELDS (REFERENCES)] \
                                     BODYSTRUCTURE)\r\n",
                                ) =>
                        {
                            if !matches!(session_state, SessionState::SelectedMailbox) {
                                tcp_stream.write_all(id.as_bytes()).await.unwrap();
                                tcp_stream
                                    .write_all(b" BAD no mailbox is selected\r\n")
                                    .await
                                    .unwrap();
                                tcp_stream.flush().await.unwrap();
                                continue 'main;
                            }
                            let sequence_set = Self::parse_sequence_set(
                                fetch
                                    .strip_prefix("FETCH ")
                                    .unwrap()
                                    .strip_suffix(
                                        " (UID FLAGS ENVELOPE BODY.PEEK[HEADER.FIELDS \
                                         (REFERENCES)] BODYSTRUCTURE)\r\n",
                                    )
                                    .unwrap(),
                            );
                            // `*` in an MSN sequence set refers to the
                            // current EXISTS count.
                            let exists = state.lock().unwrap().envelopes.len() as u32;
                            let largest = std::num::NonZeroU32::new(exists.max(1)).unwrap();
                            eprintln!(
                                "{name} loop_handler got FETCH {sequence_set:?} (exists \
                                 {exists})"
                            );
                            for msn in sequence_set.iter(largest) {
                                let Some((uid, mail)) = state
                                    .lock()
                                    .unwrap()
                                    .envelopes
                                    .get_index(msn.get() as usize - 1)
                                    .map(|(u, m)| (*u, m.clone()))
                                else {
                                    tcp_stream.write_all(id.as_bytes()).await.unwrap();
                                    tcp_stream
                                        .write_all(b" BAD msn not found\r\n")
                                        .await
                                        .unwrap();
                                    tcp_stream.flush().await.unwrap();
                                    continue 'main;
                                };
                                let references = mail.as_body_peek_references();
                                let response = Response::Data(Data::Fetch {
                                    seq: msn,
                                    items: vec![
                                        MessageDataItem::Uid((uid as u32).try_into().unwrap()),
                                        MessageDataItem::Flags(mail.as_flags()),
                                        MessageDataItem::Envelope(mail.as_envelope()),
                                        MessageDataItem::BodyStructure(mail.as_bodystructure()),
                                        references,
                                    ]
                                    .try_into()
                                    .unwrap(),
                                });
                                eprintln!(
                                    "fragment raw: {:?}",
                                    String::from_utf8_lossy(
                                        &ResponseCodec::new().encode(&response).dump()
                                    )
                                );
                                for fragment in ResponseCodec::new().encode(&response) {
                                    match fragment {
                                        Fragment::Line { data } => {
                                            tcp_stream.write_all(&data).await.unwrap();
                                            tcp_stream.flush().await.unwrap();
                                        }
                                        Fragment::Literal { data, mode } => match mode {
                                            LiteralMode::Sync => {
                                                // Wait for a continuation request.
                                                todo!()
                                            }
                                            LiteralMode::NonSync => {
                                                // We don't need to wait for a continuation request
                                                // as the server will also not send it.
                                                tcp_stream.write_all(&data).await.unwrap();
                                                tcp_stream.flush().await.unwrap();
                                            }
                                        },
                                    }
                                }
                            }
                            tcp_stream
                                .write_all(format!("{id} OK FETCH completed\r\n").as_bytes())
                                .await
                                .unwrap();
                            tcp_stream.flush().await.unwrap();
                        }
                        uid_fetch
                            if uid_fetch.starts_with("UID FETCH ")
                                && uid_fetch.ends_with(" FLAGS\r\n") =>
                        {
                            if !matches!(session_state, SessionState::SelectedMailbox) {
                                tcp_stream.write_all(id.as_bytes()).await.unwrap();
                                tcp_stream
                                    .write_all(b" BAD no mailbox is selected\r\n")
                                    .await
                                    .unwrap();
                                tcp_stream.flush().await.unwrap();
                                continue 'main;
                            }
                            let sequence_set = Self::parse_sequence_set(
                                uid_fetch
                                    .strip_prefix("UID FETCH ")
                                    .unwrap()
                                    .strip_suffix(" FLAGS\r\n")
                                    .unwrap(),
                            );
                            eprintln!("{name} loop_handler got UID FETCH flags {sequence_set:?}");
                            let largest = state.lock().unwrap().next_uid.saturating_sub(1) as u32;
                            'uid_fetch_flags: for uid in
                                sequence_set.iter(largest.try_into().unwrap())
                            {
                                let Some(mail) = state
                                    .lock()
                                    .unwrap()
                                    .envelopes
                                    .get(&(uid.get() as usize))
                                    .cloned()
                                else {
                                    continue 'uid_fetch_flags;
                                };
                                let mut items = Vec::new();
                                if !state.lock().unwrap().uid_fetch_flags_drop_uid
                                    && !state.lock().unwrap().uid_fetch_drop_uid_all
                                {
                                    items.push(MessageDataItem::Uid(uid));
                                }
                                items.push(MessageDataItem::Flags(mail.as_flags()));
                                let response = Response::Data(Data::Fetch {
                                    seq: uid,
                                    items: items.try_into().unwrap(),
                                });
                                //eprintln!(
                                //    "fragment raw: {:?}",
                                //    String::from_utf8_lossy(
                                //        &ResponseCodec::new().encode(&response).dump()
                                //    )
                                //);
                                for fragment in ResponseCodec::new().encode(&response) {
                                    match fragment {
                                        Fragment::Line { data } => {
                                            tcp_stream.write_all(&data).await.unwrap();
                                            tcp_stream.flush().await.unwrap();
                                        }
                                        Fragment::Literal { data, mode } => match mode {
                                            LiteralMode::Sync => {
                                                // Wait for a continuation request.
                                                todo!()
                                            }
                                            LiteralMode::NonSync => {
                                                // We don't need to wait for a continuation request
                                                // as the server will also not send it.
                                                tcp_stream.write_all(&data).await.unwrap();
                                                tcp_stream.flush().await.unwrap();
                                            }
                                        },
                                    }
                                }
                            }
                            tcp_stream
                                .write_all(
                                    format!("{id} OK UID FETCH flags completed\r\n").as_bytes(),
                                )
                                .await
                                .unwrap();
                            tcp_stream.flush().await.unwrap();
                        }
                        uid_fetch
                            if uid_fetch.starts_with("UID FETCH ")
                                && (uid_fetch.ends_with(
                                    " (UID FLAGS ENVELOPE BODY.PEEK[HEADER.FIELDS (REFERENCES)] \
                                     BODYSTRUCTURE)\r\n",
                                ) || uid_fetch.ends_with(
                                    " (UID FLAGS ENVELOPE BODY.PEEK[HEADER.FIELDS \
                                     (REFERENCES)])\r\n",
                                )) =>
                        {
                            if !matches!(session_state, SessionState::SelectedMailbox) {
                                tcp_stream.write_all(id.as_bytes()).await.unwrap();
                                tcp_stream
                                    .write_all(b" BAD no mailbox is selected\r\n")
                                    .await
                                    .unwrap();
                                tcp_stream.flush().await.unwrap();
                                continue 'main;
                            }
                            // Reply with a BODYSTRUCTURE item only if the
                            // client requested one (`fetch_body_structure`).
                            let rest = uid_fetch.strip_prefix("UID FETCH ").unwrap();
                            let (sequence_set_str, with_body_structure) = rest
                                .strip_suffix(
                                    " (UID FLAGS ENVELOPE BODY.PEEK[HEADER.FIELDS \
                                     (REFERENCES)] BODYSTRUCTURE)\r\n",
                                )
                                .map(|s| (s, true))
                                .or_else(|| {
                                    rest.strip_suffix(
                                        " (UID FLAGS ENVELOPE BODY.PEEK[HEADER.FIELDS \
                                         (REFERENCES)])\r\n",
                                    )
                                    .map(|s| (s, false))
                                })
                                .unwrap();
                            let sequence_set = Self::parse_sequence_set(sequence_set_str);

                            eprintln!("{name} loop_handler got UID FETCH {sequence_set:?}");
                            let largest = state.lock().unwrap().next_uid.saturating_sub(1) as u32;
                            'uid_fetch: for uid in sequence_set.iter(largest.try_into().unwrap()) {
                                let Some(mail) = state
                                    .lock()
                                    .unwrap()
                                    .envelopes
                                    .get(&(uid.get() as usize))
                                    .cloned()
                                else {
                                    continue 'uid_fetch;
                                };
                                let references = mail.as_body_peek_references();
                                let mut items = Vec::new();
                                if !state.lock().unwrap().uid_fetch_drop_uid_all {
                                    items.push(MessageDataItem::Uid(uid));
                                }
                                items.push(MessageDataItem::Flags(mail.as_flags()));
                                items.push(MessageDataItem::Envelope(mail.as_envelope()));
                                items.push(references);
                                if with_body_structure {
                                    items.push(MessageDataItem::BodyStructure(
                                        mail.as_bodystructure(),
                                    ));
                                }
                                let response = Response::Data(Data::Fetch {
                                    seq: uid,
                                    items: items.try_into().unwrap(),
                                });
                                //eprintln!(
                                //    "fragment raw: {:?}",
                                //    String::from_utf8_lossy(
                                //        &ResponseCodec::new().encode(&response).dump()
                                //    )
                                //);
                                for fragment in ResponseCodec::new().encode(&response) {
                                    match fragment {
                                        Fragment::Line { data } => {
                                            tcp_stream.write_all(&data).await.unwrap();
                                            tcp_stream.flush().await.unwrap();
                                        }
                                        Fragment::Literal { data, .. } => {
                                            // A literal in a server
                                            // *response* is written
                                            // immediately: the
                                            // continuation-request
                                            // handshake of
                                            // `LiteralMode::Sync` only
                                            // applies to literals the
                                            // client sends in commands.
                                            tcp_stream.write_all(&data).await.unwrap();
                                            tcp_stream.flush().await.unwrap();
                                        }
                                    }
                                }
                            }
                            tcp_stream
                                .write_all(format!("{id} OK UID FETCH completed\r\n").as_bytes())
                                .await
                                .unwrap();
                            tcp_stream.flush().await.unwrap();
                        }
                        other => panic!("Unexpected cmd: {id} {other:?}"),
                    }
                };
                eprintln!("{name} loop_handler is now idling");
                idle_sessions.fetch_add(1, Ordering::SeqCst);
                if state.lock().unwrap().push_before_continuation && !pending_glued.is_empty() {
                    // Emulate a server whose new-mail push races the IDLE
                    // continuation: write the buffered `* EXISTS` lines
                    // *before* the `+ idling` continuation (see
                    // `ServerState::push_before_continuation`).
                    let mut pre_continuation: Vec<u8> = Vec::new();
                    for m in pending_glued.drain(..) {
                        let (msn, uid) = state.lock().unwrap().insert(m);
                        eprintln!(
                            "{name} push_before_continuation: EXISTS uid={uid} msn={msn} \
                             written before the idling continuation"
                        );
                        pre_continuation
                            .extend_from_slice(format!("* {msn} EXISTS\r\n").as_bytes());
                    }
                    tcp_stream.write_all(&pre_continuation).await.unwrap();
                    tcp_stream.flush().await.unwrap();
                    tcp_stream.write_all(b"+ idling\r\n").await.unwrap();
                    tcp_stream.flush().await.unwrap();
                } else {
                    let mut idling_greeting: Vec<u8> = b"+ idling\r\n".to_vec();
                    let replay_real_push_bytes = state.lock().unwrap().replay_real_push_bytes;
                    for m in pending_glued.drain(..) {
                        let (msn, uid) = state.lock().unwrap().insert(m);
                        eprintln!("{name} gluing EXISTS uid={uid} msn={msn} to IDLE greeting");
                        idling_greeting.extend_from_slice(format!("* {msn} EXISTS\r\n").as_bytes());
                        if replay_real_push_bytes {
                            // Real-capture shape (see
                            // `ServerState::replay_real_push_bytes` and the
                            // provenance comment in
                            // `run_imap_watch_replay_real_push_bytes`): the
                            // captured server pairs every `* n EXISTS` line
                            // with a `* 0 RECENT` line; replay that pairing
                            // at the push seam too.
                            idling_greeting.extend_from_slice(b"* 0 RECENT\r\n");
                        }
                    }
                    // Ground truth for the replay fixture test: record the
                    // exact bytes written at this seam. The fixture payload
                    // is capture data; the test asserts byte equality on
                    // this record instead of re-deriving it.
                    idle_greeting_bytes
                        .lock()
                        .unwrap()
                        .extend_from_slice(&idling_greeting);
                    tcp_stream.write_all(&idling_greeting).await.unwrap();
                    tcp_stream.flush().await.unwrap();
                }
                'idle: loop {
                    let mut read_fut = Box::pin(read_line(
                        &mut tcp_stream,
                        &mut buf,
                        &mut buf_start,
                        &mut buf_end,
                    ));
                    let input = match future::select(&mut read_fut, command_receiver.next()).await {
                        Either::Left((value1, _)) => match value1 {
                            ReadOutcome::Eof => {
                                eprintln!(
                                    "{name} loop_handler: connection closed by peer in 'idle"
                                );
                                break 'outer;
                            }
                            ReadOutcome::Incomplete => {
                                continue 'idle;
                            }
                            ReadOutcome::Line(input) => {
                                drop(read_fut);
                                input
                            }
                        },
                        Either::Right((value2, _)) => {
                            drop(read_fut);
                            match value2.unwrap() {
                                ServerEvent::New(new_mail) => {
                                    let exists_msn = {
                                        let mut state_lck = state.lock().unwrap();
                                        let (msn, new_uid) = state_lck.insert(new_mail);
                                        eprintln!("{name} EXISTS uid = {new_uid} msn = {msn}");
                                        msn
                                    };
                                    if state.lock().unwrap().idle_no_push {
                                        eprintln!(
                                            "{name} idle_no_push: suppressing EXISTS push on \
                                             the IDLE connection"
                                        );
                                        continue 'idle;
                                    }
                                    if state.lock().unwrap().id_gated_push && !sent_id {
                                        eprintln!(
                                            "{name} id_gated_push: connection never sent ID; \
                                             suppressing EXISTS push"
                                        );
                                        continue 'idle;
                                    }
                                    let suppress_when_multi_selected = {
                                        let state_lck = state.lock().unwrap();
                                        state_lck.suppress_push_when_multi_selected
                                            && state_lck.selected_session_count("INBOX") > 1
                                    };
                                    if suppress_when_multi_selected {
                                        eprintln!(
                                            "{name} suppress_push_when_multi_selected: {} \
                                             sessions hold INBOX selected; suppressing \
                                             EXISTS push",
                                            state.lock().unwrap().selected_session_count("INBOX")
                                        );
                                        continue 'idle;
                                    }
                                    idle_exists_pushes.fetch_add(1, Ordering::SeqCst);
                                    tcp_stream
                                        .write_all(format!("* {exists_msn} EXISTS\r\n").as_bytes())
                                        .await
                                        .unwrap();
                                    tcp_stream.flush().await.unwrap();
                                }
                                ServerEvent::Delete(uid) => {
                                    let msn = {
                                        let mut state_lck = state.lock().unwrap();
                                        let msn =
                                            state_lck.envelopes.get_index_of(&uid).unwrap() + 1;
                                        eprintln!(
                                            "{name} removing msn = {} uid = {} mail = {:?}",
                                            msn,
                                            uid,
                                            state_lck.envelopes.shift_remove(&uid)
                                        );
                                        msn
                                    };
                                    tcp_stream
                                        .write_all(format!("* {msn} EXPUNGE\r\n").as_bytes())
                                        .await
                                        .unwrap();
                                    tcp_stream.flush().await.unwrap();
                                }
                                ServerEvent::RawLine(bytes) => {
                                    tcp_stream.write_all(&bytes).await.unwrap();
                                    tcp_stream.flush().await.unwrap();
                                    eprintln!(
                                        "{name} loop_handler wrote RawLine while idling: {:?}",
                                        String::from_utf8_lossy(&bytes)
                                    );
                                }
                                ServerEvent::Quit => {
                                    tcp_stream.write_all(b"* BYE world\r\n").await.unwrap();
                                    tcp_stream.write_all(idle_cmd_id.as_bytes()).await.unwrap();
                                    tcp_stream
                                        .write_all(b" OK IDLE terminated\r\n")
                                        .await
                                        .unwrap();
                                    tcp_stream.flush().await.unwrap();
                                    eprintln!(
                                        "{name} loop_handler received ServerEvent::Quit from 'main"
                                    );
                                    break 'outer;
                                }
                            }
                            continue 'idle;
                        }
                    };
                    let input = String::from_utf8_lossy(input).to_string();
                    eprintln!("{name} loop_handler 'idle received: {input:?}");
                    idle_received_lines.lock().unwrap().push(input.clone());
                    if input == "DONE\r\n" {
                        if state.lock().unwrap().ignore_done {
                            eprintln!("{name} ignore_done: staying silent on DONE");
                            continue 'idle;
                        }
                        if state.lock().unwrap().push_before_continuation && !done_bad_answered {
                            // The server's idle state machine was not ready
                            // for a DONE that raced the continuation: answer
                            // with a tagged BAD (once per connection).
                            done_bad_answered = true;
                            eprintln!(
                                "{name} push_before_continuation: answering first DONE \
                                 with a tagged BAD"
                            );
                            tcp_stream.write_all(idle_cmd_id.as_bytes()).await.unwrap();
                            tcp_stream
                                .write_all(b" BAD Server not idling yet\r\n")
                                .await
                                .unwrap();
                            tcp_stream.flush().await.unwrap();
                            continue 'outer;
                        }
                        if state.lock().unwrap().glue_exists_after_done {
                            // Answer DONE and glue an `* EXISTS` push after
                            // the tagged reply in the same TCP write
                            // (tag-not-last framing).
                            let exists = state.lock().unwrap().envelopes.len();
                            eprintln!(
                                "{name} glue_exists_after_done: writing tagged OK with \
                                 * {exists} EXISTS glued after it"
                            );
                            tcp_stream
                                .write_all(
                                    format!(
                                        "{idle_cmd_id} OK IDLE terminated\r\n* {exists} \
                                         EXISTS\r\n"
                                    )
                                    .as_bytes(),
                                )
                                .await
                                .unwrap();
                            tcp_stream.flush().await.unwrap();
                            continue 'outer;
                        }
                        tcp_stream.write_all(idle_cmd_id.as_bytes()).await.unwrap();
                        tcp_stream
                            .write_all(b" OK IDLE terminated\r\n")
                            .await
                            .unwrap();
                        continue 'outer;
                    }
                }
            }
            // The connection is gone (drop, LOGOUT or ServerEvent::Quit):
            // if this session still held a selection on INBOX, release it
            // (see `ServerState::selected_sessions`).
            if matches!(session_state, SessionState::SelectedMailbox) {
                state.lock().unwrap().session_unselected("INBOX");
            }
        }

        fn parse_sequence_set(set: &str) -> imap_types::sequence::SequenceSet {
            use imap_types::sequence::{SeqOrUid, Sequence, SequenceSet};

            if set.contains(':') {
                let [a, b]: [SeqOrUid; 2] = set
                    .split(":")
                    .map(|n| {
                        if n == "*" {
                            SeqOrUid::Asterisk
                        } else {
                            SeqOrUid::Value(n.parse::<u32>().unwrap().try_into().unwrap())
                        }
                    })
                    .collect::<Vec<SeqOrUid>>()
                    .try_into()
                    .unwrap();
                SequenceSet::try_from(vec![Sequence::Range(a, b)]).unwrap()
            } else {
                let item = set.parse::<u32>().unwrap().try_into().unwrap();
                SequenceSet::try_from(vec![Sequence::Single(item)]).unwrap()
            }
        }
    }
}

mod tests {
    use std::{
        net::TcpListener,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use futures::{
        channel::mpsc::unbounded,
        executor::block_on,
        future::{self, Either},
        pin_mut, StreamExt,
    };
    use melib::{
        backends::prelude::*,
        imap::*,
        utils::logging::{LogLevel, Logger},
        Mail,
    };
    use tempfile::TempDir;

    use super::server::*;

    /// Test that `ImapType::watch` `Stream` returns the expected `Refresh`
    /// events when altering the mail store in the IMAP server.
    pub(crate) fn run_imap_watch() {
        let mut _logger = Logger::new_with(LogLevel::TRACE, true);
        let temp_dir = TempDir::new().unwrap();
        let backend_event_queue =
            Arc::new(Mutex::new(std::collections::VecDeque::with_capacity(16)));

        let backend_event_consumer = {
            let backend_event_queue = Arc::clone(&backend_event_queue);

            BackendEventConsumer::new(Arc::new(move |ah, be| {
                eprintln!("BackendEventConsumer: ah {ah:?} be {be:?}");
                backend_event_queue.lock().unwrap().push_back((ah, be));
            }))
        };

        for var in [
            "HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
            "XDG_CONFIG_DIRS",
            "XDG_CONFIG_HOME",
            "XDG_DATA_DIRS",
            "XDG_DATA_HOME",
        ] {
            std::env::remove_var(var);
        }
        for (var, dir) in [
            ("HOME", temp_dir.path().to_path_buf()),
            ("XDG_CACHE_HOME", temp_dir.path().join(".cache")),
            ("XDG_STATE_HOME", temp_dir.path().join(".local/state")),
            ("XDG_CONFIG_HOME", temp_dir.path().join(".config")),
            ("XDG_DATA_HOME", temp_dir.path().join(".local/share")),
        ] {
            std::fs::create_dir_all(&dir).unwrap_or_else(|err| {
                panic!("Could not create {} path, {}: {}", var, dir.display(), err);
            });
            std::env::set_var(var, &dir);
        }

        let server_state = Arc::new(Mutex::new(ServerState {
            envelopes: indexmap::indexmap! {},
            next_uid: 1,
            uidvalidity: 1,
            ..Default::default()
        }));

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let local_addr = listener.local_addr().unwrap();
        let account_conf = AccountSettings {
            name: "test".to_string(),
            root_mailbox: "INBOX".to_string(),
            format: "imap".to_string(),
            identity: "user@example.com".to_string(),
            extra_identities: vec![],
            read_only: false,
            display_name: None,
            subscribed_mailboxes: vec![],
            mailboxes: indexmap::indexmap! {},
            manual_refresh: false,
            extra: indexmap::indexmap! {
                "server_hostname".to_string() => local_addr.ip().to_string(),
                "server_username".to_string() => "user".to_string(),
                "server_password".to_string() => "password".to_string(),
                "server_port".to_string() => local_addr.port().to_string(),
                "use_starttls".to_string() => "false".to_string(),
                "use_tls".to_string() => "false".to_string(),
                // Important for testing, because we expect only one connection to be used.
                "use_connection_pool".to_string() => "false".to_string(),
                "timeout".to_string() => 1_u64.to_string(),
            },
        };

        let mut imap =
            ImapType::new(&account_conf, Default::default(), backend_event_consumer).unwrap();
        let listener = smol::Async::new(listener).unwrap();
        let mut is_online_fut = imap.is_online().unwrap();
        let (main_conn_sender, main_conn_receiver) = unbounded();
        let main_conn = ImapServerStream::new(
            &listener,
            &mut is_online_fut,
            (main_conn_sender.clone(), main_conn_receiver),
            Arc::clone(&server_state),
        );
        block_on(is_online_fut).unwrap();
        let mut mailboxes_fut = imap.mailboxes().unwrap();
        let mut main_conn_loop = Box::pin(main_conn.loop_handler("main"));
        let mailboxes = match block_on(future::select(
            mailboxes_fut.as_mut(),
            main_conn_loop.as_mut(),
        )) {
            Either::Left((value1, _)) => value1.unwrap(),
            Either::Right((value2, _)) => {
                unreachable!("{:?}", value2);
            }
        };
        let inbox_hash = *mailboxes.keys().next().unwrap();

        let mut watch_fut = imap.watch().unwrap().into_future();
        let (watch_conn_sender, watch_conn_receiver) = unbounded();
        let watch_conn = ImapServerStream::new(
            &listener,
            &mut watch_fut,
            (watch_conn_sender.clone(), watch_conn_receiver),
            Arc::clone(&server_state),
        );
        // $ date -R -u -r 0
        let new_mail = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:00 +0000
Cc:
Subject: RE: your e-mail
Message-ID: <h2g7f.z0gy2pgaen5m@example.com>
Content-Type: text/plain

hello world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let new_mail_2 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Cc:
Date: Thu, 01 Jan 1970 00:00:01 +0000
Subject: RE: your e-mail 2
Message-ID: <h2g7f.z0gy2pgaen6m@example.com>
Content-Type: text/plain

hello world 2.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let new_mail_3 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Cc:
Date: Thu, 01 Jan 1970 00:00:02 +0000
Subject: RE: your e-mail 3
Message-ID: <h2g7f.z0gy2pgaen7m@example.com>
Content-Type: text/plain

hello world 3.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        watch_conn_sender
            .unbounded_send(ServerEvent::New(new_mail.clone()))
            .unwrap();
        watch_conn_sender
            .unbounded_send(ServerEvent::New(new_mail_2.clone()))
            .unwrap();
        watch_conn_sender
            .unbounded_send(ServerEvent::New(new_mail_3.clone()))
            .unwrap();
        let watch_conn_loop = watch_conn.loop_handler("watch");
        let loops = loops_fut(main_conn_loop, watch_conn_loop);
        let loops_handle = std::thread::spawn(move || {
            block_on(loops);
        });
        let fut = async move {
            let hash;
            let mut refresh_events = vec![];
            while refresh_events.len() < 3 {
                let (value1, rest) = watch_fut.await;
                let backend_event = value1.unwrap().unwrap();
                match backend_event {
                    BackendEvent::RefreshBatch(events) => {
                        refresh_events.extend(events);
                    }
                    BackendEvent::Refresh(event) => {
                        refresh_events.push(event);
                    }
                    backend_event => {
                        panic!("Expected Refresh event, got: {backend_event:?}");
                    }
                }
                watch_fut = rest.into_future();
            }
            {
                let mailbox = imap.uid_store.mailboxes.lock().await;
                let exists_lck = mailbox.values().next().unwrap().exists.lock().unwrap();
                assert_eq!(exists_lck.len(), 3);
                let unseen_lck = mailbox.values().next().unwrap().unseen.lock().unwrap();
                assert_eq!(unseen_lck.len(), 3);
            }
            {
                let mut fetch_fut = imap.fetch(inbox_hash).unwrap().into_future();
                let mut envelopes: Vec<Envelope> = vec![];
                loop {
                    let (envs, rest) = fetch_fut.await;
                    let Some(envs) = envs else {
                        break;
                    };
                    envelopes.extend(envs.unwrap());

                    fetch_fut = rest.into_future();
                }
                envelopes.sort_by_key(|env| env.date());
                for env in &mut envelopes {
                    env.set_hash(EnvelopeHash(0));
                }
                let mut expected = vec![
                    new_mail.envelope.clone(),
                    new_mail_2.envelope.clone(),
                    new_mail_3.envelope.clone(),
                ];
                for env in &mut expected {
                    env.set_hash(EnvelopeHash(0));
                }
                assert_eq!(envelopes, expected);
            }
            {
                let Some(RefreshEvent { kind: RefreshEventKind::Create(ref env), .. }) = refresh_events.iter().find(|refresh_event| matches!(refresh_event.kind, RefreshEventKind::Create(ref env) if env.subject()== "RE: your e-mail")) else {
                    panic!("Expected Create event, got: {refresh_events:?}");
                };
                assert_eq!(env.subject(), "RE: your e-mail");
                assert_eq!(env.message_id(), "h2g7f.z0gy2pgaen5m@example.com");
                hash = env.hash();
                let uid = {
                    let state_lck = server_state.lock().unwrap();
                    state_lck
                        .envelopes
                        .iter()
                        .find_map(|(uid, env)| {
                            if env.message_id() == "h2g7f.z0gy2pgaen5m@example.com" {
                                Some(*uid)
                            } else {
                                None
                            }
                        })
                        .unwrap()
                };
                watch_conn_sender
                    .unbounded_send(ServerEvent::Delete(uid))
                    .unwrap();
            }
            let watch_fut = {
                let (value1, rest) = watch_fut.await;
                let backend_event = value1.unwrap().unwrap();
                let BackendEvent::Refresh(refresh_event) = backend_event else {
                    panic!("Expected Refresh event, got: {backend_event:?}");
                };
                let RefreshEventKind::Remove(ref env_hash) = refresh_event.kind else {
                    panic!("Expected Remove event, got: {refresh_event:?}");
                };
                assert_eq!(*env_hash, hash);
                rest.into_future()
            };
            {
                let mailbox = imap.uid_store.mailboxes.lock().await;
                let exists_lck = mailbox.values().next().unwrap().exists.lock().unwrap();
                assert_eq!(exists_lck.len(), 2);
                let unseen_lck = mailbox.values().next().unwrap().unseen.lock().unwrap();
                assert_eq!(unseen_lck.len(), 2);
            }
            {
                let mut fetch_fut = imap.fetch(inbox_hash).unwrap().into_future();
                let mut envelopes: Vec<Envelope> = vec![];
                loop {
                    let (envs, rest) = fetch_fut.await;
                    let Some(envs) = envs else {
                        break;
                    };
                    envelopes.extend(envs.unwrap());

                    fetch_fut = rest.into_future();
                }
                envelopes.sort_by_key(|env| env.date());
                for env in &mut envelopes {
                    env.set_hash(EnvelopeHash(0));
                }
                let mut expected = vec![new_mail_2.envelope.clone(), new_mail_3.envelope.clone()];
                for env in &mut expected {
                    env.set_hash(EnvelopeHash(0));
                }
                assert_eq!(envelopes, expected);
            }
            watch_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
            main_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
            loops_handle.join().unwrap();
            let (mut value1, rest) = watch_fut.await;
            let watch_fut = rest.into_future();
            if matches!(
                value1,
                Some(Ok(
                    BackendEvent::Refresh(RefreshEvent {
                        kind: RefreshEventKind::Failure(ref err),
                        ..
                    })))
                if err.summary == "Disconnected"
            ) {
                value1 = watch_fut.await.0;
            }
            if let Some(val) = value1 {
                if !(matches!(val, Err(ref err) if matches!(err.kind, ErrorKind::OSError(nix::errno::Errno::EPIPE | nix::errno::Errno::ECONNRESET)))
                    || matches!(val, Err(ref err) if err.summary == "Disconnected"))
                {
                    panic!(
                        "Expected watch TCP connection to have disconnected with \
                         EPIPE/ECONNRESET, got: {val:?}"
                    );
                }
            }
        };
        std::thread::spawn(move || {
            block_on(fut);
        })
        .join()
        .unwrap();
    }

    async fn loops_fut(
        main_conn_loop: impl futures::Future<Output = ()>,
        watch_conn_loop: impl futures::Future<Output = ()>,
    ) {
        pin_mut!(main_conn_loop);
        pin_mut!(watch_conn_loop);
        match future::select(main_conn_loop.as_mut(), watch_conn_loop.as_mut()).await {
            Either::Left((_, watch_conn)) => {
                eprintln!("loops fut loop finished with main_conn",);
                watch_conn.await;
                eprintln!("loops fut loop finished with watch_conn",);
            }
            Either::Right((_, main_conn)) => {
                eprintln!("loops fut loop finished with watch_conn",);
                main_conn.await;
                eprintln!("loops fut loop finished with main_conn",);
            }
        }
    }

    /// Mirror of `melib::imap::protocol_parser::generate_envelope_hash`,
    /// which is not visible outside the `melib` crate.
    #[cfg(feature = "sqlite3")]
    fn generate_envelope_hash(mailbox_path: &str, uid: UID) -> EnvelopeHash {
        use std::{collections::hash_map::DefaultHasher, hash::Hasher};

        let mut h = DefaultHasher::new();
        h.write_usize(uid);
        h.write(mailbox_path.as_bytes());
        EnvelopeHash(h.finish())
    }

    /// Create the `test` account's cache database under `db_dir` with the
    /// current schema and seed it with an already synchronized INBOX: a
    /// `mailbox` row for `mailbox_hash` with uidvalidity 1, max_uid 3, the
    /// given `(MESSAGES, UNSEEN, UIDNEXT)` baseline, and the envelopes of
    /// `mails`.
    #[cfg(feature = "sqlite3")]
    fn seed_imap_cache_db(
        db_dir: &std::path::Path,
        mailbox_hash: MailboxHash,
        mailbox_path: &str,
        mails: &[(UID, &Mail)],
        messages: UID,
        unseen: UID,
        uidnext: UID,
    ) {
        use melib::utils::sqlite3::rusqlite;

        std::fs::create_dir_all(db_dir).unwrap();
        let conn = rusqlite::Connection::open(db_dir.join("test_header_cache.db")).unwrap();
        conn.pragma_update(None, "user_version", 5_u32).unwrap();
        conn.execute_batch(
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
            CREATE INDEX IF NOT EXISTS envelope_uid_idx ON envelopes(mailbox_hash, uid ASC);
            CREATE INDEX IF NOT EXISTS envelope_idx ON envelopes(hash);
            CREATE INDEX IF NOT EXISTS mailbox_idx ON mailbox(mailbox_hash);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO mailbox (mailbox_hash, uidvalidity, max_uid, flags, messages, unseen, \
             uidnext) VALUES (?1, 1, 3, X'', ?2, ?3, ?4);",
            rusqlite::params![
                mailbox_hash.0 as i64,
                messages as i64,
                unseen as i64,
                uidnext as i64
            ],
        )
        .unwrap();
        for (uid, mail) in mails {
            let uid = *uid;
            let mut env = mail.envelope.clone();
            env.set_hash(generate_envelope_hash(mailbox_path, uid));
            conn.execute(
                "INSERT INTO envelopes (hash, uid, mailbox_hash, modsequence, envelope) VALUES \
                 (?1, ?2, ?3, NULL, ?4);",
                rusqlite::params![env.hash().0 as i64, uid as i64, mailbox_hash.0 as i64, &env],
            )
            .unwrap();
        }
    }

    #[cfg(feature = "sqlite3")]
    async fn fetch_all_envs(imap: &mut ImapType, inbox_hash: MailboxHash) -> Vec<Envelope> {
        let mut fetch_fut = imap.fetch(inbox_hash).unwrap().into_future();
        let mut envelopes = vec![];
        loop {
            let (envs, rest) = fetch_fut.await;
            let Some(envs) = envs else {
                break;
            };
            envelopes.extend(envs.unwrap());

            fetch_fut = rest.into_future();
        }
        envelopes
    }

    /// Test the `resync_basic` STATUS quick check: when the cached
    /// `(MESSAGES, UNSEEN, UIDNEXT)` counters and UIDVALIDITY match the
    /// server's `STATUS` response (`consistent == true`), the full FLAGS
    /// resync (`UID FETCH`es) must be skipped; otherwise it must run and its
    /// final counters must be recorded so that a subsequent fetch is
    /// short-circuited.
    ///
    /// Ground truth is the mock server's received-command log.
    #[cfg(feature = "sqlite3")]
    pub(crate) fn run_imap_resync_status_shortcircuit(consistent: bool) {
        let mut _logger = Logger::new_with(LogLevel::TRACE, true);
        let temp_dir = TempDir::new().unwrap();

        for var in [
            "HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
            "XDG_CONFIG_DIRS",
            "XDG_CONFIG_HOME",
            "XDG_DATA_DIRS",
            "XDG_DATA_HOME",
        ] {
            std::env::remove_var(var);
        }
        for (var, dir) in [
            ("HOME", temp_dir.path().to_path_buf()),
            ("XDG_CACHE_HOME", temp_dir.path().join(".cache")),
            ("XDG_STATE_HOME", temp_dir.path().join(".local/state")),
            ("XDG_CONFIG_HOME", temp_dir.path().join(".config")),
            ("XDG_DATA_HOME", temp_dir.path().join(".local/share")),
        ] {
            std::fs::create_dir_all(&dir).unwrap_or_else(|err| {
                panic!("Could not create {} path, {}: {}", var, dir.display(), err);
            });
            std::env::set_var(var, &dir);
        }

        let new_mail = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:00 +0000
Cc:
Subject: RE: your e-mail
Message-ID: <h2g7f.z0gy2pgaen5m@example.com>
Content-Type: text/plain

hello world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let new_mail_2 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Cc:
Date: Thu, 01 Jan 1970 00:00:01 +0000
Subject: RE: your e-mail 2
Message-ID: <h2g7f.z0gy2pgaen6m@example.com>
Content-Type: text/plain

hello world 2.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let new_mail_3 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Cc:
Date: Thu, 01 Jan 1970 00:00:02 +0000
Subject: RE: your e-mail 3
Message-ID: <h2g7f.z0gy2pgaen7m@example.com>
Content-Type: text/plain

hello world 3.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let mails = [
            (1 as UID, &*new_mail),
            (2 as UID, &*new_mail_2),
            (3 as UID, &*new_mail_3),
        ];

        let server_state = Arc::new(Mutex::new(ServerState {
            envelopes: indexmap::indexmap! {},
            next_uid: 1,
            uidvalidity: 1,
            ..Default::default()
        }));
        {
            let mut state_lck = server_state.lock().unwrap();
            for (_, mail) in mails {
                state_lck.insert(Box::new(mail.clone()));
            }
        }

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let local_addr = listener.local_addr().unwrap();
        let account_conf = AccountSettings {
            name: "test".to_string(),
            root_mailbox: "INBOX".to_string(),
            format: "imap".to_string(),
            identity: "user@example.com".to_string(),
            extra_identities: vec![],
            read_only: false,
            display_name: None,
            subscribed_mailboxes: vec![],
            mailboxes: indexmap::indexmap! {},
            manual_refresh: false,
            extra: indexmap::indexmap! {
                "server_hostname".to_string() => local_addr.ip().to_string(),
                "server_username".to_string() => "user".to_string(),
                "server_password".to_string() => "password".to_string(),
                "server_port".to_string() => local_addr.port().to_string(),
                "use_starttls".to_string() => "false".to_string(),
                "use_tls".to_string() => "false".to_string(),
                // Important for testing, because we expect only one connection to be used.
                "use_connection_pool".to_string() => "false".to_string(),
                "timeout".to_string() => 1_u64.to_string(),
            },
        };

        let mut imap =
            ImapType::new(&account_conf, Default::default(), Default::default()).unwrap();
        let listener = smol::Async::new(listener).unwrap();
        let mut is_online_fut = imap.is_online().unwrap();
        let (main_conn_sender, main_conn_receiver) = unbounded();
        let main_conn = ImapServerStream::new(
            &listener,
            &mut is_online_fut,
            (main_conn_sender.clone(), main_conn_receiver),
            Arc::clone(&server_state),
        );
        let received_commands = Arc::clone(&main_conn.received_commands);
        block_on(is_online_fut).unwrap();
        let mut mailboxes_fut = imap.mailboxes().unwrap();
        let mut main_conn_loop = Box::pin(main_conn.loop_handler("main"));
        let mailboxes = match block_on(future::select(
            mailboxes_fut.as_mut(),
            main_conn_loop.as_mut(),
        )) {
            Either::Left((value1, _)) => value1.unwrap(),
            Either::Right((value2, _)) => {
                unreachable!("{:?}", value2);
            }
        };
        let inbox_hash = *mailboxes.keys().next().unwrap();
        let mailbox_path = {
            let mailboxes_lck = block_on(imap.uid_store.mailboxes.lock());
            mailboxes_lck[&inbox_hash].imap_path().to_string()
        };

        // The server always has 3 unseen mails and UIDNEXT 4; for
        // `consistent == false` the cached baseline is stale (2/2/3), so the
        // quick check must fail and the full FLAGS resync must run.
        let (messages, unseen, uidnext) = if consistent { (3, 3, 4) } else { (2, 2, 3) };
        seed_imap_cache_db(
            &temp_dir.path().join(".local/share/meli"),
            inbox_hash,
            &mailbox_path,
            &mails,
            messages,
            unseen,
            uidnext,
        );

        let loops_handle = std::thread::spawn(move || {
            block_on(main_conn_loop);
        });
        let received_commands_2 = Arc::clone(&received_commands);
        // Run inside a thread scope so that `imap` (and its connection) stays
        // alive until the server loop is quit and joined below; otherwise the
        // dropped connection makes the mock server's read loop spin on EOF
        // and starve its command channel.
        let (envelopes, envelopes_2, first_fetch_commands_len) = std::thread::scope(|scope| {
            let imap = &mut imap;
            scope
                .spawn(move || {
                    block_on(async {
                        let envelopes = fetch_all_envs(imap, inbox_hash).await;
                        let first_fetch_commands_len = received_commands_2.lock().unwrap().len();
                        let envelopes_2 = fetch_all_envs(imap, inbox_hash).await;
                        (envelopes, envelopes_2, first_fetch_commands_len)
                    })
                })
                .join()
                .unwrap()
        });

        let mut expected = mails
            .iter()
            .map(|(_, mail)| mail.envelope.clone())
            .collect::<Vec<_>>();
        for envelopes in [&envelopes, &envelopes_2] {
            let mut envelopes = envelopes.clone();
            assert_eq!(envelopes.len(), 3);
            envelopes.sort_by_key(|env| env.date());
            for env in &mut envelopes {
                env.set_hash(EnvelopeHash(0));
            }
            for env in &mut expected {
                env.set_hash(EnvelopeHash(0));
            }
            assert_eq!(envelopes, expected);
        }

        {
            let lck = received_commands.lock().unwrap();
            lck.iter()
                .position(|l| {
                    l.ends_with(" STATUS INBOX (MESSAGES UIDNEXT UIDVALIDITY UNSEEN)\r\n")
                })
                .unwrap_or_else(|| panic!("STATUS quick check command was not sent: {lck:?}"));
            if consistent {
                // The whole session (both fetches) must not contain any UID
                // FETCH: the FLAGS resync was entirely skipped. (`UID SEARCH
                // 1:*` lines still appear because `select_or_examine` fills
                // the MSN index with a search on every SELECT/EXAMINE; that
                // is unrelated to the resync.)
                assert!(
                    !lck.iter().any(|l| l.contains("UID FETCH")),
                    "UID FETCH sent despite matching STATUS counters: {lck:?}"
                );
            } else {
                assert!(
                    lck.iter()
                        .any(|l| l.contains(" UID FETCH 4:* (UID FLAGS ENVELOPE"))
                        && lck.iter().any(|l| l.contains(" UID FETCH 1:3 FLAGS\r\n")),
                    "full FLAGS resync did not run on STATUS mismatch: {lck:?}"
                );
                assert!(
                    lck.iter()
                        .any(|l| l.ends_with(" STATUS INBOX (MESSAGES UIDNEXT UNSEEN)\r\n")),
                    "final STATUS recording command was not sent: {lck:?}"
                );
            }
            // The final counters recorded by the first fetch (3/3/4 for both
            // scenarios) must make the second fetch short-circuit.
            assert!(
                !lck[first_fetch_commands_len..]
                    .iter()
                    .any(|l| l.contains("UID FETCH")),
                "second fetch did not short-circuit: {lck:?}"
            );
        }

        main_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        loops_handle.join().unwrap();
    }

    /// Failing-first regression test (D2): a server that answers a `UID
    /// FETCH` command without the mandatory `UID` data item (an RFC 3501
    /// §6.4.8 violation) must make `resync_basic` fail with a protocol
    /// error; it used to panic on `Option::unwrap()` when iterating the
    /// parsed FLAGS reply.
    ///
    /// The test warms the backend with two seed mails (so the initial
    /// fetch succeeds with a compliant reply), delivers a third mail (so
    /// the STATUS quick check misses and the full FLAGS resync runs on
    /// the next `refresh`), and has the mock omit the `UID` item from
    /// `UID FETCH 1:2 FLAGS` replies. `MailBackend::refresh` must return
    /// a protocol error, not panic.
    #[cfg(feature = "sqlite3")]
    pub(crate) fn run_imap_uid_fetch_reply_without_uid_item() {
        let mut _logger = Logger::new_with(LogLevel::TRACE, true);
        let temp_dir = TempDir::new().unwrap();
        let backend_event_queue =
            Arc::new(Mutex::new(std::collections::VecDeque::with_capacity(16)));
        let backend_event_consumer = {
            let backend_event_queue = Arc::clone(&backend_event_queue);

            BackendEventConsumer::new(Arc::new(move |ah, be| {
                eprintln!("BackendEventConsumer: ah {ah:?} be {be:?}");
                backend_event_queue.lock().unwrap().push_back((ah, be));
            }))
        };
        set_test_xdg_env(&temp_dir);

        let seed_mail_1 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:00 +0000
Cc:
Subject: RE: uidless seed 1
Message-ID: <uidless1@example.com>
Content-Type: text/plain

hello world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let seed_mail_2 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Cc:
Date: Thu, 01 Jan 1970 00:00:01 +0000
Subject: RE: uidless seed 2
Message-ID: <uidless2@example.com>
Content-Type: text/plain

hello world 2.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let new_mail = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Cc:
Date: Thu, 01 Jan 1970 00:00:02 +0000
Subject: RE: uidless NEW mail
Message-ID: <uidlessnew@example.com>
Content-Type: text/plain

hello new world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );

        let server_state = Arc::new(Mutex::new(ServerState {
            envelopes: indexmap::indexmap! {},
            next_uid: 1,
            uidvalidity: 1,
            uid_fetch_flags_drop_uid: true,
            ..Default::default()
        }));
        {
            let mut state_lck = server_state.lock().unwrap();
            state_lck.insert(seed_mail_1);
            state_lck.insert(seed_mail_2);
        }

        let (mut imap, _listener, main_conn_sender, loops_handle, inbox_hash, main_commands) =
            warm_start_setup(
                backend_event_consumer,
                Arc::clone(&server_state),
                600,
                300,
                true,
                true,
            );

        {
            let imap = &mut imap;
            let server_state = &server_state;
            let new_mail = &new_mail;
            std::thread::scope(|scope| {
                scope
                    .spawn(move || {
                        block_on(async {
                            let seed_envs = fetch_all_envs(imap, inbox_hash).await;
                            assert_eq!(
                                seed_envs.len(),
                                2,
                                "initial fetch must load the two seed mails"
                            );
                            // Deliver one new mail so the STATUS quick
                            // check misses and the full FLAGS resync
                            // runs on the next refresh; its `UID FETCH
                            // 1:2 FLAGS` step is the one the mock
                            // answers without the UID item.
                            server_state.lock().unwrap().insert(new_mail.clone());
                            match imap.refresh(inbox_hash).unwrap().await {
                                Ok(()) => panic!(
                                    "refresh succeeded even though the mock's UID FETCH FLAGS \
                                     reply lacked the UID item; the resync must fail with a \
                                     protocol error"
                                ),
                                Err(err) => {
                                    assert!(
                                        err.kind.is_protocol_error(),
                                        "expected a protocol error for the UID-less FETCH \
                                         reply, got: {err}"
                                    );
                                    assert!(
                                        err.to_string().to_ascii_lowercase().contains("uid"),
                                        "error should mention the missing UID item: {err}"
                                    );
                                }
                            }
                        });
                    })
                    .join()
                    .unwrap();
            });
        }

        // Ground truth that the FLAGS resync actually ran (and therefore
        // reached the code path under test):
        {
            let lck = main_commands.lock().unwrap();
            assert!(
                lck.iter().any(|l| l.contains(" UID FETCH 1:2 FLAGS\r\n")),
                "the FLAGS resync UID FETCH command was not sent: {lck:?}"
            );
        }

        main_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        loops_handle.join().unwrap();
    }

    /// Regression pin for the UID-less `UID FETCH` reply regression
    /// on the envelope-fetch path (resync Step 2i): the general
    /// `uid_fetch_drop_uid_all` mock flag omits the `UID` item from
    /// *all* `UID FETCH` replies, so after a mail is delivered the
    /// next `refresh`'s Step 2i `UID FETCH <n>:* (UID FLAGS ENVELOPE
    /// ...)` is answered without the UID item, and its Step 2ii FLAGS
    /// fetch likewise. `MailBackend::refresh` must fail cleanly with a
    /// protocol error and never panic; the error surfaces at Step 2ii
    /// because Step 2i's UID-less reply is absorbed by the client's
    /// untagged FETCH handling (a `UID SEARCH` MSN-to-UID recovery;
    /// the mock answers it here). See
    /// `run_imap_uid_fetch_reply_without_uid_item` for the FLAGS-path
    /// (resync Step 2ii) twin of this test.
    #[cfg(feature = "sqlite3")]
    pub(crate) fn run_imap_uid_fetch_reply_without_uid_item_envelope() {
        let mut _logger = Logger::new_with(LogLevel::TRACE, true);
        let temp_dir = TempDir::new().unwrap();
        let backend_event_queue =
            Arc::new(Mutex::new(std::collections::VecDeque::with_capacity(16)));
        let backend_event_consumer = {
            let backend_event_queue = Arc::clone(&backend_event_queue);

            BackendEventConsumer::new(Arc::new(move |ah, be| {
                eprintln!("BackendEventConsumer: ah {ah:?} be {be:?}");
                backend_event_queue.lock().unwrap().push_back((ah, be));
            }))
        };
        set_test_xdg_env(&temp_dir);

        let seed_mail_1 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:00 +0000
Cc:
Subject: RE: uidless env seed 1
Message-ID: <uidlessenv1@example.com>
Content-Type: text/plain

hello world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let seed_mail_2 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Cc:
Date: Thu, 01 Jan 1970 00:00:01 +0000
Subject: RE: uidless env seed 2
Message-ID: <uidlessenv2@example.com>
Content-Type: text/plain

hello world 2.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let new_mail = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Cc:
Date: Thu, 01 Jan 1970 00:00:02 +0000
Subject: RE: uidless env NEW mail
Message-ID: <uidlessenvnew@example.com>
References: <uidlessenv1@example.com>
 <uidlessenv2@example.com>
Content-Type: text/plain

hello new world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );

        let server_state = Arc::new(Mutex::new(ServerState {
            envelopes: indexmap::indexmap! {},
            next_uid: 1,
            uidvalidity: 1,
            uid_fetch_flags_drop_uid: false,
            ..Default::default()
        }));
        {
            let mut state_lck = server_state.lock().unwrap();
            state_lck.insert(seed_mail_1);
            state_lck.insert(seed_mail_2);
        }

        let (mut imap, _listener, main_conn_sender, loops_handle, inbox_hash, main_commands) =
            warm_start_setup(
                backend_event_consumer,
                Arc::clone(&server_state),
                600,
                300,
                true,
                true,
            );

        {
            let imap = &mut imap;
            let server_state = &server_state;
            let new_mail = &new_mail;
            std::thread::scope(|scope| {
                scope
                    .spawn(move || {
                        block_on(async {
                            // Baseline: with compliant replies the two
                            // seed mails load fine and populate the
                            // cache state a resync needs.
                            let seed_envs = fetch_all_envs(imap, inbox_hash).await;
                            assert_eq!(
                                seed_envs.len(),
                                2,
                                "initial fetch must load the two seed mails"
                            );
                            // From here on the mock drops the UID item
                            // from *all* UID FETCH replies. Deliver one
                            // new mail so the STATUS quick check misses
                            // and the full resync runs on the next
                            // refresh; its Step 2i `UID FETCH 3:* (UID
                            // FLAGS ENVELOPE ...)` is the command the
                            // mock answers without the UID item.
                            server_state.lock().unwrap().uid_fetch_drop_uid_all = true;
                            server_state.lock().unwrap().insert(new_mail.clone());
                            match imap.refresh(inbox_hash).unwrap().await {
                                Ok(()) => panic!(
                                    "refresh succeeded even though the mock's envelope UID \
                                     FETCH reply lacked the UID item; the resync must fail with \
                                     a protocol error"
                                ),
                                Err(err) => {
                                    assert!(
                                        err.kind.is_protocol_error(),
                                        "expected a protocol error for the UID-less FETCH \
                                         reply, got: {err}"
                                    );
                                    assert!(
                                        err.to_string().to_ascii_lowercase().contains("uid"),
                                        "error should mention the missing UID item: {err}"
                                    );
                                }
                            }
                        });
                    })
                    .join()
                    .unwrap();
            });
        }

        // Ground truth that the envelope resync (Step 2i) actually ran
        // (and therefore the mock answered it without the UID item), and
        // that the FLAGS resync (Step 2ii, where the protocol error
        // surfaces) ran too:
        {
            let lck = main_commands.lock().unwrap();
            assert!(
                lck.iter()
                    .any(|l| l.contains(" UID FETCH 3:* (UID FLAGS ENVELOPE")),
                "the envelope resync UID FETCH command was not sent: {lck:?}"
            );
            assert!(
                lck.iter().any(|l| l.contains(" UID FETCH 1:2 FLAGS\r\n")),
                "the FLAGS resync UID FETCH command was not sent: {lck:?}"
            );
        }

        main_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        loops_handle.join().unwrap();
    }

    /// Test the stale-while-revalidate fetch flow.
    ///
    /// With `with_cache == true`, the sqlite cache is seeded with three
    /// of the server's four mails (with a stale STATUS baseline, so the
    /// resync must actually run) and the mock server delays its
    /// SELECT/EXAMINE replies by 2 seconds: the first emitted envelope
    /// chunk must already have been collected by the time the server
    /// answers the fetch's first SELECT (ground truth: the server's
    /// `select_reply_sent` flag). The resync must still run afterwards
    /// and fetch the fourth (new) mail; no envelope may be emitted
    /// twice across the whole stream.
    ///
    /// With `with_cache == false` (no seeded cache), the whole session's
    /// command log must be byte-identical to the pre-change behavior:
    /// serving the cache first must not add, remove or reorder any IMAP
    /// command on the network.
    #[cfg(feature = "sqlite3")]
    pub(crate) fn run_imap_fetch_cache_first(with_cache: bool) {
        let mut _logger = Logger::new_with(LogLevel::TRACE, true);
        let temp_dir = TempDir::new().unwrap();

        for var in [
            "HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
            "XDG_CONFIG_DIRS",
            "XDG_CONFIG_HOME",
            "XDG_DATA_DIRS",
            "XDG_DATA_HOME",
        ] {
            std::env::remove_var(var);
        }
        for (var, dir) in [
            ("HOME", temp_dir.path().to_path_buf()),
            ("XDG_CACHE_HOME", temp_dir.path().join(".cache")),
            ("XDG_STATE_HOME", temp_dir.path().join(".local/state")),
            ("XDG_CONFIG_HOME", temp_dir.path().join(".config")),
            ("XDG_DATA_HOME", temp_dir.path().join(".local/share")),
        ] {
            std::fs::create_dir_all(&dir).unwrap_or_else(|err| {
                panic!("Could not create {} path, {}: {}", var, dir.display(), err);
            });
            std::env::set_var(var, &dir);
        }

        let new_mail = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:00 +0000
Cc:
Subject: RE: your e-mail
Message-ID: <h2g7f.z0gy2pgaen5m@example.com>
Content-Type: text/plain

hello world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let new_mail_2 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:01 +0000
Cc:
Subject: RE: your e-mail 2
Message-ID: <h2g7f.z0gy2pgaen6m@example.com>
Content-Type: text/plain

hello world 2.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let new_mail_3 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:02 +0000
Cc:
Subject: RE: your e-mail 3
Message-ID: <h2g7f.z0gy2pgaen7m@example.com>
Content-Type: text/plain

hello world 3.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let new_mail_4 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:03 +0000
Cc:
Subject: RE: your e-mail 4
Message-ID: <h2g7f.z0gy2pgaen8m@example.com>
Content-Type: text/plain

hello world 4.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let mails: Vec<(UID, &Mail)> = if with_cache {
            vec![
                (1 as UID, &*new_mail),
                (2 as UID, &*new_mail_2),
                (3 as UID, &*new_mail_3),
                (4 as UID, &*new_mail_4),
            ]
        } else {
            vec![
                (1 as UID, &*new_mail),
                (2 as UID, &*new_mail_2),
                (3 as UID, &*new_mail_3),
            ]
        };

        let server_state = Arc::new(Mutex::new(ServerState {
            envelopes: indexmap::indexmap! {},
            next_uid: 1,
            uidvalidity: 1,
            ..Default::default()
        }));
        {
            let mut state_lck = server_state.lock().unwrap();
            for &(_, mail) in mails.iter() {
                state_lck.insert(Box::new(mail.clone()));
            }
        }

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let local_addr = listener.local_addr().unwrap();
        let account_conf = AccountSettings {
            name: "test".to_string(),
            root_mailbox: "INBOX".to_string(),
            format: "imap".to_string(),
            identity: "user@example.com".to_string(),
            extra_identities: vec![],
            read_only: false,
            display_name: None,
            subscribed_mailboxes: vec![],
            mailboxes: indexmap::indexmap! {},
            manual_refresh: false,
            extra: indexmap::indexmap! {
                "server_hostname".to_string() => local_addr.ip().to_string(),
                "server_username".to_string() => "user".to_string(),
                "server_password".to_string() => "password".to_string(),
                "server_port".to_string() => local_addr.port().to_string(),
                "use_starttls".to_string() => "false".to_string(),
                "use_tls".to_string() => "false".to_string(),
                // Important for testing, because we expect only one connection to be used.
                "use_connection_pool".to_string() => "false".to_string(),
                // Must exceed the SELECT reply delay set below.
                "timeout".to_string() => 10_u64.to_string(),
            },
        };

        let mut imap =
            ImapType::new(&account_conf, Default::default(), Default::default()).unwrap();
        let listener = smol::Async::new(listener).unwrap();
        let mut is_online_fut = imap.is_online().unwrap();
        let (main_conn_sender, main_conn_receiver) = unbounded();
        let mut main_conn = ImapServerStream::new(
            &listener,
            &mut is_online_fut,
            (main_conn_sender.clone(), main_conn_receiver),
            Arc::clone(&server_state),
        );
        let received_commands = Arc::clone(&main_conn.received_commands);
        let select_reply_sent = Arc::clone(&main_conn.select_reply_sent);
        if with_cache {
            main_conn.select_reply_delay = Some(Duration::from_secs(2));
        }
        block_on(is_online_fut).unwrap();
        let mut mailboxes_fut = imap.mailboxes().unwrap();
        let mut main_conn_loop = Box::pin(main_conn.loop_handler("main"));
        let mailboxes = match block_on(future::select(
            mailboxes_fut.as_mut(),
            main_conn_loop.as_mut(),
        )) {
            Either::Left((value1, _)) => value1.unwrap(),
            Either::Right((value2, _)) => {
                unreachable!("{:?}", value2);
            }
        };
        let inbox_hash = *mailboxes.keys().next().unwrap();
        let mailbox_path = {
            let mailboxes_lck = block_on(imap.uid_store.mailboxes.lock());
            mailboxes_lck[&inbox_hash].imap_path().to_string()
        };

        if with_cache {
            // Seed the cache with the first three mails and a stale
            // (2/2/3) STATUS baseline so the resync cannot short-circuit
            // and must discover the fourth mail over the network.
            seed_imap_cache_db(
                &temp_dir.path().join(".local/share/meli"),
                inbox_hash,
                &mailbox_path,
                &mails[..3],
                2,
                2,
                3,
            );
        }

        let loops_handle = std::thread::spawn(move || {
            block_on(main_conn_loop);
        });
        let select_reply_sent_2 = Arc::clone(&select_reply_sent);
        // Run inside a thread scope so that `imap` (and its connection)
        // stays alive until the server loop is quit and joined below;
        // otherwise the dropped connection makes the mock server's read
        // loop spin on EOF and starve its command channel.
        let (envelopes, first_chunk_before_select_reply) = std::thread::scope(|scope| {
            let imap = &mut imap;
            scope
                .spawn(move || {
                    block_on(async {
                        let mut fetch_fut = imap.fetch(inbox_hash).unwrap().into_future();
                        let mut envelopes: Vec<Envelope> = vec![];
                        let mut first_chunk_before_select_reply: Option<bool> = None;
                        loop {
                            let (envs, rest) = fetch_fut.await;
                            let Some(envs) = envs else {
                                break;
                            };
                            let envs = envs.unwrap();
                            if first_chunk_before_select_reply.is_none() && !envs.is_empty() {
                                first_chunk_before_select_reply = Some(
                                    !select_reply_sent_2.load(std::sync::atomic::Ordering::SeqCst),
                                );
                            }
                            envelopes.extend(envs);

                            fetch_fut = rest.into_future();
                        }
                        (envelopes, first_chunk_before_select_reply)
                    })
                })
                .join()
                .unwrap()
        });

        if with_cache {
            assert_eq!(
                first_chunk_before_select_reply,
                Some(true),
                "cached envelopes were not served before the server answered SELECT; \
                 commands: {:?}",
                received_commands.lock().unwrap()
            );
            assert!(
                select_reply_sent.load(std::sync::atomic::Ordering::SeqCst),
                "the fetch never completed a SELECT round-trip"
            );

            assert_eq!(envelopes.len(), 4);
            let hashes = envelopes
                .iter()
                .map(|env| env.hash())
                .collect::<std::collections::HashSet<EnvelopeHash>>();
            assert_eq!(
                hashes.len(),
                4,
                "duplicate envelopes emitted across the fetch stream: {envelopes:?}"
            );
            let mut expected = mails
                .iter()
                .map(|(_, mail)| mail.envelope.clone())
                .collect::<Vec<_>>();
            let mut envelopes = envelopes;
            envelopes.sort_by_key(|env| env.date());
            for env in &mut envelopes {
                env.set_hash(EnvelopeHash(0));
            }
            for env in &mut expected {
                env.set_hash(EnvelopeHash(0));
            }
            assert_eq!(envelopes, expected);

            let lck = received_commands.lock().unwrap();
            assert!(
                lck.iter()
                    .any(|l| l.contains(" UID FETCH 4:* (UID FLAGS ENVELOPE")),
                "resync did not fetch the new mail after serving the cache: {lck:?}"
            );
        } else {
            assert_eq!(envelopes.len(), 3);
            // Frozen pre-change startup sequence for an empty cache: the
            // `CacheFirst` stage must not add, remove or reorder any IMAP
            // command (it only reads the local sqlite cache).
            // The tags account for the `M4 ID` handshake command: with
            // `use_id` enabled (melib's default) the connect sequence is
            // M1 CAPABILITY, M2 AUTHENTICATE, M3 CAPABILITY, M4 ID, so the
            // first command seen by the loop handler is M5.
            let expected_commands: Vec<String> = [
                "M5 LIST \"\" *\r\n",
                "M6 LSUB \"\" *\r\n",
                "M7 SELECT INBOX\r\n",
                "M8 UID SEARCH 1:*\r\n",
                "M9 EXAMINE INBOX\r\n",
                "M10 UID SEARCH 1:*\r\n",
                "M11 STATUS INBOX (UIDNEXT)\r\n",
                "M12 SELECT INBOX\r\n",
                "M13 EXAMINE INBOX\r\n",
                "M14 UID SEARCH 1:*\r\n",
                "M15 STATUS INBOX (UIDNEXT)\r\n",
                "M16 UID FETCH 1:4 (UID FLAGS ENVELOPE BODY.PEEK[HEADER.FIELDS (REFERENCES)] \
                 BODYSTRUCTURE)\r\n",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect();
            assert_eq!(
                *received_commands.lock().unwrap(),
                expected_commands,
                "empty-cache startup command sequence differs from the pre-change behavior"
            );
        }

        main_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        loops_handle.join().unwrap();
    }

    /// With an empty cache, the initial full fetch's `UID FETCH` command
    /// must contain `BODYSTRUCTURE` only when `fetch_body_structure` is
    /// enabled (the default). Ground truth is the mock server's log of
    /// received commands: with the flag disabled no command may contain
    /// `BODYSTRUCTURE`; with the default the command must be
    /// byte-identical to the pre-`fetch_body_structure` behavior.
    #[cfg(feature = "sqlite3")]
    pub(crate) fn run_imap_fetch_body_structure_flag(enabled: bool) {
        let mut _logger = Logger::new_with(LogLevel::TRACE, true);
        let temp_dir = TempDir::new().unwrap();

        for var in [
            "HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
            "XDG_CONFIG_DIRS",
            "XDG_CONFIG_HOME",
            "XDG_DATA_DIRS",
            "XDG_DATA_HOME",
        ] {
            std::env::remove_var(var);
        }
        for (var, dir) in [
            ("HOME", temp_dir.path().to_path_buf()),
            ("XDG_CACHE_HOME", temp_dir.path().join(".cache")),
            ("XDG_STATE_HOME", temp_dir.path().join(".local/state")),
            ("XDG_CONFIG_HOME", temp_dir.path().join(".config")),
            ("XDG_DATA_HOME", temp_dir.path().join(".local/share")),
        ] {
            std::fs::create_dir_all(&dir).unwrap_or_else(|err| {
                panic!("Could not create {} path, {}: {}", var, dir.display(), err);
            });
            std::env::set_var(var, &dir);
        }

        let new_mail = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:00 +0000
Cc:
Subject: RE: your e-mail
Message-ID: <h2g7f.z0gy2pgaen5m@example.com>
Content-Type: text/plain

hello world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let new_mail_2 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:01 +0000
Cc:
Subject: RE: your e-mail 2
Message-ID: <h2g7f.z0gy2pgaen6m@example.com>
Content-Type: text/plain

hello world 2.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let new_mail_3 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:02 +0000
Cc:
Subject: RE: your e-mail 3
Message-ID: <h2g7f.z0gy2pgaen7m@example.com>
Content-Type: text/plain

hello world 3.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let mails: Vec<(UID, &Mail)> = vec![
            (1 as UID, &*new_mail),
            (2 as UID, &*new_mail_2),
            (3 as UID, &*new_mail_3),
        ];

        let server_state = Arc::new(Mutex::new(ServerState {
            envelopes: indexmap::indexmap! {},
            next_uid: 1,
            uidvalidity: 1,
            ..Default::default()
        }));
        {
            let mut state_lck = server_state.lock().unwrap();
            for &(_, mail) in mails.iter() {
                state_lck.insert(Box::new(mail.clone()));
            }
        }

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let local_addr = listener.local_addr().unwrap();
        let account_conf = AccountSettings {
            name: "test".to_string(),
            root_mailbox: "INBOX".to_string(),
            format: "imap".to_string(),
            identity: "user@example.com".to_string(),
            extra_identities: vec![],
            read_only: false,
            display_name: None,
            subscribed_mailboxes: vec![],
            mailboxes: indexmap::indexmap! {},
            manual_refresh: false,
            extra: indexmap::indexmap! {
                "server_hostname".to_string() => local_addr.ip().to_string(),
                "server_username".to_string() => "user".to_string(),
                "server_password".to_string() => "password".to_string(),
                "server_port".to_string() => local_addr.port().to_string(),
                "use_starttls".to_string() => "false".to_string(),
                "use_tls".to_string() => "false".to_string(),
                // Important for testing, because we expect only one connection to be used.
                "use_connection_pool".to_string() => "false".to_string(),
                "timeout".to_string() => 10_u64.to_string(),
                "fetch_body_structure".to_string() => if enabled { "true" } else { "false" }.to_string(),
            },
        };

        let mut imap =
            ImapType::new(&account_conf, Default::default(), Default::default()).unwrap();
        let listener = smol::Async::new(listener).unwrap();
        let mut is_online_fut = imap.is_online().unwrap();
        let (main_conn_sender, main_conn_receiver) = unbounded();
        let main_conn = ImapServerStream::new(
            &listener,
            &mut is_online_fut,
            (main_conn_sender.clone(), main_conn_receiver),
            Arc::clone(&server_state),
        );
        let received_commands = Arc::clone(&main_conn.received_commands);
        block_on(is_online_fut).unwrap();
        let mut mailboxes_fut = imap.mailboxes().unwrap();
        let mut main_conn_loop = Box::pin(main_conn.loop_handler("main"));
        let mailboxes = match block_on(future::select(
            mailboxes_fut.as_mut(),
            main_conn_loop.as_mut(),
        )) {
            Either::Left((value1, _)) => value1.unwrap(),
            Either::Right((value2, _)) => {
                unreachable!("{:?}", value2);
            }
        };
        let inbox_hash = *mailboxes.keys().next().unwrap();

        let loops_handle = std::thread::spawn(move || {
            block_on(main_conn_loop);
        });
        // Run inside a thread scope so that `imap` (and its connection)
        // stays alive until the server loop is quit and joined below;
        // otherwise the dropped connection makes the mock server's read
        // loop spin on EOF and starve its command channel.
        let envelopes = std::thread::scope(|scope| {
            let imap = &mut imap;
            scope
                .spawn(move || {
                    block_on(async {
                        let mut fetch_fut = imap.fetch(inbox_hash).unwrap().into_future();
                        let mut envelopes: Vec<Envelope> = vec![];
                        loop {
                            let (envs, rest) = fetch_fut.await;
                            let Some(envs) = envs else {
                                break;
                            };
                            envelopes.extend(envs.unwrap());

                            fetch_fut = rest.into_future();
                        }
                        envelopes
                    })
                })
                .join()
                .unwrap()
        });

        assert_eq!(
            envelopes.len(),
            3,
            "envelopes were not emitted: {envelopes:?}"
        );
        assert!(
            envelopes.iter().all(|env| !env.has_attachments()),
            "has_attachments must default to false without BODYSTRUCTURE"
        );
        let mut expected = mails
            .iter()
            .map(|(_, mail)| mail.envelope.clone())
            .collect::<Vec<_>>();
        let mut envelopes = envelopes;
        envelopes.sort_by_key(|env| env.date());
        for env in &mut envelopes {
            env.set_hash(EnvelopeHash(0));
        }
        for env in &mut expected {
            env.set_hash(EnvelopeHash(0));
        }
        assert_eq!(envelopes, expected);

        let frozen_uid_fetch = if enabled {
            "M16 UID FETCH 1:4 (UID FLAGS ENVELOPE BODY.PEEK[HEADER.FIELDS (REFERENCES)] \
             BODYSTRUCTURE)\r\n"
        } else {
            "M16 UID FETCH 1:4 (UID FLAGS ENVELOPE BODY.PEEK[HEADER.FIELDS (REFERENCES)])\r\n"
        };
        {
            let lck = received_commands.lock().unwrap();
            assert!(
                lck.iter().any(|l| l == frozen_uid_fetch),
                "UID FETCH command bytes differ from the expected frozen behavior: {lck:?}"
            );
            if !enabled {
                assert!(
                    !lck.iter().any(|l| l.contains("BODYSTRUCTURE")),
                    "BODYSTRUCTURE requested despite fetch_body_structure = false: {lck:?}"
                );
            }
        }

        main_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        loops_handle.join().unwrap();
    }

    /// Test that the MSN index is persisted in the sqlite cache across
    /// sessions (simulated process restarts), eliminating the
    /// `UID SEARCH 1:*` on startup.
    ///
    /// Two sequential `ImapType` instances (a fresh `UIDStore` each,
    /// simulating a process restart) share the same XDG data dir and
    /// therefore the same sqlite cache file, and connect to the same
    /// mock server.
    ///
    /// With `stable_uidvalidity == true`, a new mail is delivered
    /// between the sessions (forcing a resync round-trip with a
    /// `SELECT` under the same UIDVALIDITY): the first session's log
    /// must contain `UID SEARCH` (fresh cache), while the second
    /// session's log must contain a `SELECT` but no `UID SEARCH`,
    /// because the MSN index is restored from the cache. The second
    /// fetch must also return the new mail.
    ///
    /// With `stable_uidvalidity == false`, the server changes the
    /// mailbox's UIDVALIDITY between the sessions: the persisted MSN
    /// index is stale and must not be reused; the second session must
    /// re-issue `UID SEARCH` (rebuild) and complete the fetch.
    ///
    /// Ground truth is the mock server's received-command log.
    #[cfg(feature = "sqlite3")]
    pub(crate) fn run_imap_fetch_msn_index_persisted(stable_uidvalidity: bool) {
        let mut _logger = Logger::new_with(LogLevel::TRACE, true);
        let temp_dir = TempDir::new().unwrap();

        for var in [
            "HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
            "XDG_CONFIG_DIRS",
            "XDG_CONFIG_HOME",
            "XDG_DATA_DIRS",
            "XDG_DATA_HOME",
        ] {
            std::env::remove_var(var);
        }
        for (var, dir) in [
            ("HOME", temp_dir.path().to_path_buf()),
            ("XDG_CACHE_HOME", temp_dir.path().join(".cache")),
            ("XDG_STATE_HOME", temp_dir.path().join(".local/state")),
            ("XDG_CONFIG_HOME", temp_dir.path().join(".config")),
            ("XDG_DATA_HOME", temp_dir.path().join(".local/share")),
        ] {
            std::fs::create_dir_all(&dir).unwrap_or_else(|err| {
                panic!("Could not create {} path, {}: {}", var, dir.display(), err);
            });
            std::env::set_var(var, &dir);
        }

        let new_mail = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:00 +0000
Cc:
Subject: RE: your e-mail
Message-ID: <h2g7f.z0gy2pgaen5m@example.com>
Content-Type: text/plain

hello world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let new_mail_2 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:01 +0000
Cc:
Subject: RE: your e-mail 2
Message-ID: <h2g7f.z0gy2pgaen6m@example.com>
Content-Type: text/plain

hello world 2.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let new_mail_3 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:02 +0000
Cc:
Subject: RE: your e-mail 3
Message-ID: <h2g7f.z0gy2pgaen7m@example.com>
Content-Type: text/plain

hello world 3.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let new_mail_4 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:03 +0000
Cc:
Subject: RE: your e-mail 4
Message-ID: <h2g7f.z0gy2pgaen8m@example.com>
Content-Type: text/plain

hello world 4.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );

        let server_state = Arc::new(Mutex::new(ServerState {
            envelopes: indexmap::indexmap! {},
            next_uid: 1,
            uidvalidity: 1,
            ..Default::default()
        }));
        {
            let mut state_lck = server_state.lock().unwrap();
            for mail in [&new_mail, &new_mail_2, &new_mail_3] {
                state_lck.insert(mail.clone());
            }
        }

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let local_addr = listener.local_addr().unwrap();
        let account_conf = AccountSettings {
            name: "test".to_string(),
            root_mailbox: "INBOX".to_string(),
            format: "imap".to_string(),
            identity: "user@example.com".to_string(),
            extra_identities: vec![],
            read_only: false,
            display_name: None,
            subscribed_mailboxes: vec![],
            mailboxes: indexmap::indexmap! {},
            manual_refresh: false,
            extra: indexmap::indexmap! {
                "server_hostname".to_string() => local_addr.ip().to_string(),
                "server_username".to_string() => "user".to_string(),
                "server_password".to_string() => "password".to_string(),
                "server_port".to_string() => local_addr.port().to_string(),
                "use_starttls".to_string() => "false".to_string(),
                "use_tls".to_string() => "false".to_string(),
                // Important for testing, because we expect only one connection to be used.
                "use_connection_pool".to_string() => "false".to_string(),
                "timeout".to_string() => 1_u64.to_string(),
            },
        };

        // Session 1: fresh cache, full sync. The MSN index is built with
        // `UID SEARCH 1:*` and must be persisted in the sqlite cache.
        let mut imap =
            ImapType::new(&account_conf, Default::default(), Default::default()).unwrap();
        let listener = smol::Async::new(listener).unwrap();
        let mut is_online_fut = imap.is_online().unwrap();
        let (session1_sender, session1_receiver) = unbounded();
        let session1_conn = ImapServerStream::new(
            &listener,
            &mut is_online_fut,
            (session1_sender.clone(), session1_receiver),
            Arc::clone(&server_state),
        );
        let received_commands_1 = Arc::clone(&session1_conn.received_commands);
        block_on(is_online_fut).unwrap();
        let mut mailboxes_fut = imap.mailboxes().unwrap();
        let mut session1_conn_loop = Box::pin(session1_conn.loop_handler("session1"));
        let mailboxes = match block_on(future::select(
            mailboxes_fut.as_mut(),
            session1_conn_loop.as_mut(),
        )) {
            Either::Left((value1, _)) => value1.unwrap(),
            Either::Right((value2, _)) => {
                unreachable!("{:?}", value2);
            }
        };
        let inbox_hash = *mailboxes.keys().next().unwrap();

        let session1_loop_handle = std::thread::spawn(move || {
            block_on(session1_conn_loop);
        });
        // Run inside a thread scope so that `imap` (and its connection)
        // stays alive until the server loop is quit and joined below;
        // otherwise the dropped connection makes the mock server's read
        // loop spin on EOF and starve its command channel.
        let envelopes_1 = std::thread::scope(|scope| {
            let imap = &mut imap;
            scope
                .spawn(move || block_on(fetch_all_envs(imap, inbox_hash)))
                .join()
                .unwrap()
        });
        session1_sender.unbounded_send(ServerEvent::Quit).unwrap();
        session1_loop_handle.join().unwrap();

        assert_eq!(envelopes_1.len(), 3);
        {
            let lck = received_commands_1.lock().unwrap();
            assert!(
                lck.iter().any(|l| l.contains("UID SEARCH")),
                "first session did not send UID SEARCH: {lck:?}"
            );
        }

        // Simulated restart boundary: same server, same XDG data dir
        // (same sqlite cache file), fresh `UIDStore` in session 2.
        if stable_uidvalidity {
            // A new mail forces a resync round-trip with a SELECT under
            // the same UIDVALIDITY.
            server_state.lock().unwrap().insert(new_mail_4.clone());
        } else {
            // A UIDVALIDITY change must invalidate the persisted MSN
            // index.
            server_state.lock().unwrap().uidvalidity = 2;
        }

        // Session 2: fresh `UIDStore` (simulated process restart).
        let mut imap =
            ImapType::new(&account_conf, Default::default(), Default::default()).unwrap();
        let mut is_online_fut = imap.is_online().unwrap();
        let (session2_sender, session2_receiver) = unbounded();
        let session2_conn = ImapServerStream::new(
            &listener,
            &mut is_online_fut,
            (session2_sender.clone(), session2_receiver),
            Arc::clone(&server_state),
        );
        let received_commands_2 = Arc::clone(&session2_conn.received_commands);
        block_on(is_online_fut).unwrap();
        let mut mailboxes_fut = imap.mailboxes().unwrap();
        let mut session2_conn_loop = Box::pin(session2_conn.loop_handler("session2"));
        let mailboxes = match block_on(future::select(
            mailboxes_fut.as_mut(),
            session2_conn_loop.as_mut(),
        )) {
            Either::Left((value1, _)) => value1.unwrap(),
            Either::Right((value2, _)) => {
                unreachable!("{:?}", value2);
            }
        };
        assert_eq!(*mailboxes.keys().next().unwrap(), inbox_hash);

        let session2_loop_handle = std::thread::spawn(move || {
            block_on(session2_conn_loop);
        });
        let envelopes_2 = std::thread::scope(|scope| {
            let imap = &mut imap;
            scope
                .spawn(move || block_on(fetch_all_envs(imap, inbox_hash)))
                .join()
                .unwrap()
        });
        session2_sender.unbounded_send(ServerEvent::Quit).unwrap();
        session2_loop_handle.join().unwrap();

        let mut expected = if stable_uidvalidity {
            vec![
                new_mail.envelope.clone(),
                new_mail_2.envelope.clone(),
                new_mail_3.envelope.clone(),
                new_mail_4.envelope.clone(),
            ]
        } else {
            vec![
                new_mail.envelope.clone(),
                new_mail_2.envelope.clone(),
                new_mail_3.envelope.clone(),
            ]
        };
        assert_eq!(envelopes_2.len(), expected.len());
        let mut envelopes_2 = envelopes_2;
        envelopes_2.sort_by_key(|env| env.date());
        for env in &mut envelopes_2 {
            env.set_hash(EnvelopeHash(0));
        }
        for env in &mut expected {
            env.set_hash(EnvelopeHash(0));
        }
        assert_eq!(envelopes_2, expected);

        {
            let lck = received_commands_2.lock().unwrap();
            assert!(
                lck.iter().any(|l| l.ends_with(" SELECT INBOX\r\n")),
                "second session did not SELECT the mailbox (the test would be vacuous): {lck:?}"
            );
            if stable_uidvalidity {
                assert!(
                    !lck.iter().any(|l| l.contains("UID SEARCH")),
                    "second session re-issued UID SEARCH despite a matching persisted MSN index: \
                     {lck:?}"
                );
            } else {
                assert!(
                    lck.iter().any(|l| l.contains("UID SEARCH")),
                    "second session did not re-issue UID SEARCH after a UIDVALIDITY change: {lck:?}"
                );
            }
        }
    }

    /// Test the no-cache offline behavior: with a dead port (nothing
    /// listening) and no offline cache, the fetch stream must return
    /// `Err` (the pre-change behavior, which must stay unchanged when
    /// `cache_served_offline` is `false`).
    ///
    /// The dead-port error is also asserted to map to one of the
    /// network-ish kinds (`Network`/`TimedOut`/`OSError`) that the
    /// `ResyncCache` entry graceful-finish branch treats as "offline";
    /// `ECONNREFUSED` reaches melib via the `From<io::Error>`
    /// `raw_os_error` branch as `ErrorKind::OSError`. If this assertion
    /// ever fails, the graceful-finish predicate would never trigger for
    /// the dead-port scenario.
    ///
    /// `is_online()` is driven once first, mirroring meli's startup
    /// (the is-online job runs from t0); a failed attempt leaves the
    /// connect error in the connection so the subsequent fetch's
    /// `ResyncCache` stage sees the real network error.
    #[cfg(feature = "sqlite3")]
    pub(crate) fn run_imap_fetch_no_cache_offline_errors() {
        let mut _logger = Logger::new_with(LogLevel::TRACE, true);
        let temp_dir = TempDir::new().unwrap();

        for var in [
            "HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
            "XDG_CONFIG_DIRS",
            "XDG_CONFIG_HOME",
            "XDG_DATA_DIRS",
            "XDG_DATA_HOME",
        ] {
            std::env::remove_var(var);
        }
        for (var, dir) in [
            ("HOME", temp_dir.path().to_path_buf()),
            ("XDG_CACHE_HOME", temp_dir.path().join(".cache")),
            ("XDG_STATE_HOME", temp_dir.path().join(".local/state")),
            ("XDG_CONFIG_HOME", temp_dir.path().join(".config")),
            ("XDG_DATA_HOME", temp_dir.path().join(".local/share")),
        ] {
            std::fs::create_dir_all(&dir).unwrap_or_else(|err| {
                panic!("Could not create {} path, {}: {}", var, dir.display(), err);
            });
            std::env::set_var(var, &dir);
        }

        // Reserve a port and release it: connecting to it afterwards
        // fails immediately with `ECONNREFUSED` (loopback, no real
        // hosts or credentials involved).
        let dead_port = {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            listener.local_addr().unwrap().port()
        };
        let account_conf = AccountSettings {
            name: "test".to_string(),
            root_mailbox: "INBOX".to_string(),
            format: "imap".to_string(),
            identity: "user@example.com".to_string(),
            extra_identities: vec![],
            read_only: false,
            display_name: None,
            subscribed_mailboxes: vec![],
            mailboxes: indexmap::indexmap! {},
            manual_refresh: false,
            extra: indexmap::indexmap! {
                "server_hostname".to_string() => "127.0.0.1".to_string(),
                "server_username".to_string() => "user".to_string(),
                "server_password".to_string() => "password".to_string(),
                "server_port".to_string() => dead_port.to_string(),
                "use_starttls".to_string() => "false".to_string(),
                "use_tls".to_string() => "false".to_string(),
                "use_connection_pool".to_string() => "false".to_string(),
                "offline_cache".to_string() => "false".to_string(),
                "timeout".to_string() => 3_u64.to_string(),
            },
        };

        std::thread::spawn(move || {
            block_on(async move {
                let mut imap =
                    ImapType::new(&account_conf, Default::default(), Default::default()).unwrap();
                let mailbox_hash = MailboxHash(7);
                {
                    let mut mailboxes_lck = imap.uid_store.mailboxes.lock().await;
                    mailboxes_lck.insert(
                        mailbox_hash,
                        ImapMailbox {
                            hash: mailbox_hash,
                            imap_path: "INBOX".to_string(),
                            path: "INBOX".to_string(),
                            name: "INBOX".to_string(),
                            ..ImapMailbox::default()
                        },
                    );
                }
                let is_online_err = imap.is_online().unwrap().await.unwrap_err();
                eprintln!("dead-port is_online error: {is_online_err:?}");
                assert!(
                    is_online_err.kind.is_network()
                        || is_online_err.kind.is_timeout()
                        || is_online_err.kind.is_oserror(),
                    "dead-port error kind must be one of Network/TimedOut/OSError for the \
                     ResyncCache offline predicate, got {:?}",
                    is_online_err.kind
                );

                let fetch_fut = imap.fetch(mailbox_hash).unwrap().into_future();
                let (item, _rest) = fetch_fut.await;
                let err = item
                    .expect("fetch stream ended without yielding an item")
                    .unwrap_err();
                eprintln!("no-cache offline fetch error: {err:?}");
                assert!(
                    err.kind.is_network() || err.kind.is_timeout() || err.kind.is_oserror(),
                    "no-cache offline fetch must Err with a network-ish kind, got {:?}",
                    err.kind
                );
            });
        })
        .join()
        .unwrap();
    }

    /// Point the process' XDG environment at `temp_dir` so the IMAP
    /// offline cache (and any other state) is created under it.
    #[cfg(feature = "sqlite3")]
    fn set_test_xdg_env(temp_dir: &TempDir) {
        for var in [
            "HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
            "XDG_CONFIG_DIRS",
            "XDG_CONFIG_HOME",
            "XDG_DATA_DIRS",
            "XDG_DATA_HOME",
        ] {
            std::env::remove_var(var);
        }
        for (var, dir) in [
            ("HOME", temp_dir.path().to_path_buf()),
            ("XDG_CACHE_HOME", temp_dir.path().join(".cache")),
            ("XDG_STATE_HOME", temp_dir.path().join(".local/state")),
            ("XDG_CONFIG_HOME", temp_dir.path().join(".config")),
            ("XDG_DATA_HOME", temp_dir.path().join(".local/share")),
        ] {
            std::fs::create_dir_all(&dir).unwrap_or_else(|err| {
                panic!("Could not create {} path, {}: {}", var, dir.display(), err);
            });
            std::env::set_var(var, &dir);
        }
    }

    /// A `test` IMAP account talking to `port` on the loopback
    /// interface. No real hosts or credentials are involved.
    #[cfg(feature = "sqlite3")]
    fn imap_account_conf(port: u16) -> AccountSettings {
        AccountSettings {
            name: "test".to_string(),
            root_mailbox: "INBOX".to_string(),
            format: "imap".to_string(),
            identity: "user@example.com".to_string(),
            extra_identities: vec![],
            read_only: false,
            display_name: None,
            subscribed_mailboxes: vec![],
            mailboxes: indexmap::indexmap! {},
            manual_refresh: false,
            extra: indexmap::indexmap! {
                "server_hostname".to_string() => "127.0.0.1".to_string(),
                "server_username".to_string() => "user".to_string(),
                "server_password".to_string() => "password".to_string(),
                "server_port".to_string() => port.to_string(),
                "use_starttls".to_string() => "false".to_string(),
                "use_tls".to_string() => "false".to_string(),
                // Important for testing, because we expect only one connection to be used.
                "use_connection_pool".to_string() => "false".to_string(),
                "timeout".to_string() => 1_u64.to_string(),
            },
        }
    }

    /// The `(hash, (path, name))` identity of a mailbox list, for
    /// comparing lists across sessions; `ImapMailbox` does not
    /// implement `PartialEq`.
    #[cfg(feature = "sqlite3")]
    fn mailbox_identity(
        mailboxes: &HashMap<MailboxHash, Mailbox>,
    ) -> HashMap<MailboxHash, (String, String)> {
        mailboxes
            .iter()
            .map(|(h, m)| (*h, (m.path().to_string(), m.name().to_string())))
            .collect()
    }

    /// Corrupt the persisted `mailbox_list` payload row of the `test`
    /// account's cache database under the current XDG data dir, so that
    /// loading it fails.
    #[cfg(feature = "sqlite3")]
    fn corrupt_cached_mailbox_list(temp_dir: &TempDir) {
        use melib::utils::sqlite3::rusqlite;

        let db_path = temp_dir
            .path()
            .join(".local/share/meli/test_header_cache.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        // Not a valid serialized mailbox list; loading must fail.
        conn.execute(
            "INSERT OR REPLACE INTO mailbox_list (id, payload) VALUES (0, X'00')",
            [],
        )
        .unwrap();
    }

    /// Test that `ImapType::mailboxes` serves the mailbox list from the
    /// offline cache before touching the network.
    ///
    /// Session 1 fills the cache through a normal scripted mock session
    /// (`LIST` + `LSUB`). Session 2 is a fresh `ImapType` (empty
    /// in-memory state, simulating a process restart) pointing at a
    /// dead port: `mailboxes()` must return the cached list
    /// immediately. Since nothing listens on the port, a network
    /// roundtrip would fail with `ECONNREFUSED`; an `Ok` return is
    /// therefore proof that no network roundtrip happened.
    ///
    /// Finally, the persisted `mailbox_list` payload is corrupted
    /// directly in the sqlite cache: `mailboxes()` must still succeed
    /// via the network path (a third mock session), proving that a
    /// corrupt cache degrades to the network instead of blocking
    /// startup.
    #[cfg(feature = "sqlite3")]
    pub(crate) fn run_imap_mailboxes_cache_first() {
        let mut _logger = Logger::new_with(LogLevel::TRACE, true);
        let temp_dir = TempDir::new().unwrap();
        set_test_xdg_env(&temp_dir);

        let server_state = Arc::new(Mutex::new(ServerState {
            envelopes: indexmap::indexmap! {},
            next_uid: 1,
            uidvalidity: 1,
            ..Default::default()
        }));

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let local_addr = listener.local_addr().unwrap();

        // Session 1: fill the offline cache with the mailbox list via
        // the normal network path.
        let mut imap = ImapType::new(
            &imap_account_conf(local_addr.port()),
            Default::default(),
            Default::default(),
        )
        .unwrap();
        let listener = smol::Async::new(listener).unwrap();
        let mut is_online_fut = imap.is_online().unwrap();
        let (session1_sender, session1_receiver) = unbounded();
        let session1_conn = ImapServerStream::new(
            &listener,
            &mut is_online_fut,
            (session1_sender.clone(), session1_receiver),
            Arc::clone(&server_state),
        );
        let received_commands_1 = Arc::clone(&session1_conn.received_commands);
        block_on(is_online_fut).unwrap();
        let mut mailboxes_fut = imap.mailboxes().unwrap();
        let mut session1_conn_loop = Box::pin(session1_conn.loop_handler("session1"));
        let mailboxes_1 = match block_on(future::select(
            mailboxes_fut.as_mut(),
            session1_conn_loop.as_mut(),
        )) {
            Either::Left((value1, _)) => value1.unwrap(),
            Either::Right((value2, _)) => unreachable!("{:?}", value2),
        };
        let expected_identity = mailbox_identity(&mailboxes_1);
        assert_eq!(
            expected_identity.len(),
            1,
            "expected exactly one mailbox from the mock LIST reply: {expected_identity:?}"
        );
        {
            let lck = received_commands_1.lock().unwrap();
            assert!(
                lck.iter().any(|l| l.contains("LIST \"\" *")),
                "first session did not send LIST: {lck:?}"
            );
            assert!(
                lck.iter().any(|l| l.contains("LSUB \"\" *")),
                "first session did not send LSUB: {lck:?}"
            );
        }
        let session1_loop_handle = std::thread::spawn(move || block_on(session1_conn_loop));
        session1_sender.unbounded_send(ServerEvent::Quit).unwrap();
        session1_loop_handle.join().unwrap();

        // Session 2: fresh `UIDStore` (simulated process restart),
        // same XDG data dir (same sqlite cache file), pointing at a
        // dead port. `mailboxes()` must serve the cached list without a
        // network roundtrip; had it tried the network path, the
        // connection would fail immediately with `ECONNREFUSED`.
        let dead_port = {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            listener.local_addr().unwrap().port()
        };
        let dead_port_identity = expected_identity.clone();
        std::thread::spawn(move || {
            block_on(async move {
                let mut imap = ImapType::new(
                    &imap_account_conf(dead_port),
                    Default::default(),
                    Default::default(),
                )
                .unwrap();
                let start = std::time::Instant::now();
                let mailboxes_2 = imap.mailboxes().unwrap().await.unwrap();
                let elapsed = start.elapsed();
                assert!(
                    elapsed < Duration::from_secs(1),
                    "cached mailboxes() took {elapsed:?}; it must return without a network \
                     roundtrip (ECONNREFUSED on the dead port is sub-millisecond)"
                );
                assert_eq!(mailbox_identity(&mailboxes_2), dead_port_identity);
            });
        })
        .join()
        .unwrap();

        // Corrupt-state probe: the persisted payload is garbage;
        // `mailboxes()` must degrade to the network path and still
        // succeed (third mock session on the same listener).
        corrupt_cached_mailbox_list(&temp_dir);
        let mut imap = ImapType::new(
            &imap_account_conf(local_addr.port()),
            Default::default(),
            Default::default(),
        )
        .unwrap();
        let mut is_online_fut = imap.is_online().unwrap();
        let (session3_sender, session3_receiver) = unbounded();
        let session3_conn = ImapServerStream::new(
            &listener,
            &mut is_online_fut,
            (session3_sender.clone(), session3_receiver),
            Arc::clone(&server_state),
        );
        let received_commands_3 = Arc::clone(&session3_conn.received_commands);
        block_on(is_online_fut).unwrap();
        let mut mailboxes_fut = imap.mailboxes().unwrap();
        let mut session3_conn_loop = Box::pin(session3_conn.loop_handler("session3"));
        let mailboxes_3 = match block_on(future::select(
            mailboxes_fut.as_mut(),
            session3_conn_loop.as_mut(),
        )) {
            Either::Left((value1, _)) => value1.unwrap(),
            Either::Right((value2, _)) => unreachable!("{:?}", value2),
        };
        assert_eq!(mailbox_identity(&mailboxes_3), expected_identity);
        {
            let lck = received_commands_3.lock().unwrap();
            assert!(
                lck.iter().any(|l| l.contains("LIST \"\" *")),
                "corrupt-cache session did not fall back to the network LIST: {lck:?}"
            );
        }
        let session3_loop_handle = std::thread::spawn(move || block_on(session3_conn_loop));
        session3_sender.unbounded_send(ServerEvent::Quit).unwrap();
        session3_loop_handle.join().unwrap();
    }

    /// Test that `MailBackend::refresh_mailboxes` on the IMAP backend
    /// forces a network `LIST`/`LSUB` roundtrip even though the
    /// in-memory mailbox map is already populated, and persists the
    /// refreshed list in the offline cache.
    ///
    /// After the initial `mailboxes()` call fills the cache, the
    /// persisted `mailbox_list` row is deleted; therefore the row that
    /// a final fresh backend on a dead port loads can only have been
    /// written by `refresh_mailboxes` itself.
    #[cfg(feature = "sqlite3")]
    pub(crate) fn run_imap_refresh_mailboxes_live() {
        let mut _logger = Logger::new_with(LogLevel::TRACE, true);
        let temp_dir = TempDir::new().unwrap();
        set_test_xdg_env(&temp_dir);

        let server_state = Arc::new(Mutex::new(ServerState {
            envelopes: indexmap::indexmap! {},
            next_uid: 1,
            uidvalidity: 1,
            ..Default::default()
        }));

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let local_addr = listener.local_addr().unwrap();

        let mut imap = ImapType::new(
            &imap_account_conf(local_addr.port()),
            Default::default(),
            Default::default(),
        )
        .unwrap();
        let listener = smol::Async::new(listener).unwrap();
        let mut is_online_fut = imap.is_online().unwrap();
        let (main_conn_sender, main_conn_receiver) = unbounded();
        let main_conn = ImapServerStream::new(
            &listener,
            &mut is_online_fut,
            (main_conn_sender.clone(), main_conn_receiver),
            Arc::clone(&server_state),
        );
        let received_commands = Arc::clone(&main_conn.received_commands);
        block_on(is_online_fut).unwrap();

        // Initial mailboxes(): network LIST/LSUB, result saved to the
        // offline cache and to the in-memory map.
        let mut mailboxes_fut = imap.mailboxes().unwrap();
        let mut main_conn_loop = Box::pin(main_conn.loop_handler("main"));
        let mailboxes_1 = match block_on(future::select(
            mailboxes_fut.as_mut(),
            main_conn_loop.as_mut(),
        )) {
            Either::Left((value1, _)) => value1.unwrap(),
            Either::Right((value2, _)) => unreachable!("{:?}", value2),
        };
        let expected_identity = mailbox_identity(&mailboxes_1);
        let commands_before_refresh = {
            let lck = received_commands.lock().unwrap();
            assert!(
                lck.iter().any(|l| l.contains("LIST \"\" *")),
                "initial mailboxes() did not send LIST: {lck:?}"
            );
            assert!(
                lck.iter().any(|l| l.contains("LSUB \"\" *")),
                "initial mailboxes() did not send LSUB: {lck:?}"
            );
            lck.len()
        };
        let loops_handle = std::thread::spawn(move || block_on(main_conn_loop));

        // Delete the persisted row so that any cache content a later
        // session observes is attributable to refresh_mailboxes' save.
        {
            use melib::utils::sqlite3::rusqlite;

            let db_path = temp_dir
                .path()
                .join(".local/share/meli/test_header_cache.db");
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute("DELETE FROM mailbox_list", []).unwrap();
        }

        // refresh_mailboxes(): the in-memory map is populated and the
        // offline cache would apply, but a refresh must skip both and
        // force a network LIST/LSUB roundtrip.
        let refresh_identity = std::thread::scope(|scope| {
            let imap = &mut imap;
            scope
                .spawn(move || {
                    block_on(async {
                        let mailboxes = imap.refresh_mailboxes().unwrap().await.unwrap();
                        mailbox_identity(&mailboxes)
                    })
                })
                .join()
                .unwrap()
        });
        assert_eq!(refresh_identity, expected_identity);
        {
            let lck = received_commands.lock().unwrap();
            let commands_after_refresh = &lck[commands_before_refresh..];
            assert!(
                commands_after_refresh
                    .iter()
                    .any(|l| l.contains("LIST \"\" *")),
                "refresh_mailboxes did not force a network LIST: {lck:?}"
            );
            assert!(
                commands_after_refresh
                    .iter()
                    .any(|l| l.contains("LSUB \"\" *")),
                "refresh_mailboxes did not force a network LSUB: {lck:?}"
            );
        }

        main_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        loops_handle.join().unwrap();

        // Fresh backend on a dead port must load the refreshed list
        // from the cache; the row can only have been written by
        // refresh_mailboxes above.
        let dead_port = {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            listener.local_addr().unwrap().port()
        };
        std::thread::spawn(move || {
            block_on(async move {
                let mut imap = ImapType::new(
                    &imap_account_conf(dead_port),
                    Default::default(),
                    Default::default(),
                )
                .unwrap();
                let mailboxes_3 = imap.mailboxes().unwrap().await.unwrap();
                assert_eq!(mailbox_identity(&mailboxes_3), expected_identity);
            });
        })
        .join()
        .unwrap();
    }

    /// Sort envelopes by date and zero their hashes, so that lists from
    /// different sources (network fetch, offline cache) compare equal.
    #[cfg(feature = "sqlite3")]
    fn normalize_envs(mut envelopes: Vec<Envelope>) -> Vec<Envelope> {
        envelopes.sort_by_key(|env| env.date());
        for env in &mut envelopes {
            env.set_hash(EnvelopeHash(0));
        }
        envelopes
    }

    /// The offline startup scenario against `dead_port`: `mailboxes()`
    /// returns `expected_identity` without a network roundtrip,
    /// the dead-port connect error classifies as offline, and `fetch()`
    /// serves the cached envelopes in its first chunk and ends the
    /// stream with `Ok`.
    #[cfg(feature = "sqlite3")]
    fn dead_port_startup_check(
        dead_port: u16,
        expected_identity: &HashMap<MailboxHash, (String, String)>,
        expected: &[Envelope],
    ) {
        let expected_identity = expected_identity.clone();
        let expected = expected.to_vec();
        std::thread::spawn(move || {
            block_on(async move {
                let mut imap = ImapType::new(
                    &imap_account_conf(dead_port),
                    Default::default(),
                    Default::default(),
                )
                .unwrap();
                // Startup fast path: nothing listens on the port, so any
                // network roundtrip would fail immediately; an `Ok`
                // return within the deadline is itself proof of cache
                // use.
                let start = std::time::Instant::now();
                let mailboxes = imap.mailboxes().unwrap().await.unwrap();
                let elapsed = start.elapsed();
                assert!(
                    elapsed < Duration::from_secs(1),
                    "cached mailboxes() took {elapsed:?}; it must return without a network \
                     roundtrip (connecting to the dead port fails immediately)"
                );
                assert_eq!(mailbox_identity(&mailboxes), expected_identity);
                let inbox_hash = *mailboxes.keys().next().unwrap();

                // Mirror meli's t0 is-online job: the failed connect
                // attempt records the real network error that the fetch
                // resync stage's graceful-finish predicate matches on. A
                // kind-less `Offline` error here would mean the connect
                // never happened.
                let is_online_err = imap.is_online().unwrap().await.unwrap_err();
                eprintln!("dead-port is_online error: {is_online_err:?}");
                assert!(
                    is_online_err.kind.is_network()
                        || is_online_err.kind.is_timeout()
                        || is_online_err.kind.is_oserror(),
                    "dead-port error kind must be one of Network/TimedOut/OSError for the \
                     resync offline predicate, got {:?}",
                    is_online_err.kind
                );

                // Offline fetch: the first chunk must serve the cached
                // envelopes, and the stream must then reach its end
                // (`None`) without any `Err` item: an `Err` would mark
                // the mailbox failed in meli instead of available.
                let fetch_fut = imap.fetch(inbox_hash).unwrap().into_future();
                let (first_chunk, rest) = fetch_fut.await;
                let first_chunk = first_chunk
                    .expect("offline fetch stream ended without emitting an item")
                    .unwrap();
                assert!(
                    !first_chunk.is_empty(),
                    "first offline fetch chunk must serve the cached envelopes"
                );
                let mut envelopes = first_chunk;
                let mut fetch_fut = rest.into_future();
                loop {
                    let (chunk, rest) = fetch_fut.await;
                    let Some(chunk) = chunk else {
                        break;
                    };
                    envelopes.extend(chunk.unwrap());
                    fetch_fut = rest.into_future();
                }
                assert_eq!(
                    normalize_envs(envelopes),
                    expected,
                    "offline fetch did not serve the cached envelopes"
                );
            });
        })
        .join()
        .unwrap();
    }

    /// End-to-end offline startup test: the acceptance scenario.
    ///
    /// Session 1 (online, mock server) drives the real backend code
    /// paths — `mailboxes()` (`LIST`/`LSUB`, which persists the mailbox
    /// list) and `fetch()` (`SELECT` + `UID FETCH`, which persists the
    /// envelopes) — filling the offline cache. Session 2 is a fresh
    /// `ImapType` (empty in-memory state, simulating a process restart)
    /// pointing at a dead port, with the same XDG data dir (the same
    /// sqlite cache): `mailboxes()` must return the cached list
    /// immediately and `fetch()` must serve the cached envelopes and
    /// end the stream with `Ok` — offline startup with a warm cache.
    ///
    /// Finally, the self-heal probe: the cache database is deleted, an
    /// online session rebuilds it through the same real code paths, and
    /// another dead-port session still starts from the rebuilt cache.
    #[cfg(feature = "sqlite3")]
    pub(crate) fn run_imap_offline_startup_uses_cache() {
        let mut _logger = Logger::new_with(LogLevel::TRACE, true);
        let temp_dir = TempDir::new().unwrap();
        set_test_xdg_env(&temp_dir);

        let new_mail = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:00 +0000
Cc:
Subject: RE: your e-mail
Message-ID: <h2g7f.z0gy2pgaen5m@example.com>
Content-Type: text/plain

hello world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let new_mail_2 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Cc:
Date: Thu, 01 Jan 1970 00:00:01 +0000
Subject: RE: your e-mail 2
Message-ID: <h2g7f.z0gy2pgaen6m@example.com>
Content-Type: text/plain

hello world 2.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let new_mail_3 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Cc:
Date: Thu, 01 Jan 1970 00:00:02 +0000
Subject: RE: your e-mail 3
Message-ID: <h2g7f.z0gy2pgaen7m@example.com>
Content-Type: text/plain

hello world 3.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let mails = [
            (1 as UID, &*new_mail),
            (2 as UID, &*new_mail_2),
            (3 as UID, &*new_mail_3),
        ];
        let mut expected = mails
            .iter()
            .map(|(_, mail)| mail.envelope.clone())
            .collect::<Vec<_>>();
        for env in &mut expected {
            env.set_hash(EnvelopeHash(0));
        }

        let server_state = Arc::new(Mutex::new(ServerState {
            envelopes: indexmap::indexmap! {},
            next_uid: 1,
            uidvalidity: 1,
            ..Default::default()
        }));
        {
            let mut state_lck = server_state.lock().unwrap();
            for (_, mail) in mails {
                state_lck.insert(Box::new(mail.clone()));
            }
        }

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let local_addr = listener.local_addr().unwrap();
        // Reserve a port and release it: connecting to it afterwards
        // fails immediately (loopback, no real hosts or credentials
        // involved).
        let dead_port = {
            let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
            listener.local_addr().unwrap().port()
        };

        // Session 1 (online): fill the offline cache through the real
        // `mailboxes()` + `fetch()` code paths.
        let mut imap = ImapType::new(
            &imap_account_conf(local_addr.port()),
            Default::default(),
            Default::default(),
        )
        .unwrap();
        let listener = smol::Async::new(listener).unwrap();
        let mut is_online_fut = imap.is_online().unwrap();
        let (session1_sender, session1_receiver) = unbounded();
        let session1_conn = ImapServerStream::new(
            &listener,
            &mut is_online_fut,
            (session1_sender.clone(), session1_receiver),
            Arc::clone(&server_state),
        );
        let received_commands_1 = Arc::clone(&session1_conn.received_commands);
        block_on(is_online_fut).unwrap();
        let mut mailboxes_fut = imap.mailboxes().unwrap();
        let mut session1_conn_loop = Box::pin(session1_conn.loop_handler("session1"));
        let mailboxes_1 = match block_on(future::select(
            mailboxes_fut.as_mut(),
            session1_conn_loop.as_mut(),
        )) {
            Either::Left((value1, _)) => value1.unwrap(),
            Either::Right((value2, _)) => unreachable!("{:?}", value2),
        };
        let expected_identity = mailbox_identity(&mailboxes_1);
        assert_eq!(
            expected_identity.len(),
            1,
            "expected exactly one mailbox from the mock LIST reply: {expected_identity:?}"
        );
        let inbox_hash = *mailboxes_1.keys().next().unwrap();
        let session1_loop_handle = std::thread::spawn(move || block_on(session1_conn_loop));
        // Run inside a thread scope so `imap` (and its connection) stays
        // alive until the server loop is quit and joined below;
        // otherwise the dropped connection makes the mock server's read
        // loop spin on EOF and starve its command channel.
        let envelopes_1 = std::thread::scope(|scope| {
            let imap = &mut imap;
            scope
                .spawn(move || block_on(fetch_all_envs(imap, inbox_hash)))
                .join()
                .unwrap()
        });
        session1_sender.unbounded_send(ServerEvent::Quit).unwrap();
        session1_loop_handle.join().unwrap();
        assert_eq!(
            normalize_envs(envelopes_1),
            expected,
            "session 1 fetch did not collect the three mock mails"
        );
        {
            let lck = received_commands_1.lock().unwrap();
            assert!(
                lck.iter().any(|l| l.contains("LIST \"\" *")),
                "session 1 did not send LIST: {lck:?}"
            );
            assert!(
                lck.iter().any(|l| l.contains("LSUB \"\" *")),
                "session 1 did not send LSUB: {lck:?}"
            );
            assert!(
                lck.iter()
                    .any(|l| l.contains("UID FETCH") && l.contains("ENVELOPE")),
                "session 1 did not send a UID FETCH: {lck:?}"
            );
        }

        // Session 2 (offline): fresh backend on a dead port, same XDG
        // data dir (same sqlite cache file).
        dead_port_startup_check(dead_port, &expected_identity, &expected);

        // Self-heal probe: delete the cache database; the online session
        // below must rebuild it, and the final dead-port session must
        // still start from the rebuilt cache.
        drop(imap);
        for suffix in ["", "-wal", "-shm"] {
            let db_path = temp_dir
                .path()
                .join(format!(".local/share/meli/test_header_cache.db{suffix}"));
            if db_path.exists() {
                std::fs::remove_file(&db_path).unwrap();
            }
        }

        // Session 3 (rebuild): fresh backend on the same listener; the
        // same real code paths recreate the cache contents.
        let mut imap = ImapType::new(
            &imap_account_conf(local_addr.port()),
            Default::default(),
            Default::default(),
        )
        .unwrap();
        let mut is_online_fut = imap.is_online().unwrap();
        let (session3_sender, session3_receiver) = unbounded();
        let session3_conn = ImapServerStream::new(
            &listener,
            &mut is_online_fut,
            (session3_sender.clone(), session3_receiver),
            Arc::clone(&server_state),
        );
        block_on(is_online_fut).unwrap();
        let mut mailboxes_fut = imap.mailboxes().unwrap();
        let mut session3_conn_loop = Box::pin(session3_conn.loop_handler("session3"));
        let mailboxes_3 = match block_on(future::select(
            mailboxes_fut.as_mut(),
            session3_conn_loop.as_mut(),
        )) {
            Either::Left((value1, _)) => value1.unwrap(),
            Either::Right((value2, _)) => unreachable!("{:?}", value2),
        };
        assert_eq!(mailbox_identity(&mailboxes_3), expected_identity);
        let inbox_hash = *mailboxes_3.keys().next().unwrap();
        let session3_loop_handle = std::thread::spawn(move || block_on(session3_conn_loop));
        let envelopes_3 = std::thread::scope(|scope| {
            let imap = &mut imap;
            scope
                .spawn(move || block_on(fetch_all_envs(imap, inbox_hash)))
                .join()
                .unwrap()
        });
        session3_sender.unbounded_send(ServerEvent::Quit).unwrap();
        session3_loop_handle.join().unwrap();
        assert_eq!(
            normalize_envs(envelopes_3),
            expected,
            "rebuild session did not re-collect the three mock mails"
        );

        // Session 4 (offline again): the rebuilt cache serves startup.
        dead_port_startup_check(dead_port, &expected_identity, &expected);
    }

    /// Subjects of every `Create` refresh event received so far through the
    /// backend event consumer, flattening `RefreshBatch` events.
    fn queue_create_subjects(
        backend_event_queue: &Arc<Mutex<std::collections::VecDeque<(AccountHash, BackendEvent)>>>,
    ) -> Vec<String> {
        let queue_lck = backend_event_queue.lock().unwrap();
        let mut ret = vec![];
        for (_, event) in queue_lck.iter() {
            match event {
                BackendEvent::Refresh(RefreshEvent {
                    kind: RefreshEventKind::Create(env),
                    ..
                }) => {
                    ret.push(env.subject().to_string());
                }
                BackendEvent::RefreshBatch(events) => {
                    ret.extend(events.iter().filter_map(|event| match &event.kind {
                        RefreshEventKind::Create(env) => Some(env.subject().to_string()),
                        _ => None,
                    }));
                }
                _ => {}
            }
        }
        ret
    }

    /// Bounded watchdog deadline for the watch tests below, in the same
    /// named-constant style as `DONE_RESPONSE_TIMEOUT` in
    /// `melib/src/imap/watch.rs`.
    const WATCH_TEST_DEADLINE: Duration = Duration::from_secs(30);
    /// Poll tick for the bounded drive-loops below: while waiting for a
    /// side condition (the mock's command logs), the watch stream is
    /// re-polled at this cadence so the client keeps progressing. The
    /// tick only affects how soon a satisfied condition is noticed, never
    /// whether a test passes or fails: every wait is bounded by
    /// `WATCH_TEST_DEADLINE` and panics loudly on expiry.
    const WATCH_TEST_POLL_TICK: Duration = Duration::from_millis(25);

    /// Common setup for warm-start refresh tests: a mock server holding two
    /// seed mails, a connected backend whose mailbox list is resolved, and
    /// the main connection's server loop running in a background thread.
    #[allow(clippy::type_complexity)]
    fn warm_start_setup(
        backend_event_consumer: BackendEventConsumer,
        server_state: Arc<Mutex<ServerState>>,
        idle_heartbeat_interval: u64,
        watch_sweep_interval: u64,
        use_id: bool,
        offline_cache: bool,
    ) -> (
        Box<ImapType>,
        smol::Async<TcpListener>,
        futures::channel::mpsc::UnboundedSender<ServerEvent>,
        std::thread::JoinHandle<()>,
        MailboxHash,
        Arc<Mutex<Vec<String>>>,
    ) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let local_addr = listener.local_addr().unwrap();
        let account_conf = AccountSettings {
            name: "test".to_string(),
            root_mailbox: "INBOX".to_string(),
            format: "imap".to_string(),
            identity: "user@example.com".to_string(),
            extra_identities: vec![],
            read_only: false,
            display_name: None,
            subscribed_mailboxes: vec![],
            mailboxes: indexmap::indexmap! {},
            manual_refresh: false,
            extra: indexmap::indexmap! {
                "server_hostname".to_string() => local_addr.ip().to_string(),
                "server_username".to_string() => "user".to_string(),
                "server_password".to_string() => "password".to_string(),
                "server_port".to_string() => local_addr.port().to_string(),
                "use_starttls".to_string() => "false".to_string(),
                "use_tls".to_string() => "false".to_string(),
                // Important for testing, because we expect only one connection to be used.
                "use_connection_pool".to_string() => "false".to_string(),
                "timeout".to_string() => 1_u64.to_string(),
                "idle_heartbeat_interval".to_string() => idle_heartbeat_interval.to_string(),
                "watch_sweep_interval".to_string() => watch_sweep_interval.to_string(),
                "use_id".to_string() => use_id.to_string(),
                "offline_cache".to_string() => offline_cache.to_string(),
            },
        };

        let mut imap =
            ImapType::new(&account_conf, Default::default(), backend_event_consumer).unwrap();
        let listener = smol::Async::new(listener).unwrap();
        let mut is_online_fut = imap.is_online().unwrap();
        let (main_conn_sender, main_conn_receiver) = unbounded();
        let main_conn = ImapServerStream::new(
            &listener,
            &mut is_online_fut,
            (main_conn_sender.clone(), main_conn_receiver),
            Arc::clone(&server_state),
        );
        // With `use_id` disabled (no `M4 ID` in the handshake), the
        // connect future completes inside `ImapServerStream::new`; do not
        // await an already-completed future ("async fn resumed after
        // completion"), but do not skip a genuinely pending one either.
        if !main_conn.driver_completed_in_handshake {
            block_on(is_online_fut).unwrap();
        }
        let main_received_commands = Arc::clone(&main_conn.received_commands);
        let mut mailboxes_fut = imap.mailboxes().unwrap();
        let mut main_conn_loop = Box::pin(main_conn.loop_handler("main"));
        let mailboxes = match block_on(future::select(
            mailboxes_fut.as_mut(),
            main_conn_loop.as_mut(),
        )) {
            Either::Left((value1, _)) => value1.unwrap(),
            Either::Right((value2, _)) => {
                unreachable!("{:?}", value2);
            }
        };
        // Find INBOX by path: with `extra_mailbox` set the map has more
        // than one entry and HashMap iteration order is not stable; for
        // single-mailbox setups this returns the same value as the
        // previous `keys().next().unwrap()`.
        let inbox_hash = *mailboxes
            .iter()
            .find(|(_, f)| f.path().eq_ignore_ascii_case("inbox"))
            .map(|(h, _)| h)
            .expect("the mock always advertises an `inbox` mailbox");
        let loops_handle = std::thread::spawn(move || {
            block_on(main_conn_loop);
        });
        (
            imap,
            listener,
            main_conn_sender,
            loops_handle,
            inbox_hash,
            main_received_commands,
        )
    }

    /// Regression test for the listing `refresh` shortcut path on a warm
    /// backend: the mailbox has already been fetched once (meli has been
    /// running), a new mail is delivered to the server, and a manual
    /// `MailBackend::refresh` must emit a `Create` `RefreshEvent` for the new
    /// mail through the backend event consumer.
    pub(crate) fn run_imap_refresh_after_initial_fetch() {
        let mut _logger = Logger::new_with(LogLevel::TRACE, true);
        let temp_dir = TempDir::new().unwrap();
        let backend_event_queue =
            Arc::new(Mutex::new(std::collections::VecDeque::with_capacity(16)));
        let backend_event_consumer = {
            let backend_event_queue = Arc::clone(&backend_event_queue);

            BackendEventConsumer::new(Arc::new(move |ah, be| {
                eprintln!("BackendEventConsumer: ah {ah:?} be {be:?}");
                backend_event_queue.lock().unwrap().push_back((ah, be));
            }))
        };
        set_test_xdg_env(&temp_dir);

        let seed_mail_1 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:00 +0000
Cc:
Subject: RE: warm seed 1
Message-ID: <warm1@example.com>
Content-Type: text/plain

hello world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let seed_mail_2 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:01 +0000
Cc:
Subject: RE: warm seed 2
Message-ID: <warm2@example.com>
Content-Type: text/plain

hello world 2.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let new_mail = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:02 +0000
Cc:
Subject: RE: warm NEW mail
Message-ID: <warmnew@example.com>
Content-Type: text/plain

hello new world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );

        let server_state = Arc::new(Mutex::new(ServerState {
            envelopes: indexmap::indexmap! {},
            next_uid: 1,
            uidvalidity: 1,
            ..Default::default()
        }));
        {
            let mut state_lck = server_state.lock().unwrap();
            state_lck.insert(seed_mail_1);
            state_lck.insert(seed_mail_2);
        }

        let (mut imap, _listener, main_conn_sender, loops_handle, inbox_hash, _main_commands) =
            warm_start_setup(
                backend_event_consumer,
                Arc::clone(&server_state),
                600,
                300,
                true,
                true,
            );

        {
            let imap = &mut imap;
            let server_state = &server_state;
            let new_mail = &new_mail;
            std::thread::scope(|scope| {
                scope
                    .spawn(move || {
                        block_on(async {
                            let seed_envs = fetch_all_envs(imap, inbox_hash).await;
                            assert_eq!(
                                seed_envs.len(),
                                2,
                                "initial fetch must load the two seed mails"
                            );
                            // New mail is delivered while meli is running.
                            server_state.lock().unwrap().insert(new_mail.clone());
                            // The listing `refresh` shortcut path.
                            imap.refresh(inbox_hash).unwrap().await.unwrap();
                        });
                    })
                    .join()
                    .unwrap();
            });
        }

        let subjects = queue_create_subjects(&backend_event_queue);
        assert!(
            subjects.iter().any(|s| s == "RE: warm NEW mail"),
            "manual refresh after initial fetch did not emit a Create event for the new mail; \
             Create events so far: {subjects:?}; full queue: {:?}",
            backend_event_queue.lock().unwrap()
        );

        main_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        loops_handle.join().unwrap();
    }

    /// Failing-first regression test for the RFC 4549 §4.3.2 hazard:
    /// servers may answer `STATUS` for the connection's currently selected
    /// mailbox from the state at `SELECT` time instead of the live
    /// counters. The mock emulates such a server
    /// (`stale_status_when_selected`): after an initial fetch and a first
    /// refresh that selects INBOX on the main connection and records the
    /// STATUS baseline, **two** new mails arrive, and a manual `refresh`
    /// must emit a `Create` `RefreshEvent` for each of them — the
    /// quick-sync skip must not trust `STATUS` while the mailbox is
    /// selected on this connection, and it must not collapse a batch of
    /// new mail to the last message sequence number alone.
    pub(crate) fn run_imap_refresh_status_stale_when_selected() {
        let mut _logger = Logger::new_with(LogLevel::TRACE, true);
        let temp_dir = TempDir::new().unwrap();
        let backend_event_queue =
            Arc::new(Mutex::new(std::collections::VecDeque::with_capacity(16)));
        let backend_event_consumer = {
            let backend_event_queue = Arc::clone(&backend_event_queue);

            BackendEventConsumer::new(Arc::new(move |ah, be| {
                eprintln!("BackendEventConsumer: ah {ah:?} be {be:?}");
                backend_event_queue.lock().unwrap().push_back((ah, be));
            }))
        };
        set_test_xdg_env(&temp_dir);

        let seed_mail_1 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:00 +0000
Cc:
Subject: RE: warm seed 1
Message-ID: <warm1@example.com>
Content-Type: text/plain

hello world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let seed_mail_2 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:01 +0000
Cc:
Subject: RE: warm seed 2
Message-ID: <warm2@example.com>
Content-Type: text/plain

hello world 2.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let new_mail = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:02 +0000
Cc:
Subject: RE: warm NEW mail
Message-ID: <warmnew@example.com>
Content-Type: text/plain

hello new world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let new_mail_b = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:03 +0000
Cc:
Subject: RE: warm NEW mail B
Message-ID: <warmnewb@example.com>
Content-Type: text/plain

hello new world b.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );

        let server_state = Arc::new(Mutex::new(ServerState {
            envelopes: indexmap::indexmap! {},
            next_uid: 1,
            uidvalidity: 1,
            stale_status_when_selected: true,
            idle_no_push: false,
            push_glued_at_idle_start: false,
            replay_real_push_bytes: false,
            ignore_done: false,
            suppress_push_when_multi_selected: false,
            id_gated_push: false,
            glue_exists_after_done: false,
            push_before_continuation: false,
            selected_sessions: Default::default(),
            extra_mailbox: None,
            uid_fetch_flags_drop_uid: false,
            uid_fetch_drop_uid_all: false,
        }));
        {
            let mut state_lck = server_state.lock().unwrap();
            state_lck.insert(seed_mail_1);
            state_lck.insert(seed_mail_2);
        }

        let (mut imap, _listener, main_conn_sender, loops_handle, inbox_hash, _main_commands) =
            warm_start_setup(
                backend_event_consumer,
                Arc::clone(&server_state),
                600,
                300,
                true,
                true,
            );

        {
            let imap = &mut imap;
            let server_state = &server_state;
            let new_mail = &new_mail;
            std::thread::scope(|scope| {
                scope
                    .spawn(move || {
                        block_on(async {
                            let seed_envs = fetch_all_envs(imap, inbox_hash).await;
                            assert_eq!(
                                seed_envs.len(),
                                2,
                                "initial fetch must load the two seed mails"
                            );
                            // First refresh with no changes: runs the full
                            // resync (SELECT INBOX on this connection) and
                            // records the STATUS baseline.
                            imap.refresh(inbox_hash).unwrap().await.unwrap();
                            // Two new mails are delivered at once while meli
                            // is running, so the refresh must catch both:
                            // flushing only the last message sequence number
                            // (or skipping the resync) is not enough.
                            {
                                let mut state_lck = server_state.lock().unwrap();
                                state_lck.insert(new_mail.clone());
                                state_lck.insert(new_mail_b.clone());
                            }
                            // Second refresh: a stale server would answer the
                            // quick-check STATUS with the select-time
                            // snapshot, equal to the baseline.
                            imap.refresh(inbox_hash).unwrap().await.unwrap();
                        });
                    })
                    .join()
                    .unwrap();
            });
        }

        let subjects = queue_create_subjects(&backend_event_queue);
        for expected in ["RE: warm NEW mail", "RE: warm NEW mail B"] {
            assert!(
                subjects.iter().any(|s| s == expected),
                "manual refresh did not emit a Create event for {expected:?}; the stale STATUS \
                 quick check skipped the resync (or only the last new message was fetched); \
                 Create events so far: {subjects:?}; full queue: {:?}",
                backend_event_queue.lock().unwrap()
            );
        }

        main_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        loops_handle.join().unwrap();
    }

    /// Regression test for the `watch` stream on a warm backend: the mailbox
    /// has already been fetched once (meli has been running), a new mail is
    /// delivered while meli is running, and the `ImapType::watch` stream
    /// must emit a `Create` `RefreshEvent` for it without a restart.
    pub(crate) fn run_imap_watch_after_initial_fetch() {
        let mut _logger = Logger::new_with(LogLevel::TRACE, true);
        let temp_dir = TempDir::new().unwrap();
        let backend_event_queue =
            Arc::new(Mutex::new(std::collections::VecDeque::with_capacity(16)));
        let backend_event_consumer = {
            let backend_event_queue = Arc::clone(&backend_event_queue);

            BackendEventConsumer::new(Arc::new(move |ah, be| {
                eprintln!("BackendEventConsumer: ah {ah:?} be {be:?}");
                backend_event_queue.lock().unwrap().push_back((ah, be));
            }))
        };
        set_test_xdg_env(&temp_dir);

        let seed_mail_1 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:00 +0000
Cc:
Subject: RE: warm seed 1
Message-ID: <warm1@example.com>
Content-Type: text/plain

hello world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let seed_mail_2 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:01 +0000
Cc:
Subject: RE: warm seed 2
Message-ID: <warm2@example.com>
Content-Type: text/plain

hello world 2.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let new_mail = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:02 +0000
Cc:
Subject: RE: warm NEW mail
Message-ID: <warmnew@example.com>
Content-Type: text/plain

hello new world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );

        let server_state = Arc::new(Mutex::new(ServerState {
            envelopes: indexmap::indexmap! {},
            next_uid: 1,
            uidvalidity: 1,
            ..Default::default()
        }));
        {
            let mut state_lck = server_state.lock().unwrap();
            state_lck.insert(seed_mail_1);
            state_lck.insert(seed_mail_2);
        }

        let (mut imap, listener, main_conn_sender, loops_handle, inbox_hash, _main_commands) =
            warm_start_setup(
                backend_event_consumer,
                Arc::clone(&server_state),
                600,
                300,
                true,
                true,
            );

        {
            let imap = &mut imap;
            std::thread::scope(|scope| {
                scope
                    .spawn(move || {
                        block_on(async {
                            let seed_envs = fetch_all_envs(imap, inbox_hash).await;
                            assert_eq!(
                                seed_envs.len(),
                                2,
                                "initial fetch must load the two seed mails"
                            );
                        });
                    })
                    .join()
                    .unwrap();
            });
        }

        let mut watch_fut = Box::pin(imap.watch().unwrap().into_future());
        let (watch_conn_sender, watch_conn_receiver) = unbounded();
        let watch_conn = ImapServerStream::new(
            &listener,
            &mut watch_fut,
            (watch_conn_sender.clone(), watch_conn_receiver),
            Arc::clone(&server_state),
        );
        // New mail is delivered while meli is running.
        watch_conn_sender
            .unbounded_send(ServerEvent::New(new_mail))
            .unwrap();
        let watch_conn_loop = watch_conn.loop_handler("watch");
        let watch_loops_handle = std::thread::spawn(move || {
            block_on(watch_conn_loop);
        });

        block_on(async {
            let mut found = false;
            while !found {
                let item = match future::select(
                    watch_fut.as_mut(),
                    smol::Timer::after(Duration::from_secs(30)),
                )
                .await
                {
                    Either::Left(((item, rest), _timeout)) => {
                        watch_fut = Box::pin(rest.into_future());
                        item
                    }
                    Either::Right((_timeout, _pending)) => {
                        panic!(
                            "watch stream did not emit the new mail's Create event within 30 \
                             seconds"
                        );
                    }
                };
                let Some(backend_event) = item else {
                    panic!("watch stream ended before the new mail's Create event");
                };
                match backend_event.unwrap() {
                    BackendEvent::RefreshBatch(events) => {
                        found = events.iter().any(|event| {
                            matches!(
                                &event.kind,
                                RefreshEventKind::Create(env)
                                    if env.subject() == "RE: warm NEW mail"
                            )
                        });
                    }
                    BackendEvent::Refresh(event) => {
                        found = matches!(
                            &event.kind,
                            RefreshEventKind::Create(env)
                                if env.subject() == "RE: warm NEW mail"
                        );
                    }
                    other => {
                        panic!("Expected Refresh event, got: {other:?}");
                    }
                }
            }
        });

        watch_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        main_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        watch_loops_handle.join().unwrap();
        loops_handle.join().unwrap();
    }

    /// Regression test for servers that never deliver untagged updates
    /// during IDLE (`idle_no_push`): the mailbox has already been fetched
    /// once (meli has been running), a new mail is delivered while the
    /// `ImapType::watch` stream is idling, and the server never sends
    /// `* EXISTS` on the IDLE connection. The watch heartbeat wake-up
    /// must re-sync the watched mailbox and emit a `Create`
    /// `RefreshEvent` for the new mail.
    pub(crate) fn run_imap_watch_idle_no_push() {
        let mut _logger = Logger::new_with(LogLevel::TRACE, true);
        let temp_dir = TempDir::new().unwrap();
        let backend_event_queue =
            Arc::new(Mutex::new(std::collections::VecDeque::with_capacity(16)));
        let backend_event_consumer = {
            let backend_event_queue = Arc::clone(&backend_event_queue);

            BackendEventConsumer::new(Arc::new(move |ah, be| {
                eprintln!("BackendEventConsumer: ah {ah:?} be {be:?}");
                backend_event_queue.lock().unwrap().push_back((ah, be));
            }))
        };
        set_test_xdg_env(&temp_dir);

        let seed_mail_1 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:00 +0000
Cc:
Subject: RE: warm seed 1
Message-ID: <warm1@example.com>
Content-Type: text/plain

hello world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let seed_mail_2 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:01 +0000
Cc:
Subject: RE: warm seed 2
Message-ID: <warm2@example.com>
Content-Type: text/plain

hello world 2.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let new_mail = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:02 +0000
Cc:
Subject: RE: warm NEW mail
Message-ID: <warmnew@example.com>
Content-Type: text/plain

hello new world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );

        let server_state = Arc::new(Mutex::new(ServerState {
            envelopes: indexmap::indexmap! {},
            next_uid: 1,
            uidvalidity: 1,
            idle_no_push: true,
            ..Default::default()
        }));
        {
            let mut state_lck = server_state.lock().unwrap();
            state_lck.insert(seed_mail_1);
            state_lck.insert(seed_mail_2);
        }

        // Short heartbeat so the compensating resync runs quickly.
        let (mut imap, listener, main_conn_sender, loops_handle, inbox_hash, _main_commands) =
            warm_start_setup(
                backend_event_consumer,
                Arc::clone(&server_state),
                2,
                300,
                true,
                true,
            );

        {
            let imap = &mut imap;
            std::thread::scope(|scope| {
                scope
                    .spawn(move || {
                        block_on(async {
                            let seed_envs = fetch_all_envs(imap, inbox_hash).await;
                            assert_eq!(
                                seed_envs.len(),
                                2,
                                "initial fetch must load the two seed mails"
                            );
                        });
                    })
                    .join()
                    .unwrap();
            });
        }

        let mut watch_fut = Box::pin(imap.watch().unwrap().into_future());
        let (watch_conn_sender, watch_conn_receiver) = unbounded();
        let watch_conn = ImapServerStream::new(
            &listener,
            &mut watch_fut,
            (watch_conn_sender.clone(), watch_conn_receiver),
            Arc::clone(&server_state),
        );
        // New mail is delivered while meli is running; the mock records it
        // but (idle_no_push) never pushes `* EXISTS` to the IDLE
        // connection.
        watch_conn_sender
            .unbounded_send(ServerEvent::New(new_mail))
            .unwrap();
        let watch_conn_loop = watch_conn.loop_handler("watch");
        let watch_loops_handle = std::thread::spawn(move || {
            block_on(watch_conn_loop);
        });

        block_on(async {
            let mut found = false;
            while !found {
                let item = match future::select(
                    watch_fut.as_mut(),
                    smol::Timer::after(Duration::from_secs(30)),
                )
                .await
                {
                    Either::Left(((item, rest), _timeout)) => {
                        watch_fut = Box::pin(rest.into_future());
                        item
                    }
                    Either::Right((_timeout, _pending)) => {
                        panic!(
                            "watch stream did not emit the new mail's Create event within 30 \n                             seconds even though the heartbeat should have re-synced it"
                        );
                    }
                };
                let Some(backend_event) = item else {
                    panic!("watch stream ended before the new mail's Create event");
                };
                match backend_event.unwrap() {
                    BackendEvent::RefreshBatch(events) => {
                        found = events.iter().any(|event| {
                            matches!(
                                &event.kind,
                                RefreshEventKind::Create(env)
                                    if env.subject() == "RE: warm NEW mail"
                            )
                        });
                    }
                    BackendEvent::Refresh(event) => {
                        found = matches!(
                            &event.kind,
                            RefreshEventKind::Create(env)
                                if env.subject() == "RE: warm NEW mail"
                        );
                    }
                    other => {
                        panic!("Expected Refresh event, got: {other:?}");
                    }
                }
            }
        });

        watch_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        main_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        watch_loops_handle.join().unwrap();
        loops_handle.join().unwrap();
    }

    /// Regression test for a server push that arrives glued to the `+ idling`
    /// greeting in a single TCP segment (`"+ idling\r\n* n EXISTS\r\n"`).
    /// The low-level `ImapBlockingConnection` read path may deliver several
    /// lines from one socket read; the IDLE watch loop must split them,
    /// filter out the continuation line, and still process the `EXISTS`
    /// push into a `Create` `RefreshEvent` for the new mail.
    pub(crate) fn run_imap_watch_push_glued_to_idling_greeting() {
        let mut _logger = Logger::new_with(LogLevel::TRACE, true);
        let temp_dir = TempDir::new().unwrap();
        let backend_event_queue =
            Arc::new(Mutex::new(std::collections::VecDeque::with_capacity(16)));
        let backend_event_consumer = {
            let backend_event_queue = Arc::clone(&backend_event_queue);

            BackendEventConsumer::new(Arc::new(move |ah, be| {
                eprintln!("BackendEventConsumer: ah {ah:?} be {be:?}");
                backend_event_queue.lock().unwrap().push_back((ah, be));
            }))
        };
        set_test_xdg_env(&temp_dir);

        let seed_mail_1 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:00 +0000
Cc:
Subject: RE: warm seed 1
Message-ID: <warm1@example.com>
Content-Type: text/plain

hello world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let seed_mail_2 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:01 +0000
Cc:
Subject: RE: warm seed 2
Message-ID: <warm2@example.com>
Content-Type: text/plain

hello world 2.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let new_mail = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:02 +0000
Cc:
Subject: RE: warm NEW mail
Message-ID: <warmnew@example.com>
Content-Type: text/plain

hello new world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );

        let server_state = Arc::new(Mutex::new(ServerState {
            envelopes: indexmap::indexmap! {},
            next_uid: 1,
            uidvalidity: 1,
            push_glued_at_idle_start: true,
            ..Default::default()
        }));
        {
            let mut state_lck = server_state.lock().unwrap();
            state_lck.insert(seed_mail_1);
            state_lck.insert(seed_mail_2);
        }

        let (mut imap, listener, main_conn_sender, loops_handle, inbox_hash, _main_commands) =
            warm_start_setup(
                backend_event_consumer,
                Arc::clone(&server_state),
                600,
                300,
                true,
                true,
            );

        {
            let imap = &mut imap;
            std::thread::scope(|scope| {
                scope
                    .spawn(move || {
                        block_on(async {
                            let seed_envs = fetch_all_envs(imap, inbox_hash).await;
                            assert_eq!(
                                seed_envs.len(),
                                2,
                                "initial fetch must load the two seed mails"
                            );
                        });
                    })
                    .join()
                    .unwrap();
            });
        }

        let mut watch_fut = Box::pin(imap.watch().unwrap().into_future());
        let (watch_conn_sender, watch_conn_receiver) = unbounded();
        let watch_conn = ImapServerStream::new(
            &listener,
            &mut watch_fut,
            (watch_conn_sender.clone(), watch_conn_receiver),
            Arc::clone(&server_state),
        );
        // New mail is delivered before the watch stream enters IDLE; the mock
        // buffers it and glues `* EXISTS` to the `+ idling` greeting so both
        // lines arrive in one TCP segment.
        watch_conn_sender
            .unbounded_send(ServerEvent::New(new_mail))
            .unwrap();
        let watch_conn_loop = watch_conn.loop_handler("watch");
        let watch_loops_handle = std::thread::spawn(move || {
            block_on(watch_conn_loop);
        });

        block_on(async {
            let mut found = false;
            while !found {
                let item = match future::select(
                    watch_fut.as_mut(),
                    smol::Timer::after(Duration::from_secs(30)),
                )
                .await
                {
                    Either::Left(((item, rest), _timeout)) => {
                        watch_fut = Box::pin(rest.into_future());
                        item
                    }
                    Either::Right((_timeout, _pending)) => {
                        panic!(
                            "watch stream did not emit the new mail's Create event within 30 \
                             seconds even though the EXISTS push was glued to the IDLE greeting"
                        );
                    }
                };
                let Some(backend_event) = item else {
                    panic!("watch stream ended before the new mail's Create event");
                };
                match backend_event.unwrap() {
                    BackendEvent::RefreshBatch(events) => {
                        found = events.iter().any(|event| {
                            matches!(
                                &event.kind,
                                RefreshEventKind::Create(env)
                                    if env.subject() == "RE: warm NEW mail"
                            )
                        });
                    }
                    BackendEvent::Refresh(event) => {
                        found = matches!(
                            &event.kind,
                            RefreshEventKind::Create(env)
                                if env.subject() == "RE: warm NEW mail"
                        );
                    }
                    other => {
                        panic!("Expected Refresh event, got: {other:?}");
                    }
                }
            }
        });

        watch_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        main_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        watch_loops_handle.join().unwrap();
        loops_handle.join().unwrap();
    }

    /// Replay fixture for the real captured push bytes: the IDLE-arm
    /// glued payload is written in the byte shape observed in the real
    /// QQ IMAP capture (`/tmp/opencode/meli-qq-trace.log`, 2026-09-13
    /// session; excerpts quoted with line refs in
    /// `.omo/evidence/fix-imap-idle-push/task-10-acceptance.md`).
    /// Protocol bytes only: the capture never contained credentials
    /// and LOGIN/AUTH token lines are not replayed here).
    ///
    /// Provenance of the fixture bytes:
    ///
    /// - `+ idling\r\n` — REAL, verbatim: the capture logs a 10-byte
    ///   socket read `"+ idling\r\n"` (trace lines 1025-1027).
    /// - `* 3 EXISTS\r\n` — real shape: the capture's EXISTS lines are
    ///   e.g. `* 4947 EXISTS` (trace lines 273, 371).
    /// - `* 0 RECENT\r\n` — REAL pairing: the captured server follows
    ///   every `* n EXISTS` with a `* 0 RECENT` line (trace lines
    ///   176-177, 273-274, 1484-1485); the push-time pairing replays
    ///   that observed shape.
    /// - Canonical, NOT in the capture (it predates the wire
    ///   instrumentation of the fix stack): the connection preamble
    ///   (`M1`/`M3 CAPABILITY` replies — melib's IMAP handshake reads no
    ///   server greeting, so none is replayed), the `DONE` tagged reply
    ///   (`M{k} OK IDLE terminated\r\n`, RFC 2177 shape) and the resync
    ///   `FETCH` replies. The captured tagged-reply phrasing
    ///   (`M17 OK NOOP Completed\r\n`, trace lines 487/1170/1604/2876)
    ///   belongs to the heartbeat NOOP path, not the push path, and is
    ///   not replayed here (the heartbeat fires at 600 s in this test).
    ///
    /// The fixture bytes are DATA: the test asserts the mock wrote
    /// exactly the expected real-shape payload to the wire and that the
    /// client processed it into the new mail's `Create` `RefreshEvent`,
    /// exactly like the wire would deliver it. Bounded by
    /// `WATCH_TEST_DEADLINE`; no sleeps.
    pub(crate) fn run_imap_watch_replay_real_push_bytes() {
        let mut _logger = Logger::new_with(LogLevel::TRACE, true);
        let temp_dir = TempDir::new().unwrap();
        let backend_event_queue =
            Arc::new(Mutex::new(std::collections::VecDeque::with_capacity(16)));
        let backend_event_consumer = {
            let backend_event_queue = Arc::clone(&backend_event_queue);

            BackendEventConsumer::new(Arc::new(move |ah, be| {
                eprintln!("BackendEventConsumer: ah {ah:?} be {be:?}");
                backend_event_queue.lock().unwrap().push_back((ah, be));
            }))
        };
        set_test_xdg_env(&temp_dir);

        let seed_mail_1 = t3_test_mail("RE: replay seed 1", "replay-seed-1@example.com");
        let seed_mail_2 = t3_test_mail("RE: replay seed 2", "replay-seed-2@example.com");
        let new_mail = t3_test_mail("RE: replay NEW mail", "replay-new@example.com");

        let server_state = Arc::new(Mutex::new(ServerState {
            envelopes: indexmap::indexmap! {},
            next_uid: 1,
            uidvalidity: 1,
            push_glued_at_idle_start: true,
            replay_real_push_bytes: true,
            ..Default::default()
        }));
        {
            let mut state_lck = server_state.lock().unwrap();
            state_lck.insert(seed_mail_1);
            state_lck.insert(seed_mail_2);
        }

        let (mut imap, listener, main_conn_sender, loops_handle, inbox_hash, _main_commands) =
            warm_start_setup(
                backend_event_consumer,
                Arc::clone(&server_state),
                600,
                300,
                true,
                true,
            );

        {
            let imap = &mut imap;
            std::thread::scope(|scope| {
                scope
                    .spawn(move || {
                        block_on(async {
                            let seed_envs = fetch_all_envs(imap, inbox_hash).await;
                            assert_eq!(
                                seed_envs.len(),
                                2,
                                "initial fetch must load the two seed mails"
                            );
                        });
                    })
                    .join()
                    .unwrap();
            });
        }

        let mut watch_fut = Box::pin(imap.watch().unwrap().into_future());
        let (watch_conn_sender, watch_conn_receiver) = unbounded();
        let watch_conn = ImapServerStream::new(
            &listener,
            &mut watch_fut,
            (watch_conn_sender.clone(), watch_conn_receiver),
            Arc::clone(&server_state),
        );
        let watch_idle_greeting_bytes = Arc::clone(&watch_conn.idle_greeting_bytes);
        let watch_idle_received_lines = Arc::clone(&watch_conn.idle_received_lines);
        // New mail is delivered before the watch stream enters IDLE; the
        // mock buffers it and writes the real-shape replay payload
        // (`+ idling\r\n* 3 EXISTS\r\n* 0 RECENT\r\n`) as one TCP write.
        watch_conn_sender
            .unbounded_send(ServerEvent::New(new_mail))
            .unwrap();
        let watch_conn_loop = watch_conn.loop_handler("watch");
        let watch_loops_handle = std::thread::spawn(move || {
            block_on(watch_conn_loop);
        });

        block_on(async {
            let mut found = false;
            while !found {
                let item = match future::select(
                    watch_fut.as_mut(),
                    smol::Timer::after(WATCH_TEST_DEADLINE),
                )
                .await
                {
                    Either::Left(((item, rest), _timeout)) => {
                        watch_fut = Box::pin(rest.into_future());
                        item
                    }
                    Either::Right((_timeout, _pending)) => {
                        panic!(
                            "watch stream did not emit the new mail's Create event within \
                             {WATCH_TEST_DEADLINE:?}; the real-shape replay push \
                             (`+ idling` + `* EXISTS` + `* RECENT` in one write) was not \
                             processed"
                        );
                    }
                };
                let Some(backend_event) = item else {
                    panic!("watch stream ended before the new mail's Create event");
                };
                match backend_event {
                    Err(err) => {
                        panic!(
                            "replay: watch stream errored instead of delivering the \
                             real-shape push: {err}"
                        );
                    }
                    Ok(BackendEvent::RefreshBatch(events)) => {
                        found = events.iter().any(|event| {
                            matches!(
                                &event.kind,
                                RefreshEventKind::Create(env)
                                    if env.subject() == "RE: replay NEW mail"
                            )
                        });
                    }
                    Ok(BackendEvent::Refresh(event)) => {
                        found = matches!(
                            &event.kind,
                            RefreshEventKind::Create(env)
                                if env.subject() == "RE: replay NEW mail"
                        );
                    }
                    Ok(other) => {
                        panic!("Expected Refresh event, got: {other:?}");
                    }
                }
            }
        });

        // DATA discipline: the replay fixture bytes are external capture
        // data; assert the mock wrote exactly the expected payload to the
        // wire. Two seed mails + the new one => `* 3 EXISTS`. The Create
        // event above can only follow the client's processing of this
        // write, so the record is complete here without any waiting.
        assert_eq!(
            watch_idle_greeting_bytes.lock().unwrap().as_slice(),
            b"+ idling\r\n* 3 EXISTS\r\n* 0 RECENT\r\n",
            "replay fixture: the IDLE-arm write must be byte-identical to the \
             real-shape payload (real `+ idling\\r\\n`; `* n EXISTS` paired with \
             `* 0 RECENT` as the capture shows)"
        );
        // The push path must have gone through the DONE exchange: the
        // client terminates IDLE (`DONE\r\n`) before the resync FETCH
        // that produced the Create event, so the mock's idle-line log is
        // complete here without any waiting.
        assert!(
            watch_idle_received_lines
                .lock()
                .unwrap()
                .iter()
                .any(|line| line == "DONE\r\n"),
            "replay fixture: the client must send DONE after the real-shape \
             push, before the resync FETCH"
        );

        watch_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        main_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        watch_loops_handle.join().unwrap();
        loops_handle.join().unwrap();
    }

    /// Test for the compensating resync at watch startup: the mailbox was
    /// fetched once (2 mails), a third mail was delivered to the server
    /// while meli was not watching, and the watch stream must notice the
    /// `EXISTS` growth in its opening EXAMINE and emit a `Create`
    /// `RefreshEvent` for it.
    pub(crate) fn run_imap_watch_startup_compensation() {
        let mut _logger = Logger::new_with(LogLevel::TRACE, true);
        let temp_dir = TempDir::new().unwrap();
        let backend_event_queue =
            Arc::new(Mutex::new(std::collections::VecDeque::with_capacity(16)));
        let backend_event_consumer = {
            let backend_event_queue = Arc::clone(&backend_event_queue);

            BackendEventConsumer::new(Arc::new(move |ah, be| {
                eprintln!("BackendEventConsumer: ah {ah:?} be {be:?}");
                backend_event_queue.lock().unwrap().push_back((ah, be));
            }))
        };
        set_test_xdg_env(&temp_dir);

        let seed_mail_1 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:00 +0000
Cc:
Subject: RE: warm seed 1
Message-ID: <warm1@example.com>
Content-Type: text/plain

hello world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let seed_mail_2 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:01 +0000
Cc:
Subject: RE: warm seed 2
Message-ID: <warm2@example.com>
Content-Type: text/plain

hello world 2.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let new_mail = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:02 +0000
Cc:
Subject: RE: warm NEW mail
Message-ID: <warmnew@example.com>
Content-Type: text/plain

hello new world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );

        let server_state = Arc::new(Mutex::new(ServerState {
            envelopes: indexmap::indexmap! {},
            next_uid: 1,
            uidvalidity: 1,
            ..Default::default()
        }));
        {
            let mut state_lck = server_state.lock().unwrap();
            state_lck.insert(seed_mail_1);
            state_lck.insert(seed_mail_2);
        }

        let (mut imap, listener, main_conn_sender, loops_handle, inbox_hash, _main_commands) =
            warm_start_setup(
                backend_event_consumer,
                Arc::clone(&server_state),
                600,
                300,
                true,
                true,
            );

        {
            let imap = &mut imap;
            std::thread::scope(|scope| {
                scope
                    .spawn(move || {
                        block_on(async {
                            let seed_envs = fetch_all_envs(imap, inbox_hash).await;
                            assert_eq!(
                                seed_envs.len(),
                                2,
                                "initial fetch must load the two seed mails"
                            );
                        });
                    })
                    .join()
                    .unwrap();
            });
        }

        // A third mail is delivered on the server side while no watch
        // stream is running (no push is sent; the state just changes).
        server_state.lock().unwrap().insert(new_mail);

        let mut watch_fut = Box::pin(imap.watch().unwrap().into_future());
        let (watch_conn_sender, watch_conn_receiver) = unbounded();
        let watch_conn = ImapServerStream::new(
            &listener,
            &mut watch_fut,
            (watch_conn_sender.clone(), watch_conn_receiver),
            Arc::clone(&server_state),
        );
        let watch_conn_loop = watch_conn.loop_handler("watch");
        let watch_loops_handle = std::thread::spawn(move || {
            block_on(watch_conn_loop);
        });

        block_on(async {
            let mut found = false;
            while !found {
                let item = match future::select(
                    watch_fut.as_mut(),
                    smol::Timer::after(Duration::from_secs(30)),
                )
                .await
                {
                    Either::Left(((item, rest), _timeout)) => {
                        watch_fut = Box::pin(rest.into_future());
                        item
                    }
                    Either::Right((_timeout, _pending)) => {
                        panic!(
                            "watch stream did not emit the delivered mail's Create event within \
                             30 seconds even though the startup compensating resync should have \
                             fetched it"
                        );
                    }
                };
                let Some(backend_event) = item else {
                    panic!("watch stream ended before the new mail's Create event");
                };
                match backend_event.unwrap() {
                    BackendEvent::RefreshBatch(events) => {
                        found = events.iter().any(|event| {
                            matches!(
                                &event.kind,
                                RefreshEventKind::Create(env)
                                    if env.subject() == "RE: warm NEW mail"
                            )
                        });
                    }
                    BackendEvent::Refresh(event) => {
                        found = matches!(
                            &event.kind,
                            RefreshEventKind::Create(env)
                                if env.subject() == "RE: warm NEW mail"
                        );
                    }
                    other => {
                        panic!("Expected Refresh event, got: {other:?}");
                    }
                }
            }
        });

        watch_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        main_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        watch_loops_handle.join().unwrap();
        loops_handle.join().unwrap();
    }

    /// Failing-first regression test for the single-mail FETCH bug in
    /// `process_untagged(Exists)`: two mails are delivered before IDLE starts
    /// and glued to the greeting as a single `* n EXISTS` push. The push
    /// reports the final total, so fetching only the last message sequence
    /// number would drop the second-to-last mail; the FETCH range must start
    /// at the first mail not known locally and both Create events must
    /// arrive.
    pub(crate) fn run_imap_watch_push_exists_multi_mail() {
        let mut _logger = Logger::new_with(LogLevel::TRACE, true);
        let temp_dir = TempDir::new().unwrap();
        let backend_event_queue =
            Arc::new(Mutex::new(std::collections::VecDeque::with_capacity(16)));
        let backend_event_consumer = {
            let backend_event_queue = Arc::clone(&backend_event_queue);

            BackendEventConsumer::new(Arc::new(move |ah, be| {
                eprintln!("BackendEventConsumer: ah {ah:?} be {be:?}");
                backend_event_queue.lock().unwrap().push_back((ah, be));
            }))
        };
        set_test_xdg_env(&temp_dir);

        let seed_mail_1 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:00 +0000
Cc:
Subject: RE: warm seed 1
Message-ID: <warm1@example.com>
Content-Type: text/plain

hello world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let seed_mail_2 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:01 +0000
Cc:
Subject: RE: warm seed 2
Message-ID: <warm2@example.com>
Content-Type: text/plain

hello world 2.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let new_mail_a = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:02 +0000
Cc:
Subject: RE: warm NEW mail A
Message-ID: <warmnewa@example.com>
Content-Type: text/plain

hello new world a.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let new_mail_b = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:03 +0000
Cc:
Subject: RE: warm NEW mail B
Message-ID: <warmnewb@example.com>
Content-Type: text/plain

hello new world b.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );

        let server_state = Arc::new(Mutex::new(ServerState {
            envelopes: indexmap::indexmap! {},
            next_uid: 1,
            uidvalidity: 1,
            push_glued_at_idle_start: true,
            ..Default::default()
        }));
        {
            let mut state_lck = server_state.lock().unwrap();
            state_lck.insert(seed_mail_1);
            state_lck.insert(seed_mail_2);
        }

        let (mut imap, listener, main_conn_sender, loops_handle, inbox_hash, _main_commands) =
            warm_start_setup(
                backend_event_consumer,
                Arc::clone(&server_state),
                600,
                300,
                true,
                true,
            );

        {
            let imap = &mut imap;
            std::thread::scope(|scope| {
                scope
                    .spawn(move || {
                        block_on(async {
                            let seed_envs = fetch_all_envs(imap, inbox_hash).await;
                            assert_eq!(
                                seed_envs.len(),
                                2,
                                "initial fetch must load the two seed mails"
                            );
                        });
                    })
                    .join()
                    .unwrap();
            });
        }

        let mut watch_fut = Box::pin(imap.watch().unwrap().into_future());
        let (watch_conn_sender, watch_conn_receiver) = unbounded();
        let watch_conn = ImapServerStream::new(
            &listener,
            &mut watch_fut,
            (watch_conn_sender.clone(), watch_conn_receiver),
            Arc::clone(&server_state),
        );
        // Both new mails are delivered before IDLE starts; the mock buffers
        // them and glues a single final `* n EXISTS` to the `+ idling`
        // greeting.
        watch_conn_sender
            .unbounded_send(ServerEvent::New(new_mail_a))
            .unwrap();
        watch_conn_sender
            .unbounded_send(ServerEvent::New(new_mail_b))
            .unwrap();
        let watch_conn_loop = watch_conn.loop_handler("watch");
        let watch_loops_handle = std::thread::spawn(move || {
            block_on(watch_conn_loop);
        });

        block_on(async {
            let mut found_a = false;
            let mut found_b = false;
            while !(found_a && found_b) {
                let item = match future::select(
                    watch_fut.as_mut(),
                    smol::Timer::after(Duration::from_secs(30)),
                )
                .await
                {
                    Either::Left(((item, rest), _timeout)) => {
                        watch_fut = Box::pin(rest.into_future());
                        item
                    }
                    Either::Right((_timeout, _pending)) => {
                        panic!(
                            "watch stream did not emit Create events for both new mails within \
                             30 seconds; found_a={found_a} found_b={found_b}"
                        );
                    }
                };
                let Some(backend_event) = item else {
                    panic!("watch stream ended before both Create events");
                };
                match backend_event.unwrap() {
                    BackendEvent::RefreshBatch(events) => {
                        for event in events {
                            if let RefreshEventKind::Create(env) = &event.kind {
                                if env.subject() == "RE: warm NEW mail A" {
                                    found_a = true;
                                }
                                if env.subject() == "RE: warm NEW mail B" {
                                    found_b = true;
                                }
                            }
                        }
                    }
                    BackendEvent::Refresh(event) => {
                        if let RefreshEventKind::Create(env) = &event.kind {
                            if env.subject() == "RE: warm NEW mail A" {
                                found_a = true;
                            }
                            if env.subject() == "RE: warm NEW mail B" {
                                found_b = true;
                            }
                        }
                    }
                    other => {
                        panic!("Expected Refresh event, got: {other:?}");
                    }
                }
            }
        });

        watch_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        main_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        watch_loops_handle.join().unwrap();
        loops_handle.join().unwrap();
    }

    /// Test for the UIDVALIDITY mismatch branch at watch startup: when the
    /// server revalidates the mailbox, the watch stream must emit a `Rescan`
    /// `RefreshEvent`.
    pub(crate) fn run_imap_watch_uidvalidity_change_rescan() {
        let mut _logger = Logger::new_with(LogLevel::TRACE, true);
        let temp_dir = TempDir::new().unwrap();
        let backend_event_queue =
            Arc::new(Mutex::new(std::collections::VecDeque::with_capacity(16)));
        let backend_event_consumer = {
            let backend_event_queue = Arc::clone(&backend_event_queue);

            BackendEventConsumer::new(Arc::new(move |ah, be| {
                eprintln!("BackendEventConsumer: ah {ah:?} be {be:?}");
                backend_event_queue.lock().unwrap().push_back((ah, be));
            }))
        };
        set_test_xdg_env(&temp_dir);

        let seed_mail_1 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:00 +0000
Cc:
Subject: RE: warm seed 1
Message-ID: <warm1@example.com>
Content-Type: text/plain

hello world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let seed_mail_2 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:01 +0000
Cc:
Subject: RE: warm seed 2
Message-ID: <warm2@example.com>
Content-Type: text/plain

hello world 2.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );

        let server_state = Arc::new(Mutex::new(ServerState {
            envelopes: indexmap::indexmap! {},
            next_uid: 1,
            uidvalidity: 1,
            ..Default::default()
        }));
        {
            let mut state_lck = server_state.lock().unwrap();
            state_lck.insert(seed_mail_1);
            state_lck.insert(seed_mail_2);
        }

        let (mut imap, listener, main_conn_sender, loops_handle, inbox_hash, _main_commands) =
            warm_start_setup(
                backend_event_consumer,
                Arc::clone(&server_state),
                600,
                300,
                true,
                true,
            );

        {
            let imap = &mut imap;
            std::thread::scope(|scope| {
                scope
                    .spawn(move || {
                        block_on(async {
                            let seed_envs = fetch_all_envs(imap, inbox_hash).await;
                            assert_eq!(
                                seed_envs.len(),
                                2,
                                "initial fetch must load the two seed mails"
                            );
                        });
                    })
                    .join()
                    .unwrap();
            });
        }

        // The server revalidates the mailbox while meli is not watching.
        server_state.lock().unwrap().uidvalidity = 2;

        let mut watch_fut = Box::pin(imap.watch().unwrap().into_future());
        let (watch_conn_sender, watch_conn_receiver) = unbounded();
        let watch_conn = ImapServerStream::new(
            &listener,
            &mut watch_fut,
            (watch_conn_sender.clone(), watch_conn_receiver),
            Arc::clone(&server_state),
        );
        let watch_conn_loop = watch_conn.loop_handler("watch");
        let watch_loops_handle = std::thread::spawn(move || {
            block_on(watch_conn_loop);
        });

        block_on(async {
            let mut found = false;
            while !found {
                let item = match future::select(
                    watch_fut.as_mut(),
                    smol::Timer::after(Duration::from_secs(30)),
                )
                .await
                {
                    Either::Left(((item, rest), _timeout)) => {
                        watch_fut = Box::pin(rest.into_future());
                        item
                    }
                    Either::Right((_timeout, _pending)) => {
                        panic!(
                            "watch stream did not emit a Rescan event within 30 seconds of the \
                             UIDVALIDITY change"
                        );
                    }
                };
                let Some(backend_event) = item else {
                    panic!("watch stream ended before the Rescan event");
                };
                match backend_event.unwrap() {
                    BackendEvent::RefreshBatch(events) => {
                        found = events
                            .iter()
                            .any(|event| matches!(&event.kind, RefreshEventKind::Rescan));
                    }
                    BackendEvent::Refresh(event) => {
                        found = matches!(&event.kind, RefreshEventKind::Rescan);
                    }
                    other => {
                        panic!("Expected Refresh event, got: {other:?}");
                    }
                }
            }
        });

        watch_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        main_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        watch_loops_handle.join().unwrap();
        loops_handle.join().unwrap();
    }

    /// Test for the silent-DONE server branch: when the server never answers
    /// the IDLE terminator `DONE`, the watch must fail the response read
    /// after the (capped) timeout and surface an error instead of hanging.
    pub(crate) fn run_imap_watch_done_no_response_errors() {
        let mut _logger = Logger::new_with(LogLevel::TRACE, true);
        let temp_dir = TempDir::new().unwrap();
        let backend_event_queue =
            Arc::new(Mutex::new(std::collections::VecDeque::with_capacity(16)));
        let backend_event_consumer = {
            let backend_event_queue = Arc::clone(&backend_event_queue);

            BackendEventConsumer::new(Arc::new(move |ah, be| {
                eprintln!("BackendEventConsumer: ah {ah:?} be {be:?}");
                backend_event_queue.lock().unwrap().push_back((ah, be));
            }))
        };
        set_test_xdg_env(&temp_dir);

        let seed_mail_1 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:00 +0000
Cc:
Subject: RE: warm seed 1
Message-ID: <warm1@example.com>
Content-Type: text/plain

hello world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );

        let server_state = Arc::new(Mutex::new(ServerState {
            envelopes: indexmap::indexmap! {},
            next_uid: 1,
            uidvalidity: 1,
            ignore_done: true,
            ..Default::default()
        }));
        server_state.lock().unwrap().insert(seed_mail_1);

        // Short heartbeat (2s) so the DONE exchange happens quickly; the
        // account socket timeout is 1s in `warm_start_setup`, so the capped
        // DONE wait is 1s.
        let (mut imap, listener, main_conn_sender, loops_handle, inbox_hash, _main_commands) =
            warm_start_setup(
                backend_event_consumer,
                Arc::clone(&server_state),
                2,
                300,
                true,
                true,
            );

        {
            let imap = &mut imap;
            std::thread::scope(|scope| {
                scope
                    .spawn(move || {
                        block_on(async {
                            let seed_envs = fetch_all_envs(imap, inbox_hash).await;
                            assert_eq!(seed_envs.len(), 1, "initial fetch must load the seed mail");
                        });
                    })
                    .join()
                    .unwrap();
            });
        }

        let mut watch_fut = Box::pin(imap.watch().unwrap().into_future());
        let (watch_conn_sender, watch_conn_receiver) = unbounded();
        let watch_conn = ImapServerStream::new(
            &listener,
            &mut watch_fut,
            (watch_conn_sender.clone(), watch_conn_receiver),
            Arc::clone(&server_state),
        );
        let watch_conn_loop = watch_conn.loop_handler("watch");
        let watch_loops_handle = std::thread::spawn(move || {
            block_on(watch_conn_loop);
        });

        block_on(async {
            loop {
                let item = match future::select(
                    watch_fut.as_mut(),
                    smol::Timer::after(Duration::from_secs(30)),
                )
                .await
                {
                    Either::Left(((item, rest), _timeout)) => {
                        watch_fut = Box::pin(rest.into_future());
                        item
                    }
                    Either::Right((_timeout, _pending)) => {
                        panic!(
                            "watch stream produced no error within 30 seconds of the silent \
                             DONE; it is likely hanging on the response read"
                        );
                    }
                };
                let Some(backend_event) = item else {
                    panic!("watch stream ended without surfacing the DONE timeout error");
                };
                match backend_event {
                    Err(err) => {
                        assert!(
                            err.kind.is_timeout(),
                            "expected a timeout error from the silent DONE, got: {err}"
                        );
                        break;
                    }
                    Ok(_) => {
                        // Startup events (e.g. Rescan/Create) may legitimately
                        // arrive first; keep draining until the error.
                    }
                }
            }
        });

        watch_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        main_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        watch_loops_handle.join().unwrap();
        loops_handle.join().unwrap();
    }

    /// The standard three mails of the gated-push watch tests: two seeds
    /// present before meli starts, and one new mail delivered while the
    /// watch is idling.
    fn gated_push_test_mails() -> (Box<Mail>, Box<Mail>, Box<Mail>) {
        let seed_mail_1 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:00 +0000
Cc:
Subject: RE: gated seed 1
Message-ID: <gated-seed-1@example.com>
Content-Type: text/plain

hello world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let seed_mail_2 = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:01 +0000
Cc:
Subject: RE: gated seed 2
Message-ID: <gated-seed-2@example.com>
Content-Type: text/plain

hello world 2.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let new_mail = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Date: Thu, 01 Jan 1970 00:00:02 +0000
Cc:
Subject: RE: gated NEW mail
Message-ID: <gated-new@example.com>
Content-Type: text/plain

hello new world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        (seed_mail_1, seed_mail_2, new_mail)
    }

    /// Configuration-path proof of the `watch_sweep_interval` seam: with
    /// no `watch_sweep_interval` key the parsed interval must be the
    /// historical 300 seconds; a small injected value must reach the
    /// parsed conf; zero must be rejected as a configuration error.
    /// (`ImapType::new` performs no I/O, so no server is needed.)
    pub(crate) fn run_imap_watch_sweep_interval_conf_default() {
        let temp_dir = TempDir::new().unwrap();
        set_test_xdg_env(&temp_dir);
        // Bind a throwaway listener only to obtain a port; nothing ever
        // connects during `ImapType::new`.
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let local_addr = listener.local_addr().unwrap();
        drop(listener);
        let make_account_conf = |watch_sweep_interval: Option<u64>| AccountSettings {
            name: "test".to_string(),
            root_mailbox: "INBOX".to_string(),
            format: "imap".to_string(),
            identity: "user@example.com".to_string(),
            extra_identities: vec![],
            read_only: false,
            display_name: None,
            subscribed_mailboxes: vec![],
            mailboxes: indexmap::indexmap! {},
            manual_refresh: false,
            extra: {
                let mut extra = indexmap::indexmap! {
                    "server_hostname".to_string() => local_addr.ip().to_string(),
                    "server_username".to_string() => "user".to_string(),
                    "server_password".to_string() => "password".to_string(),
                    "server_port".to_string() => local_addr.port().to_string(),
                    "use_starttls".to_string() => "false".to_string(),
                    "use_tls".to_string() => "false".to_string(),
                    "use_connection_pool".to_string() => "false".to_string(),
                    "timeout".to_string() => 1_u64.to_string(),
                };
                if let Some(secs) = watch_sweep_interval {
                    extra.insert("watch_sweep_interval".to_string(), secs.to_string());
                }
                extra
            },
        };

        // No key: the default must preserve the historical fixed
        // 5-minute sweep cadence exactly.
        let imap = ImapType::new(
            &make_account_conf(None),
            Default::default(),
            Default::default(),
        )
        .unwrap();
        assert_eq!(
            imap.server_conf.watch_sweep_interval,
            Duration::from_secs(300),
            "without the `watch_sweep_interval` key the parsed interval must default to \
             300s (the pre-seam fixed cadence)"
        );
        // The heartbeat default must give good out-of-the-box latency on
        // servers that never push during IDLE (one re-sync round per
        // minute); pinned here so it cannot silently regress to 600s.
        assert_eq!(
            imap.server_conf.idle_heartbeat_interval,
            Duration::from_secs(60),
            "without the idle_heartbeat_interval key the default must be 60s"
        );
        drop(imap);
        // A small injected value must reach the parsed conf (tests force
        // frequent sweeps with it).
        let imap = ImapType::new(
            &make_account_conf(Some(1)),
            Default::default(),
            Default::default(),
        )
        .unwrap();
        assert_eq!(
            imap.server_conf.watch_sweep_interval,
            Duration::from_secs(1),
            "a small `watch_sweep_interval` value must override the default"
        );
        drop(imap);

        // Zero is a configuration error, like `idle_heartbeat_interval`.
        assert!(
            ImapType::new(
                &make_account_conf(Some(0)),
                Default::default(),
                Default::default(),
            )
            .is_err(),
            "watch_sweep_interval = 0 must be rejected"
        );
    }

    /// Behavioral proof of the `watch_sweep_interval` seam: with a small
    /// injected interval (1s) and a short heartbeat (2s), the periodic
    /// sweep must run within seconds and re-check a mailbox OTHER than
    /// the watched one on the main connection (a post-snapshot
    /// `EXAMINE` of the extra `Archives` mailbox in the main
    /// connection's received command log), while the watched mailbox
    /// itself must NEVER be re-selected on the main connection (the
    /// single-session invariant: it is persistently selected only on
    /// the watch connection, which covers it via IDLE plus the
    /// heartbeat compensation re-sync). With the default 300s interval
    /// the sweep cannot fire inside any bounded test window (see
    /// `run_imap_watch_sweep_interval_conf_default`), which is what
    /// keeps the pre-existing watch tests free of sweeps.
    pub(crate) fn run_imap_watch_sweep_interval_short_sweep_fires() {
        let temp_dir = TempDir::new().unwrap();
        set_test_xdg_env(&temp_dir);
        let (seed_mail_1, seed_mail_2, new_mail) = gated_push_test_mails();

        let server_state = Arc::new(Mutex::new(ServerState {
            envelopes: indexmap::indexmap! {},
            next_uid: 1,
            uidvalidity: 1,
            extra_mailbox: Some("Archives".to_string()),
            ..Default::default()
        }));
        {
            let mut state_lck = server_state.lock().unwrap();
            state_lck.insert(seed_mail_1);
            state_lck.insert(seed_mail_2);
        }

        // Short heartbeat (2s) so IDLE wake-ups produce sweep-triggering
        // lines quickly; watch_sweep_interval = 1s forces the sweep.
        let (mut imap, listener, main_conn_sender, loops_handle, inbox_hash, main_commands) =
            warm_start_setup(
                Default::default(),
                Arc::clone(&server_state),
                2,
                1,
                true,
                false,
            );

        {
            let imap = &mut imap;
            std::thread::scope(|scope| {
                scope
                    .spawn(move || {
                        block_on(async {
                            let seed_envs = fetch_all_envs(imap, inbox_hash).await;
                            assert_eq!(
                                seed_envs.len(),
                                2,
                                "initial fetch must load the two seed mails"
                            );
                        });
                    })
                    .join()
                    .unwrap();
            });
        }

        // Everything the main connection did during the warm-start fetch
        // is before this snapshot.
        let snapshot_len = main_commands.lock().unwrap().len();

        let mut watch_fut = Box::pin(imap.watch().unwrap().into_future());
        let (watch_conn_sender, watch_conn_receiver) = unbounded();
        let watch_conn = ImapServerStream::new(
            &listener,
            &mut watch_fut,
            (watch_conn_sender.clone(), watch_conn_receiver),
            Arc::clone(&server_state),
        );
        // New mail is delivered while meli is running; no suppression
        // flags are set, so the push is delivered normally.
        watch_conn_sender
            .unbounded_send(ServerEvent::New(new_mail))
            .unwrap();
        let watch_conn_loop = watch_conn.loop_handler("watch");
        let watch_loops_handle = std::thread::spawn(move || {
            block_on(watch_conn_loop);
        });

        // The push path delivers the new mail first.
        block_on(async {
            let mut found = false;
            while !found {
                let item = match future::select(
                    watch_fut.as_mut(),
                    smol::Timer::after(WATCH_TEST_DEADLINE),
                )
                .await
                {
                    Either::Left(((item, rest), _timeout)) => {
                        watch_fut = Box::pin(rest.into_future());
                        item
                    }
                    Either::Right((_timeout, _pending)) => {
                        panic!(
                            "watch stream did not emit the new mail's Create event within \
                             {WATCH_TEST_DEADLINE:?}"
                        );
                    }
                };
                let Some(backend_event) = item else {
                    panic!("watch stream ended before the new mail's Create event");
                };
                match backend_event.unwrap() {
                    BackendEvent::RefreshBatch(events) => {
                        found = events.iter().any(|event| {
                            matches!(
                                &event.kind,
                                RefreshEventKind::Create(env)
                                    if env.subject() == "RE: gated NEW mail"
                            )
                        });
                    }
                    BackendEvent::Refresh(event) => {
                        found = matches!(
                            &event.kind,
                            RefreshEventKind::Create(env)
                                if env.subject() == "RE: gated NEW mail"
                        );
                    }
                    other => {
                        panic!("Expected Refresh event, got: {other:?}");
                    }
                }
            }
        });

        // The sweep fires on the first IDLE line after the interval
        // elapses (heartbeat wake-ups produce such lines). It must
        // examine the OTHER mailbox (`Archives`) on the main connection
        // and never the watched one. Keep driving the watch stream
        // while waiting — the client only progresses while its stream is
        // polled. Bounded by `WATCH_TEST_DEADLINE`; the poll tick only
        // affects how soon the condition is noticed.
        block_on(async {
            let deadline = std::time::Instant::now() + WATCH_TEST_DEADLINE;
            loop {
                let archives_examined = {
                    let lck = main_commands.lock().unwrap();
                    lck[snapshot_len..]
                        .iter()
                        .any(|l| l.contains("EXAMINE Archives"))
                };
                if archives_examined {
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    panic!(
                        "the forced sweep did not re-check the Archives mailbox on the \
                         main connection within {WATCH_TEST_DEADLINE:?}; post-snapshot \
                         main log: {:?}",
                        main_commands.lock().unwrap()[snapshot_len..].to_vec()
                    );
                }
                match future::select(watch_fut.as_mut(), smol::Timer::after(WATCH_TEST_POLL_TICK))
                    .await
                {
                    Either::Left(((item, rest), _tick)) => {
                        watch_fut = Box::pin(rest.into_future());
                        // Drain further events quietly; the new mail's
                        // Create was already asserted above.
                        if let Some(ev) = item {
                            let _ = ev.unwrap();
                        }
                    }
                    Either::Right((_tick, _pending)) => {}
                }
            }
            // The new invariant: the watched mailbox is never re-selected
            // on the main connection after the snapshot (it is covered by
            // the watch connection's IDLE + heartbeat compensation).
            let offenders: Vec<String> = {
                let lck = main_commands.lock().unwrap();
                lck[snapshot_len..]
                    .iter()
                    .filter(|l| l.contains("SELECT INBOX") || l.contains("EXAMINE INBOX"))
                    .cloned()
                    .collect()
            };
            assert!(
                offenders.is_empty(),
                "the watch sweep must skip the watched mailbox on the main connection; \
                 post-snapshot main log: {:?}",
                main_commands.lock().unwrap()[snapshot_len..].to_vec()
            );
        });

        watch_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        main_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        watch_loops_handle.join().unwrap();
        loops_handle.join().unwrap();
    }

    /// Regression test pinning the multi-session push suppression fix
    /// (`suppress_push_when_multi_selected` mock mode) end-to-end:
    /// while more than one session holds INBOX selected, the mock
    /// suppresses new-mail `* EXISTS` pushes to the IDLE session. The
    /// single-session invariant (T6) removes that state: at watch
    /// startup the main connection UNSELECTs the watched mailbox (it
    /// SELECTed it during the warm-start fetch, before the snapshot
    /// below), so exactly one session — the watch connection — holds
    /// the mailbox for the rest of the run, and the periodic watch
    /// sweep must never re-select the watched mailbox on the main
    /// connection.
    ///
    /// Assertions, in order:
    /// 1. the startup UNSELECT appears on the main connection after the
    ///    snapshot;
    /// 2. the new mail's Create event arrives — via the now-delivered
    ///    push (with the invariant enforced the mock no longer
    ///    suppresses it), with the watch heartbeat re-sync as the
    ///    fallback (see `run_imap_watch_startup_unselect` and
    ///    `run_imap_watch_idle_no_push`);
    /// 3. at least two heartbeat re-sync cycles ran on the watch
    ///    connection, so the sweep trigger line of the first cycle is
    ///    long past;
    /// 4. the push WAS delivered (exactly the point of the fix) and
    ///    exactly one session holds INBOX selected;
    /// 5. THE pre-T6 pin (archived RED in
    ///    `.omo/evidence/fix-imap-idle-push/task-4-gated-mocks.txt`):
    ///    no post-snapshot `SELECT INBOX`/`EXAMINE INBOX` by the main
    ///    connection.
    ///
    /// Note on assertion 4's history: pre-T6 this asserted
    /// `idle_exists_pushes == 0` under the old world where the main
    /// connection kept its warm-start selection (two sessions) and the
    /// suppressed push had to be caught by the heartbeat re-sync. With
    /// the startup UNSELECT the suppression premise is gone by design —
    /// the server pushing again to the single remaining session is the
    /// product behavior this fix exists to restore — so the assertion
    /// now pins that the push flows. The heartbeat-fallback coverage
    /// remains in `run_imap_watch_idle_no_push` (a server that never
    /// pushes) and `run_imap_watch_startup_compensation`.
    pub(crate) fn run_imap_watch_multi_session_no_push() {
        let temp_dir = TempDir::new().unwrap();
        set_test_xdg_env(&temp_dir);
        let (seed_mail_1, seed_mail_2, new_mail) = gated_push_test_mails();

        let server_state = Arc::new(Mutex::new(ServerState {
            envelopes: indexmap::indexmap! {},
            next_uid: 1,
            uidvalidity: 1,
            suppress_push_when_multi_selected: true,
            ..Default::default()
        }));
        {
            let mut state_lck = server_state.lock().unwrap();
            state_lck.insert(seed_mail_1);
            state_lck.insert(seed_mail_2);
        }

        // Short heartbeat (2s) so IDLE wake-ups produce sweep-triggering
        // lines quickly; watch_sweep_interval = 1s forces the sweep that
        // must NOT re-check the watched mailbox on the main connection.
        let (mut imap, listener, main_conn_sender, loops_handle, inbox_hash, main_commands) =
            warm_start_setup(
                Default::default(),
                Arc::clone(&server_state),
                2,
                1,
                true,
                false,
            );

        {
            let imap = &mut imap;
            std::thread::scope(|scope| {
                scope
                    .spawn(move || {
                        block_on(async {
                            let seed_envs = fetch_all_envs(imap, inbox_hash).await;
                            assert_eq!(
                                seed_envs.len(),
                                2,
                                "initial fetch must load the two seed mails"
                            );
                        });
                    })
                    .join()
                    .unwrap();
            });
        }

        // SNAPSHOT BOUNDARY: the warm-start resync's transient SELECT of
        // INBOX on the main connection happened during the initial fetch
        // above, before this length snapshot; it must not count as a
        // violation.
        let snapshot_len = main_commands.lock().unwrap().len();
        assert!(
            main_commands
                .lock()
                .unwrap()
                .iter()
                .any(|l| l.contains("SELECT INBOX")),
            "warm start must have SELECTed INBOX on the main connection before the snapshot"
        );
        assert_eq!(
            server_state.lock().unwrap().selected_session_count("INBOX"),
            1,
            "after the warm start exactly the main connection may hold INBOX selected"
        );

        let mut watch_fut = Box::pin(imap.watch().unwrap().into_future());
        let (watch_conn_sender, watch_conn_receiver) = unbounded();
        let watch_conn = ImapServerStream::new(
            &listener,
            &mut watch_fut,
            (watch_conn_sender.clone(), watch_conn_receiver),
            Arc::clone(&server_state),
        );
        let watch_commands = Arc::clone(&watch_conn.received_commands);
        let watch_pushes = Arc::clone(&watch_conn.idle_exists_pushes);
        let watch_conn_loop = watch_conn.loop_handler("watch");
        let watch_loops_handle = std::thread::spawn(move || {
            block_on(watch_conn_loop);
        });
        // The watch startup UNSELECTs INBOX on the main connection and
        // EXAMINEs it on the watch connection, so exactly ONE session
        // holds it selected: the mock's suppression mode (which only
        // engages with more than one session) must NOT suppress the
        // `* EXISTS` push to the watch connection — that is the whole
        // point of the single-session invariant.
        watch_conn_sender
            .unbounded_send(ServerEvent::New(new_mail))
            .unwrap();

        // Assertion (1): the startup UNSELECT happened on the main
        // connection after the snapshot (the bounded wait below drives
        // the client, and the UNSELECT strictly precedes the watch
        // connection's EXAMINE, so by the time any Create or heartbeat
        // cycle is observed it has long been sent).
        block_on(async {
            let deadline = std::time::Instant::now() + WATCH_TEST_DEADLINE;
            loop {
                if main_commands.lock().unwrap()[snapshot_len..]
                    .iter()
                    .any(|l| l.contains("UNSELECT"))
                {
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    panic!(
                        "the watch startup did not UNSELECT the watched mailbox on the \
                         main connection within {WATCH_TEST_DEADLINE:?}; post-snapshot main \
                         log: {:?}",
                        main_commands.lock().unwrap()[snapshot_len..].to_vec()
                    );
                }
                match future::select(watch_fut.as_mut(), smol::Timer::after(WATCH_TEST_POLL_TICK))
                    .await
                {
                    Either::Left(((item, rest), _tick)) => {
                        watch_fut = Box::pin(rest.into_future());
                        if let Some(ev) = item {
                            let _ = ev.unwrap();
                        }
                    }
                    Either::Right((_tick, _pending)) => {}
                }
            }
        });

        // Assertion (2): the new mail must still arrive as a Create event
        // — with the invariant enforced the push is delivered, and even
        // if it were not, the heartbeat re-sync of the watch connection
        // would notice it. Keep driving the watch stream while waiting
        // (the client only progresses while its stream is polled);
        // bounded by `WATCH_TEST_DEADLINE`, the poll tick only affects
        // how soon a satisfied condition is noticed.
        block_on(async {
            let deadline = std::time::Instant::now() + WATCH_TEST_DEADLINE;
            let mut found_create = false;
            while !found_create {
                if std::time::Instant::now() >= deadline {
                    panic!(
                        "watch stream did not emit the new mail's Create event within \
                         {WATCH_TEST_DEADLINE:?} even though the push path (or, failing that, \
                         the heartbeat re-sync) should have caught it"
                    );
                }
                let item = match future::select(
                    watch_fut.as_mut(),
                    smol::Timer::after(WATCH_TEST_POLL_TICK),
                )
                .await
                {
                    Either::Left(((item, rest), _tick)) => {
                        watch_fut = Box::pin(rest.into_future());
                        item
                    }
                    Either::Right((_tick, _pending)) => {
                        continue;
                    }
                };
                let Some(backend_event) = item else {
                    panic!("watch stream ended before the new mail's Create event");
                };
                match backend_event.unwrap() {
                    BackendEvent::RefreshBatch(events) => {
                        found_create = events.iter().any(|event| {
                            matches!(
                                &event.kind,
                                RefreshEventKind::Create(env)
                                    if env.subject() == "RE: gated NEW mail"
                            )
                        });
                    }
                    BackendEvent::Refresh(event) => {
                        found_create = matches!(
                            &event.kind,
                            RefreshEventKind::Create(env)
                                if env.subject() == "RE: gated NEW mail"
                        );
                    }
                    other => {
                        panic!("Expected Refresh event, got: {other:?}");
                    }
                }
            }
            // Bounded: wait until at least two heartbeat re-sync cycles
            // ran on the watch connection (without an offline cache each
            // heartbeat wake-up EXAMINEs the mailbox again, so two cycles
            // means three EXAMINEs including the startup one). The
            // sweep-triggering line of the first cycle is then long past:
            // if the sweep were going to touch the main connection, it
            // already did. Keep driving the watch stream while waiting.
            loop {
                if watch_commands
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|l| l.contains("EXAMINE INBOX"))
                    .count()
                    >= 3
                {
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    panic!(
                        "two heartbeat resync cycles did not run on the watch connection \
                         within {WATCH_TEST_DEADLINE:?}; watch command log: {:?}",
                        watch_commands.lock().unwrap()
                    );
                }
                match future::select(watch_fut.as_mut(), smol::Timer::after(WATCH_TEST_POLL_TICK))
                    .await
                {
                    Either::Left(((item, rest), _tick)) => {
                        watch_fut = Box::pin(rest.into_future());
                        if let Some(ev) = item {
                            let _ = ev.unwrap();
                        }
                    }
                    Either::Right((_tick, _pending)) => {}
                }
            }
        });

        // Assertion (4): the push WAS delivered — with the invariant
        // enforced exactly one session holds INBOX selected, so the
        // mock's multi-session suppression mode must NOT engage — and
        // that single session is the watch connection. Both facts are
        // deterministic here: the startup UNSELECT completed before the
        // watch connection's EXAMINE (they are sequential in the
        // client's startup), which completed before any push or
        // heartbeat cycle observed above.
        assert!(
            watch_pushes.load(std::sync::atomic::Ordering::SeqCst) >= 1,
            "with the single-session invariant enforced (the main connection UNSELECTed \
             INBOX at watch startup), the server must push `* EXISTS` to the watch \
             connection again; got {} pushes",
            watch_pushes.load(std::sync::atomic::Ordering::SeqCst)
        );
        assert_eq!(
            server_state.lock().unwrap().selected_session_count("INBOX"),
            1,
            "after the startup UNSELECT exactly the watch connection may hold INBOX \
             selected"
        );

        // Assertion (5), THE pre-T6 pinned failure: no post-snapshot
        // SELECT or EXAMINE of the watched mailbox by any connection
        // other than the watch connection. On pre-T6 code the forced
        // sweep violated exactly this (see the archived RED output in
        // `.omo/evidence/fix-imap-idle-push/task-4-gated-mocks.txt`).
        let offenders: Vec<String> = {
            let lck = main_commands.lock().unwrap();
            lck[snapshot_len..]
                .iter()
                .filter(|l| l.contains("SELECT INBOX") || l.contains("EXAMINE INBOX"))
                .cloned()
                .collect()
        };
        assert!(
            offenders.is_empty(),
            "the watch sweep must never re-select the watched mailbox on the MAIN \
             connection after the snapshot (commands {snapshot_len}..): {offenders:?}. The \
             watched mailbox is covered by the watch connection's IDLE plus the heartbeat \
             compensation re-sync; full post-snapshot main log: {:?}",
            main_commands.lock().unwrap()[snapshot_len..].to_vec()
        );

        watch_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        main_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        watch_loops_handle.join().unwrap();
        loops_handle.join().unwrap();
    }

    /// Failing-first pin for the single-session invariant's startup half
    /// (T6): the warm-start initial fetch leaves the watched mailbox
    /// selected on the main connection. When the watch starts, it must
    /// UNSELECT the watched mailbox on the main connection (the mock
    /// advertises `UNSELECT`) BEFORE the watch connection selects it,
    /// so that from then on exactly one session — the IDLE watch
    /// connection — holds the mailbox selected. Ground truth: the main
    /// connection's received-command log (the `UNSELECT` command) and
    /// the mock's per-mailbox `selected_session_count`.
    pub(crate) fn run_imap_watch_startup_unselect() {
        let temp_dir = TempDir::new().unwrap();
        set_test_xdg_env(&temp_dir);
        let (seed_mail_1, seed_mail_2, _new_mail) = gated_push_test_mails();

        let server_state = Arc::new(Mutex::new(ServerState {
            envelopes: indexmap::indexmap! {},
            next_uid: 1,
            uidvalidity: 1,
            ..Default::default()
        }));
        {
            let mut state_lck = server_state.lock().unwrap();
            state_lck.insert(seed_mail_1);
            state_lck.insert(seed_mail_2);
        }

        // Long heartbeat and default sweep interval: within the bounded
        // observation window only the watch startup runs, which is the
        // code path under test.
        let (mut imap, listener, main_conn_sender, loops_handle, inbox_hash, main_commands) =
            warm_start_setup(
                Default::default(),
                Arc::clone(&server_state),
                600,
                300,
                true,
                true,
            );

        {
            let imap = &mut imap;
            std::thread::scope(|scope| {
                scope
                    .spawn(move || {
                        block_on(async {
                            let seed_envs = fetch_all_envs(imap, inbox_hash).await;
                            assert_eq!(
                                seed_envs.len(),
                                2,
                                "initial fetch must load the two seed mails"
                            );
                        });
                    })
                    .join()
                    .unwrap();
            });
        }

        // Precondition: the warm start left the main connection holding
        // INBOX selected (the transient selection the invariant must
        // clear at watch startup).
        assert!(
            main_commands
                .lock()
                .unwrap()
                .iter()
                .any(|l| l.contains("SELECT INBOX")),
            "warm start must have selected INBOX on the main connection"
        );
        assert_eq!(
            server_state.lock().unwrap().selected_session_count("INBOX"),
            1,
            "before the watch starts, exactly the main connection holds INBOX selected"
        );
        let snapshot_len = main_commands.lock().unwrap().len();

        let mut watch_fut = Box::pin(imap.watch().unwrap().into_future());
        let (watch_conn_sender, watch_conn_receiver) = unbounded();
        let watch_conn = ImapServerStream::new(
            &listener,
            &mut watch_fut,
            (watch_conn_sender.clone(), watch_conn_receiver),
            Arc::clone(&server_state),
        );
        let watch_commands = Arc::clone(&watch_conn.received_commands);
        let watch_conn_loop = watch_conn.loop_handler("watch");
        let watch_loops_handle = std::thread::spawn(move || {
            block_on(watch_conn_loop);
        });

        block_on(async {
            let deadline = std::time::Instant::now() + WATCH_TEST_DEADLINE;
            // First wait until the watch connection has EXAMINEd INBOX:
            // the startup UNSELECT on the main connection happens strictly
            // before that in the watch's startup sequence, so once this is
            // observed the UNSELECT has already been sent.
            loop {
                if watch_commands
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|l| l.contains("EXAMINE INBOX"))
                {
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    panic!(
                        "watch connection did not EXAMINE INBOX within \
                         {WATCH_TEST_DEADLINE:?}; watch command log: {:?}",
                        watch_commands.lock().unwrap()
                    );
                }
                match future::select(watch_fut.as_mut(), smol::Timer::after(WATCH_TEST_POLL_TICK))
                    .await
                {
                    Either::Left(((item, rest), _tick)) => {
                        watch_fut = Box::pin(rest.into_future());
                        if let Some(ev) = item {
                            let _ = ev.unwrap();
                        }
                    }
                    Either::Right((_tick, _pending)) => {}
                }
            }
            // THE pin: the main connection must have UNSELECTed the
            // watched mailbox at watch startup.
            loop {
                if main_commands.lock().unwrap()[snapshot_len..]
                    .iter()
                    .any(|l| l.contains("UNSELECT"))
                {
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    panic!(
                        "FAILING-FIRST (pre-T6 pin): the main connection still holds the \
                         watched mailbox selected after watch startup; the watch must \
                         UNSELECT it on the main connection before the watch connection \
                         selects it. Post-snapshot main log: {:?}",
                        main_commands.lock().unwrap()[snapshot_len..].to_vec()
                    );
                }
                match future::select(watch_fut.as_mut(), smol::Timer::after(WATCH_TEST_POLL_TICK))
                    .await
                {
                    Either::Left(((item, rest), _tick)) => {
                        watch_fut = Box::pin(rest.into_future());
                        if let Some(ev) = item {
                            let _ = ev.unwrap();
                        }
                    }
                    Either::Right((_tick, _pending)) => {}
                }
            }
            // Terminal state of the invariant: exactly one session (the
            // watch connection) holds INBOX selected. The mock updates
            // `selected_sessions` while handling the commands, so the
            // bounded wait lets the (already observed) UNSELECT's
            // decrement settle; with heartbeat 600s and sweep 300s this
            // count cannot change again inside the window.
            loop {
                if server_state.lock().unwrap().selected_session_count("INBOX") == 1 {
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    panic!(
                        "exactly the watch connection must hold INBOX selected after the \
                         startup UNSELECT, but {} sessions do; main log: {:?}",
                        server_state.lock().unwrap().selected_session_count("INBOX"),
                        main_commands.lock().unwrap()[snapshot_len..].to_vec()
                    );
                }
                match future::select(watch_fut.as_mut(), smol::Timer::after(WATCH_TEST_POLL_TICK))
                    .await
                {
                    Either::Left(((item, rest), _tick)) => {
                        watch_fut = Box::pin(rest.into_future());
                        if let Some(ev) = item {
                            let _ = ev.unwrap();
                        }
                    }
                    Either::Right((_tick, _pending)) => {}
                }
            }
        });

        watch_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        main_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        watch_loops_handle.join().unwrap();
        loops_handle.join().unwrap();
    }
    ///
    /// Test for the `id_gated_push` mock mode: a server that only pushes
    /// new-mail `* EXISTS` during IDLE to sessions that identified
    /// themselves with the RFC 2971 `ID` command. With `use_id = false`
    /// the client never sends `ID` and no push is delivered; the new mail
    /// is still noticed via the watch heartbeat re-sync. With the default
    /// `use_id = true` the client sends `ID` during the connection
    /// handshake and the push is delivered normally.
    ///
    /// The `ID` command is exchanged during the connection handshake
    /// (stage `M4`), before the loop handler starts logging
    /// `received_commands`, so the assertions use the handshake-observed
    /// `sent_id` flag of the connection instead of the command log.
    pub(crate) fn run_imap_watch_id_gated_push() {
        // Phase 1: `use_id = false` — no ID, no push.
        {
            let temp_dir = TempDir::new().unwrap();
            set_test_xdg_env(&temp_dir);
            let (seed_mail_1, seed_mail_2, new_mail) = gated_push_test_mails();
            let server_state = Arc::new(Mutex::new(ServerState {
                envelopes: indexmap::indexmap! {},
                next_uid: 1,
                uidvalidity: 1,
                id_gated_push: true,
                ..Default::default()
            }));
            {
                let mut state_lck = server_state.lock().unwrap();
                state_lck.insert(seed_mail_1);
                state_lck.insert(seed_mail_2);
            }
            let (mut imap, listener, main_conn_sender, loops_handle, inbox_hash, _main_commands) =
                warm_start_setup(
                    Default::default(),
                    Arc::clone(&server_state),
                    2,
                    300,
                    false,
                    true,
                );
            {
                let imap = &mut imap;
                std::thread::scope(|scope| {
                    scope
                        .spawn(move || {
                            block_on(async {
                                let seed_envs = fetch_all_envs(imap, inbox_hash).await;
                                assert_eq!(
                                    seed_envs.len(),
                                    2,
                                    "initial fetch must load the two seed mails"
                                );
                            });
                        })
                        .join()
                        .unwrap();
                });
            }

            let mut watch_fut = Box::pin(imap.watch().unwrap().into_future());
            let (watch_conn_sender, watch_conn_receiver) = unbounded();
            let watch_conn = ImapServerStream::new(
                &listener,
                &mut watch_fut,
                (watch_conn_sender.clone(), watch_conn_receiver),
                Arc::clone(&server_state),
            );
            assert!(
                !watch_conn.sent_id,
                "with use_id = false the client must not send ID during the handshake"
            );
            let watch_pushes = Arc::clone(&watch_conn.idle_exists_pushes);
            let watch_conn_loop = watch_conn.loop_handler("watch");
            let watch_loops_handle = std::thread::spawn(move || {
                block_on(watch_conn_loop);
            });
            // New mail arrives while idling; the server records it but
            // (id_gated_push) never pushes `* EXISTS` to this connection.
            watch_conn_sender
                .unbounded_send(ServerEvent::New(new_mail))
                .unwrap();

            // The mail is still noticed by the heartbeat re-sync of the
            // watch connection (bounded wait).
            block_on(async {
                let mut found = false;
                while !found {
                    let item = match future::select(
                        watch_fut.as_mut(),
                        smol::Timer::after(WATCH_TEST_DEADLINE),
                    )
                    .await
                    {
                        Either::Left(((item, rest), _timeout)) => {
                            watch_fut = Box::pin(rest.into_future());
                            item
                        }
                        Either::Right((_timeout, _pending)) => {
                            panic!(
                                "watch stream did not emit the new mail's Create event within \
                                 {WATCH_TEST_DEADLINE:?} even though the heartbeat re-sync \
                                 should have caught it without any push"
                            );
                        }
                    };
                    let Some(backend_event) = item else {
                        panic!("watch stream ended before the new mail's Create event");
                    };
                    match backend_event.unwrap() {
                        BackendEvent::RefreshBatch(events) => {
                            found = events.iter().any(|event| {
                                matches!(
                                    &event.kind,
                                    RefreshEventKind::Create(env)
                                        if env.subject() == "RE: gated NEW mail"
                                )
                            });
                        }
                        BackendEvent::Refresh(event) => {
                            found = matches!(
                                &event.kind,
                                RefreshEventKind::Create(env)
                                    if env.subject() == "RE: gated NEW mail"
                            );
                        }
                        other => {
                            panic!("Expected Refresh event, got: {other:?}");
                        }
                    }
                }
            });
            assert_eq!(
                watch_pushes.load(std::sync::atomic::Ordering::SeqCst),
                0,
                "id_gated_push must suppress the EXISTS push for a session that never sent ID"
            );

            watch_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
            main_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
            watch_loops_handle.join().unwrap();
            loops_handle.join().unwrap();
            drop(imap);
        }

        // Phase 2: `use_id = true` (the default) — ID is sent and the
        // push is delivered.
        {
            let temp_dir = TempDir::new().unwrap();
            set_test_xdg_env(&temp_dir);
            let (seed_mail_1, seed_mail_2, new_mail) = gated_push_test_mails();
            let server_state = Arc::new(Mutex::new(ServerState {
                envelopes: indexmap::indexmap! {},
                next_uid: 1,
                uidvalidity: 1,
                id_gated_push: true,
                ..Default::default()
            }));
            {
                let mut state_lck = server_state.lock().unwrap();
                state_lck.insert(seed_mail_1);
                state_lck.insert(seed_mail_2);
            }
            let (mut imap, listener, main_conn_sender, loops_handle, inbox_hash, _main_commands) =
                warm_start_setup(
                    Default::default(),
                    Arc::clone(&server_state),
                    600,
                    300,
                    true,
                    true,
                );
            {
                let imap = &mut imap;
                std::thread::scope(|scope| {
                    scope
                        .spawn(move || {
                            block_on(async {
                                let seed_envs = fetch_all_envs(imap, inbox_hash).await;
                                assert_eq!(
                                    seed_envs.len(),
                                    2,
                                    "initial fetch must load the two seed mails"
                                );
                            });
                        })
                        .join()
                        .unwrap();
                });
            }

            let mut watch_fut = Box::pin(imap.watch().unwrap().into_future());
            let (watch_conn_sender, watch_conn_receiver) = unbounded();
            let watch_conn = ImapServerStream::new(
                &listener,
                &mut watch_fut,
                (watch_conn_sender.clone(), watch_conn_receiver),
                Arc::clone(&server_state),
            );
            assert!(
                watch_conn.sent_id,
                "with use_id = true (the default) the client must send ID during the handshake"
            );
            let watch_pushes = Arc::clone(&watch_conn.idle_exists_pushes);
            let watch_conn_loop = watch_conn.loop_handler("watch");
            let watch_loops_handle = std::thread::spawn(move || {
                block_on(watch_conn_loop);
            });
            watch_conn_sender
                .unbounded_send(ServerEvent::New(new_mail))
                .unwrap();

            // The push path delivers the mail (bounded wait).
            block_on(async {
                let mut found = false;
                while !found {
                    let item = match future::select(
                        watch_fut.as_mut(),
                        smol::Timer::after(WATCH_TEST_DEADLINE),
                    )
                    .await
                    {
                        Either::Left(((item, rest), _timeout)) => {
                            watch_fut = Box::pin(rest.into_future());
                            item
                        }
                        Either::Right((_timeout, _pending)) => {
                            panic!(
                                "watch stream did not emit the new mail's Create event within \
                                 {WATCH_TEST_DEADLINE:?} even though the ID-gated push should \
                                 have been delivered"
                            );
                        }
                    };
                    let Some(backend_event) = item else {
                        panic!("watch stream ended before the new mail's Create event");
                    };
                    match backend_event.unwrap() {
                        BackendEvent::RefreshBatch(events) => {
                            found = events.iter().any(|event| {
                                matches!(
                                    &event.kind,
                                    RefreshEventKind::Create(env)
                                        if env.subject() == "RE: gated NEW mail"
                                )
                            });
                        }
                        BackendEvent::Refresh(event) => {
                            found = matches!(
                                &event.kind,
                                RefreshEventKind::Create(env)
                                    if env.subject() == "RE: gated NEW mail"
                            );
                        }
                        other => {
                            panic!("Expected Refresh event, got: {other:?}");
                        }
                    }
                }
            });
            assert!(
                watch_pushes.load(std::sync::atomic::Ordering::SeqCst) >= 1,
                "with use_id = true the ID-gated push must be delivered to the watch \
                 connection"
            );

            watch_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
            main_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
            watch_loops_handle.join().unwrap();
            loops_handle.join().unwrap();
        }
    }

    /// A mail with the given subject and message id for the T3 hardening
    /// tests below.
    fn t3_test_mail(subject: &str, message_id: &str) -> Box<Mail> {
        Box::new(
            Mail::new(
                format!(
                    "From: \"some name\" <some@example.com>\nTo: \"me\" \
                     <myself@example.com>\nCc:\nDate: Thu, 01 Jan 1970 00:00:02 +0000\nSubject: \
                     {subject}\nMessage-ID: <{message_id}>\nContent-Type: text/plain\n\nhello \
                     world.\n"
                )
                .into_bytes(),
                None,
            )
            .unwrap(),
        )
    }

    /// Failing-first regression test for tag-not-last reply framing in
    /// `ImapStream::read_lines`: a server may glue a new-mail `* EXISTS`
    /// push after the tagged reply to the IDLE terminator `DONE`, both
    /// lines in a single TCP write
    /// (`"M{k} OK IDLE terminated\r\n* n EXISTS\r\n"`). The read loop
    /// must stop at the tag line even though it is not the last complete
    /// line of the buffer, and the trailing `* EXISTS` must still reach
    /// the untagged-response processing path so the new mail is delivered
    /// as a `Create` `RefreshEvent`.
    pub(crate) fn run_imap_watch_tag_not_last() {
        let mut _logger = Logger::new_with(LogLevel::TRACE, true);
        let temp_dir = TempDir::new().unwrap();
        let backend_event_queue =
            Arc::new(Mutex::new(std::collections::VecDeque::with_capacity(16)));
        let backend_event_consumer = {
            let backend_event_queue = Arc::clone(&backend_event_queue);

            BackendEventConsumer::new(Arc::new(move |ah, be| {
                eprintln!("BackendEventConsumer: ah {ah:?} be {be:?}");
                backend_event_queue.lock().unwrap().push_back((ah, be));
            }))
        };
        set_test_xdg_env(&temp_dir);

        let seed_mail_1 = t3_test_mail("RE: tagnotlast seed 1", "tagnotlast-seed-1@example.com");
        let seed_mail_2 = t3_test_mail("RE: tagnotlast seed 2", "tagnotlast-seed-2@example.com");
        let new_mail = t3_test_mail("RE: tagnotlast NEW mail", "tagnotlast-new@example.com");

        let server_state = Arc::new(Mutex::new(ServerState {
            envelopes: indexmap::indexmap! {},
            next_uid: 1,
            uidvalidity: 1,
            glue_exists_after_done: true,
            ..Default::default()
        }));
        {
            let mut state_lck = server_state.lock().unwrap();
            state_lck.insert(seed_mail_1);
            state_lck.insert(seed_mail_2);
        }

        let (mut imap, listener, main_conn_sender, loops_handle, inbox_hash, _main_commands) =
            warm_start_setup(
                backend_event_consumer,
                Arc::clone(&server_state),
                600,
                300,
                true,
                true,
            );

        {
            let imap = &mut imap;
            std::thread::scope(|scope| {
                scope
                    .spawn(move || {
                        block_on(async {
                            let seed_envs = fetch_all_envs(imap, inbox_hash).await;
                            assert_eq!(
                                seed_envs.len(),
                                2,
                                "initial fetch must load the two seed mails"
                            );
                        });
                    })
                    .join()
                    .unwrap();
            });
        }

        let mut watch_fut = Box::pin(imap.watch().unwrap().into_future());
        let (watch_conn_sender, watch_conn_receiver) = unbounded();
        let watch_conn = ImapServerStream::new(
            &listener,
            &mut watch_fut,
            (watch_conn_sender.clone(), watch_conn_receiver),
            Arc::clone(&server_state),
        );
        // The push triggers the DONE exchange; the mock answers DONE with
        // the tagged OK and the `* EXISTS` push glued after it in the same
        // TCP write.
        watch_conn_sender
            .unbounded_send(ServerEvent::New(new_mail))
            .unwrap();
        let watch_conn_loop = watch_conn.loop_handler("watch");
        let watch_loops_handle = std::thread::spawn(move || {
            block_on(watch_conn_loop);
        });

        block_on(async {
            let mut found = false;
            while !found {
                let item = match future::select(
                    watch_fut.as_mut(),
                    smol::Timer::after(WATCH_TEST_DEADLINE),
                )
                .await
                {
                    Either::Left(((item, rest), _timeout)) => {
                        watch_fut = Box::pin(rest.into_future());
                        item
                    }
                    Either::Right((_timeout, _pending)) => {
                        panic!(
                            "watch stream did not emit the new mail's Create event within \
                             {WATCH_TEST_DEADLINE:?}; the `* EXISTS` push glued after the \
                             tagged DONE reply was not processed (tag-not-last framing)"
                        );
                    }
                };
                let Some(backend_event) = item else {
                    panic!("watch stream ended before the new mail's Create event");
                };
                match backend_event {
                    Err(err) => {
                        panic!(
                            "tag-not-last: watch stream errored instead of delivering the \
                             `* EXISTS` push glued after the tagged DONE reply: {err}"
                        );
                    }
                    Ok(BackendEvent::RefreshBatch(events)) => {
                        found = events.iter().any(|event| {
                            matches!(
                                &event.kind,
                                RefreshEventKind::Create(env)
                                    if env.subject() == "RE: tagnotlast NEW mail"
                            )
                        });
                    }
                    Ok(BackendEvent::Refresh(event)) => {
                        found = matches!(
                            &event.kind,
                            RefreshEventKind::Create(env)
                                if env.subject() == "RE: tagnotlast NEW mail"
                        );
                    }
                    Ok(other) => {
                        panic!("Expected Refresh event, got: {other:?}");
                    }
                }
            }
        });

        watch_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        main_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        watch_loops_handle.join().unwrap();
        loops_handle.join().unwrap();
    }

    /// Failing-first regression test for the DONE-before-continuation
    /// hazard in the IDLE push branch: a server may push `* n EXISTS`
    /// *before* sending the `+ idling` continuation, and may answer the
    /// premature `DONE` terminator with a tagged BAD (its idle state
    /// machine was not ready). The client must not send DONE before the
    /// continuation arrives (buffer the push data and wait a bounded
    /// grace), and a tagged BAD answering DONE must restart the IDLE flow
    /// instead of surfacing a fatal error. The push must still be
    /// delivered and the session must keep working for subsequent pushes.
    pub(crate) fn run_imap_watch_push_before_continuation() {
        let mut _logger = Logger::new_with(LogLevel::TRACE, true);
        let temp_dir = TempDir::new().unwrap();
        let backend_event_queue =
            Arc::new(Mutex::new(std::collections::VecDeque::with_capacity(16)));
        let backend_event_consumer = {
            let backend_event_queue = Arc::clone(&backend_event_queue);

            BackendEventConsumer::new(Arc::new(move |ah, be| {
                eprintln!("BackendEventConsumer: ah {ah:?} be {be:?}");
                backend_event_queue.lock().unwrap().push_back((ah, be));
            }))
        };
        set_test_xdg_env(&temp_dir);

        let seed_mail_1 = t3_test_mail("RE: precont seed 1", "precont-seed-1@example.com");
        let seed_mail_2 = t3_test_mail("RE: precont seed 2", "precont-seed-2@example.com");
        let new_mail = t3_test_mail("RE: precont NEW mail", "precont-new@example.com");
        let new_mail_2 = t3_test_mail("RE: precont NEW mail 2", "precont-new-2@example.com");

        let server_state = Arc::new(Mutex::new(ServerState {
            envelopes: indexmap::indexmap! {},
            next_uid: 1,
            uidvalidity: 1,
            push_before_continuation: true,
            ..Default::default()
        }));
        {
            let mut state_lck = server_state.lock().unwrap();
            state_lck.insert(seed_mail_1);
            state_lck.insert(seed_mail_2);
        }

        let (mut imap, listener, main_conn_sender, loops_handle, inbox_hash, _main_commands) =
            warm_start_setup(
                backend_event_consumer,
                Arc::clone(&server_state),
                600,
                300,
                true,
                true,
            );

        {
            let imap = &mut imap;
            std::thread::scope(|scope| {
                scope
                    .spawn(move || {
                        block_on(async {
                            let seed_envs = fetch_all_envs(imap, inbox_hash).await;
                            assert_eq!(
                                seed_envs.len(),
                                2,
                                "initial fetch must load the two seed mails"
                            );
                        });
                    })
                    .join()
                    .unwrap();
            });
        }

        let mut watch_fut = Box::pin(imap.watch().unwrap().into_future());
        let (watch_conn_sender, watch_conn_receiver) = unbounded();
        let watch_conn = ImapServerStream::new(
            &listener,
            &mut watch_fut,
            (watch_conn_sender.clone(), watch_conn_receiver),
            Arc::clone(&server_state),
        );
        let watch_idle_sessions = Arc::clone(&watch_conn.idle_sessions);
        // The new mail is delivered before the watch stream enters IDLE;
        // the mock buffers it and writes the `* EXISTS` push *before* the
        // `+ idling` continuation.
        watch_conn_sender
            .unbounded_send(ServerEvent::New(new_mail))
            .unwrap();
        let watch_conn_loop = watch_conn.loop_handler("watch");
        let watch_loops_handle = std::thread::spawn(move || {
            block_on(watch_conn_loop);
        });

        block_on(async {
            let mut found = false;
            while !found {
                let item = match future::select(
                    watch_fut.as_mut(),
                    smol::Timer::after(WATCH_TEST_DEADLINE),
                )
                .await
                {
                    Either::Left(((item, rest), _timeout)) => {
                        watch_fut = Box::pin(rest.into_future());
                        item
                    }
                    Either::Right((_timeout, _pending)) => {
                        panic!(
                            "watch stream did not emit the pre-continuation push's Create \
                             event within {WATCH_TEST_DEADLINE:?}"
                        );
                    }
                };
                let Some(backend_event) = item else {
                    panic!("watch stream ended before the new mail's Create event");
                };
                match backend_event {
                    Err(err) => {
                        panic!(
                            "push-before-continuation: watch stream errored instead of \
                             delivering the push that arrived before the `+ idling` \
                             continuation: {err}"
                        );
                    }
                    Ok(BackendEvent::RefreshBatch(events)) => {
                        found = events.iter().any(|event| {
                            matches!(
                                &event.kind,
                                RefreshEventKind::Create(env)
                                    if env.subject() == "RE: precont NEW mail"
                            )
                        });
                    }
                    Ok(BackendEvent::Refresh(event)) => {
                        found = matches!(
                            &event.kind,
                            RefreshEventKind::Create(env)
                                if env.subject() == "RE: precont NEW mail"
                        );
                    }
                    Ok(other) => {
                        panic!("Expected Refresh event, got: {other:?}");
                    }
                }
            }
        });

        // The session must have recovered from the tagged BAD answering
        // the DONE: it re-entered IDLE (at least two idle sessions on the
        // connection) and delivers a second, ordinary push normally.
        watch_conn_sender
            .unbounded_send(ServerEvent::New(new_mail_2))
            .unwrap();
        block_on(async {
            let mut found = false;
            while !found {
                let item = match future::select(
                    watch_fut.as_mut(),
                    smol::Timer::after(WATCH_TEST_DEADLINE),
                )
                .await
                {
                    Either::Left(((item, rest), _timeout)) => {
                        watch_fut = Box::pin(rest.into_future());
                        item
                    }
                    Either::Right((_timeout, _pending)) => {
                        panic!(
                            "watch stream did not emit the second push's Create event \
                             within {WATCH_TEST_DEADLINE:?} although the session should \
                             have recovered from the BAD answering DONE"
                        );
                    }
                };
                let Some(backend_event) = item else {
                    panic!("watch stream ended before the second Create event");
                };
                match backend_event.unwrap() {
                    BackendEvent::RefreshBatch(events) => {
                        found = events.iter().any(|event| {
                            matches!(
                                &event.kind,
                                RefreshEventKind::Create(env)
                                    if env.subject() == "RE: precont NEW mail 2"
                            )
                        });
                    }
                    BackendEvent::Refresh(event) => {
                        found = matches!(
                            &event.kind,
                            RefreshEventKind::Create(env)
                                if env.subject() == "RE: precont NEW mail 2"
                        );
                    }
                    other => {
                        panic!("Expected Refresh event, got: {other:?}");
                    }
                }
            }
        });
        assert!(
            watch_idle_sessions.load(std::sync::atomic::Ordering::SeqCst) >= 2,
            "the watch connection must have re-entered IDLE after the BAD answering DONE"
        );

        watch_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        main_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        watch_loops_handle.join().unwrap();
        loops_handle.join().unwrap();
    }

    /// Failing-first regression test for the bare `+` keepalive line: a
    /// server may emit a continuation request without the trailing space
    /// (`+\r\n`, see <https://github.com/modern-email/defects/issues/7>)
    /// as IDLE keepalive noise. The keepalive filter must treat it as
    /// noise: it must not be misclassified as push data (no DONE may be
    /// sent for it) and the watch must keep working normally afterwards.
    pub(crate) fn run_imap_watch_bare_plus_keepalive() {
        let mut _logger = Logger::new_with(LogLevel::TRACE, true);
        let temp_dir = TempDir::new().unwrap();
        let backend_event_queue =
            Arc::new(Mutex::new(std::collections::VecDeque::with_capacity(16)));
        let backend_event_consumer = {
            let backend_event_queue = Arc::clone(&backend_event_queue);

            BackendEventConsumer::new(Arc::new(move |ah, be| {
                eprintln!("BackendEventConsumer: ah {ah:?} be {be:?}");
                backend_event_queue.lock().unwrap().push_back((ah, be));
            }))
        };
        set_test_xdg_env(&temp_dir);

        let seed_mail_1 = t3_test_mail("RE: bareplus seed 1", "bareplus-seed-1@example.com");
        let seed_mail_2 = t3_test_mail("RE: bareplus seed 2", "bareplus-seed-2@example.com");
        let new_mail = t3_test_mail("RE: bareplus NEW mail", "bareplus-new@example.com");

        let server_state = Arc::new(Mutex::new(ServerState {
            envelopes: indexmap::indexmap! {},
            next_uid: 1,
            uidvalidity: 1,
            ..Default::default()
        }));
        {
            let mut state_lck = server_state.lock().unwrap();
            state_lck.insert(seed_mail_1);
            state_lck.insert(seed_mail_2);
        }

        let (mut imap, listener, main_conn_sender, loops_handle, inbox_hash, _main_commands) =
            warm_start_setup(
                backend_event_consumer,
                Arc::clone(&server_state),
                600,
                300,
                true,
                true,
            );

        {
            let imap = &mut imap;
            std::thread::scope(|scope| {
                scope
                    .spawn(move || {
                        block_on(async {
                            let seed_envs = fetch_all_envs(imap, inbox_hash).await;
                            assert_eq!(
                                seed_envs.len(),
                                2,
                                "initial fetch must load the two seed mails"
                            );
                        });
                    })
                    .join()
                    .unwrap();
            });
        }

        let mut watch_fut = Box::pin(imap.watch().unwrap().into_future());
        let (watch_conn_sender, watch_conn_receiver) = unbounded();
        let watch_conn = ImapServerStream::new(
            &listener,
            &mut watch_fut,
            (watch_conn_sender.clone(), watch_conn_receiver),
            Arc::clone(&server_state),
        );
        let watch_idle_sessions = Arc::clone(&watch_conn.idle_sessions);
        let watch_idle_received = Arc::clone(&watch_conn.idle_received_lines);
        let watch_conn_loop = watch_conn.loop_handler("watch");
        let watch_loops_handle = std::thread::spawn(move || {
            block_on(watch_conn_loop);
        });

        // Drive the watch stream until the connection has entered IDLE, so
        // the bare `+` is injected during idle (keepalive context), not
        // during the pre-IDLE command exchange. Bounded by
        // `WATCH_TEST_DEADLINE`; the poll tick only affects how soon the
        // condition is noticed.
        block_on(async {
            let deadline = std::time::Instant::now() + WATCH_TEST_DEADLINE;
            while watch_idle_sessions.load(std::sync::atomic::Ordering::SeqCst) < 1 {
                if std::time::Instant::now() >= deadline {
                    panic!("watch connection did not enter IDLE within {WATCH_TEST_DEADLINE:?}");
                }
                match future::select(watch_fut.as_mut(), smol::Timer::after(WATCH_TEST_POLL_TICK))
                    .await
                {
                    Either::Left(((item, rest), _tick)) => {
                        watch_fut = Box::pin(rest.into_future());
                        if let Some(ev) = item {
                            let _ = ev.unwrap();
                        }
                    }
                    Either::Right((_tick, _pending)) => {}
                }
            }
        });

        // Inject the bare `+` keepalive line.
        watch_conn_sender
            .unbounded_send(ServerEvent::RawLine(b"+\r\n".to_vec()))
            .unwrap();

        // Bounded negative window: with the buggy filter the client sends
        // DONE within milliseconds of the bare `+` (the push branch runs
        // synchronously on the received line); with the fix no line is
        // ever sent. Keep driving the watch stream while waiting.
        block_on(async {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            loop {
                if std::time::Instant::now() >= deadline {
                    return;
                }
                if watch_idle_received
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|l| l == "DONE\r\n")
                {
                    panic!(
                        "bare `+` keepalive line was misclassified as push data: the client \
                         sent DONE while merely idling (idle received lines: {:?})",
                        watch_idle_received.lock().unwrap()
                    );
                }
                match future::select(watch_fut.as_mut(), smol::Timer::after(WATCH_TEST_POLL_TICK))
                    .await
                {
                    Either::Left(((item, rest), _tick)) => {
                        watch_fut = Box::pin(rest.into_future());
                        if let Some(ev) = item {
                            let _ = ev.unwrap();
                        }
                    }
                    Either::Right((_tick, _pending)) => {}
                }
            }
        });

        // The watch must still deliver an ordinary push afterwards.
        watch_conn_sender
            .unbounded_send(ServerEvent::New(new_mail))
            .unwrap();
        block_on(async {
            let mut found = false;
            while !found {
                let item = match future::select(
                    watch_fut.as_mut(),
                    smol::Timer::after(WATCH_TEST_DEADLINE),
                )
                .await
                {
                    Either::Left(((item, rest), _timeout)) => {
                        watch_fut = Box::pin(rest.into_future());
                        item
                    }
                    Either::Right((_timeout, _pending)) => {
                        panic!(
                            "watch stream did not emit the new mail's Create event within \
                             {WATCH_TEST_DEADLINE:?} after the bare `+` keepalive line"
                        );
                    }
                };
                let Some(backend_event) = item else {
                    panic!("watch stream ended before the new mail's Create event");
                };
                match backend_event.unwrap() {
                    BackendEvent::RefreshBatch(events) => {
                        found = events.iter().any(|event| {
                            matches!(
                                &event.kind,
                                RefreshEventKind::Create(env)
                                    if env.subject() == "RE: bareplus NEW mail"
                            )
                        });
                    }
                    BackendEvent::Refresh(event) => {
                        found = matches!(
                            &event.kind,
                            RefreshEventKind::Create(env)
                                if env.subject() == "RE: bareplus NEW mail"
                        );
                    }
                    other => {
                        panic!("Expected Refresh event, got: {other:?}");
                    }
                }
            }
        });

        watch_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        main_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        watch_loops_handle.join().unwrap();
        loops_handle.join().unwrap();
    }

    /// Regression pin (expected GREEN) for a `* BYE` arriving mid-IDLE
    /// with the watched mailbox selected: the connection is going away,
    /// so the watch must surface the disconnect (`Disconnected` or the
    /// OS-level EPIPE/ECONNRESET of the closed socket) instead of hanging
    /// or silently ending. The name carries "unselected" for the related
    /// gap fixed in `process_untagged` (BYE silently dropped by the
    /// unselected-state early exit); through the wire protocol that gap
    /// is not directly reachable today because the untagged parser
    /// requires a message number after `* ` and cannot produce
    /// `UntaggedResponse::Bye` — see the evidence file for the analysis.
    pub(crate) fn run_imap_watch_bye_mid_idle_unselected() {
        let mut _logger = Logger::new_with(LogLevel::TRACE, true);
        let temp_dir = TempDir::new().unwrap();
        let backend_event_queue =
            Arc::new(Mutex::new(std::collections::VecDeque::with_capacity(16)));
        let backend_event_consumer = {
            let backend_event_queue = Arc::clone(&backend_event_queue);

            BackendEventConsumer::new(Arc::new(move |ah, be| {
                eprintln!("BackendEventConsumer: ah {ah:?} be {be:?}");
                backend_event_queue.lock().unwrap().push_back((ah, be));
            }))
        };
        set_test_xdg_env(&temp_dir);

        let seed_mail_1 = t3_test_mail("RE: bye seed 1", "bye-seed-1@example.com");
        let seed_mail_2 = t3_test_mail("RE: bye seed 2", "bye-seed-2@example.com");

        let server_state = Arc::new(Mutex::new(ServerState {
            envelopes: indexmap::indexmap! {},
            next_uid: 1,
            uidvalidity: 1,
            ..Default::default()
        }));
        {
            let mut state_lck = server_state.lock().unwrap();
            state_lck.insert(seed_mail_1);
            state_lck.insert(seed_mail_2);
        }

        let (mut imap, listener, main_conn_sender, loops_handle, inbox_hash, _main_commands) =
            warm_start_setup(
                backend_event_consumer,
                Arc::clone(&server_state),
                600,
                300,
                true,
                true,
            );

        {
            let imap = &mut imap;
            std::thread::scope(|scope| {
                scope
                    .spawn(move || {
                        block_on(async {
                            let seed_envs = fetch_all_envs(imap, inbox_hash).await;
                            assert_eq!(
                                seed_envs.len(),
                                2,
                                "initial fetch must load the two seed mails"
                            );
                        });
                    })
                    .join()
                    .unwrap();
            });
        }

        let mut watch_fut = Box::pin(imap.watch().unwrap().into_future());
        let (watch_conn_sender, watch_conn_receiver) = unbounded();
        let watch_conn = ImapServerStream::new(
            &listener,
            &mut watch_fut,
            (watch_conn_sender.clone(), watch_conn_receiver),
            Arc::clone(&server_state),
        );
        let watch_idle_sessions = Arc::clone(&watch_conn.idle_sessions);
        let watch_conn_loop = watch_conn.loop_handler("watch");
        let watch_loops_handle = std::thread::spawn(move || {
            block_on(watch_conn_loop);
        });

        // Drive the watch stream until the connection has entered IDLE (the
        // watched mailbox is EXAMINEd, i.e. the selected-state context of
        // this pin). Bounded by `WATCH_TEST_DEADLINE`.
        block_on(async {
            let deadline = std::time::Instant::now() + WATCH_TEST_DEADLINE;
            while watch_idle_sessions.load(std::sync::atomic::Ordering::SeqCst) < 1 {
                if std::time::Instant::now() >= deadline {
                    panic!("watch connection did not enter IDLE within {WATCH_TEST_DEADLINE:?}");
                }
                match future::select(watch_fut.as_mut(), smol::Timer::after(WATCH_TEST_POLL_TICK))
                    .await
                {
                    Either::Left(((item, rest), _tick)) => {
                        watch_fut = Box::pin(rest.into_future());
                        if let Some(ev) = item {
                            let _ = ev.unwrap();
                        }
                    }
                    Either::Right((_tick, _pending)) => {}
                }
            }
        });

        // The server says goodbye and closes the connection mid-IDLE.
        watch_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();

        block_on(async {
            loop {
                let item = match future::select(
                    watch_fut.as_mut(),
                    smol::Timer::after(WATCH_TEST_DEADLINE),
                )
                .await
                {
                    Either::Left(((item, rest), _timeout)) => {
                        watch_fut = Box::pin(rest.into_future());
                        item
                    }
                    Either::Right((_timeout, _pending)) => {
                        panic!(
                            "watch stream produced no disconnect within {WATCH_TEST_DEADLINE:?} \
                             of the mid-IDLE `* BYE`; it is likely hanging"
                        );
                    }
                };
                let Some(backend_event) = item else {
                    panic!(
                        "watch stream ended without surfacing the mid-IDLE `* BYE` \
                         disconnect"
                    );
                };
                match backend_event {
                    Err(err) => {
                        assert!(
                            matches!(
                                err.kind,
                                ErrorKind::OSError(
                                    nix::errno::Errno::EPIPE | nix::errno::Errno::ECONNRESET
                                )
                            ) || err.summary == "Disconnected",
                            "expected the mid-IDLE BYE to surface as a disconnect \
                             (Disconnected or EPIPE/ECONNRESET), got: {err:?}"
                        );
                        break;
                    }
                    Ok(_) => {
                        // Drain any startup events until the disconnect.
                    }
                }
            }
        });

        main_conn_sender.unbounded_send(ServerEvent::Quit).unwrap();
        watch_loops_handle.join().unwrap();
        loops_handle.join().unwrap();
    }
}
