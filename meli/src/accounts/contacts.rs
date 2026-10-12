//
// meli
//
// Copyright 2026 Manos Pitsidianakis <manos@pitsidianak.is>
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

//! 账户通讯录后端的装载与增删改查作业。
//!
//! 把 melib 的多容器 [`Contacts`](melib::contacts::Contacts) 门面接到各
//! [`ContactBackend`] 实现上：账户启动时构造默认本地书、请求邮件后端自带的
//! 通讯录后端（如 JMAP），并按 `contacts` 配置逐个异步装载 mutt/vcard/notmuch/
//! `CardDAV` 后端；装载完成后拉取地址簿与卡片并入门面。增删改以 fork 的 `tokio`
//! 作业执行器提交，完成后更新门面并刷新账户状态。
//!
//! 本模块只做数据面接线：上游的通讯录 UI 回调（列表/编辑器联动、UI 事件刷新）
//! 不在此移植；账户级失败只发既有 [`UIEvent::Notification`] 通知。

use std::{
    future::Future,
    path::Path,
    sync::{Arc, Mutex},
};

use indexmap::IndexMap;
use melib::{
    backends::prelude::ResultFuture,
    contacts::{
        backend::{ContactBackend, ContactBackendCapabilities},
        mutt, notmuchcontact, vcard, AddressBook, AddressBookName, Card, CardId, ContactBackendID,
        Contacts,
    },
    error::{Error, Result, ResultIntoError},
    ErrorKind,
};

use crate::{
    accounts::{Account, JobRequest},
    jobs::{oneshot, IsAsync, JobId, JoinHandle},
    NotificationType, StatusEvent, ThreadEvent, UIEvent,
};

impl Account {
    /// 提交一个「构造通讯录后端」作业，完成后由
    /// [`Account::process_contact_event`] 接线。
    pub fn init_contact_backend<
        F: Future<Output = Result<Box<dyn ContactBackend>>> + Send + 'static,
    >(
        &mut self,
        name: String,
        format: String,
        fut: F,
        is_async: IsAsync,
    ) {
        let account_name = &self.name;
        let handle = self.main_loop_handler.job_executor.spawn(
            format!("initialize {account_name} {name} {format} contacts").into(),
            fut,
            is_async,
        );
        let job_id = handle.job_id;
        self.active_jobs.insert(
            job_id,
            JobRequest::Contacts(ContactJobRequest::Initialize {
                backend_id: Arc::new(ContactBackendID {
                    name: name.into(),
                    format: format.into(),
                }),
                handle,
            }),
        );
        self.active_job_instants
            .insert(std::time::Instant::now(), job_id);
        self.main_loop_handler
            .send(ThreadEvent::UIEvent(UIEvent::StatusEvent(
                StatusEvent::NewJob(job_id),
            )));
    }

