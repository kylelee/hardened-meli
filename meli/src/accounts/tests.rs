//
// meli
//
// Copyright 2024 Emmanouil Pitsidianakis <manos@pitsidianak.is>
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

use std::path::PathBuf;

use melib::{
    backends::{prelude::*, Mailbox, MailboxHash},
    error::Result,
    maildir::MaildirType,
    smol, MailboxPermissions, SpecialUsageMailbox,
};
use tempfile::TempDir;

use crate::{
    accounts::{build_mailboxes_order, AccountConf, FileMailboxConf, MailboxEntry, MailboxStatus},
    command::actions::MailboxOperation,
    mail::listing::{CursorPos, ListingComponent, MenuEntryCursor, OfflineListing},
    types::UIEvent,
    utilities::tests::{eprint_step_fn, eprintln_ok_fn},
};

#[test]
fn test_mailbox_utf7() {
    #[derive(Debug)]
    struct TestMailbox(String);

    impl melib::BackendMailbox for TestMailbox {
        fn hash(&self) -> MailboxHash {
            unimplemented!()
        }

        fn name(&self) -> &str {
            &self.0
        }

        fn path(&self) -> &str {
            &self.0
        }

        fn children(&self) -> &[MailboxHash] {
            unimplemented!()
        }

        fn clone(&self) -> Mailbox {
            unimplemented!()
        }

        fn special_usage(&self) -> SpecialUsageMailbox {
            unimplemented!()
        }

        fn parent(&self) -> Option<MailboxHash> {
            unimplemented!()
        }

        fn permissions(&self) -> MailboxPermissions {
            unimplemented!()
        }

        fn is_subscribed(&self) -> bool {
            unimplemented!()
        }

        fn set_is_subscribed(&mut self, _: bool) -> Result<()> {
            unimplemented!()
        }

        fn set_special_usage(&mut self, _: SpecialUsageMailbox) -> Result<()> {
            unimplemented!()
        }

        fn count(&self) -> Result<(usize, usize)> {
            unimplemented!()
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }
    for (n, d) in [
        ("~peter/mail/&U,BTFw-/&ZeVnLIqe-", "~peter/mail/台北/日本語"),
        ("&BB4EQgQ,BEAEMAQyBDsENQQ9BD0ESwQ1-", "Отправленные"),
    ] {
        let ref_mbox = TestMailbox(n.to_string());
        let mut conf: melib::MailboxConf = Default::default();
        conf.extra.insert("encoding".to_string(), "utf7".into());

        let entry = MailboxEntry::new(
            MailboxStatus::None,
            n.to_string(),
            Box::new(ref_mbox),
            FileMailboxConf {
                mailbox_conf: conf,
                ..Default::default()
            },
        );
        assert_eq!(&entry.path, d);
    }
}

fn new_maildir_backend(
    temp_dir: &TempDir,
    acc_name: &str,
    event_consumer: BackendEventConsumer,
    with_root_mailbox: bool,
) -> Result<(PathBuf, AccountConf, Box<MaildirType>)> {
    let root_mailbox = temp_dir.path().join("inbox");
    {
        std::fs::create_dir(&root_mailbox).expect("Could not create root mailbox directory.");
        if with_root_mailbox {
            for d in &["cur", "new", "tmp"] {
                std::fs::create_dir(root_mailbox.join(d))
                    .expect("Could not create root mailbox directory contents.");
            }
        }
    }
    let subscribed_mailboxes = if with_root_mailbox {
        vec!["inbox".into()]
    } else {
        vec![]
    };
    let mailboxes = if with_root_mailbox {
        vec![(
            "inbox".into(),
            melib::conf::MailboxConf {
                extra: indexmap::indexmap! {
                    "path".into() => root_mailbox.display().to_string(),
                },
                ..Default::default()
            },
        )]
        .into_iter()
        .collect()
    } else {
        indexmap::indexmap! {}
    };
    let extra = if with_root_mailbox {
        indexmap::indexmap! {
            "root_mailbox".into() => root_mailbox.display().to_string(),
        }
    } else {
        indexmap::indexmap! {}
    };

    let account_conf = melib::AccountSettings {
        name: acc_name.to_string(),
        root_mailbox: root_mailbox.display().to_string(),
        format: "maildir".to_string(),
        identity: "user@localhost".to_string(),
        extra_identities: vec![],
        read_only: false,
        display_name: None,
        subscribed_mailboxes,
        mailboxes,
        manual_refresh: true,
        extra,
    };

    let maildir = MaildirType::new(&account_conf, Default::default(), event_consumer)?;
    Ok((root_mailbox, account_conf.into(), maildir))
}

#[test]
fn test_accounts_mailbox_by_path_error_msg() {
    const ACCOUNT_NAME: &str = "test";

    let eprintln_ok = eprintln_ok_fn();
    let mut eprint_step_closure = eprint_step_fn();
    macro_rules! eprint_step {
        ($($arg:tt)+) => {{
            eprint_step_closure(format_args!($($arg)+));
        }};
    }
    let temp_dir = TempDir::new().unwrap();
    {
        eprint_step!(
            "Create maildir backend with a root mailbox, \"inbox\" which will be a valid maildir \
             folder because it will contain cur, new, tmp subdirectories..."
        );
        let mut ctx = crate::Context::new_mock(&temp_dir);
        let backend_event_queue = Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::with_capacity(16),
        ));

        let backend_event_consumer = {
            let backend_event_queue = Arc::clone(&backend_event_queue);

            BackendEventConsumer::new(Arc::new(move |ah, be| {
                backend_event_queue.lock().unwrap().push_back((ah, be));
            }))
        };

        let (root_mailbox, settings, maildir) =
            new_maildir_backend(&temp_dir, ACCOUNT_NAME, backend_event_consumer, true).unwrap();
        eprintln_ok();
        let name = maildir.account_name.to_string();
        let account_hash = maildir.account_hash;
        let mut backend = maildir as Box<dyn MailBackend>;
        let ref_mailboxes = smol::block_on(backend.mailboxes().unwrap()).unwrap();
        let contacts = melib::contacts::Contacts::new(name.to_string());

        let mut account = super::Account {
            hash: account_hash,
            name: name.into(),
            is_online: super::IsOnline::True,
            mailbox_entries: Default::default(),
            mailboxes_order: Default::default(),
            tree: Default::default(),
            contacts,
            collection: backend.collection(),
            settings,
            main_loop_handler: ctx.main_loop_handler.clone(),
            active_jobs: HashMap::default(),
            active_job_instants: std::collections::BTreeMap::default(),
            event_queue: IndexMap::default(),
            backend_capabilities: backend.capabilities(),
            backend: Arc::new(std::sync::Mutex::new(backend)),
        };
        account.init(ref_mailboxes).unwrap();
        while let Ok(thread_event) = ctx.receiver.try_recv() {
            if let crate::ThreadEvent::JobFinished(job_id) = thread_event {
                if !account.process_event(&job_id) {
                    assert!(
                        ctx.accounts[0].process_event(&job_id),
                        "unclaimed job id: {job_id:?}"
                    );
                }
            }
        }
        eprint_step!("Assert that mailbox_by_path(\"inbox\") returns the root mailbox...");
        account.mailbox_by_path("inbox").unwrap();
        eprintln_ok();
        eprint_step!(
            "Assert that mailbox_by_path(\"box\") returns an error mentioning the root mailbox..."
        );
        assert_eq!(
            account.mailbox_by_path("box").unwrap_err().to_string(),
            Error {
                summary: "Mailbox with that path not found.".into(),
                details: Some(
                    "Some matching paths that were found: [\"inbox\"]. You can inspect the list \
                     of mailbox paths of an account with the manage-mailboxes command."
                        .into()
                ),
                source: None,
                inner: None,
                related_path: None,
                kind: ErrorKind::NotFound
            }
            .to_string()
        );
        eprintln_ok();

        macro_rules! wait_for_job {
            ($job_id:expr) => {{
                let wait_for = $job_id;
                while let Ok(thread_event) = ctx.receiver.recv() {
                    if let crate::ThreadEvent::JobFinished(job_id) = thread_event {
                        if !account.process_event(&job_id) {
                            assert!(
                                ctx.accounts[0].process_event(&job_id),
                                "unclaimed job id: {:?}",
                                job_id
                            );
                        } else if job_id == wait_for {
                            break;
                        }
                    }
                }
            }};
        }
        eprint_step!(
            "Create new mailboxes: \"Sent\", \"Trash\", \"Drafts\", \"Archive\", \"Outbox\", \
             \"Archive/Archive (old)\"..."
        );
        wait_for_job!(account
            .mailbox_operation(MailboxOperation::Create("Sent".to_string()))
            .unwrap());
        wait_for_job!(account
            .mailbox_operation(MailboxOperation::Create("Trash".to_string()))
            .unwrap());
        wait_for_job!(account
            .mailbox_operation(MailboxOperation::Create("Drafts".to_string()))
            .unwrap());
        wait_for_job!(account
            .mailbox_operation(MailboxOperation::Create("Archive".to_string()))
            .unwrap());
        wait_for_job!(account
            .mailbox_operation(MailboxOperation::Create("Outbox".to_string()))
            .unwrap());
        wait_for_job!(account
            .mailbox_operation(MailboxOperation::Create(
                "inbox/Archive/Archive (old)".to_string(),
            ))
            .unwrap());
        eprintln_ok();
        eprint_step!(
            "Assert that mailbox_by_path(\"rchive\") returns an error and mentions matching \
             archives with mailboxes with the least depth in the tree hierarchy of mailboxes \
             mentioned first..."
        );
        assert_eq!(
            account.mailbox_by_path("rchive").unwrap_err().to_string(),
            Error {
                summary: "Mailbox with that path not found.".into(),
                details: Some(
                    "Some matching paths that were found: [\"inbox/Archive\", \
                     \"inbox/Archive/Archive (old)\"]. You can inspect the list of mailbox paths \
                     of an account with the manage-mailboxes command."
                        .into()
                ),
                source: None,
                inner: None,
                related_path: None,
                kind: ErrorKind::NotFound
            }
            .to_string()
        );
        eprintln_ok();
        eprint_step!("Create \"inbox/Archive/Archive{{1,2,3,4,5,6,7,8,9,10}}\" mailboxes...");
        for i in 1..=10 {
            wait_for_job!(account
                .mailbox_operation(MailboxOperation::Create(format!(
                    "inbox/Archive/Archive{i}"
                )))
                .unwrap());
        }
        eprintln_ok();
        eprint_step!(
            "Assert that mailbox_by_path(\"inbox/Archive/Archive{{n}}\") works, i.e. we have to \
             specify the root prefix \"inbox\"..."
        );
        for i in 1..=10 {
            account
                .mailbox_by_path(&format!("inbox/Archive/Archive{i}"))
                .unwrap();
            account
                .mailbox_by_path(&format!("Archive/Archive{i}"))
                .unwrap_err();
        }
        eprintln_ok();
        eprint_step!(
            "Assert that mailbox_by_path(\"rchive\") returns and error and truncates the matching \
             mailbox paths to 5 maximum..."
        );
        assert_eq!(
            account.mailbox_by_path("rchive").unwrap_err().to_string(),
            Error {
                summary: "Mailbox with that path not found.".into(),
                details: Some(
                    "Some matching paths that were found: [\"inbox/Archive\", \
                     \"inbox/Archive/Archive1\", \"inbox/Archive/Archive2\", \
                     \"inbox/Archive/Archive3\", \"inbox/Archive/Archive4\"] and 7 others. You \
                     can inspect the list of mailbox paths of an account with the \
                     manage-mailboxes command."
                        .into()
                ),
                source: None,
                inner: None,
                related_path: None,
                kind: ErrorKind::NotFound
            }
            .to_string()
        );
        eprintln_ok();
        eprint_step!(
            "Assert that mailbox_by_path(\"inbox/Archive\") returns a valid result (since the \
             root mailbox is a valid maildir folder)..."
        );
        account.mailbox_by_path("inbox/Archive").unwrap();
        eprintln_ok();

        eprint_step!("Cleanup maildir account with valid root mailbox...");
        std::fs::remove_dir_all(root_mailbox).unwrap();
        eprintln_ok();
    }

    {
        eprint_step!(
            "Create maildir backend with a root mailbox, \"inbox\" which will NOT be a valid \
             maildir folder because it will NOT contain cur, new, tmp subdirectories..."
        );
        let mut ctx = crate::Context::new_mock(&temp_dir);
        let backend_event_queue = Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::with_capacity(16),
        ));

        let backend_event_consumer = {
            let backend_event_queue = Arc::clone(&backend_event_queue);

            BackendEventConsumer::new(Arc::new(move |ah, be| {
                backend_event_queue.lock().unwrap().push_back((ah, be));
            }))
        };

        let (_root_mailbox, settings, maildir) =
            new_maildir_backend(&temp_dir, ACCOUNT_NAME, backend_event_consumer, false).unwrap();
        eprintln_ok();
        let name = maildir.account_name.to_string();
        let account_hash = maildir.account_hash;
        let mut backend = maildir as Box<dyn MailBackend>;
        let ref_mailboxes = smol::block_on(backend.mailboxes().unwrap()).unwrap();
        eprint_step!("Assert that created account has no mailboxes at all...");
        assert!(
            ref_mailboxes.is_empty(),
            "ref_mailboxes were not empty: {ref_mailboxes:?}"
        );
        eprintln_ok();
        let contacts = melib::contacts::Contacts::new(name.to_string());

        let mut account = super::Account {
            hash: account_hash,
            name: name.into(),
            is_online: super::IsOnline::True,
            mailbox_entries: Default::default(),
            mailboxes_order: Default::default(),
            tree: Default::default(),
            contacts,
            collection: backend.collection(),
            settings,
            main_loop_handler: ctx.main_loop_handler.clone(),
            active_jobs: HashMap::default(),
            active_job_instants: std::collections::BTreeMap::default(),
            event_queue: IndexMap::default(),
            backend_capabilities: backend.capabilities(),
            backend: Arc::new(std::sync::Mutex::new(backend)),
        };
        account.init(ref_mailboxes).unwrap();
        while let Ok(thread_event) = ctx.receiver.try_recv() {
            if let crate::ThreadEvent::JobFinished(job_id) = thread_event {
                if !account.process_event(&job_id) {
                    assert!(
                        ctx.accounts[0].process_event(&job_id),
                        "unclaimed job id: {job_id:?}"
                    );
                }
            }
        }
        eprint_step!(
            "Assert that mailbox_by_path(\"inbox\") does not return a valid result (there are no \
             mailboxes)..."
        );
        assert_eq!(
            account.mailbox_by_path("inbox").unwrap_err().to_string(),
            Error {
                summary: "Mailbox with that path not found.".into(),
                details: Some(
                    "You can inspect the list of mailbox paths of an account with the \
                     manage-mailboxes command."
                        .into()
                ),
                source: None,
                inner: None,
                related_path: None,
                kind: ErrorKind::NotFound
            }
            .to_string()
        );
        eprintln_ok();
        eprint_step!(
            "Create multiple maildir folders \"inbox/Archive{{1,2,3,4,5,6,7,8,9,10}}\"..."
        );
        macro_rules! wait_for_job {
            ($job_id:expr) => {{
                let wait_for = $job_id;
                while let Ok(thread_event) = ctx.receiver.recv() {
                    if let crate::ThreadEvent::JobFinished(job_id) = thread_event {
                        if !account.process_event(&job_id) {
                            assert!(
                                ctx.accounts[0].process_event(&job_id),
                                "unclaimed job id: {:?}",
                                job_id
                            );
                        } else if job_id == wait_for {
                            break;
                        }
                    }
                }
            }};
        }
        for i in 1..=10 {
            wait_for_job!(account
                .mailbox_operation(MailboxOperation::Create(format!("inbox/Archive{i}")))
                .unwrap());
        }
        eprintln_ok();
        eprint_step!(
            "Assert that mailbox_by_path(\"Archive{{n}}\") works, and that we don't have to \
             specify the root prefix \"inbox\"..."
        );
        for i in 1..=10 {
            account.mailbox_by_path(&format!("Archive{i}")).unwrap();
        }
        eprintln_ok();
        eprint_step!(
            "Assert that mailbox_by_path(\"rchive\") returns an error message with matches..."
        );
        assert_eq!(
            account.mailbox_by_path("rchive").unwrap_err().to_string(),
            Error {
                summary: "Mailbox with that path not found.".into(),
                details: Some(
                    "Some matching paths that were found: [\"Archive1\", \"Archive2\", \
                     \"Archive3\", \"Archive4\", \"Archive5\"] and 5 others. You can inspect the \
                     list of mailbox paths of an account with the manage-mailboxes command."
                        .into()
                ),
                source: None,
                inner: None,
                related_path: None,
                kind: ErrorKind::NotFound
            }
            .to_string()
        );
        eprintln_ok();
        eprint_step!(
            "Assert that mailbox_by_path(\"inbox/Archive{{n}}\") does not return a valid result..."
        );
        assert_eq!(
            account
                .mailbox_by_path("inbox/Archive1")
                .unwrap_err()
                .to_string(),
            Error {
                summary: "Mailbox with that path not found.".into(),
                details: Some(
                    "You can inspect the list of mailbox paths of an account with the \
                     manage-mailboxes command."
                        .into()
                ),
                source: None,
                inner: None,
                related_path: None,
                kind: ErrorKind::NotFound
            }
            .to_string()
        );
        eprintln_ok();
    }
}

