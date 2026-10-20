/*
 * meli - contacts module
 *
 * Copyright 2019 Manos Pitsidianakis
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
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with meli. If not, see <http://www.gnu.org/licenses/>.
 */

//! # vCard format
//!
//! This module implements the standards:
//!
//! - Version 3 (read-only) [RFC 2426: vCard MIME Directory Profile](https://datatracker.ietf.org/doc/2426)
//! - Version 4 [RFC 6350: vCard Format Specification](https://datatracker.ietf.org/doc/rfc6350/)
//! - Parameter escaping [RFC 6868 Parameter Value Encoding in iCalendar and vCard](https://datatracker.ietf.org/doc/rfc6868/)

use std::{convert::TryInto, path::PathBuf};

use indexmap::IndexMap;

use super::*;
use crate::{
    backends::prelude::ResultFuture,
    contacts::backend::{ContactBackend, ContactBackendCapabilities},
    error::{Error, ErrorKind, Result},
    utils::{
        parsec::{
            match_literal_anycase, one_or_more, peek, prefix, take_until, whitespace_wrap, Parser,
        },
        shellexpand::ShellExpandTrait as _,
    },
};

/* Supported vcard versions */
pub trait VCardVersion: Clone + std::fmt::Debug {
    const NAME: &str;
}

#[derive(Clone, Copy, Debug)]
pub struct VCardVersionUnknown;
impl VCardVersion for VCardVersionUnknown {
    const NAME: &str = "Unknown";
}

/// Version 4 <https://tools.ietf.org/html/rfc6350>
#[derive(Clone, Copy, Debug)]
pub struct VCardVersion4;
impl VCardVersion for VCardVersion4 {
    const NAME: &str = "4";
}

/// <https://tools.ietf.org/html/rfc2426>
#[derive(Clone, Copy, Debug)]
pub struct VCardVersion3;
impl VCardVersion for VCardVersion3 {
    const NAME: &str = "3";
}

pub struct CardDeserializer;

const HEADER_CRLF: &str = "BEGIN:VCARD\r\n"; //VERSION:4.0\r\n";
const FOOTER_CRLF: &str = "END:VCARD\r\n";
const HEADER_LF: &str = "BEGIN:VCARD\n"; //VERSION:4.0\n";
const FOOTER_LF: &str = "END:VCARD\n";
const HEADER: &str = "BEGIN:VCARD"; //VERSION:4.0";
const FOOTER: &str = "END:VCARD";

#[derive(Clone, Debug)]
pub struct VCard<T: VCardVersion>(
    IndexMap<String, ContentLine>,
    std::marker::PhantomData<fn() -> T>,
);

impl<V: VCardVersion> VCard<V> {
    pub fn new_v4() -> VCard<impl VCardVersion> {
        VCard(
            IndexMap::default(),
            std::marker::PhantomData::<fn() -> VCardVersion4>,
        )
    }