    /// 账户装载入口：默认本地书 + 邮件后端通讯录后端 + `contacts` 配置后端。
    pub fn init_contacts(&mut self) {
        {
            // 先装默认本地书：它是唯一可写书，也是 fork 旧持久化格式的载体。
            let default_address_backend = self.default_address_backend();
            let address_book = default_address_backend.fetch_default_book();
            let backend_id = Arc::new(ContactBackendID {
                name: DefaultContacts::NAME.into(),
                format: DefaultContacts::FORMAT.into(),
            });
            self.contacts.add_book(backend_id.clone(), address_book);
            self.contact_backends.insert(
                (*backend_id).clone(),
                Arc::new(Mutex::new(Box::new(default_address_backend))),
            );
        }

        {
            let mut backend = self.backend.lock().unwrap();
            let format = self.settings.account().format.clone();
            match backend.contact_backend() {
                Ok(contacts_job) => {
                    drop(backend);
                    self.init_contact_backend(
                        "meli".to_string(),
                        format,
                        contacts_job,
                        self.is_async(),
                    );
                }
                Err(err) => {
                    if !matches!(err.kind, ErrorKind::NotImplemented) {
                        self.main_loop_handler
                            .send(ThreadEvent::UIEvent(UIEvent::Notification {
                                title: Some(
                                    format!(
                                        "{}: Could not request mail backend's contact backend",
                                        self.name
                                    )
                                    .into(),
                                ),
                                body: err.to_string().into(),
                                kind: Some(NotificationType::Error(err.kind)),
                                source: Some(err),
                            }));
                    }
                }
            }
        }
        let conf = self.settings.account().contacts.clone();
        for (name, contact_backend) in conf {
            use melib::conf::ContactBackendConf;
            let format = contact_backend_format(&contact_backend).to_string();
            match contact_backend {
                ContactBackendConf::NotmuchAddress(query) => {
                    self.init_contact_backend(
                        name,
                        format,
                        async move {
                            Ok(Box::new(notmuchcontact::NotmuchContacts { query })
                                as Box<dyn ContactBackend>)
                        },
                        IsAsync::Async,
                    );
                }
                ContactBackendConf::MuttAlias(mutt_alias_file) => {
                    let path = Path::new(&mutt_alias_file).into();
                    self.init_contact_backend(
                        name,
                        format,
                        async move {
                            Ok(Box::new(mutt::MuttContacts { path }) as Box<dyn ContactBackend>)
                        },
                        IsAsync::Async,
                    );
                }
                ContactBackendConf::VCard(vcard_path) => {
                    let path = Path::new(&vcard_path).into();
                    self.init_contact_backend(
                        name,
                        format,
                        async move {
                            Ok(Box::new(vcard::VCardContacts { path }) as Box<dyn ContactBackend>)
                        },
                        IsAsync::Async,
                    );
                }
                ContactBackendConf::CardDAV(server_conf) => {
                    self.init_contact_backend(
                        name,
                        format,
                        async move {
                            use melib::contacts::carddav::CardDAVContacts;
                            Ok(Box::new(CardDAVContacts::new(&server_conf).await?)
                                as Box<dyn ContactBackend>)
                        },
                        IsAsync::Async,
                    );
                }
            }
        }
    }

    /// 读取 fork 默认本地书：旧单书 JSON 格式，保留 `external_resource` 过滤。
    fn default_address_backend(&self) -> DefaultContacts {
        let mut cards: IndexMap<CardId, Card> = IndexMap::default();
        let data_dir = xdg::BaseDirectories::with_profile("meli", self.name.as_ref());
        if let Ok(data) = data_dir.place_data_file("contacts") {
            if data.exists() {
                match read_default_book(&data) {
                    Err(err) => {
                        self.main_loop_handler
                            .send(ThreadEvent::UIEvent(UIEvent::Notification {
                                title: Some(
                                    format!("{}: Could not load contacts", self.name).into(),
                                ),
                                body: err.to_string().into(),
                                kind: Some(NotificationType::Error(err.kind)),
                                source: Some(err),
                            }));
                    }
                    Ok(v) => {
                        cards = v;
                    }
                }
            }
        }
        DefaultContacts { cards }
    }

    /// 后端构造完成后接线：把后端登记进账户，并拉取地址簿列表。
    pub fn add_contact_backend(
        &mut self,
        backend_id: Arc<ContactBackendID>,
        mut backend: Box<dyn ContactBackend>,
    ) {
        let is_async = backend.capabilities().is_async;
        match backend.address_books() {
            Ok(v) => {
                let handle = self.main_loop_handler.job_executor.spawn(
                    format!(
                        "fetch {name} {format} address books",
                        name = backend_id.name,
                        format = backend_id.format
                    )
                    .into(),
                    v,
                    IsAsync::from(is_async),
                );
                let job_id = handle.job_id;
                self.main_loop_handler
                    .send(ThreadEvent::UIEvent(UIEvent::StatusEvent(
                        StatusEvent::NewJob(job_id),
                    )));
                self.contact_backends
                    .insert((*backend_id).clone(), Arc::new(Mutex::new(backend)));
                self.active_jobs.insert(
                    job_id,
                    JobRequest::Contacts(ContactJobRequest::AddressBooks { backend_id, handle }),
                );
                self.active_job_instants
                    .insert(std::time::Instant::now(), job_id);
            }
            Err(err) => {
                self.main_loop_handler
                    .send(ThreadEvent::UIEvent(UIEvent::Notification {
                        title: Some(
                            format!(
                                "{account}: Could not load {name} {format} backend contacts",
                                account = self.name,
                                name = backend_id.name,
                                format = backend_id.format,
                            )
                            .into(),
                        ),
                        body: err.to_string().into(),
                        kind: Some(NotificationType::Error(err.kind)),
                        source: Some(err),
                    }));
            }
        }
    }

