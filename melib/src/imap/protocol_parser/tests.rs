//
// meli
//
// Copyright 2017- Emmanouil Pitsidianakis <manos@pitsidianak.is>
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

use super::*;

macro_rules! to_str (
    ($v:expr) => (unsafe{ std::str::from_utf8_unchecked($v) })
);

#[test]
fn test_imap_response() {
    assert_eq!(ImapResponse::try_from(&b"M12 NO [CANNOT] Invalid mailbox name: Name must not have \'/\' characters (0.000 + 0.098 + 0.097 secs).\r\n"[..]).unwrap(),
        ImapResponse::No(ResponseCode::Alert("[CANNOT] Invalid mailbox name: Name must not have '/' characters".to_string())));

    assert_eq!(
        ImapResponse::try_from(&b"M13 OK [UIDNEXT 4392] Predicted next UID\r\n"[..]).unwrap(),
        ImapResponse::Ok(ResponseCode::Uidnext(4392))
    );

    assert_eq!(
        ImapResponse::try_from(&b"M14 OK [UIDVALIDITY 3857529045] UIDs valid\r\n"[..]).unwrap(),
        ImapResponse::Ok(ResponseCode::Uidvalidity(3857529045))
    );
}

/// `LIST`/`LSUB` wire names are mUTF-7 (RFC 3501 §5.1.3). The parser must
/// keep `imap_path`/`hash`/`parent` in wire form while exposing decoded
/// UTF-8 `path`/`name` to the rest of the application.
#[test]
fn test_imap_list_mailbox_result_decodes_mutf7() {
    // `已发送` in mUTF-7 is `&XfJT0ZAB-`: UTF-16BE bytes `5d f2 53 d1 90 01`
    // -> base64 `XfJT0ZAB` (`/` mapped to `,`), computed independently with
    // python3. The full wire path uses `.` as the hierarchy separator.
    let wire = "INBOX.&XfJT0ZAB-";
    for verb in ["LIST", "LSUB"] {
        let input = format!("* {verb} (\\HasNoChildren) \".\" {wire}\r\n");
        let (rest, f) = list_mailbox_result(input.as_bytes()).unwrap();
        assert!(rest.is_empty(), "{verb}: unparsed trailing bytes: {rest:?}");
        // Wire-format fields are untouched: the sqlite3 cache keys and every
        // command sent to the server depend on `imap_path`/`hash`/`parent`.
        assert_eq!(f.imap_path, wire);
        assert_eq!(f.hash, MailboxHash::from_bytes(wire.as_bytes()));
        assert_eq!(f.parent, Some(MailboxHash::from_bytes(b"INBOX")));
        // Display fields are decoded UTF-8.
        assert_eq!(f.path, "INBOX/已发送");
        assert_eq!(f.name, "已发送");
    }
}

#[test]
fn test_imap_required_responses() {
    assert!(RequiredResponses::NO.check(b"M12 NO [CANNOT] Invalid something\r\n"));
    {
        let mut ret = Vec::new();
        let required_responses = RequiredResponses::FETCH_UID | RequiredResponses::FETCH_FLAGS;
        let response =
            &b"* 1040 FETCH (UID 1064 FLAGS ())\r\nM15 OK Fetch completed (0.001 + 0.299 secs).\r\n"[..];
        for l in response.split_rn() {
            if required_responses.check(l) {
                ret.extend_from_slice(l);
            }
        }
        assert_eq!(ret.as_slice(), &b"* 1040 FETCH (UID 1064 FLAGS ())\r\n"[..]);
        let v = protocol_parser::uid_fetch_flags_responses(response)
            .unwrap()
            .1;
        assert_eq!(v.len(), 1);
    }
    assert!(RequiredResponses::CAPABILITY.check(b"* CAPABILITY IMAP4rev1\r\n"));
    assert!(RequiredResponses::BYE.check(b"* BYE love\r\n"));
    assert!(RequiredResponses::FLAGS
        .check(b"* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n"));
    assert!(RequiredResponses::EXISTS.check(b"* 172 EXISTS\r\n"));
    assert!(RequiredResponses::RECENT.check(b"* 0 RECENT\r\n"));
    assert!(RequiredResponses::UNSEEN.check(b"* OK [UNSEEN 12] Message 12 is first unseen\r\n"));
    assert!(RequiredResponses::PERMANENTFLAGS
        .check(b"* OK [PERMANENTFLAGS (\\Deleted \\Seen \\*)] Limited\r\n"));
    assert!(RequiredResponses::UIDNEXT.check(b"* OK [UIDNEXT 4392] Predicted next UID\r\n"));
    assert!(RequiredResponses::UIDVALIDITY.check(b"* OK [UIDVALIDITY 3857529045] UIDs valid\r\n"));
    assert!(RequiredResponses::LIST.check(b"* LIST (\\HasChildren) \".\" INBOX\r\n"));
    assert!(RequiredResponses::LSUB.check(b"* LSUB (\\HasChildren) \".\" INBOX\r\n"));
    assert!(RequiredResponses::STATUS.check(b"* STATUS INBOX (MESSAGES 1057 UNSEEN 0)\r\n"));
    assert!(RequiredResponses::SEARCH.check(b"* SEARCH 1\r\n"));
    assert!(RequiredResponses::SEARCH.check(b"* SEARCH\r\n"));
    {
        let fetch =
            b"* 1429 FETCH (UID 1505 FLAGS (\\Seen) RFC822 {26}\r\nReturn-Path: <blah blah...\r\n";
        assert!(RequiredResponses::FETCH_UID.check(fetch));
        assert!(RequiredResponses::FETCH_FLAGS.check(fetch));
        assert!(RequiredResponses::FETCH_BODY.check(fetch));
        assert!((RequiredResponses::FETCH_UID | RequiredResponses::FETCH_FLAGS).check(fetch));
        assert!((RequiredResponses::FETCH_UID
            | RequiredResponses::FETCH_FLAGS
            | RequiredResponses::FETCH_BODY)
            .check(fetch));
        assert!((RequiredResponses::FETCH_UID | RequiredResponses::FETCH_BODY).check(fetch));
        assert!(!RequiredResponses::FETCH_ENVELOPE.check(fetch));
        assert!(!(RequiredResponses::FETCH_UID | RequiredResponses::FETCH_ENVELOPE).check(fetch));
    }
    {
        let modseq_fetch = b"* 1079 FETCH (UID 1103 MODSEQ (1365) FLAGS (\\Seen))\r\n";
        assert!(RequiredResponses::FETCH_MODSEQ.check(modseq_fetch));
        assert!((RequiredResponses::FETCH_UID
            | RequiredResponses::FETCH_MODSEQ
            | RequiredResponses::FETCH_FLAGS)
            .check(modseq_fetch));
        assert!(!RequiredResponses::FETCH_ENVELOPE.check(modseq_fetch));
        assert!(
            !(RequiredResponses::FETCH_UID | RequiredResponses::FETCH_ENVELOPE).check(modseq_fetch)
        );
    }

    {
        let body_fetch =
            b"* 1429 FETCH (UID 1505 FLAGS (\\Seen) RFC822 {26}\r\nReturn-Path: <blah blah...\r\n";
        assert!(RequiredResponses::FETCH_UID.check(body_fetch));
        assert!(RequiredResponses::FETCH_FLAGS.check(body_fetch));
        assert!(RequiredResponses::FETCH_BODY.check(body_fetch));
        assert!((RequiredResponses::FETCH_UID
            | RequiredResponses::FETCH_FLAGS
            | RequiredResponses::FETCH_BODY)
            .check(body_fetch));
        assert!(!RequiredResponses::FETCH_ENVELOPE.check(body_fetch));
        assert!(
            !(RequiredResponses::FETCH_UID | RequiredResponses::FETCH_ENVELOPE).check(body_fetch)
        );
    }
    {
        let full_fetch =
                    b"* 198 FETCH (UID 7608 FLAGS (\\Seen) ENVELOPE (\"Fri, 24 Jun 2011 10:09:10 +0000\" \"xxxx/xxxx\" ((\"xx@xx.com\" NIL \"xx\" \"xx.com\")) NIL NIL ((\"xx@xx\" NIL \"xx\" \"xx.com\")) ((\"'xx, xx'\" NIL \"xx.xx\" \"xx.com\")(\"xx.xx@xx.com\" NIL \"xx.xx\" \"xx.com\")(\"'xx'\" NIL \"xx.xx\" \"xx.com\")(\"'xx xx'\" NIL \"xx.xx\" \"xx.com\")(\"xx.xx@xx.com\" NIL \"xx.xx\" \"xx.com\")) NIL NIL \"<xx@xx.com>\") BODY[HEADER.FIELDS (REFERENCES)] {2}\r\n\r\n BODYSTRUCTURE ((\"text\" \"html\" (\"charset\" \"us-ascii\") \"<xx@xx>\" NIL \"7BIT\" 17236 232 NIL NIL NIL NIL)(\"image\" \"jpeg\" (\"name\" \"image001.jpg\") \"<image001.jpg@xx.xx>\" \"image001.jpg\" \"base64\" 1918 NIL (\"inline\" (\"filename\" \"image001.jpg\" \"size\" \"1650\" \"creation-date\" \"Sun, 09 Aug 2015 20:56:04 GMT\" \"modification-date\" \"Sun, 14 Aug 2022 22:11:45 GMT\")) NIL NIL) \"related\" (\"boundary\" \"xx--xx\" \"type\" \"text/html\") NIL \"en-US\"))\r\n";
        assert!(RequiredResponses::FETCH_REFERENCES.check(full_fetch));
        assert!(RequiredResponses::FETCH_BODYSTRUCTURE.check(full_fetch));
        assert!(RequiredResponses::FETCH_ENVELOPE.check(full_fetch));
    }
    {
        let fetch = b"* 2700 FETCH (UID 3223 FLAGS (\\Seen) ENVELOPE (\"Wed, 28 Aug 2024 13:53:11 +0000\" \"=?utf-8?Q?Update:=20Let's=20Talk!?=\" ((\"Newsletter\" NIL \"no-reply\" \"example.com\")) ((\"Newsletter\" NIL \"no-reply\" \"example.com\")) ((\"Newsletter\" NIL \"no-reply\" \"example.com\")) ((NIL NIL \"user\" \"example.com\")) NIL NIL NIL \"<XXXXXXXX.YYYYYYYYYYYYYY.ZZZZZZZZZZZZZZ@HHHH.TLD>\") BODY[HEADER.FIELDS (REFERENCES)] {2}\r\n\r\n BODYSTRUCTURE ((\"text\" \"plain\" (\"charset\" \"utf-8\") NIL NIL \"7bit\" 824 25 NIL NIL NIL NIL)(\"text\" \"html\" (\"charset\" \"utf-8\") NIL NIL \"7bit\" 28037 418 NIL NIL NIL NIL) \"alternative\" (\"boundary\" \"__\") NIL NIL NIL)\r\n";
        let required_responses = RequiredResponses::FETCH_UID
            | RequiredResponses::FETCH_FLAGS
            | RequiredResponses::FETCH_ENVELOPE
            | RequiredResponses::FETCH_REFERENCES
            | RequiredResponses::FETCH_BODYSTRUCTURE;
        assert!(required_responses.check(fetch));
        let lines = fetch.split_rn().collect::<Vec<_>>();
        assert_eq!(
            lines.len(),
            1,
            "Line was not split correctly by ImapLineIterator: {:?}",
            lines.iter().map(|l| to_str!(l)).collect::<Vec<&str>>()
        );
        let mut ret = Vec::new();
        for l in lines {
            if required_responses.check(l) {
                ret.extend_from_slice(l);
            } else {
                panic!("Unexpected response: {:?}", to_str!(l));
            }
        }
        assert_eq!(to_str!(fetch), to_str!(&ret));
    }
}

