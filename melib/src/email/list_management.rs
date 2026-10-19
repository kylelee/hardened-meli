/*
 * meli
 *
 * Copyright 2017-2019 Manos Pitsidianakis
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

//! Parsing of `RFC2369` and `RFC2919` `List-*` headers.

use super::{parser, Envelope};
use crate::HeaderName;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ListAction {
    Url(Vec<u8>),
    Email(Vec<u8>),
    ///`List-Post` field may contain the special value "NO".
    No,
}

impl From<&[u8]> for ListAction {
    fn from(value: &[u8]) -> Self {
        if value.starts_with(b"mailto:") {
            /* if branch looks if value looks like a mailto url but doesn't validate it.
             * parser::mailto() will handle this if user tries to unsubscribe.
             */
            Self::Email(value.to_vec())
        } else if value.starts_with(b"NO") {
            Self::No
        } else {
            /* Otherwise treat it as url. There's no foolproof way to check if this is
             * valid, so postpone it until we try an HTTP request.
             */
            Self::Url(value.to_vec())
        }
    }
}

impl ListAction {
    pub fn parse_options_list(input: &[u8]) -> Option<Vec<Self>> {
        parser::mailing_lists::rfc_2369_list_headers_action_list(input)
            .map(|(_, mut vec)| {
                /* Prefer email options first, since this _is_ a mail client after all and
                 * it's more automated */
                vec.sort_unstable_by(|a, b| {
                    match (a.starts_with(b"mailto:"), b.starts_with(b"mailto:")) {
                        (true, false) => std::cmp::Ordering::Less,
                        (false, true) => std::cmp::Ordering::Greater,
                        _ => std::cmp::Ordering::Equal,
                    }
                });

                vec.into_iter().map(Self::from).collect::<Vec<Self>>()
            })
            .ok()
            .filter(|l| !l.is_empty())
    }

    /// Returns `true` if `self` is [`Self::Url`].
    pub const fn is_url(&self) -> bool {
        matches!(self, Self::Url(_))
    }

    /// Returns `true` if `self` is [`Self::Email`].
    pub const fn is_email(&self) -> bool {
        matches!(self, Self::Email(_))
    }
}

#[derive(Debug, Default)]
pub struct ListActions {
    pub id: Option<String>,
    pub archive: Option<Vec<ListAction>>,
    pub post: Option<Vec<ListAction>>,
    pub unsubscribe: Option<Vec<ListAction>>,
    pub subscribe: Option<Vec<ListAction>>,
    pub owner: Option<Vec<ListAction>>,
    pub help: Option<Vec<ListAction>>,
}

pub fn list_id_header(envelope: &Envelope) -> Option<String> {
    envelope
        .other_headers()
        .get(HeaderName::LIST_ID)
        .map(|s| s.to_owned())
}

