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

#![cfg(unix)]

use futures::prelude::*;
use smol::net::{TcpListener, TcpStream};
use tempfile::TempDir;

use crate::types::{ThreadEvent, UIEvent};

/// Read exactly one NUL-terminated reply from `client`. Bytes past the
/// reply's NUL (e.g. the next queued reply) stay in `rest` for the
/// following call.
fn read_reply<S: futures::io::AsyncRead + Unpin>(client: &mut S, rest: &mut Vec<u8>) -> String {
    let mut output = [0u8; 5];
    loop {
        if let Some(pos) = rest.iter().position(|b| *b == b'\0') {
            let reply: Vec<u8> = rest.drain(..=pos).collect();
            return String::from_utf8_lossy(&reply).into_owned();
        }
        let b = smol::block_on(client.read(&mut output)).unwrap();
        assert!(b > 0, "connection closed before a full reply arrived");
        rest.extend(output[0..b].iter().cloned());
    }
}

#[test]
fn varlink() {
    let temp_dir = TempDir::new().unwrap();
    let ctx = crate::Context::new_mock(&temp_dir);

    let listener = smol::block_on(TcpListener::bind(("127.0.0.1", 0))).unwrap();
    let local_addr = listener.local_addr().unwrap();
    let addr = format!(
        "tcp:{ip}:{port}",
        ip = local_addr.ip(),
        port = local_addr.port()
    );

    let listener = crate::varlink::unix::TcpListener::from(listener);
    tracing::trace!("varlink listening on {addr}");
    let server = ctx.main_loop_handler.job_executor.spawn(
        "varlink-server".into(),
        crate::varlink::server::serve(addr, listener, None, ctx.main_loop_handler.clone()),
        crate::jobs::IsAsync::Async,
    );
    let mut client = smol::block_on(TcpStream::connect(local_addr)).unwrap();
    tracing::trace!("connected");
    smol::block_on(client.write_all(b"{\"method\": \"org.varlink.service.GetInfo\"}\0")).unwrap();

    let mut rest = Vec::new();
    assert_eq!(
        read_reply(&mut client, &mut rest),
        "{\"parameters\":{\"interfaces\":[\"org.meli-email.meli\",\"org.varlink.service\"],\"product\":\"meli\",\"url\":\"https://meli-email.org\",\"vendor\":\"meli\",\"version\":\"\"}}\0");
    smol::block_on(client.write_all(b"{\"method\": \"org.meli-email.meli.Command\"}\0")).unwrap();
    assert_eq!(
        read_reply(&mut client, &mut rest),
        "{\"error\":\"invalid type: null, expected struct CommandRequest\"}\0"
    );
    // Send invalid json
    smol::block_on(client.write_all(b"}\0")).unwrap();
    assert_eq!(
        read_reply(&mut client, &mut rest),
        "{\"error\":\"expected value at line 1 column 1\"}\0"
    );
    smol::block_on(client.write_all(
        b"{\"method\": \"org.meli-email.meli.Command\",\"parameters\":{\"command\":\"quit\"}}\0",
    ))
    .unwrap();
    assert_eq!(read_reply(&mut client, &mut rest), "{\"parameters\":{}}\0");

    let mut events = vec![];
    while let Ok(ev) = ctx.receiver.try_recv() {
        events.push(ev);
    }

    assert!(
        events
            .iter()
            .filter(|t| matches!(t, ThreadEvent::UIEvent(UIEvent::Command(ref c)) if c == "quit"))
            .count()
            == 1
    );

    drop(server);

    _ = temp_dir.close();
}

