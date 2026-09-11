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
            atomic::{AtomicBool, Ordering},
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
        core::{LiteralMode, NString},
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
    pub struct ServerState {
        pub envelopes: IndexMap<UID, Mail>,
        pub next_uid: UID,
        pub uidvalidity: UID,
    }

    impl ServerState {
        pub fn insert(&mut self, new: Box<Mail>) -> (usize, UID) {
            let uid = self.next_uid;
            self.envelopes.insert(uid, *new);
            let msn = self.envelopes.len();
            self.next_uid += 1;
            (msn, uid)
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
                    .map(|s| s.to_string().try_into().unwrap())
                    .unwrap_or(NString(None)),
            }
        }
    }

    #[derive(Debug)]
    pub enum ServerEvent {
        New(Box<Mail>),
        Delete(UID),
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
        /// When `Some`, sleep before answering SELECT/EXAMINE commands.
        pub select_reply_delay: Option<Duration>,
        /// Set to `true` once the server has written a SELECT/EXAMINE
        /// reply. Used as ground truth for asserting that a client
        /// action happened before the server answered a SELECT.
        pub select_reply_sent: Arc<AtomicBool>,
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
                    match block_on(future::select(next_fut, read_fut)) {
                        Either::Left((_, _)) => {
                            unreachable!();
                        }
                        Either::Right((value2, _)) => {
                            let read_bytes = value2.unwrap();
                            assert_eq!(&buf[..read_bytes], b"M3 CAPABILITY\r\n");
                        }
                    };
                    block_on(
                        tcp_stream.write_all(
                            b"* CAPABILITY IMAP4rev1 ID IDLE ENABLE\r\nM3 OK Success\r\n",
                        ),
                    )
                    .unwrap();
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
                select_reply_delay: None,
                select_reply_sent: Arc::default(),
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
                select_reply_delay,
                select_reply_sent,
            } = self;
            let mut buf_start = 0;
            let mut buf_end = 0;
            async fn read_line<'a>(
                tcp_stream: &mut Async<TcpStream>,
                buf: &'a mut [u8],
                start: &mut usize,
                end: &mut usize,
            ) -> Option<&'a [u8]> {
                // log::trace!(
                //     "read_line: buf={:?} start = {start:?} end = {end:?}",
                //     String::from_utf8_lossy(&buf[..*end])
                // );
                if *start == 0 || !buf[*start..*end].contains_subsequence(b"\r\n") {
                    let read_bytes = tcp_stream.read(&mut buf[*end..]).await.unwrap();
                    *end += read_bytes;
                    // log::trace!(
                    //     "read_line: read_bytes = {read_bytes:?} buf = {:?}",
                    //     String::from_utf8_lossy(&buf[..*end])
                    // );
                    if !buf[*start..*end].contains_subsequence(b"\r\n") {
                        // log::trace!("read_line: returning None");
                        return None;
                    }
                    let Some(input) = buf[*start..*end].split_rn().next() else {
                        // log::trace!("read_line: returning None");
                        return None;
                    };
                    *start += input.len();
                    if *start == *end {
                        *start = 0;
                        *end = 0;
                    }
                    // log::trace!("read_line: returning {:?}", String::from_utf8_lossy(input));
                    Some(input)
                } else {
                    let rest = &buf[*start..*end];
                    let input = rest.split_rn().next().unwrap();
                    *start += input.len();
                    if *start == *end {
                        *start = 0;
                        *end = 0;
                    }
                    // log::trace!("read_line: returning {:?}", String::from_utf8_lossy(input));
                    Some(input)
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
                        Either::Left((value1, _)) => {
                            if value1.is_none() {
                                continue 'main;
                            }
                            drop(read_fut);
                            value1.unwrap()
                        }
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
                            tcp_stream.write_all(id.as_bytes()).await.unwrap();
                            tcp_stream
                                .write_all(b" OK NOOP completed\r\n")
                                .await
                                .unwrap();
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
                            session_state = SessionState::SelectedMailbox;
                            let (exists, recent, uidvalidity) = {
                                let state_lck = state.lock().unwrap();
                                let uidvalidity = state_lck.uidvalidity;
                                let exists = state_lck.envelopes.len();
                                let recent = 0;
                                (exists, recent, uidvalidity)
                            };
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
                            session_state = SessionState::SelectedMailbox;
                            let (exists, recent, uidvalidity) = {
                                let state_lck = state.lock().unwrap();
                                let uidvalidity = state_lck.uidvalidity;
                                let exists = state_lck.envelopes.len();
                                let recent = 0;
                                (exists, recent, uidvalidity)
                            };
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
                            tcp_stream.write_all(id.as_bytes()).await.unwrap();
                            tcp_stream
                                .write_all(b" OK UNSELECT succeeded\r\n")
                                .await
                                .unwrap();
                            tcp_stream.flush().await.unwrap();
                        }
                        "CLOSE\r\n" => {
                            session_state = SessionState::Authenticated;
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
                            let (messages, unseen, uidnext, uidvalidity) = {
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
                            };
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
                            let msn_to_fetch = fetch
                                .strip_prefix("FETCH ")
                                .unwrap()
                                .strip_suffix(
                                    " (UID FLAGS ENVELOPE BODY.PEEK[HEADER.FIELDS (REFERENCES)] \
                                     BODYSTRUCTURE)\r\n",
                                )
                                .unwrap()
                                .parse::<usize>()
                                .unwrap();
                            eprintln!("{name} loop_handler got FETCH for msn {msn_to_fetch}");
                            let Some((uid, mail)) = state
                                .lock()
                                .unwrap()
                                .envelopes
                                .get_index(msn_to_fetch.saturating_sub(1))
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
                                seq: (msn_to_fetch as u32).try_into().unwrap(),
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
                                let response = Response::Data(Data::Fetch {
                                    seq: uid,
                                    items: vec![
                                        MessageDataItem::Uid(uid),
                                        MessageDataItem::Flags(mail.as_flags()),
                                    ]
                                    .try_into()
                                    .unwrap(),
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
                                let mut items = vec![
                                    MessageDataItem::Uid(uid),
                                    MessageDataItem::Flags(mail.as_flags()),
                                    MessageDataItem::Envelope(mail.as_envelope()),
                                    references,
                                ];
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
                                .write_all(format!("{id} OK UID FETCH completed\r\n").as_bytes())
                                .await
                                .unwrap();
                            tcp_stream.flush().await.unwrap();
                        }
                        other => panic!("Unexpected cmd: {id} {other:?}"),
                    }
                };
                eprintln!("{name} loop_handler is now idling");
                tcp_stream.write_all(b"+ idling\r\n").await.unwrap();
                tcp_stream.flush().await.unwrap();
                'idle: loop {
                    let mut read_fut = Box::pin(read_line(
                        &mut tcp_stream,
                        &mut buf,
                        &mut buf_start,
                        &mut buf_end,
                    ));
                    let input = match future::select(&mut read_fut, command_receiver.next()).await {
                        Either::Left((value1, _)) => {
                            if value1.is_none() {
                                continue 'idle;
                            }
                            drop(read_fut);
                            value1.unwrap()
                        }
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
                    if input == "DONE\r\n" {
                        tcp_stream.write_all(idle_cmd_id.as_bytes()).await.unwrap();
                        tcp_stream
                            .write_all(b" OK IDLE terminated\r\n")
                            .await
                            .unwrap();
                        continue 'outer;
                    }
                }
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
            let expected_commands: Vec<String> = [
                "M4 LIST \"\" *\r\n",
                "M5 LSUB \"\" *\r\n",
                "M6 SELECT INBOX\r\n",
                "M7 UID SEARCH 1:*\r\n",
                "M8 EXAMINE INBOX\r\n",
                "M9 UID SEARCH 1:*\r\n",
                "M10 STATUS INBOX (UIDNEXT)\r\n",
                "M11 SELECT INBOX\r\n",
                "M12 EXAMINE INBOX\r\n",
                "M13 UID SEARCH 1:*\r\n",
                "M14 STATUS INBOX (UIDNEXT)\r\n",
                "M15 UID FETCH 1:4 (UID FLAGS ENVELOPE BODY.PEEK[HEADER.FIELDS (REFERENCES)] \
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
            "M15 UID FETCH 1:4 (UID FLAGS ENVELOPE BODY.PEEK[HEADER.FIELDS (REFERENCES)] \
             BODYSTRUCTURE)\r\n"
        } else {
            "M15 UID FETCH 1:4 (UID FLAGS ENVELOPE BODY.PEEK[HEADER.FIELDS (REFERENCES)])\r\n"
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
}
