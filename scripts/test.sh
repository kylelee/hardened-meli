#!/bin/sh
# SPDX-License-Identifier: EUPL-1.2
#
# Reusable QA entrypoint: fmt check, clippy, melib + meli tests, workspace
# check. CARGO_HOME and any other toolchain configuration are inherited
# from the caller's environment; this script sets nothing
# machine-specific.

set -e

echo "=== [1/6] cargo fmt --check ==="
cargo fmt --check

echo "=== [2/6] cargo clippy -p melib --features sqlite3 --all-targets -- -D warnings ==="
cargo clippy -p melib --features sqlite3 --all-targets -- -D warnings

echo "=== [3/6] cargo test -p melib --features sqlite3 ==="
# sqlite3 feature is required: the imap sqlite cache tests/code are
# feature-gated and never compile/run without it.
# --skip test_xdg_various_mimes: that test is environment-dependent (it
# relies on the host xdg-mime defaults and fails on hosts whose default
# handler is e.g. "papers %U"), so it is skipped here.
cargo test -p melib --features sqlite3 -- --skip test_xdg_various_mimes

echo "=== [4/6] cargo test -p meli ==="
# meli's gpg rusty_fork tests write a log file under XDG_DATA_HOME; in
# sandboxed environments whose XDG data dir is read-only, export
# XDG_DATA_HOME to a writable directory before invoking this script.
cargo test -p meli

echo "=== [5/6] cargo clippy + test -p meli_sanitize_html ==="
cargo clippy -p meli_sanitize_html --all-targets -- -D warnings
cargo test -p meli_sanitize_html

echo "=== [6/6] cargo check --workspace ==="
cargo check --workspace

echo "=== all gates passed ==="
