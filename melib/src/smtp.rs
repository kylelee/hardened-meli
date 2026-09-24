/*
 * meli - smtp
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
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with meli. If not, see <http://www.gnu.org/licenses/>.
 */

#![allow(clippy::just_underscores_and_digits)]
#![allow(clippy::needless_lifetimes)]

//! SMTP client support
//!
//! This module implements a client for the SMTP protocol as specified by [RFC
//! 5321 Simple Mail Transfer Protocol](https://www.rfc-editor.org/rfc/rfc5321).
//!
//! The connection and methods are `async` and uses the `smol` runtime.
//!# Example
//!
//! ```no_run
//! extern crate melib;
//!
//! use melib::{conf::Secret, email::Address, futures, smol, smtp::*, Result};
//! let conf = SmtpServerConf {
//!     hostname: Secret::Value("smtp.example.com".into()),
//!     port: 587,
//!     security: SmtpSecurity::StartTLS {
//!         danger_accept_invalid_certs: false,
//!     },
//!     envelope_from: String::new(),
//!     extensions: SmtpExtensionSupport::default(),
//!     auth: SmtpAuth::Auto {
//!         username: Secret::Value("l15".into()),
//!         password: Secret::Evaluate {
//!             command: "gpg2 --no-tty -q -d ~/.passwords/mail.gpg".into(),
//!             store_in_memory: false,
//!         },
//!         require_auth: true,
//!         auth_type: SmtpAuthType::default(),
//!     },
//! };
//!
//! std::thread::Builder::new()
//!     .spawn(move || {
//!         let ex = smol::Executor::new();
//!         futures::executor::block_on(ex.run(futures::future::pending::<()>()));
//!     })
//!     .unwrap();
//!
//! let mut conn = futures::executor::block_on(SmtpConnection::new_connection(conf)).unwrap();
//! futures::executor::block_on(conn.mail_transaction(
//!     r#"To: l10@example.com
//! Subject: Fwd: SMTP TEST
//! From: Me <l15@example.com>
//! Message-Id: <E1hSjnr-0003fN-RL@example.com>
//! Date: Mon, 13 Jul 2020 09:02:15 +0300
//!
//! Prescriptions-R-X"#,
//!     Some(&[Address::try_from("foo-chat@example.com").unwrap()]),
//! ))
//! .unwrap();
//! futures::executor::block_on(conn.quit()).unwrap();
//! ```

use std::{borrow::Cow, convert::TryFrom};

use futures::io::{AsyncReadExt, AsyncWriteExt};
use native_tls::TlsConnector;
use smallvec::SmallVec;
use smol::{unblock, Async as AsyncWrapper};

use crate::{
    conf::Secret,
    email::{Address, Envelope},
    error::{Error, ErrorKind, Result, ResultIntoError},
    utils::{
        connections::{
            enforce_response_size_limit, std_net::connect as tcp_stream_connect, Connection,
        },
        futures::timeout,
    },
};

/// Per-read timeout for reading SMTP server replies.
///
/// IMAP and NNTP read this value from the per-account `timeout` setting
/// (16 seconds by default); the SMTP client has no such setting, so it is
/// hardcoded here to the same default. Note that this does not defend
/// against a server that trickles bytes (each read succeeds before the
/// timeout); that is what the response size cap is for.
#[cfg(not(test))]
const SMTP_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(16);

/// Shortened in test builds so the timeout can be exercised quickly.
#[cfg(test)]
const SMTP_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(300);

/// Kind of server security (StartTLS/TLS/None) the client should attempt
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type")]
pub enum SmtpSecurity {
    #[serde(alias = "starttls", alias = "STARTTLS")]
    StartTLS {
        #[serde(default = "crate::conf::false_val")]
        danger_accept_invalid_certs: bool,
    },
    #[serde(alias = "auto")]
    Auto {
        #[serde(default = "crate::conf::false_val")]
        danger_accept_invalid_certs: bool,
    },
    #[serde(alias = "tls", alias = "TLS")]
    Tls {
        #[serde(default = "crate::conf::false_val")]
        danger_accept_invalid_certs: bool,
    },
    #[serde(alias = "none")]
    None,
}

impl Default for SmtpSecurity {
    fn default() -> Self {
        Self::Auto {
            danger_accept_invalid_certs: false,
        }
    }
}

/// Kind of server authentication the client should attempt
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type")]
pub enum SmtpAuth {
    #[serde(alias = "none")]
    None,
    #[serde(alias = "auto")]
    Auto {
        username: Secret,
        password: Secret,
        #[serde(default = "crate::conf::true_val")]
        require_auth: bool,
        #[serde(skip_serializing, skip_deserializing, default)]
        auth_type: SmtpAuthType,
    },
    #[serde(alias = "xoauth2")]
    XOAuth2 {
        token: Secret,
        #[serde(default = "crate::conf::true_val")]
        require_auth: bool,
    },
    // md5, sasl, etc
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SmtpAuthType {
    plain: bool,
    login: bool,
}

impl SmtpAuth {
    pub const fn require_auth(&self) -> bool {
        use SmtpAuth::*;
        match self {
            None => false,
            Auto { require_auth, .. } | XOAuth2 { require_auth, .. } => *require_auth,
        }
    }
}

/// Server configuration for connecting the SMTP client
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SmtpServerConf {
    pub hostname: Secret,
    pub port: u16,
    #[serde(default)]
    pub envelope_from: String,
    pub auth: SmtpAuth,
    #[serde(default)]
    pub security: SmtpSecurity,
    #[serde(default)]
    pub extensions: SmtpExtensionSupport,
}

//example: "SIZE 52428800", "8BITMIME", "PIPELINING", "CHUNKING", "PRDR",
/// Configured SMTP extensions to use
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SmtpExtensionSupport {
    #[serde(default = "crate::conf::true_val")]
    pipelining: bool,
    #[serde(default = "crate::conf::true_val")]
    chunking: bool,
    /// [RFC 6152: SMTP Service Extension for 8-bit MIME Transport](https://www.rfc-editor.org/rfc/rfc6152)
    #[serde(alias = "8bitmime", default = "crate::conf::true_val")]
    _8bitmime: bool,
    /// Essentially, the PRDR extension to SMTP allows (but does not require) an
    /// SMTP server to issue multiple responses after a message has been
    /// transferred, by mutual consent of the client and server. SMTP
    /// clients that support the PRDR extension then use the expanded
    /// responses as supplemental data to the responses that were received
    /// during the earlier envelope exchange.
    #[serde(default = "crate::conf::true_val")]
    prdr: bool,
    #[serde(default = "crate::conf::true_val")]
    binarymime: bool,
    /// Resources:
    /// - <http://www.postfix.org/SMTPUTF8_README.html>
    #[serde(default = "crate::conf::true_val")]
    smtputf8: bool,
    #[serde(default = "crate::conf::true_val")]
    auth: bool,
    #[serde(default = "default_dsn")]
    dsn_notify: Option<Cow<'static, str>>,
}

fn default_dsn() -> Option<Cow<'static, str>> {
    Some("FAILURE".into())
}

