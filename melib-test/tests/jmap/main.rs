//
// meli
//
// Copyright 2025 Emmanouil Pitsidianakis <manos@pitsidianak.is>
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

#![cfg(feature = "jmap")]

/// Tests in this target rewrite process-global environment variables
/// (XDG/HOME sandbox setup for the mock-account config), so they must not
/// run concurrently on the test harness's parallel threads. Each test
/// clears and re-establishes its own environment while holding this lock,
/// replacing the per-test process isolation `rusty-fork` used to provide.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn test_jmap_refresh() {
    let _env = env_lock();
    tokio_test::block_on(async {
        tests::run_jmap_refresh();
    });
}

#[test]
fn test_jmap_watch() {
    let _env = env_lock();
    tokio_test::block_on(async {
        tests::run_jmap_watch();
    });
}

#[test]
fn test_jmap_operations() {
    let _env = env_lock();
    tokio_test::block_on(async {
        tests::run_jmap_operations();
    });
}

#[test]
fn test_jmap_contacts() {
    let _env = env_lock();
    tokio_test::block_on(async {
        tests::run_jmap_contacts();
    });
}

pub mod server {
    use std::{
        net::{SocketAddr, TcpListener},
        sync::{Arc, Mutex},
    };

    use futures::{
        channel::mpsc::UnboundedReceiver,
        future::{self, Either},
        io::{AsyncBufReadExt, AsyncWriteExt},
        StreamExt,
    };
    use melib::{
        backends::prelude::*,
        email::address::MessageID,
        jmap::{argument::*, identity, methods::*, objects::*, session::Session, *},
        smol::Async,
        Mail,
    };
    use serde::Serialize;
    use serde_json::Value;

