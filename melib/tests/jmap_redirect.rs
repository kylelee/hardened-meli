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
// SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later

//! Regression tests for the JMAP HTTP client redirect policy (audit finding
//! C7r, CWE-522).
//!
//! The JMAP client must never let `isahc`/libcurl follow redirects
//! automatically: curl engine-managed authentication
//! (`.authentication()`, i.e. `CURLOPT_HTTPAUTH`) and `default_header`
//! credentials (re-attached by isahc's `DefaultHeaders` interceptor) are
//! re-sent to cross-origin redirect targets, leaking Bearer tokens and Basic
//! credentials. Redirects are followed manually by
//! [`melib::jmap::connection::JmapConnection`] and only within the same
//! origin.
//!
//! These tests use loopback listeners on 127.0.0.1 with ephemeral ports. A
//! different port is a different origin, so no external network is involved.
//! Ground truth is what the listeners observe, not the client's return value.

#![cfg(feature = "jmap")]

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use futures::executor::block_on;
use melib::{
    backends::prelude::*,
    jmap::{connection::JmapConnection, JmapServerConf, OnlineStatus, Store},
};
use url::Url;

/// Synthetic credentials; never printed by assertions.
const TOKEN: &str = "SYNTHETIC-JMAP-TOKEN-poc";
const BASIC_USER: &str = "poc-user";
const BASIC_PASS: &str = "SYNTHETIC-BASIC-PASS-poc";

/// Upper bound for any listener thread before it gives up.
const LISTENER_DEADLINE: Duration = Duration::from_secs(5);
/// Grace period after the client call returns, for stray connections to
/// arrive at a listener we expect to be never contacted.
const GRACE: Duration = Duration::from_millis(500);

/// Sanitized record of one HTTP request received by a listener.
///
/// Only the request line and the Authorization *scheme* (not the value) are
/// kept, so no credential material can leak into logs or assertions.
#[derive(Debug, Default, Clone)]
struct Receipt {
    request_line: String,
    auth_scheme: Option<String>,
}

fn read_request(stream: &mut TcpStream) -> Receipt {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let raw = String::from_utf8_lossy(&buf).to_string();
    let request_line = raw.lines().next().unwrap_or_default().to_string();
    let auth_scheme = raw.lines().find_map(|line| {
        let lower = line.to_ascii_lowercase();
        let value = lower.strip_prefix("authorization:")?;
        let value = value.trim();
        let scheme = value.split_whitespace().next().unwrap_or_default();
        if scheme.is_empty() {
            Some("other".to_string())
        } else {
            Some(scheme.to_string())
        }
    });
    Receipt {
        request_line,
        auth_scheme,
    }
}

/// Serve exactly one HTTP request per entry in `responses`, one connection
/// each. Exits when all responses are served or the deadline passes.
fn spawn_responder(
    listener: TcpListener,
    responses: Vec<String>,
) -> thread::JoinHandle<Vec<Receipt>> {
    thread::spawn(move || {
        let deadline = Instant::now() + LISTENER_DEADLINE;
        let mut receipts = Vec::with_capacity(responses.len());
        listener
            .set_nonblocking(true)
            .expect("set_nonblocking on responder listener");
        for response in responses {
            loop {
                if Instant::now() > deadline {
                    return receipts;
                }
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_millis(500)))
                            .ok();
                        let receipt = read_request(&mut stream);
                        stream.write_all(response.as_bytes()).ok();
                        stream.flush().ok();
                        receipts.push(receipt);
                        break;
                    }
                    Err(ref err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => return receipts,
                }
            }
        }
        receipts
    })
}

/// Watch a listener that must never be contacted. If a connection does
/// arrive, serve it a 200 so the client completes quickly, and record the
/// (sanitized) request for the leak assertion.
fn spawn_watcher(listener: TcpListener) -> Arc<Mutex<Vec<Receipt>>> {
    let hits = Arc::new(Mutex::new(Vec::new()));
    let ret = Arc::clone(&hits);
    thread::spawn(move || {
        let deadline = Instant::now() + LISTENER_DEADLINE;
        listener
            .set_nonblocking(true)
            .expect("set_nonblocking on watcher listener");
        loop {
            if Instant::now() > deadline {
                return;
            }
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_read_timeout(Some(Duration::from_millis(500)))
                        .ok();
                    let receipt = read_request(&mut stream);
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                        )
                        .ok();
                    stream.flush().ok();
                    ret.lock().unwrap().push(receipt);
                }
                Err(ref err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(_) => return,
            }
        }
    });
    hits
}

