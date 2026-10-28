//
// meli
//
// Copyright 2026 Manos Pitsidianakis <manos@pitsidianak.is>
// Fork modifications Copyright 2026 Kyle Lee <kylelee@users.noreply.github.com>
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

use std::{pin::Pin, time::Duration};

use futures::{
    future::{self, BoxFuture, Either},
    io::{AsyncRead, AsyncWrite},
};
use melib::error::{Error, ErrorKind, Result};

use crate::varlink::handler;

/// A bidirectional byte stream with a varlink client.
pub trait Stream: AsyncRead + AsyncWrite + Send + Sync {
    fn shutdown(&mut self) -> Result<()>;
    fn clone(&self) -> Pin<Box<dyn Stream>>;
}

/// Something that can accept varlink client connections.
pub trait Listener: Send + 'static {
    fn accept(&self) -> BoxFuture<'_, Result<Pin<Box<dyn Stream>>>>;
}

impl Listener for Box<dyn Listener> {
    fn accept(&self) -> BoxFuture<'_, Result<Pin<Box<dyn Stream>>>> {
        (**self).accept()
    }
}

#[cfg(unix)]
/// See <https://varlink.org/#activation>
pub fn detect_listen_socket() -> Option<usize> {
    let listen_fds = std::env::var("LISTEN_FDS").ok()?;

    let n_fds = listen_fds.parse::<usize>().ok()?;
    if n_fds == 0 {
        return None;
    }
    if !matches!(std::env::var("LISTEN_PID"), Ok(pid) if pid.parse::<usize>() == Ok(std::process::id() as usize))
    {
        return None;
    }

    if n_fds == 1 {
        return Some(3);
    }

    std::env::var("LISTEN_FDNAMES")
        .ok()?
        .split(':')
        .enumerate()
        .find_map(|(i, name)| if name == "varlink" { Some(i + 3) } else { None })
}

#[cfg(not(unix))]
pub fn new_listener(_: &str, _: bool) -> Result<Box<dyn Listener>> {
    Err(Error::new("Sockets not implemented in this platform").set_kind(ErrorKind::NotImplemented))
}

#[cfg(unix)]
/// Create a varlink listener for `addr`.
///
/// Recognized address formats: `unix:<path>`, `unix:@<abstract_name>`
/// (Linux only) and `tcp:<host>:<port>`. When `replace` is given, an
/// existing filesystem socket at `unix:<path>` is removed first.
pub fn new_listener(addr: &str, replace: bool) -> Result<Box<dyn Listener>> {
    use crate::varlink::unix;

    let listen_socket = detect_listen_socket();
    if let Some(addr) = addr.strip_prefix("unix:@") {
        Ok(Box::new(unix::UnixListener::new_abstract(
            addr,
            listen_socket,
            replace,
        )?))
    } else if let Some(addr) = addr.strip_prefix("unix:") {
        Ok(Box::new(unix::UnixListener::new(
            addr,
            listen_socket,
            replace,
        )?))
    } else if let Some(addr) = addr.strip_prefix("tcp:") {
        Ok(Box::new(unix::TcpListener::new(addr, listen_socket)?))
    } else {
        Err(Error::new("Unrecognized address format")
            .set_details(format!(
                "Address {addr} is not recognized. Example values: `tcp:127.0.0.1:12345`, \
                 `unix:/run/org.meli-email.meli`, `unix:@org.meli-email.meli`"
            ))
            .set_kind(ErrorKind::ValueError))
    }
}

/// Accept clients from `addr`'s listener forever.
///
/// Each connection is handled in its own job on the main loop's job
/// executor. `timeout` bounds how long the listener waits for a first
/// client before giving up.
pub async fn serve<L: Listener>(
    addr: String,
    listener: L,
    timeout: Option<Duration>,
    main_loop_handler: crate::state::MainLoopHandler,
) -> Result<()> {
    loop {
        let conn = if let Some(timeout) = timeout {
            match future::select(listener.accept(), smol::Timer::after(timeout)).await {
                Either::Left((out, _)) => out,
                Either::Right(_) => {
                    tracing::debug!("varlink listener on {addr}: timed out waiting for a client");
                    return Err(Error::new("Timed out").set_kind(ErrorKind::TimedOut));
                }
            }
        } else {
            listener.accept().await
        };
        let conn = match conn {
            Ok(conn) => conn,
            Err(err) => {
                tracing::error!(
                    "varlink listener on {addr}: could not accept client connection: {err}"
                );
                return Err(err);
            }
        };
        let _main_loop_handler = main_loop_handler.clone();
        let job = main_loop_handler.job_executor.spawn(
            "varlink-connection".into(),
            async move {
                let handler = handler::Handler::new(_main_loop_handler, conn);
                handler.handle().await
            },
            true.into(),
        );
        // Register the job's handle in the shared table *synchronously*,
        // before the job can possibly complete: its `JobFinished` event may
        // reach the main loop before the `Ipc` wakeup below, so inserting
        // from the event would orphan the handle and leak the job. The
        // `Ipc` event is now only bookkeeping/wakeup.
        main_loop_handler.ipc_jobs.insert(job.job_id, job);
        main_loop_handler.send(crate::types::ThreadEvent::Ipc);
    }
}
