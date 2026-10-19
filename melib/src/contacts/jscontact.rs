//
// meli
//
// Copyright 2024 Emmanouil Pitsidianakis <manos@pitsidianak.is>
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

use crate::contacts::{Card, CardId};

/// Supported `JSContact` versions
pub trait JSContactVersion: std::fmt::Debug {}

#[derive(Debug)]
pub struct JSContactVersionUnknown;
impl JSContactVersion for JSContactVersionUnknown {}

/// Version 1 <https://www.rfc-editor.org/rfc/rfc9553.html>
#[derive(Clone, Debug)]
pub struct JSContactVersion1;
impl JSContactVersion for JSContactVersion1 {}

/// A generic representation of a `JSContact`.
///
/// Currently, only v1 is supported; the inner [`json_types::Card`] holds the
/// actual RFC 9553 contact data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JSContact<T: JSContactVersion>(
    pub json_types::Card,
    pub std::marker::PhantomData<fn() -> T>,
);

impl<V: JSContactVersion> JSContact<V> {
    /// Create a new version 1 `JSContact`.
    pub fn new_v1() -> JSContact<impl JSContactVersion> {
        JSContact(
            json_types::Card::default(),
            std::marker::PhantomData::<fn() -> JSContactVersion1>,
        )
    }
}

impl<V: JSContactVersion> From<JSContact<V>> for Card {
    fn from(val: JSContact<V>) -> Self {
        let json_types::Card {
            uid,
            name,
            emails,
            nicknames,
            organizations,
            keywords,
            links,
            online_services,
            crypto_keys,
            ..
        } = val.0;
        let mut card = Self::new();
        card.set_id(CardId::from(uid));
        if let Some(name) = name.full {
            card.set_name(name);
        } else if name.is_ordered {
            card.set_name(
                name.components
                    .iter()
                    .map(|c| c.value.as_str())
                    .collect::<Vec<_>>()
                    .join(name.default_separator.as_deref().unwrap_or(" ")),
            );
        } else {
            use json_types::NameComponentKind::*;

            let mut name_s = String::new();
            let sep = name.default_separator.as_deref().unwrap_or(" ");
            for kind in [
                Title, Given, Given2, Surname, Surname2, Credential, Generation,
            ] {
                if let Some(c) = name.components.iter().find(|c| c.kind == kind) {
                    if !name_s.is_empty() {
                        name_s.push_str(sep);
                    }
                    name_s.push_str(&c.value)
                }
            }
            card.set_name(name_s);
        }
        if let Some(e) = emails.get_index(0) {
            card.set_email(e.1.address.to_string());
        }
        // Preserve the old single-value display semantics: the first nickname
        // still populates `additionalname`.
        if let Some((_, nick)) = nicknames.get_index(0) {
            if card.additionalname.is_empty() {
                card.set_additionalname(nick.name.clone());
            }
        }
        // Preserve the old single-value fallbacks for `url` and `key`.
        let first_link_uri = links
            .get_index(0)
            .map(|(_, link)| link.resource.uri.clone());
        let first_crypto_key_uri = crypto_keys
            .get_index(0)
            .map(|(_, key)| key.resource.uri.clone());

        card.emails = emails;
        card.nicknames = nicknames;
        card.organizations = organizations;
        card.keywords = keywords;
        card.links = links;
        card.online_services = online_services;
        card.crypto_keys = crypto_keys;

        if card.url.is_empty() {
            if let Some(uri) = first_link_uri {
                card.set_url(uri);
            }
        }
        if card.key.is_empty() {
            if let Some(uri) = first_crypto_key_uri {
                card.set_key(uri);
            }
        }

        card
    }
}