    /// 拉取指定地址簿的卡片。
    pub fn fetch_contact_backend_address_book(
        &mut self,
        backend_id: Arc<ContactBackendID>,
        book: AddressBookName,
    ) {
        let backend = self.contact_backends[&*backend_id].as_ref();
        let mut backend = backend.lock().unwrap();
        let is_async = backend.capabilities().is_async;
        match backend.fetch_book(&book) {
            Ok(v) => {
                let handle = self.main_loop_handler.job_executor.spawn(
                    format!(
                        "fetch {name} {format} address book",
                        name = backend_id.name,
                        format = backend_id.format
                    )
                    .into(),
                    v,
                    IsAsync::from(is_async),
                );
                let job_id = handle.job_id;
                self.active_jobs.insert(
                    job_id,
                    JobRequest::Contacts(ContactJobRequest::Cards {
                        backend_id,
                        book,
                        handle,
                    }),
                );
                self.active_job_instants
                    .insert(std::time::Instant::now(), job_id);
                self.main_loop_handler
                    .send(ThreadEvent::UIEvent(UIEvent::StatusEvent(
                        StatusEvent::NewJob(job_id),
                    )));
            }
            Err(err) => {
                self.main_loop_handler
                    .send(ThreadEvent::UIEvent(UIEvent::Notification {
                        title: Some(
                            format!(
                                "{account}: Could not load {name} {format} backend contacts",
                                account = self.name,
                                name = backend_id.name,
                                format = backend_id.format
                            )
                            .into(),
                        ),
                        body: err.to_string().into(),
                        kind: Some(NotificationType::Error(err.kind)),
                        source: Some(err),
                    }));
            }
        }
    }

