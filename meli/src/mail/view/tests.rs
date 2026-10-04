//
// meli
//
// Copyright 2023 Manos Pitsidianakis
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

use std::{borrow::Cow, path::Path};

use indexmap::IndexMap;

use super::{state::PendingReplyAction, MailView, MailViewTab, ThreadView, ThreadViewFocus};
use crate::{
    accounts::{MailboxEntry, MailboxStatus},
    command::{
        actions::{Action, ComposeAction, FileAction, ListingAction, TabAction, ViewAction},
        MailingListAction,
    },
    components::{Component, ComponentId, ComponentPath},
    conf::{composing::SendMail, FileMailboxConf},
    melib::{
        backends::{BackendMailbox, Mailbox, MailboxPermissions, SpecialUsageMailbox},
        Attachment, AttachmentBuilder, Envelope, Mail,
    },
    terminal::{Key, Screen, Virtual},
    types::{Link, LinkKind, UIEvent},
    utilities::{Tabbed, UIDialog},
    view::{
        AttachmentDisplay, EnvelopeView, ViewFilter, ViewFilterContent, ViewOptions, ViewSettings,
    },
    AccountHash, Context, EnvelopeHash, MailboxHash, StatusEvent,
};

/// Insert an envelope with a `List-Unsubscribe: <mailto:…>` header into the
/// mock account's collection and return the coordinates referring to it.
///
/// The mailbox hash is synthetic: the mock account's mailbox listing job has
/// not run (no event loop in tests) and the List-Unsubscribe e-mail path only
/// requires a non-null mailbox hash.
fn insert_list_unsubscribe_envelope(context: &Context) -> (AccountHash, MailboxHash, EnvelopeHash) {
    let account_hash = *context.accounts.iter().next().unwrap().0;
    let mailbox_hash = MailboxHash::from_bytes(b"INBOX");
    let bytes = b"From: newsletter@list.example\r\n\
To: victim@victim.example\r\n\
Subject: weekly\r\n\
Message-ID: <list-unsub-1@list.example>\r\n\
Date: Thu, 1 Jan 2026 00:00:00 +0000\r\n\
List-Unsubscribe: <mailto:unsubscribe@list.example?subject=bye>\r\n\
\r\n\
hello\r\n";
    let envelope = Envelope::from_bytes(bytes, None).expect("could not parse test envelope");
    let env_hash = envelope.hash();
    context.accounts[&account_hash]
        .collection
        .insert(envelope, mailbox_hash);
    (account_hash, mailbox_hash, env_hash)
}

/// Returns `true` if any reply evidences that the send path was invoked for
/// the unsubscribe e-mail. The send path is asynchronous (`send_draft_async`
/// spawns a job and pushes `NewJob`; the completion notification only lands
/// when the main loop processes the finished job), so the job spawn itself is
/// the deterministic evidence, next to any synchronous failure notification
/// (the mock account's `send_mail` is an empty command, which fails inside
/// the job).
fn has_send_evidence(replies: &[UIEvent]) -> bool {
    replies.iter().any(|ev| {
        matches!(ev, UIEvent::Notification { title: Some(t), .. } if t.to_lowercase().contains("unsubscribe"))
            || matches!(ev, UIEvent::StatusEvent(StatusEvent::NewJob(_)))
    })
}

/// Returns `true` if any reply indicates a draft message was written to disk
/// (`save_draft` pushes this when the send path stores/restores a message).
fn has_draft_artifact(replies: &[UIEvent]) -> bool {
    replies.iter().any(|ev| {
        matches!(ev, UIEvent::Notification { body, .. } if body.contains("Message was stored"))
    })
}

/// Shared HOME for the mock contexts below. Environment variables are
/// process-global, so parallel tests must not race each other by pointing
/// them at tempdirs that get deleted while another test constructs its
/// `Context` (which reads `MELI_CONFIG`/XDG vars).
fn shared_test_home() -> &'static tempfile::TempDir {
    static HOME: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    HOME.get_or_init(|| {
        let _env = crate::utilities::tests::env_lock_shared();
        let tempdir = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", tempdir.path());
        std::env::set_var("XDG_CONFIG_HOME", tempdir.path().join(".config"));
        std::env::set_var(
            "XDG_DATA_HOME",
            tempdir.path().join(".local").join(".share"),
        );
        tempdir
    })
}

fn mock_context() -> Context {
    // Retry: parallel suites (conf tests) also overwrite the process-global
    // `MELI_CONFIG`, which can make `Settings::new()` inside `new_mock` fail
    // spuriously.
    let mut ctx = None;
    for _ in 0..3 {
        if let Ok(candidate) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Context::new_mock(shared_test_home())
        })) {
            ctx = Some(candidate);
            break;
        }
    }
    let mut ctx = ctx.unwrap_or_else(|| Context::new_mock(shared_test_home()));
    // The default `send_mail` (`ShellCommand("false")`) races: the child can
    // exit before meli finishes writing the message to its stdin, panicking
    // with a broken pipe. An empty command makes `Account::send` return a
    // deterministic error without spawning anything.
    let account_hash = *ctx.accounts.iter().next().unwrap().0;
    ctx.accounts[&account_hash].settings.send_mail = SendMail::ShellCommand(String::new());
    ctx
}

fn trigger_list_unsubscribe(view: &mut MailView, context: &mut Context) {
    let mut event = UIEvent::Action(Action::MailingListAction(
        MailingListAction::ListUnsubscribe,
    ));
    _ = view.process_event(&mut event, context);
}

/// The List-Unsubscribe action must not send an e-mail without the user
/// confirming a dialog first. Security fix W1-T3 (C2 amplifier).
#[test]
fn list_unsubscribe_requires_confirmation_before_send() {
    let mut ctx = mock_context();
    let coordinates = insert_list_unsubscribe_envelope(&ctx);
    let mut view = MailView::new(Some(coordinates), false, &mut ctx);

    trigger_list_unsubscribe(&mut view, &mut ctx);
    let replies: Vec<UIEvent> = ctx.replies().into_iter().collect();
    assert!(
        !has_send_evidence(&replies),
        "List-Unsubscribe must not auto-send without user confirmation, got replies: {replies:?}"
    );
}

/// Confirm path: pressing Enter in the confirmation dialog must invoke the
/// send path exactly once, with the dialog torn down afterwards.
#[test]
fn list_unsubscribe_confirm_sends_after_dialog() {
    let mut ctx = mock_context();
    let coordinates = insert_list_unsubscribe_envelope(&ctx);
    let mut view = MailView::new(Some(coordinates), false, &mut ctx);

    trigger_list_unsubscribe(&mut view, &mut ctx);
    let dialog_id = view
        .unsubscribe_dialog
        .as_ref()
        .expect("confirmation dialog must open before any send")
        .id();
    let replies: Vec<UIEvent> = ctx.replies().into_iter().collect();
    assert!(!has_send_evidence(&replies));
    assert!(!has_draft_artifact(&replies));

    let mut event = UIEvent::Input(Key::Char('\n'));
    _ = view.process_event(&mut event, &mut ctx);
    let replies: Vec<UIEvent> = ctx.replies().into_iter().collect();
    assert!(
        replies
            .iter()
            .any(|ev| matches!(ev, UIEvent::FinishedUIDialog(id, _) if *id == dialog_id)),
        "confirming with Enter must emit FinishedUIDialog, got: {replies:?}"
    );

    // The main loop feeds component replies back into process_event.
    for mut ev in replies {
        _ = view.process_event(&mut ev, &mut ctx);
    }
    let replies: Vec<UIEvent> = ctx.replies().into_iter().collect();
    assert!(
        has_send_evidence(&replies),
        "send path must be invoked after confirmation, got: {replies:?}"
    );
    assert_eq!(
        replies
            .iter()
            .filter(|ev| matches!(ev, UIEvent::StatusEvent(StatusEvent::NewJob(_))))
            .count(),
        1,
        "confirming must spawn the unsubscribe send job exactly once, got: {replies:?}"
    );
    assert!(view.unsubscribe_dialog.is_none());
    assert!(view.pending_unsubscribe.is_none());
}

/// Cancel path: pressing Esc must tear the dialog down and leave no draft,
/// no send attempt and no pending action behind.
#[test]
fn list_unsubscribe_cancel_does_not_send() {
    let mut ctx = mock_context();
    let coordinates = insert_list_unsubscribe_envelope(&ctx);
    let mut view = MailView::new(Some(coordinates), false, &mut ctx);

    trigger_list_unsubscribe(&mut view, &mut ctx);
    assert!(view.unsubscribe_dialog.is_some());
    assert!(view.pending_unsubscribe.is_some());

    let mut event = UIEvent::Input(Key::Esc);
    _ = view.process_event(&mut event, &mut ctx);
    let replies: Vec<UIEvent> = ctx.replies().into_iter().collect();
    assert!(!has_send_evidence(&replies));
    assert!(!has_draft_artifact(&replies));
    let confirmed = replies.iter().any(|ev| match ev {
        UIEvent::FinishedUIDialog(_, result) => result.downcast_ref::<bool>() == Some(&true),
        _ => false,
    });
    assert!(!confirmed, "cancel must not emit a confirmation event");

    // The main loop feeds component replies (ComponentUnrealize) back in.
    for mut ev in replies {
        _ = view.process_event(&mut ev, &mut ctx);
    }
    let replies: Vec<UIEvent> = ctx.replies().into_iter().collect();
    assert!(!has_send_evidence(&replies));
    assert!(!has_draft_artifact(&replies));
    assert!(view.unsubscribe_dialog.is_none());
    assert!(view.pending_unsubscribe.is_none());
}

/// Quitting (`Esc`) while the add-to-contacts selector is open must consume
/// the key and tear the selector down once the main loop feeds the
/// `ComponentUnrealize` reply back. Regression for the embedded dialogs that
/// kept consuming `q`/`Esc` and trapped `MailView` until `:quit`.
#[test]
fn contact_selector_quit_key_closes_dialog() {
    let mut ctx = mock_context();
    let coordinates = insert_list_unsubscribe_envelope(&ctx);
    let mut view = MailView::new(Some(coordinates), false, &mut ctx);

    let mut event = UIEvent::Action(Action::View(ViewAction::AddAddressesToContacts));
    assert!(view.process_event(&mut event, &mut ctx));
    assert!(view.contact_selector.is_some());

    let mut event = UIEvent::Input(Key::Esc);
    assert!(
        view.process_event(&mut event, &mut ctx),
        "the contact selector must consume the quit key"
    );

    // The main loop feeds component replies (ComponentUnrealize) back in.
    let replies: Vec<UIEvent> = ctx.replies().into_iter().collect();
    assert!(
        replies
            .iter()
            .any(|ev| matches!(ev, UIEvent::ComponentUnrealize(_))),
        "quit must emit ComponentUnrealize, got: {replies:?}"
    );
    for mut ev in replies {
        _ = view.process_event(&mut ev, &mut ctx);
    }
    assert!(
        view.contact_selector.is_none(),
        "quit must clear the embedded contact selector"
    );
}

/// Same as [`contact_selector_quit_key_closes_dialog`] for the forward
/// dialog: `forward_as_attachment` defaults to `ask`, so the dialog is built
/// with both forwarding choices; quitting it must clear the field once the
/// `ComponentUnrealize` reply is re-dispatched.
#[test]
fn forward_dialog_quit_key_closes_dialog() {
    let mut ctx = mock_context();
    let coordinates = insert_list_unsubscribe_envelope(&ctx);
    let mut view = MailView::new(Some(coordinates), false, &mut ctx);
    view.forward_dialog = Some(Box::new(UIDialog::new(
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
        None,
        &ctx,
    )));
    assert!(view.forward_dialog.is_some());

    let mut event = UIEvent::Input(Key::Esc);
    assert!(
        view.process_event(&mut event, &mut ctx),
        "the forward dialog must consume the quit key"
    );

    let replies: Vec<UIEvent> = ctx.replies().into_iter().collect();
    for mut ev in replies {
        _ = view.process_event(&mut ev, &mut ctx);
    }
    assert!(
        view.forward_dialog.is_none(),
        "quit must clear the embedded forward dialog"
    );
}

/// The envelope-view `reply` binding moved from `R` to `r`, and the vacated
/// `R` now maps to `return_to_normal_view` (see `conf::shortcuts`). A Loaded
/// mail view exposes the envelope-view shortcut map, so `r` must run the
/// reply action path and `R` must no longer do so.
#[test]
fn reply_shortcut_is_r_not_shift_r() {
    let mut ctx = mock_context();
    let (account_hash, mailbox_hash, env_hash) = insert_list_unsubscribe_envelope(&ctx);
    let mut view = MailView::new(
        Some((account_hash, mailbox_hash, env_hash)),
        false,
        &mut ctx,
    );
    // `MailView::shortcuts` is empty until the body is `Loaded` (the state's
    // shortcut map delegates to the envelope view). The synthetic mailbox has
    // no per-mailbox settings, so `MailViewState::load_bytes` cannot be used
    // (it indexes the mailbox settings map); build the `Loaded` state directly
    // with default view settings, which install the same shortcut map the real
    // load path would.
    let bytes = b"From: newsletter@list.example\r\n\
                  To: victim@victim.example\r\n\
                  Subject: weekly\r\n\
                  Message-ID: <list-unsub-1@list.example>\r\n\
                  Date: Thu, 1 Jan 2026 00:00:00 +0000\r\n\
                  \r\n\
                  hello\r\n"
        .to_vec();
    let mail = Mail::new(bytes.clone(), None).expect("could not parse test mail");
    let env_view = Box::new(EnvelopeView::new(
        Mail {
            envelope: mail.envelope.clone(),
            bytes: bytes.clone(),
        },
        None,
        None,
        None,
        ctx.main_loop_handler.clone(),
    ));
    view.state = super::state::MailViewState::Loaded {
        bytes,
        env: Box::new(mail.envelope),
        env_view,
        stack: vec![],
    };
    assert!(
        view.is_loaded(),
        "precondition: the mail view must be Loaded"
    );

    // The reply action either opens a composer tab or, if the mock account
    // cannot build it, pushes the deterministic "Could not open reply" error;
    // both mean the reply path ran.
    fn reply_path(replies: &[UIEvent]) -> bool {
        replies.iter().any(|ev| match ev {
            UIEvent::Action(Action::Tab(TabAction::New(_))) => true,
            UIEvent::Notification { title: Some(t), .. } => t == "Could not open reply",
            _ => false,
        })
    }

    let mut event = UIEvent::Input(Key::Char('r'));
    assert!(
        view.process_event(&mut event, &mut ctx),
        "`r` must be consumed by the envelope-view reply binding"
    );
    let replies: Vec<UIEvent> = ctx.replies().into_iter().collect();
    assert!(
        reply_path(&replies),
        "`r` must run the reply action path, got replies: {replies:?}"
    );

    // `R` is the vacated reply key, now `return_to_normal_view`; in the plain
    // pager state (no sub-view, no filters) it must neither be consumed by
    // this view nor open a reply.
    let mut event = UIEvent::Input(Key::Char('R'));
    let consumed = view.process_event(&mut event, &mut ctx);
    let replies: Vec<UIEvent> = ctx.replies().into_iter().collect();
    assert!(!consumed, "`R` must not trigger the reply binding");
    assert!(
        !reply_path(&replies),
        "`R` must not run the reply action path, got replies: {replies:?}"
    );
}

/// Build a `Loaded` `MailView` for the synthetic INBOX envelope used by
/// [`insert_list_unsubscribe_envelope`]. The `Loaded` state is built directly
/// (as in [`reply_shortcut_is_r_not_shift_r`]) because the synthetic mailbox
/// has no per-mailbox settings entry for `MailViewState::load_bytes`.
fn loaded_mail_view(ctx: &mut Context) -> MailView {
    _ = register_inbox(ctx);
    let coordinates = insert_list_unsubscribe_envelope(ctx);
    let mut view = MailView::new(Some(coordinates), false, ctx);
    let bytes = b"From: newsletter@list.example\r\n\
                  To: victim@victim.example\r\n\
                  Subject: weekly\r\n\
                  Message-ID: <list-unsub-1@list.example>\r\n\
                  Date: Thu, 1 Jan 2026 00:00:00 +0000\r\n\
                  \r\n\
                  hello\r\n"
        .to_vec();
    let mail = Mail::new(bytes.clone(), None).expect("could not parse test mail");
    let env_view = Box::new(EnvelopeView::new(
        Mail {
            envelope: mail.envelope.clone(),
            bytes: bytes.clone(),
        },
        None,
        None,
        None,
        ctx.main_loop_handler.clone(),
    ));
    view.state = super::state::MailViewState::Loaded {
        bytes,
        env: Box::new(mail.envelope),
        env_view,
        stack: vec![],
    };
    view
}

/// `true` if the replies prove the reply/forward action path ran: either a
/// composer tab was opened or the mock account deterministically failed to
/// build it (the same acceptance criterion as `reply_path`).
fn compose_action_path(replies: &[UIEvent]) -> bool {
    replies.iter().any(|ev| match ev {
        UIEvent::Action(Action::Tab(TabAction::New(_))) => true,
        UIEvent::Notification { title: Some(t), .. } => t == "Could not open reply",
        _ => false,
    })
}

