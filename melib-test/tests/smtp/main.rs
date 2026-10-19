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

// [ref:TODO]: add tests for SMTP server timeouts RFC5321 Section 4.5.3.2
// [ref:TODO]: add tests for STARTTLS/TLS

#![cfg(feature = "smtp")]

#[test]
fn test_smtp_transaction() {
    melib_test::init_test_logging();
    tokio_test::block_on(async {
        tests::run_smtp_transaction();
    });
}

#[test]
fn test_smtp_transaction_rcpt_rejected() {
    melib_test::init_test_logging();
    tokio_test::block_on(async {
        tests::run_smtp_transaction_rcpt_rejected();
    });
}

#[test]
fn test_smtp_pipelining() {
    melib_test::init_test_logging();
    tokio_test::block_on(async {
        tests::run_smtp_pipelining();
    });
}

#[test]
fn test_smtp_auth_methods() {
    melib_test::init_test_logging();
    tokio_test::block_on(async {
        tests::run_smtp_auth_methods();
    });
}

#[test]
fn test_smtp_bdat_wire_format() {
    melib_test::init_test_logging();
    tokio_test::block_on(async {
        tests::run_smtp_bdat_wire_format();
    });
}

#[test]
fn test_smtp_binarymime_requires_chunking() {
    melib_test::init_test_logging();
    tokio_test::block_on(async {
        tests::run_smtp_binarymime_requires_chunking();
    });
}

#[test]
fn test_smtp_rcpt_251_accepted() {
    melib_test::init_test_logging();
    tokio_test::block_on(async {
        tests::run_smtp_rcpt_251_accepted();
    });
}

pub mod server {
    use std::{
        net::{IpAddr, SocketAddr, TcpListener, TcpStream},
        sync::{Arc, Mutex},
    };

    use base64::{engine::general_purpose::STANDARD as base64, Engine as _};
    use futures::{
        channel::mpsc::UnboundedReceiver,
        future::{self, Either},
        io::{AsyncReadExt, AsyncWriteExt},
        StreamExt,
    };
    use melib::{backends::prelude::*, smol::Async, smtp::*};

    /// Token the mock `AUTH OAUTHBEARER` server accepts.
    pub const OAUTHBEARER_TOKEN: &str = "s3cr3t-t0k3n";

    #[derive(Debug)]
    pub enum ServerEvent {
        Quit,
    }

    #[derive(Clone, Debug)]
    pub enum Message {
        Helo,
        Mail {
            from: String,
        },
        Rcpt {
            from: String,
            to: Vec<String>,
        },
        DataStart {
            from: String,
            to: Vec<String>,
        },
        Data {
            #[allow(dead_code)]
            from: String,
            to: Vec<String>,
            buf: Vec<u8>,
        },
    }

    type QueuedMail = ((IpAddr, String), Message);

    #[derive(Debug)]
    pub struct ServerState {
        pub mails: Vec<QueuedMail>,
        pub stored: Vec<(String, Envelope)>,
        /// `RCPT TO` addresses that must be rejected, mapped to the full reply
        /// text that the server sends after the `550` code.
        pub rcpt_rejections: Vec<(String, String)>,
        /// `RCPT TO` addresses answered with `251 User not local; will forward`;
        /// they are accepted and stored like any other recipient.
        pub rcpt_forward: Vec<String>,
        pub auth: SmtpAuth,
        pub extensions: SmtpExtensionSupport,
        pub logindisabled: bool,
        pub plaindisabled: bool,
        pub oauthbearerdisabled: bool,
        /// Raw `AUTH LOGIN` continuation lines, including their CRLF.
        pub auth_lines: Vec<String>,
        /// Raw `BDAT <len> LAST` command lines, including their CRLF.
        pub bdat_lines: Vec<String>,
        /// Raw `MAIL FROM` command lines, including their CRLF.
        pub mail_from_lines: Vec<String>,
    }

    impl ServerState {
        pub fn clear(&mut self) {
            self.mails.clear();
            self.rcpt_rejections.clear();
            self.stored.clear();
        }

        /// <https://www.rfc-editor.org/info/rfc5321/#section-4.1.1.1>
        fn ehlo(&mut self, ip: IpAddr, domain: &str) -> String {
            tracing::debug!("EHLOing ip {ip:?} domain {domain:?}");
            let mut extensions = vec![];
            for (support, ext) in [
                (self.extensions.pipelining, "PIPELINING"),
                (self.extensions.chunking, "CHUNKING"),
                (self.extensions.prdr, "PRDR"),
                (self.extensions._8bitmime, "8BITMIME"),
                (self.extensions.binarymime, "BINARYMIME"),
                (self.extensions.smtputf8, "SMTPUTF8"),
                (self.extensions.auth && !self.plaindisabled, "AUTH PLAIN"),
                (self.extensions.auth && !self.logindisabled, "AUTH LOGIN"),
                (
                    self.extensions.auth && !self.oauthbearerdisabled,
                    "AUTH OAUTHBEARER",
                ),
            ] {
                if support {
                    extensions.push(ext);
                }
            }
            self.mails.push(((ip, domain.to_string()), Message::Helo));
            if extensions.is_empty() {
                return String::new();
            }
            let mut extensions = extensions.into_iter().peekable();
            let mut reply = String::new();
            while let Some(ext) = extensions.next() {
                let is_last = extensions.peek().is_none();
                if is_last {
                    reply.push_str("250 ");
                } else {
                    reply.push_str("250-");
                }
                reply.push_str(ext);
                reply.push_str("\r\n");
            }
            reply
        }