/// Stand-in [`melib::BackendMailbox`] for tests: fully implemented (no
/// `unimplemented!()`) so it can flow through entry construction and
/// `build_mailboxes_order`.
#[derive(Debug)]
struct SynthMailbox {
    hash: MailboxHash,
    name: String,
    usage: SpecialUsageMailbox,
}

fn synth_mailbox(name: &str, usage: SpecialUsageMailbox) -> Mailbox {
    Box::new(SynthMailbox {
        hash: MailboxHash::from_bytes(name.as_bytes()),
        name: name.to_string(),
        usage,
    })
}

impl melib::BackendMailbox for SynthMailbox {
    fn hash(&self) -> MailboxHash {
        self.hash
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn path(&self) -> &str {
        &self.name
    }

    fn children(&self) -> &[MailboxHash] {
        &[]
    }

    fn clone(&self) -> Mailbox {
        Box::new(Self {
            hash: self.hash,
            name: self.name.clone(),
            usage: self.usage,
        })
    }

    fn special_usage(&self) -> SpecialUsageMailbox {
        self.usage
    }

    fn parent(&self) -> Option<MailboxHash> {
        None
    }

    fn permissions(&self) -> MailboxPermissions {
        MailboxPermissions::default()
    }

    fn is_subscribed(&self) -> bool {
        true
    }

    fn set_is_subscribed(&mut self, _: bool) -> Result<()> {
        Ok(())
    }

    fn set_special_usage(&mut self, usage: SpecialUsageMailbox) -> Result<()> {
        self.usage = usage;
        Ok(())
    }

    fn count(&self) -> Result<(usize, usize)> {
        Ok((0, 0))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[test]
fn reconcile_mailboxes_merge() {
    const ACCOUNT_NAME: &str = "test";

    let eprintln_ok = eprintln_ok_fn();
    let mut eprint_step_closure = eprint_step_fn();
    macro_rules! eprint_step {
        ($($arg:tt)+) => {{
            eprint_step_closure(format_args!($($arg)+));
        }};
    }
    let temp_dir = TempDir::new().unwrap();
    let ctx = crate::Context::new_mock(&temp_dir);
    let backend_event_queue = Arc::new(std::sync::Mutex::new(
        std::collections::VecDeque::with_capacity(16),
    ));

    let backend_event_consumer = {
        let backend_event_queue = Arc::clone(&backend_event_queue);

        BackendEventConsumer::new(Arc::new(move |ah, be| {
            backend_event_queue.lock().unwrap().push_back((ah, be));
        }))
    };

    eprint_step!("Create maildir account with a single root mailbox \"inbox\"...");
    let (_root_mailbox, settings, maildir) =
        new_maildir_backend(&temp_dir, ACCOUNT_NAME, backend_event_consumer, true).unwrap();
    eprintln_ok();
    let name = maildir.account_name.to_string();
    let account_hash = maildir.account_hash;
    let mut backend = maildir as Box<dyn MailBackend>;
    let ref_mailboxes = smol::block_on(backend.mailboxes().unwrap()).unwrap();
    let (&inbox_hash, _) = ref_mailboxes
        .iter()
        .find(|(_, m)| m.path() == "inbox")
        .expect("root mailbox `inbox` present");
    let contacts = melib::contacts::Contacts::new(name.to_string());

    let mut account = super::Account {
        hash: account_hash,
        name: name.into(),
        is_online: super::IsOnline::True,
        mailbox_entries: Default::default(),
        mailboxes_order: Default::default(),
        tree: Default::default(),
        contacts,
        collection: backend.collection(),
        settings,
        main_loop_handler: ctx.main_loop_handler.clone(),
        active_jobs: HashMap::default(),
        active_job_instants: std::collections::BTreeMap::default(),
        event_queue: IndexMap::default(),
        backend_capabilities: backend.capabilities(),
        backend: Arc::new(std::sync::Mutex::new(backend)),
    };
    // `gone_hash` exists locally at init time but is absent from the later
    // refreshed list (i.e. it was removed server-side).
    let gone_hash = MailboxHash::from_bytes(b"Gone");
    let mut init_map = ref_mailboxes;
    init_map.insert(
        gone_hash,
        synth_mailbox("Gone", SpecialUsageMailbox::Normal),
    );
    account.init(init_map).unwrap();
    while ctx.receiver.try_recv().is_ok() {}
    eprintln_ok();

    let inbox_status_before = account.mailbox_entries[&inbox_hash].status.clone();
    let inbox_usage_before = account.mailbox_entries[&inbox_hash]
        .ref_mailbox
        .special_usage();
    assert_ne!(inbox_usage_before, SpecialUsageMailbox::Archive);
    assert!(account.mailbox_entries.contains_key(&gone_hash));
    assert!(account.mailboxes_order.contains(&gone_hash));

    eprint_step!(
        "Reconcile with a refreshed list that adds a new mailbox, changes the usage of an \
         existing one, and omits another..."
    );
    let new_hash = MailboxHash::from_bytes(b"NewBox");
    let mut refresh_map = HashMap::new();
    refresh_map.insert(
        inbox_hash,
        synth_mailbox("inbox", SpecialUsageMailbox::Archive),
    );
    refresh_map.insert(
        new_hash,
        synth_mailbox("NewBox", SpecialUsageMailbox::Normal),
    );
    account.reconcile_mailboxes(refresh_map);
    eprintln_ok();

    eprint_step!("Assert the add/update/retain three-state behavior...");
    // (a) new mailbox appears both in entries and in the rebuilt order.
    assert!(
        account.mailbox_entries.contains_key(&new_hash),
        "new mailbox missing from mailbox_entries"
    );
    assert!(
        account.mailboxes_order.contains(&new_hash),
        "new mailbox missing from mailboxes_order"
    );
    // (b) existing mailbox: `ref_mailbox` refreshed (usage changed), while
    // status and path are preserved.
    let inbox_entry = &account.mailbox_entries[&inbox_hash];
    assert_eq!(
        inbox_entry.ref_mailbox.special_usage(),
        SpecialUsageMailbox::Archive
    );
    assert_eq!(
        format!("{:?}", inbox_entry.status),
        format!("{:?}", inbox_status_before),
        "existing mailbox status must be preserved"
    );
    assert_eq!(inbox_entry.path, "inbox");
    // (c) mailbox missing from the refreshed list is retained, never deleted.
    assert!(
        account.mailbox_entries.contains_key(&gone_hash),
        "server-side removed mailbox must be retained"
    );
    assert!(account.mailboxes_order.contains(&gone_hash));
    eprintln_ok();

    eprint_step!("Assert the UI events emitted by the reconcile...");
    let mut got_mailbox_create = false;
    let mut got_status_change = false;
    while let Ok(thread_event) = ctx.receiver.try_recv() {
        if let crate::ThreadEvent::UIEvent(ui_event) = thread_event {
            match ui_event {
                UIEvent::MailboxCreate((ah, mh)) if ah == account_hash && mh == new_hash => {
                    got_mailbox_create = true;
                }
                UIEvent::AccountStatusChange(ah, Some(ref msg))
                    if ah == account_hash && msg.as_ref() == "Refreshed mailboxes." =>
                {
                    got_status_change = true;
                }
                _ => {}
            }
        }
    }
    assert!(
        got_mailbox_create,
        "UIEvent::MailboxCreate for the new mailbox was not sent"
    );
    assert!(
        got_status_change,
        "UIEvent::AccountStatusChange(\"Refreshed mailboxes.\") was not sent"
    );
    eprintln_ok();

    eprint_step!("Assert that an empty refreshed list returns early without events...");
    while ctx.receiver.try_recv().is_ok() {}
    account.reconcile_mailboxes(HashMap::new());
    while let Ok(thread_event) = ctx.receiver.try_recv() {
        if let crate::ThreadEvent::UIEvent(ui_event) = thread_event {
            assert!(
                !matches!(
                    ui_event,
                    UIEvent::MailboxCreate(_) | UIEvent::AccountStatusChange(..)
                ),
                "empty ref map must not emit mailbox/status UI events"
            );
        }
    }
    eprintln_ok();
}

/// A port that is bound and released: connecting to it afterwards fails
/// immediately (loopback, no real hosts or credentials involved).
fn dead_port() -> u16 {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.local_addr().unwrap().port()
}

/// Two cached mails (uids 1..=2) with deterministic hashes.
fn cold_start_cached_mails() -> Vec<(usize, melib::Envelope)> {
    use melib::imap::generate_envelope_hash;

    (1..=2usize)
        .map(|uid| {
            let bytes = format!(
                "From: a@b.example\r\nTo: c@d.example\r\nSubject: cold start {uid}\r\n\
                 Message-ID: <cold-{uid}@x.example>\r\n\
                 Date: Thu, 1 Jan 2026 00:00:0{uid} +0000\r\n\r\nbody {uid}\r\n"
            );
            let mail = melib::email::Mail::new(bytes.into_bytes(), None).unwrap();
            let mut env = mail.envelope;
            env.set_hash(generate_envelope_hash("inbox", &uid));
            (uid, env)
        })
        .collect()
}

/// Seed `<XDG_DATA_HOME>/meli/<account_name>_header_cache.db` with an
/// already-synchronized INBOX: the cached mailbox list, a `mailbox` state
/// row (`uidvalidity` 1, `max_uid` 2, `uidnext` 3) and the given mails, so
/// an offline `ImapType` can cold-start from it. The schema mirrors
/// `Sqlite3Cache`'s `DB_DESCRIPTION` init script (`user_version` 5).
fn seed_offline_imap_cache(account_name: &str, mails: &[(usize, melib::Envelope)]) -> MailboxHash {
    use melib::{imap::sync::cache::CachedImapMailbox, utils::sqlite3::rusqlite};

    let inbox_hash = MailboxHash::from_bytes(b"inbox");
    let db_dir = std::path::PathBuf::from(std::env::var_os("XDG_DATA_HOME").unwrap()).join("meli");
    std::fs::create_dir_all(&db_dir).unwrap();
    let conn =
        rusqlite::Connection::open(db_dir.join(format!("{account_name}_header_cache.db"))).unwrap();
    conn.pragma_update(None, "user_version", 5_u32).unwrap();
    conn.execute_batch(
        "PRAGMA foreign_keys = true;
        PRAGMA encoding = 'UTF-8';

        CREATE TABLE IF NOT EXISTS envelopes (
                        hash             INTEGER NOT NULL,
                        mailbox_hash     INTEGER NOT NULL,
                        uid              INTEGER NOT NULL,
                        modsequence      INTEGER,
                        envelope         BLOB NOT NULL,
                        PRIMARY KEY (mailbox_hash, uid),
                        FOREIGN KEY (mailbox_hash) REFERENCES mailbox(mailbox_hash) ON DELETE \
                         CASCADE
                       );
        CREATE TABLE IF NOT EXISTS mailbox (
                    mailbox_hash     INTEGER UNIQUE,
                    uidvalidity      INTEGER,
                    max_uid          INTEGER,
                    flags            BLOB NOT NULL,
                    highestmodseq    INTEGER,
                    messages         INTEGER,
                    unseen           INTEGER,
                    uidnext          INTEGER,
                    PRIMARY KEY (mailbox_hash)
                   );
        CREATE TABLE IF NOT EXISTS mailbox_list (
                    id INTEGER PRIMARY KEY CHECK (id = 0),
                    payload BLOB NOT NULL
                   );
        CREATE INDEX IF NOT EXISTS envelope_uid_idx ON envelopes(mailbox_hash, uid ASC);
        CREATE INDEX IF NOT EXISTS envelope_idx ON envelopes(hash);
        CREATE INDEX IF NOT EXISTS mailbox_idx ON mailbox(mailbox_hash);",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO mailbox (mailbox_hash, uidvalidity, max_uid, flags, messages, unseen, \
         uidnext) VALUES (?1, 1, 2, X'', 2, 2, 3);",
        rusqlite::params![inbox_hash.0 as i64],
    )
    .unwrap();
    let payload = serde_json::to_vec(&[CachedImapMailbox {
        hash: inbox_hash,
        imap_path: "inbox".to_string(),
        path: "inbox".to_string(),
        name: "inbox".to_string(),
        parent: None,
        children: vec![],
        separator: b'/',
        usage: SpecialUsageMailbox::Normal,
        no_select: false,
        is_subscribed: true,
    }])
    .unwrap();
    conn.execute(
        "INSERT INTO mailbox_list (id, payload) VALUES (0, ?1);",
        rusqlite::params![payload],
    )
    .unwrap();
    for (uid, env) in mails {
        conn.execute(
            "INSERT INTO envelopes (hash, uid, mailbox_hash, modsequence, envelope) VALUES \
             (?1, ?2, ?3, NULL, ?4);",
            rusqlite::params![env.hash().0 as i64, *uid as i64, inbox_hash.0 as i64, env],
        )
        .unwrap();
    }
    inbox_hash
}

/// `AccountConf` for an IMAP account whose server is an unreachable local
/// port: `offline_cache` stays at its default (enabled), so the sqlite3
/// cache is the only reading surface.
fn dead_port_imap_account_conf(account_name: &str, dead_port: u16) -> crate::conf::AccountConf {
    let mut account_conf = crate::conf::AccountConf::default();
    account_conf.account.name = account_name.to_string();
    account_conf.account.root_mailbox = "inbox".to_string();
    account_conf.account.format = "imap".to_string();
    account_conf.account.identity = "username@example.com".to_string();
    for (key, value) in [
        ("server_hostname", "127.0.0.1".to_string()),
        ("server_port", dead_port.to_string()),
        ("server_username", "null".to_string()),
        ("server_password", "null".to_string()),
        ("use_tls", "false".to_string()),
    ] {
        account_conf.account.extra.insert(key.to_string(), value);
    }
    account_conf.conf.format = "imap".to_string();
    account_conf
}

/// Pump the job executor like the main loop does: route `JobFinished`
/// events to `account` and every other event to `component`, until `cond`
/// holds or the deadline passes. Returns the last `cond` value.
fn pump_cold_start(
    ctx: &mut crate::Context,
    account_hash: melib::backends::prelude::AccountHash,
    component: &mut dyn crate::components::Component,
    deadline: std::time::Instant,
    cond: impl Fn(&crate::Context) -> bool,
) -> bool {
    while std::time::Instant::now() < deadline {
        if cond(ctx) {
            return true;
        }
        // Route reply events produced by the component into it again, like
        // the main loop does.
        for _ in 0..8 {
            let replies = ctx.replies();
            if replies.is_empty() {
                break;
            }
            for mut event in replies {
                let _ = component.process_event(&mut event, ctx);
            }
        }
        if cond(ctx) {
            return true;
        }
        match ctx
            .receiver
            .recv_timeout(std::time::Duration::from_millis(100))
        {
            Ok(crate::ThreadEvent::JobFinished(job_id)) => {
                let _ = ctx
                    .accounts
                    .get_mut(&account_hash)
                    .expect("cold-start account present")
                    .process_event(&job_id);
            }
            Ok(crate::ThreadEvent::UIEvent(mut event)) => {
                let _ = component.process_event(&mut event, ctx);
            }
            Ok(_) => {}
            Err(crossbeam::channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam::channel::RecvTimeoutError::Disconnected) => {
                panic!("job executor channel disconnected before the cold-start jobs finished")
            }
        }
    }
    cond(ctx)
}

/// Cold start of a remote (IMAP) account against an unreachable server
/// must still show the cached mailbox list and the cached envelopes: the
/// listing must not sit on `offline: Attempting connection.` while the
/// connect attempt times out.
///
/// This pins the `Account` wiring end-to-end (no mock backend): the startup
/// `Mailboxes` job resolves from the sqlite3 offline cache
/// (`ImapType::mailboxes()` serves `mailbox_list` before any network
/// LIST/LSUB), and `load`'s `FetchStage::CacheFirst` serves the cached
/// envelopes without a connection, finishing gracefully when the resync
/// cannot connect instead of failing the mailbox.
#[test]
fn test_account_imap_offline_cold_start_serves_cached_mail() {
    use melib::backends::Backends;

    const ACCOUNT_NAME: &str = "imap-cold-start";
    // The process-wide shared home also fixes the XDG data directory the
    // sqlite3 cache path is derived from.
    let _home = crate::golden::shared_test_home();

    let mails = cold_start_cached_mails();
    let inbox_hash = seed_offline_imap_cache(ACCOUNT_NAME, &mails);

    let temp_dir = TempDir::new().unwrap();
    let mut ctx = crate::Context::new_mock(&temp_dir);
    // Drain the mock account's own startup events so they cannot be
    // mistaken for this account's jobs below.
    while ctx.receiver.try_recv().is_ok() {}

    let account_conf = dead_port_imap_account_conf(ACCOUNT_NAME, dead_port());
    let backends = Backends::new();
    let account_hash = melib::backends::prelude::AccountHash::from_bytes(ACCOUNT_NAME.as_bytes());
    let account = crate::accounts::Account::new(
        account_hash,
        ACCOUNT_NAME.to_string(),
        account_conf,
        &backends,
        ctx.main_loop_handler.clone(),
        BackendEventConsumer::new(Arc::new(|_, _| {})),
    )
    .unwrap();

    // Route the account into the context so `pump_cold_start` can feed its
    // jobs, exactly like `State` owns the accounts in the real app.
    ctx.accounts.insert(account_hash, account);

    // Both startup jobs are in flight: `is_online` (fails fast on the dead
    // port) and `Mailboxes` (must resolve from the offline cache, never the
    // network).
    let mailboxes_loaded = pump_cold_start(
        &mut ctx,
        account_hash,
        &mut NoopComponent,
        std::time::Instant::now() + std::time::Duration::from_secs(10),
        |ctx| !ctx.accounts[&account_hash].list_mailboxes().is_empty(),
    );
    assert!(
        mailboxes_loaded,
        "cold start against a dead server must load the mailbox list from the offline cache"
    );
    {
        let account = &ctx.accounts[&account_hash];
        assert_eq!(
            account.mailboxes_order,
            vec![inbox_hash],
            "the cached INBOX must be the only mailbox"
        );
        // The reading surface is up before any connection succeeded: the
        // account is not online, yet the mailbox is there.
        assert!(!account.is_online.is_true());
    }

    // Opening the mailbox (what `Listing::change_account` does on
    // `AccountStatusChange`) must serve the cached envelopes.
    let _ = ctx
        .accounts
        .get_mut(&account_hash)
        .unwrap()
        .load(inbox_hash, true);
    let fetch_done = pump_cold_start(
        &mut ctx,
        account_hash,
        &mut NoopComponent,
        std::time::Instant::now() + std::time::Duration::from_secs(10),
        |ctx| {
            matches!(
                ctx.accounts[&account_hash].mailbox_entries[&inbox_hash].status,
                MailboxStatus::Available | MailboxStatus::Failed(_)
            )
        },
    );
    assert!(
        fetch_done,
        "cold-start fetch against a dead server must terminate (cache served, resync skipped)"
    );
    {
        let account = &ctx.accounts[&account_hash];
        assert!(
            matches!(
                account.mailbox_entries[&inbox_hash].status,
                MailboxStatus::Available
            ),
            "the mailbox must not be Failed after serving the offline cache; status is {:?}",
            account.mailbox_entries[&inbox_hash].status
        );
        let env_hashes = account
            .collection
            .mailboxes
            .read()
            .unwrap()
            .get(&inbox_hash)
            .cloned()
            .unwrap_or_default();
        assert_eq!(
            env_hashes.len(),
            mails.len(),
            "both cached envelopes must be in the collection"
        );
        for (uid, env) in &mails {
            match account
                .collection
                .envelopes
                .read()
                .unwrap()
                .get(&env.hash())
            {
                Some(e) => assert_eq!(
                    e.subject(),
                    std::borrow::Cow::from(format!("cold start {uid}")),
                    "cached envelope uid {uid} must be present with its subject"
                ),
                None => panic!("cached envelope uid {uid} missing from the collection"),
            }
        }
    }
}

/// A component that ignores everything, for `pump_cold_start` runs that
/// only need the account side.
#[derive(Debug)]
struct NoopComponent;

impl std::fmt::Display for NoopComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "noop")
    }
}

