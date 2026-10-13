//
// meli
//
// Copyright 2026  Manos Pitsidianakis
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

//! Account-side wiring of contact backends, semantically ported from
//! upstream meli `db9f7f93` (Load contacts from contact backends) and
//! `14640216` (Load carddav contact backend).
//!
//! Fork deviation: upstream replaced the account's flat `Contacts` store
//! with an aggregating multi-book `Contacts` and moved the on-disk address
//! book into a built-in `DefaultContacts` backend. The fork keeps its flat
//! `Contacts` as the account's unified store (its startup shape is pinned
//! by the CVE regression corpora), so this module only *feeds* it: each
//! configured backend is initialized as a job chain
//! (initialize → address books → address book fetch) and every fetched
//! [`AddressBook`]'s cards are merged into `account.contacts`, making
//! autocomplete and contact search naturally cover remote books. The
//! legacy implicit sources (`vcard_folder`, `mutt_alias_file`,
//! `notmuch_address_book_query`) keep loading at startup; backends
//! configured under the `contacts` map layer on top of them.

use std::{
    future::Future,
    path::Path,
    sync::{Arc, Mutex},
};

use melib::{
    conf::ContactBackendConf,
    contacts::{
        mutt, notmuchcontact, vcard, AddressBookName, ContactBackend, ContactBackendID,
    },
    error::{Error, ErrorKind, Result},
};

use crate::{
    accounts::{Account, JobRequest},
    jobs::{IsAsync, JobId, JoinHandle},
    NotificationType, StatusEvent, ThreadEvent, UIEvent,
};

impl Account {
    /// Spawn the initialization job of a contact backend: `fut` must resolve
    /// to the constructed backend, after which its address books are listed
    /// and fetched (see [`ContactJobRequest`]).
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

    /// Kick off every contact backend of this account: first the mail
    /// backend's own contact backend (currently JMAP), then each backend
    /// configured under the account's `contacts` map.
    pub fn init_contacts(&mut self) {
        {
            let mut backend = self.backend.lock().unwrap();
            let format = self.settings.account().format.clone();
            match backend.contact_backend() {
                Ok(contacts_job) => {
                    drop(backend);
                    self.init_contact_backend(
                        "meli".to_string(),
                        format,
                        async move {
                            match contacts_job.await? {
                                Some(backend) => Ok(backend),
                                None => Err(Error::new(
                                    "Mail backend does not have a contacts account.",
                                )
                                .set_kind(ErrorKind::NotSupported)),
                            }
                        },
                        self.is_async(),
                    );
                }
                Err(err) => {
                    // `NotSupported` is the common "this mail backend has no
                    // contacts" answer; only surface real failures.
                    if !matches!(err.kind, ErrorKind::NotSupported) {
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
        let Some(conf) = self.settings.account().contacts.clone() else {
            return;
        };
        for (name, contact_backend) in conf {
            match contact_backend {
                ContactBackendConf::NotmuchAddress(query) => {
                    self.init_contact_backend(
                        name,
                        "notmuch_address_book_query".to_string(),
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
                        "mutt_alias_file".to_string(),
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
                        "vcard".to_string(),
                        async move {
                            Ok(Box::new(vcard::VCardContacts { path }) as Box<dyn ContactBackend>)
                        },
                        IsAsync::Async,
                    );
                }
                #[cfg(feature = "webdav")]
                ContactBackendConf::CardDAV(server_conf) => {
                    self.init_contact_backend(
                        name,
                        "carddav".to_string(),
                        async move {
                            use melib::{
                                contacts::carddav::{CardDAVConnection, CardDAVContacts},
                                utils::webdav::WebDAVConnection,
                            };
                            let mut connection = WebDAVConnection::new(&server_conf).await?;
                            connection.connect().await?;
                            let connection = CardDAVConnection::new(connection).await?.into();

                            Ok(Box::new(CardDAVContacts { connection }) as Box<dyn ContactBackend>)
                        },
                        IsAsync::Async,
                    );
                }
            }
        }
    }

    /// Register an initialized backend and spawn the job listing its address
    /// books.
    pub fn add_contact_backend(
        &mut self,
        backend_id: Arc<ContactBackendID>,
        mut backend: Box<dyn ContactBackend>,
    ) {
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
                    backend.capabilities().is_async.into(),
                );
                let job_id = handle.job_id;
                self.main_loop_handler
                    .send(ThreadEvent::UIEvent(UIEvent::StatusEvent(
                        StatusEvent::NewJob(job_id),
                    )));
                self.contact_backends
                    .insert(backend_id.clone(), Arc::new(Mutex::new(backend)));
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

    /// Spawn the fetch job of one address book of a backend.
    pub fn fetch_contact_backend_address_book(
        &mut self,
        backend_id: Arc<ContactBackendID>,
        book: AddressBookName,
    ) {
        let backend = self.contact_backends[&backend_id].as_ref();
        let mut backend = backend.lock().unwrap();
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
                    backend.capabilities().is_async.into(),
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

    /// Process a finished contact job; returns `false` if the job id is
    /// unknown.
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
                        // A mail backend without contacts support answering
                        // `Ok(None)`/`NotSupported` is not an error worth a
                        // notification (see `init_contacts`).
                        if !matches!(err.kind, ErrorKind::NotSupported) {
                            self.main_loop_handler
                                .send(ThreadEvent::UIEvent(UIEvent::Notification {
                                    title: Some(
                                        format!(
                                            "{account}: {name} {format} contact initialization \
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
                        // Merge the fetched address book into the account's
                        // flat contacts store, deduplicating by card id like
                        // the startup disk-file load in `Account::new`.
                        // External-resource cards must pass through here:
                        // they are skipped when reloading the persisted file
                        // precisely because their backend re-fetches them.
                        for (id, c) in b.cards {
                            if !self.contacts.card_exists(id) {
                                self.contacts.add_card(c);
                            }
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
}

/// Job chain of one contact backend: initialize the backend, list its
/// address books, fetch each book's cards.
///
/// Fork deviation: upstream also routes contact edits/deletions through
/// per-backend jobs; the fork's contact manager UI keeps writing to the
/// account's flat `Contacts` store directly, so those variants are not
/// ported.
pub enum ContactJobRequest {
    Initialize {
        backend_id: Arc<ContactBackendID>,
        handle: JoinHandle<Result<Box<dyn ContactBackend>>>,
    },
    AddressBooks {
        backend_id: Arc<ContactBackendID>,
        handle: JoinHandle<Result<Vec<AddressBookName>>>,
    },
    Cards {
        backend_id: Arc<ContactBackendID>,
        book: AddressBookName,
        handle: JoinHandle<Result<melib::contacts::AddressBook>>,
    },
}

impl std::fmt::Debug for ContactJobRequest {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Initialize { backend_id, .. } => fmt
                .debug_struct("ContactJobRequest::Initialize")
                .field("backend_id", backend_id)
                .finish_non_exhaustive(),
            Self::AddressBooks { backend_id, .. } => fmt
                .debug_struct("ContactJobRequest::AddressBooks")
                .field("backend_id", backend_id)
                .finish_non_exhaustive(),
            Self::Cards { backend_id, book, .. } => fmt
                .debug_struct("ContactJobRequest::Cards")
                .field("backend_id", backend_id)
                .field("book", book)
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
        }
    }
}

impl ContactJobRequest {
    pub fn cancel(&self) -> Option<StatusEvent> {
        match self {
            Self::Initialize { handle, .. } => handle.cancel(),
            Self::AddressBooks { handle, .. } => handle.cancel(),
            Self::Cards { handle, .. } => handle.cancel(),
        }
    }
}