    /// 处理已完成的通讯录作业；返回是否认领了该作业。
    pub fn process_contact_event(&mut self, job_id: JobId, job: ContactJobRequest) -> bool {
        macro_rules! is_canceled {
            ($handle:expr) => {{
                if $handle.is_canceled() {
                    self.main_loop_handler
                        .job_executor
                        .set_job_success(job_id, false);
                    /* canceled */
                    return true;
                }
            }};
        }
        match job {
            ContactJobRequest::Initialize {
                backend_id,
                mut handle,
            } => {
                is_canceled! { handle };
                match handle.chan.try_recv() {
                    Err(_) => { /* canceled */ }
                    Ok(None) => {}
                    Ok(Some(Err(err))) => {
                        self.main_loop_handler
                            .job_executor
                            .set_job_success(job_id, false);
                        self.main_loop_handler
                            .send(ThreadEvent::UIEvent(UIEvent::Notification {
                                title: Some(
                                    format!(
                                        "{account}: {name} {format} contact initialization failed",
                                        account = self.name,
                                        name = backend_id.name,
                                        format = backend_id.format,
                                    )
                                    .into(),
                                ),
                                source: None,
                                body: err.to_string().into(),
                                kind: Some(NotificationType::Error(err.kind)),
                            }));
                    }
                    Ok(Some(Ok(backend))) => {
                        self.add_contact_backend(backend_id, backend);
                    }
                }
            }
            ContactJobRequest::AddressBooks {
                backend_id,
                mut handle,
            } => {
                is_canceled! { handle };
                match handle.chan.try_recv() {
                    Err(_) => { /* canceled */ }
                    Ok(None) => {}
                    Ok(Some(Err(err))) => {
                        self.main_loop_handler
                            .job_executor
                            .set_job_success(job_id, false);
                        self.main_loop_handler
                            .send(ThreadEvent::UIEvent(UIEvent::Notification {
                                title: Some(
                                    format!(
                                        "{account}: {name} {format} contact initialization failed",
                                        account = self.name,
                                        name = backend_id.name,
                                        format = backend_id.format,
                                    )
                                    .into(),
                                ),
                                source: None,
                                body: err.to_string().into(),
                                kind: Some(NotificationType::Error(err.kind)),
                            }));
                    }
                    Ok(Some(Ok(address_books))) => {
                        for book in address_books {
                            self.fetch_contact_backend_address_book(backend_id.clone(), book);
                        }
                    }
                }
            }
            ContactJobRequest::Cards {
                backend_id,
                book,
                mut handle,
            } => {
                is_canceled! { handle };
                match handle.chan.try_recv() {
                    Err(_) => { /* canceled */ }
                    Ok(None) => {}
                    Ok(Some(Err(err))) => {
                        self.main_loop_handler
                            .job_executor
                            .set_job_success(job_id, false);
                        self.main_loop_handler
                            .send(ThreadEvent::UIEvent(UIEvent::Notification {
                                title: Some(
                                    format!(
                                        "{account}: {name} {format} address book {book} fetch \
                                         failed",
                                        account = self.name,
                                        name = backend_id.name,
                                        format = backend_id.format,
                                    )
                                    .into(),
                                ),
                                source: None,
                                body: err.to_string().into(),
                                kind: Some(NotificationType::Error(err.kind)),
                            }));
                    }
                    Ok(Some(Ok(b))) => {
                        self.contacts.add_book(backend_id, b);
                        self.main_loop_handler.send(ThreadEvent::UIEvent(
                            UIEvent::AccountStatusChange(self.hash, None),
                        ));
                    }
                }
            }
            ContactJobRequest::Edit {
                backend_id,
                book,
                mut handle,
                create: true,
                card_id: _,
            } => {
                is_canceled! { handle };
                match handle.chan.try_recv() {
                    Err(_) => { /* canceled */ }
                    Ok(None) => {}
                    Ok(Some(Err(err))) => {
                        self.main_loop_handler
                            .job_executor
                            .set_job_success(job_id, false);
                        self.main_loop_handler
                            .send(ThreadEvent::UIEvent(UIEvent::Notification {
                                title: Some(
                                    format!(
                                        "{account}: {name} {format} contact creation failed",
                                        account = self.name,
                                        name = backend_id.name,
                                        format = backend_id.format,
                                    )
                                    .into(),
                                ),
                                source: None,
                                body: err.to_string().into(),
                                kind: Some(NotificationType::Error(err.kind)),
                            }));
                    }
                    Ok(Some(Ok(new_card))) => {
                        self.contacts.books[&(backend_id, book)].add_card(new_card);
                        self.main_loop_handler.send(ThreadEvent::UIEvent(
                            UIEvent::AccountStatusChange(self.hash, None),
                        ));
                    }
                }
            }
            ContactJobRequest::Edit {
                backend_id,
                book,
                mut handle,
                create: false,
                card_id,
            } => {
                is_canceled! { handle };
                match handle.chan.try_recv() {
                    Err(_) => { /* canceled */ }
                    Ok(None) => {}
                    Ok(Some(Err(err))) => {
                        self.main_loop_handler
                            .job_executor
                            .set_job_success(job_id, false);
                        self.main_loop_handler
                            .send(ThreadEvent::UIEvent(UIEvent::Notification {
                                title: Some(
                                    format!(
                                        "{account}: {name} {format} contact edit failed",
                                        account = self.name,
                                        name = backend_id.name,
                                        format = backend_id.format,
                                    )
                                    .into(),
                                ),
                                source: None,
                                body: err.to_string().into(),
                                kind: Some(NotificationType::Error(err.kind)),
                            }));
                    }
                    Ok(Some(Ok(new_card))) => {
                        if self.contacts.books[&(Arc::clone(&backend_id), book.clone())]
                            .card_exists(card_id)
                        {
                            self.contacts.books[&(backend_id, book)]
                                .cards
                                .insert(card_id, new_card);
                        }
                        self.main_loop_handler.send(ThreadEvent::UIEvent(
                            UIEvent::AccountStatusChange(self.hash, None),
                        ));
                    }
                }
            }
            ContactJobRequest::DeleteCard {
                backend_id,
                book,
                mut handle,
                card_id,
            } => {
                is_canceled! { handle };
                match handle.chan.try_recv() {
                    Err(_) => { /* canceled */ }
                    Ok(None) => {}
                    Ok(Some(Err(err))) => {
                        self.main_loop_handler
                            .job_executor
                            .set_job_success(job_id, false);
                        self.main_loop_handler
                            .send(ThreadEvent::UIEvent(UIEvent::Notification {
                                title: Some(
                                    format!(
                                        "{account}: {name} {format} contact deletion failed",
                                        account = self.name,
                                        name = backend_id.name,
                                        format = backend_id.format,
                                    )
                                    .into(),
                                ),
                                source: None,
                                body: err.to_string().into(),
                                kind: Some(NotificationType::Error(err.kind)),
                            }));
                    }
                    Ok(Some(Ok(()))) => {
                        if self.contacts.books[&(Arc::clone(&backend_id), book.clone())]
                            .card_exists(card_id)
                        {
                            self.contacts.books[&(backend_id, book)]
                                .cards
                                .shift_remove(&card_id);
                        }
                        self.main_loop_handler.send(ThreadEvent::UIEvent(
                            UIEvent::AccountStatusChange(self.hash, None),
                        ));
                    }
                }
            }
        }
        true
    }