impl crate::components::Component for NoopComponent {
    fn draw(
        &mut self,
        _: &mut crate::terminal::CellBuffer,
        _: crate::terminal::Area,
        _: &mut crate::Context,
    ) {
    }

    fn process_event(&mut self, _: &mut crate::types::UIEvent, _: &mut crate::Context) -> bool {
        false
    }

    fn is_dirty(&self) -> bool {
        false
    }

    fn set_dirty(&mut self, _: bool) {}

    fn id(&self) -> crate::components::ComponentId {
        crate::components::ComponentId::default()
    }
}

/// The same cold start one level up, through the real `Listing` component:
/// the first frame shows the offline placeholder (`offline: Attempting
/// connection.`), and once the startup `Mailboxes` job resolves from the
/// offline cache the listing must switch to the real mail list and render
/// the cached envelopes — without the server ever being reachable.
#[test]
fn test_listing_imap_offline_cold_start_shows_cached_mail() {
    use crate::components::Component;

    const ACCOUNT_NAME: &str = "imap-cold-start-ui";
    let _home = crate::golden::shared_test_home();

    let mails = cold_start_cached_mails();
    let _inbox_hash = seed_offline_imap_cache(ACCOUNT_NAME, &mails);

    let temp_dir = TempDir::new().unwrap();
    let mut ctx = crate::Context::new_mock(&temp_dir);
    // Make the offline IMAP account the only account, so the listing's
    // initial cursor points at it.
    let mock_hash = *ctx.accounts.iter().next().unwrap().0;
    ctx.accounts.shift_remove(&mock_hash);
    while ctx.receiver.try_recv().is_ok() {}

    let account_conf = dead_port_imap_account_conf(ACCOUNT_NAME, dead_port());
    let backends = melib::backends::Backends::new();
    let account_hash = melib::backends::prelude::AccountHash::from_bytes(ACCOUNT_NAME.as_bytes());
    let account = crate::accounts::Account::new(
        account_hash,
        ACCOUNT_NAME.to_string(),
        account_conf,
        &backends,
        ctx.main_loop_handler.clone(),
        BackendEventConsumer::new(Arc::new(|_, _| {})),
    )
    .unwrap();
    ctx.accounts.insert(account_hash, account);

    let mut listing = crate::mail::listing::Listing::new(&mut ctx);
    let mut screen = crate::golden::golden_screen(&ctx, 80, 24);
    let area = screen.area();
    listing.draw(screen.grid_mut(), area, &mut ctx);
    let first_frame = screen_text(screen.grid());
    assert!(
        first_frame.contains("offline: "),
        "the first cold-start frame must be the offline placeholder, got:\n{first_frame}"
    );

    let shown = pump_cold_start(
        &mut ctx,
        account_hash,
        &mut listing,
        std::time::Instant::now() + std::time::Duration::from_secs(15),
        |ctx| {
            ctx.accounts[&account_hash]
                .collection
                .mailboxes
                .read()
                .unwrap()
                .values()
                .any(|h| !h.is_empty())
        },
    );
    assert!(
        shown,
        "cold start against a dead server must fill the collection from the offline cache"
    );
    // Flush everything still in flight (e.g. the `MailboxUpdate` for the
    // fetch that filled the collection) and route it like the main loop
    // does, then redraw.
    while let Ok(event) = ctx.receiver.try_recv() {
        match event {
            crate::ThreadEvent::JobFinished(job_id) => {
                if let Some(account) = ctx.accounts.get_mut(&account_hash) {
                    let _ = account.process_event(&job_id);
                }
            }
            crate::ThreadEvent::UIEvent(mut event) => {
                let _ = listing.process_event(&mut event, &mut ctx);
            }
            _ => {}
        }
    }
    for _ in 0..8 {
        let replies = ctx.replies();
        if replies.is_empty() {
            break;
        }
        for mut event in replies {
            let _ = listing.process_event(&mut event, &mut ctx);
        }
    }
    listing.set_dirty(true);
    listing.draw(screen.grid_mut(), area, &mut ctx);
    let text = screen_text(screen.grid());
    // Width-independent and timezone-independent: the two cached mails
    // must render as listing rows. The row-leading date column is
    // visible at any terminal width, unlike the (truncatable) subject
    // column. It renders in the LOCAL timezone (see
    // `MailListingTrait::format_date` → `timestamp_to_string`), so the
    // expected wall-clock date has to be derived from the mail
    // timestamps instead of hard-coding the UTC date: on a host west of
    // UTC these mails render as 2025-12-31.
    let expected_dates: Vec<String> = mails
        .iter()
        .map(|(_, env)| {
            melib::utils::datetime::timestamp_to_string(env.date(), Some("%Y-%m-%d"), false)
        })
        .collect();
    assert!(
        text.lines()
            .filter(|line| expected_dates
                .iter()
                .any(|date| line.contains(date.as_str())))
            .count()
            >= 2,
        "both cached mails must be rendered after the offline cold start, got:\n{text}"
    );
    assert!(
        !text.contains("offline: "),
        "the offline placeholder must be gone once cached mail is shown"
    );
}