pub fn list_id(header: Option<&'_ str>) -> Option<&'_ str> {
    /* rfc2919 https://tools.ietf.org/html/rfc2919 */
    /* list-id-header = "List-ID:" [phrase] "<" list-id ">" CRLF */
    header.and_then(|v| {
        if let Some(l) = v.rfind('<') {
            if let Some(r) = v.rfind('>') {
                if l < r {
                    return Some(&v[l + 1..r]);
                }
            }
        }
        None
    })
}

impl ListActions {
    pub fn detect(envelope: &Envelope) -> Option<Self> {
        let mut ret = Self {
            id: list_id_header(envelope),
            ..Self::default()
        };

        if let Some(archive) = envelope.other_headers().get(HeaderName::LIST_ARCHIVE) {
            ret.archive = ListAction::parse_options_list(archive.as_bytes());
        }

        if let Some(post) = envelope.other_headers().get(HeaderName::LIST_POST) {
            ret.post = ListAction::parse_options_list(post.as_bytes());
        }

        if let Some(unsubscribe) = envelope.other_headers().get(HeaderName::LIST_UNSUBSCRIBE) {
            ret.unsubscribe = ListAction::parse_options_list(unsubscribe.as_bytes());
        }

        if let Some(subscribe) = envelope.other_headers().get(HeaderName::LIST_SUBSCRIBE) {
            ret.subscribe = ListAction::parse_options_list(subscribe.as_bytes());
        }

        if let Some(help) = envelope.other_headers().get(HeaderName::LIST_HELP) {
            ret.help = ListAction::parse_options_list(help.as_bytes());
        }

        if let Some(owner) = envelope.other_headers().get(HeaderName::LIST_OWNER) {
            ret.owner = ListAction::parse_options_list(owner.as_bytes());
        }

        if ret.id.is_none()
            && ret.archive.is_none()
            && ret.post.is_none()
            && ret.unsubscribe.is_none()
            && ret.subscribe.is_none()
            && ret.help.is_none()
            && ret.owner.is_none()
        {
            None
        } else {
            Some(ret)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope_with_headers(headers: &str) -> Envelope {
        let raw = format!("From: a@example.org\r\n{headers}\r\n\r\nbody\r\n");
        Envelope::from_bytes(raw.as_bytes(), None).expect("test mail must parse")
    }

    /// RFC 2369 multi-action header: every `<...>` option is enumerated as an
    /// owned [`ListAction`], `mailto:` options first.
    #[test]
    fn test_list_management_detect_multiple_owned_actions() {
        let envelope = envelope_with_headers(
            "List-Unsubscribe: <https://example.com/unsub>, \
             <mailto:unsub@example.com?subject=unsubscribe>",
        );
        let actions = ListActions::detect(&envelope).expect("List-Unsubscribe must be detected");
        let unsubscribe = actions.unsubscribe.as_deref().unwrap_or_default();
        assert_eq!(
            unsubscribe,
            &[
                ListAction::Email(b"mailto:unsub@example.com?subject=unsubscribe".to_vec()),
                ListAction::Url(b"https://example.com/unsub".to_vec()),
            ],
            "mailto: options must be sorted first and every option enumerated"
        );
        assert!(unsubscribe[0].is_email());
        assert!(unsubscribe[1].is_url());
        // The actions are owned: they stay usable after the envelope they were
        // detected from is long gone (no lifetime clipping).
        drop(envelope);
        assert_eq!(
            actions.unsubscribe.as_deref().unwrap_or_default(),
            &[
                ListAction::Email(b"mailto:unsub@example.com?subject=unsubscribe".to_vec()),
                ListAction::Url(b"https://example.com/unsub".to_vec()),
            ],
        );
    }

    /// All six RFC 2369 `List-*` action headers plus RFC 2919 `List-ID` are
    /// detected; `List-Post: NO` keeps its special marker.
    #[test]
    fn test_list_management_detect_all_headers() {
        let envelope = envelope_with_headers(concat!(
            "List-ID: Foo Bar <foobar.example>\n",
            "List-Archive: <https://lists.example/archive/>\n",
            "List-Post: NO\n",
            "List-Unsubscribe: <mailto:unsub@foobar.example>\n",
            "List-Subscribe: <https://lists.example/subscribe>, <mailto:sub@foobar.example>\n",
            "List-Help: <mailto:help@foobar.example>\n",
            "List-Owner: <mailto:owner@foobar.example>",
        ));
        let actions = ListActions::detect(&envelope).expect("List-* headers must be detected");
        assert_eq!(actions.id.as_deref(), Some("Foo Bar <foobar.example>"));
        assert_eq!(
            actions.archive.as_deref().unwrap_or(&[]),
            &[ListAction::Url(b"https://lists.example/archive/".to_vec())]
        );
        assert_eq!(
            actions.post.as_deref().unwrap_or(&[]),
            &[ListAction::No],
            "`List-Post: NO` is kept as its special marker value"
        );
        assert_eq!(
            actions.unsubscribe.as_deref().unwrap_or(&[]),
            &[ListAction::Email(b"mailto:unsub@foobar.example".to_vec())]
        );
        assert_eq!(
            actions.subscribe.as_deref().unwrap_or(&[]),
            &[
                ListAction::Email(b"mailto:sub@foobar.example".to_vec()),
                ListAction::Url(b"https://lists.example/subscribe".to_vec()),
            ],
        );
        assert_eq!(
            actions.help.as_deref().unwrap_or(&[]),
            &[ListAction::Email(b"mailto:help@foobar.example".to_vec())]
        );
        assert_eq!(
            actions.owner.as_deref().unwrap_or(&[]),
            &[ListAction::Email(b"mailto:owner@foobar.example".to_vec())]
        );
    }

    /// No `List-*` headers at all yields `None`; a `List-*` header whose value
    /// is not an RFC 2369 option list (bare, unbracketed URL) yields no
    /// options for that field instead of a garbage action.
    #[test]
    fn test_list_management_detect_absent_and_malformed() {
        let plain = envelope_with_headers("Subject: no list headers");
        assert!(
            ListActions::detect(&plain).is_none(),
            "no List-* headers must not be detected"
        );

        let malformed = envelope_with_headers(concat!(
            "List-Archive: https://lists.example/archive\n",
            "List-Help: <mailto:help@foobar.example>"
        ));
        let actions =
            ListActions::detect(&malformed).expect("List-Help still counts as a list header");
        assert_eq!(
            actions.archive, None,
            "a bare unbracketed URL is not an RFC 2369 option list"
        );
        assert_eq!(
            actions.help.as_deref().unwrap_or(&[]),
            &[ListAction::Email(b"mailto:help@foobar.example".to_vec())]
        );
    }

    /// `parse_options_list` on non-option junk is `None`, and on an empty
    /// parse result it is `None` as well (no `Some(vec![])`).
    #[test]
    fn test_list_management_parse_options_list_strictness() {
        assert_eq!(ListAction::parse_options_list(b"no brackets here"), None);
        assert_eq!(ListAction::parse_options_list(b""), None);
        assert_eq!(
            ListAction::parse_options_list(b"<https://example.com/one>  , <mailto:x@y.example>"),
            Some(vec![
                ListAction::Email(b"mailto:x@y.example".to_vec()),
                ListAction::Url(b"https://example.com/one".to_vec()),
            ]),
        );
    }

    /// RFC 2919 `list_id()` extraction keeps working on the raw header value.
    #[test]
    fn test_list_management_list_id_extraction() {
        assert_eq!(
            list_id(Some("Foo Bar <foobar.example>")),
            Some("foobar.example")
        );
        assert_eq!(list_id(Some("<foobar.example>")), Some("foobar.example"));
        assert_eq!(list_id(Some("no brackets")), None);
        assert_eq!(list_id(None), None);
    }
}