    pub const fn version(&self) -> &str {
        <V as VCardVersion>::NAME
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ContentLine {
    group: Option<String>,
    params: Vec<String>,
    value: String,
    others: Vec<Self>,
}

impl CardDeserializer {
    pub fn try_from_str(mut input: &str) -> Result<VCard<impl VCardVersion>> {
        input = if (!input.starts_with(HEADER_CRLF)
            || (!input.ends_with(FOOTER_CRLF) && !input.ends_with(FOOTER)))
            && (!input.starts_with(HEADER_LF)
                || (!input.ends_with(FOOTER_LF) && !input.ends_with(FOOTER)))
        {
            return Err(Error::new(format!(
                "Error while parsing vcard: input does not start or end with correct header and \
                 footer: {input:?}"
            ))
            .set_kind(ErrorKind::ValueError)
            .set_details(
                "vcard file entries are expected to start with a `BEGIN:VCARD` line and end with \
                 a `END:VCARD` line.",
            ));
        } else if input.starts_with(HEADER_CRLF) {
            // Slice by the footer variant that actually matched: the gate
            // accepts bare `FOOTER` tails, and subtracting `FOOTER_CRLF`'s
            // length there overshoots into the content (char-boundary panic /
            // truncated value).
            let footer = if input.ends_with(FOOTER_CRLF) {
                FOOTER_CRLF
            } else {
                FOOTER
            };
            &input[HEADER_CRLF.len()..input.len() - footer.len()]
        } else {
            // Same mismatch for the LF twin: subtract the *matched* footer
            // variant's length.
            let footer = if input.ends_with(FOOTER_LF) {
                FOOTER_LF
            } else {
                FOOTER
            };
            &input[HEADER_LF.len()..input.len() - footer.len()]
        };

        let mut ret: IndexMap<String, ContentLine> = IndexMap::default();

        enum Stage {
            Group,
            Name,
            Param,
            Value,
        }
        let mut stage: Stage;

        for l in input.lines() {
            let mut el = ContentLine::default();
            let mut value_start = 0;
            let mut has_colon = false;
            stage = Stage::Group;
            let mut name = String::new();
            for i in 0..l.len() {
                let byte = l.as_bytes()[i];
                match (byte, &stage) {
                    (b'.', Stage::Group) if l.as_bytes()[i] != b'\\' => {
                        el.group = Some(l[value_start..i].to_string());
                        value_start = i + 1;
                        stage = Stage::Name;
                    }
                    (b';', Stage::Group) => {
                        name = l[value_start..i].to_string();
                        value_start = i + 1;
                        stage = Stage::Param;
                    }
                    (b';', Stage::Param) => {
                        el.params.push(l[value_start..i].to_string());
                        value_start = i + 1;
                    }
                    (b';', Stage::Name) => {
                        name = l[value_start..i].to_string();
                        value_start = i + 1;
                        stage = Stage::Param;
                    }
                    (b':', Stage::Group) | (b':', Stage::Name) => {
                        name = l[value_start..i].to_string();
                        has_colon = true;
                        value_start = i + 1;
                        stage = Stage::Value;
                    }
                    (b':', Stage::Param) if l.as_bytes()[i.saturating_sub(1)] != b'\\' => {
                        el.params.push(l[value_start..i].to_string());
                        has_colon = true;
                        value_start = i + 1;
                        stage = Stage::Value;
                    }
                    _ => {}
                }
            }
            if !has_colon {
                return Err(Error::new(format!(
                    "Error while parsing vcard: error at line {l}, no colon. {el:?}"
                ))
                .set_kind(ErrorKind::ValueError));
            }
            if name.is_empty() {
                return Err(Error::new(format!(
                    "Error while parsing vcard: error at line {l}, no name for content line. \
                     {el:?}"
                ))
                .set_kind(ErrorKind::ValueError));
            }
            el.value = l[value_start..].replace("\\:", ":");
            if ret.contains_key(&name) {
                ret.entry(name).or_default().others.push(el);
            } else {
                ret.insert(name, el);
            }
        }
        Ok(VCard(
            ret,
            std::marker::PhantomData::<fn() -> VCardVersion4>,
        ))
    }
}

/// Return `true` when `value` starts with an RFC 3986 URI scheme, i.e.
/// `ALPHA *( ALPHA / DIGIT / "+" / "-" / "." ) ":"`.
///
/// This is a dependency-free equivalent of the upstream `url::Url::parse`
/// check used to decide whether a `SOCIALPROFILE` value is a URI or a bare
/// username; the `url` crate is an optional dependency here and must stay off.
fn has_uri_scheme(value: &str) -> bool {
    let Some((scheme, _)) = value.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

impl<V: VCardVersion> TryInto<Card> for VCard<V> {
    type Error = crate::error::Error;

    /// Lossily convert according to `RFC9555`.
    ///
    /// `Loffredo, M. and R. Stepanek, "JSContact: Converting from and to vCard",
    /// RFC 9555, DOI 10.17487/RFC9555, May 2024,
    /// <https://www.rfc-editor.org/info/rfc9555/>.`
    fn try_into(mut self) -> Result<Card> {
        let mut card = Card::new();
        card.set_id(CardId::Hash({
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            if let Some(val) = self.0.get("FN") {
                hasher.write(val.value.as_bytes());
            }
            if let Some(val) = self.0.get("N") {
                hasher.write(val.value.as_bytes());
            }
            if let Some(val) = self.0.get("EMAIL") {
                hasher.write(val.value.as_bytes());
            }
            hasher.finish()
        }));
        if let Some(val) = self.0.swap_remove("FN") {
            card.set_name(val.value);
        } else {
            return Err(Error::new("FN entry missing in VCard.").set_kind(ErrorKind::ValueError));
        }
        if let Some(mut val) = self.0.swap_remove("NICKNAME") {
            use crate::contacts::jscontact::json_types::Nickname;

            let add_nick =
                |card: &'_ mut Card, val: ContentLine, ctr: &mut u32| -> (String, Nickname) {
                    let ContentLine {
                        group,
                        value,
                        others,
                        params,
                    } = val;
                    debug_assert!(others.is_empty());

                    let mut group = group.unwrap_or_else(|| format!("NICK-{ctr}"));
                    let pref = params
                        .iter()
                        .find_map(|p| p.strip_prefix("PREF=")?.parse::<u64>().ok());
                    let contexts = params
                        .iter()
                        .find_map(|p| Some((p.strip_prefix("TYPE=")?.to_string(), true)))
                        .into_iter()
                        .collect();
                    while card.nicknames.contains_key(&group) {
                        *ctr += 1;
                        group = format!("NICK-{ctr}");
                    }
                    *ctr += 1;
                    (
                        group,
                        Nickname {
                            name: value,
                            contexts,
                            pref,
                            ..Nickname::default()
                        },
                    )
                };

            let first_value = val.value.clone();
            let mut ctr = 1;
            let others = std::mem::take(&mut val.others);
            let (group, entry) = add_nick(&mut card, val, &mut ctr);
            card.nicknames.insert(group, entry);
            for val in others {
                let (group, entry) = add_nick(&mut card, val, &mut ctr);
                card.nicknames.insert(group, entry);
            }
            // Keep the old single-value display semantics.
            card.set_additionalname(first_value);
        }
        if let Some(val) = self.0.swap_remove("BDAY") {
            /* 4.3.4.  DATE-AND-OR-TIME

            Either a DATE-TIME, a DATE, or a TIME value.  To allow unambiguous
            interpretation, a stand-alone TIME value is always preceded by a "T".

            Examples for "date-and-or-time":

                      19961022T140000
                      --1022T1400
                      ---22T14
                      19850412
                      1985-04
                      1985
                      --0412
                      ---12
                      T102200
                      T1022
                      T10
                      T-2200
                      T--00
                      T102200Z
                      T102200-0800
                      */
            card.birthday =
                crate::utils::datetime::timestamp_from_string(val.value.as_str(), "%Y%m%d\0")
                    .unwrap_or_default();
        }
        if let Some(mut val) = self.0.swap_remove("EMAIL") {
            use crate::contacts::jscontact::json_types::EmailAddress;

            let add_email =
                |card: &'_ mut Card, val: ContentLine, ctr: &mut u32| -> (String, EmailAddress) {
                    let ContentLine {
                        group,
                        value,
                        others,
                        params,
                    } = val;
                    debug_assert!(others.is_empty());

                    let mut group = group.unwrap_or_else(|| format!("EMAIL-{ctr}"));
                    let pref = params
                        .iter()
                        .find_map(|p| p.strip_prefix("PREF=")?.parse::<u64>().ok());
                    let contexts = params
                        .iter()
                        .find_map(|p| Some((p.strip_prefix("TYPE=")?.to_string(), true)))
                        .into_iter()
                        .collect();
                    while card.emails.contains_key(&group) {
                        *ctr += 1;
                        group = format!("EMAIL-{ctr}");
                    }
                    *ctr += 1;
                    (
                        group,
                        EmailAddress {
                            address: value,
                            pref,
                            contexts,
                            ..EmailAddress::default()
                        },
                    )
                };

            let first_value = val.value.clone();
            let mut ctr = 1;
            let others = std::mem::take(&mut val.others);
            let (group, entry) = add_email(&mut card, val, &mut ctr);
            card.emails.insert(group, entry);
            for val in others {
                let (group, entry) = add_email(&mut card, val, &mut ctr);
                card.emails.insert(group, entry);
            }
            // Keep the old single-value display semantics.
            card.set_email(first_value);
        }
        if let Some(val) = self.0.swap_remove("CATEGORIES") {
            let ContentLine {
                group: _,
                value,
                others,
                ..
            } = val;
            if !value.is_empty() {
                card.keywords.insert(value, true);
            }
            for val in others {
                let ContentLine { value, others, .. } = val;
                debug_assert!(others.is_empty());
                card.keywords.insert(value, true);
            }
        }
        if let Some(mut val) = self.0.swap_remove("URL") {
            use crate::contacts::jscontact::json_types::{Link, Resource};

            let add_link =
                |card: &'_ mut Card, val: ContentLine, ctr: &mut u32| -> (String, Link) {
                    let ContentLine {
                        group,
                        value,
                        others,
                        params,
                    } = val;
                    debug_assert!(others.is_empty());

                    let mut group = group.unwrap_or_else(|| format!("LINK-{ctr}"));
                    let pref = params
                        .iter()
                        .find_map(|p| p.strip_prefix("PREF=")?.parse::<u64>().ok());
                    let contexts = params
                        .iter()
                        .find_map(|p| Some((p.strip_prefix("TYPE=")?.to_string(), true)))
                        .into_iter()
                        .collect();
                    while card.links.contains_key(&group) {
                        *ctr += 1;
                        group = format!("LINK-{ctr}");
                    }
                    *ctr += 1;
                    (
                        group,
                        Link {
                            resource: Resource {
                                uri: value,
                                pref,
                                contexts,
                                ..Resource::default()
                            },
                            ..Link::default()
                        },
                    )
                };

            let first_value = val.value.clone();
            let mut ctr = 1;
            let others = std::mem::take(&mut val.others);
            let (group, entry) = add_link(&mut card, val, &mut ctr);
            card.links.insert(group, entry);
            for val in others {
                let (group, entry) = add_link(&mut card, val, &mut ctr);
                card.links.insert(group, entry);
            }
            // Keep the old single-value display semantics.
            card.set_url(first_value);
        }

        {
            use crate::contacts::jscontact::json_types::OnlineService;

            let add_os =
                |card: &'_ mut Card, val: ContentLine, ctr: &mut u32| -> (String, OnlineService) {
                    let ContentLine {
                        group,
                        value,
                        others,
                        params,
                    } = val;
                    debug_assert!(others.is_empty());

                    let mut group = group.unwrap_or_else(|| format!("OS-{ctr}"));
                    let service = params
                        .iter()
                        .find_map(|p| Some(p.strip_prefix("SERVICE-TYPE=")?.to_string()))
                        .unwrap_or_default();
                    let user = params
                        .iter()
                        .find_map(|p| Some(p.strip_prefix("USERNAME=")?.to_string()))
                        .unwrap_or_default();
                    let pref = params
                        .iter()
                        .find_map(|p| p.strip_prefix("PREF=")?.parse::<u64>().ok());
                    let contexts = params
                        .iter()
                        .find_map(|p| Some((p.strip_prefix("TYPE=")?.to_string(), true)))
                        .into_iter()
                        .collect();
                    while card.online_services.contains_key(&group) {
                        *ctr += 1;
                        group = format!("OS-{ctr}");
                    }
                    *ctr += 1;
                    (
                        group,
                        OnlineService {
                            service,
                            uri: value,
                            user,
                            pref,
                            contexts,
                            ..OnlineService::default()
                        },
                    )
                };
            if let Some(mut val) = self.0.swap_remove("IMPP") {
                let mut ctr = 1;
                let others = std::mem::take(&mut val.others);
                let (group, mut entry) = add_os(&mut card, val, &mut ctr);
                entry.extra.insert("vCardName".into(), "impp".into());
                card.online_services.insert(group, entry);
                for val in others {
                    let (group, mut entry) = add_os(&mut card, val, &mut ctr);
                    entry.extra.insert("vCardName".into(), "impp".into());
                    card.online_services.insert(group, entry);
                }
            }

            if let Some(mut val) = self.0.swap_remove("SOCIALPROFILE") {
                let mut ctr = 1;
                let others = std::mem::take(&mut val.others);
                let (group, mut entry) = add_os(&mut card, val, &mut ctr);
                if !has_uri_scheme(&entry.uri) {
                    std::mem::swap(&mut entry.user, &mut entry.uri);
                }
                entry
                    .extra
                    .insert("vCardName".into(), "socialprofile".into());
                card.online_services.insert(group, entry);
                for val in others {
                    let (group, mut entry) = add_os(&mut card, val, &mut ctr);
                    if !has_uri_scheme(&entry.uri) {
                        std::mem::swap(&mut entry.user, &mut entry.uri);
                    }
                    entry
                        .extra
                        .insert("vCardName".into(), "socialprofile".into());
                    card.online_services.insert(group, entry);
                }
            }
        }