/// Push a compose action and assert the mail view consumes it and runs the
/// reply/forward path.
fn assert_compose_action_runs(action: ComposeAction) {
    let mut ctx = mock_context();
    let mut view = loaded_mail_view(&mut ctx);

    let mut event = UIEvent::Action(Action::Compose(action));
    assert!(
        view.process_event(&mut event, &mut ctx),
        "the compose action must be consumed by the mail view"
    );
    let replies: Vec<UIEvent> = ctx.replies().into_iter().collect();
    assert!(
        compose_action_path(&replies),
        "the compose action must run the reply/forward path, got replies: {replies:?}"
    );
}

/// `ComposeAction::Reply` from the command palette must run the same path as
/// the envelope-view `reply` binding.
#[test]
fn compose_action_reply_opens_composer() {
    assert_compose_action_runs(ComposeAction::Reply);
}

/// `ComposeAction::ReplyToAuthor` must run the reply-to-author path.
#[test]
fn compose_action_reply_to_author_opens_composer() {
    assert_compose_action_runs(ComposeAction::ReplyToAuthor);
}

/// `ComposeAction::ReplyToAll` must run the reply-all path.
#[test]
fn compose_action_reply_to_all_opens_composer() {
    assert_compose_action_runs(ComposeAction::ReplyToAll);
}

/// `ComposeAction::ForwardInline` must forward without an extra dialog when
/// chosen explicitly.
#[test]
fn compose_action_forward_inline_opens_composer() {
    assert_compose_action_runs(ComposeAction::ForwardInline);
}

/// `ComposeAction::ForwardAttachment` must forward as an attachment when
/// chosen explicitly.
#[test]
fn compose_action_forward_attachment_opens_composer() {
    assert_compose_action_runs(ComposeAction::ForwardAttachment);
}

/// `ComposeAction::Forward` must follow the exact same `forward` flow as the
/// `Ctrl-f` shortcut: with the default `composing.forward_as_attachment =
/// ask` setting both open the inline/as-attachment chooser instead of
/// forwarding directly.
#[test]
fn compose_action_forward_uses_shortcut_dialog_flow() {
    let mut ctx = mock_context();

    let mut action_view = loaded_mail_view(&mut ctx);
    let mut action_event = UIEvent::Action(Action::Compose(ComposeAction::Forward));
    assert!(
        action_view.process_event(&mut action_event, &mut ctx),
        "the forward action must be consumed"
    );
    assert!(
        action_view.forward_dialog.is_some(),
        "the default `ask` setting must open the inline/as-attachment dialog, got replies: {:?}",
        ctx.replies()
    );

    // The `Ctrl-f` shortcut must reach the same state through the shared
    // `perform_forward` helper.
    let mut key_view = loaded_mail_view(&mut ctx);
    let mut key_event = UIEvent::Input(Key::Ctrl('f'));
    assert!(
        key_view.process_event(&mut key_event, &mut ctx),
        "`Ctrl-f` must be consumed by the forward shortcut"
    );
    assert!(
        key_view.forward_dialog.is_some(),
        "`Ctrl-f` must open the same dialog"
    );
}

/// `MailViewState::load_bytes` must not fabricate a message when the envelope
/// it refers to has been removed from the collection while its body bytes were
/// being fetched: it skips the load instead of panicking on the new
/// `Option`-returning `Collection::get_env`/`get_env_mut`.
#[test]
fn load_bytes_with_missing_envelope_does_not_fabricate_a_message() {
    let mut ctx = mock_context();
    let (account_hash, mailbox_hash, env_hash) = insert_list_unsubscribe_envelope(&ctx);
    let mut view = MailView::new(
        Some((account_hash, mailbox_hash, env_hash)),
        false,
        &mut ctx,
    );

    // The envelope is removed before its body bytes arrive.
    ctx.accounts[&account_hash]
        .collection
        .remove(env_hash, mailbox_hash);
    assert!(!ctx.accounts[&account_hash].contains_key(env_hash));

    super::state::MailViewState::load_bytes(
        &mut view,
        b"Subject: x\r\n\r\nbody".to_vec(),
        &mut ctx,
    );
    assert!(
        !matches!(view.state, super::state::MailViewState::Loaded { .. }),
        "a removed envelope must not produce a Loaded view"
    );
}

#[test]
fn test_view_filter_text_plain() {
    let bytes = b"Content-Transfer-Encoding: 8bit
Content-Type: text/plain; charset=utf-8

foobar
";
    let settings = ViewSettings::default();
    let tempdir = tempfile::tempdir().unwrap();
    let ctx = Context::new_mock(&tempdir);
    let att: Attachment = AttachmentBuilder::new(bytes).build();
    let value = ViewFilter::new_attachment(&att, &settings, &ctx).unwrap();
    assert_eq!(&value.content_type.to_string(), "text/plain");
}

#[test]
fn test_view_filter_text_html() {
    let bytes = b"Content-Transfer-Encoding: 8bit
Content-Type: text/html

foobar
";
    let settings = ViewSettings::default();
    let tempdir = tempfile::tempdir().unwrap();
    let mut ctx = Context::new_mock(&tempdir);
    let att: Attachment = AttachmentBuilder::new(bytes).build();
    let mut value = ViewFilter::new_attachment(&att, &settings, &ctx).unwrap();
    // With the built-in renderer the html job runs in-process and races the
    // brief `try_recv_timeout` window in `ViewFilter::new_html`: the filter
    // is returned either still `Running` or already swapped to the rendered
    // text. In the former case, wait for the job to finish and feed the
    // `JobFinished` event like the real event loop does.
    if matches!(value.body_text, ViewFilterContent::Running { .. }) {
        // The job executor fills the result channel before sending the
        // `JobFinished` thread event, so once it arrives the result is
        // ready; feed it to the filter like the real event loop does.
        loop {
            let mut event = match ctx
                .receiver
                .recv_timeout(std::time::Duration::from_secs(30))
                .expect("job executor thread event channel timed out")
            {
                crate::types::ThreadEvent::JobFinished(job_id) => {
                    UIEvent::StatusEvent(crate::StatusEvent::JobFinished(job_id))
                }
                crate::types::ThreadEvent::UIEvent(ev) => ev,
                _ => continue,
            };
            _ = value.process_event(&mut event, &mut ctx);
            if !matches!(value.body_text, ViewFilterContent::Running { .. }) {
                break;
            }
        }
    }
    assert_eq!(&value.content_type.to_string(), "text/plain");
    assert!(
        value
            .notice
            .as_deref()
            .is_some_and(|notice| notice.contains("built-in html renderer")),
        "unexpected notice: {:?}",
        value.notice
    );
    let inner = match &value.body_text {
        ViewFilterContent::Filtered { inner } => inner,
        other => panic!("expected rendered text, got {other:?}"),
    };
    assert!(inner.contains("foobar"));
}

#[test]
fn test_view_filter_multipart_alternative_plain_and_html() {
    let bytes = b"Content-Transfer-Encoding: 8bit
Content-Type: multipart/alternative; boundary=\"0000000000000000000000000000\"

--0000000000000000000000000000
Content-Type: text/plain; charset=\"UTF-8\"
Content-Transfer-Encoding: 8bit

plain foobar

--0000000000000000000000000000
Content-Type: text/html; charset=\"UTF-8\"
Content-Transfer-Encoding: 8bit

html foobar
";
    let settings = ViewSettings {
        auto_choose_multipart_alternative: true,
        ..ViewSettings::default()
    };

    let tempdir = tempfile::tempdir().unwrap();
    let ctx = Context::new_mock(&tempdir);
    let att: Attachment = AttachmentBuilder::new(bytes).build();
    let value = ViewFilter::new_attachment(&att, &settings, &ctx).unwrap();
    assert_eq!(&value.content_type.to_string(), "text/plain");
}

#[test]
fn test_view_filter_multipart_alternative_empty_plain_and_html() {
    let bytes = b"Content-Transfer-Encoding: 8bit
Content-Type: multipart/alternative; boundary=\"0000000000000000000000000000\"

--0000000000000000000000000000
Content-Type: text/plain; charset=\"UTF-8\"
Content-Transfer-Encoding: 8bit

--0000000000000000000000000000
Content-Type: text/html; charset=\"UTF-8\"
Content-Transfer-Encoding: 8bit

html foobar
";
    let mut settings = ViewSettings {
        auto_choose_multipart_alternative: true,
        ..ViewSettings::default()
    };

    let tempdir = tempfile::tempdir().unwrap();
    let mut ctx = Context::new_mock(&tempdir);
    let att: Attachment = AttachmentBuilder::new(bytes).build();
    let mut value = ViewFilter::new_attachment(&att, &settings, &ctx).unwrap();
    // The plain alternative is empty, so the html one is auto-chosen and
    // goes through the built-in renderer; drive the job to completion like
    // `test_view_filter_text_html` does.
    if matches!(value.body_text, ViewFilterContent::Running { .. }) {
        // The job executor fills the result channel before sending the
        // `JobFinished` thread event, so once it arrives the result is
        // ready; feed it to the filter like the real event loop does.
        loop {
            let mut event = match ctx
                .receiver
                .recv_timeout(std::time::Duration::from_secs(30))
                .expect("job executor thread event channel timed out")
            {
                crate::types::ThreadEvent::JobFinished(job_id) => {
                    UIEvent::StatusEvent(crate::StatusEvent::JobFinished(job_id))
                }
                crate::types::ThreadEvent::UIEvent(ev) => ev,
                _ => continue,
            };
            _ = value.process_event(&mut event, &mut ctx);
            if !matches!(value.body_text, ViewFilterContent::Running { .. }) {
                break;
            }
        }
    }
    assert_eq!(&value.content_type.to_string(), "text/plain");
    let inner = match &value.body_text {
        ViewFilterContent::Filtered { inner } => inner,
        other => panic!("expected rendered text, got {other:?}"),
    };
    assert!(inner.contains("html foobar"));

    settings.auto_choose_multipart_alternative = false;

    let value = ViewFilter::new_attachment(&att, &settings, &ctx).unwrap();
    assert_eq!(&value.content_type.to_string(), "text/plain");
}

// ---------------------------------------------------------------------------
// CVE-2023-4863 (issue #78) — WebP 附件显示 / 打开门禁回归
//
// 仓内孪生：`cve/src/CVE-2023-4863.rs` 从 melib 解析与纯 render 面锁定免疫；
// 这里用真实 `EnvelopeView` / `ViewFilter` 锁定显示分类与显式打开门禁。语料是
// libwebp `BuildHuffmanTable` 越界写出形状的等价结构（真 RIFF + `WEBP` + VP8L
// over-long code-length code）；meli 从不解析它。
// ---------------------------------------------------------------------------