    /// 在指定后端地址簿中新增联系人。
    pub fn add_card(
        &mut self,
        (backend_id, book): (Arc<ContactBackendID>, AddressBookName),
        card: Card,
    ) -> Result<(JobId, oneshot::Receiver<Result<()>>)> {
        let (sender, receiver) = oneshot::channel();
        let backend = self.contact_backends[&*backend_id].as_ref();
        let mut backend = backend.lock().unwrap();
        let card_id = card.id;
        let is_async = backend.capabilities().is_async;
        let fut = backend.add(&book, card)?;
        let handle = self.main_loop_handler.job_executor.spawn(
            "create contact".into(),
            async move {
                match fut.await {
                    ok @ Ok(_) => {
                        _ = sender.send(Ok(()));
                        ok
                    }
                    Err(err) => {
                        _ = sender.send(Err(err.clone()));
                        Err(err)
                    }
                }
            },
            IsAsync::from(is_async),
        );
        let job_id = handle.job_id;
        self.active_jobs.insert(
            job_id,
            JobRequest::Contacts(ContactJobRequest::Edit {
                backend_id,
                book,
                handle,
                card_id,
                create: true,
            }),
        );
        self.active_job_instants
            .insert(std::time::Instant::now(), job_id);
        self.main_loop_handler
            .send(ThreadEvent::UIEvent(UIEvent::StatusEvent(
                StatusEvent::NewJob(job_id),
            )));
        Ok((job_id, receiver))
    }

    /// 在指定后端地址簿中编辑联系人。
    pub fn edit_card(
        &mut self,
        (backend_id, book): (Arc<ContactBackendID>, AddressBookName),
        card: Card,
    ) -> Result<(JobId, oneshot::Receiver<Result<()>>)> {
        let (sender, receiver) = oneshot::channel();
        let backend = self.contact_backends[&*backend_id].as_ref();
        let mut backend = backend.lock().unwrap();
        let card_id = card.id;
        let is_async = backend.capabilities().is_async;
        let fut = backend.edit(&book, card)?;
        let handle = self.main_loop_handler.job_executor.spawn(
            "edit contact".into(),
            async move {
                match fut.await {
                    ok @ Ok(_) => {
                        _ = sender.send(Ok(()));
                        ok
                    }
                    Err(err) => {
                        _ = sender.send(Err(err.clone()));
                        Err(err)
                    }
                }
            },
            IsAsync::from(is_async),
        );
        let job_id = handle.job_id;
        self.active_jobs.insert(
            job_id,
            JobRequest::Contacts(ContactJobRequest::Edit {
                backend_id,
                book,
                handle,
                create: false,
                card_id,
            }),
        );
        self.active_job_instants
            .insert(std::time::Instant::now(), job_id);
        self.main_loop_handler
            .send(ThreadEvent::UIEvent(UIEvent::StatusEvent(
                StatusEvent::NewJob(job_id),
            )));
        Ok((job_id, receiver))
    }

    /// 从指定后端地址簿删除联系人。
    pub fn delete_card(
        &mut self,
        (backend_id, book): (Arc<ContactBackendID>, AddressBookName),
        card: Card,
    ) -> Result<(JobId, oneshot::Receiver<Result<()>>)> {
        let (sender, receiver) = oneshot::channel();
        let backend = self.contact_backends[&*backend_id].as_ref();
        let mut backend = backend.lock().unwrap();
        let card_id = card.id;
        let is_async = backend.capabilities().is_async;
        let fut = backend.delete(&book, card)?;
        let handle = self.main_loop_handler.job_executor.spawn(
            "delete contact".into(),
            async move {
                match fut.await {
                    ok @ Ok(_) => {
                        _ = sender.send(Ok(()));
                        ok
                    }
                    Err(err) => {
                        _ = sender.send(Err(err.clone()));
                        Err(err)
                    }
                }
            },
            IsAsync::from(is_async),
        );
        let job_id = handle.job_id;
        self.active_jobs.insert(
            job_id,
            JobRequest::Contacts(ContactJobRequest::DeleteCard {
                backend_id,
                book,
                handle,
                card_id,
            }),
        );
        self.active_job_instants
            .insert(std::time::Instant::now(), job_id);
        self.main_loop_handler
            .send(ThreadEvent::UIEvent(UIEvent::StatusEvent(
                StatusEvent::NewJob(job_id),
            )));
        Ok((job_id, receiver))
    }
}