impl From<Card> for JSContact<JSContactVersion1> {
    fn from(card: Card) -> Self {
        use json_types::UTCDateTime;

        let Card {
            id,
            title,
            name,
            additionalname,
            name_prefix,
            name_suffix,
            birthday,
            email,
            url,
            key,
            color: _,
            last_edited,
            extra_properties,
            external_resource: _,
            emails,
            nicknames,
            organizations,
            keywords,
            links,
            online_services,
            crypto_keys,
        } = card;

        let mut ret = Self(
            json_types::Card::default(),
            std::marker::PhantomData::<fn() -> JSContactVersion1>,
        );
        ret.0.updated = UTCDateTime(last_edited);
        if let CardId::Uuid(uid) = id {
            ret.0.uid = uid.to_string();
        }
        let need_fullname = title.is_empty()
            && additionalname.is_empty()
            && name_prefix.is_empty()
            && name_suffix.is_empty();
        if need_fullname {
            if !name.is_empty() {
                ret.0.name.full = Some(name);
            }
        } else {
            use std::collections::VecDeque;

            use json_types::{NameComponent, NameComponentKind::*};

            let given;
            let mut given2 = None;
            let mut surname = None;
            let mut surname2 = None;
            let mut rest = None;

            let words_no = name.chars().filter(|c| c.is_whitespace()).count() + 1;

            if words_no == 1 {
                given = Some(NameComponent::new(name, Given));
            } else {
                let mut words = name
                    .split_whitespace()
                    .map(|s| s.to_owned())
                    .collect::<VecDeque<_>>();
                given = words
                    .pop_front()
                    .map(|value| NameComponent::new(value, Given));
                if words.len() > 1 {
                    given2 = words
                        .pop_front()
                        .map(|value| NameComponent::new(value, Given2));
                }
                if words.len() > 1 {
                    surname = words
                        .pop_front()
                        .map(|value| NameComponent::new(value, Surname));
                }
                surname2 = words
                    .pop_front()
                    .map(|value| NameComponent::new(value, Surname2));
                if !words.is_empty() {
                    rest = Some(
                        words
                            .into_iter()
                            .map(|value| NameComponent::new(value, Surname2))
                            .collect::<Vec<_>>(),
                    );
                }
            }
            if !title.is_empty() {
                ret.0.name.components.push(NameComponent::new(title, Title));
            }
            if !name_prefix.is_empty() {
                ret.0
                    .name
                    .components
                    .push(NameComponent::new(name_prefix, Title));
            }
            if let Some(given) = given {
                ret.0.name.components.push(given);
            }
            if !additionalname.is_empty() {
                ret.0
                    .name
                    .components
                    .push(NameComponent::new(additionalname, Given2));
            }
            if let Some(given2) = given2 {
                ret.0.name.components.push(given2);
            }
            if let Some(surname) = surname {
                ret.0.name.components.push(surname);
            }
            if let Some(surname2) = surname2 {
                ret.0.name.components.push(surname2);
            }
            if let Some(rest) = rest {
                ret.0.name.components.extend(rest);
            }
            if !name_suffix.is_empty() {
                // Could also be `Generation` but the best we could do to detect
                // that is heuristics.
                ret.0
                    .name
                    .components
                    .push(NameComponent::new(name_suffix, Credential));
            }
        }
        if let Some(birthday) = birthday {
            use json_types::{Anniversary, AnniversaryDate, AnniversaryKind, Timestamp};

            ret.0.anniversaries.insert(
                "k0".into(),
                Anniversary::new(
                    AnniversaryKind::Birth,
                    AnniversaryDate::Timestamp(Timestamp::new(UTCDateTime(birthday))),
                ),
            );
        }
        if !links.is_empty() {
            ret.0.links = links;
        } else if !url.is_empty() {
            use json_types::OnlineService;

            ret.0.online_services.insert(
                "k0".into(),
                OnlineService {
                    uri: url,
                    ..OnlineService::default()
                },
            );
        }
        if !online_services.is_empty() {
            ret.0.online_services.extend(online_services);
        }
        if !crypto_keys.is_empty() {
            ret.0.crypto_keys = crypto_keys;
        } else if !key.is_empty() {
            use json_types::{CryptoKey, Resource};

            ret.0.crypto_keys.insert(
                "k0".into(),
                CryptoKey {
                    __type: None,
                    resource: Resource {
                        uri: key,
                        ..Resource::default()
                    },
                },
            );
        }
        if !nicknames.is_empty() {
            ret.0.nicknames = nicknames;
        }
        if !organizations.is_empty() {
            ret.0.organizations = organizations;
        }
        if !keywords.is_empty() {
            ret.0.keywords = keywords;
        }
        if !extra_properties.is_empty() {
            ret.0.extra.extend(
                extra_properties
                    .into_iter()
                    .map(|(s, v)| (s, serde_json::Value::String(v))),
            );
        }
        if !emails.is_empty() {
            ret.0.emails = emails;
        } else if !email.is_empty() {
            ret.0.emails.insert(
                "main".into(),
                json_types::EmailAddress {
                    address: email,
                    ..json_types::EmailAddress::default()
                },
            );
        }

        ret
    }
}

impl From<json_types::Card> for JSContact<JSContactVersion1> {
    fn from(val: json_types::Card) -> Self {
        Self(val, std::marker::PhantomData::<fn() -> JSContactVersion1>)
    }
}

impl serde::Serialize for JSContact<JSContactVersion1> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for JSContact<JSContactVersion1> {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = <json_types::Card>::deserialize(deserializer)?;
        Ok(Self(s, std::marker::PhantomData))
    }
}

pub mod json_types {
    use std::str::FromStr;

    use indexmap::IndexMap;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use crate::utils::datetime::UnixTimestamp;