/// CVE-2023-4863 的等价结构恶意 WebP：真 `RIFF` + 小端 size + `WEBP` 魔数 +
/// `VP8L` chunk（签名 + 1×1 头 + over-long code-length code）。
fn cve_2023_4863_webp_payload() -> Vec<u8> {
    const VP8L: &[u8] = &[
        0x2f, 0x00, 0x00, 0x00, 0x00, // signature + 1×1 header
        0xff, 0xff, 0xff, 0xff, 0x0f, // over-long code-length code
    ];
    let mut body = Vec::from(*b"WEBP");
    body.extend_from_slice(b"VP8L");
    body.extend_from_slice(&(VP8L.len() as u32).to_le_bytes());
    body.extend_from_slice(VP8L);
    let mut out = Vec::from(*b"RIFF");
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// 组装一封 `multipart/mixed` 语料邮件：plain + WebP attachment（`photo.webp`）+
/// inline WebP + 引用它的 HTML（`cid:`）。
fn cve_2023_4863_mail() -> Vec<u8> {
    const BOUNDARY: &str = "=_cve20234863_meli";
    let webp = cve_2023_4863_webp_payload();
    let mut out = Vec::new();
    out.extend_from_slice(
        b"From: a@b.example\r\nTo: c@d.example\r\nSubject: webp78\r\nMessage-ID: \
          <cve-2023-4863-meli@x.example>\r\nDate: Thu, 1 Jan 2026 00:00:00 \
          +0000\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; \
          boundary=\"=_cve20234863_meli\"\r\n\r\n",
    );
    let push = |out: &mut Vec<u8>, headers: &str, body: &[u8]| {
        out.extend_from_slice(b"--");
        out.extend_from_slice(BOUNDARY.as_bytes());
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(headers.as_bytes());
        out.extend_from_slice(b"\r\n\r\n");
        out.extend_from_slice(body);
        out.extend_from_slice(b"\r\n");
    };
    push(&mut out, "Content-Type: text/plain; charset=utf-8", b"body");
    push(
        &mut out,
        "Content-Type: image/webp\r\nContent-Disposition: attachment; filename=\"photo.webp\"",
        &webp,
    );
    push(
        &mut out,
        "Content-Type: image/webp\r\nContent-Disposition: inline\r\nContent-ID: \
         <webp78-meli@evil78.example>",
        &webp,
    );
    push(
        &mut out,
        "Content-Type: text/html; charset=utf-8",
        b"<p>x</p><img src=\"cid:webp78-meli@evil78.example\">",
    );
    out.extend_from_slice(b"--");
    out.extend_from_slice(BOUNDARY.as_bytes());
    out.extend_from_slice(b"--\r\n");
    out
}

/// 用 CVE-2023-4863 语料邮件构造 `EnvelopeView`。
fn cve_2023_4863_envelope_view(context: &Context, bytes: &[u8]) -> EnvelopeView {
    let mail = Mail::new(bytes.to_vec(), None).expect("webp78 corpus mail must parse");
    EnvelopeView::new(mail, None, None, None, context.main_loop_handler.clone())
}

/// 递归收集显示树里所有 WebP 的 `AttachmentDisplay::Attachment` 元数据文本。
fn cve_2023_4863_webp_display_entries(display: &[AttachmentDisplay], out: &mut Vec<String>) {
    for entry in display {
        match entry {
            AttachmentDisplay::Attachment { inner } if inner.mime_type() == "image/webp" => {
                out.push(inner.to_string());
            }
            AttachmentDisplay::Attachment { .. }
            | AttachmentDisplay::InlineText { .. }
            | AttachmentDisplay::InlineOther { .. } => {}
            AttachmentDisplay::Alternative { display, .. }
            | AttachmentDisplay::Mixed { display, .. }
            | AttachmentDisplay::InlineRfc822 { display, .. }
            | AttachmentDisplay::SignedPending { display, .. }
            | AttachmentDisplay::SignedFailed { display, .. }
            | AttachmentDisplay::SignedVerified { display, .. }
            | AttachmentDisplay::SignedUnverified { display, .. } => {
                cve_2023_4863_webp_display_entries(display, out);
            }
            AttachmentDisplay::EncryptedPending { .. }
            | AttachmentDisplay::EncryptedFailed { .. } => {}
            AttachmentDisplay::EncryptedSuccess {
                plaintext_display, ..
            } => cve_2023_4863_webp_display_entries(plaintext_display, out),
        }
    }
}

/// CVE-2023-4863（issue #78）：WebP 附件在显示树里只以元数据条目
/// `AttachmentDisplay::Attachment` 出现（attachment 形与 inline 形各一），正文
/// 一个像素都不显示；WebP 载荷字节（`RIFF` / `VP8L`）绝不进入显示文本。
#[test]
fn cve_2023_4863_webp_attachments_render_as_metadata_only() {
    let ctx = mock_context();
    let bytes = cve_2023_4863_mail();
    let view = cve_2023_4863_envelope_view(&ctx, &bytes);

    let mut entries = Vec::new();
    cve_2023_4863_webp_display_entries(&view.display, &mut entries);
    assert_eq!(
        entries.len(),
        2,
        "attachment + inline 两个 WebP 叶都必须是 AttachmentDisplay::Attachment 元数据条目: \
         {entries:?}"
    );
    assert!(
        entries.iter().any(|entry| entry.contains("photo.webp")),
        "attachment 形的 WebP 必须带文件名元数据: {entries:?}"
    );
    for entry in &entries {
        assert!(
            entry.contains("image/webp"),
            "显示文本必须带 MIME 元数据: {entry:?}"
        );
        assert!(
            !entry.contains("RIFF") && !entry.contains("VP8L"),
            "显示文本不得含 WebP 载荷字节: {entry:?}"
        );
    }
}

/// CVE-2023-4863（issue #78）：WebP 的 body-text filter 不做任何 WebP 解释——
/// attachment 形只给空正文 + 附件 notice，`unfiltered` 是传输反转后的原始字节；
/// inline `Other`/`OctetStream` 原样文本分支逐字保留已到达的字节。
#[test]
fn cve_2023_4863_webp_view_filter_only_passes_bytes_through() {
    let ctx = mock_context();
    let settings = ViewSettings::default();
    let webp = cve_2023_4863_webp_payload();

    // attachment 形：空正文 + 附件 notice，unfiltered 逐字等于载荷。
    let mut part = Vec::from(
        &b"Content-Type: image/webp\r\nContent-Disposition: attachment; \
          filename=\"photo.webp\"\r\n\r\n"[..],
    );
    part.extend_from_slice(&webp);
    let att = AttachmentBuilder::new(&part).build();
    let value = ViewFilter::new_attachment(&att, &settings, &ctx).unwrap();
    assert_eq!(value.unfiltered, webp, "filter 不得解释 / 改写 WebP 字节");
    match &value.body_text {
        ViewFilterContent::Filtered { inner } => assert!(inner.is_empty()),
        other => panic!("attached webp must filter to empty text, got {other:?}"),
    }
    assert!(
        value
            .notice
            .as_deref()
            .is_some_and(|notice| notice.contains("photo.webp")),
        "attachment notice 必须只含元数据: {:?}",
        value.notice
    );

    // inline `Other`/`OctetStream` 原样文本分支：UTF-8 的 WebP 形状字节逐字保留，
    // 不解析 VP8L、不生成像素。
    const ASCII_WEBP: &[u8] =
        b"RIFF\x16\x00\x00\x00WEBPVP8L\x0a\x00\x00\x00\x2f\x00\x00\x00\x00\x0f\x0f";
    let mut inline =
        Vec::from(&b"Content-Type: image/webp\r\nContent-Disposition: inline\r\n\r\n"[..]);
    inline.extend_from_slice(ASCII_WEBP);
    let inline_att = AttachmentBuilder::new(&inline).build();
    let inline_value = ViewFilter::new_attachment(&inline_att, &settings, &ctx).unwrap();
    match &inline_value.body_text {
        ViewFilterContent::Filtered { inner } => assert_eq!(
            inner.as_bytes(),
            ASCII_WEBP,
            "inline WebP 原样文本必须逐字保留，不做任何结构解释"
        ),
        other => panic!("inline webp must filter to raw text, got {other:?}"),
    }
}

/// CVE-2023-4863（issue #78）：打开 WebP 附件必须先是显式用户手势——不输入附件
/// 编号时 `open_attachment`（默认 `a`）绝不打开任何东西、不落临时文件、不启动
/// 进程；输入编号后门禁放行，且仍只有「进程外程序」或「原样字节 view filter」
/// 两条惰性通路。
#[test]
fn cve_2023_4863_open_attachment_requires_explicit_attachment_number() {
    let mut ctx = mock_context();
    let bytes = cve_2023_4863_mail();
    let mut view = cve_2023_4863_envelope_view(&ctx, &bytes);
    let temp_before = ctx.temp_files.len();

    // 无附件编号：按键绝不触达打开路径。
    let mut event = UIEvent::Input(Key::Char('a'));
    let handled = view.process_event(&mut event, &mut ctx);
    assert!(!handled, "无附件编号时 open_attachment 分支不得消费事件");
    assert_eq!(view.filters.len(), 0, "无编号时不得打开附件");
    assert_eq!(
        ctx.temp_files.len(),
        temp_before,
        "无编号时不得物化任何临时文件"
    );
    assert!(
        ctx.children.is_empty(),
        "无编号时不得启动任何进程: {:?}",
        ctx.children.keys().collect::<Vec<_>>()
    );
    assert!(
        !ctx.replies()
            .iter()
            .any(|ev| matches!(ev, UIEvent::ProcessRequest(_))),
        "无编号时不得启动任何进程"
    );

    // 显式输入附件编号 2（photo.webp 在附件树里的编号）再按键：门禁放行。把宿主
    // mimeapps 数据库指向空目录，让 `query_default_app("image/webp")` 确定性地
    // 失败，从而走 view filter 回退——测试不得真的拉起宿主的桌面图片查看器，
    // 进程外程序分支的行为也不应依赖宿主环境。`env_lock` 在 drop 时恢复环境
    // （仓库既有约定）。
    let empty = tempfile::tempdir().unwrap();
    let _env = crate::utilities::tests::env_lock();
    std::env::set_var("XDG_DATA_DIRS", empty.path());
    std::env::set_var("XDG_CONFIG_DIRS", empty.path());

    ctx.cmd_buf_push('2', None);
    let mut event = UIEvent::Input(Key::Char('a'));
    let handled = view.process_event(&mut event, &mut ctx);
    assert!(handled, "有附件编号时 open_attachment 分支必须消费事件");
    assert_eq!(
        view.filters.len(),
        1,
        "回退分支必须恰好打开一个 view filter"
    );
    assert_eq!(
        view.filters[0].unfiltered,
        cve_2023_4863_webp_payload(),
        "回退 view filter 不得解释 WebP 字节"
    );
    assert_eq!(
        ctx.temp_files.len(),
        temp_before,
        "回退分支不得物化临时文件"
    );
    assert!(ctx.children.is_empty(), "回退分支不得启动任何进程");
}

/// Create an executable url-launcher "spy" that appends every argument it
/// receives as a line to `dir/spy.log`, and return `(script, log)`.
///
/// The spy is the ground truth for whether (and with what argument) the url
/// launcher was invoked; it never touches the real `xdg-open`.
fn spy_launcher(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join("spy-launcher.sh");
    let log = dir.join("spy.log");
    std::fs::write(
        &script,
        format!("#!/bin/sh\nprintf '%s\\n' \"$1\" >> {}\n", log.display()),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    // Start from a clean log so repeated runs stay idempotent.
    std::fs::write(&log, b"").unwrap();
    (script, log)
}

/// Construct an `EnvelopeView` in URL mode whose sole link is `url`, with the
/// url launcher overridden to `url_launcher`.
fn url_envelope_view(context: &Context, url_launcher: String, url: &str) -> EnvelopeView {
    let bytes = format!(
        "From: a@b.example\r\nTo: c@d.example\r\nSubject: links\r\nMessage-ID: \
         <url-view-1@x.example>\r\nDate: Thu, 1 Jan 2026 00:00:00 +0000\r\nContent-Type: \
         text/plain; charset=utf-8\r\n\r\n{url}\r\n"
    );
    let mail = Mail::new(bytes.into_bytes(), None).expect("could not parse test mail");
    let view_settings = ViewSettings {
        url_launcher: Some(url_launcher),
        ..ViewSettings::default()
    };
    let mut view = EnvelopeView::new(
        mail,
        None,
        None,
        Some(view_settings),
        context.main_loop_handler.clone(),
    );
    view.options.insert(ViewOptions::URL);
    view.links.push(Link {
        start: 0,
        end: url.len(),
        value: Cow::Owned(url.to_string()),
        kind: LinkKind::Url,
    });
    view
}

/// Select link `0` (the first link; URL mode numbers them from zero) in the
/// command buffer and press `go_to_url` (`g` by default), like a user would in
/// URL mode.
fn trigger_go_to_url(view: &mut EnvelopeView, context: &mut Context) {
    context.cmd_buf_push('0', None);
    let mut event = UIEvent::Input(Key::Char('g'));
    _ = view.process_event(&mut event, context);
}

/// The `go_to_url` action must not hand a URL with a non-default scheme
/// (anything but http/https/mailto) to the system url launcher without the
/// user confirming a dialog first. Security fix W3-T17.
#[test]
fn go_to_url_non_default_scheme_requires_confirmation() {
    let mut ctx = mock_context();
    let dir = tempfile::tempdir().unwrap();
    let (script, log) = spy_launcher(dir.path());
    let mut view = url_envelope_view(
        &ctx,
        script.to_string_lossy().into_owned(),
        "file:///etc/passwd",
    );

    trigger_go_to_url(&mut view, &mut ctx);
    // Give a (hypothetical, unconfirmed) launcher process time to finish.
    std::thread::sleep(std::time::Duration::from_millis(300));
    let invoked = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        invoked.is_empty(),
        "non-default scheme must not reach the url launcher before confirmation; launcher \
         received: {invoked:?}"
    );
    assert!(view.launch_url_dialog.is_some());
    assert_eq!(
        view.pending_launch_url.as_deref(),
        Some("file:///etc/passwd")
    );
}

/// Scheme classification: only `http`, `https` and `mailto` (any casing) are
/// launched without confirmation; everything else — other schemes, URLs with
/// no scheme, and malformed input — must be gated.
#[test]
fn test_is_default_launchable_scheme() {
    use super::envelope::is_default_launchable_scheme as launchable;
    for url in [
        "http://example.example/",
        "https://example.example/a?b=c#d",
        "HTTP://EXAMPLE.EXAMPLE/",
        "HTTPS://example.example/",
        "MaIlTo:a@b.example",
        "mailto:",
    ] {
        assert!(
            launchable(url),
            "{url:?} must be launchable without confirmation"
        );
    }
    for url in [
        "file:///etc/passwd",
        "FILE:///etc/passwd",
        "gopher://gopher.example/0/menu",
        "irc://irc.example/%23meli",
        "xmpp:node@example.example",
        "ms-excel:\\\\evil.example\\book.xls",
        "web+ap:example/notes/1",
        "mailto-lookalike://x.example",
        "relative/path/garbage",
        "no scheme colon here",
        "",
        ":", // empty scheme
        "1http://x.example",
        "+http://x.example",
        "ma ilto:a@b.example",
    ] {
        assert!(
            !launchable(url),
            "{url:?} must require confirmation before launching"
        );
    }
}

/// Confirm path: pressing Enter in the dialog must invoke the url launcher
/// exactly once with the exact URL that was shown, and tear the dialog down.
#[test]
fn go_to_url_confirm_launches_url_after_dialog() {
    let mut ctx = mock_context();
    let dir = tempfile::tempdir().unwrap();
    let (script, log) = spy_launcher(dir.path());
    let mut view = url_envelope_view(
        &ctx,
        script.to_string_lossy().into_owned(),
        "gopher://gopher.example/0/menu",
    );

    trigger_go_to_url(&mut view, &mut ctx);
    assert!(view.launch_url_dialog.is_some());
    assert_eq!(
        view.pending_launch_url.as_deref(),
        Some("gopher://gopher.example/0/menu")
    );

    let mut event = UIEvent::Input(Key::Char('\n'));
    _ = view.process_event(&mut event, &mut ctx);
    let replies: Vec<UIEvent> = ctx.replies().into_iter().collect();
    assert!(
        replies.iter().any(
            |ev| matches!(ev, UIEvent::FinishedUIDialog(_, result) if result
            .downcast_ref::<bool>()
            == Some(&true))
        ),
        "confirming with Enter must emit a confirmed FinishedUIDialog, got: {replies:?}"
    );

    // The main loop feeds component replies back into process_event.
    for mut ev in replies {
        _ = view.process_event(&mut ev, &mut ctx);
    }

    let mut invoked = String::new();
    for _ in 0..150 {
        invoked = std::fs::read_to_string(&log).unwrap_or_default();
        if !invoked.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(
        invoked, "gopher://gopher.example/0/menu\n",
        "launcher must be invoked exactly once with the exact URL"
    );
    assert!(view.launch_url_dialog.is_none());
    assert!(view.pending_launch_url.is_none());
}

/// Insert an envelope with arbitrary extra headers (each already
/// CRLF-terminated) into the mock account's `INBOX` and return its
/// coordinates, so `mailbox_settings!` lookups in `MailView` resolve.
fn insert_envelope_with_headers(
    context: &Context,
    extra_headers: &str,
) -> (AccountHash, MailboxHash, EnvelopeHash) {
    let account_hash = *context.accounts.iter().next().unwrap().0;
    let mailbox_hash = MailboxHash::from_bytes(b"INBOX");
    let bytes = format!(
        "From: newsletter@list.example\r\n\
To: victim@victim.example\r\n\
Subject: weekly\r\n\
Message-ID: <list-headers-{}@list.example>\r\n\
Date: Thu, 1 Jan 2026 00:00:00 +0000\r\n\
{extra_headers}\r\n\
hello\r\n",
        extra_headers.len()
    );
    let envelope = Envelope::from_bytes(bytes.as_bytes(), None).expect("could not parse envelope");
    let env_hash = envelope.hash();
    context.accounts[&account_hash]
        .collection
        .insert(envelope, mailbox_hash);
    (account_hash, mailbox_hash, env_hash)
}

/// Point the mock context's global url-launcher setting at the spy script.
fn use_spy_launcher(context: &mut Context, script: &Path) {
    context.settings.pager.url_launcher = Some(script.to_string_lossy().into_owned());
}

/// A `List-Unsubscribe` URL option with a non-launchable scheme (UNC path,
/// `file:`, `smb:`, application scheme) must never be offered in the
/// confirmation dialog nor reach the url launcher: only the first
/// `http`/`https`/`mailto` option may. Security fix for CVE-2023-23397
/// (issue #21): the Outlook reminder-sound vector dereferenced exactly such
/// attacker-chosen paths; meli's mail-header-derived launcher targets get
/// the same scheme whitelist `go_to_url` applies to body links (W3-T17).
#[test]
fn list_unsubscribe_skips_non_launchable_url_options() {
    let mut ctx = mock_context();
    let dir = tempfile::tempdir().unwrap();
    let (script, log) = spy_launcher(dir.path());
    use_spy_launcher(&mut ctx, &script);
    _ = register_inbox(&mut ctx);
    let coordinates = insert_envelope_with_headers(
        &ctx,
        "List-Unsubscribe: <\\\\attacker.example\\share\\a.wav>, \
<File://attacker.example/share/unsub.html>, \
<smb://attacker.example/share/unsub>, \
<https://lists.example/unsub?u=victim>\r\n",
    );
    let mut view = MailView::new(Some(coordinates), false, &mut ctx);

    trigger_list_unsubscribe(&mut view, &mut ctx);
    // The non-launchable options must be skipped in order; the first
    // launchable one is offered for confirmation.
    assert!(view.unsubscribe_dialog.is_some(), "dialog must open");
    assert_eq!(
        view.pending_unsubscribe
            .as_ref()
            .map(|a| a.target_description()),
        Some("https://lists.example/unsub?u=victim".to_string()),
        "UNC/file/smb options must be skipped, got: {:?}",
        view.pending_unsubscribe
    );

    // Confirm, feed the dialog replies back like the main loop, then check
    // the spy received exactly the whitelisted URL.
    let mut event = UIEvent::Input(Key::Char('\n'));
    _ = view.process_event(&mut event, &mut ctx);
    let replies: Vec<UIEvent> = ctx.replies().into_iter().collect();
    for mut ev in replies {
        _ = view.process_event(&mut ev, &mut ctx);
    }
    let mut invoked = String::new();
    for _ in 0..150 {
        invoked = std::fs::read_to_string(&log).unwrap_or_default();
        if !invoked.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(
        invoked, "https://lists.example/unsub?u=victim\n",
        "launcher must receive exactly the whitelisted URL"
    );
}

/// A `List-Unsubscribe` header whose only URL options are non-launchable
/// (UNC `\\host\share\file`, the exact CVE-2023-23397 payload form) must
/// offer nothing at all: no dialog, no pending action, no launcher
/// invocation, and a notification explaining the refusal.
#[test]
fn list_unsubscribe_unc_only_is_refused() {
    let mut ctx = mock_context();
    let dir = tempfile::tempdir().unwrap();
    let (script, log) = spy_launcher(dir.path());
    use_spy_launcher(&mut ctx, &script);
    _ = register_inbox(&mut ctx);
    let coordinates = insert_envelope_with_headers(
        &ctx,
        "List-Unsubscribe: <\\\\attacker.example\\share\\a.wav>\r\n",
    );
    let mut view = MailView::new(Some(coordinates), false, &mut ctx);

    trigger_list_unsubscribe(&mut view, &mut ctx);
    assert!(view.unsubscribe_dialog.is_none(), "no dialog may open");
    assert!(view.pending_unsubscribe.is_none(), "no action may pend");
    let replies: Vec<UIEvent> = ctx.replies().into_iter().collect();
    assert!(
        replies.iter().any(
            |ev| matches!(ev, UIEvent::Notification { title: Some(t), .. }
            if t.contains("List-Unsubscribe"))
        ),
        "a refusal notification must be emitted, got: {replies:?}"
    );
    std::thread::sleep(std::time::Duration::from_millis(300));
    let invoked = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        invoked.is_empty(),
        "UNC List-Unsubscribe option must never reach the launcher, got: {invoked:?}"
    );
}

/// The `List-Archive` action must refuse — with a notification, and without
/// invoking the url launcher — an archive URL whose scheme is not
/// http/https/mailto. Security fix for CVE-2023-23397 (issue #21):
/// `List-Archive` is attacker-controlled header content that used to be
/// handed to the OS launcher as-is, unlike body links (W3-T17).
#[test]
fn list_archive_non_launchable_scheme_is_refused() {
    let mut ctx = mock_context();
    let dir = tempfile::tempdir().unwrap();
    let (script, log) = spy_launcher(dir.path());
    use_spy_launcher(&mut ctx, &script);
    _ = register_inbox(&mut ctx);
    let coordinates = insert_envelope_with_headers(
        &ctx,
        "List-Archive: <File://attacker.example/share/a.wav>\r\n",
    );
    let mut view = MailView::new(Some(coordinates), false, &mut ctx);

    let mut event = UIEvent::Action(Action::MailingListAction(MailingListAction::ListArchive));
    _ = view.process_event(&mut event, &mut ctx);

    std::thread::sleep(std::time::Duration::from_millis(300));
    let invoked = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(
        invoked.is_empty(),
        "non-launchable List-Archive URL must never reach the launcher, got: {invoked:?}"
    );
    let replies: Vec<UIEvent> = ctx.replies().into_iter().collect();
    assert!(
        replies.iter().any(
            |ev| matches!(ev, UIEvent::Notification { title: Some(t), .. }
                if t.contains("Refusing to open List-Archive URL"))
        ),
        "a refusal notification must be emitted, got: {replies:?}"
    );
}

/// Positive control: an `http(s)` `List-Archive` URL is still handed to the
/// configured launcher unchanged — the scheme gate must not break the
/// legitimate feature.
#[test]
fn list_archive_https_still_launches() {
    let mut ctx = mock_context();
    let dir = tempfile::tempdir().unwrap();
    let (script, log) = spy_launcher(dir.path());
    use_spy_launcher(&mut ctx, &script);
    _ = register_inbox(&mut ctx);
    let coordinates =
        insert_envelope_with_headers(&ctx, "List-Archive: <https://lists.example/archive/>\r\n");
    let mut view = MailView::new(Some(coordinates), false, &mut ctx);

    let mut event = UIEvent::Action(Action::MailingListAction(MailingListAction::ListArchive));
    _ = view.process_event(&mut event, &mut ctx);

    let mut invoked = String::new();
    for _ in 0..150 {
        invoked = std::fs::read_to_string(&log).unwrap_or_default();
        if !invoked.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(
        invoked, "https://lists.example/archive/\n",
        "launcher must receive the https archive URL unchanged"
    );
}

/// Register an `INBOX` mailbox on the mock account so per-mailbox settings
/// lookups (`mailbox_settings!`, e.g. `ThreadView::shortcuts`) resolve.
fn register_inbox(context: &mut Context) -> (AccountHash, MailboxHash) {
    #[derive(Debug)]
    struct TestMailbox {
        hash: MailboxHash,
        name: String,
        subscribed: bool,
    }

    impl BackendMailbox for TestMailbox {
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
                subscribed: self.subscribed,
            })
        }

        fn special_usage(&self) -> SpecialUsageMailbox {
            SpecialUsageMailbox::Normal
        }

        fn parent(&self) -> Option<MailboxHash> {
            None
        }

        fn permissions(&self) -> MailboxPermissions {
            MailboxPermissions::default()
        }

        fn is_subscribed(&self) -> bool {
            self.subscribed
        }

        fn set_is_subscribed(&mut self, new_val: bool) -> melib::Result<()> {
            self.subscribed = new_val;
            Ok(())
        }

        fn set_special_usage(&mut self, _new_val: SpecialUsageMailbox) -> melib::Result<()> {
            Ok(())
        }

        fn count(&self) -> melib::Result<(usize, usize)> {
            Ok((0, 0))
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }

        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    let account_hash = *context.accounts.iter().next().unwrap().0;
    let mailbox_hash = MailboxHash::from_bytes(b"INBOX");
    let account = context.accounts.get_mut(&account_hash).unwrap();
    account.mailbox_entries.insert(
        mailbox_hash,
        MailboxEntry::new(
            MailboxStatus::Available,
            "INBOX".to_string(),
            Box::new(TestMailbox {
                hash: mailbox_hash,
                name: "INBOX".to_string(),
                subscribed: true,
            }),
            FileMailboxConf::default(),
        ),
    );
    (account_hash, mailbox_hash)
}

/// Insert a two-mail thread (root + reply) into the mock account's
/// collection and build a `ThreadView` over it at the given focus,
/// mirroring `golden.rs::golden_two_mail_thread_view`.
fn two_mail_thread_view(context: &mut Context, focus: ThreadViewFocus) -> ThreadView {
    let (account_hash, mailbox_hash) = register_inbox(context);
    let root_bytes = b"From: a@b.example\r\nTo: c@d.example\r\nSubject: thread\r\nMessage-ID: <enter-root@x.example>\r\nDate: Thu, 1 Jan 2026 00:00:00 +0000\r\n\r\nroot\r\n";
    let reply_bytes = b"From: c@d.example\r\nTo: a@b.example\r\nSubject: Re: thread\r\nMessage-ID: <enter-reply@x.example>\r\nIn-Reply-To: <enter-root@x.example>\r\nDate: Thu, 1 Jan 2026 00:01:00 +0000\r\n\r\nreply\r\n";
    let mut root_hash = None;
    for bytes in [root_bytes.as_slice(), reply_bytes.as_slice()] {
        let envelope = Envelope::from_bytes(bytes, None).expect("could not parse test envelope");
        let hash = envelope.hash();
        context.accounts[&account_hash]
            .collection
            .insert(envelope, mailbox_hash);
        if root_hash.is_none() {
            root_hash = Some(hash);
        }
    }
    let root_hash = root_hash.unwrap();
    let thread_group = {
        let threads = context.accounts[&account_hash]
            .collection
            .get_threads(mailbox_hash)
            .expect("test fixture inserted mail into the mailbox threads");
        threads.find_group(threads.envelope_to_thread[&root_hash])
    };
    ThreadView::new(
        (account_hash, mailbox_hash, root_hash),
        thread_group,
        None,
        false,
        Some(focus),
        context,
    )
}

/// Whether any queued reply opens a new tab.
fn has_new_tab_reply(replies: &[UIEvent]) -> bool {
    replies
        .iter()
        .any(|ev| matches!(ev, UIEvent::Action(Action::Tab(TabAction::New(Some(_))))))
}

/// Regression: Enter with the keyboard on the mail content pane — every
/// layout's mail view (the two-pane mail layout's right pane, the thread
/// layout's mail pane) — opens the mail being viewed in a new tab, the
/// same action the `open-in-tab` command dispatches.
#[test]
fn enter_at_mail_view_focus_opens_new_tab() {
    let mut ctx = mock_context();
    let mut view = two_mail_thread_view(&mut ctx, ThreadViewFocus::MailView);
    // A frame renders first (as in real usage): `draw` settles the pending
    // expanded-entry selection the new-tab path reads.
    {
        let theme_default = crate::conf::value(&ctx, "theme_default");
        let mut screen = Screen::<Virtual>::new(theme_default);
        assert!(screen.resize(80, 24));
        let area = screen.area();
        view.draw(screen.grid_mut(), area, &mut ctx);
    }
    let _ = ctx.replies();
    let mut event = UIEvent::Input(Key::Char('\n'));
    assert!(
        view.process_event(&mut event, &mut ctx),
        "Enter at mail view focus must be consumed"
    );

    let replies: Vec<UIEvent> = ctx.replies().into_iter().collect();
    assert!(
        has_new_tab_reply(&replies),
        "Enter at mail view focus must open a new tab, got: {replies:?}"
    );

    // The new tab is the envelope view of the mail being read —
    // full-screen mail content. Draw the tab payload and assert it is NOT
    // the thread list: no list ring, no thread rows (the mock backend
    // cannot load the body, so the mail pane renders its loading/error
    // placeholder — the discriminator is the absent thread-list chrome).
    let mut tab = replies
        .into_iter()
        .find_map(|ev| match ev {
            UIEvent::Action(Action::Tab(TabAction::New(Some(component)))) => Some(component),
            _ => None,
        })
        .expect("Tab(New) reply must carry the new component");
    {
        let theme_default = crate::conf::value(&ctx, "theme_default");
        let mut screen = Screen::<Virtual>::new(theme_default);
        assert!(screen.resize(80, 24));
        let area = screen.area();
        tab.draw(screen.grid_mut(), area, &mut ctx);
        let _ = ctx.replies();
        tab.draw(screen.grid_mut(), area, &mut ctx);
        let grid = screen.grid();
        assert_eq!(
            grid[(0, 0)].ch(),
            '╭',
            "the envelope tab is framed with a rounded border"
        );
        assert_eq!(
            grid[(79, 23)].ch(),
            '╯',
            "the envelope tab frame is rounded at the bottom-right corner"
        );
        let row: String = (0..80).map(|x| grid[(x, 1)].ch()).collect();
        assert!(
            !row.contains("thread"),
            "no thread rows inside the envelope tab (only the frame), got: {row:?}"
        );
    }

    // The `open-in-tab` command from the thread list keeps the
    // whole-thread tab (the whole-list state), unchanged.
    let mut ctx = mock_context();
    let mut view = two_mail_thread_view(&mut ctx, ThreadViewFocus::Thread);
    {
        let theme_default = crate::conf::value(&ctx, "theme_default");
        let mut screen = Screen::<Virtual>::new(theme_default);
        assert!(screen.resize(80, 24));
        let area = screen.area();
        view.draw(screen.grid_mut(), area, &mut ctx);
    }
    let _ = ctx.replies();
    let mut event = UIEvent::Action(Action::Listing(ListingAction::OpenInNewTab));
    assert!(view.process_event(&mut event, &mut ctx));
    let mut tab = ctx
        .replies()
        .into_iter()
        .find_map(|ev| match ev {
            UIEvent::Action(Action::Tab(TabAction::New(Some(component)))) => Some(component),
            _ => None,
        })
        .expect("the command must open a new tab");
    {
        let theme_default = crate::conf::value(&ctx, "theme_default");
        let mut screen = Screen::<Virtual>::new(theme_default);
        assert!(screen.resize(80, 24));
        let area = screen.area();
        tab.draw(screen.grid_mut(), area, &mut ctx);
        let grid = screen.grid();
        assert_eq!(grid[(0, 0)].ch(), '╭', "whole-list frame top-left");
        let row: String = (0..80).map(|x| grid[(x, 1)].ch()).collect();
        assert!(
            row.contains("thread"),
            "the thread rows must render in the whole-list tab, got: {row:?}"
        );
    }
}

/// Enter stays inert when the thread list, not the mail content window,
/// holds the keyboard.
#[test]
fn enter_at_thread_list_focus_does_not_open_tab() {
    let mut ctx = mock_context();
    let mut view = two_mail_thread_view(&mut ctx, ThreadViewFocus::Thread);

    let mut event = UIEvent::Input(Key::Char('\n'));
    let _ = view.process_event(&mut event, &mut ctx);

    let replies: Vec<UIEvent> = ctx.replies().into_iter().collect();
    assert!(
        !has_new_tab_reply(&replies),
        "Enter at thread list focus must not open a new tab, got: {replies:?}"
    );
}

/// While a mail view dialog (List-Unsubscribe confirmation) is open, Enter
/// belongs to the dialog: it must confirm it instead of opening a new tab.
#[test]
fn enter_with_open_dialog_confirms_instead_of_new_tab() {
    let mut ctx = mock_context();
    register_inbox(&mut ctx);
    let (account_hash, mailbox_hash, env_hash) = insert_list_unsubscribe_envelope(&ctx);
    let thread_group = {
        let threads = ctx.accounts[&account_hash]
            .collection
            .get_threads(mailbox_hash)
            .expect("test fixture inserted mail into the mailbox threads");
        threads.find_group(threads.envelope_to_thread[&env_hash])
    };
    let mut view = ThreadView::new(
        (account_hash, mailbox_hash, env_hash),
        thread_group,
        None,
        false,
        Some(ThreadViewFocus::MailView),
        &mut ctx,
    );

    // Open the List-Unsubscribe confirmation dialog through the embedded
    // mail view.
    let mut open = UIEvent::Action(Action::MailingListAction(
        MailingListAction::ListUnsubscribe,
    ));
    assert!(
        view.process_event(&mut open, &mut ctx),
        "the unsubscribe action must reach the embedded mail view"
    );
    let _ = ctx.replies();

    let mut event = UIEvent::Input(Key::Char('\n'));
    assert!(
        view.process_event(&mut event, &mut ctx),
        "Enter must be consumed by the dialog"
    );
    let replies: Vec<UIEvent> = ctx.replies().into_iter().collect();
    assert!(
        replies
            .iter()
            .any(|ev| matches!(ev, UIEvent::FinishedUIDialog(_, _))),
        "Enter must confirm the open dialog, got: {replies:?}"
    );
    assert!(
        !has_new_tab_reply(&replies),
        "an open dialog must keep Enter for itself, got: {replies:?}"
    );
}

/// Cancel path: Esc must tear the dialog down without launching anything and
/// leave no pending URL behind; this must also hold for a malformed URL with
/// no parseable scheme.
#[test]
fn go_to_url_cancel_does_not_launch() {
    let mut ctx = mock_context();
    let dir = tempfile::tempdir().unwrap();
    let (script, log) = spy_launcher(dir.path());

    for url in ["file:///etc/passwd", "not-a-url-at-all"] {
        std::fs::write(&log, b"").unwrap();
        let mut view = url_envelope_view(&ctx, script.to_string_lossy().into_owned(), url);

        trigger_go_to_url(&mut view, &mut ctx);
        assert!(
            view.launch_url_dialog.is_some(),
            "dialog must open for {url:?}"
        );
        assert_eq!(view.pending_launch_url.as_deref(), Some(url));

        let mut event = UIEvent::Input(Key::Esc);
        _ = view.process_event(&mut event, &mut ctx);
        let replies: Vec<UIEvent> = ctx.replies().into_iter().collect();
        let confirmed = replies.iter().any(|ev| match ev {
            UIEvent::FinishedUIDialog(_, result) => result.downcast_ref::<bool>() == Some(&true),
            _ => false,
        });
        assert!(!confirmed, "cancel must not emit a confirmation event");

        // The main loop feeds component replies (ComponentUnrealize) back in.
        for mut ev in replies {
            _ = view.process_event(&mut ev, &mut ctx);
        }
        assert!(view.launch_url_dialog.is_none());
        assert!(view.pending_launch_url.is_none());

        std::thread::sleep(std::time::Duration::from_millis(300));
        let invoked = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(
            invoked.is_empty(),
            "cancelling for {url:?} must not invoke the launcher; got: {invoked:?}"
        );
    }
}

/// Allowlisted schemes (http/https/mailto, case-insensitive) keep launching
/// directly with no dialog — current behavior frozen.
#[test]
fn go_to_url_allowlisted_scheme_launches_directly() {
    let mut ctx = mock_context();
    for url in [
        "https://example.example/page?a=b#c",
        "HTTP://EXAMPLE.EXAMPLE/UPPER",
        "mailto:someone@example.example?subject=hi",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (script, log) = spy_launcher(dir.path());
        let mut view = url_envelope_view(&ctx, script.to_string_lossy().into_owned(), url);

        trigger_go_to_url(&mut view, &mut ctx);
        assert!(
            view.launch_url_dialog.is_none(),
            "{url:?} must launch without a confirmation dialog"
        );
        assert!(view.pending_launch_url.is_none());

        let mut invoked = String::new();
        for _ in 0..150 {
            invoked = std::fs::read_to_string(&log).unwrap_or_default();
            if !invoked.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(
            invoked,
            format!("{url}\n"),
            "{url:?} must be passed to the launcher byte-identically"
        );
    }
}

/// Like [`spy_launcher`], but the script records the argument *count*
/// and every argument, one per line (the first line is always `$#`).
/// This is the ground truth for the CVE-2007-4040 contract below: a
/// shell would split or expand the arguments, an exec(3) argv spawn
/// reproduces them verbatim.
fn argv_spy_launcher(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join("argv-spy-launcher.sh");
    let log = dir.join("argv-spy.log");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$#\" \"$@\" >> {}\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(&log, b"").unwrap();
    (script, log)
}

/// Wait (bounded) for the argv spy to record an invocation and return
/// its lines (argument count first, then each argument).
fn wait_for_argv_spy(log: &Path) -> Vec<String> {
    let mut invoked = String::new();
    for _ in 0..150 {
        invoked = std::fs::read_to_string(log).unwrap_or_default();
        if !invoked.is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    invoked.lines().map(str::to_string).collect()
}

/// CVE-2007-4040 (Outlook/OE URI parameter injection, issue #65): a
/// URL with a *whitelisted* scheme carrying shell metacharacters — the
/// CVE's kill-shot shape, since the scheme gate lets it through — must
/// reach the url launcher as exactly one literal argv element: never
/// through a shell, never split, never expanded. The `$(touch …)`
/// payload doubles as the side-effect oracle: had any shell
/// interpreted the URL, the marker file would exist.
#[test]
fn go_to_url_metacharacter_url_is_one_literal_argv_element() {
    let mut ctx = mock_context();
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("pwned-marker");
    let (script, log) = argv_spy_launcher(dir.path());
    for url in [
        format!("https://ex.example/p?next=$(touch {})", marker.display()),
        "https://ex.example/a`id`b".to_string(),
        "https://ex.example/x$(cmd)".to_string(),
        "https://ex.example/$HOME/*.txt".to_string(),
        "https://ex.example/a b\"c'd".to_string(),
        "mailto:a@b.example?subject=hi;rm%20-rf&body=;x".to_string(),
    ] {
        std::fs::write(&log, b"").unwrap();
        let mut view = url_envelope_view(&ctx, script.to_string_lossy().into_owned(), &url);

        trigger_go_to_url(&mut view, &mut ctx);
        assert!(
            view.launch_url_dialog.is_none(),
            "{url:?} must launch without a confirmation dialog"
        );

        let lines = wait_for_argv_spy(&log);
        assert_eq!(
            lines,
            vec!["1".to_string(), url.clone()],
            "{url:?} must reach the launcher as exactly one byte-identical \
             argv element (count first)"
        );
        assert!(
            !marker.exists(),
            "the `$(touch …)` payload must never execute: no shell touches the URL"
        );
    }
}

/// CVE-2021-37746, disguised-lookalike face: the corpus homoglyph and bidi
/// (RLO) spellings are honest http/https schemes of *displayed bytes*, so
/// they launch directly (no dialog) — and what reaches the launcher is the
/// exact same byte string the pager displayed: one literal argv element, no
/// reordering, no shell. meli never launches a href that was not displayed
/// (the HTML mirror shows every surviving href as a numbered footnote); this
/// locks the launcher side of that displayed == launched invariant.
#[test]
fn go_to_url_disguised_lookalike_urls_are_one_literal_argv_element() {
    let mut ctx = mock_context();
    let dir = tempfile::tempdir().unwrap();
    let (script, log) = argv_spy_launcher(dir.path());
    for url in [
        "http://evil.example/phish".to_string(),
        "https://b\u{0430}nk.example/login".to_string(),
        "http://\u{202E}elpmaxe.live\u{202C}/login".to_string(),
        "http://evil.example/\u{202E}docs".to_string(),
    ] {
        std::fs::write(&log, b"").unwrap();
        let mut view = url_envelope_view(&ctx, script.to_string_lossy().into_owned(), &url);

        trigger_go_to_url(&mut view, &mut ctx);
        assert!(
            view.launch_url_dialog.is_none(),
            "{url:?} must launch without a confirmation dialog"
        );

        let lines = wait_for_argv_spy(&log);
        assert_eq!(
            lines,
            vec!["1".to_string(), url.clone()],
            "{url:?} must reach the launcher as exactly one byte-identical \\
             argv element (count first)"
        );
    }
}

/// CVE-2007-4040, unknown-scheme face: the advisory's `unknown:$(cmd)`
/// family must never auto-launch (explicit per-URL confirmation first),
/// and *after* the user confirms, the metacharacter URL still must not
/// be shell-interpreted: it launches as one literal argv element.
#[test]
fn go_to_url_unknown_scheme_metacharacters_need_confirmation_and_stay_literal() {
    let mut ctx = mock_context();
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("pwned-marker");
    let (script, log) = argv_spy_launcher(dir.path());
    for url in [
        format!("unknown:$(touch {})", marker.display()),
        format!("unknown://x$(touch {})y", marker.display()),
        "search-ms:displayname=Policy,query=$(id)".to_string(),
    ] {
        std::fs::write(&log, b"").unwrap();
        let mut view = url_envelope_view(&ctx, script.to_string_lossy().into_owned(), &url);

        trigger_go_to_url(&mut view, &mut ctx);
        assert!(
            view.launch_url_dialog.is_some(),
            "{url:?} must be held behind a confirmation dialog"
        );
        assert_eq!(view.pending_launch_url.as_deref(), Some(url.as_str()));
        std::thread::sleep(std::time::Duration::from_millis(300));
        let invoked = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(
            invoked.is_empty(),
            "{url:?} must not reach the launcher before confirmation; got: {invoked:?}"
        );

        // Confirm the dialog, feeding the component's replies back like
        // the main loop does.
        let mut event = UIEvent::Input(Key::Char('\n'));
        _ = view.process_event(&mut event, &mut ctx);
        let replies: Vec<UIEvent> = ctx.replies().into_iter().collect();
        assert!(
            replies.iter().any(
                |ev| matches!(ev, UIEvent::FinishedUIDialog(_, result) if result
                    .downcast_ref::<bool>()
                    == Some(&true))
            ),
            "confirming must emit a confirmed FinishedUIDialog"
        );
        for mut ev in replies {
            _ = view.process_event(&mut ev, &mut ctx);
        }

        let lines = wait_for_argv_spy(&log);
        assert_eq!(
            lines,
            vec!["1".to_string(), url.clone()],
            "{url:?} must launch after confirmation as exactly one literal argv \
             element (count first)"
        );
        assert!(
            !marker.exists(),
            "even a confirmed metacharacter URL must never execute its payload"
        );
        assert!(view.launch_url_dialog.is_none());
        assert!(view.pending_launch_url.is_none());
    }
}

/// CVE-2007-4040, header-derived face: the `List-Archive` header can
/// carry attacker-controlled metacharacter bytes past the RFC 2369
/// parser and the scheme whitelist (an `https:` archive URL is
/// legitimate); its launcher invocation must still be one literal argv
/// element.
#[test]
fn list_archive_metacharacter_url_is_one_literal_argv_element() {
    let mut ctx = mock_context();
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("pwned-marker");
    let url = format!("https://lists.example/arch/$(touch {})", marker.display());
    let (script, log) = argv_spy_launcher(dir.path());
    use_spy_launcher(&mut ctx, &script);
    _ = register_inbox(&mut ctx);
    let coordinates = insert_envelope_with_headers(&ctx, &format!("List-Archive: <{url}>\r\n"));
    let mut view = MailView::new(Some(coordinates), false, &mut ctx);

    let mut event = UIEvent::Action(Action::MailingListAction(MailingListAction::ListArchive));
    _ = view.process_event(&mut event, &mut ctx);

    let lines = wait_for_argv_spy(&log);
    assert_eq!(
        lines,
        vec!["1".to_string(), url],
        "the archive metacharacter URL must reach the launcher as exactly one \
         byte-identical argv element (count first)"
    );
    assert!(
        !marker.exists(),
        "the `$(touch …)` payload must never execute from header bytes either"
    );
}

/// MIME payload of the shared multipart/mixed fixture: a text/plain body
/// part plus an `application/pdf` attachment and an inline `image/png` part
/// carrying a filename (both count as attachments for the batch save).
const ATTACHMENTS_MULTIPART_BODY: &str = "MIME-Version: 1.0\r\n\
     Content-Type: multipart/mixed; boundary=\"=_b\"\r\n\
     \r\n\
     --=_b\r\n\
     Content-Type: text/plain; charset=utf-8\r\n\
     \r\n\
     body text\r\n\
     --=_b\r\n\
     Content-Type: application/pdf; name=\"report.pdf\"\r\n\
     Content-Disposition: attachment; filename=\"report.pdf\"\r\n\
     \r\n\
     %PDF-fake\r\n\
     --=_b\r\n\
     Content-Type: image/png\r\n\
     Content-Disposition: inline; filename=\"inline-image.png\"\r\n\
     \r\n\
     PNG-fake\r\n\
     --=_b--\r\n";

/// Construct an `EnvelopeView` for an arbitrary raw MIME payload `body`
/// (everything from `MIME-Version:` onward) under the given Subject and
/// Message-ID, following the `url_envelope_view` construction pattern.
fn attachments_envelope_view(
    context: &Context,
    subject: &str,
    message_id: &str,
    body: &str,
) -> EnvelopeView {
    let bytes = format!(
        "From: a@b.example\r\nTo: c@d.example\r\nSubject: {subject}\r\nMessage-ID: \
         <{message_id}>\r\nDate: Thu, 1 Jan 2026 00:00:00 +0000\r\n{body}"
    );
    let mail = Mail::new(bytes.into_bytes(), None).expect("could not parse test mail");
    EnvelopeView::new(mail, None, None, None, context.main_loop_handler.clone())
}

/// Fire the `save-all-attachments` view action at `view`, like the parsed
/// user command would.
fn trigger_save_all_attachments(view: &mut EnvelopeView, context: &mut Context) {
    let mut event = UIEvent::Action(Action::View(ViewAction::SaveAllAttachments));
    _ = view.process_event(&mut event, context);
}

/// End-to-end happy path: `save-all-attachments` on a multipart mail with
/// one attachment and one inline-with-filename part must write both into a
/// single fresh `~/Downloads/meli-<subject>` directory with 0o600 files and
/// exactly one summary notification. The 200-CJK-char Subject exercises the
/// component-level truncation caps (≤128 chars, ≤240 bytes).
#[test]
fn save_all_attachments_happy() {
    use std::os::unix::fs::PermissionsExt;

    let mut ctx = mock_context();
    let subject = format!("{}-one", "请".repeat(200));
    let view = attachments_envelope_view(
        &ctx,
        &subject,
        "save-all-1@x.example",
        ATTACHMENTS_MULTIPART_BODY,
    );
    // Drive the destination-injection seam directly: deriving the path from
    // the process-global HOME races with sibling suites that flip HOME. The
    // seam takes the Downloads root itself, so pass the known fresh path.
    let root = tempfile::tempdir().unwrap();
    let downloads = root.path().join("Downloads");
    view.save_all_attachments_to(&mut ctx, Some(&downloads));

    let matches: Vec<String> = std::fs::read_dir(&downloads)
        .expect("Downloads must exist after a successful save")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("meli-请") && name.contains("-one"))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one output directory for this subject, got {matches:?}"
    );
    let dir_name = matches[0].as_str();
    assert!(
        dir_name.len() <= 255,
        "full directory name must stay under the filesystem limit: {dir_name:?}"
    );
    let component = dir_name
        .strip_prefix("meli-")
        .expect("name starts with the meli- prefix");
    let component = component
        .rsplit_once('-')
        .filter(|(_, digits)| !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
        .map_or(component, |(stem, _)| stem);
    assert!(
        component.chars().count() <= 128,
        "title component must hold at most 128 chars, got {}",
        component.chars().count()
    );
    assert!(
        component.len() <= 240,
        "title component must hold at most 240 bytes, got {}",
        component.len()
    );
    assert!(
        component.contains("..."),
        "truncated title must contain the ellipsis, got {component:?}"
    );
    let dir = downloads.join(dir_name);

    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .expect("output directory must be readable")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(
        files,
        vec!["inline-image.png".to_string(), "report.pdf".to_string()],
        "exactly the two attachment-like parts must be saved"
    );
    for (file, marker) in [
        ("report.pdf", "%PDF-fake"),
        ("inline-image.png", "PNG-fake"),
    ] {
        let path = dir.join(file);
        let content = std::fs::read(&path).unwrap();
        assert!(
            String::from_utf8_lossy(&content).contains(marker),
            "{file} must contain {marker:?}"
        );
        let mode = PermissionsExt::mode(&std::fs::metadata(&path).unwrap().permissions());
        assert_eq!(
            mode & 0o777,
            0o600,
            "{file} must be readable/writable by owner only"
        );
    }

    let replies: Vec<UIEvent> = ctx.replies().into_iter().collect();
    assert!(
        replies
            .iter()
            .any(|ev| matches!(ev, UIEvent::Notification { body, .. }
                if body.contains("Downloads") && body.contains("Saved 2 attachment(s)"))),
        "a single summary notification must report the save location, got {replies:?}"
    );
}

/// A mail with no attachment-like parts (single-part text/plain) must
/// produce the "No attachments to save." notification and must not create
/// any output directory.
#[test]
fn save_all_attachments_no_attachments() {
    let mut ctx = mock_context();
    let mut view = attachments_envelope_view(
        &ctx,
        "plain-no-att-two",
        "save-all-2@x.example",
        "MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nplain body \
         two\r\n",
    );
    // This test keeps the full process_event dispatch (Action→arm wiring);
    // the no-attachments path returns before touching any directory, so the
    // absence check can use a fresh tempdir without pinning HOME.
    let fresh = tempfile::tempdir().unwrap();
    let downloads = fresh.path().join("Downloads");

    trigger_save_all_attachments(&mut view, &mut ctx);

    let replies: Vec<UIEvent> = ctx.replies().into_iter().collect();
    assert!(
        replies
            .iter()
            .any(|ev| matches!(ev, UIEvent::Notification { body, .. }
                if body.as_ref() == "No attachments to save.")),
        "expected the no-attachments notification, got {replies:?}"
    );
    let no_dir = std::fs::read_dir(&downloads)
        .map(|entries| {
            entries.flatten().all(|entry| {
                !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("meli-plain")
            })
        })
        .unwrap_or(true);
    assert!(
        no_dir,
        "no output directory may be created when there is nothing to save"
    );
}

#[test]
fn save_all_attachments_shortcut() {
    // The default Ctrl-s envelope-view shortcut must dispatch the same batch
    // save as the `save-all-attachment` command through the real input path.
    let mut ctx = mock_context();
    let mut view = attachments_envelope_view(
        &ctx,
        "shortcut-five",
        "save-all-5@x.example",
        ATTACHMENTS_MULTIPART_BODY,
    );

    let mut event = UIEvent::Input(Key::Ctrl('s'));
    _ = view.process_event(&mut event, &mut ctx);

    let replies: Vec<UIEvent> = ctx.replies().into_iter().collect();
    assert!(
        replies
            .iter()
            .any(|ev| matches!(ev, UIEvent::Notification { body, .. }
            if body.contains("Saved 2 attachment(s)"))),
        "Ctrl-s must trigger the save-all-attachments summary notification, got {replies:?}"
    );
}

/// When `~/Downloads/meli-<title>` already exists, the batch save must move
/// to the `-2` suffixed directory instead of writing into the occupied one.
#[test]
fn save_all_attachments_dir_conflict() {
    let mut ctx = mock_context();
    let view = attachments_envelope_view(
        &ctx,
        "conflict-three",
        "save-all-3@x.example",
        ATTACHMENTS_MULTIPART_BODY,
    );
    let root = tempfile::tempdir().unwrap();
    let downloads = root.path().join("Downloads");
    std::fs::create_dir_all(downloads.join("meli-conflict-three")).unwrap();
    view.save_all_attachments_to(&mut ctx, Some(&downloads));

    let dir = downloads.join("meli-conflict-three-2");
    assert!(dir.is_dir(), "save must land in the -2 suffixed directory");
    let content =
        std::fs::read(dir.join("report.pdf")).expect("report.pdf must be in the -2 directory");
    assert!(String::from_utf8_lossy(&content).contains("%PDF-fake"));
    assert!(dir.join("inline-image.png").is_file());
}

/// An inline part with a filename sitting at tree position 0 (a
/// single-part, non-multipart mail) must be saved: the enumeration must
/// find it even though `open_attachment`'s lidx==0 filter would drop it.
#[test]
fn save_all_attachments_solo_inline() {
    let mut ctx = mock_context();
    let view = attachments_envelope_view(
        &ctx,
        "solo-inline-four",
        "save-all-4@x.example",
        "MIME-Version: 1.0\r\nContent-Type: image/png; name=\"solo.png\"\r\n\
         Content-Disposition: inline; filename=\"solo.png\"\r\n\r\nSOLO-fake\r\n",
    );
    let root = tempfile::tempdir().unwrap();
    let downloads = root.path().join("Downloads");
    view.save_all_attachments_to(&mut ctx, Some(&downloads));

    let content = std::fs::read(downloads.join("meli-solo-inline-four").join("solo.png"))
        .expect("the inline part at tree position 0 must be saved as solo.png");
    assert!(
        String::from_utf8_lossy(&content).contains("SOLO-fake"),
        "saved solo.png must contain the fixture body"
    );
}

/// CVE-2024-43604 corpus (Outlook for Android local privilege escalation,
/// mapped onto meli's attachment-save surface): a multipart/mixed mail
/// whose every attachment filename carries a path element or hostile
/// byte — a relative `../` chain aimed one level above the destination,
/// an absolute path (which `PathBuf::push` treats as full replacement),
/// an RFC 2047-encoded traversal (decoded verbatim by
/// `Attachment::filename`), a Windows-style backslash chain, an
/// RFC 2047-encoded ANSI control sequence, and the bare `..` special
/// component. The `../../` chains are calibrated to land inside the
/// test sandbox's root (exactly two levels up) so an escaping save is
/// observable without touching the real filesystem outside `tempdir`.
const TRAVERSAL_MULTIPART_BODY: &str = "MIME-Version: 1.0\r\n\
     Content-Type: multipart/mixed; boundary=\"=_cve-43604\"\r\n\
     \r\n\
     --=_cve-43604\r\n\
     Content-Type: application/octet-stream\r\n\
     Content-Disposition: attachment; filename=\"../../pwned-relative.txt\"\r\n\
     \r\n\
     RELATIVE-fake\r\n\
     --=_cve-43604\r\n\
     Content-Type: application/octet-stream\r\n\
     Content-Disposition: attachment; filename=\"/../../pwned-absolute.txt\"\r\n\
     \r\n\
     ABSOLUTE-fake\r\n\
     --=_cve-43604\r\n\
     Content-Type: application/octet-stream\r\n\
     Content-Disposition: attachment; filename=\"=?UTF-8?Q?..=2F..=2Fpwned-rfc2047.txt?=\"\r\n\
     \r\n\
     RFC2047-fake\r\n\
     --=_cve-43604\r\n\
     Content-Type: application/octet-stream\r\n\
     Content-Disposition: attachment; filename=\"..\\..\\pwned-backslash.txt\"\r\n\
     \r\n\
     BACKSLASH-fake\r\n\
     --=_cve-43604\r\n\
     Content-Type: application/octet-stream\r\n\
     Content-Disposition: attachment; filename=\"=?UTF-8?Q?evil=1B=5B2Jpwned-control.txt?=\"\r\n\
     \r\n\
     CONTROL-fake\r\n\
     --=_cve-43604\r\n\
     Content-Type: application/octet-stream\r\n\
     Content-Disposition: attachment; filename=\"..\"\r\n\
     \r\n\
     DOTDOT-fake\r\n\
     --=_cve-43604--\r\n";

/// Fire the `save-attachment <idx> <path>` view action at `view`, like
/// the parsed user command would.
fn trigger_save_attachment(
    view: &mut EnvelopeView,
    context: &mut Context,
    idx: usize,
    path: &Path,
) {
    let mut event = UIEvent::Action(Action::View(ViewAction::SaveAttachment(
        idx,
        FileAction::Path(path.display().to_string()),
    )));
    _ = view.process_event(&mut event, context);
}

/// Names directly inside `dir`.
fn dir_entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("{} must be readable: {err}", dir.display()))
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Every name written by the save paths must be a single flat
/// component: no separators, no control characters, no `.`/`..`.
fn assert_flat_names(dir: &Path) -> Vec<String> {
    let names = dir_entries(dir);
    for name in &names {
        assert!(!name.contains('/'), "escaped separator in {name:?}");
        assert!(!name.contains('\\'), "escaped backslash in {name:?}");
        assert!(
            name.chars().all(|c| !c.is_control()),
            "control character in {name:?}"
        );
        assert!(
            !matches!(name.as_str(), "" | "." | ".."),
            "special name {name:?}"
        );
    }
    names
}

/// CVE-2024-43604 regression (single attachment save): saving each
/// corpus attachment into an existing directory must land a flat,
/// sanitized file inside it — never a `../` escape into the parent,
/// never an absolute-path replacement of the destination, and the bare
/// `..` filename must fall back to a generated name.
#[test]
fn save_attachment_directory_traversal_is_flattened() {
    let mut ctx = mock_context();
    let mut view = attachments_envelope_view(
        &ctx,
        "cve-43604-single",
        "traversal-single@x.example",
        TRAVERSAL_MULTIPART_BODY,
    );

    let tempdir = tempfile::tempdir().unwrap();
    // Two levels deep: the corpus `../../` chains would land directly in
    // the sandbox root if they ever escaped.
    let dir = tempdir.path().join("d1").join("d2");
    std::fs::create_dir_all(&dir).unwrap();

    for idx in 1..=6 {
        trigger_save_attachment(&mut view, &mut ctx, idx, &dir);
    }

    let names = assert_flat_names(&dir);
    assert_eq!(
        names.len(),
        6,
        "all six corpus attachments must land inside the directory: {names:?}"
    );
    // Each sanitized name keeps enough of its original to stay
    // distinguishable, and the bare `..` fell back to a generated name.
    let joined = names.join(" ");
    assert!(
        joined.contains("pwned-relative.txt") && joined.contains("pwned-absolute.txt"),
        "sanitized names must stay recognizable: {names:?}"
    );
    assert!(
        joined.contains("pwned-rfc2047.txt"),
        "the RFC 2047-decoded traversal must be flattened: {names:?}"
    );
    assert!(
        joined.contains("pwned-control.txt"),
        "the ANSI control-character name must be cleaned: {names:?}"
    );
    assert!(
        names.iter().any(|n| n.starts_with("meli_attachment_")),
        "the bare `..` filename must fall back to a generated name: {names:?}"
    );
    // No escape: the two parent levels and the sandbox root contain
    // nothing but the chain of directories leading to `dir`.
    assert_eq!(
        dir_entries(&tempdir.path().join("d1")),
        vec!["d2".to_string()],
        "nothing may escape into d1"
    );
    assert_eq!(
        dir_entries(tempdir.path()),
        vec!["d1".to_string()],
        "the calibrated `../../` chains must not land in the sandbox root"
    );
    // The right bytes landed: every marker is present exactly once
    // across the saved files.
    let mut markers = [
        "RELATIVE-fake",
        "ABSOLUTE-fake",
        "RFC2047-fake",
        "BACKSLASH-fake",
        "CONTROL-fake",
        "DOTDOT-fake",
    ]
    .iter()
    .map(|m| (m.to_string(), 0))
    .collect::<IndexMap<String, usize>>();
    for name in &names {
        let content = std::fs::read(dir.join(name)).unwrap();
        let text = String::from_utf8_lossy(&content);
        for (marker, count) in markers.iter_mut() {
            if text.contains(marker.as_str()) {
                *count += 1;
            }
        }
    }
    for (marker, count) in markers {
        assert_eq!(count, 1, "marker {marker} must land in exactly one file");
    }
}

/// CVE-2003-0376 regression (issue #47, Eudora 5.2.1 "Attachment
/// Converted" dot-pile overflow): saving an attachment whose
/// mail-controlled filename is the canonical Eudora dot-pile shape
/// (`a......................exe`) or a megabyte-scale dot pile must
/// land one flat, bounded component — the trigger bytes stay inert
/// filename text, and the over-long name is capped on a character
/// boundary instead of failing the write.
#[test]
fn save_attachment_dot_pile_and_overlength_names_are_flat_and_capped() {
    let body = format!(
        "MIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=\"=_cve-2003-0376\"\r\n\
         \r\n\
         --=_cve-2003-0376\r\n\
         Content-Type: application/octet-stream\r\n\
         Content-Disposition: attachment; filename=\"a{}.exe\"\r\n\
         \r\n\
         DOTPILE-fake\r\n\
         --=_cve-2003-0376\r\n\
         Content-Type: application/octet-stream\r\n\
         Content-Disposition: attachment; filename=\"a{}.exe\"\r\n\
         \r\n\
         OVERLONG-fake\r\n\
         --=_cve-2003-0376--\r\n",
        ".".repeat(22),
        ".".repeat(100_000)
    );
    let mut ctx = mock_context();
    let mut view = attachments_envelope_view(&ctx, "cve-2003-0376", "dot-pile@x.example", &body);

    let tempdir = tempfile::tempdir().unwrap();
    let dir = tempdir.path().join("d1").join("d2");
    std::fs::create_dir_all(&dir).unwrap();

    for idx in 1..=2 {
        trigger_save_attachment(&mut view, &mut ctx, idx, &dir);
    }

    let names = assert_flat_names(&dir);
    assert_eq!(
        names.len(),
        2,
        "both corpus attachments must land inside the directory: {names:?}"
    );
    // The canonical 22-dot Eudora shape survives verbatim: a pile of
    // dots is inert filename text, nothing to normalize away.
    assert!(
        names
            .iter()
            .any(|n| n == &format!("a{}.exe", ".".repeat(22))),
        "the dot-pile trigger name must land verbatim: {names:?}"
    );
    // The megabyte-scale pile is capped into one usable component —
    // the cut lands inside the dot run, so the capped name is the
    // recognizable `a` + dots prefix.
    assert!(
        names
            .iter()
            .any(|n| n.len() <= crate::types::FILENAME_COMPONENT_MAX_BYTES
                && n.starts_with('a')
                && n.chars().skip(1).all(|c| c == '.')),
        "the over-long name must land capped and recognizable: {names:?}"
    );
    assert_eq!(
        dir_entries(&tempdir.path().join("d1")),
        vec!["d2".to_string()]
    );
    assert_eq!(dir_entries(tempdir.path()), vec!["d1".to_string()]);
    // The right bytes landed.
    let joined = names
        .iter()
        .map(|n| String::from_utf8_lossy(&std::fs::read(dir.join(n)).unwrap()).into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(joined.contains("DOTPILE-fake"), "{joined:?}");
    assert!(joined.contains("OVERLONG-fake"), "{joined:?}");
}

/// CVE-1999-0427 regression (display): opening a mail whose attachment
/// name is 64 KiB long (the CVE's denial-of-service width) must build
/// the view, embed the name in the attachment tree, and render the
/// pager bounded at real and degenerate terminal sizes — the
/// over-long name becomes wrapped continuation lines, never a wedge.
#[test]
fn envelope_view_overlong_attachment_name_renders_bounded() {
    let name = format!("CVE19990427VIEW-{}", "A".repeat(64 * 1024));
    let body = format!(
        "MIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=\"=_cve-1999-0427\"\r\n\
         \r\n\
         --=_cve-1999-0427\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         \r\n\
         body\r\n\
         --=_cve-1999-0427\r\n\
         Content-Type: application/octet-stream\r\n\
         Content-Disposition: attachment; filename=\"{name}\"\r\n\
         \r\n\
         BINARY-fake\r\n\
         --=_cve-1999-0427--\r\n"
    );
    let mut ctx = mock_context();
    let start = std::time::Instant::now();
    let mut view = attachments_envelope_view(&ctx, "cve-1999-0427", "overlong@x.example", &body);
    assert!(
        view.attachment_tree.contains("CVE19990427VIEW-"),
        "the attachment tree must embed the over-long name"
    );
    let theme_default = crate::conf::value(&ctx, "theme_default");
    for (cols, rows) in [(80usize, 24usize), (8, 4), (2, 1)] {
        view.set_dirty(true);
        let mut screen = Screen::<Virtual>::new(theme_default);
        assert!(screen.resize(cols, rows), "({cols}x{rows}) must resize");
        let area = screen.area();
        view.draw(screen.grid_mut(), area, &mut ctx);
    }
    assert!(
        start.elapsed() < std::time::Duration::from_secs(5),
        "the over-long-name mail must render bounded (took {:?})",
        start.elapsed()
    );
}

/// CVE-1999-0427 regression (batch save): `save-all-attachments` on a
/// mail whose every attachment name is over-long (64 KiB+, the CVE's
/// denial-of-service width) must land every file capped into one flat
/// component cut on a character boundary, deduping cap-colliding
/// twins, with the right bytes and permissions under each.
#[test]
fn save_all_attachments_overlength_names_land_capped_and_deduped() {
    use std::os::unix::fs::PermissionsExt;

    // Two names sharing their first 192 bytes: after the component cap
    // they collide, forcing the `_1` dedup suffix on the second.
    let long_a = format!("CVE19990427SAVE-{}", "A".repeat(64 * 1024));
    let long_a_twin = format!("CVE19990427SAVE-{}B", "A".repeat(64 * 1024));
    let long_cjk = "攻".repeat(64 * 1024 / 3);
    let body = format!(
        "MIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=\"=_cve-1999-0427\"\r\n\
         \r\n\
         --=_cve-1999-0427\r\n\
         Content-Type: application/octet-stream\r\n\
         Content-Disposition: attachment; filename=\"{long_a}\"\r\n\
         \r\n\
         AAA-fake\r\n\
         --=_cve-1999-0427\r\n\
         Content-Type: application/octet-stream\r\n\
         Content-Disposition: attachment; filename=\"{long_a_twin}\"\r\n\
         \r\n\
         TWIN-fake\r\n\
         --=_cve-1999-0427\r\n\
         Content-Type: application/octet-stream\r\n\
         Content-Disposition: attachment; filename=\"{long_cjk}\"\r\n\
         \r\n\
         CJK-fake\r\n\
         --=_cve-1999-0427--\r\n"
    );
    let mut ctx = mock_context();
    let view = attachments_envelope_view(&ctx, "cve-1999-0427", "save-all@x.example", &body);
    let root = tempfile::tempdir().unwrap();
    let downloads = root.path().join("Downloads");

    let start = std::time::Instant::now();
    view.save_all_attachments_to(&mut ctx, Some(&downloads));
    assert!(
        start.elapsed() < std::time::Duration::from_secs(5),
        "the batch save must stay bounded on over-long names (took {:?})",
        start.elapsed()
    );

    let out_dirs: Vec<_> = std::fs::read_dir(&downloads)
        .expect("Downloads must exist after a successful save")
        .flatten()
        .map(|entry| entry.path())
        .collect();
    assert_eq!(
        out_dirs.len(),
        1,
        "exactly one meli-<subject> directory must appear, got {out_dirs:?}"
    );
    let dir = out_dirs[0].clone();
    let names = assert_flat_names(&dir);
    let capped_a = format!(
        "CVE19990427SAVE-{}",
        "A".repeat(192 - "CVE19990427SAVE-".len())
    );
    let expected = vec![
        capped_a.clone(),
        format!("{capped_a}_1"),
        "攻".repeat(crate::types::FILENAME_COMPONENT_MAX_BYTES / 3),
    ];
    assert_eq!(
        names, expected,
        "every over-long name must land capped, deduped and recognizable"
    );
    for (name, marker) in [
        (&expected[0], "AAA-fake"),
        (&expected[1], "TWIN-fake"),
        (&expected[2], "CJK-fake"),
    ] {
        let path = dir.join(name);
        let content =
            std::fs::read(&path).unwrap_or_else(|err| panic!("{name:?} must land: {err}"));
        assert!(
            String::from_utf8_lossy(&content).contains(marker),
            "{name:?} must contain {marker:?}"
        );
        let mode = PermissionsExt::mode(&std::fs::metadata(&path).unwrap().permissions());
        assert_eq!(mode & 0o777, 0o600, "{name:?} must be owner-only");
    }

    let replies: Vec<UIEvent> = ctx.replies().into_iter().collect();
    assert!(
        replies
            .iter()
            .any(|ev| matches!(ev, UIEvent::Notification { body, .. }
            if body.contains("Saved 3 attachment(s)"))),
        "a single summary notification must report the save, got {replies:?}"
    );
}

/// CVE-2024-43604 review lock (batch save): deduplication must run on
/// the *sanitized* name. Two different hostile spellings of one
/// component (`../../twin.bin` and `..\..\twin.bin`) flatten to the
/// same name; the second must land with the `_1` suffix inserted
/// before its extension — if dedup ran on the raw mail-controlled
/// names first, these distinct spellings would collide only at the
/// `create_new` write and one attachment would be silently dropped.
#[test]
fn save_all_attachments_sanitized_collisions_are_deduped() {
    let body = "MIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=\"=_cve-43604-dedup\"\r\n\
         \r\n\
         --=_cve-43604-dedup\r\n\
         Content-Type: application/octet-stream\r\n\
         Content-Disposition: attachment; filename=\"../../twin.bin\"\r\n\
         \r\n\
         FIRST-twin\r\n\
         --=_cve-43604-dedup\r\n\
         Content-Type: application/octet-stream\r\n\
         Content-Disposition: attachment; filename=\"..\\..\\twin.bin\"\r\n\
         \r\n\
         SECOND-twin\r\n\
         --=_cve-43604-dedup\r\n\
         Content-Type: application/pdf\r\n\
         Content-Disposition: attachment; filename=\"report.pdf\"\r\n\
         \r\n\
         PDF-plain\r\n\
         --=_cve-43604-dedup--\r\n";
    let mut ctx = mock_context();
    let view = attachments_envelope_view(&ctx, "cve-43604-dedup", "dedup@x.example", body);
    let root = tempfile::tempdir().unwrap();
    let downloads = root.path().join("Downloads");

    view.save_all_attachments_to(&mut ctx, Some(&downloads));

    let out_dirs: Vec<_> = std::fs::read_dir(&downloads)
        .expect("Downloads must exist after a successful save")
        .flatten()
        .map(|entry| entry.path())
        .collect();
    assert_eq!(
        out_dirs.len(),
        1,
        "exactly one meli-<subject> directory must appear, got {out_dirs:?}"
    );
    let dir = out_dirs[0].clone();
    let mut names = assert_flat_names(&dir);
    names.sort();
    assert_eq!(
        names,
        vec![
            ".._.._twin.bin".to_string(),
            ".._.._twin_1.bin".to_string(),
            "report.pdf".to_string(),
        ],
        "sanitized twins must both land, the second deduped"
    );
    for (name, marker) in [
        (".._.._twin.bin", "FIRST-twin"),
        (".._.._twin_1.bin", "SECOND-twin"),
        ("report.pdf", "PDF-plain"),
    ] {
        let content =
            std::fs::read(dir.join(name)).unwrap_or_else(|err| panic!("{name:?} must land: {err}"));
        assert!(
            String::from_utf8_lossy(&content).contains(marker),
            "{name:?} must contain {marker:?}"
        );
    }
    let replies: Vec<UIEvent> = ctx.replies().into_iter().collect();
    assert!(
        replies
            .iter()
            .any(|ev| matches!(ev, UIEvent::Notification { body, .. }
            if body.contains("Saved 3 attachment(s)"))),
        "all three attachments must be reported saved, got {replies:?}"
    );
}

/// CVE-2002-2351 regression (issue #61, Eudora 5.1 trailing-dot
/// executable-attachment warning bypass): the batch save must land the
/// normalized component, with the trailing dot run stripped, so the
/// name meli displays/checks and the name a Win32-semantics filesystem
/// (`/mnt/c` drvfs, Samba, some FUSE) creates agree — the divergence
/// that let `evil.exe.` bypass Eudora's warning while landing as
/// `evil.exe`.
#[test]
fn save_all_attachments_trailing_dot_names_are_normalized() {
    let body = "MIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=\"=_cve-2002-2351\"\r\n\
         \r\n\
         --=_cve-2002-2351\r\n\
         Content-Type: application/x-msdownload\r\n\
         Content-Disposition: attachment; filename=\"evil.exe.\"\r\n\
         \r\n\
         EXE-fake\r\n\
         --=_cve-2002-2351\r\n\
         Content-Type: application/pdf\r\n\
         Content-Disposition: attachment; filename=\"report.pdf.\"\r\n\
         \r\n\
         PDF-fake\r\n\
         --=_cve-2002-2351--\r\n";
    let mut ctx = mock_context();
    let view = attachments_envelope_view(&ctx, "cve-2002-2351", "trailing-dot@x.example", body);
    let root = tempfile::tempdir().unwrap();
    let downloads = root.path().join("Downloads");

    view.save_all_attachments_to(&mut ctx, Some(&downloads));

    let out_dirs: Vec<_> = std::fs::read_dir(&downloads)
        .expect("Downloads must exist after a successful save")
        .flatten()
        .map(|entry| entry.path())
        .collect();
    assert_eq!(
        out_dirs.len(),
        1,
        "exactly one meli-<subject> directory must appear, got {out_dirs:?}"
    );
    let dir = out_dirs[0].clone();
    let mut names = assert_flat_names(&dir);
    names.sort();
    assert_eq!(
        names,
        vec!["evil.exe".to_string(), "report.pdf".to_string()],
        "every landed name must have its trailing dot run stripped"
    );
    for (name, marker) in [("evil.exe", "EXE-fake"), ("report.pdf", "PDF-fake")] {
        assert!(!name.ends_with('.'), "{name:?} must not end with a dot");
        let content =
            std::fs::read(dir.join(name)).unwrap_or_else(|err| panic!("{name:?} must land: {err}"));
        assert!(
            String::from_utf8_lossy(&content).contains(marker),
            "{name:?} must contain {marker:?}"
        );
    }
    let replies: Vec<UIEvent> = ctx.replies().into_iter().collect();
    assert!(
        replies
            .iter()
            .any(|ev| matches!(ev, UIEvent::Notification { body, .. }
            if body.contains("Saved 2 attachment(s)"))),
        "both attachments must be reported saved, got {replies:?}"
    );
}

/// CVE-2024-43604 regression (whole-message save): the `.eml` filename
/// derived from the mail-controlled `Message-ID` used to reach
/// `PathBuf::push` unsanitized, so `save-attachment 0 <dir>` on a mail
/// with `Message-ID: <../../pwned-eml>` wrote outside the destination
/// (and an absolute `Message-ID` replaced it outright). It must be
/// sanitized into one flat component.
#[test]
fn save_attachment_eml_message_id_traversal_is_flattened() {
    for hostile_msgid in [
        "../../pwned-eml",
        "/../../pwned-eml",
        "..",
        "evil=1B-control",
    ] {
        let mut ctx = mock_context();
        let mut view = attachments_envelope_view(
            &ctx,
            "cve-43604-eml",
            hostile_msgid,
            "MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\neml \
             body\r\n",
        );

        let tempdir = tempfile::tempdir().unwrap();
        let dir = tempdir.path().join("d1").join("d2");
        std::fs::create_dir_all(&dir).unwrap();

        trigger_save_attachment(&mut view, &mut ctx, 0, &dir);

        let names = assert_flat_names(&dir);
        assert_eq!(
            names.len(),
            1,
            "exactly one .eml file must land inside the directory: {names:?}"
        );
        let content = std::fs::read(dir.join(&names[0])).unwrap();
        assert!(
            String::from_utf8_lossy(&content).contains("eml body"),
            "the whole mail must be saved"
        );
        assert_eq!(
            dir_entries(&tempdir.path().join("d1")),
            vec!["d2".to_string()],
            "the Message-ID traversal must not escape d1"
        );
        assert_eq!(
            dir_entries(tempdir.path()),
            vec!["d1".to_string()],
            "the Message-ID traversal must not reach the sandbox root"
        );
    }
}

/// CVE-2024-43604 regression (batch save): `save-all-attachments` on the
/// corpus mail must write every part flat inside the fresh
/// `meli-<subject>` directory — nothing in `Downloads` besides it,
/// nothing in the sandbox root, no separators or control bytes in any
/// name, and the bare `..` filename falls back to a generated one.
#[test]
fn save_all_attachments_traversal_stays_inside_directory() {
    let mut ctx = mock_context();
    let view = attachments_envelope_view(
        &ctx,
        "cve-43604-batch",
        "traversal-batch@x.example",
        TRAVERSAL_MULTIPART_BODY,
    );

    let tempdir = tempfile::tempdir().unwrap();
    let downloads = tempdir.path().join("Downloads");
    view.save_all_attachments_to(&mut ctx, Some(&downloads));

    assert_eq!(
        dir_entries(&downloads),
        vec!["meli-cve-43604-batch".to_string()],
        "Downloads must contain exactly the fresh save directory"
    );
    let dir = downloads.join("meli-cve-43604-batch");
    let names = assert_flat_names(&dir);
    assert_eq!(
        names.len(),
        6,
        "all six corpus parts must be saved flat: {names:?}"
    );
    assert!(
        names.iter().any(|n| n.starts_with("meli_attachment_")),
        "the bare `..` filename must fall back to a generated name: {names:?}"
    );
    assert_eq!(
        dir_entries(tempdir.path()),
        vec!["Downloads".to_string()],
        "no traversal may reach the sandbox root"
    );
}

/// Regression for the `MailViewTab` panic when opening a mail in a new tab
/// (Enter on a thread while `ThreadViewFocus::MailView` is focused).
///
/// `State::process_realizations` walks the realized component tree from the
/// root through `Component::children()` and asserts every realized id
/// resolves back to itself. `MailViewTab` used to fall back to the default
/// empty `children()`, so its embedded `MailView` could not be resolved and
/// `state.rs`'s `Option::unwrap()` panicked. This replays that algorithm on
/// a `Tabbed` -> `MailViewTab` -> `MailView` tree.
#[test]
fn mailview_tab_children_resolve() {
    let mut ctx = mock_context();
    let coordinates = insert_list_unsubscribe_envelope(&ctx);
    let mailview = MailView::new(Some(coordinates), false, &mut ctx);
    let mailview_id = mailview.id();

    let mut tabbed = Tabbed::new(Vec::new(), &ctx);
    let tabbed_id = tabbed.id();
    ctx.realized.clear();
    tabbed.realize(None, &mut ctx);
    let mailviewtab = MailViewTab::new(Box::new(mailview));
    let mailviewtab_id = mailviewtab.id();
    tabbed.add_component(Box::new(mailviewtab), &mut ctx);

    let components: IndexMap<ComponentId, &dyn Component> =
        std::iter::once((tabbed_id, &tabbed as &dyn Component)).collect();
    let mut component_tree: IndexMap<ComponentId, ComponentPath> = IndexMap::default();
    // Mirrors `State::process_realizations` (meli/src/state.rs).
    while let Some((id, parent)) = ctx.realized.pop() {
        match parent {
            None => {
                component_tree.insert(id, ComponentPath::new(id));
            }
            Some(parent) if component_tree.contains_key(&parent) => {
                let mut v = component_tree[&parent].clone();
                v.push_front(id);
                if let Some(p) = v.root() {
                    assert_eq!(
                        v.resolve(components[p]).unwrap().id(),
                        id,
                        "realized component {id} must resolve through children()"
                    );
                }
                component_tree.insert(id, v);
            }
            Some(parent) if !ctx.realized.contains_key(&parent) => {
                component_tree.insert(id, ComponentPath::new(id));
            }
            Some(_) => {
                let from_index = ctx.realized.len();
                ctx.realized.insert(id, parent);
                ctx.realized.move_index(from_index, 0);
            }
        }
    }

    let tab_path = component_tree
        .get(&mailviewtab_id)
        .expect("the MailViewTab must be realized under Tabbed");
    assert_eq!(tab_path.root(), Some(&tabbed_id));
    assert_eq!(
        tab_path.resolve(components[&tabbed_id]).unwrap().id(),
        mailviewtab_id,
        "the MailViewTab path must resolve to itself through Tabbed::children()"
    );

    let path = component_tree
        .get(&mailview_id)
        .expect("the MailView inside MailViewTab must be realized");
    assert_eq!(path.root(), Some(&tabbed_id));
    assert_eq!(
        path.resolve(components[&tabbed_id]).unwrap().id(),
        mailview_id,
        "the MailView path must resolve to itself through MailViewTab::children()"
    );
}

/// Build a `message/rfc822` mail nested `depth` levels deep: each level
/// is a complete inner mail whose sole body is the next level, with a
/// plain `text/plain` core. This is the CVE-2024-21378 class payload
/// for meli's open-mail surface: opening the mail must drive every
/// nesting level through the display recursion.
fn nested_rfc822_mail(depth: usize) -> Vec<u8> {
    let mut m = String::from(
        "From: a@b.example\r\nSubject: nest\r\nDate: Thu, 1 Jan 2026 00:00:00 +0000\r\n\
         Content-Type: message/rfc822\r\n\r\n",
    );
    for _ in 0..depth {
        m.push_str(
            "From: a@b.example\r\nSubject: inner\r\nDate: Thu, 1 Jan 2026 00:00:00 +0000\r\n\
             Content-Type: message/rfc822\r\n\r\n",
        );
    }
    m.push_str("From: a@b.example\r\nSubject: core\r\n\r\nhello");
    m.into_bytes()
}

/// Maximum `InlineRfc822` chain length in a display tree.
fn rfc822_display_depth(display: &[AttachmentDisplay]) -> usize {
    display
        .iter()
        .map(|d| match d {
            AttachmentDisplay::InlineRfc822 { display, .. } => 1 + rfc822_display_depth(display),
            AttachmentDisplay::Alternative { display, .. }
            | AttachmentDisplay::Mixed { display, .. } => rfc822_display_depth(display),
            AttachmentDisplay::SignedPending { display, .. }
            | AttachmentDisplay::SignedFailed { display, .. }
            | AttachmentDisplay::SignedVerified { display, .. }
            | AttachmentDisplay::SignedUnverified { display, .. } => rfc822_display_depth(display),
            AttachmentDisplay::InlineText { .. }
            | AttachmentDisplay::InlineOther { .. }
            | AttachmentDisplay::Attachment { .. }
            | AttachmentDisplay::EncryptedPending { .. }
            | AttachmentDisplay::EncryptedFailed { .. }
            | AttachmentDisplay::EncryptedSuccess { .. } => 0,
        })
        .max()
        .unwrap_or(0)
}

/// CVE-2024-21378 (issue #23, table 2 of the CVE research report —
/// virus / code execution): opening a crafted mail must never corrupt
/// the client. meli's equivalent surface is the open-mail display
/// recursion: `attachment_to_display_helper` used to recurse once per
/// `message/rfc822` level with no bound, and each level costs tens of
/// KiB of stack (`Mail::new` + envelope parsing + this recursion) — a
/// mail a few KiB long, nested a few thousand levels deep, overflowed
/// the 2 MiB stack of the view/filter threads and aborted meli the
/// moment it was opened (CWE-674; measured pre-fix threshold: ~26
/// levels on a 2 MiB debug-build thread). The fix caps the recursion
/// at `MAX_RFC822_DISPLAY_NESTING_DEPTH` and presents the remaining
/// subtree as an inert attachment. This regression opens a mail nested
/// 16000 levels deep on the strictest stack a caller uses (2 MiB) and
/// locks both properties: no abort, and a display tree truncated at
/// the cap.
#[test]
fn deeply_nested_rfc822_opens_without_stack_overflow() {
    use std::sync::Arc;

    use crate::{
        jobs::JobExecutor,
        state::MainLoopHandler,
        view::{envelope::MAX_RFC822_DISPLAY_NESTING_DEPTH, EnvelopeView},
    };

    let child = std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            let (sender, _receiver) = crossbeam::channel::unbounded();
            let handler = MainLoopHandler {
                job_executor: Arc::new(JobExecutor::new(sender.clone())),
                sender,
            };
            let mail = melib::Mail::new(nested_rfc822_mail(16000), None).unwrap();
            let view = EnvelopeView::new(mail, None, None, None, handler);
            (
                view.attachment_tree.len(),
                rfc822_display_depth(&view.display),
            )
        })
        .unwrap();
    let (tree_len, depth) = child.join().expect(
        "opening a deeply nested message/rfc822 mail must not overflow the stack (CVE-2024-21378)",
    );
    assert!(tree_len > 0, "attachment tree must still be built");
    assert_eq!(
        depth, MAX_RFC822_DISPLAY_NESTING_DEPTH,
        "display recursion must stop at the nesting cap"
    );
}

