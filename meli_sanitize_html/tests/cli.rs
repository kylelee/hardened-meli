// SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later
//! Integration tests for the `meli_sanitize_html` command-line binary.

use std::io::Write as _;
use std::process::{Command, Stdio};

/// Run the binary with `args`, feed `input` bytes to its stdin, and return
/// `(exit code, stdout, stderr)`.
///
/// Stdin is written to completion and the handle dropped *before* waiting on
/// the child. The binary drains stdin to EOF before writing any stdout, so
/// this ordering cannot deadlock even for inputs far larger than the pipe
/// buffer.
fn run(args: &[&str], input: &[u8]) -> (i32, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_meli_sanitize_html"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn meli_sanitize_html");
    let mut stdin = child.stdin.take().expect("failed to open child stdin");
    stdin.write_all(input).expect("failed to write child stdin");
    drop(stdin);
    let output = child
        .wait_with_output()
        .expect("failed to wait for meli_sanitize_html to exit");
    let code = output
        .status
        .code()
        .expect("child was terminated by a signal");
    (
        code,
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn filters_basic_html_from_stdin() {
    let (code, stdout, stderr) = run(&[], b"<b>x</b>");
    assert_eq!(code, 0, "exit code; stderr: {stderr:?}");
    assert_eq!(stdout, "<b>x</b>", "stdout; stderr: {stderr:?}");
}

#[test]
fn empty_input_yields_empty_output() {
    let (code, stdout, stderr) = run(&[], b"");
    assert_eq!(code, 0, "exit code; stderr: {stderr:?}");
    assert!(stdout.is_empty(), "stdout: {stdout:?}");
}

#[test]
fn help_flag_prints_usage_to_stdout() {
    let (code, stdout, stderr) = run(&["--help"], b"");
    assert_eq!(code, 0, "exit code; stderr: {stderr:?}");
    assert!(stdout.contains("Usage"), "stdout: {stdout:?}");
}

#[test]
fn version_flag_prints_crate_version() {
    let (code, stdout, stderr) = run(&["--version"], b"");
    assert_eq!(code, 0, "exit code; stderr: {stderr:?}");
    assert_eq!(
        stdout,
        concat!("meli_sanitize_html ", env!("CARGO_PKG_VERSION"), "\n"),
        "stdout: {stdout:?}"
    );
}

#[test]
fn unknown_argument_exits_two_with_error_on_stderr() {
    let (code, stdout, stderr) = run(&["--nope"], b"");
    assert_eq!(code, 2, "exit code; stdout: {stdout:?} stderr: {stderr:?}");
    assert!(stdout.is_empty(), "stdout: {stdout:?}");
    assert!(!stderr.is_empty(), "stderr: {stderr:?}");
}

#[test]
fn non_utf8_input_is_tolerated() {
    let (code, _stdout, stderr) = run(&[], b"<p>\xff\xfe</p>");
    assert_eq!(code, 0, "exit code; stderr: {stderr:?}");
}

#[test]
fn megabyte_input_is_processed_without_deadlock() {
    let paragraph = "<p>lorem ipsum dolor sit amet</p>\n";
    // Well above the typical 64 KiB pipe buffer: forces multiple
    // read/write round trips between test and child.
    const TARGET: usize = 1024 * 1024;
    let repeats = TARGET / paragraph.len() + 1;
    let input = paragraph.repeat(repeats);
    assert!(input.len() >= TARGET);
    let (code, _stdout, stderr) = run(&[], input.as_bytes());
    assert_eq!(code, 0, "exit code; stderr: {stderr:?}");
}