/// Concatenate every screen row into one string (rows separated by
/// newlines) so tests can assert on visible text.
fn screen_text(grid: &crate::terminal::CellBuffer) -> String {
    let mut out = String::new();
    for y in 0..grid.rows {
        for x in 0..grid.cols {
            out.push(grid[(x, y)].ch());
        }
        out.push('\n');
    }
    out
}

/// Full-tree variant over the black-hole server: the real component stack
/// (`StatusBar(Tabbed(Listing)))`, driven like the main loop — focus
/// left/right across the mail view, list and sidebar, interleaved with the
/// timer pulses and job events that the real loop delivers. Catches a
/// panic or a hang that only the full stack reproduces; the outer
/// `timeout` catches a hang.
#[test]
fn test_statusbar_tree_blackhole_focus_roundtrip() {
    use crate::components::Component;
    use crate::terminal::Key;
    use crate::types::UIEvent;

    const ACCOUNT_NAME: &str = "imap-blackhole-tree";
    let _home = crate::golden::shared_test_home();

    let blackhole = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = blackhole.local_addr().unwrap().port();

    let mails = cold_start_cached_mails();
    let _inbox_hash = seed_offline_imap_cache(ACCOUNT_NAME, &mails);

    let temp_dir = TempDir::new().unwrap();
    let mut ctx = crate::Context::new_mock(&temp_dir);
    let mock_hash = *ctx.accounts.iter().next().unwrap().0;
    ctx.accounts.shift_remove(&mock_hash);
    while ctx.receiver.try_recv().is_ok() {}

    let account_conf = dead_port_imap_account_conf(ACCOUNT_NAME, port);
    let backends = melib::backends::Backends::new();
    let account_hash = melib::backends::prelude::AccountHash::from_bytes(ACCOUNT_NAME.as_bytes());
    let account = crate::accounts::Account::new(
        account_hash,
        ACCOUNT_NAME.to_string(),
        account_conf,
        &backends,
        ctx.main_loop_handler.clone(),
        BackendEventConsumer::new(Arc::new(|_, _| {})),
    )
    .unwrap();
    ctx.accounts.insert(account_hash, account);

    let listing = crate::mail::listing::Listing::new(&mut ctx);
    let tabbed = crate::utilities::Tabbed::new(
        vec![
            Box::new(listing),
            Box::new(crate::contacts::list::ContactList::new(&ctx)),
        ],
        &ctx,
    );
    let mut status_bar = crate::utilities::StatusBar::new(&ctx, Box::new(tabbed));
    status_bar.realize(None, &mut ctx);
    let mut screen = crate::golden::golden_screen(&ctx, 80, 24);
    let area = screen.area();

    // The timer id of the listing's menu-scrollbar timer is not observable
    // here, so timer pulses are not delivered; jobs and replies are.
    let pump_tree = |status_bar: &mut crate::utilities::StatusBar,
                     ctx: &mut crate::Context,
                     deadline: std::time::Instant|
     -> bool {
        if std::time::Instant::now() >= deadline {
            return false;
        }
        for _ in 0..8 {
            let replies = ctx.replies();
            if replies.is_empty() {
                break;
            }
            for mut event in replies {
                let _ = status_bar.process_event(&mut event, ctx);
            }
        }
        match ctx
            .receiver
            .recv_timeout(std::time::Duration::from_millis(50))
        {
            Ok(crate::ThreadEvent::JobFinished(job_id)) => {
                if let Some(account) = ctx.accounts.get_mut(&account_hash) {
                    let _ = account.process_event(&job_id);
                }
            }
            Ok(crate::ThreadEvent::UIEvent(mut event)) => {
                let _ = status_bar.process_event(&mut event, ctx);
            }
            Ok(_) => {}
            Err(_) => {}
        }
        true
    };

    // Cold start.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        if std::time::Instant::now() > deadline {
            panic!("cold start did not settle");
        }
        if ctx.accounts[&account_hash]
            .collection
            .mailboxes
            .read()
            .unwrap()
            .values()
            .any(|h| !h.is_empty())
        {
            break;
        }
        assert!(pump_tree(&mut status_bar, &mut ctx, deadline));
    }
    status_bar.set_dirty(true);
    status_bar.draw(screen.grid_mut(), area, &mut ctx);
    eprintln!("[tree-focus] cold start drew");

    // Focus chain: right opens the mail view; left twice reaches the
    // sidebar; a few sidebar moves; back right. Between keys, deliver
    // whatever jobs/pulses arrived, and always redraw.
    let keys = [
        Key::Right,
        Key::Left,
        Key::Left,
        Key::Up,
        Key::Down,
        Key::Right,
        Key::Right,
        Key::Left,
    ];
    for (i, key) in keys.into_iter().enumerate() {
        let key_desc = format!("{key:?}");
        let mut event = UIEvent::Input(key);
        let _ = status_bar.process_event(&mut event, &mut ctx);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        assert!(pump_tree(&mut status_bar, &mut ctx, deadline));
        status_bar.set_dirty(true);
        status_bar.draw(screen.grid_mut(), area, &mut ctx);
        eprintln!("[tree-focus] key {i} ({key_desc}) drew");
    }
}