    #[derive(Debug, serde_derive::Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct ClientRequest {
        pub using: Vec<String>,
        pub method_calls: Vec<[Value; 3]>,
    }

    #[derive(Clone, Debug, serde_derive::Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct ClientMethodResponse {
        #[serde(serialize_with = "method_response_ser")]
        pub method_responses: IndexMap<String, Value>,
        #[serde(default)]
        pub created_ids: IndexMap<Id<String>, Id<String>>,
        pub session_state: State<Session>,
    }

    fn method_response_ser<S>(
        value: &IndexMap<String, Value>,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        let values = value.values().cloned().collect::<Vec<Value>>();
        values.serialize(serializer)
    }

    #[derive(Debug)]
    pub enum StateChange<OBJ: objects::Object> {
        Initial,
        Created(Id<OBJ>),
        Updated(Id<OBJ>),
        Destroyed(Id<OBJ>),
    }
    /// [`Flag`] 位到 JMAP 关键字字符串的映射，供测试服务器关键字表记账。
    fn flag_keyword(flag: Flag) -> &'static str {
        match flag {
            Flag::DRAFT => "$draft",
            Flag::FLAGGED => "$flagged",
            Flag::SEEN => "$seen",
            Flag::REPLIED => "$answered",
            Flag::TRASHED => "$junk",
            Flag::PASSED => "$passed",
            _ => unimplemented!("Flag {flag:?} 没有 JMAP 关键字映射"),
        }
    }

    /// Server state with only one mailbox (INBOX).
    pub struct ServerState {
        pub envelopes: IndexMap<Id<email::EmailObject>, Mail>,
        pub identities: IndexMap<Id<identity::Identity>, identity::Identity>,
        pub mailboxes: IndexMap<Id<mailbox::MailboxObject>, mailbox::MailboxObject>,
        pub identity_state: State<identity::Identity>,
        pub email_state: State<email::EmailObject>,
        pub mailbox_state: State<mailbox::MailboxObject>,
        pub email_state_changes:
            IndexMap<State<email::EmailObject>, StateChange<email::EmailObject>>,
        /// Blobs uploaded through the `uploadUrl` endpoint, keyed by blob id.
        pub uploaded_blobs: IndexMap<Id<objects::BlobObject>, Vec<u8>>,
        /// Per-email JMAP keywords (`String[Boolean]`), including custom tags
        /// that have no [`Mail`] flag-bit representation.
        pub keywords: IndexMap<Id<email::EmailObject>, IndexMap<String, bool>>,
        /// Error response types the server has replied with, for assertions.
        pub error_responses: Vec<String>,
        pub session: Session,
    }

    /// Helper enum to refer to an e-mail internally
    pub enum EmailObjRef<'a> {
        Id(&'a Id<email::EmailObject>),
        MessageID(&'a MessageID),
    }

    impl ServerState {
        pub fn new(addr: &SocketAddr) -> Self {
            let mut session = serde_json::from_str::<Session>(&format!(
                r#"{{
     "capabilities": {{
       "urn:ietf:params:jmap:core": {{
         "maxSizeUpload": 50000000,
         "maxConcurrentUpload": 8,
         "maxSizeRequest": 10000000,
         "maxConcurrentRequests": 8,
         "maxCallsInRequest": 32,
         "maxObjectsInGet": 256,
         "maxObjectsInSet": 128,
         "collationAlgorithms": [
           "i;ascii-numeric",
           "i;ascii-casemap",
           "i;unicode-casemap"
         ]
       }},
       "urn:ietf:params:jmap:mail": {{}},
       "urn:ietf:params:jmap:contacts": {{}}
     }},
     "accounts": {{
       "A13824": {{
         "name": "user@example.com",
         "isPersonal": true,
         "isReadOnly": false,
         "accountCapabilities": {{
           "urn:ietf:params:jmap:mail": {{
             "maxMailboxesPerEmail": null,
             "maxMailboxDepth": 10
           }},
           "urn:ietf:params:jmap:contacts": {{}}
         }}
       }}
     }},
     "primaryAccounts": {{
       "urn:ietf:params:jmap:mail": "A13824",
       "urn:ietf:params:jmap:contacts": "A13824"
     }},
     "username": "john@example.com",
     "apiUrl": "http://{host}:{port}/api/",
     "downloadUrl": "http://{host}:{port}/download/{{accountId}}/{{blobId}}/{{name}}?accept={{type}}",
     "uploadUrl": "http://{host}:{port}/upload/{{accountId}}/",
     "eventSourceUrl": "http://{host}:{port}/eventsource/?types={{types}}&closeafter={{closeafter}}&ping={{ping}}",
     "state": "75128aab4b1b"
}}"#,
                host = addr.ip(),
                port = addr.port()
            ))
            .unwrap();
            session.state = State::new_random();
            let email_state = State::new_random();
            let email_state_changes =
                indexmap::indexmap! {email_state.clone() => StateChange::Initial };
            Self {
                envelopes: indexmap::indexmap! {},
                keywords: indexmap::indexmap! {},
                identities: indexmap::indexmap! {},
                mailboxes: {
                    let inbox_id = Id::new_random();
                    let inbox = mailbox::MailboxObject {
                        id: inbox_id.clone(),
                        is_subscribed: true,
                        my_rights: Default::default(),
                        name: "Inbox".to_string(),
                        parent_id: None,
                        role: Some("inbox".to_string()),
                        sort_order: 0,
                        total_emails: 0,
                        total_threads: 0,
                        unread_emails: 0,
                        unread_threads: 0,
                    };
                    indexmap::indexmap! {
                        inbox_id => inbox
                    }
                },
                identity_state: State::new_random(),
                email_state,
                mailbox_state: State::new_random(),
                email_state_changes,
                uploaded_blobs: indexmap::indexmap! {},
                error_responses: vec![],
                session,
            }
        }

        pub fn api(&mut self, request: &str) -> ClientMethodResponse {
            let request: ClientRequest = serde_json::from_str(request).unwrap();

            let mut responses = ClientMethodResponse {
                method_responses: indexmap::indexmap! {},
                created_ids: indexmap::indexmap! {},
                session_state: self.session.state.clone(),
            };

            #[derive(Clone, Copy, Debug)]
            enum MethodCallType {
                Get,
                Changes,
                Set,
                Copy,
                Query,
                QueryChanges,
                Import,
            }

            fn parse_type(value: &str) -> (String, MethodCallType) {
                let mut iter = value.split('/');
                let name = iter.next().unwrap().to_string();
                let r#type = match iter.next().unwrap() {
                    "get" => MethodCallType::Get,
                    "changes" => MethodCallType::Changes,
                    "set" => MethodCallType::Set,
                    "copy" => MethodCallType::Copy,
                    "query" => MethodCallType::Query,
                    "queryChanges" => MethodCallType::QueryChanges,
                    "import" => MethodCallType::Import,
                    other => panic!("Unexpected method call type: {other}"),
                };
                (name, r#type)
            }

            eprintln!("Received API call request {request:?}");
            for [r#type, body, id] in request.method_calls {
                let r#type: String = serde_json::value::from_value(r#type).unwrap();
                let id: String = serde_json::value::from_value(id).unwrap();
                let (object_type_name, method_type) = parse_type(&r#type);
                eprintln!(
                    "Processing method call {type}: object_type_name = {object_type_name:?}, \
                     method_type ={method_type:?}, id = {id:?}"
                );
                match method_type {
                    MethodCallType::Get => {
                        match object_type_name.as_str() {
                            "Identity" => {
                                let get: Get<identity::Identity> =
                                    serde_json::value::from_value(body).unwrap();
                                eprintln!("Parsed Identity/get object {get:?}");
                                let response: GetResponse<identity::Identity> = GetResponse {
                                    account_id: get.account_id,
                                    state: self.identity_state.clone(),
                                    list: self.identities.values().cloned().collect(),
                                    not_found: vec![],
                                };
                                eprintln!("Sending {id} {type} response: {response:?}");
                                responses
                                    .method_responses
                                    .insert(id.clone(), serde_json::json! {[r#type, response, id]});
                            }
                            "Mailbox" => {
                                let get: Get<mailbox::MailboxObject> =
                                    serde_json::value::from_value(body).unwrap();
                                eprintln!("Parsed Mailbox/get object {get:?}");
                                let response: GetResponse<mailbox::MailboxObject> = GetResponse {
                                    account_id: get.account_id,
                                    state: self.mailbox_state.clone(),
                                    list: self.mailboxes.values().cloned().collect(),
                                    not_found: vec![],
                                };
                                eprintln!("Sending {id} {type} response: {response:?}");
                                responses
                                    .method_responses
                                    .insert(id.clone(), serde_json::json! {[r#type, response, id]});
                            }
                            "Email" => {
                                let get: Get<email::EmailObject> =
                                    serde_json::value::from_value(body).unwrap();
                                eprintln!("Parsed Email/get object {get:?}");
                                let mut not_found = vec![];
                                eprintln!("get.ids = {:?}", get.ids);
                                let mut list = match get.ids {
                                    None => {
                                        // Return all.
                                        self.envelopes
                                            .iter()
                                            .map(|(k, v)| (k, v.into()))
                                            .map(|(id, mut email): (&Id<_>, email::EmailObject)| {
                                                email.id = id.clone();
                                                email.mailbox_ids.insert(
                                                    self.mailboxes.keys().next().unwrap().clone(),
                                                    true,
                                                );
                                                for (kw, on) in &self.keywords[id] {
                                                    if *on {
                                                        email.keywords.insert(kw.clone(), true);
                                                    }
                                                }

                                                email
                                            })
                                            .collect()
                                    }
                                    Some(Argument::Value(email_ids)) => {
                                        let mut list = Vec::with_capacity(email_ids.len());
                                        for email_id in email_ids {
                                            if let Some(mail) = self.envelopes.get(&email_id) {
                                                let mut email_obj: email::EmailObject = mail.into();
                                                email_obj.id = email_id.clone();
                                                email_obj.mailbox_ids.insert(
                                                    self.mailboxes.keys().next().unwrap().clone(),
                                                    true,
                                                );
                                                for (kw, on) in &self.keywords[&email_id] {
                                                    if *on {
                                                        email_obj.keywords.insert(kw.clone(), true);
                                                    }
                                                }
                                                list.push(email_obj);
                                            } else {
                                                not_found.push(email_id);
                                            }
                                        }
                                        list
                                    }
                                    Some(Argument::ResultReference {
                                        result_of,
                                        name,
                                        path,
                                    }) => {
                                        // [ref:TODO]: `path` is a JSON pointer, so we should parse it
                                        // as such. Example values: "/created", "/list/*/threadId"
                                        //
                                        // [ref:TODO]: if result reference evaluation fails, respond
                                        // with `invalidResultReference`:
                                        //
                                        // > If any result reference fails to resolve, the whole method
                                        // > MUST be rejected with an "invalidResultReference" error.
                                        // > If an arguments object contains the same argument name in
                                        // > normal and referenced form (e.g., "foo" and "#foo"), the
                                        // > method MUST return an "invalidArguments" error.
                                        let Some(prev_response) =
                                            responses.method_responses.get(&result_of)
                                        else {
                                            eprintln!(
                                                "{id} {type}: resultOf={result_of:?} not found, \
                                                 have: {method_responses:?}",
                                                method_responses = responses.method_responses
                                            );
                                            self.error_responses
                                                .push("invalidResultReference".to_string());
                                            responses.method_responses.insert(
                                                id.clone(),
                                                serde_json::json! {["error",{"type":"invalidResultReference"},id]},
                                            );
                                            continue;
                                        };
                                        if prev_response[0].as_str() == Some("error") {
                                            eprintln!(
                                                "{id} {type}: resultOf={result_of:?} was error: \
                                                 {prev_response:?}",
                                                prev_response = prev_response
                                            );
                                            self.error_responses
                                                .push("invalidResultReference".to_string());
                                            responses.method_responses.insert(
                                                id.clone(),
                                                serde_json::json! {["error",{"type":"invalidResultReference"},id]},
                                            );
                                            continue;
                                        }
                                        assert_eq!(
                                            prev_response[0].as_str(),
                                            Some(name.as_str()),
                                            "{prev_response:?}"
                                        );
                                        prev_response[1][path.trim_matches('/')]
                                            .as_array()
                                            .unwrap()
                                            .iter()
                                            .filter_map(|id| {
                                                let email_id = serde_json::from_value::<
                                                    Id<email::EmailObject>,
                                                >(
                                                    id.clone()
                                                )
                                                .unwrap();
                                                if let Some(mail) = self.envelopes.get(&email_id) {
                                                    let mut email_obj: email::EmailObject =
                                                        mail.into();
                                                    email_obj.id = email_id.clone();
                                                    email_obj.mailbox_ids.insert(
                                                        self.mailboxes
                                                            .keys()
                                                            .next()
                                                            .unwrap()
                                                            .clone(),
                                                        true,
                                                    );
                                                    for (kw, on) in &self.keywords[&email_id] {
                                                        if *on {
                                                            email_obj
                                                                .keywords
                                                                .insert(kw.clone(), true);
                                                        }
                                                    }
                                                    Some(email_obj)
                                                } else {
                                                    not_found.push(email_id);
                                                    None
                                                }
                                            })
                                            .collect::<Vec<email::EmailObject>>()
                                    }
                                };
                                for email_obj in list.iter_mut() {
                                    if let Some(keywords) = self.keywords.get(&email_obj.id) {
                                        email_obj.keywords = keywords.clone();
                                    }
                                }
                                let response: GetResponse<email::EmailObject> = GetResponse {
                                    account_id: get.account_id,
                                    state: self.email_state.clone(),
                                    list,
                                    not_found,
                                };
                                eprintln!("Sending {id} {type} response: {response:?}");
                                responses
                                    .method_responses
                                    .insert(id.clone(), serde_json::json! {[r#type, response, id]});
                            }
                            other => panic!("other type name {other}"),
                        }
                    }
                    MethodCallType::Changes => match object_type_name.as_str() {
                        "Email" => {
                            let changes: Changes<email::EmailObject> =
                                serde_json::value::from_value(body).unwrap();
                            eprintln!("Parsed Email/changes object {changes:?}");
                            let mut created = IndexSet::new();
                            let mut updated = IndexSet::new();
                            let mut destroyed = IndexSet::new();

                            if let Some(pos) = self
                                .email_state_changes
                                .keys()
                                .position(|k| *k == changes.since_state)
                            {
                                for (_, change) in &self.email_state_changes[pos..] {
                                    match change {
                                        StateChange::Initial => {}
                                        StateChange::Created(id) => {
                                            _ = created.insert(id.clone());
                                        }
                                        StateChange::Updated(id) => {
                                            if !created.contains(id) {
                                                _ = updated.insert(id.clone());
                                            }
                                        }
                                        StateChange::Destroyed(id) => {
                                            created.shift_remove(id);
                                            updated.shift_remove(id);
                                            destroyed.insert(id.clone());
                                        }
                                    }
                                }
                            } else if changes.since_state == self.email_state {
                                // No changes
                            } else {
                                eprintln!(
                                    "Email/changes since_state={since_state:?} not found, have: \
                                     {email_state_changes:?}",
                                    since_state = changes.since_state,
                                    email_state_changes = self.email_state_changes
                                );
                                self.error_responses
                                    .push("cannotCalculateChanges".to_string());
                                responses.method_responses.insert(
                                    id.clone(),
                                    serde_json::json! {["error",{"type":"cannotCalculateChanges"},id]},
                                );
                                continue;
                            }
                            let response: ChangesResponse<email::EmailObject> = ChangesResponse {
                                account_id: changes.account_id,
                                old_state: changes.since_state.clone(),
                                new_state: self.email_state.clone(),
                                has_more_changes: false,
                                created: created.into_iter().collect(),
                                updated: updated.into_iter().collect(),
                                destroyed: destroyed.into_iter().collect(),
                                _ph: std::marker::PhantomData,
                            };
                            eprintln!("Sending {id} {type} response: {response:?}");
                            responses
                                .method_responses
                                .insert(id.clone(), serde_json::json! {[r#type, response, id]});
                        }
                        other => panic!("other type name {other}"),
                    },
                    MethodCallType::Set => match object_type_name.as_str() {
                        "Identity" => {
                            let set = serde_json::value::from_value(body).unwrap();
                            eprintln!("Parsed Identity/set object {set:?}");
                            let Set {
                                account_id,
                                if_in_state,
                                create,
                                update,
                                destroy,
                            }: Set<identity::Identity> = set;
                            let mut created = indexmap::indexmap! {};
                            let updated = indexmap::indexmap! {};
                            let destroyed = vec![];
                            let old_state = self.identity_state.clone();
                            let mut new_state = self.identity_state.clone();
                            if let Some(ref if_in_state) = if_in_state {
                                if *if_in_state != self.identity_state {
                                    unimplemented!();
                                }
                            }
                            for (id, mut obj) in create.unwrap_or_default() {
                                let argument::Argument::Value(id) = id else {
                                    panic!("unimplemented");
                                };
                                obj.id = id.clone();
                                self.identities.insert(id.clone(), obj);
                                created.insert(id.clone(), serde_json::json! {{"id": id }});
                                new_state = std::mem::replace(
                                    &mut self.identity_state,
                                    State::new_random(),
                                );
                            }
                            assert_eq!(update.unwrap_or_default().len(), 0);
                            assert_eq!(destroy.unwrap_or_default().len(), 0);
                            // for _ in update.unwrap_or_default() {
                            //     unimplemented!();
                            // }
                            // for _ in destroy.unwrap_or_default() {
                            //     unimplemented!();
                            // }
                            let old_state = if new_state != old_state {
                                Some(old_state)
                            } else {
                                None
                            };
                            let response: SetResponse<identity::Identity> = SetResponse {
                                account_id,
                                old_state,
                                new_state,
                                created: if created.is_empty() {
                                    None
                                } else {
                                    Some(created)
                                },
                                updated: if updated.is_empty() {
                                    None
                                } else {
                                    Some(updated)
                                },
                                destroyed: if destroyed.is_empty() {
                                    None
                                } else {
                                    Some(destroyed)
                                },
                                not_created: None,
                                not_updated: None,
                                not_destroyed: None,
                            };
                            eprintln!("Sending {id} {type} response: {response:?}");
                            responses
                                .method_responses
                                .insert(id.clone(), serde_json::json! {[r#type, response, id]});
                        }
                        "Mailbox" => {
                            let set = serde_json::value::from_value(body).unwrap();
                            eprintln!("Parsed Mailbox/set object {set:?}");
                            let Set {
                                account_id,
                                if_in_state,
                                create,
                                update,
                                destroy,
                            }: Set<mailbox::MailboxObject> = set;
                            let mut created = indexmap::indexmap! {};
                            let updated = indexmap::indexmap! {};
                            let destroyed = vec![];
                            let mut update_state = false;
                            if let Some(ref if_in_state) = if_in_state {
                                if *if_in_state != self.mailbox_state {
                                    unimplemented!();
                                }
                            }
                            for (id, mut obj) in create.unwrap_or_default() {
                                update_state = true;
                                let argument::Argument::Value(id) = id else {
                                    unreachable!();
                                };
                                obj.id = id.clone();
                                self.mailboxes.insert(id.clone(), obj);
                                created.insert(id.clone(), serde_json::json! {{"id": id }});
                            }
                            assert_eq!(update.unwrap_or_default().len(), 0);
                            assert_eq!(destroy.unwrap_or_default().len(), 0);
                            // for _ in update.unwrap_or_default() {
                            //     unimplemented!();
                            // }
                            // for _ in destroy.unwrap_or_default() {
                            //     unimplemented!();
                            // }
                            let old_state = if update_state {
                                Some(std::mem::replace(
                                    &mut self.mailbox_state,
                                    State::new_random(),
                                ))
                            } else {
                                None
                            };
                            let new_state = self.mailbox_state.clone();
                            let response: SetResponse<mailbox::MailboxObject> = SetResponse {
                                account_id,
                                old_state,
                                new_state,
                                created: if created.is_empty() {
                                    None
                                } else {
                                    Some(created)
                                },
                                updated: if updated.is_empty() {
                                    None
                                } else {
                                    Some(updated)
                                },
                                destroyed: if destroyed.is_empty() {
                                    None
                                } else {
                                    Some(destroyed)
                                },
                                not_created: None,
                                not_updated: None,
                                not_destroyed: None,
                            };
                            eprintln!("Sending {id} {type} response: {response:?}");
                            responses
                                .method_responses
                                .insert(id.clone(), serde_json::json! {[r#type, response, id]});
                        }
                        "Email" => {
                            let set = serde_json::value::from_value(body).unwrap();
                            eprintln!("Parsed Email/set object {set:?}");
                            let Set {
                                account_id,
                                if_in_state,
                                create,
                                update,
                                destroy,
                            }: Set<email::EmailObject> = set;
                            let created = indexmap::indexmap! {};
                            let mut updated = indexmap::indexmap! {};
                            let destroyed = vec![];
                            if let Some(ref if_in_state) = if_in_state {
                                if *if_in_state != self.email_state {
                                    panic!(
                                        "if_in_state == {if_in_state} while current state is \
                                         {:?}",
                                        self.email_state
                                    );
                                }
                            }
                            if create.is_some() {
                                panic!("unimplemented");
                            }
                            let old_state = self.email_state.clone();
                            let mut new_state = self.email_state.clone();
                            tracing::debug!("updates: {update:?}");
                            for (id, obj) in update.unwrap_or_default() {
                                let Argument::Value(id) = id else {
                                    panic!("unimplemented");
                                };
                                let update: IndexMap<String, Value> =
                                    serde_json::value::from_value(obj).unwrap();
                                for (u, v) in update {
                                    let Some(keyword) = u.strip_prefix("keywords/") else {
                                        panic!("unimplemented update type: {u:?}");
                                    };
                                    let set: bool = serde_json::from_value(v).expect("bool");
                                    let flag_op = match keyword {
                                        "$draft" => {
                                            if set {
                                                FlagOp::Set(Flag::DRAFT)
                                            } else {
                                                FlagOp::UnSet(Flag::DRAFT)
                                            }
                                        }
                                        "$flagged" => {
                                            if set {
                                                FlagOp::Set(Flag::FLAGGED)
                                            } else {
                                                FlagOp::UnSet(Flag::FLAGGED)
                                            }
                                        }
                                        "$seen" => {
                                            if set {
                                                FlagOp::Set(Flag::SEEN)
                                            } else {
                                                FlagOp::UnSet(Flag::SEEN)
                                            }
                                        }
                                        "$answered" => {
                                            if set {
                                                FlagOp::Set(Flag::REPLIED)
                                            } else {
                                                FlagOp::UnSet(Flag::REPLIED)
                                            }
                                        }
                                        "$junk" => {
                                            if set {
                                                FlagOp::Set(Flag::TRASHED)
                                            } else {
                                                FlagOp::UnSet(Flag::TRASHED)
                                            }
                                        }
                                        "$passed" => {
                                            if set {
                                                FlagOp::Set(Flag::PASSED)
                                            } else {
                                                FlagOp::UnSet(Flag::PASSED)
                                            }
                                        }
                                        tag => {
                                            if set {
                                                FlagOp::SetTag(tag.to_string())
                                            } else {
                                                FlagOp::UnSetTag(tag.to_string())
                                            }
                                        }
                                    };
                                    new_state = self.set_flags(EmailObjRef::Id(&id), flag_op);
                                }
                                updated.insert(id.clone(), None);
                            }
                            assert_eq!(destroy.unwrap_or_default().len(), 0);
                            let old_state = if new_state != old_state {
                                Some(old_state)
                            } else {
                                None
                            };
                            let response: SetResponse<email::EmailObject> = SetResponse {
                                account_id,
                                old_state,
                                new_state,
                                created: if created.is_empty() {
                                    None
                                } else {
                                    Some(created)
                                },
                                updated: if updated.is_empty() {
                                    None
                                } else {
                                    Some(updated)
                                },
                                destroyed: if destroyed.is_empty() {
                                    None
                                } else {
                                    Some(destroyed)
                                },
                                not_created: None,
                                not_updated: None,
                                not_destroyed: None,
                            };
                            eprintln!("Sending {id} {type} response: {response:?}");
                            responses
                                .method_responses
                                .insert(id.clone(), serde_json::json! {[r#type, response, id]});
                        }
                        other => panic!("other type name {other}"),
                    },
                    MethodCallType::Copy => unimplemented!(),
                    MethodCallType::Query => match object_type_name.as_str() {
                        "Email" => {
                            let query: email::EmailQuery =
                                serde_json::value::from_value(body).unwrap();
                            eprintln!("Parsed Email/query object {query:?}");
                            let position = query.query_call.position;
                            let position_u = usize::try_from(position).unwrap_or_default();
                            let response: QueryResponse<email::EmailObject> = QueryResponse {
                                account_id: query.query_call.account_id,
                                query_state: String::new(),
                                can_calculate_changes: false,
                                position,
                                ids: self.envelopes.keys().skip(position_u).cloned().collect(),
                                total: if query.query_call.calculate_total {
                                    Some(
                                        u64::try_from(self.envelopes.len())
                                            .unwrap_or_default()
                                            .saturating_sub(position),
                                    )
                                } else {
                                    None
                                },
                                limit: None,
                                _ph: std::marker::PhantomData,
                            };
                            eprintln!("Sending {id} {type} response: {response:?}");
                            responses
                                .method_responses
                                .insert(id.clone(), serde_json::json! {[r#type, response, id]});
                        }
                        other => panic!("other type name {other}"),
                    },
                    MethodCallType::QueryChanges => unimplemented!(),
                    MethodCallType::Import => match object_type_name.as_str() {
                        "Email" => {
                            let import = serde_json::value::from_value(body).unwrap();
                            eprintln!("Parsed Email/import object {import:?}");
                            let email::EmailImport {
                                account_id,
                                if_in_state: _,
                                emails,
                            } = import;
                            let mut created = indexmap::indexmap! {};
                            let old_state = self.email_state.clone();
                            let mut new_state = self.email_state.clone();
                            for (_creation_id, import_object) in emails {
                                let blob_id = import_object.blob_id.clone();
                                let blob = &self.uploaded_blobs[&blob_id];
                                let size = blob.len();
                                new_state = self
                                    .insert_email(Box::new(Mail::new(blob.clone(), None).unwrap()));
                                let id = self
                                    .email_state_changes
                                    .last()
                                    .map(|(_, s)| {
                                        let StateChange::Created(id) = s else {
                                            panic!("expected Created state change");
                                        };
                                        id.clone()
                                    })
                                    .unwrap();
                                created.insert(
                                    id.clone(),
                                    email::EmailImportResult {
                                        id,
                                        blob_id,
                                        thread_id: Id::default(),
                                        size,
                                    },
                                );
                            }
                            let old_state = if old_state == self.email_state {
                                None
                            } else {
                                Some(old_state)
                            };
                            let response: email::EmailImportResponse = email::EmailImportResponse {
                                account_id,
                                old_state,
                                new_state,
                                created: Some(created),
                                not_created: None,
                            };
                            eprintln!("Sending {id} {type} response: {response:?}");
                            responses
                                .method_responses
                                .insert(id.clone(), serde_json::json! {[r#type, response, id]});
                        }
                        other => panic!("other type name {other}"),
                    },
                }
            }
            eprintln!("Sending API call responses: {responses:?}");
            responses
        }

        pub fn insert_email(&mut self, new: Box<Mail>) -> State<email::EmailObject> {
            let id: Id<email::EmailObject> = Id::new_random();
            let old_state = std::mem::replace(&mut self.email_state, State::new_random());
            self.envelopes.insert(id.clone(), *new);
            self.keywords.insert(id.clone(), indexmap::indexmap! {});
            let _none = self
                .email_state_changes
                .insert(old_state, StateChange::Created(id));
            match _none {
                None => {}
                Some(ref s) if matches!(s, StateChange::Initial) => {}
                Some(other) => panic!("{other:?}"),
            }

            self.email_state.clone()
        }

        pub fn destroy_email(&mut self, id: Id<email::EmailObject>) -> State<email::EmailObject> {
            let old_state = std::mem::replace(&mut self.email_state, State::new_random());
            self.envelopes.swap_remove(&id);
            self.keywords.swap_remove(&id);
            let _none = self
                .email_state_changes
                .insert(old_state, StateChange::Destroyed(id));
            assert!(_none.is_none());

            self.email_state.clone()
        }

        pub fn id_by_ref(&self, eref: EmailObjRef<'_>) -> Option<Id<email::EmailObject>> {
            match eref {
                EmailObjRef::Id(id) => {
                    if self.envelopes.contains_key(id) {
                        Some(id.clone())
                    } else {
                        None
                    }
                }
                EmailObjRef::MessageID(msgid) => self.envelopes.iter().find_map(|(h, e)| {
                    if e.message_id() == msgid {
                        Some(h.clone())
                    } else {
                        None
                    }
                }),
            }
        }

        pub fn set_flags(
            &mut self,
            eref: EmailObjRef<'_>,
            flag_op: FlagOp,
        ) -> State<email::EmailObject> {
            let id = self.id_by_ref(eref).unwrap();
            // 关键字表统一 JMAP 原生形态（String[Boolean]）：flag 位与自定义 tag
            // 一并记账，Email/get 注入与 set/回环测试共享同一真值源。
            let (keyword, set) = match flag_op {
                FlagOp::Set(f) => {
                    self.envelopes[&id].envelope.set_flag(f, true);
                    (flag_keyword(f).to_string(), true)
                }
                FlagOp::UnSet(f) => {
                    self.envelopes[&id].envelope.set_flag(f, false);
                    (flag_keyword(f).to_string(), false)
                }
                FlagOp::SetTag(tag) => (tag, true),
                FlagOp::UnSetTag(tag) => (tag, false),
            };
            if set {
                self.keywords
                    .entry(id.clone())
                    .or_default()
                    .insert(keyword, true);
            } else if let Some(keywords) = self.keywords.get_mut(&id) {
                keywords.shift_remove(&keyword);
            }
            let old_state = std::mem::replace(&mut self.email_state, State::new_random());
            let _none = self
                .email_state_changes
                .insert(old_state, StateChange::Updated(id));
            assert!(_none.is_none());

            self.email_state.clone()
        }
    }

    #[derive(Debug)]
    pub enum ServerEvent {
        New(Box<Mail>),
        SetFlags(String, FlagOp),
        Destroy(Id<email::EmailObject>),
        Quit,
    }

    pub struct JmapServerStream {
        pub tcp_listener: Async<TcpListener>,
        pub event_receiver: UnboundedReceiver<ServerEvent>,
        pub state: Arc<Mutex<ServerState>>,
        pub addr: SocketAddr,
    }

    impl JmapServerStream {
        pub fn new(
            tcp_listener: Async<TcpListener>,
            event_receiver: UnboundedReceiver<ServerEvent>,
            addr: SocketAddr,
        ) -> Self {
            let state = Arc::new(Mutex::new(ServerState::new(&addr)));
            Self {
                tcp_listener,
                event_receiver,
                state,
                addr,
            }
        }

        pub async fn loop_handler(self) -> Result<()> {
            let Self {
                tcp_listener,
                event_receiver,
                state,
                addr: _,
            } = self;

            let mut event_receiver = Box::pin(event_receiver.into_future());
            let mut accept = Box::pin(tcp_listener.accept());
            let mut event_streams = vec![];
            loop {
                match future::select(accept.as_mut(), event_receiver.as_mut()).await {
                    Either::Left((request, _)) => {
                        accept = Box::pin(tcp_listener.accept());
                        let mut buf = vec![];
                        let (tcp_stream, _socket_addr) = request?;
                        let mut tcp_stream = futures::io::BufReader::new(tcp_stream);
                        'bufread: loop {
                            use isahc::http::header::{self, HeaderName};

                            let bytes = tcp_stream.fill_buf().await?;

                            if bytes.is_empty() {
                                break;
                            }
                            let amt = bytes.len();
                            buf.extend(bytes);
                            tcp_stream.consume_unpin(amt);
                            let s = String::from_utf8_lossy(&buf);
                            if s.starts_with("GET ") && s.ends_with("\r\n\r\n") {
                                break;
                            }
                            let (headers, body) = s.split_once("\r\n\r\n").unwrap_or((&s, ""));
                            for hdr in headers.split("\r\n") {
                                let Some((hdrname, val)) = hdr.split_once(": ") else {
                                    continue;
                                };
                                let hdrname = hdrname.parse::<HeaderName>().unwrap();
                                if hdrname == header::CONTENT_LENGTH {
                                    let content_length = val.trim().parse::<usize>().unwrap();
                                    if body.len() == content_length {
                                        break 'bufread;
                                    }
                                }
                            }
                        }
                        let lossy = String::from_utf8_lossy(&buf);
                        eprintln!("loop_handler read:\n{lossy}");
                        tcp_stream.write_all(b"HTTP/1.1 200 OK\r\n").await?;
                        let email_state = state.lock().unwrap().email_state.clone();
                        let response = if lossy.starts_with("GET /.well-known/jmap") {
                            // Respond with Session resource.
                            serde_json::to_string(&state.lock().unwrap().session)
                                .unwrap()
                                .into_bytes()
                        } else if lossy.starts_with("POST /api/") {
                            serde_json::to_string(
                                &state
                                    .lock()
                                    .unwrap()
                                    .api(lossy.lines().rev().nth(0).unwrap()),
                            )
                            .unwrap()
                            .into_bytes()
                        } else if lossy.starts_with("POST /upload/A13824/") {
                            let (_, body) = lossy.split_once("\r\n\r\n").unwrap();
                            let size = body.len();
                            let blob_id = Id::new_random();
                            state
                                .lock()
                                .unwrap()
                                .uploaded_blobs
                                .insert(blob_id.clone(), body.as_bytes().to_vec());
                            serde_json::to_string(&UploadResponse {
                                account_id: "A13824".into(),
                                blob_id,
                                _type: "message/rfc822".into(),
                                size,
                            })
                            .unwrap()
                            .into_bytes()
                        } else if lossy.starts_with("GET /eventsource/") {
                            eprintln!("saving event stream");

                            tcp_stream
                                .write_all("Content-Type: text/event-stream\r\n\r\n".as_bytes())
                                .await?;
                            let state = state.lock().unwrap().email_state.clone();
                            tcp_stream
                                .write_all("event: state\r\ndata: ".as_bytes())
                                .await?;
                            tcp_stream
                                .write_all(
                                    serde_json::json! {
                                        {
                                            "@type": "StateChange",
                                            "changed": {
                                                "A13824": {
                                                    "Email": state,
                                                }
                                            }
                                        }

                                    }
                                    .to_string()
                                    .as_bytes(),
                                )
                                .await?;
                            tcp_stream.write_all("\r\n\r\n".as_bytes()).await?;
                            tcp_stream.flush().await?;
                            event_streams.push(tcp_stream);
                            continue;
                        } else {
                            unimplemented!()
                        };
                        let new_email_state = state.lock().unwrap().email_state.clone();
                        if email_state != new_email_state {
                            let new_states = {
                                let mut new_states = vec![];
                                let state = state.lock().unwrap();
                                if let Some(pos) = state
                                    .email_state_changes
                                    .keys()
                                    .position(|k| *k == email_state)
                                {
                                    for (state, _) in &state.email_state_changes[pos..] {
                                        new_states.push(state.clone());
                                    }
                                }
                                new_states
                            };
                            for state in new_states {
                                for event_stream in &mut event_streams {
                                    event_stream.write_all(b"event: state\r\ndata: ").await?;
                                    event_stream
                                        .write_all(
                                            serde_json::json! {
                                                {
                                                    "@type": "StateChange",
                                                    "changed": {
                                                        "A13824": {
                                                            "Email": state,
                                                        }
                                                    }
                                                }

                                            }
                                            .to_string()
                                            .as_bytes(),
                                        )
                                        .await?;
                                    event_stream.write_all(b"\r\n\r\n").await?;
                                    event_stream.flush().await?;
                                }
                            }
                        }
                        tcp_stream
                            .write_all(
                                format!("Content-Length: {}\r\n\r\n", response.len()).as_bytes(),
                            )
                            .await?;
                        tcp_stream.write_all(&response).await?;
                        tcp_stream.flush().await?;
                    }
                    Either::Right(((event, stream), _)) => {
                        *event_receiver = stream.into_future();
                        let Some(event) = event else {
                            continue;
                        };
                        match event {
                            ServerEvent::Quit => {
                                return Ok(());
                            }
                            ServerEvent::New(mail) => {
                                let new_state = {
                                    let mut state = state.lock().unwrap();
                                    state.insert_email(mail)
                                };
                                for event_stream in &mut event_streams {
                                    event_stream
                                        .write_all("event: state\r\ndata: ".as_bytes())
                                        .await?;
                                    event_stream
                                        .write_all(
                                            serde_json::json! {
                                                {
                                                    "@type": "StateChange",
                                                    "changed": {
                                                        "A13824": {
                                                            "Email": new_state,
                                                        }
                                                    }
                                                }

                                            }
                                            .to_string()
                                            .as_bytes(),
                                        )
                                        .await?;
                                    event_stream.write_all("\r\n\r\n".as_bytes()).await?;
                                    event_stream.flush().await?;
                                }
                            }
                            ServerEvent::SetFlags(msgid, flag_op) => {
                                let new_state = {
                                    let mut state = state.lock().unwrap();
                                    state.set_flags(
                                        EmailObjRef::MessageID(&MessageID::new(msgid)),
                                        flag_op,
                                    )
                                };
                                for event_stream in &mut event_streams {
                                    event_stream
                                        .write_all("event: state\r\ndata: ".as_bytes())
                                        .await?;
                                    event_stream
                                        .write_all(
                                            serde_json::json! {
                                                {
                                                    "@type": "StateChange",
                                                    "changed": {
                                                        "A13824": {
                                                            "Email": new_state,
                                                        }
                                                    }
                                                }

                                            }
                                            .to_string()
                                            .as_bytes(),
                                        )
                                        .await?;
                                    event_stream.write_all("\r\n\r\n".as_bytes()).await?;
                                    event_stream.flush().await?;
                                }
                            }
                            ServerEvent::Destroy(mail_id) => {
                                let new_state = {
                                    let mut state = state.lock().unwrap();
                                    state.destroy_email(mail_id)
                                };
                                for event_stream in &mut event_streams {
                                    event_stream
                                        .write_all("event: state\r\n".as_bytes())
                                        .await?;
                                    event_stream
                                        .write_all(
                                            serde_json::json! {
                                                {
                                                    "@type": "StateChange",
                                                    "changed": {
                                                        "A13824": {
                                                            "Email": new_state,
                                                        }
                                                    }
                                                }

                                            }
                                            .to_string()
                                            .as_bytes(),
                                        )
                                        .await?;
                                    event_stream.write_all("\r\n\r\n".as_bytes()).await?;
                                    event_stream.flush().await?;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

mod tests {
    use std::{
        net::TcpListener,
        sync::{Arc, Mutex},
    };

    use futures::{channel::mpsc::unbounded, executor::block_on, StreamExt};
    use melib::{backends::prelude::*, jmap::*, Mail};
    use tempfile::TempDir;

    use super::server::*;

    /// Test that `JmapType::refresh` returns the expected `Refresh` events when
    /// altering the mail store in the jmap server.
    pub(crate) fn run_jmap_refresh() {
        melib_test::init_test_logging();
        let temp_dir = TempDir::new().unwrap();
        let backend_event_queue =
            Arc::new(Mutex::new(std::collections::VecDeque::with_capacity(16)));

        let backend_event_consumer = {
            let backend_event_queue = Arc::clone(&backend_event_queue);

            BackendEventConsumer::new(Arc::new(move |ah, be| {
                eprintln!("BackendEventConsumer: ah {ah:?} be {be:?}");
                backend_event_queue.lock().unwrap().push_back((ah, be));
            }))
        };

        for var in [
            "HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
            "XDG_CONFIG_DIRS",
            "XDG_CONFIG_HOME",
            "XDG_DATA_DIRS",
            "XDG_DATA_HOME",
        ] {
            std::env::remove_var(var);
        }
        for (var, dir) in [
            ("HOME", temp_dir.path().to_path_buf()),
            ("XDG_CACHE_HOME", temp_dir.path().join(".cache")),
            ("XDG_STATE_HOME", temp_dir.path().join(".local/state")),
            ("XDG_CONFIG_HOME", temp_dir.path().join(".config")),
            ("XDG_DATA_HOME", temp_dir.path().join(".local/share")),
        ] {
            std::fs::create_dir_all(&dir).unwrap_or_else(|err| {
                panic!("Could not create {} path, {}: {}", var, dir.display(), err);
            });
            std::env::set_var(var, &dir);
        }

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let local_addr = listener.local_addr().unwrap();
        let account_conf = AccountSettings {
            name: "test".to_string(),
            root_mailbox: "INBOX".to_string(),
            format: "jmap".to_string(),
            identity: "user@example.com".to_string(),
            extra_identities: vec![],
            read_only: false,
            display_name: None,
            subscribed_mailboxes: vec![],
            mailboxes: indexmap::indexmap! {},
            manual_refresh: false,
            extra: {
                let mut m: indexmap::IndexMap<String, serde_json::Value> =
                    indexmap::IndexMap::new();

                m.insert(
                    "server_url".to_string(),
                    serde_json::Value::String(format!(
                        "http://{}:{}",
                        local_addr.ip(),
                        local_addr.port()
                    )),
                );
                m.insert(
                    "server_username".to_string(),
                    serde_json::Value::String("user".to_string()),
                );
                m.insert(
                    "server_password".to_string(),
                    serde_json::Value::String("password".to_string()),
                );
                m.insert(
                    "use_token".to_string(),
                    serde_json::Value::String("true".to_string()),
                );

                m
            },
        };

        let mut jmap =
            JmapType::new(&account_conf, Default::default(), backend_event_consumer).unwrap();
        let (server_event_sender, server_event_receiver) = unbounded();
        let server = JmapServerStream::new(
            smol::Async::new(listener).unwrap(),
            server_event_receiver,
            local_addr,
        );
        let server_state = server.state.clone();
        std::thread::spawn(move || {
            block_on(server.loop_handler()).unwrap();
        });
        eprintln!(
            "Assert that account is not online because we have not performed any operations yet."
        );
        assert_eq!(
            &block_on(jmap.is_online().unwrap()).unwrap_err().to_string(),
            "Account is uninitialised."
        );
        let mailboxes = block_on(jmap.mailboxes().unwrap()).unwrap();
        assert_eq!(
            mailboxes.len(),
            1,
            "Expected one mailbox, Inbox. Got: {mailboxes:?}"
        );
        let inbox_hash = *mailboxes.keys().next().unwrap();
        eprintln!("Inbox hash: {inbox_hash:?}");
        {
            let events = backend_event_queue
                .lock()
                .unwrap()
                .drain(..)
                .collect::<Vec<_>>();
            assert_eq!(
                events.len(),
                0,
                "Unexpected events without having performed any changes: {events:?}"
            );
        }
        eprintln!("Refresh account and assert that no events arrive.");
        block_on(jmap.refresh(inbox_hash).unwrap()).unwrap();
        {
            let events = backend_event_queue
                .lock()
                .unwrap()
                .drain(..)
                .collect::<Vec<_>>();
            assert_eq!(
                events.len(),
                0,
                "Unexpected events after refreshing without having performed any changes: \
                 {events:?}"
            );
        }

        eprintln!(
            "Add new e-mail on server store and assert that a refresh results in a Refresh Create \
             event."
        );
        let new_mail = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Cc:
Subject: RE: your e-mail
Message-ID: <h2g7f.z0gy2pgaen5m@example.com>
Content-Type: text/plain

hello world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let fetch_result: Vec<_> = block_on(jmap.fetch(inbox_hash).unwrap().collect::<Vec<_>>());
        for res in fetch_result {
            let res = res.unwrap();
            assert_eq!(res, vec![]);
        }
        server_event_sender
            .unbounded_send(ServerEvent::New(new_mail))
            .unwrap();
        block_on(jmap.refresh(inbox_hash).unwrap()).unwrap();
        let env_1 = {
            let events = backend_event_queue
                .lock()
                .unwrap()
                .drain(..)
                .collect::<Vec<_>>();
            assert_eq!(
                events.len(),
                1,
                "Expected one Refresh Create event: {events:?}"
            );
            let backend_event = events.into_iter().next().unwrap().1;
            let BackendEvent::Refresh(refresh_event) = backend_event else {
                panic!("Expected Refresh event, got: {backend_event:?}");
            };
            let RefreshEventKind::Create(env) = refresh_event.kind else {
                panic!("Expected Create event, got: {refresh_event:?}");
            };
            env
        };
        let fetch_result: Vec<_> = block_on(jmap.fetch(inbox_hash).unwrap().collect::<Vec<_>>());
        for res in fetch_result {
            let res = res.unwrap();
            assert_eq!(res.len(), 1);
            assert_eq!(res[0].hash(), env_1.hash());
        }
        eprintln!(
            "Destroy e-mail on server store and assert that a refresh results in a Refresh Remove \
             event."
        );
        let id_1 = server_state
            .lock()
            .unwrap()
            .envelopes
            .keys()
            .next()
            .unwrap()
            .clone();
        server_event_sender
            .unbounded_send(ServerEvent::Destroy(id_1))
            .unwrap();
        block_on(jmap.refresh(inbox_hash).unwrap()).unwrap();
        {
            let events = backend_event_queue
                .lock()
                .unwrap()
                .drain(..)
                .collect::<Vec<_>>();
            assert_eq!(
                events.len(),
                1,
                "Expected one Refresh Remove event: {events:?}"
            );
            let backend_event = events.into_iter().next().unwrap().1;
            let BackendEvent::Refresh(refresh_event) = backend_event else {
                panic!("Expected Refresh event, got: {backend_event:?}");
            };
            let RefreshEventKind::Remove(env_hash) = refresh_event.kind else {
                panic!("Expected Remove event, got: {refresh_event:?}");
            };
            assert_eq!(env_hash, env_1.hash());
        }
        {
            let error_responses = server_state.lock().unwrap().error_responses.clone();
            assert!(
                error_responses.is_empty(),
                "server replied with error responses during the test, expected none (in \
                 particular no cannotCalculateChanges): {error_responses:?}"
            );
        }
        server_event_sender
            .unbounded_send(ServerEvent::Quit)
            .unwrap();
    }

    /// Test that `JmapType::watch` `Stream` returns the expected `Refresh`
    /// events when altering the mail store in the JMAP server, using
    /// eventSourceUrl-type instead of polling.
    /// Test various data operations: saving an e-mail with both flags and
    /// tags must round-trip through upload, `Email/import` and `Email/set`
    /// keyword updates.
    pub(crate) fn run_jmap_operations() {
        melib_test::init_test_logging();
        let temp_dir = TempDir::new().unwrap();
        let backend_event_queue =
            Arc::new(Mutex::new(std::collections::VecDeque::with_capacity(16)));

        let backend_event_consumer = {
            let backend_event_queue = Arc::clone(&backend_event_queue);

            BackendEventConsumer::new(Arc::new(move |ah, be| {
                eprintln!("BackendEventConsumer: ah {ah:?} be {be:?}");
                backend_event_queue.lock().unwrap().push_back((ah, be));
            }))
        };

        for var in [
            "HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
            "XDG_CONFIG_DIRS",
            "XDG_CONFIG_HOME",
            "XDG_DATA_DIRS",
            "XDG_DATA_HOME",
        ] {
            std::env::remove_var(var);
        }
        for (var, dir) in [
            ("HOME", temp_dir.path().to_path_buf()),
            ("XDG_CACHE_HOME", temp_dir.path().join(".cache")),
            ("XDG_STATE_HOME", temp_dir.path().join(".local/state")),
            ("XDG_CONFIG_HOME", temp_dir.path().join(".config")),
            ("XDG_DATA_HOME", temp_dir.path().join(".local/share")),
        ] {
            std::fs::create_dir_all(&dir).unwrap_or_else(|err| {
                panic!("Could not create {} path, {}: {}", var, dir.display(), err);
            });
            std::env::set_var(var, &dir);
        }

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let local_addr = listener.local_addr().unwrap();
        let account_conf = AccountSettings {
            name: "test".to_string(),
            root_mailbox: "INBOX".to_string(),
            format: "jmap".to_string(),
            identity: "user@example.com".to_string(),
            extra_identities: vec![],
            read_only: false,
            display_name: None,
            subscribed_mailboxes: vec![],
            mailboxes: indexmap::indexmap! {},
            manual_refresh: false,
            extra: {
                let mut m: indexmap::IndexMap<String, serde_json::Value> =
                    indexmap::IndexMap::new();

                m.insert(
                    "server_url".to_string(),
                    serde_json::Value::String(format!(
                        "http://{}:{}",
                        local_addr.ip(),
                        local_addr.port()
                    )),
                );
                m.insert(
                    "server_username".to_string(),
                    serde_json::Value::String("user".to_string()),
                );
                m.insert(
                    "server_password".to_string(),
                    serde_json::Value::String("password".to_string()),
                );
                m.insert(
                    "use_token".to_string(),
                    serde_json::Value::String("true".to_string()),
                );

                m
            },
        };

        let mut jmap =
            JmapType::new(&account_conf, Default::default(), backend_event_consumer).unwrap();
        let (server_event_sender, server_event_receiver) = unbounded();
        let server = JmapServerStream::new(
            smol::Async::new(listener).unwrap(),
            server_event_receiver,
            local_addr,
        );
        std::thread::spawn(move || {
            block_on(server.loop_handler()).unwrap();
        });
        eprintln!(
            "Assert that account is not online because we have not performed any operations yet."
        );
        assert_eq!(
            &block_on(jmap.is_online().unwrap()).unwrap_err().to_string(),
            "Account is uninitialised."
        );
        let mailboxes = block_on(jmap.mailboxes().unwrap()).unwrap();
        assert_eq!(
            mailboxes.len(),
            1,
            "Expected one mailbox, Inbox. Got: {mailboxes:?}"
        );
        let inbox_hash = *mailboxes.keys().next().unwrap();
        eprintln!("Inbox hash: {inbox_hash:?}");
        {
            let events = backend_event_queue
                .lock()
                .unwrap()
                .drain(..)
                .collect::<Vec<_>>();
            assert_eq!(
                events.len(),
                0,
                "Unexpected events without having performed any changes: {events:?}"
            );
        }
        eprintln!("Refresh account and assert that no events arrive.");
        block_on(jmap.refresh(inbox_hash).unwrap()).unwrap();
        {
            let events = backend_event_queue
                .lock()
                .unwrap()
                .drain(..)
                .collect::<Vec<_>>();
            assert_eq!(
                events.len(),
                0,
                "Unexpected events after refreshing without having performed any changes: \
                 {events:?}"
            );
        }

        eprintln!(
            "Add new e-mail on server store and assert that a refresh results in a Refresh \
             Create event."
        );
        let new_mail = Box::new(
            Mail::new(
                br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Cc:
Subject: RE: your e-mail
Message-ID: <h2g7f.z0gy2pgaen5m@example.com>
Content-Type: text/plain

hello world.
"#
                .to_vec(),
                None,
            )
            .unwrap(),
        );
        let fetch_result: Vec<_> = block_on(
            jmap.fetch(inbox_hash)
                .unwrap()
                .collect::<Vec<Result<Vec<Envelope>>>>(),
        );
        for res in fetch_result {
            let res = res.unwrap();
            assert_eq!(res, vec![]);
        }
        block_on(
            jmap.save(
                new_mail.bytes.clone(),
                inbox_hash,
                Some((Flag::DRAFT, vec!["sent".to_string()])),
            )
            .unwrap(),
        )
        .unwrap();
        block_on(jmap.refresh(inbox_hash).unwrap()).unwrap();
        {
            let events = backend_event_queue
                .lock()
                .unwrap()
                .drain(..)
                .collect::<Vec<_>>();
            eprintln!("got events: {events:?}");
        };
        let fetch_result: Vec<_> = block_on(
            jmap.fetch(inbox_hash)
                .unwrap()
                .collect::<Vec<Result<Vec<Envelope>>>>(),
        );
        let collection = jmap.collection();
        let fetched = fetch_result
            .into_iter()
            .map(|res| res.unwrap())
            .flatten()
            .collect::<Vec<Envelope>>();
        assert_eq!(fetched.len(), 1);
        {
            let tag_index = collection.tag_index.read().unwrap();
            assert_eq!(fetched[0].flags(), Flag::DRAFT);
            assert_eq!(
                fetched[0]
                    .tags()
                    .into_iter()
                    .map(|t| &tag_index[t])
                    .collect::<Vec<_>>(),
                vec!["sent"]
            );
        }
        server_event_sender
            .unbounded_send(ServerEvent::Quit)
            .unwrap();
    }

    pub(crate) fn run_jmap_watch() {
        melib_test::init_test_logging();
        let temp_dir = TempDir::new().unwrap();
        let backend_event_queue =
            Arc::new(Mutex::new(std::collections::VecDeque::with_capacity(16)));

        let backend_event_consumer = {
            let backend_event_queue = Arc::clone(&backend_event_queue);

            BackendEventConsumer::new(Arc::new(move |ah, be| {
                eprintln!("BackendEventConsumer: ah {ah:?} be {be:?}");
                backend_event_queue.lock().unwrap().push_back((ah, be));
            }))
        };

        for var in [
            "HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
            "XDG_CONFIG_DIRS",
            "XDG_CONFIG_HOME",
            "XDG_DATA_DIRS",
            "XDG_DATA_HOME",
        ] {
            std::env::remove_var(var);
        }
        for (var, dir) in [
            ("HOME", temp_dir.path().to_path_buf()),
            ("XDG_CACHE_HOME", temp_dir.path().join(".cache")),
            ("XDG_STATE_HOME", temp_dir.path().join(".local/state")),
            ("XDG_CONFIG_HOME", temp_dir.path().join(".config")),
            ("XDG_DATA_HOME", temp_dir.path().join(".local/share")),
        ] {
            std::fs::create_dir_all(&dir).unwrap_or_else(|err| {
                panic!("Could not create {} path, {}: {}", var, dir.display(), err);
            });
            std::env::set_var(var, &dir);
        }

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let local_addr = listener.local_addr().unwrap();
        let account_conf = AccountSettings {
            name: "test".to_string(),
            root_mailbox: "INBOX".to_string(),
            format: "jmap".to_string(),
            identity: "user@example.com".to_string(),
            extra_identities: vec![],
            read_only: false,
            display_name: None,
            subscribed_mailboxes: vec![],
            mailboxes: indexmap::indexmap! {},
            manual_refresh: false,
            extra: {
                let mut m: indexmap::IndexMap<String, serde_json::Value> =
                    indexmap::IndexMap::new();

                m.insert(
                    "server_url".to_string(),
                    serde_json::Value::String(format!(
                        "http://{}:{}",
                        local_addr.ip(),
                        local_addr.port()
                    )),
                );
                m.insert(
                    "server_username".to_string(),
                    serde_json::Value::String("user".to_string()),
                );
                m.insert(
                    "server_password".to_string(),
                    serde_json::Value::String("password".to_string()),
                );
                m.insert(
                    "use_token".to_string(),
                    serde_json::Value::String("true".to_string()),
                );

                m
            },
        };

        let mut jmap =
            JmapType::new(&account_conf, Default::default(), backend_event_consumer).unwrap();
        let (server_event_sender, server_event_receiver) = unbounded();
        let server = JmapServerStream::new(
            smol::Async::new(listener).unwrap(),
            server_event_receiver,
            local_addr,
        );
        let server_state = server.state.clone();
        std::thread::spawn(move || {
            block_on(server.loop_handler()).unwrap();
        });
        let fut = async move {
            eprintln!(
                "Assert that account is not online because we have not performed any operations \
                 yet."
            );
            assert_eq!(
                &jmap.is_online().unwrap().await.unwrap_err().to_string(),
                "Account is uninitialised."
            );
            let mailboxes = jmap.mailboxes().unwrap().await.unwrap();
            assert_eq!(
                mailboxes.len(),
                1,
                "Expected one mailbox, Inbox. Got: {mailboxes:?}"
            );
            let inbox_hash = *mailboxes.keys().next().unwrap();
            eprintln!("Inbox hash: {inbox_hash:?}");
            {
                let events = backend_event_queue
                    .lock()
                    .unwrap()
                    .drain(..)
                    .collect::<Vec<_>>();
                assert_eq!(
                    events.len(),
                    0,
                    "Unexpected events without having performed any changes: {events:?}"
                );
            }

            eprintln!(
                "Add new e-mail on server store and assert that watch() creates a Refresh Create \
                 event."
            );
            let new_mail = Box::new(
                Mail::new(
                    br#"From: "some name" <some@example.com>
To: "me" <myself@example.com>
Cc:
Subject: RE: your e-mail
Message-ID: <h2g7f.z0gy2pgaen5m@example.com>
Content-Type: text/plain

hello world.
"#
                    .to_vec(),
                    None,
                )
                .unwrap(),
            );
            let fetch_result: Vec<_> = jmap.fetch(inbox_hash).unwrap().collect::<Vec<_>>().await;
            for res in fetch_result {
                let res = res.unwrap();
                assert_eq!(res, vec![]);
            }
            server_event_sender
                .unbounded_send(ServerEvent::New(new_mail))
                .unwrap();
            let watch_fut = jmap.watch().unwrap().into_future();
            let (backend_event, watch_fut) = watch_fut.await;
            let env_1 = {
                let Some(Ok(BackendEvent::Refresh(refresh_event))) = backend_event else {
                    panic!("Expected Refresh event, got: {backend_event:?}");
                };
                let RefreshEventKind::Create(env) = refresh_event.kind else {
                    panic!("Expected Create event, got: {refresh_event:?}");
                };
                env
            };
            let fetch_result: Vec<_> = jmap.fetch(inbox_hash).unwrap().collect::<Vec<_>>().await;
            for res in fetch_result {
                let res = res.unwrap();
                assert_eq!(res.len(), 1);
                assert_eq!(res[0].hash(), env_1.hash());
            }
            server_event_sender
                .unbounded_send(ServerEvent::SetFlags(
                    "h2g7f.z0gy2pgaen5m@example.com".to_string(),
                    FlagOp::Set(Flag::SEEN),
                ))
                .unwrap();
            let (backend_event, _watch_fut) = watch_fut.into_future().await;
            {
                let Some(Ok(BackendEvent::Refresh(refresh_event))) = backend_event else {
                    panic!("Expected Refresh event, got: {backend_event:?}");
                };
                let RefreshEventKind::NewFlags(env_hash, flags) = refresh_event.kind else {
                    panic!("Expected NewFlags event, got: {refresh_event:?}");
                };
                assert_eq!(env_hash, env_1.hash());
                assert_eq!(flags, (Flag::SEEN, vec![]));
            };
            eprintln!(
                "Destroy e-mail on server store and assert that a refresh results in a Refresh \
                 Remove event."
            );
            let id_1 = server_state
                .lock()
                .unwrap()
                .envelopes
                .keys()
                .next()
                .unwrap()
                .clone();
            server_event_sender
                .unbounded_send(ServerEvent::Destroy(id_1))
                .unwrap();
            jmap.refresh(inbox_hash).unwrap().await.unwrap();
            // The Remove event lands in the consumer queue asynchronously,
            // after `refresh()` resolves; poll briefly for the first arrival
            // instead of racing the drain (this was a load-dependent flake).
            // The SSE stream itself has already served its Create/NewFlags
            // pushes and does not yield a third item here.
            let events = {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                loop {
                    let events: Vec<_> = backend_event_queue.lock().unwrap().drain(..).collect();
                    if !events.is_empty() || std::time::Instant::now() >= deadline {
                        break events;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
            };
            assert_eq!(
                events.len(),
                1,
                "Expected one Refresh Remove event: {events:?}"
            );
            let backend_event = events.into_iter().next().unwrap().1;
            let BackendEvent::Refresh(refresh_event) = backend_event else {
                panic!("Expected Refresh event, got: {backend_event:?}");
            };
            let RefreshEventKind::Remove(env_hash) = refresh_event.kind else {
                panic!("Expected Remove event: {refresh_event:?}");
            };
            assert_eq!(env_hash, env_1.hash());
            {
                let error_responses = server_state.lock().unwrap().error_responses.clone();
                assert!(
                    error_responses.is_empty(),
                    "server replied with error responses during the test, expected none (in \
                     particular no cannotCalculateChanges): {error_responses:?}"
                );
            }
            server_event_sender
                .unbounded_send(ServerEvent::Quit)
                .unwrap();
        };
        std::thread::spawn(move || {
            block_on(fut);
        })
        .join()
        .unwrap();
    }
    /// Test that `JmapType::contact_backend` wires a JMAP contacts backend
    /// against the mock server's session object: the session advertises the
    /// contacts capability and a primary contacts account, so
    /// `contact_backend()` must resolve into a `JmapContacts` instance whose
    /// reported contact capabilities match the mock server's (empty) contacts
    /// capability object, all offline.
    pub(crate) fn run_jmap_contacts() {
        melib_test::init_test_logging();
        let temp_dir = TempDir::new().unwrap();
        let backend_event_consumer = {
            BackendEventConsumer::new(Arc::new(move |ah, be| {
                eprintln!("BackendEventConsumer: ah {ah:?} be {be:?}");
            }))
        };

        for var in [
            "HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
            "XDG_CONFIG_DIRS",
            "XDG_CONFIG_HOME",
            "XDG_DATA_DIRS",
            "XDG_DATA_HOME",
        ] {
            std::env::remove_var(var);
        }
        for (var, dir) in [
            ("HOME", temp_dir.path().to_path_buf()),
            ("XDG_CACHE_HOME", temp_dir.path().join(".cache")),
            ("XDG_STATE_HOME", temp_dir.path().join(".local/state")),
            ("XDG_CONFIG_HOME", temp_dir.path().join(".config")),
            ("XDG_DATA_HOME", temp_dir.path().join(".local/share")),
        ] {
            std::fs::create_dir_all(&dir).unwrap_or_else(|err| {
                panic!("Could not create {} path, {}: {}", var, dir.display(), err);
            });
            std::env::set_var(var, &dir);
        }

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let local_addr = listener.local_addr().unwrap();
        let account_conf = AccountSettings {
            name: "test".to_string(),
            root_mailbox: "INBOX".to_string(),
            format: "jmap".to_string(),
            identity: "user@example.com".to_string(),
            extra_identities: vec![],
            read_only: false,
            display_name: None,
            subscribed_mailboxes: vec![],
            mailboxes: indexmap::indexmap! {},
            manual_refresh: false,
            extra: {
                let mut m: indexmap::IndexMap<String, serde_json::Value> =
                    indexmap::IndexMap::new();

                m.insert(
                    "server_url".to_string(),
                    serde_json::Value::String(format!(
                        "http://{}:{}",
                        local_addr.ip(),
                        local_addr.port()
                    )),
                );
                m.insert(
                    "server_username".to_string(),
                    serde_json::Value::String("user".to_string()),
                );
                m.insert(
                    "server_password".to_string(),
                    serde_json::Value::String("password".to_string()),
                );
                m.insert(
                    "use_token".to_string(),
                    serde_json::Value::String("true".to_string()),
                );

                m
            },
        };

        let mut jmap =
            JmapType::new(&account_conf, Default::default(), backend_event_consumer).unwrap();
        let (server_event_sender, server_event_receiver) = unbounded();
        let server = JmapServerStream::new(
            smol::Async::new(listener).unwrap(),
            server_event_receiver,
            local_addr,
        );
        std::thread::spawn(move || {
            block_on(server.loop_handler()).unwrap();
        });

        eprintln!("Assert contact_backend resolves into a JMAP contacts backend.");
        let mut contacts = block_on(jmap.contact_backend().unwrap()).unwrap();
        block_on(contacts.is_online().unwrap()).unwrap();
        let contact_caps = contacts.capabilities();
        assert!(
            contact_caps.is_async && contact_caps.is_remote && contact_caps.supports_search,
            "Unexpected contact backend capabilities: {contact_caps:?}"
        );
        assert!(
            !contact_caps.can_create_address_book,
            "mock server does not advertise mayCreateAddressBook: {contact_caps:?}"
        );
        assert_eq!(
            contact_caps.max_address_books_per_card, None,
            "mock server does not advertise maxAddressBooksPerCard: {contact_caps:?}"
        );
        eprintln!("Assert MailBackend capabilities advertise contacts support.");
        let mail_caps = jmap.capabilities();
        assert!(
            mail_caps.supports_contacts,
            "expected MailBackendCapabilities::supports_contacts after session connect:              {mail_caps:?}"
        );
        server_event_sender
            .unbounded_send(ServerEvent::Quit)
            .unwrap();
    }
}