/// The cap must not clip legitimate mail: a normally-nested forwarded
/// message (a `message/rfc822` inside a `message/rfc822`) still renders
/// fully inline through the whole chain.
#[test]
fn shallow_rfc822_nesting_still_renders_fully_inline() {
    use std::sync::Arc;

    use crate::{jobs::JobExecutor, state::MainLoopHandler, view::EnvelopeView};

    let child = std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            let (sender, _receiver) = crossbeam::channel::unbounded();
            let handler = MainLoopHandler {
                job_executor: Arc::new(JobExecutor::new(sender.clone())),
                sender,
            };
            let mail = melib::Mail::new(nested_rfc822_mail(2), None).unwrap();
            let view = EnvelopeView::new(mail, None, None, None, handler);
            rfc822_display_depth(&view.display)
        })
        .unwrap();
    // `nested_rfc822_mail(2)` is three `message/rfc822` declarations in
    // total (the outer mail plus two wrapped levels), all of which must
    // render fully inline.
    assert_eq!(
        child.join().unwrap(),
        3,
        "shallow nesting must render every InlineRfc822 level"
    );
}

/// The body-text filter pipeline (`ViewFilter::new_attachment`) recurses
/// into `message/rfc822` through the same unbounded path and shares the
/// fix's depth cap: past the cap the remaining subtree renders as inert
/// text. Locked on the strictest caller stack (2 MiB) at a depth that
/// used to abort the process pre-fix.
#[test]
fn view_filter_deep_rfc822_nesting_renders_inert_text_without_overflow() {
    let att = melib::AttachmentBuilder::new(&nested_rfc822_mail(16000)).build();
    let settings = ViewSettings::default();
    let child = std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            let ctx = mock_context();
            ViewFilter::new_attachment(&att, &settings, &ctx).is_ok()
        })
        .unwrap();
    assert!(
        child
            .join()
            .expect("view filter must survive deep rfc822 nesting"),
        "the deep corpus must still produce a view filter"
    );
}