/// 配置后端种类 → fork 所用后端 `format` 标识（与上游字面量一致）。
fn contact_backend_format(conf: &melib::conf::ContactBackendConf) -> &'static str {
    use melib::conf::ContactBackendConf;
    match conf {
        ContactBackendConf::NotmuchAddress(_) => "notmuch_address_book_query",
        ContactBackendConf::MuttAlias(_) => "mutt_alias_file",
        ContactBackendConf::VCard(_) => "vcard",
        ContactBackendConf::CardDAV(_) => "carddav",
    }
}

/// 读取 fork 旧单书 JSON 格式的 contacts 文件（整份 [`Contacts`] 序列化形状，
/// 门面反序列化会把 `cards` 落进默认书）。
fn read_contacts_file(file: &Path) -> Result<Contacts> {
    let reader = std::io::BufReader::new(std::fs::File::open(file).chain_err_related_path(file)?);
    let data: Contacts = serde_json::from_reader(reader).chain_err_related_path(file)?;
    Ok(data)
}

/// 读取默认本地书并保留 `external_resource` 过滤（fork 语义：外部资源卡片
/// 不进可写的默认书）。
fn read_default_book(file: &Path) -> Result<IndexMap<CardId, Card>> {
    let contacts = read_contacts_file(file)?;
    Ok(contacts
        .cards()
        .iter()
        .filter(|(_, c)| !c.external_resource())
        .map(|(id, c)| (*id, c.clone()))
        .collect())
}

/// 待处理的通讯录作业。
pub enum ContactJobRequest {
    Initialize {
        backend_id: Arc<ContactBackendID>,
        handle: JoinHandle<Result<Box<dyn melib::contacts::backend::ContactBackend>>>,
    },
    AddressBooks {
        backend_id: Arc<ContactBackendID>,
        handle: JoinHandle<Result<Vec<AddressBookName>>>,
    },
    Cards {
        backend_id: Arc<ContactBackendID>,
        book: AddressBookName,
        handle: JoinHandle<Result<AddressBook>>,
    },
    Edit {
        backend_id: Arc<ContactBackendID>,
        book: AddressBookName,
        card_id: CardId,
        create: bool,
        handle: JoinHandle<Result<Card>>,
    },
    DeleteCard {
        backend_id: Arc<ContactBackendID>,
        book: AddressBookName,
        card_id: CardId,
        handle: JoinHandle<Result<()>>,
    },
}

impl std::fmt::Debug for ContactJobRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Initialize { backend_id, .. } => f
                .debug_struct("ContactJobRequest::Initialize")
                .field("backend_id", backend_id)
                .finish_non_exhaustive(),
            Self::AddressBooks { backend_id, .. } => f
                .debug_struct("ContactJobRequest::AddressBooks")
                .field("backend_id", backend_id)
                .finish_non_exhaustive(),
            Self::Cards { backend_id, .. } => f
                .debug_struct("ContactJobRequest::Cards")
                .field("backend_id", backend_id)
                .finish_non_exhaustive(),
            Self::Edit {
                backend_id,
                book,
                create: true,
                ..
            } => f
                .debug_struct("ContactJobRequest::Edit(create)")
                .field("backend_id", backend_id)
                .field("book", book)
                .finish_non_exhaustive(),
            Self::Edit {
                backend_id,
                book,
                create: false,
                card_id,
                ..
            } => f
                .debug_struct("ContactJobRequest::Edit")
                .field("backend_id", backend_id)
                .field("book", book)
                .field("card_id", card_id)
                .finish_non_exhaustive(),
            Self::DeleteCard {
                backend_id,
                book,
                card_id,
                ..
            } => f
                .debug_struct("ContactJobRequest::DeleteCard")
                .field("backend_id", backend_id)
                .field("book", book)
                .field("card_id", card_id)
                .finish_non_exhaustive(),
        }
    }
}

impl std::fmt::Display for ContactJobRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Initialize { .. } => write!(f, "Initialize"),
            Self::AddressBooks { .. } => write!(f, "Get address books"),
            Self::Cards { .. } => write!(f, "Get cards"),
            Self::Edit { create: true, .. } => write!(f, "Add card"),
            Self::Edit { .. } => write!(f, "Edit card"),
            Self::DeleteCard { .. } => write!(f, "Delete card"),
        }
    }
}

