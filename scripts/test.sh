#!/bin/sh
# SPDX-License-Identifier: EUPL-1.2
#
# Reusable QA entrypoint: fmt check, clippy, melib + meli tests, workspace
# check. CARGO_HOME and any other toolchain configuration are inherited
# from the caller's environment; this script sets nothing
# machine-specific.

set -e

echo "=== [1/5] cargo fmt --check ==="
cargo fmt --check

echo "=== [2/5] cargo clippy -p melib --features sqlite3 --all-targets -- -D warnings ==="
cargo clippy -p melib --features sqlite3 --all-targets -- -D warnings

echo "=== [3/5] cargo test -p melib --features sqlite3 ==="
# sqlite3 feature is required: the imap sqlite cache tests/code are
# feature-gated and never compile/run without it.
# test_xdg_various_mimes is hermetic: it pins every XDG env var it
# consults to test-created temp dirs, so it runs on any host.
cargo test -p melib --features sqlite3

echo "=== [4/5] cargo test -p meli ==="
# meli's gpg rusty_fork tests write a log file under XDG_DATA_HOME; in
# sandboxed environments whose XDG data dir is read-only, export
# XDG_DATA_HOME to a writable directory before invoking this script.
cargo test -p meli

echo "=== [5/5] cargo check --workspace ==="
cargo check --workspace

echo "=== [optional] private-CSI watchdog PTY regression ==="
# Not one of the cargo gates: a PTY end-to-end check that needs tmux. The
# script itself skips (exit 0) when tmux is unavailable.
if command -v bash >/dev/null 2>&1 && [ -x ./scripts/test-private-csi-watchdog.sh ]; then
    ./scripts/test-private-csi-watchdog.sh || exit 1
fi

echo "=== all gates passed ==="