/// `export-thread` must sanitize each message's `Message-ID` into a flat
/// filename component: the raw header bytes are kept verbatim by melib
/// whenever they do not parse as `<id-left@id-right>`, so an unbracketed
/// traversal spelling reaches the export filename as-is — the CWE-35
/// path-traversal class of CVE-2025-47176 (`'.../...//'` in Outlook).
/// Locked on the exact spellings of that class.
#[test]
fn export_thread_filename_is_flat_for_traversal_message_ids() {
    use super::thread::thread_export_filename;

    for id in [
        "../evil",
        "../../evil",
        ".../...//evil",
        r"..\..\evil",
        r"..\/..//evil",
        "/absolute/evil",
        r"\\attacker\share\evil",
        r"C:\Temp\evil",
        "%2e%2e%2fevil",
        "evil\x00name",
        "..",
    ] {
        let filename = thread_export_filename(&crate::melib::MessageID::new(id));
        assert!(!filename.contains('/'), "{id:?} -> {filename:?}");
        assert!(!filename.contains('\\'), "{id:?} -> {filename:?}");
        assert!(!filename.contains('\0'), "{id:?} -> {filename:?}");
        assert_eq!(
            Path::new(&filename).components().count(),
            1,
            "{id:?} -> {filename:?} must be one flat component"
        );
        assert!(filename.ends_with(".eml"), "{id:?} -> {filename:?}");
    }
}

