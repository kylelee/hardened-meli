//
// meli
//
// Copyright 2024 Emmanouil Pitsidianakis <manos@pitsidianak.is>
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

//! Regression tests for C1 (CWE-78): mailcap entry command injection.
//!
//! Fixes under test:
//! 1. `meli/src/mailcap.rs` — every substituted value of `%t`, `%{param}`
//!    and `%s` is single-quote escaped for the shell (POSIX `'...'` with
//!    embedded `'` becoming `'\''`), like mutt does.
//! 2. `meli/src/mailcap.rs` — mailcap entry key matching direction is
//!    `content_type.fnmatches(key)` (the entry key is the glob pattern);
//!    the pre-fix inversion let a mail declaring `Content-Type: */*` match
//!    any mailcap entry.
//! 3. `melib` `AttachmentBuilder::set_content_type_from_bytes` —
//!    type/subtype tokens containing bytes outside the RFC2045 token set
//!    (SPACE, CTLs, tspecials, non-ASCII) are rejected and fall back to
//!    the default content type.
//!
//! The command-construction logic is replicated byte-for-byte from the
//! fixed `meli/src/mailcap.rs` because it lives in the `meli` binary crate
//! and cannot be imported from integration tests (`%s` exercises the
//! meli-internal temp file API and is omitted from the replica). Ground
//! truth for execution proofs is marker-file (non-)existence under a
//! unique per-run directory in the system temp dir; markers are removed
//! before and after each run.

use std::path::Path;

use melib::email::AttachmentBuilder;
use melib::utils::fnmatch::Fnmatch;

/// Unique, per-test marker directory under the system temp dir. The test
/// process id plus a per-test tag keeps concurrent test runs on the same
/// machine from colliding; `/tmp/opencode` may be a read-only mount.
fn marker_dir(tag: &str) -> String {
    std::env::temp_dir()
        .join(format!("meli-c1-poc-{}-{tag}", std::process::id()))
        .to_string_lossy()
        .into_owned()
}

/// Removes the marker directory tree when dropped, so tests clean up after
/// themselves even when an assertion fails mid-test.
struct MarkerGuard(String);

impl Drop for MarkerGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Replica of `split_command!` in `meli/src/mailcap.rs`.
macro_rules! split_command {
    ($cmd:expr) => {{
        $cmd.split_whitespace().collect::<Vec<&str>>()
    }};
}

/// Replica of `quote_shell_word` in the fixed `meli/src/mailcap.rs`.
fn quote_shell_word(s: &str) -> String {
    let mut ret = String::with_capacity(s.len() + 2);
    ret.push('\'');
    for c in s.chars() {
        if c == '\'' {
            ret.push_str("'\\''");
        } else {
            ret.push(c);
        }
    }
    ret.push('\'');
    ret
}

/// Replica of the fixed mailcap key-matching expression in
/// `meli/src/mailcap.rs`: the entry key is the glob pattern, the mail's
/// content type is the name.
fn mailcap_key_matches(key: &str, content_type: &str) -> bool {
    key.starts_with(content_type) || content_type.fnmatches(key)
}

/// Replica of the fixed command construction in `meli/src/mailcap.rs`,
/// returning the `sh -c` argument exactly as mailcap builds it.
fn build_cmd_string(mailcap_command: &str, a: &melib::email::Attachment) -> String {
    let parts = split_command!(mailcap_command);
    let (cmd, args) = (parts[0], &parts[1..]);
    let params = a.parameters();
    let args = args
        .iter()
        .map(|arg| match *arg {
            "%t" => Ok(quote_shell_word(&a.content_type().to_string())),
            param if param.starts_with("%{") && param.ends_with('}') => {
                let param = &param["%{".len()..param.len() - 1];
                let value = if let Some(v) = params.iter().find(|(k, _)| *k == param.as_bytes()) {
                    String::from_utf8_lossy(v.1).into()
                } else if param == "charset" {
                    String::from("utf-8")
                } else {
                    String::new()
                };
                Ok(quote_shell_word(&value))
            }
            a => Ok(a.to_string()),
        })
        .collect::<Result<Vec<String>, ()>>()
        .unwrap();
    format!("{} {}", cmd, args.join(" "))
}