        fn auto_credentials(&self) -> Option<(String, String)> {
            match &self.auth {
                SmtpAuth::Auto {
                    username, password, ..
                } => Some((username.value().ok()?, password.value().ok()?)),
                _ => None,
            }
        }

        fn mail(&mut self, ip: IpAddr, domain: &str, from: &str) -> ReplyCode {
            tracing::debug!("mail() ip {ip:?} domain {domain:?} from {from:?}");
            if let Some((_, message)) = self
                .mails
                .iter_mut()
                .rev()
                .find(|((i, d), _)| (i, d.as_str()) == (&ip, domain))
            {
                tracing::debug!("mail is {message:?}");
                if matches!(message, Message::Helo) {
                    *message = Message::Mail {
                        from: from.to_string(),
                    };
                    return ReplyCode::_250;
                }
            }
            ReplyCode::_451
        }

        fn rcpt(&mut self, to: &str) -> ReplyCode {
            tracing::debug!("rcpt() to {to:?}");
            if let Some((_, message)) = self.mails.last_mut() {
                tracing::debug!("rcpt mail is {message:?}");
                if let Message::Mail { from } = message {
                    *message = Message::Rcpt {
                        from: from.clone(),
                        to: vec![to.to_string()],
                    };
                    return ReplyCode::_250;
                } else if let Message::Rcpt { to: tos, .. } = message {
                    tos.push(to.to_string());
                    return ReplyCode::_250;
                }
                return ReplyCode::_451;
            }
            ReplyCode::_451
        }

        fn data_start(&mut self) -> ReplyCode {
            if let Some(((_, _), ref mut message)) = self.mails.last_mut() {
                tracing::debug!("data_start mail is {message:?}");
                if let Message::Rcpt { from, to } = message {
                    *message = Message::DataStart {
                        from: from.to_string(),
                        to: to.to_vec(),
                    };
                    return ReplyCode::_354;
                }
            }
            ReplyCode::_451
        }

        fn data(&mut self, buf: &[u8]) -> std::result::Result<(), std::io::Error> {
            if let Some(((_, _), ref mut message)) = self.mails.last_mut() {
                if let Message::DataStart { from, to } = message {
                    *message = Message::Data {
                        from: from.to_string(),
                        to: to.clone(),
                        buf: buf.to_vec(),
                    };
                    return Ok(());
                } else if let Message::Data { buf: data, .. } = message {
                    data.extend(buf.iter());
                    return Ok(());
                }
            }
            Ok(())
        }

        fn data_end(&mut self) -> ReplyCode {
            let last = self.mails.pop();
            if let Some(((ip, domain), Message::Data { from: _, to, buf })) = last {
                for to in to {
                    match Envelope::from_bytes(&buf, None) {
                        Ok(env) => {
                            tracing::debug!("data_end env is {env:?}");
                            tracing::debug!(
                                "data_end env.other_headers is {:?}",
                                env.other_headers()
                            );
                            self.stored.push((to.clone(), env));
                        }
                        Err(err) => {
                            tracing::debug!("envelope parse error {err}");
                        }
                    }
                }
                self.mails.push(((ip, domain), Message::Helo));
                return ReplyCode::_250;
            }
            tracing::debug!("last self.mails item was not Message::Data: {last:?}");
            ReplyCode::_451
        }