impl ContactJobRequest {
    pub fn cancel(&self) -> Option<StatusEvent> {
        match self {
            Self::Initialize { handle, .. } => handle.cancel(),
            Self::AddressBooks { handle, .. } => handle.cancel(),
            Self::Cards { handle, .. } => handle.cancel(),
            Self::Edit { handle, .. } => handle.cancel(),
            Self::DeleteCard { handle, .. } => handle.cancel(),
        }
    }
}

/// 默认本地通讯录后端：单书、可写，承载 fork 旧 JSON 持久化语义。
#[derive(Debug)]
struct DefaultContacts {
    cards: IndexMap<CardId, Card>,
}

impl DefaultContacts {
    /// 默认书名称，与门面 [`melib::contacts::DEFAULT_BOOK_NAME`] 一致。
    const DEFAULT_ADDRESS_BOOK: &'static str = melib::contacts::DEFAULT_BOOK_NAME;
    /// 默认后端名称，与门面 [`melib::contacts::DEFAULT_BACKEND_NAME`] 一致。
    const NAME: &'static str = melib::contacts::DEFAULT_BACKEND_NAME;
    /// 默认后端格式，与门面 [`melib::contacts::DEFAULT_BACKEND_FORMAT`] 一致。
    const FORMAT: &'static str = melib::contacts::DEFAULT_BACKEND_FORMAT;

    fn address_books(&self) -> Vec<AddressBookName> {
        vec![AddressBookName(
            Self::DEFAULT_ADDRESS_BOOK.to_string().into(),
        )]
    }

    fn fetch_default_book(&self) -> AddressBook {
        AddressBook {
            name: Self::DEFAULT_ADDRESS_BOOK.into(),
            read_only: false,
            cards: self.cards.clone(),
        }
    }
}

impl ContactBackend for DefaultContacts {
    fn capabilities(&mut self) -> ContactBackendCapabilities {
        ContactBackendCapabilities {
            is_async: true,
            is_remote: false,
            ..ContactBackendCapabilities::default()
        }
    }

    fn address_books(&mut self) -> ResultFuture<Vec<AddressBookName>> {
        let address_books = Self::address_books(self);
        Ok(Box::pin(async { Ok(address_books) }))
    }

    fn fetch_book(&mut self, address_book: &AddressBookName) -> ResultFuture<AddressBook> {
        if address_book.0.as_ref() != Self::DEFAULT_ADDRESS_BOOK {
            return Err(Error::new("").set_kind(ErrorKind::ValueError));
        }
        let book = Self::fetch_default_book(self);

        Ok(Box::pin(async move { Ok(book) }))
    }

    fn search(
        &self,
        _term: &str,
        _address_book: Option<&AddressBookName>,
    ) -> ResultFuture<Vec<Card>> {
        Err(Error::new("").set_kind(ErrorKind::NotSupported))
    }

    fn add(&mut self, address_book: &AddressBookName, new_card: Card) -> ResultFuture<Card> {
        if address_book.0.as_ref() != Self::DEFAULT_ADDRESS_BOOK {
            return Err(Error::new("").set_kind(ErrorKind::ValueError));
        }
        self.cards.insert(new_card.id, new_card.clone());
        Ok(Box::pin(async move { Ok(new_card) }))
    }

    fn edit(&mut self, address_book: &AddressBookName, card: Card) -> ResultFuture<Card> {
        self.add(address_book, card)
    }

    fn delete(&mut self, address_book: &AddressBookName, card: Card) -> ResultFuture<()> {
        if address_book.0.as_ref() != Self::DEFAULT_ADDRESS_BOOK {
            return Err(Error::new("").set_kind(ErrorKind::ValueError));
        }
        let ret = if self.cards.shift_remove(&card.id).is_some() {
            Ok(())
        } else {
            Err(Error::new("").set_kind(ErrorKind::NotSupported))
        };

        Ok(Box::pin(async move { ret }))
    }
}

#[cfg(test)]
mod tests {
    use melib::{
        conf::{ContactBackendConf, Secret},
        smol,
        utils::webdav::{WebDAVAuthentication, WebDAVServerConf},
    };

    use super::*;