/// Regression test for the `RequiredResponses::NO` zero-valued bit-flag
/// bug: `NO` must be a real bit, otherwise `intersects` is always `false`
/// for it and the expected-`NO` paths (the `read_response` guard and
/// `Connection::unselect`'s RFC 3691 fallback) silently never match.
#[test]
fn test_imap_required_responses_no_is_nonzero_bit() {
    assert_ne!(
        RequiredResponses::NO.bits(),
        0,
        "NO must be a real bit; a zero-valued flag makes intersects() always false"
    );
    assert!(RequiredResponses::NO.intersects(RequiredResponses::NO));
    assert!(
        !RequiredResponses::empty().intersects(RequiredResponses::NO),
        "empty() must not imply the expected-NO flag"
    );
    assert!(!RequiredResponses::CAPABILITY.intersects(RequiredResponses::NO));
    // A bare `NO` still matches a tagged `NO` line and nothing else.
    assert!(RequiredResponses::NO.check(b"M12 NO [CANNOT] Invalid something\r\n"));
    assert!(!RequiredResponses::NO.check(b"M12 OK done\r\n"));
    assert!(
        !RequiredResponses::empty().check(b"M12 NO [CANNOT] Invalid something\r\n"),
        "empty() must not treat a tagged NO as an expected response"
    );
}