        /// Complete a `BDAT ... LAST` transaction: the payload is exactly the
        /// bytes the client announced, with no DATA terminator scanning.
        fn bdat(&mut self, buf: &[u8]) -> ReplyCode {
            let transitioned = if let Some(((_, _), ref mut message)) = self.mails.last_mut() {
                if let Message::Rcpt { from, to } = message {
                    *message = Message::Data {
                        from: from.to_string(),
                        to: to.to_vec(),
                        buf: buf.to_vec(),
                    };
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if transitioned {
                self.data_end()
            } else {
                ReplyCode::_451
            }
        }
    }

    pub struct SmtpServer {
        pub tcp_listener: Async<TcpListener>,
        pub event_receiver: UnboundedReceiver<ServerEvent>,
        pub state: Arc<Mutex<ServerState>>,
        pub addr: SocketAddr,
    }

    impl SmtpServer {
        pub fn new(
            event_receiver: UnboundedReceiver<ServerEvent>,
            auth: SmtpAuth,
            extensions: SmtpExtensionSupport,
        ) -> Self {
            // Do not call `extensions.validate()` here: tests need to simulate
            // non-compliant servers (e.g. BINARYMIME without CHUNKING).
            let tcp_listener =
                smol::Async::new(TcpListener::bind(("127.0.0.1", 0)).unwrap()).unwrap();
            let addr = tcp_listener.as_ref().local_addr().unwrap();
            let state = Arc::new(Mutex::new(ServerState {
                mails: vec![],
                stored: vec![],
                rcpt_rejections: vec![],
                rcpt_forward: vec![],
                auth,
                extensions,
                logindisabled: true,
                plaindisabled: false,
                oauthbearerdisabled: true,
                auth_lines: vec![],
                bdat_lines: vec![],
                mail_from_lines: vec![],
            }));
            Self {
                tcp_listener,
                event_receiver,
                state,
                addr,
            }
        }

        async fn handle_connection(
            (mut tcp_stream, socket_addr): (Async<TcpStream>, SocketAddr),
            server_addr: SocketAddr,
            state: Arc<Mutex<ServerState>>,
        ) -> Result<()> {
            async fn write_reply_code(
                tcp_stream: &mut Async<TcpStream>,
                reply_code: ReplyCode,
            ) -> Result<()> {
                tracing::debug!("writing reply code {reply_code:?}");
                tcp_stream
                    .write_all(
                        format!("{} {}\r\n", reply_code.value(), reply_code.as_str()).as_bytes(),
                    )
                    .await?;
                tcp_stream.flush().await?;
                Ok(())
            }

            async fn write_reply_code_with_text(
                tcp_stream: &mut Async<TcpStream>,
                reply_code: ReplyCode,
                text: &str,
            ) -> Result<()> {
                tcp_stream
                    .write_all(format!("{} {}\r\n", reply_code.value(), text).as_bytes())
                    .await?;
                tcp_stream.flush().await?;
                Ok(())
            }

            /// Read a single CRLF-terminated line, keeping unconsumed bytes in
            /// `buf` so several pipelined commands in one TCP read are all
            /// processed. `Ok(None)` means the peer closed the connection.
            async fn read_line(
                tcp_stream: &mut Async<TcpStream>,
                buf: &mut [u8],
                start: &mut usize,
                end: &mut usize,
            ) -> Result<Option<Vec<u8>>> {
                loop {
                    if let Some(pos) = buf[*start..*end].windows(2).position(|w| w == b"\r\n") {
                        let line = buf[*start..*start + pos + 2].to_vec();
                        *start += pos + 2;
                        if *start == *end {
                            *start = 0;
                            *end = 0;
                        }
                        return Ok(Some(line));
                    }
                    if *start > 0 {
                        buf.copy_within(*start..*end, 0);
                        *end -= *start;
                        *start = 0;
                    }
                    let read_bytes = tcp_stream.read(&mut buf[*end..]).await?;
                    if read_bytes == 0 {
                        return Ok(None);
                    }
                    *end += read_bytes;
                }
            }

            fn decode_b64_line(line: &str) -> Option<String> {
                let line = line.strip_suffix("\r\n").unwrap_or(line);
                let bytes = base64.decode(line).ok()?;
                String::from_utf8(bytes).ok()
            }

            fn parse_path(rest: &str, prefix: &str) -> Option<String> {
                let rest = rest.trim_end().strip_prefix(prefix)?;
                let end = rest.find('>')?;
                Some(rest[..end].to_string())
            }

            let mut buf = vec![0; 64 * 1024];
            let mut buf_start = 0;
            let mut buf_end = 0;
            tcp_stream
                .write_all(format!("220 {} Service ready\r\n", server_addr.ip()).as_bytes())
                .await?;
            tcp_stream.flush().await?;
            let mut authenticated = state.lock().unwrap().auth == SmtpAuth::None;
            let mut domain = String::new();
            'main: loop {
                let Some(line) =
                    read_line(&mut tcp_stream, &mut buf, &mut buf_start, &mut buf_end).await?
                else {
                    return Ok(());
                };
                let input = String::from_utf8_lossy(&line).to_string();
                let (cmd, rest) = input.split_once(' ').unwrap_or((&input, ""));
                if cmd == "AUTH" {
                    // Never log AUTH payloads: they carry credentials or tokens.
                    tracing::debug!("received: AUTH <redacted>");
                } else {
                    tracing::debug!("received: {input:?}");
                }
                match cmd {
                    "QUIT\r\n" => {
                        write_reply_code(&mut tcp_stream, ReplyCode::_221).await?;
                        return Ok(());
                    }
                    "NOOP\r\n" => {
                        write_reply_code(&mut tcp_stream, ReplyCode::_250).await?;
                    }
                    "EHLO" => {
                        domain = rest.trim().to_string();
                        let reply = state.lock().unwrap().ehlo(socket_addr.ip(), &domain);
                        if reply.is_empty() {
                            write_reply_code(&mut tcp_stream, ReplyCode::_250).await?;
                        } else {
                            tcp_stream.write_all(reply.as_bytes()).await?;
                            tcp_stream.flush().await?;
                        }
                    }
                    "AUTH" => {
                        if authenticated {
                            write_reply_code(&mut tcp_stream, ReplyCode::_503).await?;
                            continue 'main;
                        }
                        if rest.starts_with("LOGIN\r\n") {
                            // b"User Name"
                            tcp_stream.write_all(b"334 VXNlciBOYW1lAA==\r\n").await?;
                            tcp_stream.flush().await?;
                            let Some(line) =
                                read_line(&mut tcp_stream, &mut buf, &mut buf_start, &mut buf_end)
                                    .await?
                            else {
                                return Ok(());
                            };
                            let raw = String::from_utf8_lossy(&line).to_string();
                            state.lock().unwrap().auth_lines.push(raw.clone());
                            let username = decode_b64_line(&raw);
                            // b"Password"
                            tcp_stream.write_all(b"334 UGFzc3dvcmQA\r\n").await?;
                            tcp_stream.flush().await?;
                            let Some(line) =
                                read_line(&mut tcp_stream, &mut buf, &mut buf_start, &mut buf_end)
                                    .await?
                            else {
                                return Ok(());
                            };
                            let raw = String::from_utf8_lossy(&line).to_string();
                            state.lock().unwrap().auth_lines.push(raw.clone());
                            let password = decode_b64_line(&raw);
                            let expected = state.lock().unwrap().auto_credentials();
                            if let (Some(username), Some(password), Some(expected)) =
                                (username, password, expected)
                            {
                                if (username, password) == expected {
                                    authenticated = true;
                                    write_reply_code(&mut tcp_stream, ReplyCode::_235).await?;
                                } else {
                                    write_reply_code(&mut tcp_stream, ReplyCode::_535).await?;
                                }
                            } else {
                                write_reply_code(&mut tcp_stream, ReplyCode::_535).await?;
                            }
                        } else if let Some(username_password) = rest.strip_prefix("PLAIN ") {
                            let actual = decode_b64_line(username_password);
                            let expected = state.lock().unwrap().auto_credentials();
                            if actual
                                == expected
                                    .as_ref()
                                    .map(|(username, password)| format!("\0{username}\0{password}"))
                            {
                                authenticated = true;
                                write_reply_code(&mut tcp_stream, ReplyCode::_235).await?;
                            } else {
                                write_reply_code(&mut tcp_stream, ReplyCode::_535).await?;
                            }
                        } else if let Some(token) = rest.strip_prefix("OAUTHBEARER ") {
                            let actual = decode_b64_line(token);
                            if actual.as_deref() == Some(OAUTHBEARER_TOKEN) {
                                authenticated = true;
                                write_reply_code(&mut tcp_stream, ReplyCode::_235).await?;
                            } else {
                                write_reply_code(&mut tcp_stream, ReplyCode::_535).await?;
                            }
                        } else {
                            write_reply_code(&mut tcp_stream, ReplyCode::_504).await?;
                        }
                    }
                    "MAIL" => {
                        state.lock().unwrap().mail_from_lines.push(input.clone());
                        if !authenticated {
                            write_reply_code(&mut tcp_stream, ReplyCode::_530).await?;
                            continue 'main;
                        }
                        let Some(from) = parse_path(rest, "FROM:<") else {
                            write_reply_code(&mut tcp_stream, ReplyCode::_501).await?;
                            continue 'main;
                        };
                        let reply_code =
                            state.lock().unwrap().mail(socket_addr.ip(), &domain, &from);
                        write_reply_code(&mut tcp_stream, reply_code).await?;
                    }
                    "RCPT" => {
                        if !authenticated {
                            write_reply_code(&mut tcp_stream, ReplyCode::_530).await?;
                            continue 'main;
                        }
                        let Some(to) = parse_path(rest, "TO:<") else {
                            write_reply_code(&mut tcp_stream, ReplyCode::_501).await?;
                            continue 'main;
                        };
                        let (reply_code, rejection) = {
                            let mut state = state.lock().unwrap();
                            if let Some((_, text)) = state
                                .rcpt_rejections
                                .iter()
                                .find(|(addr, _)| addr == &to)
                                .cloned()
                            {
                                (ReplyCode::_550, Some(text))
                            } else if state.rcpt_forward.iter().any(|addr| addr == &to) {
                                let _ = state.rcpt(&to);
                                (ReplyCode::_251, None)
                            } else {
                                (state.rcpt(&to), None)
                            }
                        };
                        if let Some(text) = rejection {
                            write_reply_code_with_text(&mut tcp_stream, ReplyCode::_550, &text)
                                .await?;
                        } else {
                            write_reply_code(&mut tcp_stream, reply_code).await?;
                        }
                    }
                    "DATA\r\n" => {
                        if !authenticated {
                            write_reply_code(&mut tcp_stream, ReplyCode::_530).await?;
                            continue 'main;
                        }
                        let reply_code = state.lock().unwrap().data_start();
                        write_reply_code(&mut tcp_stream, reply_code).await?;
                        if reply_code != ReplyCode::_354 {
                            continue 'main;
                        }
                        let mut data: Vec<u8> = Vec::new();
                        loop {
                            let Some(line) =
                                read_line(&mut tcp_stream, &mut buf, &mut buf_start, &mut buf_end)
                                    .await?
                            else {
                                return Ok(());
                            };
                            if line.as_slice() == b".\r\n" {
                                break;
                            }
                            // Undo transparency for a line starting with a dot.
                            if let Some(rest) = line.strip_prefix(b".") {
                                data.extend_from_slice(rest);
                            } else {
                                data.extend_from_slice(&line);
                            }
                        }
                        state.lock().unwrap().data(&data).unwrap();
                        let reply_code = state.lock().unwrap().data_end();
                        write_reply_code(&mut tcp_stream, reply_code).await?;
                    }
                    "BDAT" => {
                        if !authenticated {
                            write_reply_code(&mut tcp_stream, ReplyCode::_530).await?;
                            continue 'main;
                        }
                        let mut params = rest.split_whitespace();
                        let Some(len) = params.next().and_then(|len| len.parse::<usize>().ok())
                        else {
                            write_reply_code(&mut tcp_stream, ReplyCode::_501).await?;
                            continue 'main;
                        };
                        if params.next() != Some("LAST") {
                            write_reply_code(&mut tcp_stream, ReplyCode::_501).await?;
                            continue 'main;
                        }
                        state.lock().unwrap().bdat_lines.push(input.clone());
                        // Consume the announced payload exactly: it can contain
                        // any bytes, including a DATA terminator sequence.
                        let mut payload = Vec::with_capacity(len);
                        let available = buf_end - buf_start;
                        let take = available.min(len);
                        payload.extend_from_slice(&buf[buf_start..buf_start + take]);
                        buf_start += take;
                        if buf_start == buf_end {
                            buf_start = 0;
                            buf_end = 0;
                        }
                        while payload.len() < len {
                            let mut chunk = vec![0; len - payload.len()];
                            let read_bytes = tcp_stream.read(&mut chunk).await?;
                            if read_bytes == 0 {
                                return Ok(());
                            }
                            payload.extend_from_slice(&chunk[..read_bytes]);
                        }
                        let reply_code = state.lock().unwrap().bdat(&payload);
                        write_reply_code(&mut tcp_stream, reply_code).await?;
                    }
                    other => {
                        tracing::debug!("unexpected command: {other:?} {rest:?}");
                        write_reply_code(&mut tcp_stream, ReplyCode::_500).await?;
                    }
                }
            }
        }

        pub async fn serve(self) -> Result<()> {
            let Self {
                tcp_listener,
                event_receiver,
                state,
                addr,
            } = self;

            let mut event_receiver = Box::pin(event_receiver.into_future());
            let mut accept = Box::pin(tcp_listener.accept());
            loop {
                match future::select(accept.as_mut(), event_receiver.as_mut()).await {
                    Either::Left((request, _)) => {
                        accept = Box::pin(tcp_listener.accept());
                        let (tcp_stream, socket_addr) = request?;
                        if let Err(err) =
                            Self::handle_connection((tcp_stream, socket_addr), addr, state.clone())
                                .await
                        {
                            tracing::warn!("Failed connection: {err}");
                        }
                    }
                    Either::Right(((event, stream), _)) => {
                        *event_receiver = stream.into_future();
                        let Some(event) = event else {
                            continue;
                        };
                        match event {
                            ServerEvent::Quit => {
                                return Ok(());
                            }
                        }
                    }
                }
            }
        }
    }
}

pub mod tests {
    use std::thread;

