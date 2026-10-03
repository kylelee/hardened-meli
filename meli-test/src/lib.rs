/*
 * meli-test - lib.rs
 *
 * Copyright 2026 Kyle Lee
 *
 * SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later
 */

//! Integration test suite for the [`meli`] terminal e-mail client.
//!
//! This crate hosts the integration tests that used to live in
//! `meli/tests/`, so they can be driven directly by `cargo test` as a
//! standalone workspace member. Test-only dependencies
//! (`assert_cmd`, `predicates`, `tokio-test`, …) stay out of the shipped
//! `meli` manifest.
//!
//! The test targets live in `tests/`; this library target only exists so
//! the package has a buildable root.