impl Default for SmtpExtensionSupport {
    fn default() -> Self {
        Self {
            pipelining: true,
            chunking: true,
            prdr: true,
            _8bitmime: true,
            binarymime: true,
            smtputf8: true,
            auth: true,
            dsn_notify: Some("FAILURE".into()),
        }
    }
}

/// SMTP client session object.
///
/// See module-wide documentation.
#[derive(Debug)]
pub struct SmtpConnection {
    stream: AsyncWrapper<Connection>,
    read_buffer: String,
    server_conf: SmtpServerConf,
}

impl SmtpConnection {
    /// Performs connection and if configured: TLS negotiation and SMTP AUTH
    pub async fn new_connection(mut server_conf: SmtpServerConf) -> Result<Self> {
        let path = server_conf
            .hostname
            .value_with_timeout(std::time::Duration::new(4, 0))
            .await
            .chain_err_summary(|| "hostname")?;
        let mut res = String::with_capacity(8 * 1024);
        let stream = match server_conf.security {
            SmtpSecurity::Auto {
                danger_accept_invalid_certs,
            }
            | SmtpSecurity::Tls {
                danger_accept_invalid_certs,
            }
            | SmtpSecurity::StartTLS {
                danger_accept_invalid_certs,
            } => {
                let mut connector = TlsConnector::builder();
                if danger_accept_invalid_certs {
                    connector.danger_accept_invalid_certs(true);
                }
                let connector = connector.build()?;

                let addr = (path.as_str(), server_conf.port);
                let mut socket = {
                    let conn = Connection::new_tcp(tcp_stream_connect(
                        addr,
                        Some(std::time::Duration::new(4, 0)),
                    )?);
                    #[cfg(feature = "smtp-trace")]
                    let conn = conn.trace(true).with_id("smtp");

                    AsyncWrapper::new(conn)?
                };
                if matches!(server_conf.security, SmtpSecurity::Auto { .. }) {
                    if server_conf.port == 465 {
                        server_conf.security = SmtpSecurity::Tls {
                            danger_accept_invalid_certs,
                        };
                    } else if server_conf.port == 587 {
                        server_conf.security = SmtpSecurity::StartTLS {
                            danger_accept_invalid_certs,
                        };
                    } else {
                        return Err(Error::new(
                            "Please specify what SMTP security transport to use explicitly \
                             instead of `auto`.",
                        ));
                    }
                }

                if !matches!(server_conf.security, SmtpSecurity::Tls { .. }) {
                    let pre_ehlo_extensions_reply = read_lines(
                        &mut socket,
                        &mut res,
                        Some((ReplyCode::_220, &[])),
                        &mut String::new(),
                    )
                    .await?;
                    drop(pre_ehlo_extensions_reply);
                }
                if !matches!(server_conf.security, SmtpSecurity::Tls { .. }) {
                    socket.write_all(b"EHLO meli-email.org\r\n").await?;
                }
                if matches!(server_conf.security, SmtpSecurity::StartTLS { .. }) {
                    let pre_tls_extensions_reply = read_lines(
                        &mut socket,
                        &mut res,
                        Some((ReplyCode::_250, &[])),
                        &mut String::new(),
                    )
                    .await?;
                    drop(pre_tls_extensions_reply);
                    socket.write_all(b"STARTTLS\r\n").await?;
                    let _post_starttls_extensions_reply = read_lines(
                        &mut socket,
                        &mut res,
                        Some((ReplyCode::_220, &[])),
                        &mut String::new(),
                    )
                    .await?;
                }

                let mut ret = {
                    let socket = socket.into_inner()?;
                    #[cfg(feature = "smtp-trace")]
                    let socket = socket.trace(false);
                    let _path = path.clone();

                    socket.set_nonblocking(false)?;
                    let conn = unblock(move || connector.connect(&_path, socket)).await?;
                    AsyncWrapper::new({
                        let conn = Connection::new_tls(conn);
                        #[cfg(feature = "smtp-trace")]
                        {
                            conn.trace(true).with_id("smtp")
                        }
                        #[cfg(not(feature = "smtp-trace"))]
                        {
                            conn
                        }
                    })?
                };
                if matches!(server_conf.security, SmtpSecurity::Tls { .. }) {
                    let pre_ehlo_extensions_reply = read_lines(
                        &mut ret,
                        &mut res,
                        Some((ReplyCode::_220, &[])),
                        &mut String::new(),
                    )
                    .await?;
                    drop(pre_ehlo_extensions_reply);
                }
                ret.write_all(b"EHLO meli-email.org\r\n").await?;
                ret
            }
            SmtpSecurity::None => {
                let addr = (path.as_str(), server_conf.port);
                let mut ret = AsyncWrapper::new({
                    let conn = Connection::new_tcp(tcp_stream_connect(
                        addr,
                        Some(std::time::Duration::new(4, 0)),
                    )?);
                    #[cfg(feature = "smtp-trace")]
                    {
                        conn.trace(true).with_id("smtp")
                    }
                    #[cfg(not(feature = "smtp-trace"))]
                    {
                        conn
                    }
                })?;
                res.clear();
                let reply = read_lines(
                    &mut ret,
                    &mut res,
                    Some((ReplyCode::_220, &[])),
                    &mut String::new(),
                )
                .await?;
                let code = reply.code;
                let result: Result<ReplyCode> = reply.into();
                result?;
                if code != ReplyCode::_220 {
                    return Err(Error::new(format!(
                        "SMTP Server didn't reply with a 220 greeting: {:?}",
                        Reply::new(&res, code)
                    )));
                }
                ret.write_all(b"EHLO meli-email.org\r\n").await?;
                ret
            }
        };
        let mut ret = Self {
            stream,
            read_buffer: String::new(),
            server_conf: server_conf.clone(),
        };
        let no_auth_needed: bool;
        {
            let pre_auth_extensions_reply = ret
                .read_lines(&mut res, Some((ReplyCode::_250, &[])))
                .await?;
            if ret.server_conf.auth != SmtpAuth::None
                && ret.server_conf.auth.require_auth()
                && !pre_auth_extensions_reply
                    .lines
                    .iter()
                    .any(|l| l.starts_with("AUTH"))
            {
                return Err(Error::new(format!(
                    "SMTP Server doesn't advertise Authentication support. Server response was: \
                     {pre_auth_extensions_reply:?}"
                ))
                .set_kind(ErrorKind::Authentication));
            }
            no_auth_needed =
                ret.server_conf.auth == SmtpAuth::None || !ret.server_conf.auth.require_auth();
            if no_auth_needed {
                ret.set_extension_support(pre_auth_extensions_reply);
            } else if let SmtpAuth::Auto {
                ref mut auth_type, ..
            } = ret.server_conf.auth
            {
                if let Some(l) = pre_auth_extensions_reply
                    .lines
                    .iter()
                    .find(|l| l.starts_with("AUTH"))
                {
                    // The server may advertise bare `AUTH` with no mechanism
                    // list (or an `AUTH`-prefixed line shorter than
                    // `"AUTH "`); `.get()` yields no mechanisms instead of
                    // panicking on the out-of-bounds slice.
                    for _type in parse_auth_mechanisms(l) {
                        if _type == "PLAIN" {
                            auth_type.plain = true;
                        } else if _type == "LOGIN" {
                            auth_type.login = true;
                        }
                    }
                }
            }
        }
        if !no_auth_needed {
            match &ret.server_conf.auth {
                SmtpAuth::None => {}
                SmtpAuth::Auto {
                    username,
                    password,
                    auth_type,
                    ..
                } => {
                    let username = username
                        .value_with_timeout(std::time::Duration::new(4, 0))
                        .await
                        .chain_err_summary(|| "username")?;
                    let password = password
                        .value_with_timeout(std::time::Duration::new(4, 0))
                        .await
                        .chain_err_summary(|| "password")?;
                    if auth_type.login {
                        ret.send_command(&[b"AUTH LOGIN"]).await?;
                        ret.read_lines(&mut res, Some((ReplyCode::_334, &[])))
                            .await
                            .chain_err_kind(ErrorKind::Authentication)?;
                        #[allow(deprecated)]
                        let buf = base64::encode(&username);
                        ret.send_command(&[buf.as_bytes()]).await?;
                        ret.read_lines(&mut res, Some((ReplyCode::_334, &[])))
                            .await
                            .chain_err_kind(ErrorKind::Authentication)?;
                        #[allow(deprecated)]
                        let buf = base64::encode(&password);
                        ret.send_command(&[buf.as_bytes()]).await?;
                    } else {
                        // # RFC 4616 The PLAIN SASL Mechanism
                        // # https://www.ietf.org/rfc/rfc4616.txt
                        // message   = [authzid] UTF8NUL authcid UTF8NUL passwd
                        // authcid   = 1*SAFE ; MUST accept up to 255 octets
                        // authzid   = 1*SAFE ; MUST accept up to 255 octets
                        // passwd    = 1*SAFE ; MUST accept up to 255 octets
                        // UTF8NUL   = %x00 ; UTF-8 encoded NUL character
                        let username_password = {
                            let mut buf = Vec::with_capacity(2 + username.len() + password.len());
                            buf.push(b'\0');
                            buf.extend(username.as_bytes().to_vec());
                            buf.push(b'\0');
                            buf.extend(password.as_bytes());
                            #[allow(deprecated)]
                            base64::encode(buf)
                        };
                        ret.send_command(&[b"AUTH PLAIN ", username_password.as_bytes()])
                            .await?;
                    }
                    ret.read_lines(&mut res, Some((ReplyCode::_235, &[])))
                        .await
                        .chain_err_kind(ErrorKind::Authentication)?;
                    ret.send_command(&[b"EHLO meli-email.org"]).await?;
                }
                SmtpAuth::XOAuth2 { token, .. } => {
                    let password_token = token
                        .value_with_timeout(std::time::Duration::new(4, 0))
                        .await
                        .chain_err_summary(|| "SMTP XOAUTH2 token")?;
                    // https://developers.google.com/gmail/imap/xoauth2-protocol#smtp_protocol_exchange
                    ret.send_command(&[b"AUTH XOAUTH2 ", password_token.as_bytes()])
                        .await?;
                    ret.read_lines(&mut res, Some((ReplyCode::_235, &[])))
                        .await
                        .chain_err_kind(ErrorKind::Authentication)?;
                    ret.send_command(&[b"EHLO meli-email.org"]).await?;
                }
            }
            {
                let extensions_reply = ret
                    .read_lines(&mut res, Some((ReplyCode::_250, &[])))
                    .await?;
                ret.set_extension_support(extensions_reply);
            }
        }
        Ok(ret)
    }