        if let Some(mut val) = self.0.swap_remove("KEY") {
            use crate::contacts::jscontact::json_types::{CryptoKey, Resource};

            let add_key =
                |card: &'_ mut Card, val: ContentLine, ctr: &mut u32| -> (String, CryptoKey) {
                    let ContentLine {
                        group,
                        value,
                        others,
                        params,
                    } = val;
                    debug_assert!(others.is_empty());

                    let mut group = group.unwrap_or_else(|| format!("KEY-{ctr}"));
                    let pref = params
                        .iter()
                        .find_map(|p| p.strip_prefix("PREF=")?.parse::<u64>().ok());
                    let contexts = params
                        .iter()
                        .find_map(|p| Some((p.strip_prefix("TYPE=")?.to_string(), true)))
                        .into_iter()
                        .collect();
                    while card.crypto_keys.contains_key(&group) {
                        *ctr += 1;
                        group = format!("KEY-{ctr}");
                    }
                    *ctr += 1;
                    (
                        group,
                        CryptoKey {
                            __type: None,
                            resource: Resource {
                                uri: value,
                                contexts,
                                pref,
                                ..Resource::default()
                            },
                        },
                    )
                };

            let first_value = val.value.clone();
            let mut ctr = 1;
            let others = std::mem::take(&mut val.others);
            let (group, entry) = add_key(&mut card, val, &mut ctr);
            card.crypto_keys.insert(group, entry);
            for val in others {
                let (group, entry) = add_key(&mut card, val, &mut ctr);
                card.crypto_keys.insert(group, entry);
            }
            // Keep the old single-value display semantics.
            card.set_key(first_value);
        }
        for (k, v) in self.0.into_iter() {
            if k.eq_ignore_ascii_case("VERSION") || k.eq_ignore_ascii_case("N") {
                continue;
            }
            card.set_extra_property(&k, v.value);
        }

        Ok(card)
    }
}

