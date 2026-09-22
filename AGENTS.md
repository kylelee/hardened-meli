# AGENTS.md

Rust workspace for **meli**, a terminal e-mail client. CI runs on Gitea Actions (`.gitea/`), not GitHub Actions.

## Layout

- Workspace members are only `meli/` and `melib/` (root `Cargo.toml`). `tools/` and `fuzz/` are standalone crates (tools declares its own `[workspace]`); they are not built by `cargo build` at the root.
- `melib/` — mail library. Backend trait in `src/backends.rs`; protocol implementations are per-protocol modules: `src/imap/` (connection pool + sqlite3 sync cache in `src/imap/sync/`), `src/maildir/`, `src/mbox/`, `src/notmuch/`, `src/jmap/`, `src/nntp/`, `src/smtp/`. E-mail parsing in `src/email/`.
- `meli/` — terminal UI (binary `src/main.rs`). UI components in `src/mail/`, `src/terminal/`; thread-pool job executor in `src/jobs.rs`; account/backend glue in `src/accounts/`.

## Commands

- Dev build/run: `cargo build` / `cargo run`. `make` builds the **release** binary (fat LTO, `codegen-units=1` — slow to link; don't use it for iteration). Local install: `make PREFIX=~/.local install`.
- Format: `make fmt` — runs `cargo +nightly fmt` (`rustfmt.toml` uses nightly-only options `imports_granularity`/`group_imports`; falls back to stable) **and** `cargo-sort -w` on `melib` and `meli` manifests.
- Lint: `make lint` (clippy, all targets). Check: `make check`. Both set `RUSTFLAGS="-D warnings -W unreachable-pub -W rust-2021-compatibility"` — warnings are errors for these targets and in CI.
- `make check`/`make test`/`make lint` default to `--all-features`; override with `MELI_FEATURES="<features>"`.
- Tests: `make test` (runs rustdoc tests first, then everything). Single suite: `cargo test -p melib --test imap` (integration suites live in `melib/tests/<name>/main.rs`: `imap`, `jmap`, `smtp`, `maildir`, `notmuch`, `integration`). Single test with output: `cargo test -p melib <name> -- --nocapture`.
- CI parity without a runner: `for m in .gitea/Makefile.*; do make -f "$m" || break; done`. The CI workflows (`.gitea/workflows/`) just drive `.gitea/Makefile.{build,lint,manifest-lint}`, which additionally run `cargo-nextest`, `cargo-msrv`, `cargo-derivefmt`, and `cargo-sort`.

## Non-obvious rules

- MSRV is **1.85.0** (`rust-version` in Cargo.toml; BUILD.md's "1.80" is stale).
- Commits require a DCO sign-off: always `git commit -s` (CI enforces it via `.gitea/check_dco.sh`).
- New files need a license preamble (`"This file is part of meli"` / `"This file is part of melib"` or an `SPDX-License-Identifier` line) — enforced by `scripts/pre-commit`.
- `derive` attributes must be sorted alphabetically (`cargo-derivefmt` lints melib, meli, and tools in CI).
- `meli/src/conf/overrides.rs` is `@generated` by `meli/build.rs` from the settings structs in `meli/src/conf/{pager,listing,notifications,shortcuts,composing,tags,pgp}.rs`. Never hand-edit; to regenerate, touch the sentinel file `meli/src/conf/.rebuild.overrides.rs` and rebuild.
- The `cli-docs` feature (in meli's default features) shells out to `mandoc` or `man` in `meli/build.rs`; without either binary the build panics — disable the feature if they're unavailable.

## Debugging / runtime

- Trace logs: a debug build writes all logs to `./log/`. Build with `--features debug-tracing` (plus `imap-trace`/`smtp-trace`/`nntp-trace`/`jmap-trace` for protocol dumps) for extra tracing; set `MELI_DEBUG_STDERR=yes` to log to stderr instead.