    macro_rules! impl_json_type_struct_serde {
        ($t:tt, $s:literal) => {
            #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
            pub struct $t;

            impl Serialize for $t {
                fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
                where
                    S: Serializer,
                {
                    serializer.serialize_str($s)
                }
            }

            impl<'de> Deserialize<'de> for $t {
                fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
                where
                    D: Deserializer<'de>,
                {
                    let s = <String>::deserialize(deserializer)?;
                    if s != $s {
                        return Err(serde::de::Error::custom(format!(
                            concat!(r#"expected @type value ""#, $s, ", found `{}`"),
                            s
                        )));
                    }
                    Ok(Self)
                }
            }
        };
    }

    impl_json_type_struct_serde! {CardType, "Card"}
    impl_json_type_struct_serde! {NameType, "Name"}
    impl_json_type_struct_serde! {NameComponentType, "NameComponent"}
    impl_json_type_struct_serde! {EmailAddressType, "EmailAddress"}
    impl_json_type_struct_serde! {OnlineServiceType, "OnlineService"}
    impl_json_type_struct_serde! {CryptoKeyType, "CryptoKey"}
    impl_json_type_struct_serde! {AnniversaryType, "Anniversary"}
    impl_json_type_struct_serde! {PartialDateType, "PartialDate"}
    impl_json_type_struct_serde! {TimestampType, "Timestamp"}
    impl_json_type_struct_serde! {PersonalInfoType, "PersonalInfo"}
    impl_json_type_struct_serde! {AddressType, "Address"}
    impl_json_type_struct_serde! {SpeakToAsType, "SpeakToAs"}
    impl_json_type_struct_serde! {PronounsType, "Pronouns"}
    impl_json_type_struct_serde! {TitleType, "Title"}
    impl_json_type_struct_serde! {NicknameType, "Nickname"}
    impl_json_type_struct_serde! {NoteType, "Note"}
    impl_json_type_struct_serde! {AuthorType, "Author"}
    impl_json_type_struct_serde! {OrganizationType, "Organization"}
    impl_json_type_struct_serde! {LinkType, "Link"}

    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    #[non_exhaustive]
    pub enum CardVersion {
        #[default]
        _1_0,
    }

    impl Serialize for CardVersion {
        fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.serialize_str("1.0")
        }
    }