    use futures::{channel::mpsc::unbounded, executor::block_on};
    use melib::{conf::Secret, email::Address, error::ErrorKind, smtp::*};

    use super::server::*;

    const NEW_MAIL: &str = r#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Cc:
Subject: RE: your e-mail
Message-ID: <h2g7f.z0gy2pgaen5m@example.com>
Content-Type: text/plain

hello world.
"#;

    fn no_extensions() -> SmtpExtensionSupport {
        SmtpExtensionSupport {
            pipelining: false,
            chunking: false,
            _8bitmime: false,
            prdr: false,
            binarymime: false,
            smtputf8: false,
            auth: false,
            dsn_notify: None,
        }
    }

    fn server_conf(server: &SmtpServer, auth: SmtpAuth) -> SmtpServerConf {
        SmtpServerConf {
            hostname: Secret::Value(server.addr.ip().to_string()),
            port: server.addr.port(),
            envelope_from: "user@example.com".into(),
            auth,
            security: SmtpSecurity::None,
            extensions: Default::default(),
            trace: true,
            timeout: 60,
        }
    }

    /// Shut the mock server down after every client connection has been
    /// dropped (the server accepts serially).
    fn shutdown(
        server_event_sender: futures::channel::mpsc::UnboundedSender<ServerEvent>,
        smtp_handle: thread::JoinHandle<melib::Result<()>>,
    ) {
        server_event_sender
            .unbounded_send(ServerEvent::Quit)
            .unwrap();
        smtp_handle.join().unwrap().unwrap();
    }