/// Register `INBOX` and `Archive` on the single mock account and rebuild its
/// mailbox tree/order, so `Listing::new` snapshots two sidebar entries (the
/// sidebar order is `INBOX` then `Archive`).
fn register_listing_mailboxes(
    context: &mut crate::Context,
) -> (
    melib::backends::prelude::AccountHash,
    MailboxHash,
    MailboxHash,
) {
    let account_hash = *context.accounts.iter().next().unwrap().0;
    let account = context.accounts.get_mut(&account_hash).unwrap();
    for name in ["INBOX", "Archive"] {
        let mailbox_hash = MailboxHash::from_bytes(name.as_bytes());
        account.mailbox_entries.insert(
            mailbox_hash,
            MailboxEntry::new(
                MailboxStatus::Available,
                name.to_string(),
                synth_mailbox(name, SpecialUsageMailbox::Normal),
                FileMailboxConf::default(),
            ),
        );
    }
    build_mailboxes_order(
        &mut account.tree,
        &account.mailbox_entries,
        &mut account.mailboxes_order,
    );
    (
        account_hash,
        MailboxHash::from_bytes(b"INBOX"),
        MailboxHash::from_bytes(b"Archive"),
    )
}

/// Build a sidebar mailbox row for the synthetic `Listing::accounts` used by
/// the cursor tests below.
fn mailbox_entry(name: &str) -> crate::mail::listing::MailboxMenuEntry {
    crate::mail::listing::MailboxMenuEntry {
        depth: 0,
        indentation: 0,
        has_sibling: false,
        visible: true,
        collapsed: false,
        mailbox_hash: MailboxHash::from_bytes(name.as_bytes()),
        index_style: None,
    }
}