/// Smoke test over a filesystem UNIX socket: exercises `new_listener`'
/// `unix:` address parsing, the listener's socket-file removal on drop,
/// and the standard `org.varlink.service.GetInfo` reply.
#[test]
fn varlink_unix_socket() {
    let temp_dir = TempDir::new().unwrap();
    let ctx = crate::Context::new_mock(&temp_dir);
    let sock_path = temp_dir.path().join("org.meli-email.meli.sock");
    let addr = format!("unix:{}", sock_path.display());

    let listener = crate::varlink::server::new_listener(&addr, false).unwrap();

    // R1: the filesystem socket must be owner-only the moment it exists;
    // this is the varlink server's local-access guarantee (see
    // `crate::varlink::unix::UnixListener::new`).
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&sock_path).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "filesystem varlink socket must not be accessible by other users"
        );
    }

    let server = ctx.main_loop_handler.job_executor.spawn(
        "varlink-server".into(),
        crate::varlink::server::serve(addr, listener, None, ctx.main_loop_handler.clone()),
        crate::jobs::IsAsync::Async,
    );

    // The listener binds asynchronously on the job executor; wait for the
    // socket file to show up before connecting.
    let mut bound = false;
    for _ in 0..40 {
        if sock_path.exists() {
            bound = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(bound, "listener did not bind the socket file in time");

    let mut client = smol::block_on(smol::net::unix::UnixStream::connect(&sock_path)).unwrap();
    smol::block_on(client.write_all(b"{\"method\": \"org.varlink.service.GetInfo\"}\0")).unwrap();

    let mut rest = Vec::new();
    assert_eq!(
        read_reply(&mut client, &mut rest),
        "{\"parameters\":{\"interfaces\":[\"org.meli-email.meli\",\"org.varlink.service\"],\"product\":\"meli\",\"url\":\"https://meli-email.org\",\"vendor\":\"meli\",\"version\":\"\"}}\0");

    drop(client);
    drop(server);

    // Dropping the server's handle cancels it; its listener must unlink
    // the socket file.
    let mut removed = false;
    for _ in 0..40 {
        if !sock_path.exists() {
            removed = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(removed, "socket file was not removed after server drop");

    _ = temp_dir.close();
}

/// R4: a single read may carry several NUL-framed requests; the handler must
/// answer each one in order instead of discarding the bytes past the first
/// NUL.
#[test]
fn varlink_two_messages_in_one_read() {
    let temp_dir = TempDir::new().unwrap();
    let ctx = crate::Context::new_mock(&temp_dir);

    let listener = smol::block_on(TcpListener::bind(("127.0.0.1", 0))).unwrap();
    let local_addr = listener.local_addr().unwrap();
    let addr = format!(
        "tcp:{ip}:{port}",
        ip = local_addr.ip(),
        port = local_addr.port()
    );

    let listener = crate::varlink::unix::TcpListener::from(listener);
    let server = ctx.main_loop_handler.job_executor.spawn(
        "varlink-server".into(),
        crate::varlink::server::serve(addr, listener, None, ctx.main_loop_handler.clone()),
        crate::jobs::IsAsync::Async,
    );

    let mut client = smol::block_on(TcpStream::connect(local_addr)).unwrap();
    smol::block_on(client.write_all(
        b"{\"method\": \"org.varlink.service.GetInfo\"}\0\
          {\"method\": \"org.varlink.service.GetInfo\"}\0",
    ))
    .unwrap();

    let expected = "{\"parameters\":{\"interfaces\":[\"org.meli-email.meli\",\"org.varlink.service\"],\"product\":\"meli\",\"url\":\"https://meli-email.org\",\"vendor\":\"meli\",\"version\":\"\"}}\0";
    let mut rest = Vec::new();
    assert_eq!(read_reply(&mut client, &mut rest), expected);
    assert_eq!(read_reply(&mut client, &mut rest), expected);

    drop(client);
    drop(server);

    _ = temp_dir.close();
}

/// R2: connection jobs are registered in the shared `ipc_jobs` table when
/// spawned, so processing every connection job's `JobFinished` (in any
/// interleaving with the `Ipc` wakeups) must drain the table to zero —
/// handles can no longer be orphaned by event reordering.
#[test]
fn varlink_ipc_job_table_drains() {
    let temp_dir = TempDir::new().unwrap();
    let ctx = crate::Context::new_mock(&temp_dir);

    let listener = smol::block_on(TcpListener::bind(("127.0.0.1", 0))).unwrap();
    let local_addr = listener.local_addr().unwrap();
    let addr = format!(
        "tcp:{ip}:{port}",
        ip = local_addr.ip(),
        port = local_addr.port()
    );

    let listener = crate::varlink::unix::TcpListener::from(listener);
    let server = ctx.main_loop_handler.job_executor.spawn(
        "varlink-server".into(),
        crate::varlink::server::serve(addr, listener, None, ctx.main_loop_handler.clone()),
        crate::jobs::IsAsync::Async,
    );

    const CLIENTS: usize = 8;
    let mut rest = Vec::new();
    for _ in 0..CLIENTS {
        let mut client = smol::block_on(TcpStream::connect(local_addr)).unwrap();
        smol::block_on(client.write_all(b"{\"method\": \"org.varlink.service.GetInfo\"}\0"))
            .unwrap();
        read_reply(&mut client, &mut rest);
        drop(client);
    }

    // Drive the main loop's bookkeeping: each varlink connection job's
    // completion arrives as `ThreadEvent::JobFinished`; remove its handle
    // from the shared table exactly like the main loop does.
    let mut finished = 0;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while finished < CLIENTS {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {CLIENTS} connection jobs to finish"
        );
        match ctx
            .receiver
            .recv_timeout(std::time::Duration::from_secs(1))
        {
            Ok(ThreadEvent::JobFinished(id)) => {
                ctx.main_loop_handler.job_executor.set_job_finished(id);
                if ctx.main_loop_handler.ipc_jobs.remove(&id).is_some() {
                    finished += 1;
                }
            }
            Ok(_) => {}
            Err(_) => {}
        }
    }
    assert!(
        ctx.main_loop_handler.ipc_jobs.is_empty(),
        "every connection job's handle must be removed once its JobFinished is processed"
    );

    drop(server);

    _ = temp_dir.close();
}