#[test]
fn test_imap_line_iterator() {
    {
        let s = b"* 1429 FETCH (UID 1505 FLAGS (\\Seen) RFC822 {26}\r\nReturn-Path: <blah blah...)\r\n* 1430 FETCH (UID 1506 FLAGS (\\Seen))\r\n* 1431 FETCH (UID 1507 FLAGS (\\Seen))\r\n* 1432 FETCH (UID 1500 FLAGS (\\Seen) RFC822 {4}\r\nnull)\r\n";
        let line_a =
            b"* 1429 FETCH (UID 1505 FLAGS (\\Seen) RFC822 {26}\r\nReturn-Path: <blah blah...)\r\n";
        let line_b = b"* 1430 FETCH (UID 1506 FLAGS (\\Seen))\r\n";
        let line_c = b"* 1431 FETCH (UID 1507 FLAGS (\\Seen))\r\n";
        let line_d = b"* 1432 FETCH (UID 1500 FLAGS (\\Seen) RFC822 {4}\r\nnull)\r\n";

        let mut iter = s.split_rn();

        assert_eq!(to_str!(iter.next().unwrap()), to_str!(line_a));
        assert_eq!(to_str!(iter.next().unwrap()), to_str!(line_b));
        assert_eq!(to_str!(iter.next().unwrap()), to_str!(line_c));
        assert_eq!(to_str!(iter.next().unwrap()), to_str!(line_d));
        assert!(iter.next().is_none());
    }

    {
        let s = b"* 23 FETCH (FLAGS (\\Seen) RFC822.SIZE 44827)\r\n";
        let mut iter = s.split_rn();
        assert_eq!(to_str!(iter.next().unwrap()), to_str!(s));
        assert!(iter.next().is_none());
    }

    {
        let s = b"";
        let mut iter = s.split_rn();
        assert!(iter.next().is_none());
    }
    {
        let s = b"* 172 EXISTS\r\n* 1 RECENT\r\n* OK [UNSEEN 12] Message 12 is first unseen\r\n* OK [UIDVALIDITY 3857529045] UIDs valid\r\n* OK [UIDNEXT 4392] Predicted next UID\r\n* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n* OK [PERMANENTFLAGS (\\Deleted \\Seen \\*)] Limited\r\n* OK [NOMODSEQ] Sorry, this mailbox format doesn't support modsequences\r\n* A142 OK [READ-WRITE] SELECT completed\r\n";
        let mut iter = s.split_rn();
        for l in &[
            &b"* 172 EXISTS\r\n"[..],
            &b"* 1 RECENT\r\n"[..],
            &b"* OK [UNSEEN 12] Message 12 is first unseen\r\n"[..],
            &b"* OK [UIDVALIDITY 3857529045] UIDs valid\r\n"[..],
            &b"* OK [UIDNEXT 4392] Predicted next UID\r\n"[..],
            &b"* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n"[..],
            &b"* OK [PERMANENTFLAGS (\\Deleted \\Seen \\*)] Limited\r\n"[..],
            &b"* OK [NOMODSEQ] Sorry, this mailbox format doesn't support modsequences\r\n"[..],
            &b"* A142 OK [READ-WRITE] SELECT completed\r\n"[..],
        ] {
            assert_eq!(to_str!(iter.next().unwrap()), to_str!(l));
        }
        assert!(iter.next().is_none());
    }
    {
        let s = b"* 2700 FETCH (UID 3223 FLAGS (\\Seen) ENVELOPE (\"Wed, 28 Aug 2024 13:53:11 +0000\" \"=?utf-8?Q?Update:=20Let's=20Talk!?=\" ((\"Newsletter\" NIL \"no-reply\" \"example.com\")) ((\"Newsletter\" NIL \"no-reply\" \"example.com\")) ((\"Newsletter\" NIL \"no-reply\" \"example.com\")) ((NIL NIL \"user\" \"example.com\")) NIL NIL NIL \"<XXXXXXXX.YYYYYYYYYYYYYY.ZZZZZZZZZZZZZZ@HHHH.TLD>\") BODY[HEADER.FIELDS (REFERENCES)] {2}\r\n\r\n BODYSTRUCTURE ((\"text\" \"plain\" (\"charset\" \"utf-8\") NIL NIL \"7bit\" 824 25 NIL NIL NIL NIL)(\"text\" \"html\" (\"charset\" \"utf-8\") NIL NIL \"7bit\" 28037 418 NIL NIL NIL NIL) \"alternative\" (\"boundary\" \"__\") NIL NIL NIL)\r\n";
        let mut iter = s.split_rn();
        assert_eq!(to_str!(iter.next().unwrap()), to_str!(s));
        assert!(iter.next().is_none());
    }
    {
        let s = b"{6}\r\n\r\n\r\n\r\n rest\r\n";
        let mut iter = s.split_rn();
        assert_eq!(to_str!(iter.next().unwrap()), to_str!(s));
        assert!(iter.next().is_none());
    }
    {
        let s = b"{6}\r\n\r\n\r\n\r\n first\r\nsecond not a literal{ 5}\r\n";
        let mut iter = s.split_rn();
        for l in &[
            &b"{6}\r\n\r\n\r\n\r\n first\r\n"[..],
            &b"second not a literal{ 5}\r\n"[..],
        ] {
            assert_eq!(to_str!(iter.next().unwrap()), to_str!(l));
        }
        assert!(iter.next().is_none());
    }
}

#[test]
fn test_imap_untagged_responses() {
    use UntaggedResponse::*;
    assert_eq!(
        untagged_responses(b"* 2 EXISTS\r\n")
            .map(|(_, v, _)| v)
            .unwrap()
            .unwrap(),
        Exists(2)
    );
    assert_eq!(
        untagged_responses(b"* 1079 FETCH (UID 1103 MODSEQ (1365) FLAGS (\\Seen))\r\n")
            .map(|(_, v, _)| v)
            .unwrap()
            .unwrap(),
        Fetch(Box::new(FetchResponse {
            uid: Some(1103),
            message_sequence_number: 1079,
            modseq: Some(ModSequence(std::num::NonZeroU64::new(1365_u64).unwrap())),
            flags: Some((Flag::SEEN, vec![])),
            body: None,
            references: None,
            envelope: None,
            bodystructure: false,
            raw_fetch_value: &b"* 1079 FETCH (UID 1103 MODSEQ (1365) FLAGS (\\Seen))\r\n"[..],
        }))
    );
    assert_eq!(
        untagged_responses(b"* 1 FETCH (FLAGS (\\Seen))\r\n")
            .map(|(_, v, _)| v)
            .unwrap()
            .unwrap(),
        Fetch(Box::new(FetchResponse {
            uid: None,
            message_sequence_number: 1,
            modseq: None,
            flags: Some((Flag::SEEN, vec![])),
            body: None,
            references: None,
            envelope: None,
            bodystructure: false,
            raw_fetch_value: &b"* 1 FETCH (FLAGS (\\Seen))\r\n"[..],
        }))
    );
}

