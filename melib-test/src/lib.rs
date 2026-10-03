/*
 * melib-test - lib.rs
 *
 * Copyright 2026 Kyle Lee
 *
 * SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later
 */

//! Integration test suite for the [`melib`] e-mail library.
//!
//! This crate hosts the integration tests that used to live in
//! `melib/tests/`, so they can be driven directly by `cargo test` as a
//! standalone workspace member. Backend suites are gated behind features
//! that forward to the matching `melib` features (`imap`, `jmap`,
//! `maildir`, `notmuch`, `smtp`, `sqlite3`), all enabled by default.
//!
//! The test targets live in `tests/`; this library target only exists so
//! the package has a buildable root, and hosts the shared test-logging
//! helper [`init_test_logging`].

use std::sync::Once;

/// Install a process-global `tracing` subscriber writing everything at
/// `TRACE` level to stderr (pretty format), so test runs surface melib's
/// diagnostic output. Idempotent: the first call wins, later calls are
/// no-ops.
pub fn init_test_logging() {
    static INIT: Once = Once::new();

    INIT.call_once(|| {
        use tracing::level_filters::LevelFilter;

        let _ = tracing::subscriber::set_global_default(
            tracing_subscriber::fmt()
                .pretty()
                .with_ansi(false)
                .with_max_level(LevelFilter::TRACE)
                .with_writer(std::io::stderr)
                .finish(),
        );
    });
}
