//
// meli
//
// Copyright 2025 PoC engineers (security audit scratch harness)
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
// along with meli.  If not, see <http://www.gnu.org/licenses/>.
//
// SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later

//! C6 regression: SMTP reply parsing must not panic on hostile short lines.
//!
//! `Reply::new` used to slice `&l[4..l.len()]` for every line without a
//! length check, so a truncated final line (e.g. `b"250"` with the stream
//! closed right after, reachable via `read_lines`) panicked with a slice
//! index error. Short lines now degrade to empty text lines; well-formed
//! replies parse byte-identically (pinned below).

#[test]
fn test_c6_reply_new_short_lines_degrade_instead_of_panicking() {
    let code = melib::smtp::ReplyCode::try_from("250").unwrap();

    // 3-byte final line (`b"250"`, stream closed): one empty text line.
    let reply = melib::smtp::Reply::new("250", code);
    assert_eq!(reply.code, code);
    assert_eq!(
        reply.lines.iter().copied().collect::<Vec<_>>(),
        vec![""],
        "truncated 3-byte line must degrade to an empty text line"
    );

    // 1-byte line (`b"2"`): same degradation.
    let reply = melib::smtp::Reply::new("2", code);
    assert_eq!(
        reply.lines.iter().copied().collect::<Vec<_>>(),
        vec![""],
        "1-byte line must degrade to an empty text line"
    );

    // Empty reply: no lines at all.
    let reply = melib::smtp::Reply::new("", code);
    assert!(reply.lines.is_empty());

    // 4-byte prefix-only line (`b"250-"`, stream closed): empty text.
    let reply = melib::smtp::Reply::new("250-", code);
    assert_eq!(reply.lines.iter().copied().collect::<Vec<_>>(), vec![""]);

    // Multi-line reply with a bare short code in the middle.
    let reply = melib::smtp::Reply::new("250-ok\r\n221\r\n250 done\r\n", code);
    assert_eq!(
        reply.lines.iter().copied().collect::<Vec<_>>(),
        vec!["ok", "", "done"],
        "short middle line must degrade to an empty text line"
    );
}

#[test]
fn test_c6_reply_new_wellformed_unchanged() {
    let code = melib::smtp::ReplyCode::try_from("250").unwrap();
    let reply = melib::smtp::Reply::new("250 ok\r\n", code);
    assert_eq!(reply.lines.iter().copied().collect::<Vec<_>>(), vec!["ok"]);
    let reply = melib::smtp::Reply::new("250-ok\r\n250 ok\r\n", code);
    assert_eq!(
        reply.lines.iter().copied().collect::<Vec<_>>(),
        vec!["ok", "ok"]
    );
    let reply = melib::smtp::Reply::new("250-\r\n250 ok\r\n", code);
    assert_eq!(
        reply.lines.iter().copied().collect::<Vec<_>>(),
        vec!["", "ok"]
    );
}