/// Parse one or more vCard text blocks out of a `.vcf` file.
///
/// A `.vcf` file is a *vCard stream* (RFC 6350 §3.1/§3.2): any number of
/// `BEGIN:VCARD`…`END:VCARD` blocks separated by (at least) line endings.
/// [`take_until`] hands over each block *including* its `END:VCARD` token but
/// not the line ending that follows it, so the leading whitespace skip of
/// [`whitespace_wrap`] is what lets the next block's `BEGIN:VCARD` peek match
/// again. Without it, `one_or_more` stopped after the first card and every
/// further contact in the file was silently dropped (address-book exports are
/// almost always multi-card files).
fn parse_card<'a>() -> impl Parser<'a, Vec<&'a str>> {
    move |input| {
        one_or_more(whitespace_wrap(prefix(
            peek(match_literal_anycase(HEADER)),
            take_until(match_literal_anycase(FOOTER)),
        )))
        .parse(input)
    }
}

pub fn load_cards(p: &std::path::Path) -> Result<Vec<Card>> {
    /// Read and parse a single `.vcf` file, pushing one result per card.
    ///
    /// Read errors (including non-UTF-8 contents: a vCard is a UTF-8 text
    /// format) must cost only that file. Propagating them with `?` used to
    /// abort the whole address book load and drop every card from every other
    /// file too (CWE-754), so they are downgraded to warnings here.
    fn read_file(
        f: &std::path::Path,
        ret: &mut Vec<Result<Card>>,
        contents: &mut String,
        is_any_valid: &mut bool,
    ) {
        use std::io::Read as _;
        contents.clear();
        if let Err(err) = std::fs::File::open(f)
            .map_err(|err| err.to_string())
            .and_then(|mut file| file.read_to_string(contents).map_err(|err| err.to_string()))
        {
            tracing::warn!("Could not read vcard file {}: {}", f.display(), err);
            return;
        }
        match parse_card().parse(contents.as_str()) {
            Ok((_, c)) => {
                for s in c {
                    ret.push(
                        CardDeserializer::try_from_str(s)
                            .and_then(TryInto::try_into)
                            .map(|mut card| {
                                Card::set_external_resource(&mut card, true);
                                *is_any_valid = true;
                                card
                            }),
                    );
                }
            }
            Err(err) => {
                tracing::warn!("Could not parse vcard from {}: {}", f.display(), err);
            }
        }
    }

    let mut ret: Vec<Result<_>> = Vec::new();
    let mut is_any_valid = false;
    let mut contents = String::with_capacity(256);
    if !p.is_dir() {
        read_file(p, &mut ret, &mut contents, &mut is_any_valid);
    } else if let Ok(vcf_dir) = std::fs::read_dir(p) {
        for f in vcf_dir {
            if f.is_err() {
                continue;
            }
            let f = f?.path();
            if f.is_file() {
                read_file(&f, &mut ret, &mut contents, &mut is_any_valid);
            }
        }
    }
    for c in &ret {
        if c.is_err() {
            tracing::debug!("Could not parse contact card: {c:?}");
        }
    }
    if is_any_valid {
        ret.retain(Result::is_ok);
    }
    ret.into_iter().collect::<Result<Vec<Card>>>()
}