/// CVE-2024-43604 review lock (`.eml` export): a `Message-ID` made
/// only of control bytes sanitizes down to an empty stem and must take
/// the generated fallback name — pre-review it exported as the bare
/// `.eml` hidden dotfile that every such mail collides on under the
/// `create_new` write. (A mail with *no* `Message-ID` header never
/// reaches this path with an empty identifier: melib synthesizes
/// `<hash>` in `Envelope::populate_headers`.)
#[test]
fn save_whole_mail_degenerate_message_id_gets_generated_name() {
    let mut ctx = mock_context();
    let bytes = b"From: a@b.example\r\nTo: c@d.example\r\nSubject: no usable message id\r\nMessage-ID: <\x1b\x07>\r\nDate: Thu, 1 Jan 2026 00:00:00 +0000\r\nContent-Type: text/plain\r\n\r\nbody\r\n";
    let mail = Mail::new(bytes.to_vec(), None).expect("could not parse test mail");
    assert_eq!(
        mail.message_id().as_str(),
        "\u{1b}\u{7}",
        "the corpus mail must carry the control-only Message-ID verbatim"
    );
    let mut view = EnvelopeView::new(mail, None, None, None, ctx.main_loop_handler.clone());

    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("out");
    std::fs::create_dir(&dir).unwrap();
    let mut event = UIEvent::Action(Action::View(ViewAction::SaveAttachment(
        0,
        FileAction::Path(dir.display().to_string()),
    )));
    _ = view.process_event(&mut event, &mut ctx);

    let names = assert_flat_names(&dir);
    assert_eq!(names.len(), 1, "exactly one file must land: {names:?}");
    assert!(
        names[0].starts_with("meli_mail_") && names[0].ends_with(".eml"),
        "the empty Message-ID must take the generated fallback: {names:?}"
    );
}