    /// Run a simple SMTP transaction without credential authentication.
    pub fn run_smtp_transaction() {
        melib_test::init_test_logging();
        let (server_event_sender, server_event_receiver) = unbounded();
        let server = SmtpServer::new(server_event_receiver, SmtpAuth::None, no_extensions());
        let server_state = server.state.clone();
        let smtp_server_conf = server_conf(&server, SmtpAuth::None);
        let smtp_handle = thread::spawn(move || block_on(server.serve()));
        {
            let mut connection =
                block_on(SmtpConnection::new_connection(smtp_server_conf.clone())).unwrap();
            block_on(connection.mail_transaction(
                NEW_MAIL,
                Some(&[
                    Address::new(None::<&str>, "user2@example.com"),
                    Address::new(None::<&str>, "postmaster@example.com"),
                ]),
            ))
            .unwrap();
            block_on(connection.quit()).unwrap();
            let stored = std::mem::take(&mut server_state.lock().unwrap().stored);
            assert_eq!(stored.len(), 2);
            assert_eq!(stored[0].0, "user2@example.com");
            assert_eq!(stored[1].0, "postmaster@example.com");
        }
        {
            let mut connection =
                block_on(SmtpConnection::new_connection(smtp_server_conf.clone())).unwrap();
            block_on(connection.mail_transaction(NEW_MAIL, None)).unwrap();
            block_on(connection.quit()).unwrap();
            let stored = std::mem::take(&mut server_state.lock().unwrap().stored);
            assert_eq!(stored.len(), 1);
            assert_eq!(stored[0].0, "myself@example.com");
        }
        let new_mail = r#"From: "some name" <some@example.com>
To: "first m. name" <myself@example.com>
Cc: "first m. name" <myself@example.com>, "Bj=?UTF-8?B?w7Y=?=rn" <bjorn@example.com>
Subject: RE: your e-mail
Message-ID: <h2g7f.z0gy2pgaen5m@example.com>
Content-Type: text/plain

hello world.
"#;
        {
            let mut connection =
                block_on(SmtpConnection::new_connection(smtp_server_conf)).unwrap();
            block_on(connection.mail_transaction(new_mail, None)).unwrap();
            block_on(connection.quit()).unwrap();
            let stored = std::mem::take(&mut server_state.lock().unwrap().stored);
            assert_eq!(stored.len(), 3);
            assert_eq!(stored[0].0, "myself@example.com");
            assert_eq!(stored[1].0, "myself@example.com");
            assert_eq!(stored[2].0, "bjorn@example.com");
        }
        shutdown(server_event_sender, smtp_handle);
    }