    /// Set a new value for `envelope_from`.
    pub fn set_envelope_from(&mut self, envelope_from: String) {
        self.server_conf.envelope_from = envelope_from;
    }

    fn set_extension_support(&mut self, reply: Reply) {
        debug_assert_eq!(reply.code, ReplyCode::_250);
        self.server_conf.extensions.pipelining &= reply.lines.contains(&"PIPELINING");
        self.server_conf.extensions.chunking &= reply.lines.contains(&"CHUNKING");
        self.server_conf.extensions.prdr &= reply.lines.contains(&"PRDR");
        self.server_conf.extensions._8bitmime &= reply.lines.contains(&"8BITMIME");
        self.server_conf.extensions.binarymime &= reply.lines.contains(&"BINARYMIME");
        self.server_conf.extensions.smtputf8 &= reply.lines.contains(&"SMTPUTF8");
        if !reply.lines.contains(&"DSN") {
            self.server_conf.extensions.dsn_notify = None;
        }
    }

    pub async fn read_lines<'r>(
        &mut self,
        ret: &'r mut String,
        expected_reply_code: Option<(ReplyCode, &[ReplyCode])>,
    ) -> Result<Reply<'r>> {
        read_lines(
            &mut self.stream,
            ret,
            expected_reply_code,
            &mut self.read_buffer,
        )
        .await
    }

    pub async fn send_command(&mut self, command: &[&[u8]]) -> Result<()> {
        for c in command {
            self.stream.write_all(c).await?;
        }
        self.stream.write_all(b"\r\n").await?;
        Ok(())
    }

    /// Sends mail
    pub async fn mail_transaction(&mut self, mail: &str, tos: Option<&[Address]>) -> Result<()> {
        let mut res = String::with_capacity(8 * 1024);
        let mut pipelining_queue: SmallVec<[ExpectedReplyCode; 16]> = SmallVec::new();
        let mut pipelining_results: SmallVec<[Result<ReplyCode>; 16]> = SmallVec::new();
        let mut prdr_results: SmallVec<[Result<ReplyCode>; 16]> = SmallVec::new();
        let dsn_notify = self.server_conf.extensions.dsn_notify.clone();
        let envelope_from = self.server_conf.envelope_from.clone();
        let envelope = Envelope::from_bytes(mail.as_bytes(), None)
            .chain_err_summary(|| "SMTP submission was aborted")?;
        let tos = tos.unwrap_or_else(|| envelope.to());
        if tos.is_empty() && envelope.cc().is_empty() && envelope.bcc().is_empty() {
            return Err(Error::new(
                "SMTP submission was aborted because there was no e-mail address found in the To: \
                 header field. Consider adding recipients.",
            ));
        }
        let mut current_command: SmallVec<[&[u8]; 16]> = SmallVec::new();
        //first step in the procedure is the MAIL command.
        // `MAIL FROM:<reverse-path> [SP <mail-parameters> ] <CRLF>`
        current_command.push(b"MAIL FROM:<");
        if !envelope_from.is_empty() {
            current_command.push(envelope_from.trim().as_bytes());
        } else {
            if envelope.from().is_empty() || envelope.from()[0].get_email().is_empty() {
                return Err(Error::new(
                    "SMTP submission was aborted because there was no e-mail address found in the \
                     From: header field. Consider adding a valid value or setting `envelope_from` \
                     in SMTP client settings",
                ));
            } else if envelope.from().len() != 1 {
                return Err(Error::new(
                    "SMTP submission was aborted because there was more than one e-mail address \
                     found in the From: header field. Consider setting `envelope_from` in SMTP \
                     client settings",
                ));
            }
            current_command.push(envelope.from()[0].get_email().trim().as_bytes());
        }
        current_command.push(b">");
        if self.server_conf.extensions.prdr {
            current_command.push(b" PRDR");
        }
        if self.server_conf.extensions.binarymime {
            current_command.push(b" BODY=BINARYMIME");
        } else if self.server_conf.extensions._8bitmime {
            current_command.push(b" BODY=8BITMIME");
        }
        self.send_command(&current_command).await?;
        current_command.clear();
        if !self.server_conf.extensions.pipelining {
            self.read_lines(&mut res, Some((ReplyCode::_250, &[])))
                .await?;
        } else {
            pipelining_queue.push(Some((ReplyCode::_250, &[])));
        }
        //The second step in the procedure is the RCPT command. This step of the
        // procedure can be repeated any number of times. If accepted, the SMTP
        // server returns a "250 OK" reply. If the mailbox specification is not
        // acceptable for some reason, the server MUST return a reply indicating
        // whether the failure is permanent (i.e., will occur again if
        // the client tries to send the same address again) or temporary (i.e., the
        // address might be accepted if the client tries again later).
        for addr in tos
            .iter()
            .chain(envelope.cc().iter())
            .chain(envelope.bcc().iter())
        {
            macro_rules! send_to_mailbox {
                ($mailbox:expr) => {{
                    if $mailbox.get_email().trim().is_empty() {
                        continue;
                    }
                    current_command.clear();
                    current_command.push(b"RCPT TO:<");
                    current_command.push($mailbox.get_email().trim().as_bytes());
                    if let Some(dsn_notify) = dsn_notify.as_ref() {
                        current_command.push(b"> NOTIFY=");
                        current_command.push(dsn_notify.as_bytes());
                    } else {
                        current_command.push(b">");
                    }
                    self.send_command(&current_command).await?;

                    //`RCPT TO:<forward-path> [ SP <rcpt-parameters> ] <CRLF>`
                    // If accepted, the SMTP server returns a "250 OK" reply and stores the
                    // forward-path.
                    if !self.server_conf.extensions.pipelining {
                        self.read_lines(&mut res, Some((ReplyCode::_250, &[])))
                            .await?;
                    } else {
                        pipelining_queue.push(Some((ReplyCode::_250, &[])));
                    }
                }};
            }

            match addr {
                Address::Mailbox(_) => send_to_mailbox!(addr),
                Address::Group(g) => {
                    for m in &g.mailbox_list {
                        send_to_mailbox!(m);
                    }
                }
            }
        }

        // Since it has been a common source of errors, it is worth noting that spaces
        // are not permitted on either side of the colon following FROM in the
        // MAIL command or TO in the RCPT command. The syntax is exactly as
        // given above.

        if self.server_conf.extensions.binarymime {
            let mail_length = format!("{}", mail.len());
            self.send_command(&[b"BDAT", mail_length.as_bytes(), b"LAST"])
                .await?;
            self.stream.write_all(mail.as_bytes()).await?;
        } else {
            //The third step in the procedure is the DATA command
            //(or some alternative specified in a service extension).
            //DATA `<CRLF>`
            self.send_command(&[b"DATA"]).await?;
            //Client SMTP implementations that employ pipelining MUST check ALL statuses
            // associated with each command in a group. For example, if none of
            // the RCPT TO recipient addresses were accepted the client must
            // then check the response to the DATA command -- the client
            // cannot assume that the DATA command will be rejected just because none of the
            // RCPT TO commands worked. If the DATA command was properly
            // rejected the client SMTP can just issue RSET, but if the DATA
            // command was accepted the client SMTP should send a single dot.
            for expected_reply_code in pipelining_queue {
                let reply = self.read_lines(&mut res, expected_reply_code).await?;
                pipelining_results.push(reply.into());
            }

            //If accepted, the SMTP server returns a 354 Intermediate reply and considers
            // all succeeding lines up to but not including the end of mail data
            // indicator to be the message text. When the end of text is
            // successfully received and stored, the SMTP-receiver sends a "250
            // OK" reply.
            self.read_lines(&mut res, Some((ReplyCode::_354, &[])))
                .await?;

            //Before sending a line of mail text, the SMTP client checks the first
            // character of the line.If it is a period, one additional period is
            // inserted at the beginning of the line.
            for line in mail.lines() {
                if line.starts_with('.') {
                    self.stream.write_all(b".").await?;
                }
                self.stream.write_all(line.as_bytes()).await?;
                self.stream.write_all(b"\r\n").await?;
            }

            //The mail data are terminated by a line containing only a period, that is, the
            // character sequence "`<CRLF>`.`<CRLF>`", where the first `<CRLF>` is
            // actually the terminator of the previous line (see Section 4.5.2).
            // This is the end of mail data indication.
            self.stream.write_all(b".\r\n").await?;
        }

        //The end of mail data indicator also confirms the mail transaction and tells
        // the SMTP server to now process the stored recipients and mail data.
        // If accepted, the SMTP server returns a "250 OK" reply.
        let reply_code = self
            .read_lines(
                &mut res,
                Some((
                    ReplyCode::_250,
                    if self.server_conf.extensions.prdr {
                        &[ReplyCode::_353]
                    } else {
                        &[]
                    },
                )),
            )
            .await?
            .code;
        // PRDR extension only:
        if reply_code == ReplyCode::_353 {
            // Read one line for each accepted recipient.
            for pipe_result in pipelining_results.iter().skip(1) {
                if pipe_result.is_err() {
                    continue;
                }
                prdr_results.push(self.read_lines(&mut res, None).await?.into());
            }
        }
        Ok(())
    }

    pub async fn quit(&mut self) -> Result<()> {
        self.send_command(&[b"QUIT"]).await?;

        Ok(())
    }
}