/// CVE-2024-43604 review lock (thread export): the per-message export
/// loop claims names *after* sanitization, like the batch attachment
/// save — two thread messages whose hostile identifiers flatten to the
/// same component must both export (the second with the `_1` suffix
/// before its extension), and a degenerate identifier takes the
/// generated fallback instead of a hidden `.eml`.
#[test]
fn export_thread_filenames_dedup_after_sanitization() {
    use super::thread::thread_export_filename;

    let mut used = std::collections::HashSet::new();
    let first = crate::unique_filename_component(
        &mut used,
        &thread_export_filename(&crate::melib::MessageID::new("../../evil")),
    );
    let second = crate::unique_filename_component(
        &mut used,
        &thread_export_filename(&crate::melib::MessageID::new(r"..\..\evil")),
    );
    assert_eq!(first, ".._.._evil.eml");
    assert_eq!(second, ".._.._evil_1.eml");
    let degenerate = crate::unique_filename_component(
        &mut used,
        &thread_export_filename(&crate::melib::MessageID::new("")),
    );
    assert!(
        degenerate.starts_with("meli_mail_") && degenerate.ends_with(".eml"),
        "degenerate identifiers take the generated fallback: {degenerate:?}"
    );
}

/// `save-attachment 0 <dir>` saves the whole mail as `<Message-ID>.eml`
/// inside `dir`: with a raw (unparseable, hence stored-verbatim)
/// traversal `Message-ID`, the file must land as a single flat entry of
/// `dir` — pre-fix, the unsanitized `path.push` escaped the directory
/// (`../escaped` wrote one level up). End-to-end lock of the
/// CVE-2025-47176 CWE-35 fix in `EnvelopeView::save_attachment`.
#[test]
fn save_whole_mail_to_dir_sanitizes_traversal_message_id() {
    let mut ctx = mock_context();
    // No angle brackets: `msg_id` parsing fails, so the raw header bytes
    // are stored verbatim as the envelope's `Message-ID`.
    let bytes = b"From: a@b.example\r\nTo: c@d.example\r\nSubject: whole-mail export\r\nMessage-ID: ../escaped\r\nDate: Thu, 1 Jan 2026 00:00:00 +0000\r\nContent-Type: text/plain\r\n\r\nbody\r\n";
    let mail = Mail::new(bytes.to_vec(), None).expect("could not parse test mail");
    let mut view = EnvelopeView::new(mail, None, None, None, ctx.main_loop_handler.clone());

    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("out");
    std::fs::create_dir(&dir).unwrap();
    let mut event = UIEvent::Action(Action::View(ViewAction::SaveAttachment(
        0,
        FileAction::Path(dir.display().to_string()),
    )));
    _ = view.process_event(&mut event, &mut ctx);

    // The escaped target must not exist …
    assert!(
        !root.path().join("escaped.eml").try_exists().unwrap(),
        "the traversal Message-ID must not write outside the target directory"
    );
    // … and the whole mail landed as exactly one flat file inside `dir`.
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .expect("target directory must be readable")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    assert_eq!(
        files,
        vec![".._escaped.eml".to_string()],
        "the whole-mail export must be one flat file inside the target directory"
    );
    let replies: Vec<UIEvent> = ctx.replies().into_iter().collect();
    assert!(
        replies.iter().any(|ev| matches!(ev, UIEvent::Notification { body, .. }
            if body.contains("Saved at"))),
        "a success notification must report the save, got {replies:?}"
    );
}

