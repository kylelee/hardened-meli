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
//! the package has a buildable root.