    impl<'de> Deserialize<'de> for CardVersion {
        fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let s = <String>::deserialize(deserializer)?;
            if s != "1.0" {
                return Err(serde::de::Error::custom(format!(
                    r#"expected version value "1.0", found `{s}`"#
                )));
            }
            Ok(Self::_1_0)
        }
    }

    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub struct UTCDateTime(pub UnixTimestamp);

    impl UTCDateTime {
        pub const fn is_none(&self) -> bool {
            matches!(self, Self(0))
        }

        pub fn now() -> Self {
            Self(crate::utils::datetime::now())
        }
    }

    impl std::fmt::Display for UTCDateTime {
        fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
            use crate::utils::datetime::{formats::RFC3339_DATETIME_Z, timestamp_to_string_utc};

            timestamp_to_string_utc(self.0, Some(RFC3339_DATETIME_Z), true).fmt(fmt)
        }
    }

    impl Serialize for UTCDateTime {
        fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            use crate::utils::datetime::{formats::RFC3339_DATETIME_Z, timestamp_to_string_utc};

            serializer.serialize_str(&timestamp_to_string_utc(
                self.0,
                Some(RFC3339_DATETIME_Z),
                true,
            ))
        }
    }

    impl<'de> Deserialize<'de> for UTCDateTime {
        fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            use crate::utils::datetime::{
                formats::RFC3339_DATETIME_Z, parse_timestamp_from_string,
            };

            let s = <String>::deserialize(deserializer)?;
            let Ok((_, val)) = parse_timestamp_from_string(s.as_str(), RFC3339_DATETIME_Z) else {
                return Err(serde::de::Error::custom(format!(
                    r#"expected UTCDateTime value, found `{s}`"#
                )));
            };
            Ok(Self(val))
        }
    }

    #[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "lowercase")]
    pub enum CardKind {
        #[default]
        /// A single person
        Individual,
        /// A group of people or entities
        Group,
        /// an organization
        Org,
        /// A named location
        Location,
        /// A device such as an appliance, a computer, or a network element
        Device,
        /// A software application
        Application,
    }

    impl CardKind {
        pub const fn is_individual(&self) -> bool {
            matches!(self, Self::Individual)
        }

        pub const fn as_str(&self) -> &'static str {
            match self {
                Self::Individual => "individual",
                Self::Group => "group",
                Self::Org => "org",
                Self::Location => "location",
                Self::Device => "device",
                Self::Application => "application",
            }
        }
    }

    impl FromStr for CardKind {
        type Err = serde_json::Error;

        fn from_str(s: &str) -> Result<Self, Self::Err> {
            serde_json::from_str(s)
        }
    }

    #[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Card {
        #[serde(rename = "@type")]
        pub __type: CardType,
        pub version: CardVersion,
        pub uid: String,
        #[serde(default, skip_serializing_if = "UTCDateTime::is_none")]
        pub created: UTCDateTime,
        #[serde(default, skip_serializing_if = "UTCDateTime::is_none")]
        pub updated: UTCDateTime,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        /// The language tag, as defined in `RFC5646`, that best describes the
        /// language used for text in the card, optionally including
        /// additional information such as the script.
        pub language: String,
        #[serde(default, skip_serializing_if = "CardKind::is_individual")]
        pub kind: CardKind,
        #[serde(default, skip_serializing_if = "Name::is_empty")]
        pub name: Name,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub nicknames: IndexMap<String, Nickname>,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub organizations: IndexMap<String, Organization>,
        #[serde(default, skip_serializing_if = "SpeakToAs::is_empty")]
        pub speak_to_as: SpeakToAs,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub titles: IndexMap<String, Title>,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub members: IndexMap<String, bool>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        pub prod_id: String,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub related_to: IndexMap<String, serde_json::Value>,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub emails: IndexMap<String, EmailAddress>,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub addresses: IndexMap<String, serde_json::Value>,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub online_services: IndexMap<String, OnlineService>,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub phones: IndexMap<String, serde_json::Value>,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub preferred_languages: IndexMap<String, serde_json::Value>,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub calendars: IndexMap<String, serde_json::Value>,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub scheduling_addresses: IndexMap<String, serde_json::Value>,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub crypto_keys: IndexMap<String, CryptoKey>,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub directories: IndexMap<String, serde_json::Value>,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub links: IndexMap<String, Link>,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub media: IndexMap<String, serde_json::Value>,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub localizations: IndexMap<String, serde_json::Value>,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub anniversaries: IndexMap<String, Anniversary>,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub keywords: IndexMap<String, bool>,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub notes: IndexMap<String, Note>,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub personal_info: IndexMap<String, PersonalInfo>,
        #[serde(default, flatten, skip_serializing_if = "IndexMap::is_empty")]
        pub extra: IndexMap<String, serde_json::Value>,
    }

    #[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "lowercase")]
    pub enum NameComponentKind {
        Title,
        Given,
        Given2,
        Surname,
        Surname2,
        Credential,
        Generation,
        Separator,
    }

    #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct NameComponent {
        #[serde(rename = "@type", default, skip_serializing_if = "Option::is_none")]
        pub __type: Option<NameComponentType>,
        pub value: String,
        pub kind: NameComponentKind,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        pub phonetic: String,
        #[serde(default, flatten, skip_serializing_if = "IndexMap::is_empty")]
        pub extra: IndexMap<String, serde_json::Value>,
    }

    impl NameComponent {
        #[inline]
        pub fn new(value: String, kind: NameComponentKind) -> Self {
            Self {
                __type: None,
                value,
                kind,
                phonetic: String::new(),
                extra: IndexMap::new(),
            }
        }
    }

    #[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
    #[serde(rename_all = "camelCase")]
    pub struct Name {
        #[serde(rename = "@type", default, skip_serializing_if = "Option::is_none")]
        pub __type: Option<NameType>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub components: Vec<NameComponent>,
        #[serde(default)]
        pub is_ordered: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub default_separator: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub full: Option<String>,
        #[serde(default, flatten, skip_serializing_if = "IndexMap::is_empty")]
        pub extra: IndexMap<String, serde_json::Value>,
    }

    impl Name {
        pub fn is_empty(&self) -> bool {
            self.components.is_empty() && self.full.is_none()
        }
    }

    impl serde::Serialize for Name {
        fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            use serde::ser::SerializeMap as _;

            let map_len = usize::from(self.__type.is_some())
                + usize::from(!self.components.is_empty()) * 2
                + usize::from(self.default_separator.is_some())
                + usize::from(self.full.is_some());
            let mut map = serializer.serialize_map(Some(map_len))?;
            if let Some(__type) = &self.__type {
                map.serialize_entry("@type", __type)?;
            }
            if !self.components.is_empty() {
                map.serialize_entry("components", &self.components)?;
                map.serialize_entry("isOrdered", &self.is_ordered)?;
            }
            if let Some(default_separator) = &self.default_separator {
                map.serialize_entry("defaultSeparator", default_separator)?;
            }
            if let Some(full) = &self.full {
                map.serialize_entry("full", full)?;
            }
            map.end()
        }
    }

    impl From<String> for Name {
        fn from(full: String) -> Self {
            Self {
                full: Some(full),
                ..Self::default()
            }
        }
    }

    #[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Nickname {
        #[serde(rename = "@type", default, skip_serializing_if = "Option::is_none")]
        pub __type: Option<NicknameType>,
        pub name: String,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub contexts: IndexMap<String, bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub pref: Option<u64>,
        #[serde(default, flatten, skip_serializing_if = "IndexMap::is_empty")]
        pub extra: IndexMap<String, serde_json::Value>,
    }

    impl From<String> for Nickname {
        fn from(name: String) -> Self {
            Self {
                name,
                ..Self::default()
            }
        }
    }

    #[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct EmailAddress {
        #[serde(rename = "@type", default, skip_serializing_if = "Option::is_none")]
        pub __type: Option<EmailAddressType>,
        pub address: String,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub contexts: IndexMap<String, bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub pref: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub label: Option<String>,
        #[serde(default, flatten, skip_serializing_if = "IndexMap::is_empty")]
        pub extra: IndexMap<String, serde_json::Value>,
    }

    impl From<String> for EmailAddress {
        fn from(address: String) -> Self {
            Self {
                address,
                ..Self::default()
            }
        }
    }

    impl EmailAddress {
        #[inline]
        pub fn new(address: String) -> Self {
            Self {
                address,
                ..Self::default()
            }
        }
    }

    #[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "lowercase")]
    pub enum GrammaticalGender {
        #[default]
        None,
        Animate,
        Common,
        Feminine,
        Inanimate,
        Masculine,
        Neuter,
    }

    impl GrammaticalGender {
        pub const fn is_none(&self) -> bool {
            matches!(self, Self::None)
        }

        pub const fn as_str(&self) -> &'static str {
            match self {
                Self::None => "none",
                Self::Animate => "animate",
                Self::Common => "common",
                Self::Feminine => "feminine",
                Self::Inanimate => "inanimate",
                Self::Masculine => "masculine",
                Self::Neuter => "neuter",
            }
        }
    }

    #[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct SpeakToAs {
        #[serde(rename = "@type", default, skip_serializing_if = "Option::is_none")]
        pub __type: Option<SpeakToAsType>,
        #[serde(default, skip_serializing_if = "GrammaticalGender::is_none")]
        pub grammatical_gender: GrammaticalGender,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub pronouns: IndexMap<String, Pronouns>,
        #[serde(default, flatten, skip_serializing_if = "IndexMap::is_empty")]
        pub extra: IndexMap<String, serde_json::Value>,
    }

    impl SpeakToAs {
        pub fn is_empty(&self) -> bool {
            self.grammatical_gender.is_none() && self.pronouns.is_empty()
        }
    }

    #[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Pronouns {
        #[serde(rename = "@type", default, skip_serializing_if = "Option::is_none")]
        pub __type: Option<PronounsType>,
        pub pronouns: String,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub contexts: IndexMap<String, bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub pref: Option<u64>,
        #[serde(default, flatten, skip_serializing_if = "IndexMap::is_empty")]
        pub extra: IndexMap<String, serde_json::Value>,
    }

    #[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct OnlineService {
        #[serde(rename = "@type", default, skip_serializing_if = "Option::is_none")]
        pub __type: Option<OnlineServiceType>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        pub service: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        pub uri: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        pub user: String,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub contexts: IndexMap<String, bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub pref: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub label: Option<String>,
        #[serde(default, flatten, skip_serializing_if = "IndexMap::is_empty")]
        pub extra: IndexMap<String, serde_json::Value>,
    }

    /// Abstract type for resource properties.
    #[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Resource {
        pub uri: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        pub media_type: String,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub contexts: IndexMap<String, bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub pref: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub label: Option<String>,
        #[serde(default, flatten, skip_serializing_if = "IndexMap::is_empty")]
        pub extra: IndexMap<String, serde_json::Value>,
    }

    #[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct CryptoKey {
        #[serde(rename = "@type", default, skip_serializing_if = "Option::is_none")]
        pub __type: Option<CryptoKeyType>,
        #[serde(flatten)]
        pub resource: Resource,
    }

    #[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
    #[non_exhaustive]
    #[serde(rename_all = "lowercase")]
    pub enum LinkKind {
        #[default]
        Contact,
    }

    impl LinkKind {
        pub const fn is_contact(&self) -> bool {
            matches!(self, Self::Contact)
        }
    }

    #[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Link {
        #[serde(rename = "@type", default, skip_serializing_if = "Option::is_none")]
        pub __type: Option<LinkType>,
        #[serde(default, skip_serializing_if = "LinkKind::is_contact")]
        pub kind: LinkKind,
        #[serde(flatten)]
        pub resource: Resource,
    }

    const fn is_zero(n: &u32) -> bool {
        *n == 0
    }

    #[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct PartialDate {
        #[serde(rename = "@type", default, skip_serializing_if = "Option::is_none")]
        pub __type: Option<PartialDateType>,
        #[serde(default, skip_serializing_if = "is_zero")]
        pub year: u32,
        #[serde(default, skip_serializing_if = "is_zero")]
        pub month: u32,
        #[serde(default, skip_serializing_if = "is_zero")]
        pub day: u32,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        pub calendar_scale: String,
        #[serde(default, flatten, skip_serializing_if = "IndexMap::is_empty")]
        pub extra: IndexMap<String, serde_json::Value>,
    }

    #[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Timestamp {
        #[serde(rename = "@type", default, skip_serializing_if = "Option::is_none")]
        pub __type: Option<TimestampType>,
        pub utc: UTCDateTime,
        #[serde(default, flatten, skip_serializing_if = "IndexMap::is_empty")]
        pub extra: IndexMap<String, serde_json::Value>,
    }

    impl Timestamp {
        #[inline]
        pub fn new(utc: UTCDateTime) -> Self {
            Self {
                utc,
                ..Self::default()
            }
        }
    }

    #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(untagged)]
    pub enum AnniversaryDate {
        Partial(PartialDate),
        Timestamp(Timestamp),
    }

    #[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "lowercase")]
    pub enum AnniversaryKind {
        Birth,
        Death,
        Wedding,
    }

    #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Anniversary {
        #[serde(rename = "@type", default, skip_serializing_if = "Option::is_none")]
        pub __type: Option<AnniversaryType>,
        pub kind: AnniversaryKind,
        pub date: AnniversaryDate,
        #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
        pub place: serde_json::Value,
        #[serde(default, flatten, skip_serializing_if = "IndexMap::is_empty")]
        pub extra: IndexMap<String, serde_json::Value>,
    }

    impl Anniversary {
        #[inline]
        pub fn new(kind: AnniversaryKind, date: AnniversaryDate) -> Self {
            Self {
                __type: None,
                kind,
                date,
                place: serde_json::Value::Null,
                extra: IndexMap::new(),
            }
        }
    }

    #[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "lowercase")]
    pub enum PersonalInfoKind {
        Expertise,
        Hobby,
        Interest,
    }

    #[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "lowercase")]
    pub enum PersonalInfoLevel {
        #[default]
        None,
        High,
        Medium,
        Low,
    }

    impl PersonalInfoLevel {
        pub const fn is_none(&self) -> bool {
            matches!(self, Self::None)
        }
    }

    #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct PersonalInfo {
        #[serde(rename = "@type", default, skip_serializing_if = "Option::is_none")]
        pub __type: Option<PersonalInfoType>,
        pub kind: PersonalInfoKind,
        pub value: String,
        #[serde(default, skip_serializing_if = "PersonalInfoLevel::is_none")]
        pub level: PersonalInfoLevel,
        #[serde(default, skip_serializing_if = "is_zero")]
        pub list_as: u32,
        #[serde(default, flatten, skip_serializing_if = "IndexMap::is_empty")]
        pub extra: IndexMap<String, serde_json::Value>,
    }

    #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Note {
        #[serde(rename = "@type", default, skip_serializing_if = "Option::is_none")]
        pub __type: Option<NoteType>,
        pub note: String,
        #[serde(default, skip_serializing_if = "UTCDateTime::is_none")]
        pub created: UTCDateTime,
        #[serde(default, skip_serializing_if = "UTCDateTime::is_none")]
        pub updated: UTCDateTime,
        #[serde(default, skip_serializing_if = "Author::is_empty")]
        pub author: Author,
        #[serde(default, flatten, skip_serializing_if = "IndexMap::is_empty")]
        pub extra: IndexMap<String, serde_json::Value>,
    }

    #[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Author {
        #[serde(rename = "@type", default, skip_serializing_if = "Option::is_none")]
        pub __type: Option<AuthorType>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        pub name: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        pub uri: String,
        #[serde(default, flatten, skip_serializing_if = "IndexMap::is_empty")]
        pub extra: IndexMap<String, serde_json::Value>,
    }

    impl Author {
        pub fn is_empty(&self) -> bool {
            self.name.is_empty() && self.uri.is_empty()
        }
    }

    #[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "lowercase")]
    pub enum TitleKind {
        #[default]
        Title,
        Role,
    }

    impl TitleKind {
        pub const fn is_title(&self) -> bool {
            matches!(self, Self::Title)
        }
    }

    #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Title {
        #[serde(rename = "@type", default, skip_serializing_if = "Option::is_none")]
        pub __type: Option<TitleType>,
        pub name: String,
        #[serde(default, skip_serializing_if = "TitleKind::is_title")]
        pub kind: TitleKind,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        pub organization_id: String,
        #[serde(default, flatten, skip_serializing_if = "IndexMap::is_empty")]
        pub extra: IndexMap<String, serde_json::Value>,
    }

    #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Organization {
        #[serde(rename = "@type", default, skip_serializing_if = "Option::is_none")]
        pub __type: Option<OrganizationType>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        pub name: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub units: Vec<serde_json::Value>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        pub sort_as: String,
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub contexts: IndexMap<String, bool>,
        #[serde(default, flatten, skip_serializing_if = "IndexMap::is_empty")]
        pub extra: IndexMap<String, serde_json::Value>,
    }

    #[test]
    fn test_addressbook_jscontact() {
        use super::JSContactVersion1;
        use crate::contacts::{jscontact::JSContact, Card as MeliCard, CardId};

        assert_eq!(
            Card {
                __type: CardType,
                version: CardVersion::_1_0,
                uid: "22B2C7DF-9120-4969-8460-05956FE6B065".to_string(),
                kind: CardKind::Individual,
                name: Name {
                    __type: None,
                    components: vec![],
                    full: Some("full_name".to_string()),
                    is_ordered: false,
                    default_separator: None,
                    ..Name::default()
                },
                emails: indexmap! {
                    "main".to_string() => EmailAddress {
                        __type: None,
                        address: "user@example.com".to_string(),
                        contexts: IndexMap::new(),
                        pref: None,
                        label: None,
                        ..EmailAddress::default()
                    }
                },
                ..Card::default()
            },
            serde_json::from_str(
                r#"{
            "@type": "Card",
            "version": "1.0",
            "uid": "22B2C7DF-9120-4969-8460-05956FE6B065",
            "kind": "individual",
            "emails": {
                "main": {
                    "address": "user@example.com"
                }
            },
            "name": {
                "components": [],
                "full": "full_name",
                "isOrdered": false
            }
        }"#
            )
            .unwrap(),
        );
        assert_eq!(
            MeliCard {
                last_edited: 1727155810,
                ..JSContact::<JSContactVersion1>::from(
                    serde_json::from_str::<Card>(
                        r#"{
            "@type": "Card",
            "version": "1.0",
            "uid": "22B2C7DF-9120-4969-8460-05956FE6B065",
            "kind": "individual",
            "emails": {
                "main": {
                    "address": "user@example.com"
                }
            },
            "name": {
                "components": [],
                "full": "full_name",
                "isOrdered": false
            }
        }"#
                    )
                    .unwrap()
                )
                .into()
            },
            MeliCard {
                id: CardId::Uuid(
                    uuid::Uuid::try_parse("22B2C7DF-9120-4969-8460-05956FE6B065").unwrap()
                ),
                title: "".into(),
                name: "full_name".into(),
                additionalname: "".into(),
                name_prefix: "".into(),
                name_suffix: "".into(),
                birthday: None,
                email: "user@example.com".into(),
                url: "".into(),
                key: "".into(),
                color: 0,
                last_edited: 1727155810,
                emails: indexmap! {
                    "main".to_string() => EmailAddress {
                        address: "user@example.com".to_string(),
                        ..EmailAddress::default()
                    }
                },
                extra_properties: indexmap::indexmap! {},
                external_resource: false,
                ..MeliCard::default()
            },
        );
    }
}