    /// Run an SMTP transaction where the mock server rejects one `RCPT TO` with
    /// a custom `550` reply text, and assert the propagated error names both
    /// the offending command (with the rejected address) and the numeric
    /// reply code.
    pub fn run_smtp_transaction_rcpt_rejected() {
        melib_test::init_test_logging();
        let (server_event_sender, server_event_receiver) = unbounded();
        let server = SmtpServer::new(server_event_receiver, SmtpAuth::None, no_extensions());
        server.state.lock().unwrap().rcpt_rejections = vec![(
            "invalid@example.com".to_string(),
            "Mailbox not found or access denied".to_string(),
        )];
        let smtp_server_conf = server_conf(&server, SmtpAuth::None);
        let smtp_handle = thread::spawn(move || block_on(server.serve()));
        let new_mail = r#"From: "some name" <some@example.com>
To: "me" <invalid@example.com>
Subject: RE: your e-mail
Message-ID: <h2g7f.z0gy2pgaen5m@example.com>
Content-Type: text/plain

hello world.
"#;
        let mut connection = block_on(SmtpConnection::new_connection(smtp_server_conf)).unwrap();
        let err = block_on(connection.mail_transaction(
            new_mail,
            Some(&[Address::new(None::<&str>, "invalid@example.com")]),
        ))
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("RCPT TO:<invalid@example.com>"),
            "error should name the rejected recipient: {err}"
        );
        assert!(
            err.contains("550"),
            "error should contain the numeric reply code: {err}"
        );
        assert!(
            err.contains("Mailbox not found or access denied"),
            "error should contain the server reply text: {err}"
        );
        // The transaction failed mid-way; QUIT lets the mock server thread exit.
        block_on(connection.quit()).unwrap();
        drop(connection);
        shutdown(server_event_sender, smtp_handle);
    }

    /// Pipelining: the success path delivers to both recipients, and a `RCPT`
    /// rejected mid-pipeline is still surfaced as an error before any message
    /// data is stored.
    pub fn run_smtp_pipelining() {
        melib_test::init_test_logging();
        let (server_event_sender, server_event_receiver) = unbounded();
        let extensions = SmtpExtensionSupport {
            pipelining: true,
            ..no_extensions()
        };
        let server = SmtpServer::new(server_event_receiver, SmtpAuth::None, extensions);
        let server_state = server.state.clone();
        let smtp_server_conf = server_conf(&server, SmtpAuth::None);
        let smtp_handle = thread::spawn(move || block_on(server.serve()));
        let tos = [
            Address::new(None::<&str>, "user2@example.com"),
            Address::new(None::<&str>, "postmaster@example.com"),
        ];
        {
            let mut connection =
                block_on(SmtpConnection::new_connection(smtp_server_conf.clone())).unwrap();
            block_on(connection.mail_transaction(NEW_MAIL, Some(&tos))).unwrap();
            block_on(connection.quit()).unwrap();
            let stored = std::mem::take(&mut server_state.lock().unwrap().stored);
            assert_eq!(stored.len(), 2);
            assert_eq!(stored[0].0, "user2@example.com");
            assert_eq!(stored[1].0, "postmaster@example.com");
        }
        {
            server_state.lock().unwrap().rcpt_rejections.push((
                "user2@example.com".to_string(),
                "Mailbox not found".to_string(),
            ));
            let mut connection =
                block_on(SmtpConnection::new_connection(smtp_server_conf.clone())).unwrap();
            let err = block_on(connection.mail_transaction(NEW_MAIL, Some(&tos))).unwrap_err();
            assert!(
                err.details.as_ref().unwrap().starts_with("SMTP 550 error"),
                "{err:?}"
            );
            block_on(connection.quit()).unwrap();
            let mut server_state = server_state.lock().unwrap();
            let stored = std::mem::take(&mut server_state.stored);
            assert_eq!(stored.len(), 0);
            server_state.clear();
        }
        {
            server_state.lock().unwrap().rcpt_rejections.push((
                "postmaster@example.com".to_string(),
                "Mailbox not found".to_string(),
            ));
            let mut connection =
                block_on(SmtpConnection::new_connection(smtp_server_conf.clone())).unwrap();
            let err = block_on(connection.mail_transaction(NEW_MAIL, Some(&tos))).unwrap_err();
            assert!(
                err.details.as_ref().unwrap().starts_with("SMTP 550 error"),
                "{err:?}"
            );
            block_on(connection.quit()).unwrap();
            let mut server_state = server_state.lock().unwrap();
            let stored = std::mem::take(&mut server_state.stored);
            assert_eq!(stored.len(), 0);
            server_state.clear();
        }
        shutdown(server_event_sender, smtp_handle);
    }