#[derive(Debug)]
pub struct VCardContacts {
    pub path: PathBuf,
}

impl ContactBackend for VCardContacts {
    fn capabilities(&mut self) -> ContactBackendCapabilities {
        ContactBackendCapabilities::default()
    }

    fn address_books(&mut self) -> ResultFuture<Vec<AddressBookName>> {
        Ok(Box::pin(async {
            Ok(vec![AddressBookName("vcard_folder".into())])
        }))
    }

    fn fetch_book(&mut self, address_book: &AddressBookName) -> ResultFuture<AddressBook> {
        if address_book.0.as_ref() != "vcard_folder" {
            return Err(Error::new("").set_kind(ErrorKind::ValueError));
        }
        let name = address_book.clone();
        let vcard_path = self.path.clone();
        Ok(Box::pin(async move {
            let expanded_path = vcard_path.expand();
            let cards = load_cards(&expanded_path)
                .map_err(|err| {
                    let mut err = err.set_summary("Could not load vcards");
                    if expanded_path != *vcard_path {
                        err = err.set_details(format!(
                            "Note: vcard_folder was expanded from {} to {}",
                            vcard_path.display(),
                            expanded_path.display()
                        ));
                    }
                    err
                })?
                .into_iter()
                .map(|c| (c.id, c))
                .collect();

            Ok(AddressBook {
                name,
                cards,
                read_only: true,
            })
        }))
    }

    fn search(
        &self,
        _term: &str,
        _address_book: Option<&AddressBookName>,
    ) -> ResultFuture<Vec<Card>> {
        Err(Error::new("").set_kind(ErrorKind::NotSupported))
    }

    fn add(&mut self, _: &AddressBookName, _: Card) -> ResultFuture<Card> {
        Err(Error::new("").set_kind(ErrorKind::NotSupported))
    }

    fn edit(&mut self, _: &AddressBookName, _: Card) -> ResultFuture<Card> {
        Err(Error::new("").set_kind(ErrorKind::NotSupported))
    }

    fn delete(&mut self, _: &AddressBookName, _: Card) -> ResultFuture<()> {
        Err(Error::new("").set_kind(ErrorKind::NotSupported))
    }
}

