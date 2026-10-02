**English** | [简体中文](./SYNC.zh-CN.md)

# SYNC.md — Upstream meli Sync Log

This repository is a downstream fork of [meli](https://github.com/meli/meli) that is kept in continuous sync with upstream.
This file records the time and content of every sync from upstream, so each local change can be traced back to its upstream commit.

## Upstream repositories

| Repository | URL |
| --- | --- |
| Upstream (GitHub) | <https://github.com/meli/meli> |

Recommended local setup:

```sh
git remote add upstream https://github.com/meli/meli.git
git fetch upstream
```

## Entry format

After each sync, prepend a new section at the **top** of "Sync log" below (newest first):

```markdown
## YYYY-MM-DD HH:mm (UTC+8)

- Method: <per-commit semantic port / merge / cherry-pick>
- Upstream range: <start hash>..<end hash> (or list each commit)
- Merge commit: <local merge/commit hash>
- Conflicts: <none / conflicted files and how they were resolved>
- Accounting kinds: a row may be SKIP (fork has an equivalent or porting is not applicable, with reason), PARTIAL (named hunks ported, remainder deferred — must add a debt entry), or N:1 (N upstream commits in one local commit, with reason, e.g. an upstream broken-intermediate commit)

| Local commit | Upstream commit | Type | Notes |
| --- | --- | --- | --- |
| <hash> | <hash> | fix/perf/feat/refactor | <one-line summary> |
```

## Sync log


## 2026-10-02 22:30 (UTC+8)

- Method: semantic port on two parallel worktree branches (`sync-melib-trio`, `sync-flag-toggle`), executed by two dsh sub-agents with TDD (red→green) per change
- Upstream range: `253ba7dd..aea4508b` (16 commits; 4 ported, 12 SKIPped — see table)
- Merge commits: `361f97b` (sync-melib-trio, fast-forward), `aea11df` via merge `86d1ede` (sync-flag-toggle)
- Conflicts: none (disjoint file sets)
- Port-gap disclosure: upstream's `f6ddf9a4` (2026-09-28, `deserialize_extra_field` + `ExtraSetting` typed extra deserialization) predates this range and was NOT carried by the 2026-10-01 port — that gap is the root cause of the numeric/boolean extra-config silent-swallow bug fixed locally in `ebdfcff8` (`AccountSettings::extra_conf_string`). Known divergence: fork keeps `extra_conf_string` string-coercion (real-network verified against QQ Mail 993 implicit TLS); upstream uses typed deserialization. Follow-up task: evaluate unifying on one mechanism.
- Verification: `make check`, `make lint`, `make test` all green; CI parity: `Makefile.build` (incl. rustdoc) + `Makefile.manifest-lint` (cargo-sort + debian/changelog) + `clippy` green — `rustfmt` and `cargo-msrv` skipped locally (no nightly toolchain / `cargo-msrv` not installed, `~/.cargo` read-only; the CI runner has both); protection gate: `test_account_settings_extra_conf_string` + `test_conf_numeric_and_boolean_extra_values_reach_imap_server_conf` pass unchanged, all four `get_conf_val!` macros untouched; real-network smoke: qq + work 993 implicit TLS login OK (`M2 OK Success login ok`), 0 STARTTLS lines

| Local commit | Upstream commit | Kind | Type | Notes |
| --- | --- | --- | --- | --- |
| `361f97b` | `5151e75c` | port | fix(melib) | `set_flags` ignores `Flag::PASSED` (Set/UnSet no-op arms) — was falling into the "more than one flag bit" error branch; mock-server test `test_imap_set_flags_ignores_passed` (verified red pre-fix) |
| `361f97b` | `32733460` | port | perf(melib) | `BackendEvent` hand-written `Debug`: `RefreshBatch` ≥30 entries prints length + first 30 (derived Debug printed every event — multi-MB log lines); unit tests verify bounded output and completeness below the threshold |
| `361f97b` | `1218cb74` | port | feat(melib) | `TryFrom<Vec<EnvelopeHash>> for EnvelopeHashBatch` (empty → `Err`); unit test covers empty/single/many |
| `aea11df` | `8404d74a` | port+deviation | feat(meli) | `flag toggle <FLAG>` command (upstream #765): `FlagAction::Toggle`, third parser alt arm, dual-batch execution (envs without flag → Set batch, with flag → UnSet batch) in one `toggle-flag` job; deviations: fork parser arm structure per fork convention, `EnvelopeHashBatch` via existing slice `TryFrom` (no dependency on the Vec impl), `account.is_async()` spawn lane; palette completion entry added |
| — | `aea4508b`, `7d1d5b4d`, `4fb09060`, `b690d871` | **SKIP** | feat(meli) | compose/edit_attachments Add/Remove buttons + ButtonWidget theme attrs — UI layer; fork has its own UI architecture (no UI sync per policy) |
| — | `90e68695`, `b7b7565e` | **SKIP** | chore | clippy lint fixes on upstream-diverged code shapes; fork CI is green |
| — | `537c687e`, `68f368aa`, `cf64e366`, `553f3baf` | **SKIP** | feat(meli) | command-completion improvements — superseded by fork's nucleo command palette |
| — | `a41cb7b0`, `08561c25` | **SKIP** | feat(melib) | `ShellExpandTrait::expand_tilde` + tilde expansion in `complete()` — only serves upstream's completion framework; the fork has no production caller of `complete()` (its only call sites are the trait's own unit tests, which already `.expand()` first), so porting would be dead code |

## 2026-10-01 21:30 (UTC+8)

- Method: semantic port on seven parallel worktree branches (batch 1: `t1-mailcap`, `t3-notmuch`, `t4-melib-fixes`, `t5-trace-flag`, `t6-listing-compose`, `t7-contrib-gpg`; batch 2 after batch-1 merge: `t2-secret`)
- Upstream range: `bb6d5916..253ba7dd` (35 commits; 6 SKIPped/partial — see table)
- Merge commits: `96c35621` (t7), `c7e0745c` (t1), `9cfbcd80` (t6), `eb44b9ac` (t4), `bf84926b` (t3), `1b568f27` (t5), `0fcd631d` (t2)
- Conflicts: `meli/src/mail/listing/conversations.rs` `filter()` only — combined t4's `get_threads` Option-ization with t6's counter-tuple reduction (restriction removal)
- Acceptance fixes during merge: `e5c9b3c0` + `ac9dc1a7` (clippy 1.98 new-lint compliance), `9a33ac65` (list-unsubscribe test realigned with the async `send_draft_async` job path — stale assertion pre-existing on `5a04bc0d`, reproduced on detached base), `92f06575` (extra fields moved from `serde_json::Map` + `preserve_order` feature to `IndexMap` — the feature had silently changed JMAP wire key order and broke `test_jmap_query`; upstream uses `IndexMap` without the feature), plus the `decl_version_map` doc example updated for `v0_10_0`
- Verification: `make check`, `make lint`, `make test` all green (all features, 0 warnings)

| Local commit | Upstream commit | Kind | Type | Notes |
| --- | --- | --- | --- | --- |
| `705de33c` | `0d4b0bf9` | port+deviation | refactor(meli) | `ProcessRequest` struct + `temporary_files: Vec<Arc<File>>`; fork keeps the files alive until after `result_cb` (upstream's wildcard binding drops them before the process runs) |
| `f8c11e21` | `c8cad6fb` | port | fix(meli) | exit-status check on the `spawn: None` branch; the `spawn: Some` branch was already in the fork |
| `31f87da4` | `253ba7dd` | port+deviation | feat(meli) | full RFC 1524 mailcap engine (all fields, `MailcapParser`, `run_candidates`, upstream test suite); fork hardening replayed: POSIX single-quote shell quoting for every substituted value (`%t`, `%{param}`, paths), unknown `%` sequences → `Error` (upstream panics), nametemplate bounds bug fixed, malformed entries skipped individually (upstream skips the whole label) |
| `e61a8b1f` | `2309c167` | port | fix(meli) | `sanitize_filename` narrows punctuation stripping to `!"'/\`; boundary tests added |
| `3646287b` | `2ca62c90` | port | refactor(melib) | `Drop` moved to `DbPointer` (owns `Arc<NotmuchLibrary>`), `DbConnection` is `Clone`; debug_assert `expect` panic paths removed |
| `035223be` | `05a08b6c` | port+deviation | fix(melib) | refresh diffs current vs snapshot tags/existence → precise `RefreshEvent`s; `LazyCountSet` counters; fork deviation: `mailboxes.get()+continue` instead of map-index panic, `let-else` Err returns in set_flags/NotmuchOp |
| `f3f8fa42` | `0d7e1532` | port+deviation | refactor(melib) | `MailboxCounters` single mutex for imap+notmuch; fork's cache-first paging, ghost cleanup and watch compensation preserved; jmap/maildir/nntp double-lock sites NOT migrated (deferred — see debt) |
| `b612ca4c` | `57e60bec` | port | fix(melib) | notmuch search terms combined with `AND` (upstream #766) |
| `4ba5328e` | `09c6d05c` | port | perf(melib) | fetch chunk 250→1000 |
| `934b8838` | `3a19fe2e` | port | refactor(melib) | `ignore_not_found` promoted to `melib::error` |
| `dacdd4b3` | `ee38e475` | port+deviation | perf(meli) | `AccountCache::update`/rename in place; insert/update share a private `store()` helper; fork's `account_id` hardening kept, `?` chains instead of upstream `unwrap` |
| `4695f051` | `d45ea5fe` | port+deviation | fix(melib) | `get_mailbox`/`get_threads` return `Option` (fork's `get_env` already did); upstream's compose/view signature refactor not needed (fork already adapted) |
| `23d930f2` | `34e40e0e` | port+deviation | fix(melib) | maildir user actions emit events directly; fork's "no filesystem IO under the cache lock" discipline and concurrent-modification reconcile preserved |
| `8715f27a` | `6429fccc`+`d75d3be8` | **2:1** port+deviation | feat(conf) | per-account `trace` replaces `{imap,jmap,nntp,smtp}-trace` features; fork keeps `debug-tracing` (./log/) and `to_str!`; connection-id logging follows fork's scheme; `test_trace_redact_*` redaction suite stays green |
| `4f4153b1` | `e4565617` | port | fix(meli) | Junk no longer a Trash fallback |
| `ca826d98` | `2b86929b` | port | fix(meli) | search over a filter hits the whole mailbox (fork had the same bug in all four listing kinds); `filter_on_top_of_filter_searches_whole_mailbox` regression test, red-verified |
| `c39d9313` | `2d7fa2fa` | port+deviation | feat(command) | `public-inbox import`/`import-thread` from lore.kernel.org; fork parser is byte-oriented, confirmation flows via `UIEvent::Callback` (fork dialogs lack a context-taking done_fn), palette + docs + parser tests |
| `492f72eb` | `7fe6cc1e`+`0ef78a0d`+`78eb0d5e`+`bb17a5bc` | **4:1** port | feat(compose) | multi-address autocomplete (parse valid prefix, complete last segment, dedupe), `Contacts::search` → `Card`, `From<Card> for Address`, fields submodule + `test_compose_address_complete` port |
| `ab257c5a` | `a7c98b05`+`547f600e` | **2:1** port+deviation | fix(contrib) | Python 3.9 compat + stderr in error JSON; deviation: upstream's rewritten `hash_algo = str(other)` NameError bug fixed (`str(hash_algo)`) |
| `d7fb255a` | `f6ddf9a4` | port | feat(melib) | `ExtraSetting` trait (+`Secret::prepopulate`) |
| `a0ced034` | `97a08539` | port+deviation | feat(conf) | extra fields as values; acceptance fix `92f06575` settled on `IndexMap<String, serde_json::Value>` (upstream's shape) after `serde_json::Map` + `preserve_order` broke JMAP wire key order |
| `ecba109b` | `254cee97` | port+deviation | feat(melib) | `Secret` for all server personal fields, resolved at the last moment before auth bytes; fork's password-command error hygiene moved into `Secret::value`; oauth2 validation equivalent; `server_password_command` rejected at validate with migration hint |
| `b9434a12` | `483f0629` | port+deviation | feat(meli) | `ServerPasswordCommand` migration hung on new `v0_10_0` (fork's `v0_9_0` is released and content-diverges from upstream's); `is_applicable` via `get_included_configs` + raw contains; crate version bumped to 0.10.0 |
| — | `266b918a` | **SKIP** | refactor(ui) | Selector done-callback context — fork dialogs rewritten (UI) |
| — | `59ffaaeb` | **SKIP** | refactor | `RowsState` generic removal — pure internal, fork listing rewritten |
| — | `facc045c` | **SKIP** | refactor | `to_str!` removal — fork's tolerant IMAP parser still uses it |
| — | `9ff2e38e` | **SKIP** | fix | `change_log_level` max-level — fork already has it (with explanatory comment) |
| — | `41b547c6` | **SKIP** | test | mock-config TRACE — fork test infra differs |
| — | `63894a9c` | **SKIP** | test | `new_mock` env reset — fork has its own hermetic XDG helpers per test |
| — | `05dde1d2` | **partial SKIP** | chore | obsolete-macro half subsumed by the `d75d3be8` port; fork keeps the `debug-tracing` cargo feature (./log/ file logging) — docs updated to describe both layers |

- Debt register (carried over + new):
  - FilterOutputMetadata not ported (decryption recipients / per-filter signature status display) — align on fork's SignedVerified pipeline within 1-2 syncs *(from 2026-09-14)*
  - filters.rs ViewFilter path does not trigger cleartext verification *(from 2026-09-14)*
  - imap test-infra fork: upstream imap test commits must be reimplemented *(from 2026-09-14)*
  - sqlite3 search backend ignores raw_search (upstream quirk, verbatim) *(from 2026-09-14)*
  - Upstream command-completion framework not ported; watch future upstream `meli/src/command/**` commits for non-UI fixes *(from 2026-09-28)*
  - `mailbox_changed` SSE branch implemented but not exercised by the mock server *(from 2026-09-28)*
  - jmap/maildir/nntp double-lock counters not migrated to `MailboxCounters` (upstream `0d7e1532` touched them too; fork deferred to keep this sync's blast radius at backends+imap+notmuch) *(new 2026-10-01)*
  - `test_jmap_watch` Destroy assert initially raced the manual `refresh()` against the SSE-triggered resync for the same state diff (drained the queue before delivery landed); fixed by awaiting the watch future like the Create/NewFlags asserts — the race is gone, keep this entry as provenance *(new 2026-10-01, resolved same day)*

## 2026-09-28 03:47 (UTC+8)

- Method: semantic port on four parallel worktree branches (`t1-melib-conf-fixes` small melib/conf fixes, `t2-pgp-backends` PGP backends, `t3-jmap-eventsource` JMAP push, `t4-compose-notify-fixes` compose/notify fixes)
- Upstream range: `3d7eb2c5..bb6d5916` (22 commits; 4 SKIPped — see table)
- Merge commits: `b6360713` (T1), `5f0c3d3a` (T3), `240a1cbf` (T2), `57490df6` (T4)
- Conflicts: `meli/src/conf/tests.rs` only — both sides appended new test blocks at EOF; kept both (T1's `test_conf_tag_rename` + T2's `pgp_backend_choice_tests`)
- Verification: `make check`, `make lint`, `make test` all green (all features, 0 warnings)

| Local commit | Upstream commit | Kind | Type | Notes |
| --- | --- | --- | --- | --- |
| `3e49d1a1` | `ed162e11` | port+deviation | fix(melib) | `resync_condstore` `== 0`→`< 2` at the condstore FLAGS site only; `resync_basic`'s inclusive `..=` site verified unaffected (empty-range hazard does not exist there), noted in commit body |
| `8fab4a01` | `3f8427b0` | port | fix(melib) | PUA → Ambiguous width `Some(1)` + `wcswidth("\u{F09B}")` regression test |
| `8eda294c` | `d8cc16b9` | port | fix(meli) | `TagName` hashes `TagHash` (not name); fields `pub`; `test_conf_tag_rename` added |
| `53b101f3` | `fe48ab20` | port | refactor(meli) | feature-dependent import removed at fork's equivalent site |
| `c975e9a4` | `b33e50fc` | port | style(melib) | clippy `allow` attribute placement corrected |
| — | `9cbb4f41` | **SKIP** | style | fork's `conf.rs` carries no such import (fork top level is `extern crate serde;` only) |
| `d0250dc6` | `40a45b04` | port+deviation | refactor(melib) | `url_template` module extracted from `methods.rs` (RFC 8620 URI templates, with unit tests); fork keeps `Arc<FutureMutex<usize>>` + async `add_call` (upstream's `AtomicUsize` rewrite not adopted — internal only) |
| `98fd3296` | `af619461` | port+deviation | feat(melib) | EventSource (SSE) push against `eventSourceUrl`, replacing polling; deviations: upstream `panic!`/`assert_eq!` on protocol violations → `ErrorKind::ProtocolError` (fork no-panic hardening); SSE request `RedirectPolicy::None` (keeps fork's no-cross-origin-redirect-credential hardening, upstream `Limit(10)`); dead `last_method_response` field + hardcoded 10s `new()` timeout removed (would kill `timeout: None` SSE); fork test instrumentation (`error_responses`, since_state==current → empty response) preserved; new `run_jmap_watch`/`test_jmap_watch` SSE mock test |
| `afbd67f8`+`debe90c0`+`5805c091` | `89f834b6`+`97f02477`+`7110e8d0` | **3:1** port+deviation | feat(pgp) | `PGPBackend` trait + `Key` abstraction in `melib::email::pgp`; gpgme behind the trait; `cli` script backend (`[pgp] backend = "cli"`) + six GnuPG reference scripts in `contrib/pgp-cli-backends/gpg/`; `compose/gpg.rs`→`pgp.rs`; keylist `IndexSet` dedupe; deserializer error message. Deviations: cleartext pipeline (`UnverifiedSignature`/`extract_unverified_signature`/`verify_cleartext`) and SignedPending→SignedVerified routing preserved verbatim — no ViewFilter/FilterOutputMetadata; owned `PGPBackendInstance` for `JobExecutor::spawn`; `From<gpgme::Key> for pgp::Key` instead of upstream's `GpgmeKey` rename; upstream's removal of `#[cfg(feature="gpgme")]` guards in `command/{actions,parser}.rs` NOT ported (fork no-gpgme UI contract, keeps `00d3b65d` semantics); `overrides.rs` regenerated via sentinel |
| `971f1f9d` | `59c5ad4b` | port | fix(compose) | editor resolution: `composing.editor_command` > `$VISUAL` > `$EDITOR`, message updated |
| `86348503` | `bb6d5916` | port | fix(compose) | editor args via argv (`sh -c '<editor> "$@"'` + `.arg(&editor).arg(path)`), no string interpolation; fork's `EDITOR_TEMP_FILENAME_MAX_BYTES` hardening untouched |
| `d9c32af8` | `c498d7e6` | port+deviation | fix(ui) | clear full `cached_area` (borders included) before drawing; adapted to fork's `draw_rounded_frame` (fork has no `create_box`) |
| `dc0537a5` | `6d3dd4bd` | port | fix(ui) | compact tag text printed at `area_col_4.skip_cols(1)` |
| — | `a041bc90` | **SKIP** | feat(ui) | upstream command-completion overhaul (new `completions.rs`, parser/UI rework, ~2800 lines); fork's command palette + nucleo fuzzy matcher supersedes it (user decision 2026-09-28). Debt: future upstream commits to `meli/src/command/**` must be re-evaluated against the fork's parser, not diffed |
| — | `03e1f5de` | **SKIP** | refactor | `ListingTrait::select` lift — pure internal refactor, zero callers outside `listing*.rs` in the whole upstream range; fork keeps per-listing inherent impls |
| — | `0eaae124` | **SKIP** | ci | upstream removed cargo-derivefmt (broken with current Rust syntax); fork CI is green and keeps the step — re-evaluate only if the fork's derivefmt starts failing |
| — | `45a5d376` | **SKIP** | chore(deps) | `quote` 1.0.47 already present in fork `Cargo.lock` |

- Debt register:
  - FilterOutputMetadata not ported (decryption recipients / per-filter signature status display) — align on fork's SignedVerified pipeline within 1-2 syncs *(from 2026-09-14)*
  - filters.rs ViewFilter path does not trigger cleartext verification (mail opened via ViewFilter bypasses the envelope.rs routing) *(from 2026-09-14)*
  - imap test-infra fork: upstream imap test commits must be reimplemented (see SKIP b08a39b3) *(from 2026-09-14)*
  - sqlite3 search backend ignores raw_search (upstream quirk, verbatim) *(from 2026-09-14)*
  - Upstream command-completion framework not ported (fork command palette supersedes); watch future upstream `meli/src/command/**` commits for non-UI fixes buried in completion rewrites
  - `mailbox_changed` SSE branch implemented but not exercised by the mock server (upstream's new suite doesn't cover it either)

## 2026-09-15 02:11 (UTC+8)

- Method: verification-only check (no code changes)
- Upstream range: `3d7eb2c5` (upstream HEAD unchanged — no new commits since the 2026-09-14 23:05 sync)
- Merge commit: none (documentation-only)
- Conflicts: none

| Local commit | Upstream commit | Kind | Type | Notes |
| --- | --- | --- | --- | --- |
| — | — | check | docs | `git fetch` on the upstream mirror confirms `origin/master` still at `3d7eb2c5` (2026-09-14 12:44:51 +0300); the fork is fully synced. CHANGELOG.md `[Unreleased]` records this status. Debt register unchanged (4 open items from 2026-09-14 23:05) |

## 2026-09-14 23:05 (UTC+8)

- Method: semantic port on two parallel worktree branches (`sync-24h-a` raw-search chain, `sync-24h-b` PGP chain)
- Upstream range: `4f2414a3..3d7eb2c5` (19 commits; WIP `8b51d601` excluded)
- Merge commits: `956abda0` (chain A), `7d183160` (chain B)
- Conflicts: none (the chains share no files)

| Local commit | Upstream commit | Kind | Type | Notes |
| --- | --- | --- | --- | --- |
| `a3432d8f` | `d1941d6f` | port | chore(deps) | futures manifest → 0.3.34 (Cargo.lock already resolved it) |
| `68ef3e66` | `2e00c62e` | port+deviation | ci | nextest ci profile; **terminate-after=4 (upstream 2)**: fork watch tests self-limit at 30s (`WATCH_TEST_DEADLINE`), 40s keeps panic-before-kill diagnostics; `Makefile.build` wired with `--profile ci --config-file` |
| `25a2a9f8` | `3d7eb2c5` | port | docs | BUILD.md MSRV sentence removed (Cargo.toml is source of truth) |
| `5f54195c` + `de8e6c1b` | `d321e3f2` | port+deviation | test(jmap) | since_state==current → empty response instead of cannotCalculateChanges; **defensive alignment** (branch unreachable in current scenario, not a behavior proof); fork-added `error_responses` server instrumentation (dedupe/second-site fixup `de8e6c1b` from adversarial review) |
| `0fac10a5` | `2c5670cb` | port | test(notmuch) | macos lib detection (`library_file_path`) |
| — | `b08a39b3` | **SKIP** | test-infra | imap test-server refactor (2243 lines): fork's own 8700-line suite covers more (incl. fetch-cache regressions); upstream's new suite has no raw_search coverage. Debt: future upstream imap-test commits need reimplementation, not porting |
| — | `939c17d5` | **SKIP** | test | test_imap_fetch depends on b08a39b3 infra; fork has its own fetch-cache tests (melib/tests/imap/main.rs:199-249) |
| `8e280430` | `9e7014cf` | port | refactor(melib) | EMPTY_MAIL_BACKEND_CAPABILITIES + Default, 6 construction sites |
| `0fcafcb0` | `75752b38`+`c121b79e` | **2:1** | feat(melib) | upstream `75752b38` alone does not compile (imap capabilities is a full literal missing the new field — broken intermediate); supports_raw_search + `raw_search` trait default (NotSupported) + Gmail X-GM-EXT-1 detect + UID SEARCH X-GM-RAW literal. Fork-only: mock continuation arm, `run_imap_raw_search_gmail`, `test_maildir_raw_search_not_supported`, pre-existing default-features test compile fix (un-gate `set_test_xdg_env`/`fetch_all_envs`, 19× `cfg!` offline_cache; 39 E0425 on base — folded into this commit rather than a prep commit, noted here) |
| `4c83d5aa` | `2825d224` | port | feat(notmuch) | raw_search passthrough (mailbox query_str prefix concat) + tests; local run skipped (no notmuch binary on dev machine) — CI installs notmuch and runs for real |
| `32590b9d` | `a072648f` | port | feat(command) | raw-search/raw-select commands, Search/Select tuple→struct variants (9 listing arms etc.), man +15 lines; **sqlite3-search-backend ignores raw_search quirk preserved verbatim** (upstream HEAD unfixed); fork-only: parser unit tests + account.search wiring discriminator test |
| `8ef3a7c0` | `ad3252b0` | port | ui | StatusBar ascii mouse fallback under ascii_drawing |
| `00d3b65d` | `ab62f82a` | port | fix(compose) | toggle stuff guarded under gpgme feature |
| `29987b9c` | `d2c91b59` | port | feat(pgp) | cleartext verification: `UnverifiedSignature` + `extract_unverified_signature` + `Context::verify_cleartext` + safety warning; upstream test ported verbatim (same test key, md5-verified); fork-only: melib 3-branch unit tests; CI apt + `gnupg libgpgme11` |
| `0d2e1d7a` | `d5e9360c` | **PARTIAL** | feat(view) | non-view hunks only (mail.rs un-gate + pgp.rs function-level cfg, byte-identical) + fork routing: text/plain cleartext → existing SignedPending/SignedVerified pipeline, armored text shown unstripped; filters.rs untouched. NOT ported: ViewFilter/FilterOutputMetadata machinery (view layer rewritten in ratatui waves) — see debt |
| `81037c3d` | `1b837d42` | port | refactor(view) | dead `EnvelopeView::html_filter` field removed |
| `46bba4bb` | `0cad2282` | port | refactor(melib) | LocateKey moved gpgme→email::pgp; overrides.rs regenerated via sentinel |
| `77d12462` | — | fork-only | test(melib) | negative-path tests for extract_unverified_signature Detached arm (adversarial-review finding: happy-path-only tests survived micalg-validation removal) |

- Debt register:
  - FilterOutputMetadata not ported (decryption recipients / per-filter signature status display) — align on fork's SignedVerified pipeline within 1-2 syncs
  - filters.rs ViewFilter path does not trigger cleartext verification (mail opened via ViewFilter bypasses the envelope.rs routing)
  - imap test-infra fork: upstream imap test commits must be reimplemented (see SKIP b08a39b3)
  - sqlite3 search backend ignores raw_search (upstream quirk, verbatim)


- Method: semantic port
- Upstream commit: `4f2414a3`
- Merge commit: `05d8b13c` (branch `meli-upstream-sync-24h`, translating the upstream cache-then-resync ordering change)

| Local commit | Upstream commit | Type | Notes |
| --- | --- | --- | --- |
| `c250f34c` | `4f2414a3` | fix(melib) | IMAP sync now fetches from cache first, then resyncs, reconciling sets against server truth (cache-then-resync ordering) |

## 2026-09-11 22:52 (UTC+8)

- Method: bulk port on a branch (branch `sync-upstream-meli`, based on `e9f480c`)
- Merge commit: `71093054` — 8 upstream fix/perf commits in total
- Conflicts: `meli/src/mail/listing.rs` was the only conflicted file; kept BOTH sides — main's `#[cfg(test)] mod listing_menu_tests` stays in place at the end of the file, and the sync branch's `TagsIterator` (struct + impls) is appended after it, both verbatim; the `focus_right` sidebar hunk (~:2198) auto-merged to main's version untouched
- Follow-up: `c05f976e` (2026-09-12) — moved the test module after `TagsIterator` and fixed stable-rustfmt drift

| Local commit | Upstream commit | Type | Notes |
| --- | --- | --- | --- |
| `4eb6f8b1` | `2b7c2123` | fix(melib) | Error classification: classify by `ErrorKind` before falling back to raw errno |
| `8d076703` | `48490fcc` | perf(melib) | `examine_mailbox()` accepts an existing SELECT, reusing connection state |
| `b8f8ec91` | `4ca86e2a` | fix(melib) | `get_text_recursive()` checks the text content_type |
| `d5af6d3e` | `40cab0f7` | perf(melib) | Added a `FetchState::response` buffer (semantic port) |
| `16846e42` | `18654e06` | perf(melib) | FetchState cache stages avoid a forced re-SELECT (semantic port) |
| `d86129b1` | `5863123b` | refactor(meli) | Added `TagsIterator` (semantic port) |
| `219a85d0` | `1713ec06` | fix(meli) | Give 1 cell of room when printing a tag |
| `f52f1fdb` | `31298d3f` | feat(meli) | Added the `tags.rename` setting |
