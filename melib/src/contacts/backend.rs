//
// meli
//
// Copyright 2026 Manos Pitsidianakis
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

//! Pluggable contact backend contract, semantically ported from upstream
//! meli `62c24b52`/`3e7133b9`/`5eda1ee1`/`6c61f34b` (upstream HEAD
//! `1ada94c3`).
//!
//! Fork deviation: upstream declares these types in
//! `melib/src/contacts/mod.rs` and renames the historical flat `Contacts`
//! container to `AddressBook`. The fork keeps its flat `Contacts` type and
//! its `with_account` startup loader unchanged (its shape is pinned by CVE
//! regression corpora), so the new address-book types live here instead,
//! re-exported from the module root.

use std::{borrow::Borrow, ops::Deref, sync::Arc};

use indexmap::IndexMap;

use crate::{
    backends::{prelude::ResultFuture, MailBackendExtensionStatus},
    contacts::{Card, CardId},
};

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct AddressBookName(pub Arc<str>);

impl Deref for AddressBookName {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<&str> for AddressBookName {
    fn from(s: &str) -> Self {
        Self(s.to_string().into_boxed_str().into())
    }
}

impl From<Arc<str>> for AddressBookName {
    fn from(inner: Arc<str>) -> Self {
        Self(inner)
    }
}

impl std::fmt::Display for AddressBookName {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.0.fmt(fmt)
    }
}

impl Borrow<str> for AddressBookName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AddressBook {
    pub name: AddressBookName,
    pub read_only: bool,
    pub cards: IndexMap<CardId, Card>,
}

impl AddressBook {
    pub fn new(name: AddressBookName, read_only: bool) -> Self {
        Self {
            name,
            read_only,
            cards: IndexMap::default(),
        }
    }

    /// Insert `card`, keyed by its id (idempotent replace).
    pub fn add_card(&mut self, card: Card) {
        self.cards.insert(card.id, card);
    }

    pub fn remove_card(&mut self, card_id: CardId) {
        self.cards.shift_remove(&card_id);
    }

    pub fn card_exists(&self, card_id: CardId) -> bool {
        self.cards.contains_key(&card_id)
    }

    pub fn search(&self, term: &str) -> Vec<Card> {
        self.cards
            .values()
            .filter(|c| c.email.contains(term) || c.name.contains(term))
            .cloned()
            .collect()
    }
}

impl Deref for AddressBook {
    type Target = IndexMap<CardId, Card>;

    fn deref(&self) -> &IndexMap<CardId, Card> {
        &self.cards
    }
}

#[derive(Clone, Hash, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContactBackendID {
    pub name: Box<str>,
    pub format: Box<str>,
}

#[derive(Clone, Debug)]
pub struct ContactBackendCapabilities {
    pub is_async: bool,
    pub is_remote: bool,
    pub extensions: Option<Vec<(String, MailBackendExtensionStatus)>>,
    pub supports_search: bool,
    pub metadata: Option<serde_json::Value>,
    pub can_create_address_book: bool,
    pub max_address_books_per_card: Option<u64>,
}

/// A default for [`ContactBackendCapabilities`] for use in const contexts.
pub const EMPTY_CONTACT_BACKEND_CAPABILITIES: ContactBackendCapabilities =
    ContactBackendCapabilities {
        is_async: false,
        is_remote: false,
        supports_search: false,
        extensions: None,
        metadata: None,
        can_create_address_book: false,
        max_address_books_per_card: None,
    };

impl Default for ContactBackendCapabilities {
    fn default() -> Self {
        EMPTY_CONTACT_BACKEND_CAPABILITIES
    }
}

pub trait ContactBackend: ::std::fmt::Debug + Send + Sync {
    fn capabilities(&mut self) -> ContactBackendCapabilities;
    fn is_online(&mut self) -> ResultFuture<()> {
        Ok(Box::pin(async { Ok(()) }))
    }
    fn address_books(&mut self) -> ResultFuture<Vec<AddressBookName>>;
    fn fetch_book(&mut self, address_book: &AddressBookName) -> ResultFuture<AddressBook>;
    fn add(&mut self, address_book: &AddressBookName, new_card: Card) -> ResultFuture<Card>;
    fn edit(&mut self, address_book: &AddressBookName, card: Card) -> ResultFuture<Card>;
    fn delete(&mut self, address_book: &AddressBookName, card: Card) -> ResultFuture<()>;
    fn search(&self, term: &str, address_book: Option<&AddressBookName>)
        -> ResultFuture<Vec<Card>>;
}