#[test]
fn test_card_to_jscontact_multiple_values() {
    use json_types::{
        CryptoKey, EmailAddress, Link, Nickname, OnlineService, Organization, Resource,
    };

    let mut card = Card::new();
    card.set_name("Forrest Gump".into());
    card.emails.insert(
        "work".into(),
        EmailAddress {
            address: "forrest@work.example".into(),
            ..EmailAddress::default()
        },
    );
    card.emails.insert(
        "home".into(),
        EmailAddress {
            address: "forrest@home.example".into(),
            ..EmailAddress::default()
        },
    );
    card.nicknames
        .insert("n0".into(), Nickname::from("Forrest".to_string()));
    card.organizations.insert(
        "o0".into(),
        Organization {
            __type: None,
            name: "Bubba Gump Shrimp Co.".into(),
            units: Vec::new(),
            sort_as: String::new(),
            contexts: indexmap::IndexMap::new(),
            extra: indexmap::IndexMap::new(),
        },
    );
    card.keywords.insert("friends".into(), true);
    card.links.insert(
        "l0".into(),
        Link {
            resource: Resource {
                uri: "https://example.com/forrest".into(),
                ..Resource::default()
            },
            ..Link::default()
        },
    );
    card.online_services.insert(
        "os0".into(),
        OnlineService {
            service: "matrix".into(),
            uri: "https://matrix.example/@forrest".into(),
            user: "@forrest".into(),
            ..OnlineService::default()
        },
    );
    card.crypto_keys.insert(
        "c0".into(),
        CryptoKey {
            __type: None,
            resource: Resource {
                uri: "https://keys.example/forrest.asc".into(),
                ..Resource::default()
            },
        },
    );
    let last_edited = card.last_edited;

    let js = JSContact::<JSContactVersion1>::from(card);
    assert_eq!(js.0.updated, json_types::UTCDateTime(last_edited));
    assert_eq!(js.0.emails.len(), 2);
    assert_eq!(js.0.emails["work"].address, "forrest@work.example");
    assert_eq!(js.0.emails["home"].address, "forrest@home.example");
    assert_eq!(js.0.nicknames.len(), 1);
    assert_eq!(js.0.nicknames["n0"].name, "Forrest");
    assert_eq!(js.0.organizations.len(), 1);
    assert_eq!(js.0.organizations["o0"].name, "Bubba Gump Shrimp Co.");
    assert_eq!(js.0.keywords.len(), 1);
    assert!(js.0.keywords["friends"]);
    assert_eq!(js.0.links.len(), 1);
    assert_eq!(js.0.links["l0"].resource.uri, "https://example.com/forrest");
    assert_eq!(js.0.online_services.len(), 1);
    assert_eq!(js.0.online_services["os0"].service, "matrix");
    assert_eq!(js.0.crypto_keys.len(), 1);
    assert_eq!(
        js.0.crypto_keys["c0"].resource.uri,
        "https://keys.example/forrest.asc"
    );
    // With no title/additional name/prefix/suffix the full name is preserved.
    assert_eq!(js.0.name.full.as_deref(), Some("Forrest Gump"));
    assert!(js.0.name.components.is_empty());
}