#[test]
fn c1_fnmatch_direction() {
    // The Fnmatch trait treats its argument as the glob pattern and `self`
    // as the name. In the fixed direction, a mailcap glob key matches the
    // mail's content type:
    assert!(mailcap_key_matches("image/*", "image/png"));
    assert!(mailcap_key_matches("*/*", "application/pdf"));
    assert!(mailcap_key_matches("application/pdf", "application/pdf"));

    // Why the pre-fix inversion was dangerous: with the mail's content
    // type as the pattern, a mail declaring `*/*` matched any entry key.
    assert!("application/pdf".fnmatches("*/*"));

    // A mail declaring a hostile glob `*/*` must NOT match arbitrary keys,
    // and unrelated types must not match:
    assert!(!mailcap_key_matches("application/pdf", "*/*"));
    assert!(!mailcap_key_matches("image/png", "application/pdf"));
}

#[test]
fn c1_content_type_token_hardening() {
    // Backticks, `$` and `'` are legal RFC2045 token bytes, so such types
    // still parse through; the quoting layer neutralizes them (see the
    // execution tests below).
    let a = AttachmentBuilder::new(b"Content-Type: text/x`id`\r\n\r\nbody").build();
    assert_eq!(a.content_type().to_string(), "text/x`id`");

    // `*/*` also consists of legal token bytes and still parses; the
    // matching direction fix closes that vector, not token validation.
    let a = AttachmentBuilder::new(b"Content-Type: */*\r\n\r\nbody").build();
    assert_eq!(a.content_type().to_string(), "*/*");

    // SPACE in the subtype is outside the token set: rejected, falls back
    // to the default content type.
    let a = AttachmentBuilder::new(b"Content-Type: text/pl ain\r\n\r\nbody").build();
    assert_eq!(a.content_type().to_string(), "text/plain");

    // tspecials are outside the token set.
    let a = AttachmentBuilder::new(b"Content-Type: text/x=y\r\n\r\nbody").build();
    assert_eq!(a.content_type().to_string(), "text/plain");
    let a = AttachmentBuilder::new(b"Content-Type: text/(x)\r\n\r\nbody").build();
    assert_eq!(a.content_type().to_string(), "text/plain");

    // Non-ASCII bytes are outside the token set.
    let a = AttachmentBuilder::new("Content-Type: text/xé\r\n\r\nbody".as_bytes()).build();
    assert_eq!(a.content_type().to_string(), "text/plain");

    // RFC OWS before `;` is tolerated; the type survives (trimmed).
    let raw = b"Content-Type: application/pdf ; name=\"f.pdf\"\r\n\r\n%PDF-1.4";
    let a = AttachmentBuilder::new(raw).build();
    assert_eq!(a.content_type().to_string(), "application/pdf");

    // Param values are NOT restricted (quoted-string grammar); quoting at
    // substitution time is what neutralizes them.
    let raw = b"Content-Type: application/x-poc; exec=\"$(printf pwned)\"\r\n\r\nbody";
    let a = AttachmentBuilder::new(raw).build();
    let params = a.parameters();
    let v = params
        .iter()
        .find(|(k, _)| *k == b"exec")
        .map(|(_, v)| v.to_vec())
        .expect("exec param present");
    assert_eq!(v, b"$(printf pwned)".to_vec());
}

#[test]
fn c1_percent_param_substitution_is_inert() {
    let dir = marker_dir("param");
    let _guard = MarkerGuard(dir.clone());
    std::fs::create_dir_all(&dir).unwrap();
    let marker = format!("{dir}/c1_exec_param");
    let _ = std::fs::remove_file(&marker);

    // Attacker mail: command substitution in a Content-Type param value.
    let raw = format!(
        "Content-Type: application/pdf; injection=\"$(printf C1 > {marker})\"\r\n\r\n%PDF-1.4 fake"
    );
    let a = AttachmentBuilder::new(raw.as_bytes()).build();

    // Victim mailcap entry with a bare %{injection} token. The whole
    // substitution must be one single-quoted shell word.
    let cmd_string = build_cmd_string("pdfviewer --data %{injection}", &a);
    eprintln!("c1 param cmd_string: {cmd_string}");
    assert_eq!(
        cmd_string,
        format!("pdfviewer --data '$(printf C1 > {marker})'")
    );

    // Execute exactly like mailcap.rs does with `sh -c`. The fake viewer
    // binary does not exist — irrelevant: without quoting the command
    // substitution would run before argv lookup and create the marker.
    let _status = std::process::Command::new("sh")
        .arg("-c")
        .arg(&cmd_string)
        .status()
        .expect("spawn sh");
    assert!(
        !Path::new(&marker).exists(),
        "command substitution must NOT execute via %{{param}}"
    );
    let _ = std::fs::remove_file(&marker);
}

