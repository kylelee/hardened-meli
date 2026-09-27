/*
 * meli - imap module.
 *
 * Copyright 2017 - 2023 Manos Pitsidianakis
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

//! Convert [`crate::search::Query`] into IMAP search criteria.

use std::collections::VecDeque;

use crate::{
    search::*,
    utils::datetime::{formats::IMAP_DATE, timestamp_to_string_utc},
};

mod private {
    pub trait Sealed {}
}

/// A piece of a serialized IMAP `SEARCH` command.
///
/// ASCII-only queries serialize to a single [`ImapSearchSegment::Text`]. A
/// value containing non-ASCII octets cannot be sent inside a quoted string
/// (RFC 3501 limits quoted strings to 7-bit `CHAR`s) and is emitted as an
/// [`ImapSearchSegment::Literal`] instead.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImapSearchSegment {
    /// Plain command text, safe to write verbatim.
    Text(String),
    /// Raw octets to send as an IMAP literal. The preceding `Text` segment
    /// ends with the `{n}` octet count.
    Literal(Vec<u8>),
}

impl ImapSearchSegment {
    /// Whether this segment carries raw literal octets.
    pub fn is_literal(&self) -> bool {
        matches!(self, Self::Literal(_))
    }

    /// The text of an [`ImapSearchSegment::Text`] segment.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            Self::Literal(_) => None,
        }
    }
}

/// A single wire action while sending an IMAP command that contains
/// synchronizing literals.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImapSearchSendStep {
    /// Write these bytes verbatim. The command tag and the
    /// `UID SEARCH CHARSET UTF-8 ` prefix are prepended by the caller to the
    /// first `Write` step only.
    Write(Vec<u8>),
    /// Wait for the server's continuation request (`+ ...`) before writing
    /// the next literal octets.
    WaitContinuation,
}

pub trait ToImapSearch: private::Sealed {
    /// Convert [`crate::search::Query`] into IMAP search criteria, split into
    /// text and literal segments.
    fn to_imap_search_segments(&self) -> Vec<ImapSearchSegment>;
}

impl private::Sealed for Query {}

macro_rules! space_pad {
    ($s:ident) => {{
        if !$s.is_empty() && !$s.ends_with('(') && !$s.ends_with(' ') {
            $s.push(' ');
            false
        } else {
            true
        }
    }};
}

/// Marker used internally to remember where a literal's octets belong in the
/// serialized command. It is a Unicode private-use code point: ASCII values go
/// through [`escape_double_quote`] (so they only contain ASCII) and non-ASCII
/// values are emitted as literals, hence the marker can only originate here.
const LITERAL_MARKER_START: char = '\u{E000}';
const LITERAL_MARKER_END: char = '\u{E001}';

/// Append a string-valued search condition.
///
/// Pure ASCII values keep the historical inline quoted form. Anything else is
/// emitted as an IMAP synchronizing literal: a `{n}` octet count goes into the
/// command text and the raw UTF-8 octets are queued in `literals`, with a
/// private-use marker recording their position.
fn push_search_value(s: &mut String, value: &str, literals: &mut Vec<Vec<u8>>) {
    if value.is_ascii() {
        s.push('"');
        s.extend(escape_double_quote(value).chars());
        s.push('"');
    } else {
        s.push('{');
        s.push_str(&value.len().to_string());
        s.push('}');
        s.push(LITERAL_MARKER_START);
        s.push_str(&literals.len().to_string());
        s.push(LITERAL_MARKER_END);
        literals.push(value.as_bytes().to_vec());
    }
}

/// Turn serialized search segments into the ordered wire actions needed to
/// send them.
///
/// Only meaningful when at least one segment is a literal. Each text segment is
/// written followed by CRLF (the CRLF that ends the `{n}` line before a literal
/// or the command line at the end); literal octets are written verbatim after a
/// continuation request. A trailing CRLF is appended when the query ends on a
/// literal.
pub fn search_send_steps(segments: &[ImapSearchSegment]) -> Vec<ImapSearchSendStep> {
    let mut steps = Vec::with_capacity(segments.len() * 2);
    let mut ends_with_literal = false;
    for segment in segments {
        match segment {
            ImapSearchSegment::Text(text) => {
                let mut bytes = Vec::with_capacity(text.len() + 2);
                bytes.extend_from_slice(text.as_bytes());
                bytes.extend_from_slice(b"\r\n");
                steps.push(ImapSearchSendStep::Write(bytes));
                ends_with_literal = false;
            }
            ImapSearchSegment::Literal(octets) => {
                steps.push(ImapSearchSendStep::WaitContinuation);
                steps.push(ImapSearchSendStep::Write(octets.clone()));
                ends_with_literal = true;
            }
        }
    }
    if ends_with_literal {
        steps.push(ImapSearchSendStep::Write(b"\r\n".to_vec()));
    }
    steps
}

/// Rewrite the trailing `{n}` octet count of a text segment that precedes a
/// literal into the non-synchronizing ([RFC 7888] `LITERAL+`) `{n+}` form.
///
/// Text segments that do not end in a `{n}` count are returned unchanged.
///
/// [RFC 7888]: https://datatracker.ietf.org/doc/html/rfc7888
fn non_sync_literal_text(text: &str) -> String {
    let Some(open) = text.rfind('{') else {
        return text.to_string();
    };
    let Some(count) = text[open + 1..].strip_suffix('}') else {
        return text.to_string();
    };
    if count.is_empty() || !count.bytes().all(|byte| byte.is_ascii_digit()) {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + 1);
    out.push_str(&text[..open + 1]);
    out.push_str(count);
    out.push('+');
    out.push('}');
    out
}

/// Turn serialized search segments into the wire actions for a server that
/// accepts non-synchronizing literals ([RFC 7888] `LITERAL+` / `LITERAL-`).
///
/// Unlike [`search_send_steps`], the `{n}` counts of the text segments become
/// `{n+}`, so the whole command can be written in one pass without ever
/// waiting for a continuation request: no
/// [`ImapSearchSendStep::WaitContinuation`] step is produced. A trailing CRLF
/// is appended when the query ends on a literal, mirroring
/// [`search_send_steps`].
///
/// [RFC 7888]: https://datatracker.ietf.org/doc/html/rfc7888
pub fn search_send_steps_non_sync(segments: &[ImapSearchSegment]) -> Vec<ImapSearchSendStep> {
    let mut steps = Vec::with_capacity(segments.len() + 1);
    let mut ends_with_literal = false;
    for segment in segments {
        match segment {
            ImapSearchSegment::Text(text) => {
                let mut bytes = non_sync_literal_text(text).into_bytes();
                bytes.extend_from_slice(b"\r\n");
                steps.push(ImapSearchSendStep::Write(bytes));
                ends_with_literal = false;
            }
            ImapSearchSegment::Literal(octets) => {
                steps.push(ImapSearchSendStep::Write(octets.clone()));
                ends_with_literal = true;
            }
        }
    }
    if ends_with_literal {
        steps.push(ImapSearchSendStep::Write(b"\r\n".to_vec()));
    }
    steps
}

impl ToImapSearch for Query {
    fn to_imap_search_segments(&self) -> Vec<ImapSearchSegment> {
        enum Step<'a> {
            Q(&'a Query),
            Lit(char),
        }
        use Step::*;

        let mut stack = VecDeque::new();
        stack.push_front(Q(self));
        let mut s = String::new();
        let mut literals: Vec<Vec<u8>> = Vec::new();
        while let Some(q) = stack.pop_front() {
            use Query::*;
            match q {
                Lit(lit) => {
                    s.push(lit);
                }
                Q(Subject(t)) => {
                    space_pad!(s);
                    s.push_str("SUBJECT ");
                    push_search_value(&mut s, t, &mut literals);
                }
                Q(From(t)) => {
                    space_pad!(s);
                    s.push_str("FROM ");
                    push_search_value(&mut s, t, &mut literals);
                }
                Q(To(t)) => {
                    space_pad!(s);
                    s.push_str("TO ");
                    push_search_value(&mut s, t, &mut literals);
                }
                Q(Cc(t)) => {
                    space_pad!(s);
                    s.push_str("CC ");
                    push_search_value(&mut s, t, &mut literals);
                }
                Q(Bcc(t)) => {
                    space_pad!(s);
                    s.push_str("BCC ");
                    push_search_value(&mut s, t, &mut literals);
                }
                Q(AllText(t)) => {
                    space_pad!(s);
                    s.push_str("TEXT ");
                    push_search_value(&mut s, t, &mut literals);
                }
                Q(Flags(v)) => {
                    space_pad!(s);
                    for f in v {
                        match f.as_str() {
                            "draft" => {
                                s.push_str("DRAFT ");
                            }
                            "deleted" => {
                                s.push_str("DELETED ");
                            }
                            "flagged" => {
                                s.push_str("FLAGGED ");
                            }
                            "recent" => {
                                s.push_str("RECENT ");
                            }
                            "seen" | "read" => {
                                s.push_str("SEEN ");
                            }
                            "unseen" | "unread" => {
                                s.push_str("UNSEEN ");
                            }
                            "answered" => {
                                s.push_str("ANSWERED ");
                            }
                            "unanswered" => {
                                s.push_str("UNANSWERED ");
                            }
                            keyword => {
                                s.push_str("KEYWORD ");
                                s.push_str(keyword);
                                s.push(' ');
                            }
                        }
                    }
                }
                Q(And(q1, q2)) => {
                    let is_empty = space_pad!(s);
                    if !is_empty {
                        stack.push_front(Lit(')'));
                    }
                    stack.push_front(Q(q2));
                    stack.push_front(Q(q1));
                    if !is_empty {
                        stack.push_front(Lit('('));
                    }
                }
                Q(Or(q1, q2)) => {
                    space_pad!(s);
                    s.push_str("OR");
                    stack.push_front(Q(q2));
                    stack.push_front(Q(q1));
                }
                Q(Not(q)) => {
                    space_pad!(s);
                    s.push_str("NOT (");
                    stack.push_front(Lit(')'));
                    stack.push_front(Q(q));
                }
                Q(Before(t)) => {
                    space_pad!(s);
                    s.push_str("BEFORE ");
                    s.push_str(&timestamp_to_string_utc(*t, Some(IMAP_DATE), true));
                }
                Q(After(t)) => {
                    space_pad!(s);
                    s.push_str("SINCE ");
                    s.push_str(&timestamp_to_string_utc(*t, Some(IMAP_DATE), true));
                }
                Q(Between(t1, t2)) => {
                    space_pad!(s);
                    s.push_str("(SINCE ");
                    s.push_str(&timestamp_to_string_utc(*t1, Some(IMAP_DATE), true));
                    s.push_str(" BEFORE ");
                    s.push_str(&timestamp_to_string_utc(*t2, Some(IMAP_DATE), true));
                    s.push(')');
                }
                Q(On(t)) => {
                    space_pad!(s);
                    s.push_str("ON ");
                    s.push_str(&timestamp_to_string_utc(*t, Some(IMAP_DATE), true));
                }
                Q(InReplyTo(t)) => {
                    space_pad!(s);
                    s.push_str("HEADER \"In-Reply-To\" ");
                    push_search_value(&mut s, t, &mut literals);
                }
                Q(References(t)) => {
                    space_pad!(s);
                    s.push_str("HEADER \"References\" ");
                    push_search_value(&mut s, t, &mut literals);
                }
                Q(Header(t, v)) => {
                    space_pad!(s);
                    s.push_str("HEADER \"");
                    s.push_str(t.as_str());
                    s.push_str("\" ");
                    push_search_value(&mut s, v, &mut literals);
                }
                Q(AllAddresses(t)) => {
                    let is_empty = space_pad!(s);
                    if !is_empty {
                        s.push('(');
                    }
                    s.push_str("OR FROM ");
                    push_search_value(&mut s, t, &mut literals);
                    s.push_str(" (OR TO ");
                    push_search_value(&mut s, t, &mut literals);
                    s.push_str(" (OR CC ");
                    push_search_value(&mut s, t, &mut literals);
                    s.push_str(" BCC ");
                    push_search_value(&mut s, t, &mut literals);
                    s.push_str("))");
                    if !is_empty {
                        s.push(')');
                    }
                }
                Q(Body(t)) => {
                    // meli's local fallback scans subject, from and to for a
                    // bare term, so mirror that here instead of the narrower
                    // IMAP `BODY` (which excludes the subject).
                    space_pad!(s);
                    s.push_str("OR SUBJECT ");
                    push_search_value(&mut s, t, &mut literals);
                    s.push_str(" (OR FROM ");
                    push_search_value(&mut s, t, &mut literals);
                    s.push_str(" (OR TO ");
                    push_search_value(&mut s, t, &mut literals);
                    s.push_str(" (BODY ");
                    push_search_value(&mut s, t, &mut literals);
                    s.push_str(")))");
                }
                Q(HasAttachment) => {
                    log::warn!("HasAttachment in IMAP is unimplemented.");
                }
                Q(Answered) => {
                    space_pad!(s);
                    s.push_str(r#"ANSWERED ""#);
                }
                Q(AnsweredBy { by }) => {
                    space_pad!(s);
                    s.push_str("HEADER \"From\" ");
                    push_search_value(&mut s, by, &mut literals);
                }
                Q(Larger { than }) => {
                    space_pad!(s);
                    s.push_str("LARGER ");
                    s.push_str(&than.to_string());
                }
                Q(Smaller { than }) => {
                    space_pad!(s);
                    s.push_str("SMALLER ");
                    s.push_str(&than.to_string());
                }
            }
        }
        while s.ends_with(' ') {
            s.pop();
        }

        let mut segments = Vec::new();
        let mut rest = s.as_str();
        while let Some(start) = rest.find(LITERAL_MARKER_START) {
            let index_start = start + LITERAL_MARKER_START.len_utf8();
            let Some(index_len) = rest[index_start..].find(LITERAL_MARKER_END) else {
                break;
            };
            let index_end = index_start + index_len;
            segments.push(ImapSearchSegment::Text(rest[..start].to_string()));
            let index = rest[index_start..index_end]
                .parse::<usize>()
                .expect("literal marker index must be a valid usize");
            segments.push(ImapSearchSegment::Literal(literals[index].clone()));
            rest = &rest[index_end + LITERAL_MARKER_END.len_utf8()..];
        }
        if !rest.is_empty() {
            segments.push(ImapSearchSegment::Text(rest.to_string()));
        }
        segments
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{utils::parsec::Parser, HeaderName};

    /// Flatten segments back into one string for the ASCII-only assertions:
    /// literal octets are rendered after their `{n}` count.
    fn serialize(query: &Query) -> String {
        query
            .to_imap_search_segments()
            .iter()
            .map(|segment| match segment {
                ImapSearchSegment::Text(text) => text.clone(),
                ImapSearchSegment::Literal(octets) => {
                    format!("{{{}}}{}", octets.len(), String::from_utf8_lossy(octets))
                }
            })
            .collect()
    }

    #[test]
    fn test_imap_query_search() {
        let (_, q) = query().parse_complete("subject: test and i").unwrap();
        assert_eq!(
            serialize(&q),
            r#"SUBJECT "test" OR SUBJECT "i" (OR FROM "i" (OR TO "i" (BODY "i")))"#
        );

        let (_, q) = query().parse_complete("is:unseen").unwrap();
        assert_eq!(serialize(&q), r#"UNSEEN"#);

        let (_, q) = query().parse_complete("from:user@example.org").unwrap();
        assert_eq!(serialize(&q), r#"FROM "user@example.org""#);

        let (_, q) = query()
            .parse_complete(
                "from:user@example.org and subject:
            \"foobar space\"",
            )
            .unwrap();
        assert_eq!(
            serialize(&q),
            r#"FROM "user@example.org" SUBJECT "foobar space""#
        );

        assert_eq!(
            &timestamp_to_string_utc(1685739600, Some(IMAP_DATE), true),
            "02-Jun-2023"
        );

        let (_, q) = query()
            .parse_complete("before:2023-06-04 from:user@example.org")
            .unwrap();
        assert_eq!(serialize(&q), r#"BEFORE 04-Jun-2023 FROM "user@example.org""#);
        let (_, q) = query()
            .parse_complete(r#"subject:"wah ah ah" or (from:Manos and from:Sia)"#)
            .unwrap();
        assert_eq!(
            serialize(&q),
            r#"OR SUBJECT "wah ah ah" (FROM "Manos" FROM "Sia")"#
        );

        let (_, q) = query()
            .parse_complete(r#"subject:wo or (all-addresses:Manos)"#)
            .unwrap();
        assert_eq!(
            serialize(&q),
            r#"OR SUBJECT "wo" (OR FROM "Manos" (OR TO "Manos" (OR CC "Manos" BCC "Manos")))"#
        );
    }

    #[test]
    fn test_imap_query_ascii_body_expands_to_or_chain() {
        let query = Query::Body("hello".to_string());
        assert_eq!(
            query.to_imap_search_segments(),
            vec![ImapSearchSegment::Text(
                r#"OR SUBJECT "hello" (OR FROM "hello" (OR TO "hello" (BODY "hello")))"#
                    .to_string(),
            )]
        );
    }

    #[test]
    fn test_imap_query_non_ascii_body_uses_literals() {
        // A bare search term is parsed as `Query::Body`; the non-ASCII value
        // must become a literal in every OR branch instead of being placed
        // inside a quoted string.
        let (_, query) = query().parse_complete("账号").unwrap();
        assert_eq!(query, Query::Body("账号".to_string()));
        let octets = "账号".as_bytes().to_vec();
        assert_eq!(
            query.to_imap_search_segments(),
            vec![
                ImapSearchSegment::Text("OR SUBJECT {6}".to_string()),
                ImapSearchSegment::Literal(octets.clone()),
                ImapSearchSegment::Text(" (OR FROM {6}".to_string()),
                ImapSearchSegment::Literal(octets.clone()),
                ImapSearchSegment::Text(" (OR TO {6}".to_string()),
                ImapSearchSegment::Literal(octets.clone()),
                ImapSearchSegment::Text(" (BODY {6}".to_string()),
                ImapSearchSegment::Literal(octets),
                ImapSearchSegment::Text(")))".to_string()),
            ]
        );
    }

    #[test]
    fn test_imap_query_non_ascii_subject_uses_literal() {
        let query = Query::Subject("中".to_string());
        assert_eq!(
            query.to_imap_search_segments(),
            vec![
                ImapSearchSegment::Text("SUBJECT {3}".to_string()),
                ImapSearchSegment::Literal("中".as_bytes().to_vec()),
            ]
        );
    }

    #[test]
    fn test_imap_query_header_name_inline_value_literal() {
        let query = Query::Header(HeaderName::SUBJECT, "值".to_string());
        assert_eq!(
            query.to_imap_search_segments(),
            vec![
                ImapSearchSegment::Text(r#"HEADER "subject" {3}"#.to_string()),
                ImapSearchSegment::Literal("值".as_bytes().to_vec()),
            ]
        );
    }

    #[test]
    fn test_imap_query_ascii_embedded_quote_escaped() {
        let query = Query::Subject("a\"b".to_string());
        assert_eq!(
            query.to_imap_search_segments(),
            vec![ImapSearchSegment::Text(r#"SUBJECT "a""b""#.to_string())]
        );
    }

    #[test]
    fn test_imap_search_send_steps_multiple_literals() {
        let segments = vec![
            ImapSearchSegment::Text("SUBJECT {3}".to_string()),
            ImapSearchSegment::Literal("中".as_bytes().to_vec()),
            ImapSearchSegment::Text(" FROM {3}".to_string()),
            ImapSearchSegment::Literal("值".as_bytes().to_vec()),
        ];
        assert_eq!(
            search_send_steps(&segments),
            vec![
                ImapSearchSendStep::Write(b"SUBJECT {3}\r\n".to_vec()),
                ImapSearchSendStep::WaitContinuation,
                ImapSearchSendStep::Write("中".as_bytes().to_vec()),
                ImapSearchSendStep::Write(b" FROM {3}\r\n".to_vec()),
                ImapSearchSendStep::WaitContinuation,
                ImapSearchSendStep::Write("值".as_bytes().to_vec()),
                ImapSearchSendStep::Write(b"\r\n".to_vec()),
            ]
        );
    }

    #[test]
    fn test_imap_search_send_steps_body_cjk() {
        let segments = Query::Body("账号".to_string()).to_imap_search_segments();
        let steps = search_send_steps(&segments);
        assert_eq!(
            steps
                .iter()
                .filter(|step| **step == ImapSearchSendStep::WaitContinuation)
                .count(),
            4,
            "one continuation request per literal"
        );
        assert_eq!(
            steps.first(),
            Some(&ImapSearchSendStep::Write(b"OR SUBJECT {6}\r\n".to_vec()))
        );
        assert_eq!(
            steps.last(),
            Some(&ImapSearchSendStep::Write(b")))\r\n".to_vec()))
        );
    }

    #[test]
    fn test_imap_search_send_steps_non_sync_single_literal() {
        let segments = vec![
            ImapSearchSegment::Text("SUBJECT {3}".to_string()),
            ImapSearchSegment::Literal("中".as_bytes().to_vec()),
        ];
        let steps = search_send_steps_non_sync(&segments);
        assert_eq!(
            steps,
            vec![
                ImapSearchSendStep::Write(b"SUBJECT {3+}\r\n".to_vec()),
                ImapSearchSendStep::Write("中".as_bytes().to_vec()),
                ImapSearchSendStep::Write(b"\r\n".to_vec()),
            ]
        );
        assert!(
            !steps.contains(&ImapSearchSendStep::WaitContinuation),
            "non-sync literals must not wait for a continuation request"
        );
    }

    #[test]
    fn test_imap_search_send_steps_non_sync_body_cjk() {
        let segments = Query::Body("账号".to_string()).to_imap_search_segments();
        let steps = search_send_steps_non_sync(&segments);
        assert_eq!(
            steps
                .iter()
                .filter(|step| **step == ImapSearchSendStep::WaitContinuation)
                .count(),
            0,
            "non-sync literals must not wait for a continuation request"
        );
        let bytes = steps
            .iter()
            .filter_map(|step| match step {
                ImapSearchSendStep::Write(bytes) => Some(bytes.clone()),
                ImapSearchSendStep::WaitContinuation => None,
            })
            .flatten()
            .collect::<Vec<u8>>();
        let text = String::from_utf8_lossy(&bytes);
        assert_eq!(
            text.matches("{6+}").count(),
            4,
            "every `{{n}}` count must become `{{n+}}`"
        );
        assert_eq!(
            steps.first(),
            Some(&ImapSearchSendStep::Write(b"OR SUBJECT {6+}\r\n".to_vec()))
        );
        assert_eq!(
            steps.last(),
            Some(&ImapSearchSendStep::Write(b")))\r\n".to_vec()))
        );
    }

    #[test]
    fn test_imap_search_send_steps_non_sync_keeps_plain_text() {
        // A text segment that does not carry a `{n}` count must be left as-is.
        let segments = vec![
            ImapSearchSegment::Text("FROM {3}".to_string()),
            ImapSearchSegment::Literal("值".as_bytes().to_vec()),
            ImapSearchSegment::Text(" FLAGGED".to_string()),
        ];
        let steps = search_send_steps_non_sync(&segments);
        assert_eq!(
            steps,
            vec![
                ImapSearchSendStep::Write(b"FROM {3+}\r\n".to_vec()),
                ImapSearchSendStep::Write("值".as_bytes().to_vec()),
                ImapSearchSendStep::Write(b" FLAGGED\r\n".to_vec()),
            ]
        );
    }
}