    /// Authentication negotiation: 530 when the server requires auth, the
    /// LOGIN and PLAIN successes, the LOGIN failure, the "no mechanism"
    /// negotiation error, and OAUTHBEARER success/failure.
    pub fn run_smtp_auth_methods() {
        melib_test::init_test_logging();
        let (server_event_sender, server_event_receiver) = unbounded();
        let server = SmtpServer::new(
            server_event_receiver,
            SmtpAuth::Auto {
                username: Secret::Value("username".into()),
                password: Secret::Value("password".into()),
                require_auth: true,
                auth_type: SmtpAuthType::default(),
            },
            SmtpExtensionSupport {
                auth: true,
                ..no_extensions()
            },
        );
        let hostname = server.addr.ip().to_string();
        let server_state = server.state.clone();
        let base_conf = server_conf(&server, SmtpAuth::None);
        let smtp_handle = thread::spawn(move || block_on(server.serve()));
        let tos = [
            Address::new(None::<&str>, "user2@example.com"),
            Address::new(None::<&str>, "postmaster@example.com"),
        ];
        // a. The server requires authentication but the client configured none.
        {
            let mut connection =
                block_on(SmtpConnection::new_connection(base_conf.clone())).unwrap();
            let auth_err = block_on(connection.mail_transaction(NEW_MAIL, Some(&tos))).unwrap_err();
            drop(connection);
            assert!(
                auth_err
                    .details
                    .as_ref()
                    .unwrap()
                    .starts_with("SMTP 530 error"),
                "{auth_err:?}"
            );
            assert_eq!(auth_err.kind, ErrorKind::Authentication);
        }
        let auto_conf = SmtpServerConf {
            auth: SmtpAuth::Auto {
                username: Secret::Value("username".into()),
                password: Secret::Value("password".into()),
                require_auth: true,
                auth_type: SmtpAuthType::default(),
            },
            ..base_conf.clone()
        };
        // b. LOGIN path: only AUTH LOGIN is advertised.
        server_state.lock().unwrap().logindisabled = false;
        server_state.lock().unwrap().plaindisabled = true;
        {
            let mut connection =
                block_on(SmtpConnection::new_connection(auto_conf.clone())).unwrap();
            block_on(connection.mail_transaction(NEW_MAIL, Some(&tos))).unwrap();
            block_on(connection.quit()).unwrap();
            let stored = std::mem::take(&mut server_state.lock().unwrap().stored);
            assert_eq!(stored.len(), 2);
            assert_eq!(stored[0].0, "user2@example.com");
            assert_eq!(stored[1].0, "postmaster@example.com");
            assert_eq!(
                server_state.lock().unwrap().auth_lines,
                vec![
                    format!("{}\r\n", base64_encode("username")),
                    format!("{}\r\n", base64_encode("password")),
                ],
                "AUTH LOGIN credentials must be written raw, without a command wrapper"
            );
        }
        // c. LOGIN failure: the wrong password must not leak into the error.
        let wrong_conf = SmtpServerConf {
            auth: SmtpAuth::Auto {
                username: Secret::Value("username".into()),
                password: Secret::Value("wr0ng-p4ss".into()),
                require_auth: true,
                auth_type: SmtpAuthType::default(),
            },
            ..base_conf.clone()
        };
        {
            let auth_err = match block_on(SmtpConnection::new_connection(wrong_conf)) {
                Ok(mut connection) => {
                    let err =
                        block_on(connection.mail_transaction(NEW_MAIL, Some(&tos))).unwrap_err();
                    drop(connection);
                    err
                }
                Err(err) => err,
            };
            assert_eq!(auth_err.kind, ErrorKind::Authentication, "{auth_err:?}");
            assert!(
                !auth_err.to_string().contains("wr0ng-p4ss"),
                "the password must never appear in an error: {auth_err}"
            );
        }
        // d. PLAIN path: only AUTH PLAIN is advertised.
        server_state.lock().unwrap().logindisabled = true;
        server_state.lock().unwrap().plaindisabled = false;
        {
            let mut connection =
                block_on(SmtpConnection::new_connection(auto_conf.clone())).unwrap();
            block_on(connection.mail_transaction(NEW_MAIL, Some(&tos))).unwrap();
            block_on(connection.quit()).unwrap();
            let stored = std::mem::take(&mut server_state.lock().unwrap().stored);
            assert_eq!(stored.len(), 2);
        }
        // e. Neither PLAIN nor LOGIN is advertised.
        server_state.lock().unwrap().logindisabled = true;
        server_state.lock().unwrap().plaindisabled = true;
        {
            let auth_err = block_on(SmtpConnection::new_connection(auto_conf.clone())).unwrap_err();
            assert_eq!(
                auth_err.summary.as_ref(),
                format!(
                    "SMTP Server {hostname} doesn't advertise PLAIN/LOGIN Authentication support"
                )
                .as_str()
            );
            assert_eq!(auth_err.kind, ErrorKind::Authentication);
        }
        // f. OAUTHBEARER success.
        server_state.lock().unwrap().oauthbearerdisabled = false;
        let oauthbearer_conf = SmtpServerConf {
            auth: SmtpAuth::OAuthBearer {
                token: Secret::Value(OAUTHBEARER_TOKEN.into()),
                require_auth: true,
            },
            ..base_conf.clone()
        };
        {
            let mut connection =
                block_on(SmtpConnection::new_connection(oauthbearer_conf.clone())).unwrap();
            block_on(connection.mail_transaction(NEW_MAIL, Some(&tos))).unwrap();
            block_on(connection.quit()).unwrap();
            let stored = std::mem::take(&mut server_state.lock().unwrap().stored);
            assert_eq!(stored.len(), 2);
            assert_eq!(stored[0].0, "user2@example.com");
            assert_eq!(stored[1].0, "postmaster@example.com");
        }
        // g. OAUTHBEARER not advertised: fail without leaking the token.
        server_state.lock().unwrap().oauthbearerdisabled = true;
        {
            let auth_err = block_on(SmtpConnection::new_connection(oauthbearer_conf)).unwrap_err();
            assert!(
                auth_err
                    .summary
                    .contains("doesn't advertise OAUTHBEARER Authentication support"),
                "{auth_err:?}"
            );
            assert_eq!(auth_err.kind, ErrorKind::Authentication);
            let rendered = auth_err.to_string();
            assert!(
                !rendered.contains(OAUTHBEARER_TOKEN),
                "the raw token must never appear in an error: {rendered}"
            );
            assert!(
                !rendered.contains(&base64_encode(OAUTHBEARER_TOKEN)),
                "the encoded token must never appear in an error: {rendered}"
            );
        }
        shutdown(server_event_sender, smtp_handle);
    }

