<!-- SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later -->

# melib-test

Integration test suite of the [`melib`](../melib) e-mail library, as a
standalone workspace crate.

The backend suites live in [`tests/`](tests/) (`imap`, `jmap`, `smtp`,
`maildir`, `notmuch`, `integration`, plus `jmap_redirect.rs`), each gated
behind the feature that forwards to the matching `melib` feature — all
enabled by default, so plain `cargo test` drives every test case. Tests run
on a dedicated `tokio` runtime through the `tokio-test` crate, replacing
the `rusty-fork` process-fork harness these suites used to rely on;
process-global environment rewrites are serialized per test target instead.
