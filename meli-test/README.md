<!-- SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later -->

# meli-test

Integration test suite of the [`meli`](../meli) terminal e-mail client, as a
standalone workspace crate.

The test targets live in [`tests/`](tests/); they are driven directly by
`cargo test` (from the workspace root or with `cargo test -p meli-test`).
Tests run on a dedicated `tokio` runtime through the `tokio-test` crate,
replacing the `rusty-fork` process-fork harness this suite used to rely on.

`meli`'s unit tests and their fixtures (the golden-rendering corpus under
[`meli/tests/golden`](../meli/tests/golden)) stay in the `meli` package.