/// Build a sidebar account row holding the named synthetic mailboxes.
fn account_entry(name: &str, mailboxes: &[&str]) -> crate::mail::listing::AccountMenuEntry {
    crate::mail::listing::AccountMenuEntry {
        name: name.to_string(),
        hash: melib::backends::prelude::AccountHash::from_bytes(name.as_bytes()),
        index: 0,
        entries: mailboxes.iter().map(|m| mailbox_entry(m)).collect(),
    }
}

/// Regression: while the focused account is offline, a background
/// `AccountStatusChange` (e.g. `"Attempting authentication."`) triggers
/// `change_account`. That reconcile must not snap the user's sidebar
/// selection back to the account's default mailbox (`INBOX`); the cursor and
/// the sidebar highlight must stay on the mailbox the user picked.
#[test]
fn listing_offline_status_change_keeps_selected_mailbox() {
    use crate::components::Component;

    let _home = crate::golden::shared_test_home();
    let temp_dir = TempDir::new().unwrap();
    let mut ctx = crate::Context::new_mock(&temp_dir);
    let (account_hash, _inbox_hash, archive_hash) = register_listing_mailboxes(&mut ctx);
    let mut listing = crate::mail::listing::Listing::new(&mut ctx);

    // Precondition: two sidebar entries, default mailbox is INBOX (index 0).
    assert_eq!(
        listing.accounts[0].entries.len(),
        2,
        "precondition: the mock account must expose INBOX and Archive"
    );
    // Park the cursor on the non-default Archive row (index 1) and force the
    // offline placeholder component, the exact state reached while a remote
    // account is trying to (re)connect. The sidebar cursor is on a second
    // account, as the default launch focus leaves it: sidebar navigation only
    // moves `menu_cursor_pos` until the user confirms the selection.
    listing.cursor_pos = CursorPos {
        account: 0,
        menu: MenuEntryCursor::Mailbox(1),
    };
    listing
        .accounts
        .push(account_entry("acct-b", &["b-inbox", "b-archive"]));
    listing.menu_cursor_pos = CursorPos {
        account: 1,
        menu: MenuEntryCursor::Mailbox(0),
    };
    listing.component =
        ListingComponent::Offline(OfflineListing::new((account_hash, archive_hash)));

    let _ = listing.process_event(
        &mut UIEvent::AccountStatusChange(account_hash, Some("Attempting authentication.".into())),
        &mut ctx,
    );

    assert_eq!(
        listing.cursor_pos.menu,
        MenuEntryCursor::Mailbox(1),
        "a background status reconcile must not snap the cursor to the default mailbox"
    );
    assert_eq!(
        listing.menu_cursor_pos,
        CursorPos {
            account: 1,
            menu: MenuEntryCursor::Mailbox(0),
        },
        "a background status reconcile must not move the user's sidebar highlight"
    );
}

