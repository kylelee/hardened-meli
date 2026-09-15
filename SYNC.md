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