fn test_store() -> Arc<Store> {
    Arc::new(Store {
        account_name: Arc::new("jmap-redirect-poc".to_string()),
        account_hash: AccountHash::from_bytes(b"jmap-redirect-poc"),
        main_identity: "poc@example.com".to_string(),
        extra_identities: vec![],
        byte_cache: Arc::new(FutureMutex::new(Default::default())),
        id_store: Arc::new(FutureMutex::new(Default::default())),
        reverse_id_store: Arc::new(FutureMutex::new(Default::default())),
        blob_id_store: Arc::new(FutureMutex::new(Default::default())),
        collection: Collection::default(),
        mailboxes: Default::default(),
        mailboxes_index: Default::default(),
        mailbox_state: Arc::new(FutureMutex::new(None)),
        email_state: Arc::new(FutureMutex::new(None)),
        online_status: OnlineStatus(Arc::new(FutureMutex::new((
            Instant::now(),
            Err(Error::new("Account is uninitialised.")),
        )))),
        is_subscribed: IsSubscribedFn::default(),
        core_capabilities: Arc::new(Mutex::new(Default::default())),
        metadata: Arc::new(Mutex::new(Default::default())),
        event_consumer: BackendEventConsumer::new(Arc::new(
            |_account_hash: AccountHash, _event: BackendEvent| {},
        )),
    })
}

/// Build a `JmapConnection` exactly as production code does, pointing at the
/// loopback origin `http://127.0.0.1:{origin_port}`.
fn test_conn(origin_port: u16, use_token: bool) -> JmapConnection {
    let server_conf = JmapServerConf {
        server_url: Url::parse(&format!("http://127.0.0.1:{origin_port}")).unwrap(),
        server_username: BASIC_USER.to_string(),
        server_password: if use_token {
            TOKEN.to_string()
        } else {
            BASIC_PASS.to_string()
        },
        use_token,
        danger_accept_invalid_certs: false,
        timeout: Some(Duration::from_secs(5)),
    };
    JmapConnection::new(&server_conf, test_store()).expect("build JmapConnection")
}

fn origin_url(origin_port: u16) -> Url {
    Url::parse(&format!("http://127.0.0.1:{origin_port}/.well-known/jmap")).unwrap()
}

fn redirect_response(location: &str) -> String {
    format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
}

fn ok_response() -> String {
    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_string()
}

/// Cross-origin (different port) redirect with Bearer-token configuration:
/// the client must return an error *before* dialing the redirect target, so
/// the target observes zero connections.
#[test]
fn jmap_redirect_cross_origin_bearer_refused() {
    let origin = TcpListener::bind("127.0.0.1:0").unwrap();
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin_port = origin.local_addr().unwrap().port();
    let target_port = target.local_addr().unwrap().port();
    assert_ne!(origin_port, target_port, "ports must differ (cross-origin)");

    let responder = spawn_responder(
        origin,
        vec![redirect_response(&format!(
            "http://127.0.0.1:{target_port}/steal"
        ))],
    );
    let watcher = spawn_watcher(target);

    let conn = test_conn(origin_port, true);
    let result = block_on(conn.get_async(&origin_url(origin_port)));

    let err = result.expect_err("cross-origin redirect must be refused");
    assert!(
        err.to_string().contains("different origin"),
        "unexpected error message: {err}"
    );

    thread::sleep(GRACE);
    let hits = watcher.lock().unwrap().clone();
    assert!(
        hits.is_empty(),
        "LEAK: cross-origin target received {} connection(s): {hits:?}",
        hits.len()
    );
    let receipts = responder.join().unwrap();
    assert_eq!(receipts.len(), 1, "origin must have served one request");
}

/// Cross-origin redirect with Basic-auth configuration: same guarantee, the
/// redirect target observes zero connections (curl forced auth never gets to
/// run because the redirected request is never issued).
#[test]
fn jmap_redirect_cross_origin_basic_refused() {
    let origin = TcpListener::bind("127.0.0.1:0").unwrap();
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin_port = origin.local_addr().unwrap().port();
    let target_port = target.local_addr().unwrap().port();

    let responder = spawn_responder(
        origin,
        vec![redirect_response(&format!(
            "http://127.0.0.1:{target_port}/steal"
        ))],
    );
    let watcher = spawn_watcher(target);

    let conn = test_conn(origin_port, false);
    let result = block_on(conn.get_async(&origin_url(origin_port)));

    let err = result.expect_err("cross-origin redirect must be refused");
    assert!(
        err.to_string().contains("different origin"),
        "unexpected error message: {err}"
    );

    thread::sleep(GRACE);
    let hits = watcher.lock().unwrap().clone();
    assert!(
        hits.is_empty(),
        "LEAK: cross-origin target received {} connection(s): {hits:?}",
        hits.len()
    );
    let receipts = responder.join().unwrap();
    assert_eq!(receipts.len(), 1);
}