// ---------------------------------------------------------------------------
// CVE-2023-4874 (issue #43): viewing structurally-void mails
// ---------------------------------------------------------------------------

/// The CVE-2023-4874 corpus family — mutt > 1.5.2, < 2.2.12 crashed
/// on a NULL-pointer dereference when *viewing* a crafted mail whose
/// structural fields are missing. The family: 无 Message-ID、无
/// From、空 body、无 Content-Type, alone and combined down to the
/// zero-byte mail, plus the void-value spellings (`From: `,
/// `Message-ID: ` headers that exist but carry no value). The
/// envelope-level drives over the same family live in
/// `cve/src/CVE-2023-4874.rs`; this module locks the view rendering
/// itself.
const STRUCTURAL_GAP_CORPUS: &[(&str, &[u8])] = &[
    ("SEPARATOR_ONLY_CRLF", b"\r\n\r\n"),
    ("SEPARATOR_ONLY_LF", b"\n\n"),
    ("BODY_ONLY", b"no header block\r\nno separator\r\njust body bytes\r\n"),
    // The issue's canonical attack shape: no From, no Message-ID, no
    // Content-Type, empty body.
    (
        "MINIMAL_ATTACK",
        b"To: victim@victim.example\r\n\
Subject: crafted\r\n\
Date: Thu, 1 Jan 2026 00:00:00 +0000\r\n\
\r\n",
    ),
    (
        "EMPTY_BODY",
        b"From: a@b.example\r\n\
To: c@d.example\r\n\
Subject: empty body\r\n\
Message-ID: <empty-body@x.example>\r\n\
Date: Thu, 1 Jan 2026 00:00:00 +0000\r\n\
\r\n",
    ),
    (
        "NO_FROM",
        b"To: c@d.example\r\n\
Subject: no from\r\n\
Message-ID: <no-from@x.example>\r\n\
Date: Thu, 1 Jan 2026 00:00:00 +0000\r\n\
\r\n\
body\r\n",
    ),
    (
        "NO_MESSAGE_ID",
        b"From: a@b.example\r\n\
To: c@d.example\r\n\
Subject: no message id\r\n\
Date: Thu, 1 Jan 2026 00:00:00 +0000\r\n\
\r\n\
body\r\n",
    ),
    (
        "NO_CONTENT_TYPE",
        b"From: a@b.example\r\n\
To: c@d.example\r\n\
Subject: legacy plain mail\r\n\
Message-ID: <legacy-plain@x.example>\r\n\
Date: Thu, 1 Jan 2026 00:00:00 +0000\r\n\
\r\n\
plain body without any MIME metadata\r\n",
    ),
    (
        "NO_DATE_NO_SUBJECT",
        b"From: a@b.example\r\n\
To: c@d.example\r\n\
Message-ID: <no-date-no-subject@x.example>\r\n\
\r\n\
body\r\n",
    ),
    (
        "EMPTY_FROM_VALUE",
        b"From: \r\n\
To: c@d.example\r\n\
Subject: void sender\r\n\
Message-ID: <void-from@x.example>\r\n\
Date: Thu, 1 Jan 2026 00:00:00 +0000\r\n\
\r\n\
body\r\n",
    ),
    (
        "EMPTY_MESSAGE_ID_VALUE",
        b"From: a@b.example\r\n\
To: c@d.example\r\n\
Subject: void message id\r\n\
Message-ID: \r\n\
Date: Thu, 1 Jan 2026 00:00:00 +0000\r\n\
\r\n\
body\r\n",
    ),
    (
        "ALL_VOID_VALUES",
        b"From: \r\n\
To: \r\n\
Subject: \r\n\
Message-ID: \r\n\
Date: \r\n\
\r\n",
    ),
];

/// CVE-2023-4874 regression (mutt `< 2.2.12` NULL dereference `DoS` when
/// viewing a crafted mail): meli's `EnvelopeView` must render every
/// structurally-void corpus mail degraded — the sticky header labels
/// (Date/From/To/Subject/Message-ID) still paint over their void or
/// synthesized values, and the body pager still paints — at real and
/// degenerate terminal sizes and through the sticky-header walk,
/// without panicking. In meli's memory model every structural gap is
/// an allocated default (empty address slices, hash-synthesized
/// Message-ID, epoch timestamp), so the mutt NULL-deref semantics
/// have no pointer to dereference.
#[test]
fn missing_structural_fields_mail_renders_degraded_in_envelope_view() {
    let mut ctx = mock_context();
    let theme_default = crate::conf::value(&ctx, "theme_default");

    let mut rendered = 0usize;
    for (name, raw) in STRUCTURAL_GAP_CORPUS {
        let Ok(mail) = Mail::new(raw.to_vec(), None) else {
            // A clean typed rejection is the receive path's own safe
            // degradation; the envelope-level drive over every corpus
            // mail (rejections included) is locked in
            // `cve/src/CVE-2023-4874.rs`.
            continue;
        };
        rendered += 1;
        let mut view = EnvelopeView::new(mail, None, None, None, ctx.main_loop_handler.clone());
        _ = view.process_event(&mut UIEvent::Resize, &mut ctx);

        // Real terminal size first: the header labels must paint —
        // degraded display instead of the crash.
        {
            let mut screen = Screen::<Virtual>::new(theme_default);
            assert!(screen.resize(80, 24), "{name}: screen must resize");
            let area = screen.area();
            view.draw(screen.grid_mut(), area, &mut ctx);
            let grid = screen.grid();
            let text: String = (0..grid.rows())
                .flat_map(|y| (0..grid.cols()).map(move |x| grid[(x, y)].ch()))
                .collect();
            for label in ["Date:", "From:", "To:", "Subject:", "Message-ID:"] {
                assert!(
                    text.contains(label),
                    "{name}: the {label} header label must render degraded, not crash"
                );
            }
        }

        // The sticky-header walk and the pager scroll — every key
        // redraw must stay panic-free.
        for _ in 0..6 {
            let _ = view.process_event(&mut UIEvent::Input(Key::Down), &mut ctx);
            view.set_dirty(true);
            let mut screen = Screen::<Virtual>::new(theme_default);
            assert!(screen.resize(80, 24));
            let area = screen.area();
            view.draw(screen.grid_mut(), area, &mut ctx);
        }

        // Degenerate terminal sizes: the pane shorter than the header
        // block must degrade to a bounded render, not a crash.
        for (cols, rows) in [(8usize, 4usize), (2, 1)] {
            let mut screen = Screen::<Virtual>::new(theme_default);
            assert!(screen.resize(cols, rows), "{name}: ({cols}x{rows}) must resize");
            let area = screen.area();
            view.draw(screen.grid_mut(), area, &mut ctx);
        }
    }
    // The corpus must not silently go vacuous: every mail that
    // parses (all entries but the separator-less BODY_ONLY, which
    // the envelope rejects cleanly) must have rendered through the
    // full view path.
    assert!(
        rendered >= 11,
        "at least 11 corpus mails must render through the view path, got {rendered}"
    );
}

/// Synthetic S/MIME mail: a `multipart/signed` body whose detached
/// signature part is `application/pkcs7-signature` (S/MIME, not
/// `OpenPGP`). The signature bytes are an inert base64 placeholder — the
/// open-view path never parses them, so validity does not matter.
///
/// This is the corpus for `issue #16` / CVE-2008-3068 (S/MIME
/// certificate revocation / AIA-URL retrieval): the regression target
/// is that **opening** such a mail stays entirely local. melib
/// classifies the second part as `ContentType::CMSSignature`, meli's
/// `EnvelopeView` renders the cleartext body synchronously without
/// touching any crypto engine, and the detached-verify job (when
/// dispatched) dies at melib's `protocol` gate before the CMS bytes
/// can reach a PGP engine.
const SMIME_SIGNED_MAIL: &[u8] = b"From: sender@example.com\r\n\
To: victim@example.com\r\n\
Subject: Quarterly receipt\r\n\
Message-ID: <smime-open-1@example.com>\r\n\
Date: Thu, 1 Jan 2026 00:00:00 +0000\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/signed; protocol=\"application/pkcs7-signature\"; micalg=sha-256; boundary=\"=_smime-open\"\r\n\
\r\n\
--=_smime-open\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Quarterly receipt enclosed.\r\n\
--=_smime-open\r\n\
Content-Type: application/pkcs7-signature; name=\"smime.p7s\"\r\n\
Content-Disposition: attachment; filename=\"smime.p7s\"\r\n\
\r\n\
TUlNRS1mYWtl\r\n\
--=_smime-open--\r\n";

/// Construct an `EnvelopeView` for [`SMIME_SIGNED_MAIL`] under `settings`,
/// following the `attachments_envelope_view` construction pattern.
fn smime_envelope_view(context: &Context, settings: ViewSettings) -> EnvelopeView {
    let mail =
        Mail::new(SMIME_SIGNED_MAIL.to_vec(), None).expect("could not parse the S/MIME test mail");
    EnvelopeView::new(
        mail,
        None,
        None,
        Some(settings),
        context.main_loop_handler.clone(),
    )
}

/// Concatenate every locally rendered text payload in the display tree,
/// recursing through the multipart/signed wrapper (and any other
/// multipart variant) via [`AttachmentDisplay::as_multipart`]. This is
/// the offline body reachable straight out of `EnvelopeView::new`, before
/// any asynchronous filter or verify job can complete.
fn collect_rendered_text(displays: &[AttachmentDisplay]) -> String {
    let mut acc = String::new();
    for display in displays {
        if let AttachmentDisplay::InlineText { text, .. } = display {
            acc.push_str(text);
        }
        if let Some(parts) = display.as_multipart() {
            acc.push_str(&collect_rendered_text(parts));
        }
    }
    acc
}

/// Issue #16 / CVE-2008-3068: opening an S/MIME `multipart/signed` mail
/// must render its cleartext body locally, synchronously and offline.
/// When a PGP backend can be instantiated the view dispatches the
/// asynchronous detached-verify job and shows `SignedPending`; otherwise
/// it shows `SignedUnverified`. In **both** cases the display is built
/// from the locally rendered parts — never a `SignedVerified`
/// (synchronous result), and no certificate/AIA/CRL network path is
/// reachable from the open path.
#[test]
fn smime_open_renders_content_offline_without_engine() {
    let ctx = mock_context();
    let settings = ViewSettings {
        auto_verify_signatures: true.into(),
        ..Default::default()
    };
    let backend_available = settings.pgp_backend.instantiate().is_ok();

    let view = smime_envelope_view(&ctx, settings);
    let root = view
        .display
        .first()
        .expect("the multipart/signed root must have a display entry");

    if backend_available {
        assert!(
            matches!(root, AttachmentDisplay::SignedPending { .. }),
            "with an instantiable backend, opening the mail must dispatch the async verify job \
             and display `SignedPending`, got {root:?}"
        );
    } else {
        assert!(
            matches!(root, AttachmentDisplay::SignedUnverified { .. }),
            "without an instantiable backend, opening the mail must display `SignedUnverified`, \
             got {root:?}"
        );
    }
    assert!(
        !matches!(
            root,
            AttachmentDisplay::SignedVerified { .. } | AttachmentDisplay::SignedFailed { .. }
        ),
        "the open path must never synchronously verify or fail the signature, got {root:?}"
    );

    let rendered = collect_rendered_text(&view.display);
    assert!(
        rendered.contains("Quarterly receipt enclosed."),
        "the S/MIME cleartext body must be rendered locally on open; rendered text: {rendered:?}"
    );
    assert!(
        view.attachment_tree.contains("S/MIME signature"),
        "the CMS part must surface in the attachment tree as `S/MIME signature`; tree: {:?}",
        view.attachment_tree
    );
}

/// Issue #16 / CVE-2008-3068: the async verify job dispatched by the
/// open path can never feed the CMS bytes to a PGP engine. The very
/// first step of the job is `extract_unverified_signature`, which
/// rejects `multipart/signed` whose `protocol` is not
/// `application/pgp-signature` with a `ValueError` mentioning the
/// `protocol` parameter. Without an engine there is no certificate
/// parsing and therefore no AIA/CRL fetch primitive.
#[test]
fn smime_open_verify_job_dies_at_protocol_gate() {
    let attachment = AttachmentBuilder::new(SMIME_SIGNED_MAIL).build();
    let Err(err) = melib::email::pgp::extract_unverified_signature(&attachment) else {
        panic!("a non-PGP `protocol` must be rejected before any crypto engine sees the CMS bytes");
    };
    assert_eq!(
        err.kind,
        melib::ErrorKind::ValueError,
        "the protocol gate must return a ValueError, got {err:?}"
    );
    assert!(
        err.to_string().contains("protocol"),
        "the protocol gate must name the offending `protocol` parameter, got {err}"
    );
}

/// Issue #16 / CVE-2008-3068: with `auto_verify_signatures` disabled the
/// open path must not dispatch any job at all (regardless of backend
/// availability) and must still render the cleartext body locally under
/// `SignedUnverified`.
#[test]
fn smime_open_auto_verify_off_shows_unverified() {
    let ctx = mock_context();
    let settings = ViewSettings {
        auto_verify_signatures: false.into(),
        ..Default::default()
    };

    let view = smime_envelope_view(&ctx, settings);
    let root = view
        .display
        .first()
        .expect("the multipart/signed root must have a display entry");
    assert!(
        matches!(root, AttachmentDisplay::SignedUnverified { .. }),
        "with auto-verify off the open path must display `SignedUnverified`, got {root:?}"
    );
    let rendered = collect_rendered_text(&view.display);
    assert!(
        rendered.contains("Quarterly receipt enclosed."),
        "the body must still render locally with auto-verify off; rendered text: {rendered:?}"
    );
    assert!(
        view.attachment_tree.contains("S/MIME signature"),
        "the CMS part must still surface in the attachment tree; tree: {:?}",
        view.attachment_tree
    );
}
