/*
 * meli - melib library
 *
 * Copyright 2020  Manos Pitsidianakis
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

//! Connections layers (TCP/fd/TLS/Deflate) to use with remote backends.
use std::{
    borrow::Cow,
    os::{
        fd::{AsFd, BorrowedFd, OwnedFd},
        unix::io::AsRawFd,
    },
    time::Duration,
};

use flate2::{read::DeflateDecoder, write::DeflateEncoder, Compression};
#[cfg(any(target_os = "openbsd", target_os = "netbsd", target_os = "haiku"))]
use libc::SO_KEEPALIVE as KEEPALIVE_OPTION;
#[cfg(any(target_os = "macos", target_os = "ios"))]
use libc::TCP_KEEPALIVE as KEEPALIVE_OPTION;
#[cfg(not(any(
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "haiku",
    target_os = "macos",
    target_os = "ios"
)))]
use libc::TCP_KEEPIDLE as KEEPALIVE_OPTION;
use libc::{self, c_int, c_void};

use crate::error::{Error, ErrorKind, NetworkErrorKind, Result};

// pub mod smol;
pub mod std_net;

pub const CONNECTION_ATTEMPT_DELAY: std::time::Duration = std::time::Duration::from_millis(250);

pub enum Connection {
    Tcp {
        inner: std::net::TcpStream,
        id: Option<&'static str>,
        trace: bool,
    },
    Fd {
        inner: OwnedFd,
        id: Option<&'static str>,
        trace: bool,
    },
    #[cfg(feature = "tls")]
    Tls {
        inner: native_tls::TlsStream<Self>,
        id: Option<&'static str>,
        trace: bool,
    },
    Deflate {
        inner: DeflateEncoder<DeflateDecoder<Box<Self>>>,
        id: Option<&'static str>,
        trace: bool,
    },
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Tcp {
                ref trace,
                ref inner,
                ref id,
            } => fmt
                .debug_struct(crate::identify!(Connection))
                .field("variant", &stringify!(Tcp))
                .field(stringify!(trace), trace)
                .field(stringify!(id), id)
                .field(stringify!(inner), inner)
                .finish(),
            #[cfg(feature = "tls")]
            Tls {
                ref trace,
                ref inner,
                ref id,
            } => fmt
                .debug_struct(crate::identify!(Connection))
                .field("variant", &stringify!(Tls))
                .field(stringify!(trace), trace)
                .field(stringify!(id), id)
                .field(stringify!(inner), inner.get_ref())
                .finish(),
            Fd {
                ref trace,
                ref inner,
                ref id,
            } => fmt
                .debug_struct(crate::identify!(Connection))
                .field("variant", &stringify!(Fd))
                .field(stringify!(trace), trace)
                .field(stringify!(id), id)
                .field(stringify!(inner), inner)
                .finish(),
            Deflate {
                ref trace,
                ref inner,
                ref id,
            } => fmt
                .debug_struct(crate::identify!(Connection))
                .field("variant", &stringify!(Deflate))
                .field(stringify!(trace), trace)
                .field(stringify!(id), id)
                .field(stringify!(inner), inner)
                .finish(),
        }
    }
}

use Connection::*;

macro_rules! syscall {
    ($fn: ident ( $($arg: expr),* $(,)* ) ) => {{
        #[allow(unused_unsafe)]
        let res = unsafe { libc::$fn($($arg, )*) };
        if res == -1 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(res)
        }
    }};
}

/// Hardcoded `setsockopt` arguments for type safety when calling
/// [`Connection::setsockopt`] in an `unsafe` block.
///
/// Add new variants when you need to call `setsockopt` with new arguments.
pub enum SockOpts {
    /// Set TCP Keep Alive.
    ///
    /// Following text is sourced from <https://tldp.org/HOWTO/html_single/TCP-Keepalive-HOWTO/>.
    ///
    /// ```text
    /// 4.2. The setsockopt function call
    ///
    /// All you need to enable keepalive for a specific socket is to set the specific socket option
    /// on the socket itself. The prototype of the function is as follows:
    ///
    ///
    ///   int setsockopt(int s, int level, int optname,
    ///                  const void *optval, socklen_t optlen)
    ///
    ///
    /// The first parameter is the socket, previously created with the socket(2); the second one
    /// must be SOL_SOCKET, and the third must be SO_KEEPALIVE . The fourth parameter must be a
    /// boolean integer value, indicating that we want to enable the option, while the last is the
    /// size of the value passed before.
    ///
    /// According to the manpage, 0 is returned upon success, and -1 is returned on error (and
    /// errno is properly set).
    ///
    /// There are also three other socket options you can set for keepalive when you write your
    /// application. They all use the SOL_TCP level instead of SOL_SOCKET, and they override
    /// system-wide variables only for the current socket. If you read without writing first, the
    /// current system-wide parameters will be returned.
    ///
    ///     TCP_KEEPCNT: overrides tcp_keepalive_probes
    ///
    ///     TCP_KEEPIDLE: overrides tcp_keepalive_time
    ///
    ///     TCP_KEEPINTVL: overrides tcp_keepalive_intvl
    /// ```
    ///
    /// Field `duration` overrides `tcp_keepalive_time`:
    ///
    /// ```text
    /// tcp_keepalive_time
    ///
    ///    the interval between the last data packet sent (simple ACKs are not considered data) and the
    ///    first keepalive probe; after the connection is marked to need keepalive, this counter is not
    ///    used any further
    /// ```
    ///
    /// The default value in the Linux kernel is 7200 seconds (2 hours).
    KeepAlive {
        enable: bool,
        duration: Option<Duration>,
    },
    TcpNoDelay {
        enable: bool,
    },
}

/// The maximum server response size enforced by
/// [`enforce_response_size_limit`].
///
/// Lowered in test builds so that cap regression tests don't have to pump
/// dozens of MiB through pipes.
#[cfg(not(test))]
fn max_server_response_size() -> usize {
    Connection::MAX_SERVER_RESPONSE_SIZE
}

#[cfg(test)]
pub(crate) fn max_server_response_size() -> usize {
    128 * 1024
}

/// Returns `Err` if `len`, the amount of data accumulated so far while
/// reading a server response, exceeds the maximum allowed server response
/// size ([`Connection::MAX_SERVER_RESPONSE_SIZE`]).
///
/// Protocol reader loops call this after appending each chunk they read, so
/// that a malicious or malfunctioning server which never terminates its
/// reply cannot make the client accumulate data unboundedly.
pub(crate) fn enforce_response_size_limit(len: usize) -> Result<()> {
    let max = max_server_response_size();
    if len > max {
        Err(Error::new(format!(
            "server response size {len} bytes exceeds the maximum allowed size of {max} bytes"
        ))
        .set_kind(ErrorKind::Network(NetworkErrorKind::ProtocolViolation)))
    } else {
        Ok(())
    }
}

impl Connection {
    pub const IO_BUF_SIZE: usize = 64 * 1024;

    /// Maximum size in bytes a single server response (or, for line-oriented
    /// readers, a single line) may accumulate before the reader gives up and
    /// errors out.
    ///
    /// This bounds memory usage when a remote server never terminates its
    /// reply: an IMAP server that keeps sending untagged responses without
    /// ever sending the tagged command completion, an NNTP server whose
    /// multiline response never reaches the terminating `.` line, or an SMTP
    /// server that keeps sending continuation lines forever. Per-read
    /// timeouts do not defend against such servers, because each individual
    /// read succeeds.
    pub const MAX_SERVER_RESPONSE_SIZE: usize = 64 * 1024 * 1024;

    pub fn deflate(mut self) -> Self {
        let trace = self.is_trace_enabled();
        let id = self.id();
        self.set_trace(false);
        Self::Deflate {
            inner: DeflateEncoder::new(
                DeflateDecoder::new_with_buf(Box::new(self), vec![0; Self::IO_BUF_SIZE]),
                Compression::default(),
            ),
            id,
            trace,
        }
    }

    #[cfg(feature = "tls")]
    pub fn new_tls(mut inner: native_tls::TlsStream<Self>) -> Self {
        let trace = inner.get_ref().is_trace_enabled();
        let id = inner.get_ref().id();
        if trace {
            inner.get_mut().set_trace(false);
        }
        Self::Tls { inner, id, trace }
    }

    pub fn new_tcp(inner: std::net::TcpStream) -> Self {
        let ret = Self::Tcp {
            inner,
            id: None,
            trace: false,
        };
        _ = ret.setsockopt(SockOpts::TcpNoDelay { enable: true });

        ret
    }

    pub fn trace(mut self, val: bool) -> Self {
        match self {
            Tcp { ref mut trace, .. } => *trace = val,
            #[cfg(feature = "tls")]
            Tls { ref mut trace, .. } => *trace = val,
            Fd { ref mut trace, .. } => {
                *trace = val;
            }
            Deflate { ref mut trace, .. } => *trace = val,
        }
        self
    }

    pub fn with_id(mut self, val: &'static str) -> Self {
        match self {
            Tcp { ref mut id, .. } => *id = Some(val),
            #[cfg(feature = "tls")]
            Tls { ref mut id, .. } => *id = Some(val),
            Fd { ref mut id, .. } => {
                *id = Some(val);
            }
            Deflate { ref mut id, .. } => *id = Some(val),
        }
        self
    }

    pub fn set_trace(&mut self, val: bool) {
        match self {
            Tcp { ref mut trace, .. } => *trace = val,
            #[cfg(feature = "tls")]
            Tls { ref mut trace, .. } => *trace = val,
            Fd { ref mut trace, .. } => {
                *trace = val;
            }
            Deflate { ref mut trace, .. } => *trace = val,
        }
    }

    pub fn set_nonblocking(&self, nonblocking: bool) -> std::io::Result<()> {
        if self.is_trace_enabled() {
            let id = self.id();
            log::trace!(
                "{}{}{}{:?} set_nonblocking({:?})",
                if id.is_some() { "[" } else { "" },
                if let Some(id) = id.as_ref() { id } else { "" },
                if id.is_some() { "]: " } else { "" },
                self,
                nonblocking
            );
        }
        match self {
            Tcp { ref inner, .. } => inner.set_nonblocking(nonblocking),
            #[cfg(feature = "tls")]
            Tls { ref inner, .. } => inner.get_ref().set_nonblocking(nonblocking),
            Fd { inner, .. } => {
                // [ref:VERIFY]
                nix::fcntl::fcntl(
                    inner.as_fd(),
                    nix::fcntl::FcntlArg::F_SETFL(if nonblocking {
                        nix::fcntl::OFlag::O_NONBLOCK
                    } else {
                        !nix::fcntl::OFlag::O_NONBLOCK
                    }),
                )
                .map_err(|err| std::io::Error::from_raw_os_error(err as i32))?;
                Ok(())
            }
            Deflate { ref inner, .. } => inner.get_ref().get_ref().set_nonblocking(nonblocking),
        }
    }

    pub fn set_read_timeout(&self, dur: Option<Duration>) -> std::io::Result<()> {
        if self.is_trace_enabled() {
            let id = self.id();
            log::trace!(
                "{}{}{}{:?} set_read_timeout({:?})",
                if id.is_some() { "[" } else { "" },
                if let Some(id) = id.as_ref() { id } else { "" },
                if id.is_some() { "]: " } else { "" },
                self,
                dur
            );
        }
        match self {
            Tcp { ref inner, .. } => inner.set_read_timeout(dur),
            #[cfg(feature = "tls")]
            Tls { ref inner, .. } => inner.get_ref().set_read_timeout(dur),
            Fd { .. } => Ok(()),
            Deflate { ref inner, .. } => inner.get_ref().get_ref().set_read_timeout(dur),
        }
    }

    pub fn set_write_timeout(&self, dur: Option<Duration>) -> std::io::Result<()> {
        if self.is_trace_enabled() {
            let id = self.id();
            log::trace!(
                "{}{}{}{:?} set_write_timeout({:?})",
                if id.is_some() { "[" } else { "" },
                if let Some(id) = id.as_ref() { id } else { "" },
                if id.is_some() { "]: " } else { "" },
                self,
                dur
            );
        }
        match self {
            Tcp { ref inner, .. } => inner.set_write_timeout(dur),
            #[cfg(feature = "tls")]
            Tls { ref inner, .. } => inner.get_ref().set_write_timeout(dur),
            Fd { .. } => Ok(()),
            Deflate { ref inner, .. } => inner.get_ref().get_ref().set_write_timeout(dur),
        }
    }

    pub fn keepalive(&self) -> std::io::Result<Option<Duration>> {
        if self.is_trace_enabled() {
            log::trace!("{:?} keepalive()", self);
        }
        if matches!(self, Fd { .. }) {
            return Ok(None);
        }
        unsafe {
            let raw: c_int = self.__getsockopt(libc::SOL_SOCKET, libc::SO_KEEPALIVE)?;
            if raw == 0 {
                return Ok(None);
            }
            let secs: c_int = self.__getsockopt(libc::IPPROTO_TCP, KEEPALIVE_OPTION)?;
            Ok(Some(Duration::new(secs as u64, 0)))
        }
    }

    pub fn set_keepalive(&self, keepalive: Option<Duration>) -> std::io::Result<()> {
        if self.is_trace_enabled() {
            let id = self.id();
            log::trace!(
                "{}{}{}{:?} set_keepalive({:?})",
                if id.is_some() { "[" } else { "" },
                if let Some(id) = id.as_ref() { id } else { "" },
                if id.is_some() { "]: " } else { "" },
                self,
                keepalive
            );
        }
        if matches!(self, Fd { .. }) {
            return Ok(());
        }
        self.setsockopt(SockOpts::KeepAlive {
            enable: keepalive.is_some(),
            duration: keepalive,
        })
    }

    unsafe fn inner_setsockopt<T>(&self, opt: c_int, val: c_int, payload: T) -> std::io::Result<()>
    where
        T: Copy,
    {
        let payload = std::ptr::addr_of!(payload) as *const c_void;
        syscall!(setsockopt(
            self.as_raw_fd(),
            opt,
            val,
            payload,
            std::mem::size_of::<T>() as libc::socklen_t,
        ))?;
        Ok(())
    }

    pub fn setsockopt(&self, option: SockOpts) -> std::io::Result<()> {
        match option {
            SockOpts::KeepAlive {
                enable: true,
                duration,
            } => {
                unsafe {
                    self.inner_setsockopt(libc::SOL_SOCKET, libc::SO_KEEPALIVE, <c_int>::from(true))
                }?;
                if let Some(dur) = duration {
                    unsafe {
                        self.inner_setsockopt(
                            libc::IPPROTO_TCP,
                            KEEPALIVE_OPTION,
                            dur.as_secs() as c_int,
                        )
                    }?;
                }
                Ok(())
            }
            SockOpts::KeepAlive {
                enable: false,
                duration: _,
            } => unsafe {
                self.inner_setsockopt(libc::SOL_SOCKET, libc::SO_KEEPALIVE, <c_int>::from(false))
            },
            SockOpts::TcpNoDelay { enable } => unsafe {
                #[cfg(any(
                    target_os = "openbsd",
                    target_os = "netbsd",
                    target_os = "haiku",
                    target_os = "macos",
                    target_os = "ios"
                ))]
                {
                    self.inner_setsockopt(
                        libc::IPPROTO_TCP,
                        libc::TCP_NODELAY,
                        if enable { c_int::from(1_u8) } else { 0 },
                    )
                }
                #[cfg(not(any(
                    target_os = "openbsd",
                    target_os = "netbsd",
                    target_os = "haiku",
                    target_os = "macos",
                    target_os = "ios"
                )))]
                {
                    self.inner_setsockopt(
                        libc::SOL_TCP,
                        libc::TCP_NODELAY,
                        if enable { c_int::from(1_u8) } else { 0 },
                    )
                }
            },
        }
    }

    #[inline]
    unsafe fn __getsockopt<T: Copy>(&self, opt: c_int, val: c_int) -> std::io::Result<T> {
        let mut slot: T = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<T>() as libc::socklen_t;
        syscall!(getsockopt(
            self.as_raw_fd(),
            opt,
            val,
            std::ptr::addr_of_mut!(slot) as *mut _,
            &raw mut len,
        ))?;
        assert_eq!(len as usize, std::mem::size_of::<T>());
        Ok(slot)
    }

    fn is_trace_enabled(&self) -> bool {
        match self {
            Fd { trace, .. } | Tcp { trace, .. } => *trace,
            #[cfg(feature = "tls")]
            Tls { trace, .. } => *trace,
            Deflate { trace, .. } => *trace,
        }
    }

    fn id(&self) -> Option<&'static str> {
        match self {
            Fd { id, .. } | Tcp { id, .. } => *id,
            #[cfg(feature = "tls")]
            Tls { id, .. } => *id,
            Deflate { id, .. } => *id,
        }
    }
}

impl std::io::Read for Connection {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let res = match self {
            Tcp { ref mut inner, .. } => inner.read(buf),
            #[cfg(feature = "tls")]
            Tls { ref mut inner, .. } => inner.read(buf),
            Fd { ref inner, .. } => {
                use std::os::unix::io::{FromRawFd, IntoRawFd};
                let mut f = unsafe { std::fs::File::from_raw_fd(inner.as_raw_fd()) };
                let ret = f.read(buf);
                let _ = f.into_raw_fd();
                ret
            }
            Deflate { ref mut inner, .. } => inner.read(buf),
        };
        if self.is_trace_enabled() {
            let id = self.id();
            match &res {
                Ok(len) => {
                    let slice = &buf[..*len];
                    log::trace!(
                        "{}{}{}{:?} read {:?} bytes:{}",
                        if id.is_some() { "[" } else { "" },
                        if let Some(id) = id.as_ref() { id } else { "" },
                        if id.is_some() { "]: " } else { "" },
                        self,
                        len,
                        std::str::from_utf8(slice)
                            .map(Cow::Borrowed)
                            .or_else(|_| crate::text::hex::bytes_to_hex(slice).map(Cow::Owned))
                            .unwrap_or(Cow::Borrowed("Could not convert to hex."))
                    );
                }
                Err(err) if matches!(err.kind(), std::io::ErrorKind::WouldBlock) => {}
                Err(err) => {
                    log::trace!(
                        "{}{}{}{:?} could not read {:?}",
                        if id.is_some() { "[" } else { "" },
                        if let Some(id) = id.as_ref() { id } else { "" },
                        if id.is_some() { "]: " } else { "" },
                        self,
                        err,
                    );
                }
            }
        }
        res
    }
}

/// Redaction marker substituted for SASL credential payloads in trace dumps,
/// mirroring the `<REDACTED>` marker the logger uses for HTTP auth headers.
const SASL_REDACTED: &[u8] = b"<REDACTED>";

/// Minimum length for a buffer to be considered a standalone SASL base64
/// continuation write. Real SASL PLAIN/XOAUTH2 payloads are longer; the
/// threshold keeps short non-credential writes from being redacted.
const MIN_SASL_BASE64_LEN: usize = 8;

fn is_base64_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'+' || b == b'/'
}

/// Removes one optional line terminator (`\r\n` or `\n`) from the end.
fn strip_line_terminator(buf: &[u8]) -> &[u8] {
    let buf = buf.strip_suffix(b"\n").unwrap_or(buf);
    buf.strip_suffix(b"\r").unwrap_or(buf)
}

/// Returns `true` if `token` is a base64 blob of at least
/// [`MIN_SASL_BASE64_LEN`] bytes with 0-2 `=` padding characters at the end.
fn is_base64_token(token: &[u8]) -> bool {
    if token.len() < MIN_SASL_BASE64_LEN {
        return false;
    }
    let padding = token.iter().rev().take_while(|&&b| b == b'=').count();
    if padding > 2 {
        return false;
    }
    let body = &token[..token.len() - padding];
    !body.is_empty() && body.iter().all(|&b| is_base64_byte(b))
}

/// Splits an optional IMAP command tag (e.g. `M1`) plus space off the start
/// of `line`, returning the remainder. The tag charset is `astring`-like:
/// base64 bytes plus `.`/`_`/`-`.
fn split_tag(line: &[u8]) -> Option<&[u8]> {
    let space = line.iter().position(|&b| b == b' ')?;
    let (tag, rest) = line.split_at(space);
    if !tag.is_empty()
        && tag.len() <= 32
        && tag
            .iter()
            .all(|&b| is_base64_byte(b) || matches!(b, b'.' | b'_' | b'-'))
    {
        Some(&rest[1..])
    } else {
        None
    }
}

/// If `line` (without terminator) is an IMAP `AUTHENTICATE <mech> <blob>` or
/// SMTP `AUTH <mech> <blob>` line carrying a SASL initial response, returns
/// the offset in `line` where the credential blob starts; `line[start..]` is
/// what gets redacted in the trace dump. Returns `None` when the line carries
/// no initial response (e.g. `AUTHENTICATE PLAIN` alone, which precedes a
/// separate continuation write) or is unrelated.
///
/// If the keyword is present but the mechanism/blob split is ambiguous, the
/// whole rest of the line is treated as the blob (conservative).
fn sasl_initial_response_start(line: &[u8]) -> Option<usize> {
    let candidates = std::iter::once((line, 0))
        .chain(split_tag(line).map(|tagged| (tagged, line.len() - tagged.len())));
    for (candidate, offset) in candidates {
        let keyword = if candidate.starts_with(b"AUTHENTICATE ") {
            &b"AUTHENTICATE "[..]
        } else if candidate.starts_with(b"AUTH ") {
            &b"AUTH "[..]
        } else {
            continue;
        };
        let after_keyword = &candidate[keyword.len()..];
        if let Some(space) = after_keyword.iter().position(|&b| b == b' ') {
            let start = offset + keyword.len() + space + 1;
            if start < line.len() {
                return Some(start);
            }
        }
        // A bare `AUTHENTICATE <mech>` line carries no credentials.
        return None;
    }
    None
}

/// Redacts SASL credentials from a client write buffer for trace logging.
///
/// Two leak shapes are handled, matching how the IMAP and SMTP frontends
/// actually write credentials:
///
/// 1. Lines carrying a SASL initial response: `AUTHENTICATE <mech> <base64>`
///    (IMAP SASL-IR, optionally prefixed by the command tag) and
///    `AUTH <mech> <base64>` (SMTP, RFC 4954). The payload after the
///    mechanism is replaced with [`SASL_REDACTED`]; an ambiguous split
///    redacts the rest of the line conservatively.
/// 2. Standalone base64 continuation writes: the credential literal meli
///    sends after an `AUTHENTICATE` continuation request (`send_literal`
///    writes the blob, then the CRLF separately), and the base64
///    username/password lines of SMTP `AUTH LOGIN`. A buffer that is exactly
///    one base64 token plus an optional line terminator is redacted whole:
///    this cannot be told apart from a genuine credential blob mid-stream,
///    and over-redacting a trace line is preferable to leaking a secret.
///
/// Buffers without any of these shapes are returned unchanged (borrowed), so
/// non-SASL writes render byte-identically. The bytes written to the wire are
/// never modified; this only shapes the trace dump.
fn redact_sasl_secrets(buf: &[u8]) -> Cow<'_, [u8]> {
    // Standalone base64 continuation write: redact whole.
    let content = strip_line_terminator(buf);
    if is_base64_token(content) {
        let mut out = Vec::with_capacity(SASL_REDACTED.len() + buf.len() - content.len());
        out.extend_from_slice(SASL_REDACTED);
        out.extend_from_slice(&buf[content.len()..]);
        return Cow::Owned(out);
    }
    // Line scan for AUTHENTICATE/AUTH initial-response lines.
    let mut out: Option<Vec<u8>> = None;
    let mut rest = buf;
    while !rest.is_empty() {
        let (line_end, mut content) = match rest.iter().position(|&b| b == b'\n') {
            Some(nl) => (nl + 1, &rest[..nl]),
            None => (rest.len(), rest),
        };
        if content.last() == Some(&b'\r') {
            content = &content[..content.len() - 1];
        }
        if let Some(start) = sasl_initial_response_start(content) {
            let out = out.get_or_insert_with(|| buf[..buf.len() - rest.len()].to_vec());
            out.extend_from_slice(&content[..start]);
            out.extend_from_slice(SASL_REDACTED);
            out.extend_from_slice(&rest[content.len()..line_end]);
        } else if let Some(out) = out.as_mut() {
            out.extend_from_slice(&rest[..line_end]);
        }
        rest = &rest[line_end..];
    }
    match out {
        Some(out) => Cow::Owned(out),
        None => Cow::Borrowed(buf),
    }
}

impl std::io::Write for Connection {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.is_trace_enabled() {
            let id = self.id();
            let redacted = redact_sasl_secrets(buf);
            log::trace!(
                "{}{}{}{:?} writing {} bytes:{}",
                if id.is_some() { "[" } else { "" },
                if let Some(id) = id.as_ref() { id } else { "" },
                if id.is_some() { "]: " } else { "" },
                self,
                buf.len(),
                std::str::from_utf8(&redacted)
                    .map(Cow::Borrowed)
                    .or_else(|_| crate::text::hex::bytes_to_hex(&redacted).map(Cow::Owned))
                    .unwrap_or(Cow::Borrowed("Could not convert to hex."))
            );
        }
        match self {
            Tcp { ref mut inner, .. } => inner.write(buf),
            #[cfg(feature = "tls")]
            Tls { ref mut inner, .. } => inner.write(buf),
            Fd { ref inner, .. } => {
                use std::os::unix::io::{FromRawFd, IntoRawFd};
                let mut f = unsafe { std::fs::File::from_raw_fd(inner.as_raw_fd()) };
                let ret = f.write(buf);
                let _ = f.into_raw_fd();
                ret
            }
            Deflate { ref mut inner, .. } => inner.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Tcp { ref mut inner, .. } => inner.flush(),
            #[cfg(feature = "tls")]
            Tls { ref mut inner, .. } => inner.flush(),
            Fd { ref inner, .. } => {
                use std::os::unix::io::{FromRawFd, IntoRawFd};
                let mut f = unsafe { std::fs::File::from_raw_fd(inner.as_raw_fd()) };
                let ret = f.flush();
                let _ = f.into_raw_fd();
                ret
            }
            Deflate { ref mut inner, .. } => inner.flush(),
        }
    }
}

impl std::os::unix::io::AsRawFd for Connection {
    fn as_raw_fd(&self) -> std::os::unix::io::RawFd {
        match self {
            Tcp { ref inner, .. } => inner.as_raw_fd(),
            #[cfg(feature = "tls")]
            Tls { ref inner, .. } => inner.get_ref().as_raw_fd(),
            Fd { ref inner, .. } => inner.as_raw_fd(),
            Deflate { ref inner, .. } => inner.get_ref().get_ref().as_raw_fd(),
        }
    }
}

impl AsFd for Connection {
    fn as_fd(&'_ self) -> BorrowedFd<'_> {
        match self {
            Tcp { ref inner, .. } => inner.as_fd(),
            #[cfg(feature = "tls")]
            Tls { ref inner, .. } => inner.get_ref().as_fd(),
            Fd { ref inner, .. } => inner.as_fd(),
            Deflate { ref inner, .. } => inner.get_ref().get_ref().as_fd(),
        }
    }
}

unsafe impl async_io::IoSafe for Connection {}

#[deprecated = "While it supports IPv6, it does not implement the happy eyeballs algorithm. Use \
                {std_net,smol}::tcp_stream_connect instead."]
pub fn lookup_ip(host: &str, port: u16) -> crate::Result<std::net::SocketAddr> {
    use std::net::ToSocketAddrs;

    use crate::error::{Error, ErrorKind, NetworkErrorKind};

    let addrs = (host, port).to_socket_addrs()?;
    for addr in addrs {
        if matches!(
            addr,
            std::net::SocketAddr::V4(_) | std::net::SocketAddr::V6(_)
        ) {
            return Ok(addr);
        }
    }

    Err(
        Error::new(format!("Could not lookup address {host}:{port}"))
            .set_kind(ErrorKind::Network(NetworkErrorKind::HostLookupFailed)),
    )
}

/// Test-only helpers for the server response size cap regression tests: a
/// real [`Connection`] over an OS socketpair whose peer plays a "malicious
/// server" writer thread, plus a timeout guard so that a regression makes
/// the test fail instead of hanging.
#[cfg(test)]
// The items below must be `pub(crate)` for the imap/nntp/smtp test modules;
// `clippy::redundant_pub_crate` misfires on `cfg(test)`-only modules.
#[expect(clippy::redundant_pub_crate)]
pub(crate) mod cap_test_utils {
    use std::{io::Write as _, os::unix::net::UnixStream, sync::mpsc, thread, time::Duration};

    use super::Connection;

    /// Returns one end of a socketpair wrapped in a melib [`Connection`],
    /// with a spawned writer thread playing a malicious server: it writes
    /// `chunk` repeatedly until roughly `budget` bytes have been written,
    /// and never sends anything else.
    ///
    /// The writer may block once the reader stops consuming; dropping the
    /// returned stream unblocks it (its writes then fail), so its join
    /// handle can be joined.
    pub(crate) fn malicious_server(
        chunk: &'static [u8],
        budget: usize,
    ) -> (smol::Async<Connection>, thread::JoinHandle<usize>) {
        let (reader_sock, mut writer_sock) = UnixStream::pair().unwrap();
        let conn = Connection::Fd {
            inner: reader_sock.into(),
            id: None,
            trace: false,
        };
        let stream = smol::Async::new(conn).unwrap();
        let writer = thread::spawn(move || {
            let mut written = 0usize;
            while written < budget {
                match writer_sock.write(chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => written += n,
                }
            }
            written
        });
        (stream, writer)
    }

    /// Returns one end of a socketpair wrapped in a melib [`Connection`],
    /// with a spawned peer that stays silent (but keeps the connection
    /// open) for `silence`, to exercise read timeouts.
    pub(crate) fn silent_server(
        silence: Duration,
    ) -> (smol::Async<Connection>, thread::JoinHandle<()>) {
        let (reader_sock, writer_sock) = UnixStream::pair().unwrap();
        let conn = Connection::Fd {
            inner: reader_sock.into(),
            id: None,
            trace: false,
        };
        let stream = smol::Async::new(conn).unwrap();
        let writer = thread::spawn(move || {
            thread::sleep(silence);
            drop(writer_sock);
        });
        (stream, writer)
    }

    /// Panics if `f` does not complete within `secs` seconds.
    pub(crate) fn assert_completes_within<F>(secs: u64, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let (tx, rx) = mpsc::channel::<()>();
        let handle = thread::spawn(move || {
            f();
            let _ = tx.send(());
        });
        match rx.recv_timeout(Duration::from_secs(secs)) {
            Ok(()) => handle.join().unwrap(),
            Err(_) => {
                if handle.is_finished() {
                    handle.join().unwrap();
                }
                panic!("test did not complete within {secs} seconds (hung?)");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enforce_response_size_limit() {
        enforce_response_size_limit(0).unwrap();
        enforce_response_size_limit(max_server_response_size()).unwrap();
        let err = enforce_response_size_limit(max_server_response_size() + 1).unwrap_err();
        assert!(
            matches!(
                err.kind,
                ErrorKind::Network(NetworkErrorKind::ProtocolViolation)
            ),
            "unexpected error: {err:?}"
        );
    }

    // Dummy SASL payload (base64 of "ABCDEFG"); never a real credential.
    const DUMMY_BLOB: &[u8] = b"QUJDREVGRw==";

    fn assert_absent(out: &[u8], marker: &[u8]) {
        assert!(
            !out.windows(marker.len()).any(|w| w == marker),
            "credential marker {marker:?} leaked into trace output: {out:?}"
        );
    }

    #[test]
    fn test_trace_redact_sasl_ir_line() {
        let buf = &b"M1 AUTHENTICATE PLAIN QUJDREVGRw==\r\n"[..];
        let out = redact_sasl_secrets(buf);
        assert_eq!(&*out, &b"M1 AUTHENTICATE PLAIN <REDACTED>\r\n"[..]);
        assert_absent(&out, DUMMY_BLOB);
    }

    #[test]
    fn test_trace_redact_sasl_ir_line_xoauth2_and_smtp() {
        let buf = &b"M1 AUTHENTICATE XOAUTH2 QUJDREVGRw==\r\n"[..];
        assert_eq!(
            &*redact_sasl_secrets(buf),
            &b"M1 AUTHENTICATE XOAUTH2 <REDACTED>\r\n"[..]
        );
        // SMTP SASL initial response (RFC 4954).
        let buf = &b"AUTH PLAIN QUJDREVGRw==\r\n"[..];
        assert_eq!(
            &*redact_sasl_secrets(buf),
            &b"AUTH PLAIN <REDACTED>\r\n"[..]
        );
    }

    #[test]
    fn test_trace_redact_sasl_continuation_blob() {
        // send_literal writes the blob alone, then the CRLF separately.
        assert_eq!(&*redact_sasl_secrets(DUMMY_BLOB), &b"<REDACTED>"[..]);
        assert_eq!(
            &*redact_sasl_secrets(b"QUJDREVGRw==\r\n"),
            &b"<REDACTED>\r\n"[..]
        );
        // The ANONYMOUS dummy continuation (8 bytes incl. padding).
        assert_eq!(&*redact_sasl_secrets(b"c2lyaGM="), &b"<REDACTED>"[..]);
    }

    #[test]
    fn test_trace_redact_bare_authenticate_untouched() {
        // No initial response: nothing to redact, byte-identical.
        let buf = &b"M1 AUTHENTICATE PLAIN\r\n"[..];
        assert!(matches!(redact_sasl_secrets(buf), Cow::Borrowed(_)));
        assert_eq!(&*redact_sasl_secrets(buf), buf);
        let buf = &b"AUTH LOGIN\r\n"[..];
        assert!(matches!(redact_sasl_secrets(buf), Cow::Borrowed(_)));
        assert_eq!(&*redact_sasl_secrets(buf), buf);
    }

    #[test]
    fn test_trace_redact_non_sasl_byte_identical() {
        for buf in [
            &b"M2 SELECT \"INBOX\"\r\n"[..],
            &b"M3 UID FETCH 1:* (FLAGS)\r\n"[..],
            &b"a CAPABILITY\r\n"[..],
            &b"MAIL FROM:<user@example.com>\r\n"[..],
            &b"M4 LIST \"\" *\r\n"[..],
        ] {
            let out = redact_sasl_secrets(buf);
            assert!(matches!(out, Cow::Borrowed(_)), "{buf:?} was copied");
            assert_eq!(&*out, buf);
        }
        assert!(matches!(redact_sasl_secrets(b""), Cow::Borrowed(_)));
    }

    #[test]
    fn test_trace_redact_malformed_and_split_shapes() {
        // A write chunk ending mid-mechanism carries no secret: unchanged.
        let buf = &b"M1 AUTHENTICATE PLA"[..];
        assert!(matches!(redact_sasl_secrets(buf), Cow::Borrowed(_)));
        assert_eq!(&*redact_sasl_secrets(buf), buf);
        // The follow-up blob chunk is a standalone base64 token: redacted.
        let out = redact_sasl_secrets(b"REVGRw==\r\n");
        assert_eq!(&*out, &b"<REDACTED>\r\n"[..]);
        assert_absent(&out, b"REVGRw==");
        // Blob split inside the IR line's single write: partial blob redacted.
        let out = redact_sasl_secrets(b"M1 AUTHENTICATE PLAIN QUJDREVGRw");
        assert_eq!(&*out, &b"M1 AUTHENTICATE PLAIN <REDACTED>"[..]);
        assert_absent(&out, b"QUJDREVGRw");
        // No keyword at all.
        let buf = &b"M1 NOOP"[..];
        assert!(matches!(redact_sasl_secrets(buf), Cow::Borrowed(_)));
    }

    #[test]
    fn test_trace_redact_mixed_lines_freeze() {
        let buf = &b"M1 NOOP\r\nM2 AUTHENTICATE PLAIN QUJDREVGRw==\r\nM3 LOGOUT\r\n"[..];
        let out = redact_sasl_secrets(buf);
        assert_eq!(
            &*out,
            &b"M1 NOOP\r\nM2 AUTHENTICATE PLAIN <REDACTED>\r\nM3 LOGOUT\r\n"[..]
        );
        assert_absent(&out, DUMMY_BLOB);
        // Non-SASL lines survive byte-identically around the redacted one.
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains("M1 NOOP\r\n"));
        assert!(s.contains("<REDACTED>\r\nM3 LOGOUT\r\n"));
    }
}