/// The POST API path (`post_async`) must enforce the same policy.
#[test]
fn jmap_redirect_cross_origin_post_refused() {
    let origin = TcpListener::bind("127.0.0.1:0").unwrap();
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin_port = origin.local_addr().unwrap().port();
    let target_port = target.local_addr().unwrap().port();

    let responder = spawn_responder(
        origin,
        vec![redirect_response(&format!(
            "http://127.0.0.1:{target_port}/steal"
        ))],
    );
    let watcher = spawn_watcher(target);

    let conn = test_conn(origin_port, true);
    let api_url = Url::parse(&format!("http://127.0.0.1:{origin_port}/jmap/")).unwrap();
    let result = block_on(conn.post_async(Some(&api_url), r#"{"using":[]}"#));

    let err = result.expect_err("cross-origin redirect must be refused");
    assert!(
        err.to_string().contains("different origin"),
        "unexpected error message: {err}"
    );

    thread::sleep(GRACE);
    let hits = watcher.lock().unwrap().clone();
    assert!(
        hits.is_empty(),
        "LEAK: cross-origin target received {} connection(s): {hits:?}",
        hits.len()
    );
    assert_eq!(
        responder.join().unwrap().len(),
        1,
        "origin must have served the POST request"
    );
}

/// Same-origin redirect (same host+port, different path) must keep working,
/// and the followed request must still carry the Bearer credentials.
#[test]
fn jmap_redirect_same_origin_bearer_followed() {
    let origin = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin_port = origin.local_addr().unwrap().port();

    let responder = spawn_responder(
        origin,
        vec![
            redirect_response(&format!("http://127.0.0.1:{origin_port}/session")),
            ok_response(),
        ],
    );

    let conn = test_conn(origin_port, true);
    let resp = block_on(conn.get_async(&origin_url(origin_port)))
        .expect("same-origin redirect must be followed");
    assert!(resp.status().is_success());

    let receipts = responder.join().unwrap();
    assert_eq!(receipts.len(), 2, "origin must have served both requests");
    assert_eq!(
        receipts[1].request_line, "GET /session HTTP/1.1",
        "followed request must target the redirect location: {:?}",
        receipts[1]
    );
    assert_eq!(
        receipts[1].auth_scheme.as_deref(),
        Some("bearer"),
        "followed same-origin request must carry Bearer credentials: {:?}",
        receipts[1]
    );
}

/// Same-origin redirect with Basic-auth configuration: followed, and the
/// followed request still carries Basic credentials.
#[test]
fn jmap_redirect_same_origin_basic_followed() {
    let origin = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin_port = origin.local_addr().unwrap().port();

    let responder = spawn_responder(
        origin,
        vec![
            redirect_response(&format!("http://127.0.0.1:{origin_port}/session")),
            ok_response(),
        ],
    );

    let conn = test_conn(origin_port, false);
    let resp = block_on(conn.get_async(&origin_url(origin_port)))
        .expect("same-origin redirect must be followed");
    assert!(resp.status().is_success());

    let receipts = responder.join().unwrap();
    assert_eq!(receipts.len(), 2);
    assert_eq!(
        receipts[1].request_line, "GET /session HTTP/1.1",
        "followed request must target the redirect location: {:?}",
        receipts[1]
    );
    assert_eq!(
        receipts[1].auth_scheme.as_deref(),
        Some("basic"),
        "followed same-origin request must carry Basic credentials: {:?}",
        receipts[1]
    );
}

/// A same-origin redirect loop must terminate with an error after a bounded
/// number of hops instead of looping forever.
#[test]
fn jmap_redirect_same_origin_loop_is_capped() {
    let origin = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin_port = origin.local_addr().unwrap().port();
    // The client allows at most 5 followed redirects, i.e. at most 6
    // requests before giving up.
    let responses: Vec<String> = (0..6)
        .map(|_| redirect_response(&format!("http://127.0.0.1:{origin_port}/loop")))
        .collect();
    let responder = spawn_responder(origin, responses);

    let conn = test_conn(origin_port, true);
    let result = block_on(conn.get_async(&origin_url(origin_port)));

    let err = result.expect_err("redirect loop must terminate with an error");
    assert!(
        err.to_string().to_ascii_lowercase().contains("redirect"),
        "unexpected error message: {err}"
    );
    let receipts = responder.join().unwrap();
    assert!(
        receipts.len() <= 6,
        "client made {} requests, redirect cap not enforced",
        receipts.len()
    );
}

/// A `Location` header that is not a valid URL must be rejected with an
/// error, not a panic, and must not be followed anywhere.
#[test]
fn jmap_redirect_malformed_location_rejected() {
    let origin = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin_port = origin.local_addr().unwrap().port();

    let responder = spawn_responder(origin, vec![redirect_response("http://[")]);

    let conn = test_conn(origin_port, true);
    let result = block_on(conn.get_async(&origin_url(origin_port)));

    let err = result.expect_err("malformed redirect location must be an error");
    assert!(
        !err.to_string().contains(TOKEN),
        "error message must not contain credential material"
    );
    assert_eq!(responder.join().unwrap().len(), 1);
}
