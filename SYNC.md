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

## 2026-10-23 (UTC+8)

- Method: per-commit semantic port (task S5 of the `1ada94c3..3b543312` range, small fixes batch)
- Upstream range: `1ada94c3..3b543312` (this task: `ad6f2b68`, `d2470aa8`, `eec57247`, `8180e9c2`, `e4b5d4d1`, `2bb886ee`, `a5d5e862`, `ef05916a`)
- Conflicts: none (fork-kept deviations: byte-input command parsers with fork `argcheck` convention, `ShortcutKeys` binding type instead of bare `Key`, fork's `CmdInput`+`ChangeMode` command-input chain, `server_password_command` config format in tests, `PASSED` flag keeps its `$passed` keyword mapping). Intentional behavior change for the release notes: the help/shortcuts panel search trigger key moved from `listing.search` (`/` or `F3`) to `general.search` (`/` only), so a user remap of `listing.search`'s `F3` no longer opens the help-panel search)
- Accounting kinds: see the SKIP notes in the table

| Local commit | Upstream commit | Type | Notes |
| --- | --- | --- | --- |
| — | `3b543312` | port | fix(meli) | Respect shortcut config overrides (upstream #226), 11 sites switched from global `context.settings.shortcuts` reads to override-aware resolution: the four listing kinds' `Component::shortcuts` (`compact.rs`/`plain.rs`/`conversations.rs`/`listing/thread.rs`) and `Listing::shortcuts`' LISTING section (which used to overwrite the children's maps with the global one) now resolve `mailbox_settings!` (mailbox → account → global, `context has` guard so still-loading accounts don't panic); `Listing::process_event` keeps a focused child's own GENERAL section and otherwise resolves the open mailbox's override (upstream's `contains_key` guard); `ContactList` (`contacts/list.rs`), `AccountMailboxManager` (`manage/mailboxes.rs`) and `AccountStatus` (`mail/status.rs`) resolve account-level `contact_list`/`general` via `account_settings!`. Account-agnostic components keep the global reads with a comment, matching upstream which passes `None` (global) for them too: `Pager`, `Tabbed`, `UIDialog`/`UIConfirmationDialog`, `JobManager`. COVERED (fork already override-aware, upstream's diff there was a mechanical refactor onto its new Context methods): `mail/compose.rs` + `compose/edit_attachments.rs` (`account_settings!`), `mail/view/thread.rs` + `mail/view.rs` + `mail/view/state.rs` (`mailbox_settings!` for thread_view/envelope_view), listing `commands` dispatch in `mail/listing.rs` (guarded `mailbox_settings!`); `utilities/listings.rs` has no fork equivalent. NOT ported: upstream's 682-line `Context::{general,listing,composing,envelope_view,thread_view,contact_list,pager}_shortcuts`/`*_commands` layer and per-field `Option<Key>` `*ShortcutsOverride` structs — under the fork's whole-struct override model an override block replaces the whole section (unset fields take the hardcoded defaults, not the global values), which is the fork's pre-existing config semantics, unchanged here; fork-specific quit pre-arm reads (`utilities.rs`, `mail/listing.rs`) stay on global settings (no upstream counterpart). Regression test `mail::listing::listing_menu_tests::listing_shortcuts_respect_config_overrides` (red before the fix, green after). KNOWN GAP (review follow-up): the `general.scroll_*` fallbacks in `mail/compose.rs` and `compose/edit_attachments.rs` still read the global settings (`context.settings.shortcuts.general.scroll_{right,left}`), while upstream `3b543312` resolves them through the now account-aware GENERAL shortcut map with the global read only as the final fallback — minor, not ported here. OVERRIDE-MODEL NOTE (review follow-up): the fork's override model replaces a whole section (`ShortcutsOverride` holds `Option<CompleteSection>`), whereas upstream #226 merges the `general` section field-by-field; the net effect matches in the "global customization + account defaults" scenario, but the semantics differ — in the fork, fields left unset inside an override block take the hardcoded defaults, not the global values |

## 2026-10-20 (UTC+8)

- Method: per-commit semantic port (task T6 of the `1ada94c3..1e66d41e` range, conf preprocessing)
- Upstream range: `1ada94c3..1e66d41e` (upstream `9a9fee81` only)
- Conflicts: none (fork-kept deviations: depth>7 cyclic-include guard, infallible `xdg::BaseDirectories::with_prefix`, melib-root import style; upstream's `expand_config` removal adopted — fork had no external callers)
- Accounting kinds: see the notes column

| Local commit | Upstream commit | Type | Notes |
| --- | --- | --- | --- |
| — | `9a9fee81` | port | refactor(meli/conf) | remove m4 runtime dependency: `Preprocessed`/`Spanned` + stack-based (`VecDeque`) pure-Rust include expansion replaces both `Command::new("m4")` pipelines; `M4_PREAMBLE`s and `expand_config` deleted; missing-directory include errors unified on upstream's line-numbered message (more informative than the fork's two divergent styles); fork's recursion guard kept where upstream dropped it; docs: `meli.conf.5` include directive wording no longer references m4, `BUILD.md` termux pkg list drops m4; tests: nested/cyclic/self/missing/directory/non-UTF-8/malformed includes + no-`PATH` no-subprocess proof (issue #186) |

## 2026-10-20 (UTC+8)

- Method: per-commit semantic port (task T5 of the `1ada94c3..1e66d41e` range, melib misc improvements)
- Upstream range: `1ada94c3..1e66d41e` (RFC2369 / thread subject trim / percent_encoding / attachment accessors only)
- Conflicts: none; all fork callers adapted mechanically to the owned `ListAction` API (`meli/src/mail/{view,compose}`, `meli/src/mail/view/envelope.rs`, seven CVE corpus suites, `meli-test/tests/test_unsubscribe_confirm.rs`) without adopting upstream's dialog-based UI
- Accounting kinds: see the SKIP/PARTIAL notes in the table

| Local commit | Upstream commit | Type | Notes |
| --- | --- | --- | --- |
| — | `f819b9d5` | port | feat(melib/list_management) | RFC2369 full support: `ListAction` owned `Vec<u8>`, `HeaderName`-keyed lookup, six `List-*` action headers (`Archive`/`Post`/`Unsubscribe`/`Subscribe`/`Help`/`Owner`), empty options filtered, `is_url()`/`is_email()`; `List-Archive` now parsed as an RFC 2369 options list (stricter: bare unbracketed values no longer accepted); `mailto.rs` `TryFrom` error `String`→`Error` with `set_details`. Upstream's view-layer unsubscribe-selection dialog is UI design and NOT ported (fork keeps its confirmation dialog + CVE-2023-23397 scheme gate) |
| — | `b61be3d6` | port | fix(melib/thread) | `SubjectPrefix::strip_prefixes` FIXME: per-character `"`/`\t`/`\r` loop replaced by `trim_ascii_start()` — every ASCII whitespace kind (`\n`, `\x0c` included) after `Re:`/`Fwd:` prefixes is now trimmed; fork's `&str` impl delegates to the bytes impl so one site covers both |
| — | `9d0c3e9a` | port | feat(melib/percent_encoding) | `RESERVED` ASCII set from RFC 3986 §2.2 (gen-delims + sub-delims); upstream's duplicate `.add(b'(')` typo corrected to the intended `.add(b'\'')` so the set is exactly `!#$&'()*+,/:;=?@[]` |
| — | `e596371b` | port | refactor(melib/percent_encoding) | `percent_encode`/`utf8_percent_encode`/`PercentEncode` take `&'s AsciiSet` (any lifetime, runtime-constructible sets); existing `&'static` call sites unchanged |
| — | `5a3160bd` (melib side) | port | feat(melib/attachments) | `Attachment::content_id()` — parses the part's `Content-ID`, strips `<>`, RFC 2047 phrase-decodes |
| — | `3827316e` (melib side) | port | feat(melib/attachments) | `Attachment::parts()` instance accessor; upstream's view/filters path-from-root rewiring is UI and NOT ported |
| — | `b52a8675` | **SKIP** | chore(melib) | remove public dep re-exports: the fork no longer has those re-exports |
| — | `e1272d9f` | **SKIP** | fix(melib/logging) | debug logs not written to file: the fork's tracing-based logging is an intentional deviation |
| — | `d54dff03` | **SKIP** | chore | assert_cmd 2.2.2 upgrade: fork's `Cargo.lock` already resolves `assert_cmd` 2.2.2 |
| — | (VERIFY/TODO/clippy cosmetic commits in range) | **SKIP** | style | pure appearance cleanups outside this task's scope |

## 2026-10-20 (UTC+8)

- Method: per-commit semantic port (task T2 of the `1ada94c3..1e66d41e` range)
- Upstream range: `1ada94c3..1e66d41e` (IMAP/ManageSieve cluster only)
- Conflicts: none (test harness reimplemented under fork conventions: tracing instead of `log`, no `rusty_fork`, mock-server flags instead of upstream's event-queue redesign)
- Accounting kinds: see the SKIP/PARTIAL notes in the table

| Local commit | Upstream commit | Type | Notes |
| --- | --- | --- | --- |
| — | `3b5670e9` | port | feat(melib/imap) | PREAUTH greeting: `ImapStream::preauthed` + skip login/lazy-CAPABILITY/ID + `AccountStateChange` with the server message; detection scans the `M1 CAPABILITY` reply buffer (the fork, like upstream, sends `CAPABILITY` before anything else, so the greeting rides along in that reply) |
| — | `0b501a1f` | **PARTIAL** | feat(melib/imap) | OAUTHBEARER (RFC 7628) alongside XOAUTH2: capability-based selection, TLS requirement, SASL string built from username+token with legacy base64-string recovery, SASL-IR and continuation forms, SASL failure continuation decoded into error details with the mechanism's cancel token; token resolved from `Secret` only at send time (fork redaction policy). NOT ported: the `new_connection`→`new`+`authenticate()` split and `UIDStore::preauth_capabilities` — the fork's inline auth keeps connection-pool hardening intact and already merges pre/post-auth capabilities in one set (documented in issue #182); docs example updated to the raw-token command form |
| — | `c063b4a8c` | **SKIP** | fix(melib/imap/untagged) | skip fetch on `(0, 0)` EXISTS: fork's cache-first model already early-returns on `current_exists >= n` |
| — | `747676ab1` | port | fix(melib/imap/untagged) | equal-count EXISTS (`current_exists == n`) probes the highest MSN once (`FETCH n`), catching an expunge+arrival swap whose total is unchanged; the probe is best-effort (a stale EXISTS may reference an expunged MSN; the UID resync stays authoritative) and dedup by UID suppresses duplicate Create events |
| — | `2e408a029` | port | fix(melib/imap/managesieve) | literal framing via `send_raw` (no doubled CRLF), replies read with `read_lines(Some(b"OK"))`, `getscript` parses `{N+}` non-synchronizing literals (local parser extension), `ManageSieveConnection::connect()` added; `havespace`/`renamescript` fake-`Ok` stubs did not exist in the fork (removed earlier), nothing to fix; upstream's `putscript` drops the `OK` line it validates against (`keep_termination_string = false`) — the fork keeps it so `response_oknobye` can parse the reply |
| — | `4cc43b035` | port | feat(meli) | `tools managesieve-shell` REPL subcommand (CLI smoke test for `--help`); upstream's `tools/src/managesieve.rs` binary never existed in the fork |
| — | `0d23bddf7` | port | test(melib-test/imap) | 72 mock-server `eprintln!`s converted to `tracing::debug!`/`trace!` (fork tracing convention; output visible through `melib_test::init_test_logging`) |
| — | `79a8c8291` | port | test(melib-test/imap) | TLS support: `ImapServerStream::new_with_tls` wraps the accepted socket in `Connection::new_tls` (WouldBlock handshake loop); negative cert-validation case, PLAIN/OAUTHBEARER/XOAUTH2 over TLS |
| — | `7092d90cf` | port | test(melib-test/imap) | SASL continuation form: the mock answers the bare `AUTHENTICATE` with `+ ` and asserts the base64 exchange line (byte-exact RFC 7628 form) |
| — | `43b0a52e4` | port | test(melib-test/imap) | `send_capabilities_on_auth` mock mode: capabilities glued to the auth reply; the client must not re-send `CAPABILITY` (asserted via the handshake command log) |
| — | `6ef74bf8f` | **SKIP** | test(melib-test/imap) | exit stream recv on client disconnect: the fork's `read_line` already distinguishes EOF (`ReadOutcome::Eof` breaks the loop); read *errors* are now also mapped to EOF instead of panicking |
| — | `5fddcb89a` | **SKIP** | test(melib-test) | `test_imap_fetch` cache-enabled check: the fork's cache-dependent tests are already `#[cfg(feature = "sqlite3")]`-gated |



## 2026-10-13 20:00 (UTC+8)

- Method: per-commit semantic port on seven parallel worktree branches (`sync/w1-jscontact`, `sync/w1-contacts-core`, `sync/w2-carddav`, `sync/w3-jmap`, `sync/w5-smallfix`, `sync/w4-contacts-glue`, `sync/w6-backend-api`) + two contract commits on main (`f0f859d`, `3e0eaab`) that landed the `ContactBackend`/`AddressBook` types ahead of the branches as a shared compilation base
- Upstream range: `aea4508b..1ada94c3` (79 commits; 46 ported, 33 SKIP/deferred)
- Merge commits: `e2e0dc3` (jscontact), `caa1249` (contacts-core), `26b648b` (carddav), `6f8b50e`+`d690759` (smallfix, split-history merge), `2cf010c` (jmap), `619ae1a` (backend-api), `d7a5a7f` (contacts-glue); port-induced integration fixes `11cc78d`, `32cb8ee`, `ee786ee`, `b23b781`/`bd115d4`, `bb10dc4`
- Conflicts: one textual (`melib/src/imap/mod.rs` capabilities literal — resolved to the `..Default::default()` form); three semantic, all the same class (w5's `Error::set_source` now takes `Box` while w1/w2/w3 code still used `Arc` → `11cc78d`, `ee786ee`, `b23b781`); one port-induced regression (UTF-7 mailbox decode match-arm typo → `bb10dc4`)
- Verification: `make lint`, full `cargo test --workspace`, `cargo test -p cve` **1174/1174** (hard user gate: the cve corpus is frozen and must pass unmodified), `cargo check/test` with `webdav` and `jmap` features, CVE-2021-31855 and CVE-2001-0145/-1088/-2006-2386 anchor suites individually re-run green
- External concurrent work: three commits landed on main during the window from a parallel session (issue #100 IMAP SEARCH retry hardening: `5d6a300`, `f464a05`, `720736f`) — reviewed, tested, retained; **not** part of this sync

| Upstream | Local | Type | Notes |
| --- | --- | --- | --- |
| `62c24b52`+`3e7133b9`+`5eda1ee1`+`6c61f34b` (contract) | `f0f859d`+`3e0eaab` | feat | `ContactBackend` trait + `AddressBook`/`AddressBookName`/`ContactBackendID` in `contacts/backend.rs`; fork deviation: flat `Contacts` kept (CVE-2001-1088/-2006-2386 pin its text), aggregate `Contacts`/`contacts_new` not adopted |
| `b61d1178`+`d08d69e7`+`195aabb6`+`06a63d1b`+`3a98bf00`+`c5b0faaa` | `6775ce5` | feat | JSContact to upstream final shape (serde, schema fix, Send+Sync, Clone, name components, `From`/`Into` Card APIs) — 6:1 |
| `1554983e` | `aa066bc` (part) | feat | mutt alias ContactBackend |
| `982dd328`+`d5ecc5a8` | `aa066bc` (part) | feat | notmuch address ContactBackend with `--output=recipients` — 2:1 |
| `254d8a5a` | `aa066bc` (part) | feat | vCard folder ContactBackend |
| `8b82457a` | `aa066bc` (part) | fix | Card Display via `as_address()` (fork already had it; gap-fill) |
| `d771b08f` | `266068c`→`b710580` | **deferred** | vCard value folding — ported then reverted: frozen CVE-2001-0145 pins folded-BDAY-as-clean-error; revisit when corpus unfreezing is approved |
| `88c8451c` | `872b2ae`+`b88bf93`+`12e9c2d` | feat | WebDAV transport + CardDAV backend (`webdav` feature); fork hardening: TLS-only URLs (loopback excepted), same-origin redirect + userinfo strip, only `danger_accept_invalid_certs`; isahc 2 API; quick-xml 0.38.4 (upstream 0.31, offline cache) |
| `4325dbe1` | `17eeedd` | fix | JMAP Set responses only carry server-set properties |
| `085de222` | `d5a2096` | fix | identities on `JmapConnection` (upstream: `Store` — frozen CVE-2024-50624 corpus pins `Store`'s field list); survives session refresh |
| `c1b32e3a` | `5bdabcf` | fix | `EmailFetcher.mail_account_id` |
| `d5ba7be8` | `4c509b7` | fix | SSE stream timeout zero, connect timeout fixed 60s; fork `RedirectPolicy::None` kept |
| `834679d6` | `50823a0` | refactor | Mail/Submission capabilities into module (fork already had `capabilities.rs`; gap-fill) |
| `281da6a3` | `5f7402c`+`aabcbdc` | feat | JMAP Contacts backend + `supports_contacts`/`contact_backend()` on `MailBackend` |
| `5b68048d` | `7ee2001` | test | test server tracks keywords; Email/set+import, upload endpoint, SSE replay |
| `b0c6bcda` | `a7bddea` | fix | SMTP 530 reply code parsing (closes fork's pinned known-gap) |
| `c83c9e67` | `3283b55`+`af09556` | fix | no duplicated period in error chains |
| `9cc5293d` | `71c0818` | refactor | `set_source(Box)` downcasts `Self`, preserving source chains |
| `f1bd593d` | `f40c9b4` | fix | error Display always shows details/related path |
| `ab506b45` | `81ca6b7` | fix | `ServerPasswordCommand` migration chains related path |
| `d628a7dd` | `5168587` | fix | NNTP submission CRLF canonicalization |
| `d4d83770`+`b0b2aa1e` | `c44a5c0` | refactor | `convert_attachment_to_rfc_spec` → utils, returns `Cow` — 2:1 |
| `924d00f8` | `2d91501`+`32cb8ee` | feat | `MailboxConf.extra` arbitrary TOML values + `i64`; account-level Number/Bool coercion untouched; test literals migrated |
| `d6b2bc79` | `4157ed0` | fix | temp path extension reset when path exists |
| `e8276c7a` | `36cddb9` | fix | gpgme alternative library names |
| `9e0d441d` | `0feff2b` | refactor | IMAP capabilities via `Default` completion |
| `c8f0a95e` | `c070133` | feat | `From<bool> for IsAsync` |
| `c3de4afd` | `14edbe1` | feat | `meli migrate` CLI (list/apply/revert), adapted to fork's v0_11_0 migrations |
| `4dd81ffa` | `9421852` | refactor | `submit`/`submit_batch` drop unused `flags` |
| `eeb91753` | `6119c94` | feat | **PARTIAL/detour**: `save_with_tags` new trait method (default delegates to `save`); upstream re-signs `save()` which CVE-2021-31855 pins — full upstream shape deferred |
| `7cf9e3c2` | `a3c9294` | feat | notmuch save-to-mailbox with tag indexing (`SaveToConf`) |
| `fa4a3daa` | `3bbd5cc` | refactor | hand-written `Message` Debug |
| `b2360adf` | `6a730e2` | perf | database revision/uuid no-change short-circuit |
| `cb633c63` | `5b4df0e` | fix | refresh filters by mailbox hash |
| `db9f7f93`+`14640216` (conf+glue) | `fc3a00f`+`99583fb`+`7f207c3` | feat | `ContactBackendConf` + `accounts.contacts` map + account glue feeding the flat store; legacy sources kept as fallback/overlay (upstream replaces them); `dc14fbbf` job-metadata fix included |
| `dc14fbbf` | `7f207c3` (part) | fix | contact job `is_async` from backend capabilities |
| UI/terminal/theme ×22 (`1ada94c3` `24f832df` `9dbf2614` `9d5592a2` `239e162b` `d32858d1` `168410ab` `15ec10ec` `21895e71` `b5d389db` `c9182179` `9182179c` `82d3a4b5` `64d89b8e` `0cc149ee` `33147fad` `a0dc6d37` `5d41a2bc` `e6b2bdfe` `1c9cbe30` `fc9ab541` `3fa4ec87`) | — | SKIP | UI layer — fork UI architecture supersedes (policy); `1c9cbe30`+`e6b2bdfe` already superseded by the 2026-10-01 multi-address autocomplete port |
| sequoia group ×6 (`2fa32889` `16ada4cb` `6af4a32e` `6e0740db` `d3640f72` `edaa6736`) | — | SKIP/deferred | `sqz` 0.1.0 + nettle build deps vs dependency governance; fork ships gpgme+cli PGP backends; MSRV stays 1.85 |
| docs/cleanup ×4 (`c6c49d77` `dcf153ee` `e4098bf9` `5aa4fb7a`) | — | SKIP | man-page docs diverged; FIXME churn; theme tag refs are UI layer |

## 2026-10-03 01:20 (UTC+8)

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

## Downstream mirror log

Entries below record publishes of this fork to its public mirror
[hardened-meli](https://github.com/kylelee/hardened-meli) (independent root,
kept tree-identical to this repository at each publish).

### 2026-10-13 (UTC+8)

- Method: atomic publish — `git cherry-pick -x -X theirs 0222891..7fce2bf` of this fork's post-v0.11.0 commits (67 total; 13 merge nodes skipped), one explicit `align` commit clearing replay residue, one `--no-ff --signoff` merge to `master`
- Outcome: **50 picks** with `-x` provenance lines + **4 empty-drops** (changes already present in the mirror's interim port: jscontact final shape, JMAP eventsource timeout, `ServerPasswordCommand` related-path, webdav `set_source` Box adaptation — content guaranteed by the align gate, not lost) + 1 align + 1 merge
- Hard gate: `git diff` against this fork's `7fce2bf` = **0** (tree-identical, re-verified on the published tip)
- Superseded in the mirror by this publish: its interim self-directed upstream sync (`aea4508b..168410ab`, 73 commits — aggregate-Contacts design, v0.12.0, UI ports, sequoia backend, frozen-CVE-corpus assertion edits), removed by the align commit (visible deletions: `melib/src/sequoia.rs`, `meli/src/version_migrations/v0_12_0.rs`)
- Verification in the mirror repo: `make lint`, full `cargo test --workspace` (28 suites), `cargo test -p cve` **1174/1174**, sensitive-info scan clean (no internal domains/private IPs/credentials in messages or historical blobs)
- Publish: force-push to `origin/master` pending (sandbox has no `ssh`; to be run from the user's shell)

### 2026-10-20 (UTC+8)

- Method: atomic publish — `git cherry-pick -x -X theirs 7fce2bf..b87b971a3` (43 commits; 1 merge node skipped), one explicit `align` commit, one baseline clippy fix landed here first and picked, one `--no-ff --signoff` merge to `master`
- Outcome: **41 picks** with `-x` provenance lines + **1 empty-drop** (the 2026-10-13 ledger entry, already content-equivalent on the mirror) + 1 align + 1 merge; the 4 AGENTS doc commits expected to drop empty replayed instead as converging picks (the mirror-side equivalents sat at intermediate wording states; final trees identical either way)
- Replay fix (align): `-X theirs` let the `cve_2026_96869` module block land at a divergent position and a later pick drop it; the align commit restored it at the source-tip position (backed by the diff=0 gate)
- Baseline fix: the fork tip carried a clippy break in `cve_2026_43864` (`unreachable_pub` x5, `useless_format`, `bool_comparison`) — fixed here first (visibility narrowed to `pub(super)`, the `format!`-escaped literal inlined unescaped, `== false` turned into a negation) and cherry-picked to the mirror; module tests 6/6
- Identity unification: `611ada51` re-authored post-pick to Kyle Lee <kylelee@users.noreply.github.com> (sign-off rewritten, author name kept); new-range author/committer/sign-off verified unique
- Hard gate: `git diff` against this fork's `main` tip = **0** (tree-identical, re-verified on the published tip)
- Verification in the mirror repo: `make lint`, full `cargo test --workspace` (29 suites, 2634 tests), `cargo test -p cve` **1427/1427**, sensitive-info scan clean (new-range commit messages and both trees: no internal domains/repo names)
- Publish: push to `origin/master` pending (fast-forward, no history rewritten; to be run from the user's shell)
