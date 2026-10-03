# AGENTS.md

Rust workspace for **meli**, a terminal e-mail client. CI runs on Gitea Actions (`.gitea/`), not GitHub Actions.

## Layout

- Workspace members are `cve/`, `meli/`, `meli-test/`, `melib/`, `melib-test/` (root `Cargo.toml`).
- `melib/` — mail library. Backend trait in `src/backends.rs`; protocol implementations are per-protocol modules: `src/imap/` (connection pool + sqlite3 sync cache in `src/imap/sync/`), `src/maildir/`, `src/mbox/`, `src/notmuch/`, `src/jmap/`, `src/nntp/`, `src/smtp/`. E-mail parsing in `src/email/`.
- `meli/` — terminal UI (binary `src/main.rs`). UI components in `src/mail/`, `src/terminal/`; `tokio`-runtime job executor in `src/jobs.rs` — created in `main.rs` before signal watching, it is the single async runtime driving jobs, timers, heartbeats and signal delivery (`src/signal_handlers.rs`, event-driven via `tokio::signal`, no polling thread); account/backend glue in `src/accounts/`. Its unit-test fixtures (golden corpus) stay in `meli/tests/golden`.
- `meli-test/`, `melib-test/` — integration test suites of `meli`/`melib` as standalone crates driven by `cargo test`.
- Every test in the workspace (unit and integration) runs via `tokio-test`; `rusty-fork` is gone from the workspace — tests that rewrite process-global environment variables serialize on a shared per-crate lock instead of forking.
- `cve/` — security CVE workspace: the CVE research reports (`SECURITY-CVE-RESEARCH*.md`) and the crate scaffolding for CVE-driven regression tests targeting `meli`/`melib`.

## Commands

- Dev build/run: `cargo build` / `cargo run`. `make` builds the **release** binary (fat LTO, `codegen-units=1` — slow to link; don't use it for iteration). Local install: `make PREFIX=~/.local install`.
- Format: `make fmt` — runs `cargo +nightly fmt` (`rustfmt.toml` uses nightly-only options `imports_granularity`/`group_imports`; falls back to stable) **and** `cargo-sort -w` on the `melib`, `meli`, and `cve` manifests.
- Lint: `make lint` (clippy, all targets). Check: `make check`. Both set `RUSTFLAGS="-D warnings -W unreachable-pub -W rust-2021-compatibility"` — warnings are errors for these targets and in CI.
- `make check`/`make test`/`make lint` default to `--all-features`; override with `MELI_FEATURES="<features>"`.
- Tests: `make test` (runs rustdoc tests first, then everything). Single suite: `cargo test -p melib-test --test imap` (integration suites live in `melib-test/tests/<name>/main.rs`: `imap`, `jmap`, `smtp`, `maildir`, `notmuch`, `integration`; `meli`'s integration tests live in `meli-test/tests/`). Single test with output: `cargo test -p melib-test <name> -- --nocapture`.
- CI parity without a runner: `for m in .gitea/Makefile.*; do make -f "$m" || break; done`. The CI workflows (`.gitea/workflows/`) just drive `.gitea/Makefile.{build,lint,manifest-lint}`, which additionally run `cargo-nextest`, `cargo-msrv`, `cargo-derivefmt`, and `cargo-sort`.

## Non-obvious rules

- MSRV is **1.85.0** (`rust-version` in Cargo.toml; BUILD.md's "1.80" is stale).
- Commits require a DCO sign-off: always `git commit -s` (CI enforces it via `.gitea/check_dco.sh`).
- New files need a license preamble (`"This file is part of meli"` / `"This file is part of melib"` or an `SPDX-License-Identifier` line) — enforced by `scripts/pre-commit`.
- `derive` attributes must be sorted alphabetically (`cargo-derivefmt` lints melib and meli in CI).
- `meli/src/conf/overrides.rs` is `@generated` by `meli/build.rs` from the settings structs in `meli/src/conf/{pager,listing,notifications,shortcuts,composing,tags,pgp}.rs`. Never hand-edit; to regenerate, touch the sentinel file `meli/src/conf/.rebuild.overrides.rs` and rebuild.
- The `cli-docs` feature (in meli's default features) shells out to `mandoc` or `man` in `meli/build.rs`; without either binary the build panics — disable the feature if they're unavailable.

## Debugging / runtime

- Logs: `meli`/`melib` log via the `tracing` crate directly (the `log` dependency and the old `melib` logging module are gone; there is no logging cargo feature). One `meli/src/logging.rs::init_log()` call at startup configures everything: `tracing-subscriber` pretty formatting into `tracing-appender` hourly-rotated files `./log/meli.<YYYY-MM-DD-HH>` (kept at `0600`), a background retention sweep keeping 7 days of logs, `DEBUG` level in debug builds and `ERROR` in release builds (`tracing`'s `release_max_level_error` also compiles everything below `ERROR` out of release). `MELI_DEBUG_STDERR=yes` duplicates every line on stderr. The legacy `[logging] log_file`/`maximum_level` config keys are inert (warning at startup). Protocol-level connection dumps are runtime settings: `trace = true` in the account's extra settings (IMAP, JMAP, NNTP) or in `composing.send_mail` SMTP settings.
