/*
 * cve - lib.rs
 *
 * Copyright 2026 Kyle Lee
 *
 * SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later
 */

//! Security CVE workspace of the hardened-meli fork.
//!
//! This crate is the scaffolding for CVE-driven regression tests that target
//! the `meli` and `melib` crates without adding test-only dependencies to
//! the shipped binaries. Every regression follows one pattern: a CVE (or a
//! CVE family), its verbatim attack payload as a fixture, and an inertness
//! or round-trip assertion against the hardened code path — see for example
//! the CVE-2025-66376 tag-splitting corpus that already locks down meli's
//! built-in HTML sanitizer from the `meli` crate.
//!
//! The source-verified research reports live next to this crate:
//!
//! - [SECURITY-CVE-RESEARCH.md](../SECURITY-CVE-RESEARCH.md) — survey of
//!   CVEs delivered through e-mail or directly attacking mail clients and
//!   their rendering path (tracking/privacy, malware/code execution,
//!   web/HTML embedding, e-mail-reachable browser engines, protocol and
//!   crypto trust boundaries);
//! - [SECURITY-CVE-RESEARCH.zh-CN.md](../SECURITY-CVE-RESEARCH.zh-CN.md) —
//!   its Simplified Chinese counterpart.
//!
//! New regressions go into test files under `src/` (unit) or `tests/`
//! (integration); run everything with `cargo test -p cve`.
//!
//! First in-crate regression: MFSA-2005-11 (issue #13), the Thunderbird
//! cookie-tracking advisory — [`mfsa_2005_11`] locks the sanitize →
//! html2text pipeline's immunity against cookie-beacon mail.
//! CVE-2005-2512 (issue #14) extends the corpus approach to the second
//! renders Mail.app leaked on — [`cve_2005_2512`] proves reply-quote,
//! pipe/print, forward and export paths never relax remote-content
//! stripping.

/// MFSA-2005-11 (Thunderbird 0.6–0.9 / Mozilla Suite 1.7–1.7.3)
/// cookie-tracking regression (issue #13): an HTML-mail beacon corpus
/// proving the render pipeline performs no remote reference loading —
/// every auto-load vector loses tag and URL in `sanitize`, and the
/// rendered mail is plain terminal text whose only remote references are
/// user-facing link footnotes meli never fetches.
#[cfg(test)]
#[path = "MFSA-2005-11.rs"]
mod mfsa_2005_11;

/// CVE-2005-2512 (Apple Mail.app ≤ 10.4.2) tracking-pixel regression
/// (issue #14): Mail.app fetched remote images when printing or
/// forwarding HTML mail, ignoring the "don't load remote images"
/// preference. This corpus proves meli's equivalent surfaces immune:
/// there is exactly one HTML display pipeline (sanitize → html2text,
/// no fetcher), every second render (reply-quote, pipe/print, forward,
/// export, nested .eml view) reuses or re-enters it with the same
/// strength, and no operation type relaxes beacon stripping.
#[cfg(test)]
#[path = "CVE-2005-2512.rs"]
mod cve_2005_2512;