#[test]
fn test_card_to_jscontact_single_values() {
    let mut card = Card::new();
    card.set_name("Forrest Gump".into())
        .set_title("Shrimp Man".into())
        .set_additionalname("Forresty".into())
        .set_email("forrest@example.com".into())
        .set_url("https://example.com/forrest".into())
        .set_key("https://keys.example/forrest.asc".into());
    card.birthday = Some(1727155810);

    let js = JSContact::<JSContactVersion1>::from(card);
    assert_eq!(js.0.emails.len(), 1);
    assert_eq!(js.0.emails["main"].address, "forrest@example.com");
    assert_eq!(
        js.0.online_services["k0"].uri,
        "https://example.com/forrest"
    );
    assert_eq!(
        js.0.crypto_keys["k0"].resource.uri,
        "https://keys.example/forrest.asc"
    );
    // The presence of a title/additional name forces component splitting.
    assert!(js.0.name.full.is_none());
    use json_types::NameComponentKind::*;
    assert!(js
        .0
        .name
        .components
        .iter()
        .any(|c| c.kind == Title && c.value == "Shrimp Man"));
    assert!(js
        .0
        .name
        .components
        .iter()
        .any(|c| c.kind == Given && c.value == "Forrest"));
    assert!(js
        .0
        .name
        .components
        .iter()
        .any(|c| c.kind == Given2 && c.value == "Forresty"));
    let anniversary = &js.0.anniversaries["k0"];
    assert_eq!(anniversary.kind, json_types::AnniversaryKind::Birth);
    assert_eq!(
        anniversary.date,
        json_types::AnniversaryDate::Timestamp(json_types::Timestamp::new(
            json_types::UTCDateTime(1727155810)
        ))
    );
}

