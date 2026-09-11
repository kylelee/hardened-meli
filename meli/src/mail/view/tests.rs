//
// meli
//
// Copyright 2023 Manos Pitsidianakis
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

use super::MailView;
use crate::{
    command::{actions::Action, MailingListAction},
    components::Component,
    conf::composing::SendMail,
    melib::{Attachment, AttachmentBuilder, Envelope, Mail},
    terminal::Key,
    types::{Link, LinkKind, UIEvent},
    view::{EnvelopeView, ViewFilter, ViewOptions, ViewSettings},
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
    let ctx = Context::new_mock(&tempdir);
    let att: Attachment = AttachmentBuilder::new(bytes).build();
    let value = ViewFilter::new_attachment(&att, &settings, &ctx).unwrap();
    assert_eq!(&value.content_type.to_string(), "text/html");
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
    let ctx = Context::new_mock(&tempdir);
    let att: Attachment = AttachmentBuilder::new(bytes).build();
    let value = ViewFilter::new_attachment(&att, &settings, &ctx).unwrap();
    assert_eq!(&value.content_type.to_string(), "text/html");

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