#[test]
fn test_imap_fetch_response() {
    {
        #[rustfmt::skip]
    let input: &[u8] = b"* 198 FETCH (UID 7608 FLAGS (\\Seen) ENVELOPE (\"Fri, 24 Jun 2011 10:09:10 +0000\" \"xxxx/xxxx\" ((\"xx@xx.com\" NIL \"xx\" \"xx.com\")) NIL NIL ((\"xx@xx\" NIL \"xx\" \"xx.com\")) ((\"'xx, xx'\" NIL \"xx.xx\" \"xx.com\")(\"xx.xx@xx.com\" NIL \"xx.xx\" \"xx.com\")(\"'xx'\" NIL \"xx.xx\" \"xx.com\")(\"'xx xx'\" NIL \"xx.xx\" \"xx.com\")(\"xx.xx@xx.com\" NIL \"xx.xx\" \"xx.com\")) NIL NIL \"<xx@xx.com>\") BODY[HEADER.FIELDS (REFERENCES)] {2}\r\n\r\n BODYSTRUCTURE ((\"text\" \"html\" (\"charset\" \"us-ascii\") \"<xx@xx>\" NIL \"7BIT\" 17236 232 NIL NIL NIL NIL)(\"image\" \"jpeg\" (\"name\" \"image001.jpg\") \"<image001.jpg@xx.xx>\" \"image001.jpg\" \"base64\" 1918 NIL (\"inline\" (\"filename\" \"image001.jpg\" \"size\" \"1650\" \"creation-date\" \"Sun, 09 Aug 2015 20:56:04 GMT\" \"modification-date\" \"Sun, 14 Aug 2022 22:11:45 GMT\")) NIL NIL) \"related\" (\"boundary\" \"xx--xx\" \"type\" \"text/html\") NIL \"en-US\"))\r\n";
        let mut address = SmallVec::new();
        address.push(Address::new(None::<&str>, "xx@xx.com"));
        let mut env = Envelope::new(EnvelopeHash::default());
        env.set_subject(b"xxxx/xxxx".to_vec());
        env.set_date(b"Fri, 24 Jun 2011 10:09:10 +0000");
        env.set_from(address.clone());
        env.set_to(address);
        env.set_message_id(b"<xx@xx.com>");
        assert_eq!(
            fetch_response(input).unwrap(),
            (
                &b""[..],
                FetchResponse {
                    uid: Some(7608),
                    message_sequence_number: 198,
                    flags: Some((Flag::SEEN, vec![])),
                    modseq: None,
                    body: None,
                    references: Some(b""),
                    envelope: Some(env),
                    bodystructure: true,
                    raw_fetch_value: input,
                },
                vec![]
            )
        );
    }
    {
        // Same as above, but the server quotes the header field name in the
        // `BODY[HEADER.FIELDS ("REFERENCES")]` response, as Zoho does. An empty
        // `References` header must still yield `Some("")` and not `None`,
        // otherwise the response fails `RequiredResponses::FETCH_REFERENCES` and
        // the envelope is silently dropped.
        #[rustfmt::skip]
    let input: &[u8] = b"* 198 FETCH (UID 7608 FLAGS (\\Seen) ENVELOPE (\"Fri, 24 Jun 2011 10:09:10 +0000\" \"xxxx/xxxx\" ((\"xx@xx.com\" NIL \"xx\" \"xx.com\")) NIL NIL ((\"xx@xx\" NIL \"xx\" \"xx.com\")) NIL NIL NIL \"<xx@xx.com>\") BODY[HEADER.FIELDS (\"REFERENCES\")] {2}\r\n\r\n BODYSTRUCTURE ((\"text\" \"html\" (\"charset\" \"us-ascii\") \"<xx@xx>\" NIL \"7BIT\" 17236 232 NIL NIL NIL NIL) \"related\" (\"boundary\" \"xx--xx\" \"type\" \"text/html\") NIL \"en-US\"))\r\n";
        let mut address = SmallVec::new();
        address.push(Address::new(None::<&str>, "xx@xx.com"));
        let mut env = Envelope::new(EnvelopeHash::default());
        env.set_subject(b"xxxx/xxxx".to_vec());
        env.set_date(b"Fri, 24 Jun 2011 10:09:10 +0000");
        env.set_from(address.clone());
        env.set_to(address);
        env.set_message_id(b"<xx@xx.com>");
        assert_eq!(
            fetch_response(input).unwrap(),
            (
                &b""[..],
                FetchResponse {
                    uid: Some(7608),
                    message_sequence_number: 198,
                    flags: Some((Flag::SEEN, vec![])),
                    modseq: None,
                    body: None,
                    references: Some(b""),
                    envelope: Some(env),
                    bodystructure: true,
                    raw_fetch_value: input,
                },
                vec![]
            )
        );
    }
    {
        let input = b"* 1429 FETCH (UID 1505 FLAGS (\\Seen))\r\n* 1430 FETCH (UID 1506 FLAGS (\\Seen))\r\n* OK Searched 99% of the mailbox, ETA 0:00\r\n* 1431 FETCH (UID 1507 FLAGS (\\Seen))\r\n* 1432 FETCH (UID 1500 FLAGS (\\Seen) RFC822 {4}\r\nnull\r\n)\r\n";
        assert_eq!(
            fetch_responses(input).unwrap(),
            (
                &b""[..],
                vec![
                    FetchResponse {
                        uid: Some(1505),
                        message_sequence_number: 1429,
                        modseq: None,
                        flags: Some((Flag::SEEN, vec![])),
                        body: None,
                        references: None,
                        envelope: None,
                        bodystructure: false,
                        raw_fetch_value: &b"* 1429 FETCH (UID 1505 FLAGS (\\Seen))\r\n"[..]
                    },
                    FetchResponse {
                        uid: Some(1506),
                        message_sequence_number: 1430,
                        modseq: None,
                        flags: Some((Flag::SEEN, vec![])),
                        body: None,
                        references: None,
                        envelope: None,
                        bodystructure: false,
                        raw_fetch_value: &b"* 1430 FETCH (UID 1506 FLAGS (\\Seen))\r\n"[..]
                    },
                    FetchResponse {
                        uid: Some(1507),
                        message_sequence_number: 1431,
                        modseq: None,
                        flags: Some((Flag::SEEN, vec![])),
                        body: None,
                        references: None,
                        envelope: None,
                        bodystructure: false,
                        raw_fetch_value: &b"* 1431 FETCH (UID 1507 FLAGS (\\Seen))\r\n"[..]
                    },
                    FetchResponse {
                        uid: Some(1500),
                        message_sequence_number: 1432,
                        modseq: None,
                        flags: Some((Flag::SEEN, vec![])),
                        body: Some(&b"null"[..]),
                        references: None,
                        envelope: None,
                        bodystructure: false,
                        raw_fetch_value:
                            &b"* 1432 FETCH (UID 1500 FLAGS (\\Seen) RFC822 {4}\r\nnull\r\n)\r\n"[..]
                    }
                ],
                vec![ImapResponse::Ok(ResponseCode::Alert(
                    "Searched 99% of the mailbox, ETA 0:00".to_string()
                ))]
            )
        );
    }
}

#[test]
fn test_imap_uid_fetch_envelopes_response_optional_bodystructure() {
    // With `fetch_body_structure = false`, servers reply without a
    // BODYSTRUCTURE item; the parser must accept both shapes. When the
    // item is absent, `has_attachments` stays at its default (`false`).
    #[rustfmt::skip]
    let with_bodystructure: &[u8] = b"* 198 FETCH (UID 7608 FLAGS (\\Seen) ENVELOPE (\"Fri, 24 Jun 2011 10:09:10 +0000\" \"xxxx/xxxx\" ((\"xx@xx.com\" NIL \"xx\" \"xx.com\")) NIL NIL ((\"xx@xx\" NIL \"xx\" \"xx.com\")) NIL NIL NIL \"<xx@xx.com>\") BODYSTRUCTURE ((\"text\" \"plain\" (\"charset\" \"us-ascii\") NIL NIL \"7BIT\" 5 1)(\"image\" \"png\" (\"name\" \"x.png\") \"<x.png>\" \"x.png\" \"base64\" 1918 (\"attachment\" (\"filename\" \"x.png\")) NIL NIL) \"mixed\"))\r\n";
    let (rest, v) = uid_fetch_envelopes_response(with_bodystructure).unwrap();
    assert!(rest.is_empty());
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].0, 7608);
    assert_eq!(v[0].1, Some((Flag::SEEN, vec![])));
    assert!(v[0].2.has_attachments());

    #[rustfmt::skip]
    let without_bodystructure: &[u8] = b"* 198 FETCH (UID 7608 FLAGS (\\Seen) ENVELOPE (\"Fri, 24 Jun 2011 10:09:10 +0000\" \"xxxx/xxxx\" ((\"xx@xx.com\" NIL \"xx\" \"xx.com\")) NIL NIL ((\"xx@xx\" NIL \"xx\" \"xx.com\")) NIL NIL NIL \"<xx@xx.com>\"))\r\n";
    let (rest, v) = uid_fetch_envelopes_response(without_bodystructure).unwrap();
    assert!(rest.is_empty());
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].0, 7608);
    assert!(!v[0].2.has_attachments());
}

#[test]
fn test_imap_fetch_responses_without_bodystructure() {
    // Same response shape, parsed with the generic `fetch_responses` used
    // by the fetch/resync code paths.
    #[rustfmt::skip]
    let without_bodystructure: &[u8] = b"* 198 FETCH (UID 7608 FLAGS (\\Seen) ENVELOPE (\"Fri, 24 Jun 2011 10:09:10 +0000\" \"xxxx/xxxx\" ((\"xx@xx.com\" NIL \"xx\" \"xx.com\")) NIL NIL ((\"xx@xx\" NIL \"xx\" \"xx.com\")) NIL NIL NIL \"<xx@xx.com>\") BODY[HEADER.FIELDS (REFERENCES)] {2}\r\n\r\n)\r\n";
    let (rest, v, _) = fetch_responses(without_bodystructure).unwrap();
    assert!(rest.is_empty());
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].uid, Some(7608));
    assert!(v[0].envelope.is_some());
    assert!(!v[0].bodystructure);
    assert!(!v[0].envelope.as_ref().unwrap().has_attachments());

    // The light `RequiredResponses` set (what
    // `common_attributes_light()` requests) must accept the response,
    // while a set containing `FETCH_BODYSTRUCTURE` must reject it.
    let light = RequiredResponses::FETCH_UID
        | RequiredResponses::FETCH_FLAGS
        | RequiredResponses::FETCH_ENVELOPE
        | RequiredResponses::FETCH_REFERENCES;
    assert!(light.check(without_bodystructure));
    assert!(!(light | RequiredResponses::FETCH_BODYSTRUCTURE).check(without_bodystructure));
}

#[test]
fn test_imap_search() {
    assert_eq!(search_results(b"* SEARCH\r\n").map(|(_, v)| v), Ok(vec![]));
    assert_eq!(
        search_results(b"* SEARCH 1\r\n").map(|(_, v)| v),
        Ok(vec![1])
    );
    assert_eq!(
        search_results(b"* SEARCH 1 2 3 4\r\n").map(|(_, v)| v),
        Ok(vec![1, 2, 3, 4])
    );
    assert_eq!(
        search_results_raw(b"* SEARCH 1 2 3 4\r\n").map(|(_, v)| v),
        Ok(&b"1 2 3 4"[..])
    );
}