#[test]
fn test_vcard_v4_parse() {
    // Test with CRLF endings
    let j = "BEGIN:VCARD\r\nVERSION:4.0\r\nN:Gump;Forrest;;Mr.;\r\nFN:Forrest Gump\r\nORG:Bubba Gump Shrimp Co.\r\nTITLE:Shrimp Man\r\nPHOTO;MEDIATYPE=image/gif:http://www.example.com/dir_photos/my_photo.gif\r\nTEL;TYPE=work,voice;VALUE=uri:tel:+1-111-555-1212\r\nTEL;TYPE=home,voice;VALUE=uri:tel:+1-404-555-1212\r\nADR;TYPE=WORK;PREF=1;LABEL=\"100 Waters Edge\\nBaytown\\, LA 30314\\nUnited States of America\":;;100 Waters Edge;Baytown;LA;30314;United States of America\r\nADR;TYPE=HOME;LABEL=\"42 Plantation St.\\nBaytown\\, LA 30314\\nUnited States of America\":;;42 Plantation St.;Baytown;LA;30314;United States of America\r\nEMAIL:forrestgump@example.com\r\nREV:20080424T195243Z\r\nx-qq:21588891\r\nEND:VCARD\r\n";

    let parsed = CardDeserializer::try_from_str(j).unwrap();
    let contents = indexmap::indexmap! {
        "VERSION".to_string() => ContentLine {
            group: None,
            params: vec![],
            value: "4.0".to_string(),
            others: vec![],
        },
        "N".to_string() => ContentLine {
            group: None,
            params: vec![],
            value: "Gump;Forrest;;Mr.;".to_string(),
            others: vec![],
        },
        "FN".to_string() => ContentLine {
            group: None,
            params: vec![],
            value: "Forrest Gump".to_string(),
            others: vec![],
        },
        "ORG".to_string() => ContentLine {
            group: None,
            params: vec![],
            value: "Bubba Gump Shrimp Co.".to_string(),
            others: vec![],
        },
        "TITLE".to_string() => ContentLine {
            group: None,
            params: vec![],
            value: "Shrimp Man".to_string(),
            others: vec![],
        },
        "PHOTO".to_string() => ContentLine {
            group: None,
            params: vec![
                "MEDIATYPE=image/gif".to_string(),
            ],
            value: "http://www.example.com/dir_photos/my_photo.gif".to_string(),
            others: vec![],
        },
        "TEL".to_string() => ContentLine {
            group: None,
            params: vec!["TYPE=work,voice".into(), "VALUE=uri".into()],
            value: "tel:+1-111-555-1212".into(),
            others: vec![ContentLine {
                group: None,
                params: vec![
                    "TYPE=home,voice".to_string(),
                    "VALUE=uri".to_string(),
                ],
                value: "tel:+1-404-555-1212".to_string(),
                others: vec![],
            }],
        },
        "ADR".to_string() => ContentLine {
            group: None,
            params: vec![
                "TYPE=WORK".to_string(),
                "PREF=1".to_string(),
                "LABEL=\"100 Waters Edge\\nBaytown\\, LA 30314\\nUnited States of America\"".to_string(),
            ],
            value: ";;100 Waters Edge;Baytown;LA;30314;United States of America".to_string(),
            others: vec![ContentLine {
                group: None,
                params: vec![
                    "TYPE=HOME".to_string(),
                    "LABEL=\"42 Plantation St.\\nBaytown\\, LA 30314\\nUnited States of America\"".to_string(),
                ],
                value: ";;42 Plantation St.;Baytown;LA;30314;United States of America".to_string(),
                others: vec![],
            }],
        },
        "EMAIL".to_string() => ContentLine {
            group: None,
            params: vec![],
            value: "forrestgump@example.com".to_string(),
            others: vec![],
        },
        "REV".to_string() => ContentLine {
            group: None,
            params: vec![],
            value: "20080424T195243Z".to_string(),
            others: vec![],
        },
        "x-qq".to_string() => ContentLine {
            group: None,
            params: vec![],
            value: "21588891".to_string(),
            others: vec![],
        },
    };
    assert_eq!(parsed.version(), VCardVersion4::NAME);
    assert_eq!(parsed.0, contents,);

    // Test with LF endings
    let j = "BEGIN:VCARD\nVERSION:4.0\nN:Gump;Forrest;;Mr.;\nFN:Forrest Gump\nORG:Bubba Gump Shrimp Co.\nTITLE:Shrimp Man\nPHOTO;MEDIATYPE=image/gif:http://www.example.com/dir_photos/my_photo.gif\nTEL;TYPE=work,voice;VALUE=uri:tel:+1-111-555-1212\nTEL;TYPE=home,voice;VALUE=uri:tel:+1-404-555-1212\nADR;TYPE=WORK;PREF=1;LABEL=\"100 Waters Edge\\nBaytown\\, LA 30314\\nUnited States of America\":;;100 Waters Edge;Baytown;LA;30314;United States of America\nADR;TYPE=HOME;LABEL=\"42 Plantation St.\\nBaytown\\, LA 30314\\nUnited States of America\":;;42 Plantation St.;Baytown;LA;30314;United States of America\nEMAIL:forrestgump@example.com\nREV:20080424T195243Z\nx-qq:21588891\nEND:VCARD\n";
    let parsed2 = CardDeserializer::try_from_str(j).unwrap();
    assert_eq!(parsed2.version(), VCardVersion4::NAME);
    assert_eq!(parsed2.0, contents,);
}

// Regression tests (CWE-1287) for the header/footer variant-length mismatch in
// `try_from_str`: inputs ending with the bare `FOOTER` ("END:VCARD", 9 bytes)
// pass the accept-gate, but the slicing branches subtracted the
// newline-terminated variant's length (`FOOTER_CRLF` = 11, `FOOTER_LF` = 10),
// overshooting the end index into the content — a non-char-boundary panic when
// a multi-byte character sits at the tail, or silent truncation. The slice must
// use the *actually matched* variant's length in both branches.
#[test]
fn test_vcard_v4_multibyte_tail_bare_footer_snowman() {
    // HEADER_CRLF branch: 3-byte ☃ flush against the bare footer.
    let j = "BEGIN:VCARD\r\nFN:☃END:VCARD";
    let parsed = CardDeserializer::try_from_str(j).unwrap();
    assert_eq!(parsed.0["FN"].value, "☃");

    // HEADER_LF branch: the 27-byte repro.
    let j = "BEGIN:VCARD\nFN:☃END:VCARD";
    let parsed = CardDeserializer::try_from_str(j).unwrap();
    assert_eq!(parsed.0["FN"].value, "☃");

    // Multi-byte char not flush against the footer.
    let j = "BEGIN:VCARD\r\nFN:a☃END:VCARD";
    let parsed = CardDeserializer::try_from_str(j).unwrap();
    assert_eq!(parsed.0["FN"].value, "a☃");
}