#[test]
fn c1_percent_t_substitution_is_inert() {
    let dir = marker_dir("t");
    let _guard = MarkerGuard(dir.clone());
    std::fs::create_dir_all(&dir).unwrap();
    let marker = format!("{dir}/c1_exec_t");
    let _ = std::fs::remove_file(&marker);

    // The original PoC payload injected an absolute path into the subtype;
    // `/` and `>` are tspecials, so token validation rejects the whole
    // type and the mail falls back to the default content type:
    let raw = format!("Content-Type: text/x`printf C1T > {marker}`\r\n\r\nbody");
    let a = AttachmentBuilder::new(raw.as_bytes()).build();
    assert_eq!(a.content_type().to_string(), "text/plain");

    // A payload made only of legal token bytes (backticks, `${IFS}`) still
    // parses through; quoting must neutralize it. `sh` is spawned with the
    // marker dir as cwd so the slash-free payload is observable there.
    let marker = format!("{dir}/c1_exec_t_rel");
    let _ = std::fs::remove_file(&marker);
    let raw = b"Content-Type: text/x`touch${IFS}c1_exec_t_rel`\r\n\r\nbody";
    let a = AttachmentBuilder::new(raw).build();
    assert_eq!(
        a.content_type().to_string(),
        "text/x`touch${IFS}c1_exec_t_rel`"
    );
    let cmd_string = build_cmd_string("textviewer -T %t", &a);
    eprintln!("c1 %t cmd_string: {cmd_string}");
    assert_eq!(
        cmd_string,
        "textviewer -T 'text/x`touch${IFS}c1_exec_t_rel`'"
    );

    let _status = std::process::Command::new("sh")
        .arg("-c")
        .arg(&cmd_string)
        .current_dir(&dir)
        .status()
        .expect("spawn sh");
    assert!(
        !Path::new(&marker).exists(),
        "backtick command must NOT execute via %t"
    );
    let _ = std::fs::remove_file(&marker);
    let _ = std::fs::remove_file(format!("{dir}/c1_exec_t"));
}

#[test]
fn c1_single_quote_escape_form() {
    let dir = marker_dir("escape");
    let _guard = MarkerGuard(dir.clone());
    std::fs::create_dir_all(&dir).unwrap();
    let marker = format!("{dir}/c1-marker");
    let _ = std::fs::remove_file(&marker);

    // Attacker mail: param value attempting to break out of single quotes.
    let value = format!("x'; touch {marker}; '");
    let raw = format!("Content-Type: application/pdf; exec=\"{value}\"\r\n\r\n%PDF-1.4 fake");
    let a = AttachmentBuilder::new(raw.as_bytes()).build();
    let cmd_string = build_cmd_string("pdfviewer --data %{exec}", &a);
    eprintln!("c1 escape-form cmd_string: {cmd_string}");

    // POSIX single-quote escaping: the value `x'; touch ...; '` must
    // appear as `'x'\''; touch <marker>; '\'''`.
    assert_eq!(
        cmd_string,
        format!("pdfviewer --data 'x'\\''; touch {marker}; '\\'''")
    );

    let _status = std::process::Command::new("sh")
        .arg("-c")
        .arg(&cmd_string)
        .status()
        .expect("spawn sh");
    assert!(
        !Path::new(&marker).exists(),
        "quote-breakout attempt must NOT create the marker file"
    );
    let _ = std::fs::remove_file(marker);
}

#[test]
fn c1_malformed_mailcap_entries() {
    let raw = b"Content-Type: application/pdf; exec=\"pwned\"\r\n\r\n%PDF-1.4";
    let a = AttachmentBuilder::new(raw).build();

    // Entry without any % token: literal words pass through untouched.
    assert_eq!(build_cmd_string("pdfviewer --data", &a), "pdfviewer --data");

    // Empty %{param} name and unknown params: one empty quoted word.
    assert_eq!(
        build_cmd_string("pdfviewer --data %{}", &a),
        "pdfviewer --data ''"
    );
    assert_eq!(
        build_cmd_string("pdfviewer --data %{nope}", &a),
        "pdfviewer --data ''"
    );

    // %{charset} keeps its documented default.
    assert_eq!(
        build_cmd_string("pdfviewer --charset %{charset}", &a),
        "pdfviewer --charset 'utf-8'"
    );

    // Values containing newlines stay one quoted word: `sh` treats bytes
    // inside '...' (including newlines) literally.
    assert_eq!(quote_shell_word("a\nb"), "'a\nb'");
    assert_eq!(quote_shell_word(""), "''");
}