    /// 配置后端种类到 `format` 标识的分派必须与上游字面量逐一对应。
    #[test]
    fn test_contact_backend_format_dispatch() {
        assert_eq!(
            contact_backend_format(&ContactBackendConf::NotmuchAddress("tag:inbox".into())),
            "notmuch_address_book_query"
        );
        assert_eq!(
            contact_backend_format(&ContactBackendConf::MuttAlias("~/.aliases".into())),
            "mutt_alias_file"
        );
        assert_eq!(
            contact_backend_format(&ContactBackendConf::VCard("~/.vcards".into())),
            "vcard"
        );
        // 占位符配置：只验证分派，不发起任何网络连接。
        let carddav = ContactBackendConf::CardDAV(WebDAVServerConf {
            url: "https://carddav.example.invalid/".parse().unwrap(),
            username: Secret::Value("placeholder".into()),
            password: Secret::Value("placeholder".into()),
            auth: WebDAVAuthentication::Basic,
            danger_accept_invalid_certs: false,
            timeout: None,
        });
        assert_eq!(contact_backend_format(&carddav), "carddav");
    }

    /// 默认本地后端离线 CRUD：新增/编辑/删除/取书/列书。
    #[test]
    fn test_default_contacts_offline_crud() {
        let mut backend = DefaultContacts {
            cards: IndexMap::default(),
        };
        let book_name = AddressBookName(DefaultContacts::DEFAULT_ADDRESS_BOOK.into());
        let books = smol::block_on(ContactBackend::address_books(&mut backend).unwrap()).unwrap();
        assert_eq!(books, vec![book_name.clone()]);

        let mut card = Card::new();
        card.email = "placeholder@example.invalid".into();
        let card_id = card.id;
        let added = smol::block_on(backend.add(&book_name, card.clone()).unwrap()).unwrap();
        assert_eq!(added.id, card_id);

        let book = smol::block_on(backend.fetch_book(&book_name).unwrap()).unwrap();
        assert!(book.card_exists(card_id));
        assert_eq!(book.name, book_name);

        let mut edited = card;
        edited.name = "Edited".into();
        smol::block_on(backend.edit(&book_name, edited).unwrap()).unwrap();
        let book = smol::block_on(backend.fetch_book(&book_name).unwrap()).unwrap();
        assert_eq!(book.cards[&card_id].name, "Edited");

        smol::block_on(
            backend
                .delete(&book_name, book.cards[&card_id].clone())
                .unwrap(),
        )
        .unwrap();
        let book = smol::block_on(backend.fetch_book(&book_name).unwrap()).unwrap();
        assert!(!book.card_exists(card_id));

        // 不存在的书必须报 ValueError。
        let other = AddressBookName("other".into());
        assert!(backend.fetch_book(&other).is_err());
    }

    /// 默认后端标识必须与门面默认书识别常量一致。
    #[test]
    fn test_default_contacts_identity_matches_facade() {
        assert_eq!(DefaultContacts::NAME, melib::contacts::DEFAULT_BACKEND_NAME);
        assert_eq!(
            DefaultContacts::FORMAT,
            melib::contacts::DEFAULT_BACKEND_FORMAT
        );
        assert_eq!(
            DefaultContacts::DEFAULT_ADDRESS_BOOK,
            melib::contacts::DEFAULT_BOOK_NAME
        );
    }

    /// fork 旧单书 JSON 持久化格式必须可读回默认书，且 `external_resource`
    /// 卡片被过滤（不回退）。
    #[test]
    fn test_read_default_book_legacy_format_and_filter() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("contacts");

        let mut contacts = Contacts::new("test".to_string());
        let mut local = Card::new();
        local.name = "Legacy".into();
        local.email = "legacy@example.invalid".into();
        let local_id = local.id;
        let mut external = Card::new();
        external.name = "Remote".into();
        external.email = "remote@example.invalid".into();
        external.set_external_resource(true);
        let external_id = external.id;
        contacts.add_card(local);
        contacts.add_card(external);

        // 门面 Serialize 写出的就是生产落盘形状（`{"name","cards"}`）。
        std::fs::write(&path, serde_json::to_string(&contacts).unwrap()).unwrap();

        let read = read_contacts_file(&path).unwrap();
        assert!(read.card_exists(local_id));
        assert!(read.card_exists(external_id));

        let default_book = read_default_book(&path).unwrap();
        assert!(default_book.contains_key(&local_id));
        assert!(
            !default_book.contains_key(&external_id),
            "外部资源卡片不得进可写默认书"
        );
    }
}