#[test]
fn test_vcard_v4_multibyte_tail_bare_footer_e_acute() {
    // HEADER_CRLF branch: end index lands exactly on é's first byte, so the
    // pre-fix behaviour was silent truncation (FN parsed as "") instead of a
    // panic; assert the value survives.
    let j = "BEGIN:VCARD\r\nFN:éEND:VCARD";
    let parsed = CardDeserializer::try_from_str(j).unwrap();
    assert_eq!(parsed.0["FN"].value, "é");

    // HEADER_LF branch: end index lands on é's second byte → char-boundary
    // panic before the fix.
    let j = "BEGIN:VCARD\nFN:éEND:VCARD";
    let parsed = CardDeserializer::try_from_str(j).unwrap();
    assert_eq!(parsed.0["FN"].value, "é");
}

#[test]
fn test_vcard_v4_bare_footer_wellformed() {
    // Bare-footer (no trailing newline) inputs remain accepted and every
    // content-line value is kept intact, for both newline conventions.
    for nl in ["\r\n", "\n"] {
        let j = format!(
            "BEGIN:VCARD{nl}VERSION:4.0{nl}N:Gump;Forrest;;Mr.;{nl}FN:Forrest \
             Gump{nl}EMAIL:forrestgump@example.com{nl}END:VCARD"
        );
        let parsed = CardDeserializer::try_from_str(&j).unwrap();
        assert_eq!(parsed.0["FN"].value, "Forrest Gump");
        assert_eq!(parsed.0["N"].value, "Gump;Forrest;;Mr.;");
        assert_eq!(parsed.0["EMAIL"].value, "forrestgump@example.com");
    }
}

#[test]
fn test_vcard_malformed_probes_rejected() {
    // Malformed-input probes: rejected with an error, never a panic.
    CardDeserializer::try_from_str("").unwrap_err();
    CardDeserializer::try_from_str("END:VCARD").unwrap_err();
    CardDeserializer::try_from_str("END:VCARD\r\n").unwrap_err();
    CardDeserializer::try_from_str("BEGIN:VCARD\r\nEND:VCARD\n").unwrap_err();
}

// Regression tests for CVE-2006-2386 (issue #29, the WAB
// contact-record attack mapped onto meli's address-book parsing):
// multi-card `.vcf` files and unreadable/binary-garbage files in the
// `vcard_folder` are exactly the shapes a mail-distributed address
// book entry arrives in once saved to disk.

#[test]
fn test_vcard_parse_card_stream_loads_every_card() {
    // Regression: `take_until` hands over each block including its
    // `END:VCARD` but not the following line ending, so without the
    // whitespace skip the stream parser stopped after the first card
    // and every further contact was silently dropped.
    let stream = "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:One\r\nEND:VCARD\r\n\
                  BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Two\r\nEND:VCARD\r\n\
                  BEGIN:VCARD\nVERSION:4.0\nFN:Three\nEND:VCARD\n";
    let (_, cards) = parse_card().parse(stream).unwrap();
    assert_eq!(
        cards.len(),
        3,
        "every card of a multi-card stream must load"
    );
    assert!(cards[0].ends_with("END:VCARD"));
    assert!(cards[1].ends_with("END:VCARD"));
    // Mixed newline conventions and surrounding blank lines keep working.
    let (_, cards) = parse_card()
        .parse("\r\nBEGIN:VCARD\r\nFN:One\r\nEND:VCARD\r\n\r\nBEGIN:VCARD\nFN:Two\nEND:VCARD\n")
        .unwrap();
    assert_eq!(cards.len(), 2);
    // A lone card keeps parsing unchanged.
    let (_, cards) = parse_card()
        .parse("BEGIN:VCARD\r\nVERSION:4.0\r\nFN:One\r\nEND:VCARD\r\n")
        .unwrap();
    assert_eq!(cards.len(), 1);
}

#[test]
fn test_vcard_load_cards_multi_card_file_loads_every_card() {
    // End-to-end over `load_cards`: a multi-contact `.vcf` file (the
    // shape every address-book export produces) must yield one `Card`
    // per `BEGIN:VCARD` block, not just the first.
    let tempdir = tempfile::tempdir().unwrap();
    std::fs::write(
        tempdir.path().join("contacts.vcf"),
        "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:One\r\nEMAIL:one@x.example\r\nEND:VCARD\r\n\
         BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Two\r\nEMAIL:two@x.example\r\nEND:VCARD\r\n",
    )
    .unwrap();
    let cards = load_cards(tempdir.path()).unwrap();
    let mut names: Vec<_> = cards.iter().map(|c| c.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["One", "Two"]);
}

#[test]
fn test_vcard_load_cards_isolates_unreadable_files() {
    // Regression (CWE-754): one binary-garbage `.vcf` file used to
    // abort the whole `load_cards` call through `?`, dropping every
    // contact from every other file. A file that cannot be opened or
    // decoded must cost only itself.
    let tempdir = tempfile::tempdir().unwrap();
    std::fs::write(
        tempdir.path().join("good.vcf"),
        "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Good\r\nEMAIL:good@x.example\r\nEND:VCARD\r\n",
    )
    .unwrap();
    std::fs::write(
        tempdir.path().join("garbage.vcf"),
        b"BEGIN:VCARD\r\nFN:\x00\x01\x9c\xba\xff\xfe binary garbage \x80\x81\r\nEND:VCARD\r\n",
    )
    .unwrap();
    let cards = load_cards(tempdir.path()).unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].name, "Good");
}