#[test]
fn test_imap_select_response() {
    let r = b"* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n* OK [PERMANENTFLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft \\*)] Flags permitted.\r\n* 45 EXISTS\r\n* 0 RECENT\r\n* OK [UNSEEN 16] First unseen.\r\n* OK [UIDVALIDITY 1554422056] UIDs valid\r\n* OK [UIDNEXT 50] Predicted next UID\r\n";

    assert_eq!(
        select_response(r).expect("Could not parse IMAP select response"),
        SelectResponse {
            exists: 45,
            recent: 0,
            flags: (
                Flag::REPLIED | Flag::SEEN | Flag::TRASHED | Flag::DRAFT | Flag::FLAGGED,
                Vec::new()
            ),
            first_unseen: 16,
            uidvalidity: 1554422056,
            uidnext: 50,
            permanentflags: (
                Flag::REPLIED | Flag::SEEN | Flag::TRASHED | Flag::DRAFT | Flag::FLAGGED,
                vec!["*".into()]
            ),
            can_create_flags: true,
            read_only: false,
            highestmodseq: None
        }
    );
    let r = b"* 172 EXISTS\r\n* 1 RECENT\r\n* OK [UNSEEN 12] Message 12 is first unseen\r\n* OK [UIDVALIDITY 3857529045] UIDs valid\r\n* OK [UIDNEXT 4392] Predicted next UID\r\n* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n* OK [PERMANENTFLAGS (\\Deleted \\Seen \\*)] Limited\r\n* OK [HIGHESTMODSEQ 715194045007]\r\n* A142 OK [READ-WRITE] SELECT completed\r\n";

    assert_eq!(
        select_response(r).expect("Could not parse IMAP select response"),
        SelectResponse {
            exists: 172,
            recent: 1,
            flags: (
                Flag::REPLIED | Flag::SEEN | Flag::TRASHED | Flag::DRAFT | Flag::FLAGGED,
                Vec::new()
            ),
            first_unseen: 12,
            uidvalidity: 3857529045,
            uidnext: 4392,
            permanentflags: (Flag::SEEN | Flag::TRASHED, vec!["*".into()]),
            can_create_flags: true,
            read_only: false,
            highestmodseq: Some(Ok(ModSequence(
                std::num::NonZeroU64::new(715194045007_u64).unwrap()
            ))),
        }
    );
    let r = b"* 172 EXISTS\r\n* 1 RECENT\r\n* OK [UNSEEN 12] Message 12 is first unseen\r\n* OK [UIDVALIDITY 3857529045] UIDs valid\r\n* OK [UIDNEXT 4392] Predicted next UID\r\n* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)\r\n* OK [PERMANENTFLAGS (\\Deleted \\Seen \\*)] Limited\r\n* OK [NOMODSEQ] Sorry, this mailbox format doesn't support modsequences\r\n* A142 OK [READ-WRITE] SELECT completed\r\n";

    assert_eq!(
        select_response(r).expect("Could not parse IMAP select response"),
        SelectResponse {
            exists: 172,
            recent: 1,
            flags: (
                Flag::REPLIED | Flag::SEEN | Flag::TRASHED | Flag::DRAFT | Flag::FLAGGED,
                Vec::new()
            ),
            first_unseen: 12,
            uidvalidity: 3857529045,
            uidnext: 4392,
            permanentflags: (Flag::SEEN | Flag::TRASHED, vec!["*".into()]),
            can_create_flags: true,
            read_only: false,
            highestmodseq: Some(Err(())),
        }
    );
}

#[test]
fn test_imap_select_response_malformed_terminators() {
    // A server-supplied select response line missing its terminator must yield
    // a parse error instead of panicking on an unwrapped `find()`.
    assert_eq!(
        select_response(b"* OK [UNSEEN ").unwrap_err().kind,
        ErrorKind::ProtocolError
    );
    assert_eq!(
        select_response(b"* OK [UNSEEN \r\n").unwrap_err().kind,
        ErrorKind::ProtocolError
    );
    assert_eq!(
        select_response(b"* OK [UIDVALIDITY ").unwrap_err().kind,
        ErrorKind::ProtocolError
    );
    assert_eq!(
        select_response(b"* OK [UIDVALIDITY \r\n").unwrap_err().kind,
        ErrorKind::ProtocolError
    );
    assert_eq!(
        select_response(b"* OK [UIDNEXT ").unwrap_err().kind,
        ErrorKind::ProtocolError
    );
    assert_eq!(
        select_response(b"* OK [UIDNEXT \r\n").unwrap_err().kind,
        ErrorKind::ProtocolError
    );
    assert_eq!(
        select_response(b"* OK [PERMANENTFLAGS (").unwrap_err().kind,
        ErrorKind::ProtocolError
    );
    assert_eq!(
        select_response(b"* OK [PERMANENTFLAGS (\r\n")
            .unwrap_err()
            .kind,
        ErrorKind::ProtocolError
    );
    // Positive cases: the terminators are present.
    assert_eq!(
        select_response(b"* OK [UNSEEN 16] First unseen.\r\n")
            .expect("Could not parse IMAP select response")
            .first_unseen,
        16
    );
    assert_eq!(
        select_response(b"* OK [UIDVALIDITY 1554422056] UIDs valid\r\n")
            .expect("Could not parse IMAP select response")
            .uidvalidity,
        1554422056
    );
    assert_eq!(
        select_response(b"* OK [UIDNEXT 50] Predicted next UID\r\n")
            .expect("Could not parse IMAP select response")
            .uidnext,
        50
    );
    assert_eq!(
        select_response(b"* OK [PERMANENTFLAGS (\\Deleted \\Seen \\*)] Limited\r\n")
            .expect("Could not parse IMAP select response")
            .permanentflags,
        (Flag::SEEN | Flag::TRASHED, vec!["*".into()])
    );
}

#[test]
fn test_imap_envelope() {
    let input: &[u8] = b"(\"Fri, 24 Jun 2011 10:09:10 +0000\" \"xxxx/xxxx\" ((\"xx@xx.com\" NIL \"xx\" \"xx.com\")) NIL NIL ((\"xx@xx\" NIL \"xx\" \"xx.com\")) ((\"'xx, xx'\" NIL \"xx.xx\" \"xx.com\") (\"xx.xx@xx.com\" NIL \"xx.xx\" \"xx.com\") (\"'xx'\" NIL \"xx.xx\" \"xx.com\") (\"'xx xx'\" NIL \"xx.xx\" \"xx.com\") (\"xx.xx@xx.com\" NIL \"xx.xx\" \"xx.com\")) NIL NIL \"<xx@xx.com>\")";
    _ = envelope(input).unwrap();
}

#[test]
fn test_imap_envelope_qq_mail_quoted_local_part_message_id() {
    // QQ Mail relays a Message-ID header with a quoted local part, e.g.
    // `<605067.JavaMail."billing@example.com"@example.center>`, as several
    // adjacent quoted strings in the ENVELOPE, which is not a single valid
    // nstring. The parser must accept it and concatenate the parts.
    let input: &[u8] = b"(\"Wed, 20 May 2026 12:34:30 +0800\" \"subject\" ((\"name\" NIL \"user\" \"example.com\")) NIL NIL ((\"name\" NIL \"user\" \"example.com\")) NIL NIL \"<reply.\"l@p\"@d>\" \"<605067.8696.JavaMail.\"billing@example.com\"@example.center.na620>\")";
    let (rest, env) = envelope(input).unwrap();
    assert!(rest.is_empty());
    assert_eq!(
        env.message_id().to_string(),
        "605067.8696.JavaMail.billing@example.com@example.center.na620"
    );
}