/// Expected reply code in a single or multi-line reply by the server
pub type ExpectedReplyCode = Option<(ReplyCode, &'static [ReplyCode])>;

/// Recognized kinds of SMTP reply codes
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplyCode {
    /// System status, or system help reply
    _211,
    /// Help message (Information on how to use the receiver or the meaning of a
    /// particular non-standard command; this reply is useful only to the human
    /// user)
    _214,
    /// `<domain>` Service ready
    _220,
    /// `<domain>` Service closing transmission channel
    _221,
    /// Authentication successful,
    _235,
    /// Requested mail action okay, completed
    _250,
    /// User not local; will forward to `<forward-path>` (See Section 3.4)
    _251,
    /// Cannot VRFY user, but will accept message and attempt delivery (See
    /// Section 3.5.3)
    _252,
    /// rfc4954 AUTH continuation request
    _334,
    /// PRDR specific, eg "content analysis has started|
    _353,
    /// Start mail input; end with `<CRLF>`.`<CRLF>`
    _354,
    /// `<domain>` Service not available, closing transmission channel (This may
    /// be a reply to any command if the service knows it must shut down)
    _421,
    /// Requested mail action not taken: mailbox unavailable (e.g., mailbox busy
    /// or temporarily blocked for policy reasons)
    _450,
    /// Requested action aborted: local error in processing
    _451,
    /// Requested action not taken: insufficient system storage
    _452,
    /// Server unable to accommodate parameters
    _455,
    /// Syntax error, command unrecognized (This may include errors such as
    /// command line too long)
    _500,
    /// Syntax error in parameters or arguments
    _501,
    /// Command not implemented (see Section 4.2.4)
    _502,
    /// Bad sequence of commands
    _503,
    /// Command parameter not implemented
    _504,
    /// Authentication failed
    _535,
    /// Requested action not taken: mailbox unavailable (e.g., mailbox not
    /// found, no access, or command rejected for policy reasons)
    _550,
    /// User not local; please try `<forward-path>` (See Section 3.4)
    _551,
    /// Requested mail action aborted: exceeded storage allocation
    _552,
    /// Requested action not taken: mailbox name not allowed (e.g., mailbox
    /// syntax incorrect)
    _553,
    /// Transaction failed (Or, in the case of a connection-opening response,
    /// "No SMTP service here")
    _554,
    /// MAIL FROM/RCPT TO parameters not recognized or not implemented
    _555,
    /// Must issue a STARTTLS command first
    _530,
}

impl ReplyCode {
    /// Return a description of the reply code in English.
    pub const fn as_str(&self) -> &'static str {
        use ReplyCode::*;
        match self {
            _211 => "System status, or system help reply",
            _214 => "Help message",
            _220 => "Service ready",
            _221 => "Service closing transmission channel",
            _250 => "Requested mail action okay, completed",
            _235 => "Authentication successful",
            _251 => "User not local; will forward",
            _252 => "Cannot VRFY user, but will accept message and attempt delivery",
            _334 => "Intermediate response to the AUTH command",
            _353 => "PRDR specific notice",
            _354 => "Start mail input; end with <CRLF>.<CRLF>",
            _421 => "Service not available, closing transmission channel",
            _450 => "Requested mail action not taken: mailbox unavailable",
            _451 => "Requested action aborted: local error in processing",
            _452 => "Requested action not taken: insufficient system storage",
            _455 => "Server unable to accommodate parameters",
            _500 => "Syntax error, command unrecognized",
            _501 => "Syntax error in parameters or arguments",
            _502 => "Command not implemented",
            _503 => "Bad sequence of commands",
            _504 => "Command parameter not implemented",
            _535 => "Authentication failed",
            _550 => {
                "Requested action not taken: mailbox unavailable (e.g., mailbox not found, no \
                 access, or command rejected for policy reasons)"
            }
            _551 => "User not local",
            _552 => "Requested mail action aborted: exceeded storage allocation",
            _553 => {
                "Requested action not taken: mailbox name not allowed (e.g., mailbox syntax \
                 incorrect)"
            }
            _554 => "Transaction failed",
            _555 => "MAIL FROM/RCPT TO parameters not recognized or not implemented",
            _530 => "Must issue a STARTTLS command first",
        }
    }

    /// Return the reply code as an integer.
    pub const fn value(&self) -> u16 {
        use ReplyCode::*;
        match self {
            _211 => 211,
            _214 => 214,
            _220 => 220,
            _221 => 221,
            _250 => 250,
            _235 => 235,
            _251 => 251,
            _252 => 252,
            _334 => 334,
            _353 => 353,
            _354 => 354,
            _421 => 421,
            _450 => 450,
            _451 => 451,
            _452 => 452,
            _455 => 455,
            _500 => 500,
            _501 => 501,
            _502 => 502,
            _503 => 503,
            _504 => 504,
            _535 => 535,
            _550 => 550,
            _551 => 551,
            _552 => 552,
            _553 => 553,
            _554 => 554,
            _555 => 555,
            _530 => 530,
        }
    }

    /// Returns `true` if reply code indicates that an error has occurred.
    pub const fn is_err(&self) -> bool {
        use ReplyCode::*;
        matches!(
            self,
            _421 | _450
                | _451
                | _452
                | _455
                | _500
                | _501
                | _502
                | _503
                | _504
                | _535
                | _550
                | _551
                | _552
                | _553
                | _554
                | _555
                | _530
        )
    }
}