    /// BINARYMIME/BDAT: the wire command must be `BDAT <len> LAST` (with the
    /// spaces), the payload length must be exact, and the pipelined envelope
    /// replies must be drained after the payload is sent.
    pub fn run_smtp_bdat_wire_format() {
        melib_test::init_test_logging();
        let (server_event_sender, server_event_receiver) = unbounded();
        let extensions = SmtpExtensionSupport {
            pipelining: true,
            chunking: true,
            binarymime: true,
            ..no_extensions()
        };
        let server = SmtpServer::new(server_event_receiver, SmtpAuth::None, extensions);
        let server_state = server.state.clone();
        let smtp_server_conf = server_conf(&server, SmtpAuth::None);
        let smtp_handle = thread::spawn(move || block_on(server.serve()));
        let tos = [
            Address::new(None::<&str>, "user2@example.com"),
            Address::new(None::<&str>, "postmaster@example.com"),
        ];
        {
            let mut connection =
                block_on(SmtpConnection::new_connection(smtp_server_conf)).unwrap();
            block_on(connection.mail_transaction(NEW_MAIL, Some(&tos))).unwrap();
            block_on(connection.quit()).unwrap();
        }
        let mut server_state = server_state.lock().unwrap();
        assert_eq!(
            server_state.bdat_lines.len(),
            1,
            "exactly one BDAT command expected: {:?}",
            server_state.bdat_lines
        );
        assert_eq!(
            server_state.bdat_lines[0].trim_end_matches("\r\n"),
            format!("BDAT {} LAST", NEW_MAIL.len()),
            "BDAT wire format must separate the command, length and LAST with spaces"
        );
        let stored = std::mem::take(&mut server_state.stored);
        assert_eq!(stored.len(), 2);
        assert_eq!(stored[0].0, "user2@example.com");
        assert_eq!(stored[1].0, "postmaster@example.com");
        drop(server_state);
        shutdown(server_event_sender, smtp_handle);
    }

    /// A non-compliant server advertising BINARYMIME without CHUNKING must not
    /// make the client use BDAT or `BODY=BINARYMIME`; binarymime is reset and
    /// the transaction uses DATA.
    pub fn run_smtp_binarymime_requires_chunking() {
        melib_test::init_test_logging();
        let (server_event_sender, server_event_receiver) = unbounded();
        let extensions = SmtpExtensionSupport {
            chunking: false,
            binarymime: true,
            ..no_extensions()
        };
        let server = SmtpServer::new(server_event_receiver, SmtpAuth::None, extensions);
        let server_state = server.state.clone();
        let smtp_server_conf = server_conf(&server, SmtpAuth::None);
        let smtp_handle = thread::spawn(move || block_on(server.serve()));
        let tos = [Address::new(None::<&str>, "user2@example.com")];
        {
            let mut connection =
                block_on(SmtpConnection::new_connection(smtp_server_conf)).unwrap();
            block_on(connection.mail_transaction(NEW_MAIL, Some(&tos))).unwrap();
            block_on(connection.quit()).unwrap();
        }
        let mut server_state = server_state.lock().unwrap();
        assert!(
            server_state.bdat_lines.is_empty(),
            "BDAT must not be used without CHUNKING: {:?}",
            server_state.bdat_lines
        );
        assert!(
            server_state
                .mail_from_lines
                .iter()
                .all(|line| !line.contains("BODY=BINARYMIME")),
            "BODY=BINARYMIME must be reset without CHUNKING: {:?}",
            server_state.mail_from_lines
        );
        let stored = std::mem::take(&mut server_state.stored);
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].0, "user2@example.com");
        drop(server_state);
        shutdown(server_event_sender, smtp_handle);
    }

    /// `251 User not local; will forward` is an accepted `RCPT TO` reply, with
    /// and without pipelining.
    pub fn run_smtp_rcpt_251_accepted() {
        melib_test::init_test_logging();
        for pipelining in [false, true] {
            let (server_event_sender, server_event_receiver) = unbounded();
            let extensions = SmtpExtensionSupport {
                pipelining,
                ..no_extensions()
            };
            let server = SmtpServer::new(server_event_receiver, SmtpAuth::None, extensions);
            server
                .state
                .lock()
                .unwrap()
                .rcpt_forward
                .push("forwarded@example.com".to_string());
            let server_state = server.state.clone();
            let smtp_server_conf = server_conf(&server, SmtpAuth::None);
            let smtp_handle = thread::spawn(move || block_on(server.serve()));
            {
                let mut connection =
                    block_on(SmtpConnection::new_connection(smtp_server_conf)).unwrap();
                block_on(connection.mail_transaction(
                    NEW_MAIL,
                    Some(&[
                        Address::new(None::<&str>, "user2@example.com"),
                        Address::new(None::<&str>, "forwarded@example.com"),
                    ]),
                ))
                .unwrap();
                block_on(connection.quit()).unwrap();
            }
            let mut server_state = server_state.lock().unwrap();
            let stored = std::mem::take(&mut server_state.stored);
            assert_eq!(
                stored.len(),
                2,
                "pipelining={pipelining}: both recipients must be stored"
            );
            assert!(
                stored.iter().any(|(to, _)| to == "forwarded@example.com"),
                "pipelining={pipelining}: the forwarded recipient must be stored"
            );
            drop(server_state);
            shutdown(server_event_sender, smtp_handle);
        }
    }

    fn base64_encode(value: &str) -> String {
        use base64::{engine::general_purpose::STANDARD as base64, Engine as _};
        base64.encode(value)
    }
}
