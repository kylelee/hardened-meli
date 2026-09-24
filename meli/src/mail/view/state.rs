/*
 * meli
 *
 * Copyright 2017 Manos Pitsidianakis
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

use melib::{text::Truncate, Envelope, Error, Mail, Result};

use super::{EnvelopeView, MailView, ViewSettings};
use crate::{
    jobs::JoinHandle, mailbox_settings, Component, Context, ShortcutMaps, ThreadEvent, UIEvent,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PendingReplyAction {
    Reply,
    ReplyToAuthor,
    ReplyToAll,
    ForwardAttachment,
    ForwardInline,
}

#[derive(Debug)]
pub enum MailViewState {
    Init {
        pending_action: Option<PendingReplyAction>,
    },
    LoadingBody {
        main_loop_handler: crate::MainLoopHandler,
        handle: JoinHandle<Result<Vec<u8>>>,
        pending_action: Option<PendingReplyAction>,
    },
    Error {
        err: Error,
    },
    Loaded {
        bytes: Vec<u8>,
        env: Box<Envelope>,
        env_view: Box<EnvelopeView>,
        stack: Vec<Box<dyn Component>>,
    },
}

impl Drop for MailViewState {
    fn drop(&mut self) {
        let Self::LoadingBody {
            handle,
            main_loop_handler,
            ..
        } = self
        else {
            return;
        };
        let Some(ev) = handle.cancel() else {
            return;
        };
        main_loop_handler.send(ThreadEvent::UIEvent(UIEvent::StatusEvent(ev)));
    }
}

impl std::fmt::Display for MailViewState {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Init { .. } | Self::LoadingBody { .. } => {
                write!(fmt, "loading")
            }
            Self::Error { err } => {
                let err = err.to_string();
                write!(fmt, "{}", err.trim_at_boundary(8))
            }
            Self::Loaded { env, .. } => {
                write!(fmt, "{}", env.subject().as_ref().trim_at_boundary(8))
            }
        }
    }
}

impl MailViewState {
    pub fn load_bytes(self_: &mut MailView, bytes: Vec<u8>, context: &mut Context) {
        #[cfg(debug_assertions)]
        let __span = crate::state::DrawSpan::enter("MailViewState::load_bytes");
        melib::log::debug!(
            "load_bytes: acquiring envelope write lock ({} bytes)",
            bytes.len()
        );
        let Some(coordinates) = self_.coordinates else {
            return;
        };
        let account = &mut context.accounts[&coordinates.0];
        // Ensure all envelope headers are populated, because the email backend might
        // have not populated them all.
        let Some(mut env_ref) = account.collection.get_env_mut(coordinates.2) else {
            // The envelope was removed while its body was being fetched: skip
            // loading rather than fabricating a blank message.
            melib::log::error!(
                "Could not load email body: envelope {} is no longer in the mailbox",
                coordinates.2
            );
            return;
        };
        melib::log::debug!("load_bytes: write lock acquired; populating headers");
        _ = env_ref.populate_headers(&bytes);
        drop(env_ref);
        let Some(env_ref) = account.collection.get_env(coordinates.2) else {
            melib::log::error!(
                "Could not load email body: envelope {} is no longer in the mailbox",
                coordinates.2
            );
            return;
        };
        let env = Box::new(env_ref.clone());
        drop(env_ref);
        melib::log::debug!("load_bytes: constructing EnvelopeView");
        let env_view = Box::new(EnvelopeView::new(
            Mail {
                envelope: *env.clone(),
                bytes: bytes.clone(),
            },
            None,
            None,
            Some(ViewSettings {
                theme_default: crate::conf::value(context, "theme_default"),
                body_theme: crate::conf::value(context, "mail.view.body"),
                env_view_shortcuts: mailbox_settings!(
                    context[coordinates.0][&coordinates.1]
                        .shortcuts
                        .envelope_view
                )
                .key_values(),
                pager_filter: mailbox_settings!(
                    context[coordinates.0][&coordinates.1].pager.filter
                )
                .clone(),
                html_filter: mailbox_settings!(
                    context[coordinates.0][&coordinates.1].pager.html_filter
                )
                .clone(),
                url_launcher: mailbox_settings!(
                    context[coordinates.0][&coordinates.1].pager.url_launcher
                )
                .clone(),
                auto_choose_multipart_alternative: mailbox_settings!(
                    context[coordinates.0][&coordinates.1]
                        .pager
                        .auto_choose_multipart_alternative
                )
                .is_true(),
                expand_headers: false,
                sticky_headers: *mailbox_settings!(
                    context[coordinates.0][&coordinates.1].pager.sticky_headers
                ),
                show_date_in_my_timezone: mailbox_settings!(
                    context[coordinates.0][&coordinates.1]
                        .pager
                        .show_date_in_my_timezone
                )
                .is_true(),
                show_extra_headers: mailbox_settings!(
                    context[coordinates.0][&coordinates.1]
                        .pager
                        .show_extra_headers
                )
                .clone(),
                auto_verify_signatures: *mailbox_settings!(
                    context[coordinates.0][&coordinates.1]
                        .pgp
                        .auto_verify_signatures
                ),
                auto_decrypt: *mailbox_settings!(
                    context[coordinates.0][&coordinates.1].pgp.auto_decrypt
                ),
                charset: None,
            }),
            context.main_loop_handler.clone(),
        ));
        self_.state = Self::Loaded {
            env,
            bytes,
            env_view,
            stack: vec![],
        };
    }

    pub fn is_dirty(&self) -> bool {
        matches!(self, Self::Loaded { ref env_view, .. } if env_view.is_dirty())
    }

    pub fn set_dirty(&mut self, dirty: bool) {
        if let Self::Loaded {
            ref mut env_view, ..
        } = self
        {
            env_view.set_dirty(dirty);
        }
    }

    pub fn shortcuts(&self, context: &Context) -> ShortcutMaps {
        if let Self::Loaded { ref env_view, .. } = self {
            env_view.shortcuts(context)
        } else {
            ShortcutMaps::default()
        }
    }

    pub fn process_event(&mut self, event: &mut UIEvent, context: &mut Context) -> bool {
        if let Self::Loaded {
            ref mut env_view, ..
        } = self
        {
            env_view.process_event(event, context)
        } else {
            false
        }
    }

    pub(crate) fn has_active_modal(&self) -> bool {
        matches!(self, Self::Loaded { ref env_view, .. } if env_view.has_active_modal())
    }

    /// Whether the mail body has finished loading into this view. Used by
    /// `ThreadView`'s key interception gate: while not `Loaded`, vertical
    /// scroll keys still bubble up to drive the thread list, preserving the
    /// ability to browse while loading.
    pub(crate) fn is_loaded(&self) -> bool {
        matches!(self, Self::Loaded { .. })
    }
}

impl Default for MailViewState {
    fn default() -> Self {
        Self::Init {
            pending_action: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{IsAsync, JobExecutor};

    fn loaded_state() -> MailViewState {
        let bytes = b"From: a@b.example\r\nTo: c@d.example\r\nSubject: s\r\nMessage-ID: \
                      <state-test-1@x.example>\r\nDate: Thu, 1 Jan 2026 00:00:00 +0000\r\n\
                      Content-Type: text/plain; charset=utf-8\r\n\r\nhello\r\n"
            .to_vec();
        let mail = Mail::new(bytes.clone(), None).expect("could not parse test mail");
        let (sender, _receiver) = crossbeam::channel::unbounded();
        let handler = crate::MainLoopHandler {
            sender,
            job_executor: std::sync::Arc::new(JobExecutor::new(crossbeam::channel::unbounded().0)),
        };
        let env_view = Box::new(EnvelopeView::new(
            Mail {
                envelope: mail.envelope.clone(),
                bytes: bytes.clone(),
            },
            None,
            None,
            None,
            handler,
        ));
        MailViewState::Loaded {
            bytes,
            env: Box::new(mail.envelope),
            env_view,
            stack: vec![],
        }
    }

    #[test]
    fn test_mail_view_state_is_loaded_truth_table() {
        assert!(!MailViewState::default().is_loaded());
        assert!(!MailViewState::Error {
            err: Error::new("err")
        }
        .is_loaded());
        // `LoadingBody` needs a spawned `JoinHandle`; spawn a trivial future
        // on a throwaway executor to cover it cheaply.
        let executor = JobExecutor::new(crossbeam::channel::unbounded().0);
        let handle = executor.spawn(
            std::borrow::Cow::Borrowed("test"),
            async { Ok(Vec::new()) },
            IsAsync::Async,
        );
        assert!(!MailViewState::LoadingBody {
            main_loop_handler: crate::MainLoopHandler {
                sender: crossbeam::channel::unbounded().0,
                job_executor: std::sync::Arc::new(executor),
            },
            handle,
            pending_action: None,
        }
        .is_loaded());
        assert!(loaded_state().is_loaded());
    }

    #[test]
    fn test_mail_view_is_loaded_forwards_state() {
        let (sender, _receiver) = crossbeam::channel::unbounded();
        let main_loop_handler = crate::MainLoopHandler {
            sender,
            job_executor: std::sync::Arc::new(JobExecutor::new(crossbeam::channel::unbounded().0)),
        };
        let view = MailView {
            coordinates: None,
            dirty: true,
            contact_selector: None,
            forward_dialog: None,
            unsubscribe_dialog: None,
            pending_unsubscribe: None,
            theme_default: Default::default(),
            pane_fill: None,
            active_jobs: Default::default(),
            initialized: false,
            state: loaded_state(),
            main_loop_handler,
            id: Default::default(),
        };
        assert!(view.is_loaded());
    }
}