/// `menu_step_prev`/`menu_step_next` walk the whole sidebar column: within an
/// account `Status` and its mailboxes, then across account boundaries. This
/// is the logic behind `prev_mailbox`/`next_mailbox`.
#[test]
fn listing_menu_step_crosses_account_boundaries() {
    let _home = crate::golden::shared_test_home();
    let temp_dir = TempDir::new().unwrap();
    let mut ctx = crate::Context::new_mock(&temp_dir);
    let mut listing = crate::mail::listing::Listing::new(&mut ctx);
    // Deterministic sidebar shape: two accounts with two mailboxes each.
    listing.accounts = vec![
        account_entry("acct-a", &["a-inbox", "a-archive"]),
        account_entry("acct-b", &["b-inbox", "b-archive"]),
    ];

    // `Mailbox(0)` steps up to its own account's `Status` row.
    let mut cursor = CursorPos {
        account: 0,
        menu: MenuEntryCursor::Mailbox(0),
    };
    assert!(listing.menu_step_prev(&mut cursor));
    assert_eq!(cursor.account, 0);
    assert_eq!(cursor.menu, MenuEntryCursor::Status);

    // Account 0's `Status` is the top of the sidebar: prev is a no-op.
    assert!(!listing.menu_step_prev(&mut cursor));
    assert_eq!(cursor.account, 0);
    assert_eq!(cursor.menu, MenuEntryCursor::Status);

    // A later account's `Status` steps up to the previous account's last
    // mailbox.
    cursor = CursorPos {
        account: 1,
        menu: MenuEntryCursor::Status,
    };
    assert!(listing.menu_step_prev(&mut cursor));
    assert_eq!(cursor.account, 0);
    assert_eq!(cursor.menu, MenuEntryCursor::Mailbox(1));

    // `Status` steps down into the account's first mailbox.
    cursor = CursorPos {
        account: 0,
        menu: MenuEntryCursor::Status,
    };
    assert!(listing.menu_step_next(&mut cursor));
    assert_eq!(cursor.account, 0);
    assert_eq!(cursor.menu, MenuEntryCursor::Mailbox(0));

    // The last mailbox of an account crosses into the next account's
    // `Status` row.
    cursor = CursorPos {
        account: 0,
        menu: MenuEntryCursor::Mailbox(1),
    };
    assert!(listing.menu_step_next(&mut cursor));
    assert_eq!(cursor.account, 1);
    assert_eq!(cursor.menu, MenuEntryCursor::Status);

    // The last account's last row is the bottom of the sidebar: next is a
    // no-op.
    cursor = CursorPos {
        account: 1,
        menu: MenuEntryCursor::Mailbox(1),
    };
    assert!(!listing.menu_step_next(&mut cursor));
    assert_eq!(cursor.account, 1);
    assert_eq!(cursor.menu, MenuEntryCursor::Mailbox(1));
}