/// Parse a [`ReplyCode`] from a string slice of three digits, e.g. `"250"`.
impl TryFrom<&'_ str> for ReplyCode {
    type Error = Error;
    fn try_from(val: &'_ str) -> Result<Self> {
        if val.len() != 3 {
            debug!("{}", val);
        }
        debug_assert!(val.len() == 3);
        use ReplyCode::*;
        match val {
            "211" => Ok(_211),
            "214" => Ok(_214),
            "220" => Ok(_220),
            "221" => Ok(_221),
            "235" => Ok(_235),
            "250" => Ok(_250),
            "251" => Ok(_251),
            "252" => Ok(_252),
            "334" => Ok(_334),
            "353" => Ok(_353),
            "354" => Ok(_354),
            "421" => Ok(_421),
            "450" => Ok(_450),
            "451" => Ok(_451),
            "452" => Ok(_452),
            "455" => Ok(_455),
            "500" => Ok(_500),
            "501" => Ok(_501),
            "502" => Ok(_502),
            "503" => Ok(_503),
            "504" => Ok(_504),
            "535" => Ok(_535),
            "550" => Ok(_550),
            "551" => Ok(_551),
            "552" => Ok(_552),
            "553" => Ok(_553),
            "554" => Ok(_554),
            "555" => Ok(_555),
            _ => Err(Error::new(format!("Unknown SMTP reply code: {val}"))),
        }
    }
}

