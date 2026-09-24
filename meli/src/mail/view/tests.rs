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

use super::{state::PendingReplyAction, MailView};
use crate::{
    command::{
        actions::{Action, ViewAction},
        MailingListAction,
    },
    components::Component,
    conf::composing::SendMail,
    melib::{Attachment, AttachmentBuilder, Envelope, Mail},
    terminal::Key,
    types::{Link, LinkKind, UIEvent},
    utilities::UIDialog,
    view::{EnvelopeView, ViewFilter, ViewFilterContent, ViewOptions, ViewSettings},
    AccountHash, Context, EnvelopeHash, MailboxHash,
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

/// Returns `true` if any reply is a notification produced by attempting to send
/// an unsubscribe e-mail (the mock account's `send_mail` command is `false`,
/// so a send attempt deterministically fails with a notification).
fn has_send_evidence(replies: &[UIEvent]) -> bool {
    replies.iter().any(|ev| {
        matches!(ev, UIEvent::Notification { title: Some(t), .. } if t.to_lowercase().contains("unsubscribe"))
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
