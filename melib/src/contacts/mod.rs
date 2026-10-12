/*
 * meli - contacts module
 *
 * Copyright 2019 Manos Pitsidianakis
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

pub mod backend;
#[cfg(feature = "webdav")]
pub mod carddav;
pub mod jscontact;
pub mod mutt;
pub mod notmuchcontact;
pub mod vcard;

mod card;
use std::{
    borrow::Borrow,
    hash::{Hash, Hasher},
    ops::Deref,
    path::Path,
    process::Command,
    sync::{Arc, LazyLock},
};

pub use card::*;
use indexmap::{IndexMap, IndexSet};
use serde::{
    de::{Deserialize, Deserializer},
    ser::{Serialize, SerializeStruct, Serializer},
};
use uuid::Uuid;

use crate::utils::{parsec::Parser, shellexpand::ShellExpandTrait};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(from = "String")]
#[serde(into = "String")]
pub enum CardId {
    Uuid(Uuid),
    Hash(u64),
}

impl CardId {
    pub const fn as_uuid(&self) -> Option<Uuid> {
        let Self::Uuid(uuid) = self else {
            return None;
        };
        Some(*uuid)
    }
}

impl std::fmt::Display for CardId {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Uuid(u) => u.as_hyphenated().fmt(fmt),
            Self::Hash(u) => u.fmt(fmt),
        }
    }
}

impl From<CardId> for String {
    fn from(val: CardId) -> Self {
        val.to_string()
    }
}

impl From<String> for CardId {
    fn from(s: String) -> Self {
        use std::{collections::hash_map::DefaultHasher, str::FromStr};

        if let Ok(u) = Uuid::try_parse(s.as_str()) {
            Self::Uuid(u)
        } else if let Ok(num) = u64::from_str(s.trim()) {
            Self::Hash(num)
        } else {
            let mut hasher = DefaultHasher::default();
            s.hash(&mut hasher);
            Self::Hash(hasher.finish())
        }
    }
}

/// 一个通讯录（地址簿）的名称。
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

/// 单个通讯录容器，承载一组联系人卡片。
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

/// 通讯录后端标识，用于把通讯录归属到具体的来源后端。
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct ContactBackendID {
    pub name: Box<str>,
    pub format: Box<str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Contacts {
    name: String,
    pub books: IndexMap<(Arc<ContactBackendID>, AddressBookName), AddressBook>,
    pub backends: IndexSet<Arc<ContactBackendID>>,
}

/// fork 兼容门面：默认书的固定 backend 标识（S5 前端重做后随门面拆除）。
const DEFAULT_BACKEND_NAME: &str = "default";
const DEFAULT_BACKEND_FORMAT: &str = "default";
/// fork 兼容门面：默认书名称（S5 前端重做后随门面拆除）。
const DEFAULT_BOOK_NAME: &str = "default";

impl Contacts {
    pub fn new(name: String) -> Self {
        Self {
            name,
            books: IndexMap::default(),
            backends: IndexSet::default(),
        }
    }

    pub fn with_account(s: &crate::conf::AccountSettings) -> Self {
        let mut ret = Self::new(s.name.clone());
        // fork 兼容门面：账户总是拥有一个可写的默认书，旧前端的
        // add_card/cards/Deref 调用点全部落到这本书（S5 前端重做后拆除）。
        ret.add_book(
            Self::default_backend_id(),
            AddressBook::new(DEFAULT_BOOK_NAME.into(), false),
        );
        for (_backend_name, backend_conf) in &s.contacts {
            match backend_conf {
                crate::conf::ContactBackendConf::MuttAlias(mutt_alias_file) => {
                    match std::fs::read_to_string(Path::new(mutt_alias_file).expand())
                        .map_err(|err| err.to_string())
                        .and_then(|contents| {
                            contents
                                .lines()
                                .map(|line| mutt::parse_mutt_contact().parse(line).map(|(_, c)| c))
                                .collect::<Result<Vec<Card>, &str>>()
                                .map_err(|err| err.to_string())
                        }) {
                        Ok(cards) => {
                            let mut book = AddressBook::new(mutt_alias_file.as_str().into(), true);
                            for c in cards {
                                book.add_card(c);
                            }
                            ret.add_book(
                                Self::source_backend_id(&s.name, "mutt_alias_file"),
                                book,
                            );
                        }
                        Err(err) => {
                            tracing::warn!(
                                "Could not load mutt alias file {:?}: {}",
                                mutt_alias_file,
                                err
                            );
                        }
                    }
                }
                crate::conf::ContactBackendConf::VCard(vcard_path) => {
                    let expanded_path = Path::new(vcard_path).expand();
                    match vcard::load_cards(&expanded_path) {
                        Ok(cards) => {
                            let mut book = AddressBook::new(vcard_path.as_str().into(), true);
                            for c in cards {
                                book.add_card(c);
                            }
                            ret.add_book(Self::source_backend_id(&s.name, "vcard"), book);
                        }
                        Err(err) => {
                            tracing::warn!("Could not load vcards from {:?}: {}", vcard_path, err);
                            if expanded_path.display().to_string() != *vcard_path {
                                tracing::warn!(
                                    "Note: vcard_folder was expanded from {} to {}",
                                    vcard_path,
                                    expanded_path.display()
                                );
                            }
                        }
                    }
                }
                crate::conf::ContactBackendConf::NotmuchAddress(notmuch_addressbook_query) => {
                    match Command::new("sh")
                        .args([
                            "-c",
                            &format!(
                                "notmuch address --format=json --output=recipients \
                                 {notmuch_addressbook_query}",
                            ),
                        ])
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::piped())
                        .stderr(std::process::Stdio::piped())
                        .output()
                    {
                        Ok(notmuch_addresses) => {
                            if notmuch_addresses.status.success() {
                                match std::str::from_utf8(&notmuch_addresses.stdout) {
                                    Ok(notmuch_address_out) => {
                                        match notmuchcontact::parse_notmuch_contacts(
                                            notmuch_address_out,
                                        ) {
                                            Ok(contacts) => {
                                                let mut book = AddressBook::new(
                                                    notmuch_addressbook_query.as_str().into(),
                                                    true,
                                                );
                                                for c in contacts {
                                                    book.add_card(c.clone());
                                                }
                                                ret.add_book(
                                                    Self::source_backend_id(
                                                        &s.name,
                                                        "notmuch_address_book_query",
                                                    ),
                                                    book,
                                                );
                                            }
                                            Err(err) => {
                                                tracing::warn!(
                                                    "Unable to parse notmuch contact result into \
                                                     cards: {} {}",
                                                    notmuch_address_out,
                                                    err
                                                );
                                            }
                                        }
                                    }
                                    Err(err) => {
                                        tracing::warn!(
                                            "Unable to read from notmuch address query: {} {}",
                                            notmuch_addressbook_query,
                                            err
                                        );
                                    }
                                }
                            } else {
                                tracing::warn!(
                                    "Error ({}) running notmuch address: {} {}",
                                    notmuch_addresses.status,
                                    String::from_utf8_lossy(&notmuch_addresses.stdout),
                                    String::from_utf8_lossy(&notmuch_addresses.stderr)
                                );
                            }
                        }
                        Err(e) => tracing::warn!("Unable to run notmuch address command: {}", e),
                    }
                }
                #[cfg(feature = "webdav")]
                crate::conf::ContactBackendConf::CardDAV(_) => {
                    tracing::warn!(
                        "Account `{}`: CardDAV contact backend `{}` is not loaded by the legacy \
                         Contacts::with_account facade yet; skipping.",
                        s.name,
                        _backend_name
                    );
                }
            }
        }
        ret
    }

    pub fn add_book(&mut self, backend_id: Arc<ContactBackendID>, book: AddressBook) {
        let id = if let Some(id) = self.backends.iter().find(|i| {
            i.name.as_ref() == backend_id.name.as_ref()
                && i.format.as_ref() == backend_id.format.as_ref()
        }) {
            Arc::clone(id)
        } else {
            let id = backend_id;
            self.backends.insert(Arc::clone(&id));
            id
        };
        self.books.insert((id, book.name.clone()), book);
    }

    pub fn get_book(&self, backend_id: &ContactBackendID, name: &str) -> Option<&AddressBook> {
        let id = self.backends.iter().find(|i| {
            i.name.as_ref() == backend_id.name.as_ref()
                && i.format.as_ref() == backend_id.format.as_ref()
        })?;
        self.books.iter().find_map(|(k, v)| {
            if k.0 == *id && (k.1).0.as_ref() == name {
                Some(v)
            } else {
                None
            }
        })
    }

    // ---- fork 兼容门面开始（临时兼容层，S5 前端重做任务拆除）----
    // meli 前端仍按「单一通讯录」模型使用 Contacts：卡片统一写入默认书、
    // 查询跨全部书。此层只为保持旧调用点可编译且持久化格式不变。

    /// 默认书的固定 backend 标识。
    fn default_backend_id() -> Arc<ContactBackendID> {
        Arc::new(ContactBackendID {
            name: DEFAULT_BACKEND_NAME.into(),
            format: DEFAULT_BACKEND_FORMAT.into(),
        })
    }

    /// 为 with_account 的来源书构造 backend 标识：`name` 为账户名，
    /// `format` 为来源种类。
    fn source_backend_id(account_name: &str, format: &str) -> Arc<ContactBackendID> {
        Arc::new(ContactBackendID {
            name: account_name.into(),
            format: format.into(),
        })
    }

    fn is_default_book_key(key: &(Arc<ContactBackendID>, AddressBookName)) -> bool {
        key.0.name.as_ref() == DEFAULT_BACKEND_NAME
            && key.0.format.as_ref() == DEFAULT_BACKEND_FORMAT
            && (key.1).0.as_ref() == DEFAULT_BOOK_NAME
    }

    fn default_book(&self) -> Option<&AddressBook> {
        self.books
            .iter()
            .find(|(k, _)| Self::is_default_book_key(k))
            .map(|(_, v)| v)
    }

    fn default_book_mut(&mut self) -> &mut AddressBook {
        if let Some(idx) = self
            .books
            .iter()
            .position(|(k, _)| Self::is_default_book_key(k))
        {
            return self.books.get_index_mut(idx).expect("index in range").1;
        }
        self.add_book(
            Self::default_backend_id(),
            AddressBook::new(DEFAULT_BOOK_NAME.into(), false),
        );
        let idx = self
            .books
            .iter()
            .position(|(k, _)| Self::is_default_book_key(k))
            .expect("default book was just inserted");
        self.books.get_index_mut(idx).expect("index in range").1
    }

    /// 插入默认书。
    pub fn add_card(&mut self, card: Card) {
        self.default_book_mut().add_card(card);
    }

    /// 从所有书中移除该卡片。
    pub fn remove_card(&mut self, card_id: CardId) {
        for book in self.books.values_mut() {
            book.remove_card(card_id);
        }
    }

    /// 跨所有书查询卡片是否存在。
    pub fn card_exists(&self, card_id: CardId) -> bool {
        self.books.values().any(|book| book.card_exists(card_id))
    }

    /// 跨所有书搜索，过滤条件与 fork 旧实现一致。
    pub fn search(&self, term: &str) -> Vec<Card> {
        self.books
            .values()
            .flat_map(|book| book.search(term))
            .collect()
    }

    /// 返回默认书的卡片表，供前端持久化合并逻辑使用。
    pub fn cards(&self) -> &IndexMap<CardId, Card> {
        static EMPTY: LazyLock<IndexMap<CardId, Card>> = LazyLock::new(|| IndexMap::default());
        self.default_book()
            .map(|book| &book.cards)
            .unwrap_or(&EMPTY)
    }

    // ---- fork 兼容门面结束 ----
}

// fork 兼容门面：旧前端把 `Contacts` 当作单张 `IndexMap` 使用（`len()`、
// `values()`、下标索引等），这里统一映射到默认书（S5 前端重做后拆除）。
impl Deref for Contacts {
    type Target = IndexMap<CardId, Card>;

    fn deref(&self) -> &IndexMap<CardId, Card> {
        self.cards()
    }
}

// fork 兼容门面：serde 持久化仍沿用旧的单书 JSON 形状
// `{"name": <name>, "cards": {<CardId>: <Card>}}`；`books`/`backends`
// 为纯运行时结构，不落盘（S5 前端重做后拆除）。
impl Serialize for Contacts {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("Contacts", 2)?;
        state.serialize_field("name", &self.name)?;
        state.serialize_field("cards", self.cards())?;
        state.end()
    }
}

// fork 兼容门面：读取旧格式进入默认书；`name` 与历史字段名
// `display_name` 都接受，保证旧 contacts 文件可读（S5 前端重做后拆除）。
impl<'de> Deserialize<'de> for Contacts {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct LegacyContacts {
            #[serde(default, alias = "name")]
            display_name: String,
            #[serde(default)]
            cards: IndexMap<CardId, Card>,
        }

        let legacy = LegacyContacts::deserialize(deserializer)?;
        let mut contacts = Self::new(legacy.display_name);
        let mut book = AddressBook::new(DEFAULT_BOOK_NAME.into(), false);
        book.cards = legacy.cards;
        contacts.add_book(Self::default_backend_id(), book);
        Ok(contacts)
    }
}