/// A single line or multi-line server reply, along with its reply code
#[derive(Clone, Debug)]
pub struct Reply<'s> {
    pub code: ReplyCode,
    pub lines: SmallVec<[&'s str; 16]>,
}

impl<'s> From<Reply<'s>> for Result<ReplyCode> {
    fn from(val: Reply<'s>) -> Self {
        if val.code.is_err() {
            Err(Error::new(val.lines.join("\n")).set_summary(val.code.as_str()))
        } else {
            Ok(val.code)
        }
    }
}

impl<'s> Reply<'s> {
    /// `s` must be raw SMTP output i.e each line must start with 3 digit reply
    /// code, a space or '-' and end with '\r\n'
    ///
    /// Lines shorter than four bytes (from a hostile or truncated server)
    /// contribute an empty text line instead of panicking.
    pub fn new(s: &'s str, code: ReplyCode) -> Self {
        let lines: SmallVec<_> = s.lines().map(|l| l.get(4..).unwrap_or_default()).collect();
        Self { lines, code }
    }
}

/// Extract the authentication mechanism names from an EHLO capability line
/// that starts with `AUTH`.
///
/// A hostile or broken server may send a bare `AUTH` line with no mechanism
/// list (or an `AUTH`-prefixed line shorter than the `"AUTH "` prefix), so the
/// prefix is stripped with `str::get` and an empty iterator is returned
/// instead of panicking on an out-of-bounds slice.
fn parse_auth_mechanisms(line: &str) -> impl Iterator<Item = &str> {
    line.get("AUTH ".len()..)
        .unwrap_or_default()
        .split_whitespace()
}

