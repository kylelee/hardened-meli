//
// meli
//
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

#![cfg(unix)]

use futures::prelude::*;
use smol::net::{TcpListener, TcpStream};
use tempfile::TempDir;

use crate::types::{ThreadEvent, UIEvent};

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
        crate::varlink::server::serve(listener, None, ctx.main_loop_handler.clone()),
        crate::jobs::IsAsync::Async,
    );
    let mut client = smol::block_on(TcpStream::connect(local_addr)).unwrap();
    tracing::trace!("connected");
    smol::block_on(client.write_all(b"{\"method\": \"org.varlink.service.GetInfo\"}\0")).unwrap();

    fn read_reply(client: &mut TcpStream) -> String {
        let mut output_buf = vec![];
        let mut output = [0u8; 5];
        loop {
            let b = smol::block_on(client.read(&mut output)).unwrap();
            if b == 0 {
                break;
            }
            output_buf.extend(output[0..b].iter().cloned());
            if output_buf.contains(&b'\0') {
                break;
            }
        }
        String::from_utf8_lossy(&output_buf).into_owned()
    }

    assert_eq!(
        &read_reply(&mut client),
        "{\"parameters\":{\"interfaces\":[\"org.meli-email.meli\",\"org.varlink.service\"],\"product\":\"meli\",\"url\":\"https://meli-email.org\",\"vendor\":\"meli\",\"version\":\"\"}}\0");
    smol::block_on(client.write_all(b"{\"method\": \"org.meli-email.meli.Command\"}\0")).unwrap();
    assert_eq!(
        &read_reply(&mut client),
        "{\"error\":\"invalid type: null, expected struct CommandRequest\"}\0"
    );
    // Send invalid json
    smol::block_on(client.write_all(b"}\0")).unwrap();
    assert_eq!(
        &read_reply(&mut client),
        "{\"error\":\"expected value at line 1 column 1\"}\0"
    );
    smol::block_on(client.write_all(
        b"{\"method\": \"org.meli-email.meli.Command\",\"parameters\":{\"command\":\"quit\"}}\0",
    ))
    .unwrap();
    assert_eq!(&read_reply(&mut client), "{\"parameters\":{}}\0");

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
    let server = ctx.main_loop_handler.job_executor.spawn(
        "varlink-server".into(),
        crate::varlink::server::serve(listener, None, ctx.main_loop_handler.clone()),
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

    fn read_reply(client: &mut smol::net::unix::UnixStream) -> String {
        let mut output_buf = vec![];
        let mut output = [0u8; 5];
        loop {
            let b = smol::block_on(client.read(&mut output)).unwrap();
            if b == 0 {
                break;
            }
            output_buf.extend(output[0..b].iter().cloned());
            if output_buf.contains(&b'\0') {
                break;
            }
        }
        String::from_utf8_lossy(&output_buf).into_owned()
    }

    assert_eq!(
        &read_reply(&mut client),
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