#[test]
fn test_vcard_v4_multiple_values() {
    let j = "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Forrest Gump\r\n\
             NICKNAME;TYPE=work;PREF=1:Forrest\r\n\
             NICKNAME;TYPE=home:Forresty\r\n\
             EMAIL;TYPE=work,voice;PREF=1:forrest@work.example\r\n\
             EMAIL;TYPE=home:forrest@home.example\r\n\
             URL;TYPE=work:https://work.example/forrest\r\n\
             URL;TYPE=home:https://home.example/forrest\r\n\
             CATEGORIES:friends\r\n\
             CATEGORIES:colleagues\r\n\
             IMPP;SERVICE-TYPE=matrix;USERNAME=@forrest:matrix:@forrest:example.org\r\n\
             SOCIALPROFILE;SERVICE-TYPE=github:https://github.com/forrest\r\n\
             KEY;TYPE=work;PREF=2:https://keys.example/forrest.asc\r\n\
             KEY:https://keys.example/forrest2.asc\r\n\
             END:VCARD\r\n";
    let card: Card = CardDeserializer::try_from_str(j)
        .unwrap()
        .try_into()
        .unwrap();

    // Single-value accessors keep returning the first entry.
    assert_eq!(card.name, "Forrest Gump");
    assert_eq!(card.additionalname, "Forrest");
    assert_eq!(card.email, "forrest@work.example");
    assert_eq!(card.url, "https://work.example/forrest");
    assert_eq!(card.key, "https://keys.example/forrest.asc");

    assert_eq!(card.nicknames.len(), 2);
    let (_, nick) = card.nicknames.get_index(0).unwrap();
    assert_eq!(nick.name, "Forrest");
    assert_eq!(nick.pref, Some(1));
    assert!(nick.contexts.contains_key("work"));
    let (_, nick) = card.nicknames.get_index(1).unwrap();
    assert_eq!(nick.name, "Forresty");
    assert!(nick.contexts.contains_key("home"));

    assert_eq!(card.emails.len(), 2);
    let (_, email) = card.emails.get_index(0).unwrap();
    assert_eq!(email.address, "forrest@work.example");
    assert_eq!(email.pref, Some(1));
    assert!(email.contexts.contains_key("work,voice"));
    let (_, email) = card.emails.get_index(1).unwrap();
    assert_eq!(email.address, "forrest@home.example");
    assert!(email.contexts.contains_key("home"));

    assert_eq!(card.links.len(), 2);
    let (_, link) = card.links.get_index(0).unwrap();
    assert_eq!(link.resource.uri, "https://work.example/forrest");
    assert!(link.resource.contexts.contains_key("work"));
    let (_, link) = card.links.get_index(1).unwrap();
    assert_eq!(link.resource.uri, "https://home.example/forrest");

    assert_eq!(card.keywords.len(), 2);
    assert!(card.keywords.contains_key("friends"));
    assert!(card.keywords.contains_key("colleagues"));
    // CATEGORIES is consumed as a keyword and must not leak into extras.
    assert!(!card.extra_properties.contains_key("CATEGORIES"));

    assert_eq!(card.online_services.len(), 2);
    let (_, service) = card.online_services.get_index(0).unwrap();
    assert_eq!(service.service, "matrix");
    assert_eq!(service.user, "@forrest");
    assert_eq!(service.uri, "matrix:@forrest:example.org");
    assert_eq!(service.extra.get("vCardName").unwrap(), "impp");
    let (_, service) = card.online_services.get_index(1).unwrap();
    assert_eq!(service.service, "github");
    assert_eq!(service.uri, "https://github.com/forrest");
    assert_eq!(service.extra.get("vCardName").unwrap(), "socialprofile");

    assert_eq!(card.crypto_keys.len(), 2);
    let (_, key) = card.crypto_keys.get_index(0).unwrap();
    assert_eq!(key.resource.uri, "https://keys.example/forrest.asc");
    assert_eq!(key.resource.pref, Some(2));
    assert!(key.resource.contexts.contains_key("work"));
    let (_, key) = card.crypto_keys.get_index(1).unwrap();
    assert_eq!(key.resource.uri, "https://keys.example/forrest2.asc");

    // Everything else was consumed; only VERSION and N are skipped.
    assert!(card.extra_properties.is_empty());
}

#[test]
fn test_vcard_load_cards_single_file() {
    // `load_cards` accepts a single `.vcf` path, not just a directory, and a
    // multi-card file still yields every card.
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path().join("contacts.vcf");
    std::fs::write(
        &path,
        "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:One\r\nEMAIL:one@x.example\r\nEND:VCARD\r\n\
         BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Two\r\nEMAIL:two@x.example\r\nEND:VCARD\r\n",
    )
    .unwrap();
    let cards = load_cards(&path).unwrap();
    let mut names: Vec<_> = cards.iter().map(|c| c.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["One", "Two"]);
    assert!(cards.iter().all(Card::external_resource));
}