async fn read_lines<'r>(
    _self: &mut (impl futures::io::AsyncRead + std::marker::Unpin + Send),
    ret: &'r mut String,
    expected_reply_code: Option<(ReplyCode, &[ReplyCode])>,
    buffer: &mut String,
) -> Result<Reply<'r>> {
    let mut buf: [u8; 1024] = [0; 1024];
    ret.clear();
    ret.extend(buffer.drain(..));
    let mut last_line_idx: usize = 0;
    let mut returned_code: Option<ReplyCode> = None;
    'read_loop: loop {
        while let Some(pos) = ret[last_line_idx..].find("\r\n") {
            // "Formally, a reply is defined to be the sequence: a three-digit code, `<SP>`,
            // one line of text, and `<CRLF>`, or a multiline reply (as defined in the same
            // section)."
            if ret[last_line_idx..].len() < 4
                || !ret[last_line_idx..]
                    .chars()
                    .take(3)
                    .all(|c| c.is_ascii_digit())
            {
                return Err(Error::new(format!("Invalid SMTP reply: {ret}")));
            }
            if let Some(ref returned_code) = returned_code {
                if ReplyCode::try_from(ret[last_line_idx..].get(..3).unwrap())? != *returned_code {
                    buffer.extend(ret.drain(last_line_idx..));
                    if ret.lines().last().and_then(|l| l.chars().nth(4)) != Some(' ') {
                        return Err(Error::new(format!("Invalid SMTP reply: {ret}")));
                    }
                    break 'read_loop;
                }
                if ret[last_line_idx + 3..].starts_with(' ') {
                    buffer.extend(ret.drain(last_line_idx + pos + "\r\n".len()..));
                    break 'read_loop;
                }
            } else {
                if ret[last_line_idx + 3..].starts_with(' ') {
                    buffer.extend(ret.drain(last_line_idx + pos + "\r\n".len()..));
                    break 'read_loop;
                }
                returned_code = Some(ReplyCode::try_from(&ret[last_line_idx..last_line_idx + 3])?);
            }
            last_line_idx += pos + "\r\n".len();
        }
        match timeout(Some(SMTP_READ_TIMEOUT), _self.read(&mut buf)).await? {
            Ok(0) => break,
            Ok(b) => {
                ret.push_str(&String::from_utf8_lossy(&buf[0..b]));
                enforce_response_size_limit(ret.len())?;
            }
            Err(err) => {
                return Err(Error::from(err));
            }
        }
    }
    if ret.len() < 3 || !ret.chars().take(3).all(|c| c.is_ascii_digit()) {
        return Err(Error::new(format!("Invalid SMTP reply: {ret}")));
    }
    let code = ReplyCode::try_from(&ret[..3])?;
    let reply = Reply::new(ret, code);
    //debug!(&reply);
    if expected_reply_code
        .map(|(exp, exp_list)| exp != reply.code && !exp_list.contains(&reply.code))
        .unwrap_or(false)
    {
        let result: Result<ReplyCode> = reply.clone().into();
        result?;
        return Err(Error::new(format!(
            "SMTP Server didn't reply with expected greeting code {:?}: {:?}",
            expected_reply_code.unwrap(),
            reply
        )));
    }
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{
        error::NetworkErrorKind,
        utils::connections::{cap_test_utils, max_server_response_size},
    };

    /// Regression test for the server response size cap in SMTP
    /// [`read_lines`]: a server that keeps sending `250-` continuation lines
    /// (never the final `250 ` line) must produce an error once the cap is
    /// reached, instead of accumulating unboundedly.
    #[test]
    fn test_smtp_read_lines_response_size_cap() {
        cap_test_utils::assert_completes_within(10, || {
            let cap = max_server_response_size();
            let (mut stream, writer) =
                cap_test_utils::malicious_server(b"250-never ending\r\n", cap.saturating_mul(2));
            let mut ret = String::new();
            let mut buffer = String::new();
            let res = smol::block_on(read_lines(
                &mut stream,
                &mut ret,
                Some((ReplyCode::_250, &[])),
                &mut buffer,
            ));
            drop(stream);
            let _ = writer.join();
            let err = res.unwrap_err();
            assert!(
                matches!(
                    err.kind,
                    ErrorKind::Network(NetworkErrorKind::ProtocolViolation)
                ),
                "unexpected error: {err:?}"
            );
            assert!(ret.len() > cap);
            assert!(
                ret.len() <= cap + 1024,
                "accumulation must stay bounded: {} > {}",
                ret.len(),
                cap + 1024
            );
        });
    }

    /// SMTP read timeout regression test: a server that stays silent (but
    /// keeps the connection open) must produce a `TimedOut` error instead of
    /// hanging forever. SMTP previously had no read timeout at all.
    #[test]
    fn test_smtp_read_lines_read_timeout() {
        cap_test_utils::assert_completes_within(10, || {
            let (mut stream, writer) =
                cap_test_utils::silent_server(std::time::Duration::from_secs(2));
            let mut ret = String::new();
            let mut buffer = String::new();
            let res = smol::block_on(read_lines(
                &mut stream,
                &mut ret,
                Some((ReplyCode::_220, &[])),
                &mut buffer,
            ));
            drop(stream);
            let _ = writer.join();
            let err = res.unwrap_err();
            assert!(
                matches!(err.kind, ErrorKind::TimedOut),
                "unexpected error: {err:?}"
            );
        });
    }

    /// C6 regression: a short code line (`221\r\n`) followed by a
    /// mismatching long line (`250-x\r\n`) used to panic: draining the
    /// mismatch left the bare `221` line, whose 4th char does not exist,
    /// and `.nth(4).unwrap()` panicked. The reverse ordering (long code
    /// first) was already a graceful error.
    #[test]
    fn test_smtp_read_lines_short_code_first_mismatch_is_err() {
        cap_test_utils::assert_completes_within(10, || {
            let (mut stream, writer) = cap_test_utils::malicious_server(b"221\r\n250-x\r\n", 13);
            let mut ret = String::new();
            let mut buffer = String::new();
            let res = smol::block_on(read_lines(&mut stream, &mut ret, None, &mut buffer));
            drop(stream);
            let _ = writer.join();
            let err = res.unwrap_err();
            assert!(
                err.to_string().contains("Invalid SMTP reply"),
                "unexpected error: {err:?}"
            );
        });
    }

    /// C6 regression (order twin): long code first leaves a well-formed
    /// `250-x` line after the drain, which was already rejected gracefully;
    /// it must stay an error and not become a panic.
    #[test]
    fn test_smtp_read_lines_long_code_first_mismatch_is_err() {
        cap_test_utils::assert_completes_within(10, || {
            let (mut stream, writer) = cap_test_utils::malicious_server(b"250-x\r\n221\r\n", 13);
            let mut ret = String::new();
            let mut buffer = String::new();
            let res = smol::block_on(read_lines(&mut stream, &mut ret, None, &mut buffer));
            drop(stream);
            let _ = writer.join();
            let err = res.unwrap_err();
            assert!(
                err.to_string().contains("Invalid SMTP reply"),
                "unexpected error: {err:?}"
            );
        });
    }

    /// C6 regression: a final chunk of exactly `b"250"` followed by stream
    /// close passes the `ret.len() < 3` check and used to panic inside
    /// `Reply::new` (`&l[4..l.len()]` on a 3-byte line). It must now parse
    /// to a reply with a single empty text line.
    #[test]
    fn test_smtp_read_lines_truncated_three_byte_reply_degrades() {
        cap_test_utils::assert_completes_within(10, || {
            let (mut stream, writer) = cap_test_utils::malicious_server(b"250", 3);
            let mut ret = String::new();
            let mut buffer = String::new();
            let res = smol::block_on(read_lines(&mut stream, &mut ret, None, &mut buffer));
            drop(stream);
            let _ = writer.join();
            let reply = res.unwrap();
            assert_eq!(reply.code, ReplyCode::_250);
            assert_eq!(reply.lines.iter().copied().collect::<Vec<_>>(), vec![""]);
        });
    }

    /// C6 regression: shorter-than-a-code truncated replies must stay
    /// errors (this was already graceful and must not regress).
    #[test]
    fn test_smtp_read_lines_truncated_shorter_than_code_is_err() {
        cap_test_utils::assert_completes_within(10, || {
            let (mut stream, writer) = cap_test_utils::malicious_server(b"2", 1);
            let mut ret = String::new();
            let mut buffer = String::new();
            let res = smol::block_on(read_lines(&mut stream, &mut ret, None, &mut buffer));
            drop(stream);
            let _ = writer.join();
            let err = res.unwrap_err();
            assert!(
                err.to_string().contains("Invalid SMTP reply"),
                "unexpected error: {err:?}"
            );
        });
    }

    /// C6 regression: an empty line and an immediate EOF must be errors
    /// (both already graceful; pinned so no fix reintroduces a panic).
    #[test]
    fn test_smtp_read_lines_empty_line_and_eof_are_err() {
        cap_test_utils::assert_completes_within(10, || {
            for chunk in [b"\r\n" as &[u8], b""] {
                let (mut stream, writer) = cap_test_utils::malicious_server(chunk, chunk.len());
                let mut ret = String::new();
                let mut buffer = String::new();
                let res = smol::block_on(read_lines(&mut stream, &mut ret, None, &mut buffer));
                drop(stream);
                let _ = writer.join();
                let err = res.unwrap_err();
                assert!(
                    err.to_string().contains("Invalid SMTP reply"),
                    "unexpected error for {chunk:?}: {err:?}"
                );
            }
        });
    }

    /// C6 regression: invalid UTF-8 in an otherwise valid reply used to be
    /// pushed into the `String` via `from_utf8_unchecked` (constructive
    /// UB); it must now decode lossily to U+FFFD replacement characters.
    #[test]
    fn test_smtp_read_lines_invalid_utf8_degrades_lossy() {
        cap_test_utils::assert_completes_within(10, || {
            let (mut stream, writer) = cap_test_utils::malicious_server(b"250 \xff\xfe\r\n", 8);
            let mut ret = String::new();
            let mut buffer = String::new();
            let res = smol::block_on(read_lines(&mut stream, &mut ret, None, &mut buffer));
            drop(stream);
            let _ = writer.join();
            let reply = res.unwrap();
            assert_eq!(reply.code, ReplyCode::_250);
            let lines: Vec<_> = reply.lines.iter().copied().collect();
            drop(reply);
            assert_eq!(
                lines,
                vec!["\u{FFFD}\u{FFFD}"],
                "invalid UTF-8 must decode lossily"
            );
            assert!(ret.contains('\u{FFFD}'), "ret was {ret:?}");
        });
    }

    /// Freeze: a well-formed multi-line reply parses exactly as before.
    #[test]
    fn test_smtp_read_lines_valid_multiline_unchanged() {
        cap_test_utils::assert_completes_within(10, || {
            let (mut stream, writer) =
                cap_test_utils::malicious_server(b"250-first\r\n250 second\r\n", 23);
            let mut ret = String::new();
            let mut buffer = String::new();
            let res = smol::block_on(read_lines(
                &mut stream,
                &mut ret,
                Some((ReplyCode::_250, &[])),
                &mut buffer,
            ));
            drop(stream);
            let _ = writer.join();
            let reply = res.unwrap();
            assert_eq!(reply.code, ReplyCode::_250);
            assert_eq!(
                reply.lines.iter().copied().collect::<Vec<_>>(),
                vec!["first", "second"]
            );
        });
    }

    /// Regression: the PRDR `353` reply code must be parseable. Without a
    /// `"353"` arm in [`ReplyCode`]'s `TryFrom<&str>` impl, a valid PRDR
    /// server reply is rejected as "Unknown SMTP reply code" before the
    /// expected-reply-code list (which accepts `353`) is ever consulted,
    /// making the whole PRDR content-analysis path unreachable.
    #[test]
    fn test_smtp_reply_code_353_parses() {
        let code = ReplyCode::try_from("353").expect("353 is a valid PRDR reply code");
        assert_eq!(code, ReplyCode::_353);
        assert_eq!(code.value(), 353);
        assert_eq!(code.as_str(), "PRDR specific notice");
        assert!(!code.is_err());
    }

    /// Every parseable [`ReplyCode`] variant's numeric value must round-trip
    /// through the string parser; this pins the parser against future
    /// additions of variants that forget a match arm (the bug class behind
    /// the missing `"353"` arm fixed here).
    #[test]
    fn test_smtp_reply_code_roundtrip() {
        let all = [
            ReplyCode::_211,
            ReplyCode::_214,
            ReplyCode::_220,
            ReplyCode::_221,
            ReplyCode::_235,
            ReplyCode::_250,
            ReplyCode::_251,
            ReplyCode::_252,
            ReplyCode::_334,
            ReplyCode::_353,
            ReplyCode::_354,
            ReplyCode::_421,
            ReplyCode::_450,
            ReplyCode::_451,
            ReplyCode::_452,
            ReplyCode::_455,
            ReplyCode::_500,
            ReplyCode::_501,
            ReplyCode::_502,
            ReplyCode::_503,
            ReplyCode::_504,
            ReplyCode::_535,
            ReplyCode::_550,
            ReplyCode::_551,
            ReplyCode::_552,
            ReplyCode::_553,
            ReplyCode::_554,
            ReplyCode::_555,
        ];
        for code in all {
            let s = code.value().to_string();
            assert_eq!(
                ReplyCode::try_from(s.as_str()).ok(),
                Some(code),
                "reply code {s} must round-trip through TryFrom<&str>"
            );
        }
        // `_530` is a known pre-existing gap: its arm is also absent from
        // `TryFrom<&str>`, but fixing it is out of scope for this change, so
        // pin the current behaviour instead of asserting it round-trips.
        let err = ReplyCode::try_from("530").unwrap_err();
        assert!(err.to_string().contains("Unknown SMTP reply code"));
    }

    #[test]
    fn test_parse_auth_mechanisms_tolerates_bare_auth() {
        // A bare `AUTH` EHLO line (no mechanism list) used to panic on
        // `l["AUTH ".len()..]`; it must simply advertise no mechanisms.
        assert_eq!(
            parse_auth_mechanisms("AUTH").collect::<Vec<_>>(),
            Vec::<&str>::new()
        );
        assert_eq!(
            parse_auth_mechanisms("AUTH PLAIN LOGIN").collect::<Vec<_>>(),
            vec!["PLAIN", "LOGIN"]
        );
    }
}