#[test]
fn test_jscontact_to_card_multiple_values() {
    let js: JSContact<JSContactVersion1> = serde_json::from_str(
        r#"{
            "@type": "Card",
            "version": "1.0",
            "uid": "22B2C7DF-9120-4969-8460-05956FE6B065",
            "name": { "full": "Forrest Gump" },
            "emails": {
                "work": { "address": "forrest@work.example" },
                "home": { "address": "forrest@home.example" }
            },
            "nicknames": {
                "n0": { "name": "Forrest" }
            },
            "organizations": {
                "o0": { "name": "Bubba Gump Shrimp Co." }
            },
            "keywords": { "friends": true },
            "links": {
                "l0": { "uri": "https://example.com/forrest" }
            },
            "onlineServices": {
                "os0": { "service": "matrix", "uri": "https://matrix.example/@forrest" }
            },
            "cryptoKeys": {
                "c0": { "uri": "https://keys.example/forrest.asc" }
            }
        }"#,
    )
    .unwrap();

    let card: Card = js.into();
    assert_eq!(card.emails.len(), 2);
    assert_eq!(card.email, "forrest@work.example");
    assert_eq!(card.nicknames.len(), 1);
    assert_eq!(card.additionalname, "Forrest");
    assert_eq!(card.organizations.len(), 1);
    assert_eq!(card.organizations["o0"].name, "Bubba Gump Shrimp Co.");
    assert!(card.keywords["friends"]);
    assert_eq!(card.links.len(), 1);
    assert_eq!(card.url, "https://example.com/forrest");
    assert_eq!(card.online_services.len(), 1);
    assert_eq!(card.online_services["os0"].service, "matrix");
    assert_eq!(card.crypto_keys.len(), 1);
    assert_eq!(card.key, "https://keys.example/forrest.asc");
}