#[test]
fn test_imap_envelope_address() {
    assert_eq!(
        envelope_address(b"\"=?UTF-8?Q?=CE=A6_Info_Dr=2E_Grifter_=26_Associates?=\" NIL \"info\" \"example.com\"").unwrap(),
        (
            &[][..],
            AddressValue::Address(Address::new(
                Some("Φ Info Dr. Grifter & Associates".to_string()),
                "info@example.com".to_string()
            ))
        )
    );

    assert_eq!(
        envelope_address(br#"NIL NIL "undisclosed-recipients" NIL"#).unwrap(),
        (
            &[][..],
            AddressValue::GroupStart(b"undisclosed-recipients".to_vec()),
        )
    );

    assert_eq!(
        envelope_address(b"NIL NIL NIL NIL").unwrap(),
        (&[][..], AddressValue::GroupEnd,)
    );

    assert_eq!(
        envelope_addresses(br#"((NIL NIL "undisclosed-recipients" NIL)(NIL NIL NIL NIL))"#)
            .unwrap(),
        (
            &[][..],
            Some(smallvec::smallvec![Address::new_group(
                "undisclosed-recipients".to_string(),
                vec![]
            )])
        )
    );

    assert_eq!(
        envelope_addresses(br#"((NIL NIL "undisclosed-recipients" NIL))"#).unwrap(),
        (&[][..], Some(smallvec::smallvec![]))
    );

    assert_eq!(
        envelope_addresses(br#"((NIL NIL "undisclosed-recipients" NIL)(NIL NIL "undisclosed-recipients2" NIL)(NIL NIL NIL NIL)(NIL NIL NIL NIL))"#).unwrap(),
        (
            &[][..],
            Some(smallvec::smallvec![Address::new_group(
                "undisclosed-recipients2".to_string(),
                vec![]
            )])
        )
    );

    assert_eq!(
        envelope_addresses(
            br#"(("Pete" NIL "silly" "example.com")(NIL NIL NIL NIL)(NIL NIL NIL NIL))"#,
        )
        .unwrap(),
        (
            &[][..],
            Some(smallvec::smallvec![Address::new(
                Some("Pete".to_string()),
                "silly@example.com".to_string()
            )])
        )
    );

    // Adapted from Appendix A.1.3.  Group Addresses in RFC5322:
    assert_eq!(
        envelope_addresses(br#"((NIL NIL "A Group" NIL)("Ed Jones" NIL "c" "example.com")(NIL NIL "joe" "example.com")("John" NIL "jdoe" "example.com")(NIL NIL NIL NIL))"#).unwrap(),
        (
            &[][..],
            Some(smallvec::smallvec![Address::new_group(
                "A Group".to_string(),
                vec![
                    Address::new(Some("Ed Jones".to_string()), "c@example.com".to_string()),
                    Address::new(None::<&str>, "joe@example.com".to_string()),
                    Address::new(Some("John".to_string()), "jdoe@example.com".to_string()),
                ]
            )])
        )
    );

    assert_eq!(envelope_addresses(br#"NIL"#).unwrap(), (&[][..], None));
    assert_eq!(envelope_addresses(br#""""#).unwrap(), (&[][..], None));

    assert_eq!(
            envelope_addresses(b"((NIL NIL \"Mohamed\" NIL)(\"Mohamed\" NIL \"moh\" \"example.com\")(\"markus\" NIL \"mark\" \"example.com\"))").unwrap(),
            (
                &[][..],
                Some(smallvec::smallvec![
                    Address::new(Some("Mohamed".to_string()), "moh@example.com".to_string()),
                    Address::new(Some("markus".to_string()), "mark@example.com".to_string()),
                ])
            )
        );
    assert_eq!(
        envelope_addresses(b"((NIL NIL \"Mohamed\" NIL))").unwrap(),
        (&[][..], Some(smallvec::smallvec![]))
    );
}

/// Module providing a global log-capture facility for asserting on
/// `log::error!` output in tests.
///
/// The buffer is append-only: tests snapshot a mark before acting and read
/// only the entries appended since, so parallel tests can never consume
/// each other's entries. Assertions must still filter by a unique marker
/// (e.g. the envelope subject) because other tests may append in between.
mod sanitize_log {
    use std::sync::{Mutex, Once};

    static CAPTURED: Mutex<Vec<String>> = Mutex::new(Vec::new());
    static INIT: Once = Once::new();

    struct CaptureLogger;

    impl log::Log for CaptureLogger {
        fn enabled(&self, metadata: &log::Metadata) -> bool {
            metadata.level() <= log::Level::Error
        }

        fn log(&self, record: &log::Record) {
            if self.enabled(record.metadata()) {
                if let Ok(mut captured) = CAPTURED.lock() {
                    captured.push(format!("{}", record.args()));
                }
            }
        }

        fn flush(&self) {}
    }

    pub(super) fn init() {
        INIT.call_once(|| {
            let _ = log::set_boxed_logger(Box::new(CaptureLogger));
            log::set_max_level(log::LevelFilter::Error);
        });
    }

    pub(super) fn mark() -> usize {
        CAPTURED.lock().map(|captured| captured.len()).unwrap_or(0)
    }

    pub(super) fn since(mark: usize) -> Vec<String> {
        CAPTURED
            .lock()
            .map(|captured| captured[mark.min(captured.len())..].to_vec())
            .unwrap_or_default()
    }
}

/// Assert the cache-safety invariant for every address of an [`Envelope`]
/// produced by the ENVELOPE parser.
///
/// The sqlite3 cache serializes `Address` values via their `Display` form
/// and strictly re-parses them on load (the asymmetry behind the T8 report
/// §7.1 defect), so the display string of every address must re-parse, and
/// the whole envelope must survive the serde round-trip.
///
/// Note on precision: the strict parser un-quotes quoted local parts when
/// reconstructing the addr-spec (e.g. `"recipients:"@qq.com` re-parses to
/// spec `recipients:@qq.com`), so the re-parsed value is not always
/// byte-equal to the original; successful re-parse plus the content
/// assertions in the dedicated tests below is the exact cache-failure
/// predicate.
fn assert_addresses_cache_roundtrip_safe(env: &Envelope) {
    let mut all: Vec<&Address> = Vec::new();
    all.extend(env.from.iter());
    all.extend(env.to.iter());
    all.extend(env.cc.iter());
    all.extend(env.bcc.iter());
    for addr in all {
        let display = addr.to_string();
        Address::try_from(display.as_str()).unwrap_or_else(|err| {
            panic!("address display form `{display}` does not re-parse: {err}")
        });
    }
    let blob = serde_json::to_vec(env).unwrap();
    serde_json::from_slice::<Envelope>(&blob).unwrap_or_else(|err| {
        panic!("envelope does not survive the cache serde round-trip: {err}")
    });
}

#[test]
fn test_imap_envelope_addresses_cache_roundtrip_invariant() {
    // Battery of real-world-shaped ENVELOPE inputs: valid addresses must
    // stay untouched, poisoned ones must be normalized or substituted, and
    // the invariant must hold for ALL of them.
    let inputs: &[&[u8]] = &[
        // plain valid
        b"(\"Fri, 24 Jun 2011 10:09:10 +0000\" \"s\" ((\"xx\" NIL \"xx\" \"xx.com\")) NIL NIL ((\"xx\" NIL \"xx\" \"xx.com\")) NIL NIL NIL \"<xx@xx.com>\")",
        // dotted local parts, quoted display name with specials, unicode
        b"(\"date\" \"s\" ((\"J\xC3\xB6rg T. Doe\" NIL \"xx.yy\" \"example.com\") (\"'weird, name'\" NIL \"a.b.c\" \"x.com\")) NIL NIL NIL NIL NIL NIL NIL)",
        // T8 real-world poison: display `"recipients:" <recipients:@qq.com>`
        b"(\"date\" \"s\" ((\"recipients:\" NIL \"recipients:\" \"qq.com\")) NIL NIL ((\"user\" NIL \"user\" \"example.com\")) NIL NIL NIL \"<m@example.com>\")",
        // empty local part
        b"(\"date\" \"s\" ((\"recipients:\" NIL \"\" \"qq.com\")) NIL NIL NIL NIL NIL NIL NIL)",
        // empty host
        b"(\"date\" \"s\" ((\"recipients:\" NIL \"recipients\" \"\")) NIL NIL NIL NIL NIL NIL NIL)",
        // host with an embedded space: not fixable by quoting the local part
        b"(\"date\" \"s\" ((\"recipients:\" NIL \"recipients\" \"qq .com\")) NIL NIL NIL NIL NIL NIL NIL)",
        // group whose name contains a special character
        b"(\"date\" \"s\" ((NIL NIL \"A Group:x\" NIL)(\"Ed Jones\" NIL \"c\" \"example.com\")(NIL NIL NIL NIL)) NIL NIL NIL NIL NIL NIL NIL)",
        // local part with an embedded control byte
        b"(\"date\" \"s\" ((\"n\" NIL \"a\x00b\" \"x.com\")) NIL NIL NIL NIL NIL NIL NIL)",
        // T1 split-token message-id envelope
        b"(\"Wed, 20 May 2026 12:34:30 +0800\" \"subject\" ((\"name\" NIL \"user\" \"example.com\")) NIL NIL ((\"name\" NIL \"user\" \"example.com\")) NIL NIL \"<reply.\"l@p\"@d>\" \"<605067.8696.JavaMail.\"billing@example.com\"@example.center.na620>\")",
    ];
    for input in inputs {
        let (rest, env) = envelope(input).unwrap();
        assert!(rest.is_empty(), "unparsed trailing bytes: {rest:?}");
        assert_addresses_cache_roundtrip_safe(&env);
    }

    // Truncated/garbage input must fail parsing (or parse into something
    // safe) without panicking.
    envelope(b"(\"date\" \"s\" ((\"a\" NIL \"b\"").unwrap_err();
    envelope(b"").unwrap_err();
    let _ = envelope(b"(\x00\x01\x02\xff");
}

#[test]
fn test_imap_envelope_poison_address_normalized_to_quoted_local_part() {
    // The T8 real-world poison display string `"recipients:" <recipients:@qq.com>`
    // must be normalized to the quoted-local-part form, content preserved.
    let input: &[u8] = b"(\"Wed, 20 May 2026 12:34:30 +0800\" \"subject\" ((\"recipients:\" NIL \"recipients:\" \"qq.com\")) NIL NIL ((\"user\" NIL \"user\" \"example.com\")) NIL NIL NIL \"<m@example.com>\")";
    let (rest, env) = envelope(input).unwrap();
    assert!(rest.is_empty());
    assert_eq!(env.from.len(), 1);
    let addr = env.from.first().unwrap();
    assert_eq!(addr.get_display_name(), Some("recipients:"));
    assert_eq!(addr.get_email(), "\"recipients:\"@qq.com");
    // valid addresses in the same envelope stay untouched
    assert_eq!(
        env.to.first().unwrap(),
        &Address::new(Some("user"), "user@example.com")
    );
    // normalization is deterministic
    let (_, env_second) = envelope(input).unwrap();
    assert_eq!(addr, env_second.from.first().unwrap());
    assert_addresses_cache_roundtrip_safe(&env);
}

#[test]
fn test_imap_envelope_unfixable_address_replaced_with_placeholder() {
    sanitize_log::init();
    let mark = sanitize_log::mark();
    // Empty local part cannot be represented in a cache-safe form.
    let input: &[u8] =
        b"(\"date\" \"t13-unfixable\" ((\"recipients:\" NIL \"\" \"qq.com\")) NIL NIL NIL NIL NIL NIL NIL)";
    let (rest, env) = envelope(input).unwrap();
    assert!(rest.is_empty());
    assert_eq!(env.from.len(), 1);
    let addr = env.from.first().unwrap();
    assert_eq!(addr.get_email(), "invalid@invalid.invalid");
    // original raw value stays visible for diagnosability
    assert_eq!(
        addr.get_display_name(),
        Some("[invalid address: \"recipients:\" <@qq.com>]")
    );
    assert_addresses_cache_roundtrip_safe(&env);
    let logs = sanitize_log::since(mark);
    assert!(
        logs.iter().any(|l| l.contains("From field")
            && l.contains("t13-unfixable")
            && l.contains("<@qq.com>")),
        "expected an error log naming the From field, the subject and the original value, got {logs:?}"
    );
}

#[test]
fn test_imap_envelope_all_address_fields_sanitized() {
    sanitize_log::init();
    // An unfixable poisoned address (host with an embedded space) is placed
    // in each of the six ENVELOPE address field positions in turn; every
    // field must route through validation.
    let valid: &str = "((\"user\" NIL \"user\" \"example.com\"))";
    let poison: &str = "((\"recipients:\" NIL \"recipients\" \"qq .com\"))";
    for (field, position) in [
        ("From", 0usize),
        ("Sender", 1),
        ("Reply-To", 2),
        ("To", 3),
        ("Cc", 4),
        ("Bcc", 5),
    ] {
        let mark = sanitize_log::mark();
        let subject = format!("t13-route-{field}");
        let mut fields: Vec<&str> = vec![valid; 6];
        fields[position] = poison;
        let input = format!(
            "(\"date\" \"{subject}\" {} {} {} {} {} {} NIL NIL)",
            fields[0], fields[1], fields[2], fields[3], fields[4], fields[5]
        );
        let (rest, env) = envelope(input.as_bytes()).unwrap();
        assert!(
            rest.is_empty(),
            "unparsed trailing bytes for {field}: {rest:?}"
        );
        match field {
            "From" | "To" | "Cc" | "Bcc" => {
                let addr = match field {
                    "From" => env.from.first(),
                    "To" => env.to.first(),
                    "Cc" => env.cc.first(),
                    _ => env.bcc.first(),
                }
                .unwrap();
                assert_eq!(addr.get_email(), "invalid@invalid.invalid", "{field}");
            }
            // `Sender`/`Reply-To` are not stored in `Envelope`; routing is
            // observable only through the error log.
            _ => {}
        }
        assert_addresses_cache_roundtrip_safe(&env);
        let logs = sanitize_log::since(mark);
        assert!(
            logs.iter()
                .any(|l| l.contains(&format!("{field} field")) && l.contains(&subject)),
            "expected an error log naming the {field} field and subject `{subject}`, got {logs:?}"
        );
    }
}

#[test]
fn test_imap_envelope_message_id_survives_cache_serde_roundtrip() {
    // `MessageID` serializes as a plain string and deserializes via
    // `MessageID::new` (no strict re-parse), so message-ids — including the
    // split-token concatenations accepted by the T1 tolerance patch — can
    // never fail the cache round-trip. This pins that invariant.
    let input: &[u8] = b"(\"Wed, 20 May 2026 12:34:30 +0800\" \"subject\" ((\"name\" NIL \"user\" \"example.com\")) NIL NIL ((\"name\" NIL \"user\" \"example.com\")) NIL NIL \"<reply.\"l@p\"@d>\" \"<605067.8696.JavaMail.\"billing@example.com\"@example.center.na620>\")";
    let (rest, env) = envelope(input).unwrap();
    assert!(rest.is_empty());
    assert_eq!(
        env.message_id().to_string(),
        "605067.8696.JavaMail.billing@example.com@example.center.na620"
    );
    let blob = serde_json::to_vec(&env).unwrap();
    let env_deserialized: Envelope = serde_json::from_slice(&blob).unwrap();
    assert_eq!(
        env_deserialized.message_id().to_string(),
        "605067.8696.JavaMail.billing@example.com@example.center.na620"
    );
}

#[test]
fn test_imap_envelope_raw_bytes_fallback_atom_first_message_id() {
    // A message-id relayed as a single unquoted atom (no enclosing quotes)
    // is not a valid IMAP nstring, so strict parsing fails. The raw-bytes
    // fallback stage must accept it and store the atom verbatim; until then
    // the whole ENVELOPE fails to parse (TDD red light).
    let input: &[u8] = b"(\"Wed, 20 May 2026 12:34:30 +0800\" \"subject\" ((\"name\" NIL \"user\" \"example.com\")) NIL NIL ((\"name\" NIL \"user\" \"example.com\")) NIL NIL NIL 605067.JavaMail.billing@example.com@example.center)";
    let (rest, env) = envelope(input).unwrap();
    assert!(rest.is_empty());
    assert_eq!(
        env.message_id().to_string(),
        "605067.JavaMail.billing@example.com@example.center"
    );
}

#[test]
fn test_imap_envelope_raw_bytes_fallback_unterminated_quote_message_id() {
    // A message-id whose quote is never closed cannot be scanned as a
    // quoted string. The raw-bytes fallback must store the raw bytes up to
    // the field boundary, leading `"` included (`set_message_id` only trims
    // whitespace); until then the ENVELOPE fails to parse (TDD red light).
    let input: &[u8] = b"(\"Wed, 20 May 2026 12:34:30 +0800\" \"subject\" ((\"name\" NIL \"user\" \"example.com\")) NIL NIL ((\"name\" NIL \"user\" \"example.com\")) NIL NIL NIL \"<605067.JavaMail.)";
    let (rest, env) = envelope(input).unwrap();
    assert!(rest.is_empty());
    assert_eq!(env.message_id().to_string(), "\"<605067.JavaMail.");
}

#[test]
fn test_imap_envelope_raw_bytes_fallback_literal_guard() {
    // Characterization pin of the current `quoted_or_nil_concat` behavior:
    // a literal whose declared length exceeds the available input must
    // terminate with an error (the guard must not raw-scan a truncated
    // literal), while a valid literal is consumed as a single value with
    // the remaining input returned untouched.
    quoted_or_nil_concat(b"{13}\r\n605067.JavaM").unwrap_err();
    let (rest, value) = quoted_or_nil_concat(b"{12}\r\n605067.JavaM)").unwrap();
    assert_eq!(rest, &b")"[..]);
    assert_eq!(value.as_deref(), Some(&b"605067.JavaM"[..]));
}

#[test]
fn test_imap_envelope_fallback_does_not_conflate_adjacent_fields() {
    // Two adjacent malformed nstring fields must be captured independently:
    // in-reply-to is the unterminated quoted token `"<abc`, message-id is
    // the bare atom `<m@example.com>` (unquoted, so the closing-quote scan
    // of `quoted()` cannot pair in-reply-to's dangling quote with it and
    // conflate both fields into one value). The in-reply-to assertion goes
    // through `other_headers()` because a bad-shape value never parses into
    // `env.in_reply_to()`.
    let input: &[u8] = b"(\"Wed, 20 May 2026 12:34:30 +0800\" \"subject\" ((\"name\" NIL \"user\" \"example.com\")) NIL NIL ((\"name\" NIL \"user\" \"example.com\")) NIL NIL \"<abc <m@example.com>)";
    let (rest, env) = envelope(input).unwrap();
    assert!(rest.is_empty());
    assert_eq!(&env.other_headers()[HeaderName::IN_REPLY_TO], "\"<abc");
    assert_eq!(env.message_id().to_string(), "m@example.com");
}

#[test]
fn test_imap_response_code_missing_value_does_not_panic() {
    // A malformed status response whose `]` appears before the end of the
    // keyword used to build a reversed slice range (`&val[10..7]`) and
    // panic. It must degrade to the documented `0` fallback instead.
    assert_eq!(
        ImapResponse::try_from(&b"M1 OK [UIDNEXT]\r\n"[..]).unwrap(),
        ImapResponse::Ok(ResponseCode::Uidnext(0))
    );
    assert_eq!(
        ImapResponse::try_from(&b"M1 OK [UIDVALIDITY]\r\n"[..]).unwrap(),
        ImapResponse::Ok(ResponseCode::Uidvalidity(0))
    );
    assert_eq!(
        ImapResponse::try_from(&b"M1 OK [UNSEEN]\r\n"[..]).unwrap(),
        ImapResponse::Ok(ResponseCode::Unseen(0))
    );
}

#[test]
fn test_imap_response_bare_status_word_does_not_panic() {
    // `OK`/`NO`/`BAD`/`PREAUTH`/`BYE` without the trailing space used to
    // index past the end of the value (`&val[b"OK ".len()..]`).
    for line in [
        &b"M1 OK\r\n"[..],
        &b"M1 NO\r\n"[..],
        &b"M1 BAD\r\n"[..],
        &b"M1 PREAUTH\r\n"[..],
        &b"M1 BYE\r\n"[..],
    ] {
        ImapResponse::try_from(line).unwrap();
    }
}

#[test]
fn test_imap_fetch_response_oversized_uid_is_error() {
    // A UID that does not fit the `usize` type used to hit `.unwrap()` on
    // the parse result and panic. It must be a typed protocol error.
    let input = &b"* 1 FETCH (UID 99999999999999999999999999999999 FLAGS ())\r\n"[..];
    let err = fetch_response(input).unwrap_err();
    assert_eq!(err.kind, ErrorKind::ProtocolError);
}

#[test]
fn test_imap_fetch_response_bare_untagged_prefix_is_error() {
    // `fetch_response` starts indexing right after `* `; an input that is
    // exactly the prefix used to panic with an out-of-bounds index.
    fetch_response(b"* ").unwrap_err();
}

#[test]
fn test_imap_select_response_missing_counts_does_not_panic() {
    // `* EXISTS` / `* RECENT` without a count used to build a reversed
    // slice range and panic. The line must be skipped gracefully.
    let input = &b"* OK [UIDNEXT 1] Predicted next UID\r\n* EXISTS\r\n* RECENT\r\n"[..];
    let ret = select_response(input).unwrap();
    assert_eq!(ret.exists, 0);
    assert_eq!(ret.recent, 0);
}

#[test]
fn test_imap_select_response_missing_code_value_is_error() {
    // A select response code whose `]` precedes its value must be a typed
    // protocol error, not a reversed-slice panic.
    for input in [
        &b"* OK [UIDNEXT ] Predicted next UID\r\n"[..],
        &b"* OK [UIDVALIDITY ] UIDs valid\r\n"[..],
        &b"* OK [UNSEEN ] First unseen\r\n"[..],
        &b"* OK [UIDNEXT 1]\r\n* FLAGS ("[..],
    ] {
        let err = select_response(input).unwrap_err();
        assert_eq!(err.kind, ErrorKind::ProtocolError);
    }
}

#[test]
fn test_imap_bodystructure_depth_limit() {
    // Unbounded recursion on a run of `(` used to overflow the stack; the
    // parser must instead stop with an error past the named depth bound,
    // while a normal nested part list still parses.
    let deep = "(".repeat(500).into_bytes();
    bodystructure_has_attachments(&deep).unwrap_err();

    let valid = &b"((\"text\" \"plain\"))"[..];
    bodystructure_has_attachments(valid).unwrap();
}

#[test]
fn test_imap_search_results_non_utf8_is_error() {
    // `is_not` accepts arbitrary bytes; a non-UTF-8 search result used to go
    // through `from_utf8_unchecked` (undefined behaviour) and must now be a
    // parse error, while valid results keep parsing.
    search_results(b"* SEARCH \xff\xfe\r\n").unwrap_err();
    let (rest, v) = search_results(b"* SEARCH 1 2 3\r\n").unwrap();
    assert!(rest.is_empty());
    assert_eq!(v, vec![1, 2, 3]);
}
