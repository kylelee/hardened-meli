/*
 * meli
 *
 * Copyright 2017-2018 Manos Pitsidianakis
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

use std::{
    collections::HashSet,
    convert::TryFrom,
    io::Write,
    process::{Command, Stdio},
};

use indexmap::IndexSet;
use melib::{
    email::attachment_types::ContentType, list_management, mailto::Mailto, parser::BytesExt, Card,
    Draft, FlagOp, HeaderName, SpecialUsageMailbox,
};
use smallvec::SmallVec;

use super::*;
use crate::{accounts::JobRequest, jobs::JobId};

mod utils;
pub use utils::*;

mod thread;
pub use thread::*;
mod types;
pub use types::*;
pub mod state;
use state::*;

pub mod envelope;
pub use envelope::EnvelopeView;

pub mod filters;
pub use filters::*;

pub mod html_render;

#[cfg(test)]
mod tests;

/// What a `List-Unsubscribe` action will do once the user confirms it.
///
/// The target shown in the confirmation dialog is derived from the exact same
/// parsed values that will be used when the action is performed, so what the
/// user sees is what will be sent/opened.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UnsubscribeAction {
    /// Send an e-mail built from this `mailto:` URI.
    Send(Mailto),
    /// Open this URL with the system url launcher.
    OpenUrl(String),
}

impl UnsubscribeAction {
    /// The exact target that will be used if this action is confirmed.
    pub fn target_description(&self) -> String {
        match self {
            Self::Send(mailto) => mailto
                .address
                .iter()
                .map(|a| a.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            Self::OpenUrl(url) => url.clone(),
        }
    }
}

/// Pick the `List-Unsubscribe` option that would be performed, if any.
///
/// Mirrors the historical dispatch order: the first `mailto:` option that
/// parses wins, otherwise the first URL option; unparseable options are
/// skipped.
pub fn unsubscribe_action(
    unsubscribe: &[list_management::ListAction<'_>],
) -> Option<UnsubscribeAction> {
    for option in unsubscribe.iter() {
        match option {
            list_management::ListAction::Email(email) => {
                if let Ok(mailto) = Mailto::try_from(*email) {
                    return Some(UnsubscribeAction::Send(mailto));
                }
            }
            list_management::ListAction::Url(url) => {
                return Some(UnsubscribeAction::OpenUrl(
                    String::from_utf8_lossy(url).into_owned(),
                ));
            }
            list_management::ListAction::No => {}
        }
    }
    None
}

/// Contains an Envelope view, with sticky headers, a pager for the body, and
/// subviews for more menus
#[derive(Debug)]
pub struct MailView {
    coordinates: Option<(AccountHash, MailboxHash, EnvelopeHash)>,
    dirty: bool,
    contact_selector: Option<Box<UIDialog<Card>>>,
    forward_dialog: Option<Box<UIDialog<Option<PendingReplyAction>>>>,
    unsubscribe_dialog: Option<Box<UIConfirmationDialog>>,
    pending_unsubscribe: Option<UnsubscribeAction>,
    theme_default: ThemeAttribute,
    active_jobs: HashSet<JobId>,
    initialized: bool,
    state: MailViewState,
    main_loop_handler: MainLoopHandler,
    id: ComponentId,
}

impl std::fmt::Display for MailView {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.state.fmt(f)
    }
}

impl Drop for MailView {
    fn drop(&mut self) {
        if let MailViewState::LoadingBody { ref mut handle, .. } = self.state {
            if let Some(canceled) = handle.cancel() {
                self.main_loop_handler
                    .send(UIEvent::StatusEvent(canceled).into());
            }
        }
    }
}

impl MailView {
    pub fn new(
        coordinates: Option<(AccountHash, MailboxHash, EnvelopeHash)>,
        initialize_now: bool,
        context: &mut Context,
    ) -> Self {
        let mut ret = Self {
            coordinates,
            dirty: true,
            contact_selector: None,
            forward_dialog: None,
            unsubscribe_dialog: None,
            pending_unsubscribe: None,
            theme_default: crate::conf::value(context, "mail.view.body"),
            active_jobs: Default::default(),
            initialized: false,
            state: MailViewState::default(),
            main_loop_handler: context.main_loop_handler.clone(),
            id: ComponentId::default(),
        };

        if initialize_now {
            ret.init_futures(context);
        }
        ret
    }

    pub(crate) fn has_active_modal(&self) -> bool {
        self.contact_selector.is_some()
            || self.forward_dialog.is_some()
            || self.unsubscribe_dialog.is_some()
            || self.state.has_active_modal()
    }

    /// Whether the mail body has finished loading; see
    /// [`MailViewState::is_loaded`].
    pub(crate) fn is_loaded(&self) -> bool {
        self.state.is_loaded()
    }

    /// Bridge across the module-private `state` field for tests in sibling
    /// modules: opens the force charset selector inside a `Loaded` mail view.
    #[cfg(test)]
    pub(crate) fn open_force_charset_modal_for_tests(&mut self, context: &Context) {
        if let MailViewState::Loaded {
            ref mut env_view, ..
        } = self.state
        {
            env_view.set_force_charset_modal_for_tests(context);
        }
    }

    fn init_futures(&mut self, context: &mut Context) {
        log::trace!("MailView::init_futures");
        #[cfg(debug_assertions)]
        let __span = crate::state::DrawSpan::enter("MailView::init_futures");
        self.theme_default = crate::conf::value(context, "mail.view.body");
        let mut pending_action = None;
        let Some(coordinates) = self.coordinates else {
            log::debug!("init_futures: no coordinates");
            return;
        };
        let account = &mut context.accounts[&coordinates.0];
        if account.contains_key(coordinates.2) {
            {
                log::debug!("init_futures: requesting envelope bytes");
                match account.envelope_bytes_by_hash(coordinates.2) {
                    Ok(fut) => {
                        log::debug!("init_futures: spawning fetch-envelope");
                        let mut handle = account.main_loop_handler.job_executor.spawn(
                            "fetch-envelope".into(),
                            fut,
                            account.is_async(),
                        );
                        log::debug!("init_futures: fetch-envelope spawned, waiting up to 3ms");
                        let job_id = handle.job_id;
                        pending_action = if let MailViewState::Init {
                            ref mut pending_action,
                        } = self.state
                        {
                            pending_action.take()
                        } else {
                            None
                        };
                        #[cfg(debug_assertions)]
                        let got_bytes = if let Ok(Some(bytes_result)) =
                            try_recv_timeout!(&mut handle.chan)
                        {
                            log::debug!("init_futures: fetch-envelope completed synchronously");
                            match bytes_result {
                                Ok(bytes) => {
                                    log::debug!(
                                        "init_futures: load_bytes begin ({} bytes)",
                                        bytes.len()
                                    );
                                    MailViewState::load_bytes(self, bytes, context);
                                    log::debug!("init_futures: load_bytes done");
                                }
                                Err(err) => {
                                    log::debug!("init_futures: fetch-envelope errored: {err}");
                                    self.state = MailViewState::Error { err };
                                }
                            }
                            true
                        } else {
                            log::debug!("init_futures: fetch-envelope still running; LoadingBody");
                            self.state = MailViewState::LoadingBody {
                                main_loop_handler: self.main_loop_handler.clone(),
                                handle,
                                pending_action: pending_action.take(),
                            };
                            self.active_jobs.insert(job_id);
                            context
                                .replies
                                .push_back(UIEvent::StatusEvent(StatusEvent::NewJob(job_id)));
                            false
                        };
                        #[cfg(debug_assertions)]
                        let _ = got_bytes;
                    }
                    Err(err) => {
                        context.replies.push_back(UIEvent::Notification {
                            title: Some("Could not get message".into()),
                            source: None,
                            body: err.to_string().into(),
                            kind: Some(NotificationType::Error(err.kind)),
                        });
                    }
                }
            }
        }
        if let Some(p) = pending_action {
            self.perform_action(p, context);
        }
        self.initialized = true;
    }

    fn perform_action(&mut self, action: PendingReplyAction, context: &mut Context) {
        let Some(coordinates) = self.coordinates else {
            return;
        };
        let (bytes, reply_body, env) = match self.state {
            MailViewState::Init {
                ref mut pending_action,
                ..
            }
            | MailViewState::LoadingBody {
                ref mut pending_action,
                ..
            } => {
                *pending_action = Some(action);
                return;
            }
            MailViewState::Loaded {
                ref bytes,
                ref env,
                ref env_view,
                ..
            } => (bytes, env_view.body_text(), env),
            MailViewState::Error { .. } => {
                return;
            }
        };
        let composer = match action {
            PendingReplyAction::Reply => {
                Composer::reply_to_select(coordinates, reply_body.to_string(), context)
            }
            PendingReplyAction::ReplyToAuthor => {
                Composer::reply_to_author(coordinates, reply_body.to_string(), context)
            }
            PendingReplyAction::ReplyToAll => {
                Composer::reply_to_all(coordinates, reply_body.to_string(), context)
            }
            PendingReplyAction::ForwardAttachment => {
                Ok(Composer::forward(coordinates, bytes, env, true, context))
            }
            PendingReplyAction::ForwardInline => {
                Ok(Composer::forward(coordinates, bytes, env, false, context))
            }
        };
        let composer = match composer {
            Ok(composer) => Box::new(composer),
            Err(err) => {
                let kind = err.kind;
                let err_string = format!(
                    "Could not open reply: envelope {} is no longer available: {err}",
                    coordinates.2
                );
                log::error!("{err_string}");
                context.replies.push_back(UIEvent::Notification {
                    title: Some("Could not open reply".into()),
                    source: Some(err),
                    body: err_string.into(),
                    kind: Some(NotificationType::Error(kind)),
                });
                return;
            }
        };

        context
            .replies
            .push_back(UIEvent::Action(Tab(New(Some(composer)))));
    }

    pub fn update(
        &mut self,
        new_coordinates: (AccountHash, MailboxHash, EnvelopeHash),
        context: &mut Context,
    ) {
        if let MailViewState::LoadingBody { ref mut handle, .. } = self.state {
            if let Some(canceled) = handle.cancel() {
                context.replies.push_back(UIEvent::StatusEvent(canceled));
            }
        }
        if self.coordinates != Some(new_coordinates) {
            self.coordinates = Some(new_coordinates);
            self.init_futures(context);
            self.set_dirty(true);
        }
    }

    fn start_contact_selector(&mut self, context: &mut Context) {
        let Some(coordinates) = self.coordinates else {
            return;
        };
        let account = &context.accounts[&coordinates.0];
        // First retrieve user's identities, and remove them from the final address
        // list.
        let mut seen = {
            let extra = account.settings.account().extra_identity_addresses();
            let mut ret = IndexSet::with_capacity(extra.len() + 1);
            ret.extend(extra);
            ret.insert(account.settings.account().main_identity_address());
            ret
        };
        let Some(envelope) = account.collection.get_env(coordinates.2) else {
            context.replies.push_back(UIEvent::Notification {
                title: None,
                source: None,
                body: "Email not found".into(),
                kind: None,
            });
            return;
        };

        let mut entries: IndexMap<Card, (Card, String)> = IndexMap::default();
        for addr in envelope
            .from()
            .iter()
            .chain(envelope.to().iter())
            .chain(envelope.cc().iter())
        {
            if seen.contains(addr) {
                continue;
            }
            seen.insert(addr.clone());
            let mut new_card: Card = Card::new();
            new_card
                .set_email(addr.get_email().to_string())
                .set_id(addr.get_email().to_string().into());
            if let Some(display_name) = addr.get_display_name() {
                new_card.set_name(display_name.to_string());
            }
            entries.insert(new_card.clone(), (new_card, format!("{addr}")));
        }
        drop(envelope);
        self.contact_selector = Some(Box::new(Selector::new(
            "select contacts to add",
            entries.into_iter().map(|(_, v)| v).collect(),
            false,
            Some(Box::new(move |id: ComponentId, results: &[Card]| {
                Some(UIEvent::FinishedUIDialog(id, Box::new(results.to_vec())))
            })),
            context,
        )));
        self.dirty = true;
    }

    fn perform_unsubscribe_action(&self, action: UnsubscribeAction, context: &mut Context) {
        let Some(coordinates) = self.coordinates else {
            return;
        };
        match action {
            UnsubscribeAction::Send(mailto) => {
                let mut draft: Draft = mailto.into();
                draft.set_header(
                    HeaderName::FROM,
                    context.accounts[&coordinates.0]
                        .settings
                        .account()
                        .main_identity_address()
                        .to_string(),
                );
                if let Err(err) = super::compose::send_draft(
                    ToggleFlag::False,
                    context,
                    coordinates.0,
                    draft,
                    SpecialUsageMailbox::Sent,
                    Flag::SEEN,
                    true,
                ) {
                    context.replies.push_back(UIEvent::Notification {
                        title: Some("Couldn't send unsubscribe e-mail".into()),
                        source: None,
                        body: err.to_string().into(),
                        kind: Some(NotificationType::Error(err.kind)),
                    });
                }
            }
            UnsubscribeAction::OpenUrl(url_arg) => {
                let url_launcher =
                    mailbox_settings!(context[coordinates.0][&coordinates.1].pager.url_launcher)
                        .as_ref()
                        .map(|s| s.as_str())
                        .unwrap_or(if cfg!(target_os = "macos") {
                            "open"
                        } else {
                            "xdg-open"
                        });
                match Command::new(url_launcher)
                    .arg(&url_arg)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .spawn()
                {
                    Ok(child) => {
                        context
                            .children
                            .entry(url_launcher.to_string().into())
                            .or_default()
                            .push(ForkedProcess::Generic {
                                id: url_launcher.to_string().into(),
                                command: Some(format!("{url_launcher} {url_arg}").into()),
                                child,
                            });
                    }
                    Err(err) => {
                        context.replies.push_back(UIEvent::Notification {
                            title: Some(format!("Couldn't launch {url_launcher}").into()),
                            source: None,
                            body: err.to_string().into(),
                            kind: Some(NotificationType::Error(err.kind().into())),
                        });
                    }
                }
            }
        }
    }
}

impl Component for MailView {
    fn draw(&mut self, grid: &mut CellBuffer, area: Area, context: &mut Context) {
        #[cfg(debug_assertions)]
        let __draw_span = crate::state::DrawSpan::enter("MailView");
        if !self.is_dirty() {
            return;
        }
        let Some(coordinates) = self.coordinates else {
            return;
        };

        if !self.initialized {
            self.init_futures(context);
            return;
        }
        {
            let account = &context.accounts[&coordinates.0];
            if !account.contains_key(coordinates.2) {
                /* The envelope has been renamed or removed, so wait for the appropriate
                 * event to arrive */
                return;
            }
        }

        if let MailViewState::Loaded {
            ref mut env_view, ..
        } = self.state
        {
            {
                let account = &mut context.accounts[&coordinates.0];
                if account
                    .collection
                    .get_env(coordinates.2)
                    .is_some_and(|env| !env.is_seen())
                {
                    if let Err(err) = account.set_flags(
                        coordinates.2.into(),
                        coordinates.1,
                        vec![FlagOp::Set(Flag::SEEN)],
                    ) {
                        if !matches!(err.kind, ErrorKind::NotImplemented) {
                            context.replies.push_back(UIEvent::Notification {
                                title: Some("Could not set message as seen".into()),
                                source: None,
                                body: err.to_string().into(),
                                kind: Some(NotificationType::Error(err.kind)),
                            });
                        }
                    }
                }
            }
            env_view.draw(grid, area, context);
        } else if let MailViewState::Error { ref err } = self.state {
            grid.clear_area(area, self.theme_default);
            context.dirty_areas.push_back(area);
            context.replies.push_back(UIEvent::Notification {
                title: Some("Failed to open e-mail".into()),
                source: None,
                body: err.to_string().into(),
                kind: Some(NotificationType::Error(err.kind)),
            });
            log::error!("Failed to open envelope: {err}");
            if err.is_recoverable() {
                self.init_futures(context);
            }
            return;
        } else {
            grid.clear_area(area, self.theme_default);
            context.dirty_areas.push_back(area);
            return;
        };
        if let Some(ref mut s) = self.contact_selector.as_mut() {
            s.draw(grid, area, context);
        } else if let Some(ref mut s) = self.forward_dialog.as_mut() {
            s.draw(grid, area, context);
        } else if let Some(ref mut s) = self.unsubscribe_dialog.as_mut() {
            s.draw(grid, area, context);
        }

        self.dirty = false;
    }

    fn process_event(&mut self, mut event: &mut UIEvent, context: &mut Context) -> bool {
        if let Some(ref mut s) = self.contact_selector {
            // [ref:FIXME]: contact_selector should not forward navigation events and return true
            if s.process_event(event, context) {
                return true;
            }
        }

        if let Some(ref mut s) = self.forward_dialog {
            if s.process_event(event, context) {
                return true;
            }
        }

        if let Some(ref mut s) = self.unsubscribe_dialog {
            if s.process_event(event, context) {
                return true;
            }
        }

        let Some(coordinates) = self.coordinates else {
            return false;
        };
        if coordinates.0.is_null() || coordinates.1.is_null() {
            return false;
        }

        /* If envelope data is loaded, pass it to envelope views */
        if self.state.process_event(event, context) {
            return true;
        }

        if let Some(dialog_id) = self.unsubscribe_dialog.as_ref().map(|s| s.id()) {
            match event {
                UIEvent::FinishedUIDialog(id, result) if *id == dialog_id => {
                    self.unsubscribe_dialog = None;
                    let confirmed = result.downcast_ref::<bool>().copied().unwrap_or(false);
                    let action = self.pending_unsubscribe.take();
                    if confirmed {
                        if let Some(action) = action {
                            self.perform_unsubscribe_action(action, context);
                        }
                    }
                    self.set_dirty(true);
                    return true;
                }
                UIEvent::ComponentUnrealize(id) if *id == dialog_id => {
                    self.unsubscribe_dialog = None;
                    self.pending_unsubscribe = None;
                    self.set_dirty(true);
                    return true;
                }
                _ => {}
            }
        }

        match (
            &mut self.contact_selector,
            &mut self.forward_dialog,
            &mut event,
        ) {
            (Some(ref s), _, UIEvent::FinishedUIDialog(id, results)) if *id == s.id() => {
                if let Some(results) = results.downcast_ref::<Vec<Card>>() {
                    let account = &mut context.accounts[&coordinates.0];
                    {
                        for card in results.iter() {
                            account.contacts.add_card(card.clone());
                        }
                    }
                    self.contact_selector = None;
                }
                self.set_dirty(true);
                return true;
            }
            (_, Some(ref s), UIEvent::FinishedUIDialog(id, result)) if *id == s.id() => {
                if let Some(result) = result.downcast_ref::<Option<PendingReplyAction>>() {
                    self.forward_dialog = None;
                    if let Some(result) = *result {
                        self.perform_action(result, context);
                    }
                }
                self.set_dirty(true);
                return true;
            }
            _ => {}
        }
        match &event {
            UIEvent::StatusEvent(StatusEvent::JobFinished(ref job_id))
                if self.active_jobs.contains(job_id) =>
            {
                match self.state {
                    MailViewState::LoadingBody { ref mut handle, .. }
                        if handle.job_id == *job_id =>
                    {
                        match handle.chan.try_recv() {
                            Err(_) => { /* Job was canceled */ }
                            Ok(None) => { /* something happened, perhaps a worker
                                  * thread panicked */
                            }
                            Ok(Some(Ok(bytes))) => {
                                MailViewState::load_bytes(self, bytes, context);
                            }
                            Ok(Some(Err(err))) => {
                                self.state = MailViewState::Error { err };
                            }
                        }
                    }
                    MailViewState::Init { .. } => {
                        self.init_futures(context);
                    }
                    MailViewState::Loaded { .. } => {
                        log::debug!(
                            "MailView.active_jobs contains job id {:?} but MailViewState is \
                             already loaded; what job was this and why was it in active_jobs?",
                            job_id
                        );
                    }
                    _ => {}
                }
                self.active_jobs.remove(job_id);
                self.set_dirty(true);
            }
            _ => {}
        }

        let shortcuts = &self.shortcuts(context);
        match *event {
            UIEvent::ConfigReload { old_settings: _ } => {
                self.theme_default = crate::conf::value(context, "theme_default");
                self.set_dirty(true);
            }
            UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::ENVELOPE_VIEW]["reply"]) =>
            {
                self.perform_action(PendingReplyAction::Reply, context);
                return true;
            }
            UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::ENVELOPE_VIEW]["reply_to_all"]) =>
            {
                self.perform_action(PendingReplyAction::ReplyToAll, context);
                return true;
            }
            UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::ENVELOPE_VIEW]["reply_to_author"]) =>
            {
                self.perform_action(PendingReplyAction::ReplyToAuthor, context);
                return true;
            }
            UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::ENVELOPE_VIEW]["forward"]) =>
            {
                match mailbox_settings!(
                    context[coordinates.0][&coordinates.1]
                        .composing
                        .forward_as_attachment
                ) {
                    f if f.is_ask() => {
                        self.forward_dialog = Some(Box::new(UIDialog::new(
                            "How do you want the email to be forwarded?",
                            vec![
                                (
                                    Some(PendingReplyAction::ForwardInline),
                                    "inline".to_string(),
                                ),
                                (
                                    Some(PendingReplyAction::ForwardAttachment),
                                    "as attachment".to_string(),
                                ),
                            ],
                            true,
                            Some(Box::new(
                                move |id: ComponentId, result: &[Option<PendingReplyAction>]| {
                                    Some(UIEvent::FinishedUIDialog(
                                        id,
                                        Box::new(result.first().cloned().flatten()),
                                    ))
                                },
                            )),
                            context,
                        )));
                    }
                    f if f.is_true() => {
                        self.perform_action(PendingReplyAction::ForwardAttachment, context);
                    }
                    _ => {
                        self.perform_action(PendingReplyAction::ForwardInline, context);
                    }
                }
                return true;
            }
            UIEvent::FinishedUIDialog(id, ref result) if id == self.id() => {
                if let Some(result) = result.downcast_ref::<PendingReplyAction>() {
                    self.perform_action(*result, context);
                }
                return true;
            }
            UIEvent::Input(ref key)
                if shortcut!(key == shortcuts[Shortcuts::ENVELOPE_VIEW]["edit"]) =>
            {
                let account_hash = coordinates.0;
                let env_hash = coordinates.2;
                let (sender, mut receiver) = crate::jobs::oneshot::channel();
                let fut = context.accounts[&account_hash].envelope_bytes_by_hash(env_hash);
                let bytes_job = async move {
                    let _ = sender.send(fut?.await);
                    Ok(())
                };
                let handle = context.main_loop_handler.job_executor.spawn(
                    "fetch-envelope".into(),
                    bytes_job,
                    context.accounts[&account_hash].is_async(),
                );
                context.accounts[&account_hash].insert_job(
                    handle.job_id,
                    JobRequest::Generic {
                        name: "fetch envelope".into(),
                        handle,
                        on_finish: Some(CallbackFn(Box::new(move |context: &mut Context| {
                            match receiver.try_recv() {
                                Err(_) => { /* Job was canceled */ }
                                Ok(None) => { /* something happened, perhaps a worker
                                      * thread panicked */
                                }
                                Ok(Some(result)) => {
                                    match result.and_then(|bytes| {
                                        Composer::edit(account_hash, env_hash, &bytes, context)
                                    }) {
                                        Ok(composer) => {
                                            context.replies.push_back(UIEvent::Action(Tab(New(
                                                Some(Box::new(composer)),
                                            ))));
                                        }
                                        Err(err) => {
                                            let err_string = format!(
                                                "Failed to open envelope {:?}: {}",
                                                context.accounts[&account_hash]
                                                    .collection
                                                    .envelopes
                                                    .read()
                                                    .unwrap()
                                                    .get(&env_hash)
                                                    .map(|env| env.message_id()),
                                                err
                                            );
                                            log::error!("{err_string}");
                                            context.replies.push_back(UIEvent::Notification {
                                                title: Some("Failed to open e-mail".into()),
                                                source: None,
                                                body: err_string.into(),
                                                kind: Some(NotificationType::Error(err.kind)),
                                            });
                                        }
                                    }
                                }
                            }
                        }))),
                        log_level: LogLevel::DEBUG,
                    },
                );
                return true;
            }
            UIEvent::Action(View(ViewAction::AddAddressesToContacts)) => {
                self.start_contact_selector(context);
                return true;
            }
            UIEvent::Input(ref key)
                if self.contact_selector.is_none()
                    && shortcut!(
                        key == shortcuts[Shortcuts::ENVELOPE_VIEW]["add_addresses_to_contacts"]
                    ) =>
            {
                self.start_contact_selector(context);
                return true;
            }
            UIEvent::Input(Key::Esc) | UIEvent::Input(Key::Char('\x1b'))
                if self.contact_selector.is_some() || self.forward_dialog.is_some() =>
            {
                if let Some(s) = self.contact_selector.take() {
                    s.unrealize(context);
                }
                if let Some(s) = self.forward_dialog.take() {
                    s.unrealize(context);
                }
                self.set_dirty(true);
                return true;
            }
            UIEvent::EnvelopeRename(old_hash, new_hash) if coordinates.2 == old_hash => {
                self.coordinates.as_mut().unwrap().2 = new_hash;
            }
            UIEvent::Action(MailingListAction(ref e)) => {
                let account = &context.accounts[&coordinates.0];
                if !account.contains_key(coordinates.2) {
                    /* The envelope has been renamed or removed, so wait for the appropriate
                     * event to arrive */
                    return true;
                }
                let Some(envelope) = account.collection.get_env(coordinates.2) else {
                    /* The envelope has been renamed or removed, so wait for the
                     * appropriate event to arrive */
                    log::error!(
                        "Could not perform mailing list action: envelope {} no longer exists",
                        coordinates.2
                    );
                    return true;
                };
                let detect = list_management::ListActions::detect(&envelope);
                if let Some(ref actions) = detect {
                    match e {
                        MailingListAction::ListPost if actions.post.is_some() => {
                            /* open composer */
                            let mut failure = true;
                            // `post` can be `Some(empty)` for a malformed
                            // `List-Post` value: take the first entry
                            // fallibly instead of indexing `[0]`.
                            if let Some(list_management::ListAction::Email(list_post_addr)) =
                                actions.post.as_deref().and_then(|p| p.first())
                            {
                                if let Ok(mailto) = Mailto::try_from(*list_post_addr) {
                                    let draft: Draft = mailto.into();
                                    let mut composer =
                                        Composer::with_account(coordinates.0, context);
                                    composer.set_draft(draft, context);
                                    context.replies.push_back(UIEvent::Action(Tab(New(Some(
                                        Box::new(composer),
                                    )))));
                                    failure = false;
                                }
                            }
                            if failure {
                                context.replies.push_back(UIEvent::Notification {
                                    title: None,
                                    source: None,
                                    body: "Couldn't parse List-Post header value".into(),
                                    kind: None,
                                });
                            }
                            return true;
                        }
                        MailingListAction::ListUnsubscribe if actions.unsubscribe.is_some() => {
                            /* Ask for confirmation before proceeding with an action */
                            if let Some(action) = unsubscribe_action(
                                actions.unsubscribe.as_deref().unwrap_or_default(),
                            ) {
                                let entry =
                                    format!("List-Unsubscribe: {}", action.target_description());
                                self.pending_unsubscribe = Some(action);
                                self.unsubscribe_dialog =
                                    Some(Box::new(UIConfirmationDialog::new(
                                        "Confirm List-Unsubscribe action",
                                        vec![(true, entry)],
                                        /* only one choice */
                                        true,
                                        Some(Box::new(move |id: ComponentId, result: bool| {
                                            Some(UIEvent::FinishedUIDialog(id, Box::new(result)))
                                        })),
                                        context,
                                    )));
                                self.set_dirty(true);
                            }
                            return true;
                        }
                        MailingListAction::ListArchive if actions.archive.is_some() => {
                            /* open archive url with url_launcher */
                            let url_launcher = mailbox_settings!(
                                context[coordinates.0][&coordinates.1].pager.url_launcher
                            )
                            .as_ref()
                            .map(|s| s.as_str())
                            .unwrap_or(if cfg!(target_os = "macos") {
                                "open"
                            } else {
                                "xdg-open"
                            });
                            let url_arg = actions.archive.unwrap();
                            match Command::new(url_launcher)
                                .arg(url_arg)
                                .stdin(Stdio::piped())
                                .stdout(Stdio::piped())
                                .spawn()
                            {
                                Ok(child) => context
                                    .children
                                    .entry(url_launcher.to_string().into())
                                    .or_default()
                                    .push(ForkedProcess::Generic {
                                        id: url_launcher.to_string().into(),
                                        command: Some(format!("{url_launcher} {url_arg}").into()),
                                        child,
                                    }),
                                Err(err) => {
                                    context.replies.push_back(UIEvent::Notification {
                                        title: Some(
                                            format!("Couldn't launch {url_launcher}").into(),
                                        ),
                                        source: None,
                                        body: err.to_string().into(),
                                        kind: Some(NotificationType::Error(err.kind().into())),
                                    });
                                }
                            }
                            return true;
                        }
                        _ => { /* error print message to user */ }
                    }
                };
            }
            UIEvent::Action(Listing(OpenInNewTab)) => {
                let mut new_tab = Self::new(self.coordinates, true, context);
                new_tab.set_dirty(true);
                context
                    .replies
                    .push_back(UIEvent::Action(Tab(New(Some(Box::new(new_tab))))));
                return true;
            }
            UIEvent::Input(ref key)
                if mailbox_settings!(context has [coordinates.0][&coordinates.1])
                    && mailbox_settings!(
                        context[coordinates.0][&coordinates.1]
                            .shortcuts
                            .envelope_view
                            .commands
                    )
                    .iter()
                    .any(|cmd| {
                        if cmd.shortcut == *key {
                            for cmd in &cmd.command {
                                context.replies.push_back(UIEvent::Command(cmd.to_string()));
                            }
                            return true;
                        }
                        false
                    }) =>
            {
                return true;
            }
            _ => {}
        }
        false
    }

    fn is_dirty(&self) -> bool {
        self.dirty
            || self.state.is_dirty()
            || self
                .contact_selector
                .as_ref()
                .map(|s| s.is_dirty())
                .unwrap_or(false)
            || self
                .forward_dialog
                .as_ref()
                .map(|s| s.is_dirty())
                .unwrap_or(false)
            || self
                .unsubscribe_dialog
                .as_ref()
                .map(|s| s.is_dirty())
                .unwrap_or(false)
    }

    fn set_dirty(&mut self, value: bool) {
        self.dirty = value;
        if let Some(ref mut s) = self.contact_selector {
            s.set_dirty(value);
        } else if let Some(ref mut s) = self.forward_dialog {
            s.set_dirty(value);
        } else if let Some(ref mut s) = self.unsubscribe_dialog {
            s.set_dirty(value);
        }
        self.state.set_dirty(value);
    }

    fn shortcuts(&self, context: &Context) -> ShortcutMaps {
        let mut map = self.state.shortcuts(context);
        if let Some(envelope_view_map) = map.get_mut(Shortcuts::ENVELOPE_VIEW) {
            if let Some((account_hash, mailbox_hash, _)) = self.coordinates {
                if mailbox_settings!(context has [account_hash][&mailbox_hash]) {
                    for command in mailbox_settings!(
                        context[account_hash][&mailbox_hash]
                            .shortcuts
                            .envelope_view
                            .commands
                    ) {
                        // Shadow only the colliding key (see `Listing::shortcuts`).
                        envelope_view_map.retain(|_, shortcut| {
                            shortcut.0.retain(|k| k != &command.shortcut);
                            !shortcut.0.is_empty()
                        });
                    }
                }
            }
        }
        map
    }

    fn id(&self) -> ComponentId {
        self.id
    }

    fn kill(&mut self, id: ComponentId, context: &mut Context) {
        if self.id == id {
            context
                .replies
                .push_back(UIEvent::Action(Tab(Kill(self.id))));
        }
    }
}
