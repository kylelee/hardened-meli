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
//!
//! CVE-2006-1045 (issue #15) attacks the reference-form side of the
//! same invariant: Thunderbird's "block remote images" preference
//! was bypassed by reference forms it did not classify —
//! [`cve_2006_1045`] proves every form (`<img src>`/`srcset`, CSS
//! `background:url()`, `@import`, `<video poster>`, `<source src>` and
//! relatives) dies in `sanitize` in every embedding context.
//!
//! CVE-2008-3068 (issue #16) moves from the rendering path to the
//! crypto trust boundary: Outlook's CryptoAPI dereferenced AIA/CRL
//! URLs embedded in an S/MIME certificate during revocation checking,
//! turning every mail read into an encrypted-receipt beacon.
//! [`cve_2008_3068`] proves meli's equivalent surface immune with a
//! genuinely signed corpus: the CMS part stays an opaque
//! `CMSSignature` blob, the PGP verification gate never dispatches
//! S/MIME to an engine, and every gpgme context is born offline,
//! local-only and non-retrieving — so no certificate-borne URL can
//! ever be fetched.

//! CVE-2008-4491 (issue #17) turns to the draft store: Apple Mail kept
//! drafts of S/MIME-encrypted mail as plaintext on the server.
//! [`cve_2008_4491`] maps the attack onto meli's composer draft
//! lifecycle with a genuine gpg-encrypted PGP/MIME corpus: the wire
//! form and melib parsing surface no session plaintext, quoting the
//! decrypted view is exactly what serializes it — and the composer's
//! save policy (no auto-save, no silent plaintext persistence of an
//! encryption-armed draft, warnings on explicit saves, encrypted
//! post-submission copies) is what keeps those bytes off the server.
//!
//! CVE-2017-17688 (issue #18) is the EFAIL chapter: OpenPGP CFB
//! malleability grafts an HTML exfiltration gadget onto decrypted
//! plaintext. [`cve_2017_17688`] proves the gadget corpus genuine (real
//! gpg ciphertext, real bit-flip variants, real tampered-decryption
//! outputs) and locks meli's walls: the gadget dies in `sanitize`
//! before rendering, parts never splice, and decryption failure
//! surfaces no plaintext — closing the CLI backend's stdout-echo gap
//! the issue exposed.

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

/// CVE-2006-1045 (Mozilla Thunderbird 1.5) tracking-pixel regression
/// (issue #15): the "block remote images in HTML mail" preference was
/// bypassed and externally linked resources loaded anyway. This corpus
/// proves meli's equivalent surface immune: remote-content stripping is
/// unconditional — every reference form (`<img src>`/`srcset`, CSS
/// `background:url()`, `@import`, `<video poster>`, `<source src>` and
/// era relatives) dies in `sanitize` in every embedding context, and
/// the rendered text keeps only the manual unsubscribe footnote.
#[cfg(test)]
#[path = "CVE-2006-1045.rs"]
mod cve_2006_1045;

/// CVE-2008-3068 (Outlook, Windows Live Mail, Office 2007 CryptoAPI)
/// encrypted-receipt beacon regression (issue #16): an S/MIME mail
/// whose embedded certificate carries attacker AIA/CRL-distribution-
/// point URLs; the victim's client fetched them automatically during
/// CRL revocation checking, leaking read time and IP. The corpus is a
/// genuinely valid openssl-signed S/MIME mail, and the regression
/// proves the equivalent meli surface immune layer by layer: the CMS
/// part parses as an opaque blob whose URLs surface nowhere in mail
/// metadata, the only signature-verification gate rejects non-OpenPGP
/// protocols (and a smuggled carrier still hands the engine opaque
/// bytes only), and every production gpgme context is created offline
/// with local-only key location — per the GPGME manual, offline mode
/// bars Dirmngr's CRL/OCSP validation for CMS and disables Dirmngr
/// entirely for OpenPGP, so no certificate-borne URL is ever
/// dereferenced.
#[cfg(test)]
#[path = "CVE-2008-3068.rs"]
mod cve_2008_3068;

/// CVE-2008-4491 (Apple Mail.app 3.5) plaintext-draft-at-rest regression
/// (issue #17): with "store drafts on server" enabled, drafts of
/// S/MIME-encrypted mail were saved as plaintext on the server, readable
/// by the server operator and intermediaries. This corpus maps the
/// attack onto meli's composer draft lifecycle: a genuine gpg-encrypted
/// PGP/MIME mail whose secrets exist only off the wire; parsing and
/// reply-quoting of the stored form leak nothing; quoting the decrypted
/// session view is what produces the plaintext draft — and the tests
/// document that the bar keeping it off a server-side Drafts mailbox is
/// the composer save policy (no auto-save, submission-setup failures
/// refuse to persist encryption-armed drafts, explicit saves warn,
/// post-submission copies are the encrypted wire form), locked by the
/// meli regressions named in the module docs.
#[cfg(test)]
#[path = "CVE-2008-4491.rs"]
mod cve_2008_4491;

/// CVE-2017-17688 (EFAIL, OpenPGP variant; 11 clients incl. Apple Mail,
/// Thunderbird, Outlook) CFB-malleability exfiltration regression
/// (issue #18): an attacker flips one ciphertext block of an
/// intercepted PGP/MIME mail so the decrypted plaintext opens an
/// unclosed `<img src="https:` gadget whose URL tail is the
/// confidential remainder — and a client that renders it (or echoes it
/// in a decryption error) exfiltrates the plaintext. The corpus is a
/// genuine gpg-encrypted mail plus its two bit-flipped variants (real
/// `gpg --ignore-mdc-error` outputs embedded), and the regression locks
/// the walls layer by layer: the wire carries no plaintext, the
/// tampering is pure CFB XOR, the decrypted gadget dies in `sanitize`
/// before rendering (no fetcher exists to begin with), no code path
/// splices decrypted bytes with attacker-controlled outer parts, and
/// decryption failure surfaces no plaintext fragment — the gap this
/// issue closed: the CLI backend's error used to embed the failing
/// decrypt script's captured stdout byte-for-byte, which GnuPG fills
/// with literal data *before* the MDC verdict.
#[cfg(test)]
#[path = "CVE-2017-17688.rs"]
mod cve_2017_17688;

/// CVE-2017-17689 (EFAIL, S/MIME variant; 11 clients incl. Apple Mail,
/// Thunderbird, Outlook) CBC-malleability exfiltration regression
/// (issue #19): an attacker XORs the IV of an intercepted
/// `application/pkcs7-mime` enveloped-data mail so the first decrypted
/// block becomes an unclosed `<img src="https:` gadget whose URL tail
/// is the confidential remainder — CBC has no integrity check, so the
/// whole tail still decrypts to the original bytes and the decrypting
/// client (or its renderer) exfiltrates the plaintext. The corpus is a
/// genuine openssl S/MIME envelope (AES-128-CBC) plus its IV-flipped
/// variants (real `openssl smime -decrypt` outputs embedded), and the
/// regression locks the walls layer by layer, complementing
/// [`cve_2017_17688`]'s CFB corpus: the wire carries no plaintext,
/// the tampering is a pure 16-byte IV XOR with no damage trail, meli's
/// decrypt entry refuses the single-part S/MIME carrier before any
/// engine is consulted, a smuggled `multipart/encrypted` carrier that
/// does reach an engine fails without leaking plaintext through
/// either real backend, the gadget dies in `sanitize` before
/// rendering, and the direct-exfiltration decoy can never splice with
/// decrypted bytes.
#[cfg(test)]
#[path = "CVE-2017-17689.rs"]
mod cve_2017_17689;

/// CVE-2026-0818 (Thunderbird < 147.0.1, ESR < 140.7.1; MFSA
/// 2026-07/08) CSS-exfiltration regression (issue #20): an HTML+CSS
/// mail wraps an inline OpenPGP block in a stylesheet of extraction
/// oracles — attribute-prefix selectors, `@font-face`
/// `unicode-range` font probes and a `@keyframes`/`steps()` timing
/// probe — and a client that decrypts in place renders the decrypted
/// content *inside the attacker document's CSS context*, leaking the
/// secret one fetch (or one timed tick) per character. The corpus is
/// a genuine gpg-encrypted inline-PGP mail (real cv25519 armor,
/// CRC-24-verified in-test, decrypt-verified round-trip to the
/// embedded plaintext) in both delivery forms — the advisory's
/// `multipart/alternative` (armor mid-prose, never dispatching in
/// meli) and the strongest dispatchable `multipart/mixed` (armor-only
/// part, whose decrypt output re-enters as its own attachment) — and
/// the regression locks the walls layer by layer: the wire carries no
/// plaintext, the whole kit (`<style>`, `style=`, `<link>`, `<font>`,
/// every marker) dies in `sanitize` in every embedding context, the
/// decrypted view renders through its own `sanitize` with the secret
/// delivered but zero probe references, and even the Thunderbird
/// splice (outer kit document concatenated with decrypted HTML, both
/// orders) sanitizes to an inert fixed point — meli's terminal-text
/// pipeline has no CSS engine to inherit from at all.
#[cfg(test)]
#[path = "CVE-2026-0818.rs"]
mod cve_2026_0818;

/// CVE-2023-23397 (Microsoft Outlook on Windows, CVSS 9.8, CISA KEV,
/// APT28) zero-click credential-theft regression (issue #21, table 2
/// of `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution): a
/// malicious `text/calendar` meeting invite sets the reminder sound
/// (`VALARM` `ATTACH;VALUE=URI`, MAPI `PidLidReminderFileParameter`)
/// to a Windows UNC path `\\attacker.example\share\a.wav`, and
/// Outlook dereferenced it at reminder time with no user interaction,
/// authenticating to the attacker's SMB server with the victim's NTLM
/// credentials. meli has no MAPI store, no reminder subsystem and no
/// SMB client, so the equivalent surfaces — every place
/// mail-controlled bytes can become an "openable link" the OS url
/// launcher dispatches — are mapped and locked: the calendar part
/// renders through the plain-text `InlineText` branch with no
/// iCalendar interpretation, the pager's linkify extraction cannot
/// carry the UNC's backslash tail (and every `file:`/`smb:` URL it
/// does surface is held behind the `go_to_url` confirmation gate),
/// the HTML mirror's scheme-bearing hrefs die in `sanitize`, and —
/// the real gap this issue exposed — the two mail-header-derived
/// launcher sites (`List-Unsubscribe` URL options, `List-Archive`)
/// that used to hand attacker-controlled header bytes straight to
/// `Command::new(url_launcher)` now enforce the same http/https/
/// mailto whitelist, fixed and regression-locked together with the
/// corpus in `meli/src/mail/view.rs` / `meli/src/mail/view/tests.rs`.
#[cfg(test)]
#[path = "CVE-2023-23397.rs"]
mod cve_2023_23397;

/// CVE-2024-21413 (Microsoft Outlook on Windows, CVSS 9.8, public PoC,
/// exploited in the wild) MonikerLink regression (issue #22, table 2
/// of `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution): a
/// `file://` URL whose `!`-suffix makes Outlook resolve it as a COM
/// moniker (`file://attacker.example/share/leak.html!Exploit`, with
/// `file:\\`, `search-ms:` and `mhtml:` variants), embedded as a link
/// in the mail body — clicking it bypassed Protected View and made
/// the client dereference the attacker's share over SMB/WebDAV with
/// the victim's NTLM credentials. meli has no Protected View, no COM
/// moniker resolver and no SMB/NTLM client, so the equivalent
/// surfaces — every place a mail-borne link spelling can reach the OS
/// url launcher — are mapped and locked as an immunity proof: the
/// `go_to_url` gate holds the whole moniker family (`file:`, with or
/// without the `!` suffix, `mhtml:`, `search-ms:`, application
/// monikers, UNC, every casing) behind an explicit per-URL
/// confirmation, URL-mode extraction never yields a launchable
/// moniker (the one documented nuance — an `mhtml:https://…` prefix
/// is consumed by extraction, leaving a plain https link
/// byte-identical to what any mail may carry by design — is locked
/// with its mapping rationale), the HTML mirror's scheme-bearing
/// hrefs die in `sanitize`, and the issue #21 header-derived launch
/// sites skip `mhtml:`/`search-ms:`/`file:` options on the same
/// whitelist. No gap was exposed on this corpus; no production code
/// needed changing.
#[cfg(test)]
#[path = "CVE-2024-21413.rs"]
mod cve_2024_21413;

/// CVE-2024-21378 (Microsoft Outlook on Windows, CVSS 8.8, February
/// 2024 Patch Tuesday) open-mail remote-code-execution regression
/// (issue #23, table 2 of `SECURITY-CVE-RESEARCH.zh-CN.md` — virus /
/// code execution): processing a crafted message corrupted Outlook
/// memory. meli links no MAPI store, COM runtime or scripting engine,
/// so the equivalent surface is the whole 「receive → parse →
/// display」 path as a class-level memory-safety regression:
/// [`cve_2024_21378`] drives a malformed-MIME corpus (boundary
/// delimiters that used to spin or slice out of bounds, broken
/// base64/quoted-printable/RFC 2047/charset decoding, unexpected
/// nesting combinations) through envelope parsing, tree building,
/// every decode/display conversion and the HTML pipeline with zero
/// panics — and exposes one real gap, fixed with this regression:
/// meli's open-mail display recursion over nested `message/rfc822`
/// (`EnvelopeView::attachment_to_display_helper` and
/// `ViewFilter::new_attachment`) was unbounded, and a mail a few KiB
/// long nested a few thousand levels deep stack-overflowed the 2 MiB
/// view/filter threads the moment it was opened (CWE-674); the fix
/// caps the recursion at `MAX_RFC822_DISPLAY_NESTING_DEPTH`,
/// regression-locked in `meli/src/mail/view/tests.rs` together with
/// the corpus here.
#[cfg(test)]
#[path = "CVE-2024-21378.rs"]
mod cve_2024_21378;

/// CVE-2024-30103 (Microsoft Outlook on Windows, CVSS 8.8, June 2024
/// Patch Tuesday, discovered by Morphisec) crafted-message RCE
/// regression (issue #24, table 2 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution):
/// processing a specially crafted e-mail executed code because a
/// parsing path filtered attacker input incompletely (CWE-184). meli
/// links no MAPI store, no Outlook Forms engine and no scripting
/// host, so the equivalent surface the issue prescribes is the
/// header parsing path (`melib/src/email/headers.rs` +
/// `melib/src/email/parser.rs`): overlong header values, malformed
/// parameter lists and abnormal encoding combinations must parse
/// with bounded time and memory — no panic, no whole-value expansion
/// allocation. [`cve_2024_30103`] locks that layer by layer and
/// exposes one real gap, fixed with this regression: `phrase()`
/// rescanned for the next `=?` encoded-word opener from every token,
/// so a header value of many whitespace-separated tokens closed by a
/// lone `=?` sentinel was quadratic (CWE-407) — 64 KiB stalled every
/// header's decode for ~17 s and 256 KiB for minutes, an open-mail
/// denial of service through `Subject`, custom headers, display
/// names, attachment names and IMAP envelope strings alike. The fix
/// caches the next opener position so the scan is one forward pass —
/// the width-side sibling of the CVE-2024-21378 depth caps: those
/// bound recursion depth, this bounds rescan width, and they compose
/// without overlap.
#[cfg(test)]
#[path = "CVE-2024-30103.rs"]
mod cve_2024_30103;

/// CVE-2025-21361 (Microsoft Outlook Office/LTSC/M365 apps, macOS
/// included, CVSS 7.8, January 2025 Patch Tuesday) open-mail RCE
/// regression (issue #25, table 2 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution): a
/// network attacker could execute code in the client through
/// malicious input; MSRC did not disclose the primitive. meli links
/// no MAPI store, COM runtime, forms engine or scripting host, so
/// the equivalent surface the issue prescribes is
/// `melib/src/email/parser` under a 「多形态畸形 MIME/头/附件组合」
/// corpus — and scoped past what #23 (MIME tree/boundary/encoding
/// shapes, display depth cap) and #24 (value width, token-rescan
/// linearity) already locked: the *header-value grammars* under
/// combination. [`cve_2025_21361`] drives the address grammar
/// (groups, nested comments, quoted display names, domain literals,
/// obs routes, the regex error-recovery fallback), the date grammar
/// (obs zones and years, leap seconds, encoded-word dates, the mbox
/// fallback), the message-id grammar, RFC 2231 segmented attachment
/// names, RFC 2369 list headers and RFC 6068 mailto URIs, alone and
/// stacked in kitchen-sink combination mails through the whole
/// 「receive → parse → display」 pipeline — and exposes one real gap,
/// fixed with this regression: `no_fold_literal` returned its
/// `[dtext]` span one byte short of the closing `]`, so every
/// domain-literal `Message-ID` was stored, keyed and displayed one
/// byte truncated (`<a@[127.0.0.1` — CWE-193, the incomplete-
/// filtering face of the CVE family; checked slicing kept it
/// memory-safe, but threading silently split on literal-form ids).
/// The fix returns the whole literal, regression-locked in
/// `melib/src/email/parser/tests.rs::test_email_parser_msg_id`
/// together with the corpus here. The one measured nuance, kept as
/// documentation: a nested comment pays a *linear* ~56× alternation
/// re-parse constant (every `alt` branch of the address grammar
/// re-enters `opt(cfws())` from the same prefix — 200 KB of nested
/// parens ≈ 0.7 s release), two orders below the quadratic faces
/// #23/#24 closed and with no re-scan width to cache.
#[cfg(test)]
#[path = "CVE-2025-21361.rs"]
mod cve_2025_21361;
/// CVE-2025-47176 (Microsoft Office Outlook, M365 Apps for Enterprise
/// / Office LTSC 2024, June 2025 Patch Tuesday) path-traversal RCE
/// regression (issue #26, table 2 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution):
/// `'.../...//' in Microsoft Office Outlook allows an authorized
/// attacker to execute code locally` (CWE-35/CWE-22, CVSS 3.1 7.8).
/// meli links no MAPI store and no Windows path normalization, so the
/// equivalent surfaces are the issue-prescribed 「open mail parses
/// it」 class (`melib/src/email/parser.rs` +
/// `melib/src/email/attachments.rs`: malformed structure and extreme
/// sizes must parse without panics, bounded, with deterministic
/// errors) plus the advisory's own face: every path a mail-controlled
/// identifier (attachment `filename`/`name`, raw `Message-ID`) can
/// reach the filesystem. [`cve_2025_47176`] locks both, and exposes
/// two real gaps, fixed with this regression: the whole-mail export
/// sites (`save-attachment 0 <dir>` and `export-thread`) pushed the
/// raw `Message-ID` into the export directory with no sanitization,
/// so a mail with `Message-ID: ../evil` wrote outside the directory
/// the user chose — and `sanitize_separator` stripped only the
/// host's native separator, leaving the other spelling alive on the
/// other platform family. The exports now run the header bytes
/// through `sanitize_filename` (regression-locked in
/// `meli/src/mail/view/tests.rs`), and the separator sanitizer
/// strips both separators everywhere.
#[cfg(test)]
#[path = "CVE-2025-47176.rs"]
mod cve_2025_47176;

/// CVE-2026-70329 (Microsoft Outlook, Office 2019/2021/M365, CVSS
/// 8.8) integer-overflow RCE regression (issue #27, table 2 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution):
/// integer wraparound on an attacker-declared size delivered over
/// the network vector corrupted Outlook memory and executed code
/// (CWE-190). meli links no MAPI store and none of Outlook's
/// Windows protocol machinery, so the equivalent surfaces the issue
/// prescribes are every place meli computes a length from a
/// *declared* size — IMAP literal declarations (`{4294967295}`) in
/// `melib/src/imap/protocol_parser.rs`, the mbox `Content-Length`
/// header in `melib/src/mbox/mod.rs`, and multipart boundary offset
/// arithmetic in `melib/src/email/parser.rs` — and the invariant is
/// checked semantics throughout: no wraparound, oversized
/// declarations rejected or clamped, never trusted for a slice, a
/// skip or an allocation. [`cve_2026_70329`] locks that layer by
/// layer and exposes four real bugs across the declared-size
/// surfaces, all fixed with this regression: the IMAP line splitter
/// skipped literal continuation lines with unchecked `i += pos + 2
/// + len` on the server-declared length — a
/// `{18446744073709551615}` declaration overflowed it (CWE-190),
/// panicking debug builds of every IMAP session and wrapping
/// release builds past the buffer; the fix saturates the skip. The
/// `mboxcl`/`mboxcl2` reader cut each message with
/// `&input[..headers_end + bytes]` on the header-declared length —
/// a near-`usize::MAX` declaration overflowed the addition and a
/// merely-larger-than-the-file declaration indexed out of bounds
/// directly (CWE-190/CWE-125): one crafted line crashed the reader;
/// the fix computes the message end with `checked_add` and clamps
/// it to what is actually present. And the same reader's
/// `find_content_length` never worked at all: it handed
/// `header_value` the slice starting at the field name — so the
/// parsed "value" was the whole header line and every
/// `usize::from_str` failed — and `headers_end` stripped the line
/// terminator off a final `Content-Length` header, the position
/// the built-in mboxcl2 writer always writes it to; both fixed, so
/// the honest writer→reader round-trip passes for the first time.
/// The IMAP literal parser itself and the boundary scanner's
/// guarded bookkeeping are proven immune on the same corpus.
#[cfg(test)]
#[path = "CVE-2026-70329.rs"]
mod cve_2026_70329;

/// CVE-2006-1305 (Microsoft Outlook 2000/2002/2003, no CVSS score
/// assigned) parsing-DoS regression (issue #30, table 2 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution family,
/// DoS face): a remote attacker's malformed headers — an oversized
/// `Subject` and a mass of `To`/`Cc` recipients — exhausted Outlook's
/// memory and interrupted e-mail recovery (CWE-400). meli links none
/// of Outlook's PST/MAPI machinery, so the equivalent surface is the
/// issue-prescribed header path (`melib/src/email/headers.rs`, the
/// `headers` module of `melib/src/email/parser.rs` and
/// `Envelope::populate_headers`), and the invariant reads in meli's
/// memory model as *no amplification*: the mail is already in memory
/// when parsing starts, so a header can only exhaust memory by
/// expanding past its encoded form, re-scanning earlier bytes per
/// token, or compounding per-header work. [`cve_2006_1305`] locks
/// that layer by layer — MB-level subjects (plain, folded,
/// invalid-UTF-8 lossy, 32 768 valid encoded words, 1 MiB of
/// unterminated `=?charset?encoding?` prefixes), ten-thousand-strong
/// recipient storms (interleaved headers, one 10 000-address list,
/// one 10 000-member group), a hundred-thousand custom-header storm,
/// and the combined mail with its recovery pass (`Draft::new_reply`,
/// the meli face of Outlook's interrupted e-mail recovery) — and the
/// result is **immune, no gap found**: every layer parses linear in
/// time and memory, the malformed-prefix subject degrades to a
/// dropped subject instead of a stall, and the honest carrier proves
/// the budgets come from linearity, not truncation. The corpus
/// composes with #24 (CVE-2024-30103): the malformed-prefix subject
/// re-locks that issue's `phrase()` rescan-cache fix at this CVE's
/// own MB scale.
#[cfg(test)]
#[path = "CVE-2006-1305.rs"]
mod cve_2006_1305;

/// CVE-2020-9818 (Apple iOS Mail, iOS 12–13.4.1, no CVSS assigned)
/// zero-click out-of-bounds write regression (issue #31, table 2 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution): a
/// malformed e-mail corrupted iOS Mail's memory the moment the
/// background fetch received it — no tap, no open (ZecOps; exploited
/// in the wild since at least 2018 per their report, unconfirmed by
/// Apple). meli links no MFMessage framework and no Objective-C heap,
/// so the equivalent surface is the issue-prescribed zero-interaction
/// 「receive → parse」 path: IMAP FETCH responses
/// (`melib/src/imap/protocol_parser.rs`) plus the mail bytes they
/// carry (`melib/src/email/parser.rs`). [`cve_2020_9818`] locks that
/// layer by layer and exposes one real gap, fixed with this
/// regression: the `FLAGS`/`MODSEQ` branches advanced the scan cursor
/// with a `+ 1` that skips the list-closing `)` — on a response
/// truncated right after the flags list or the MODSEQ digits there
/// was no `)` left, the cursor overshot `input.len()` and the
/// `raw_fetch_value` slice panicked with an out-of-range end index
/// (CWE-125, the contained face of this CVE's write class): five
/// one-line server bytes crashed every IMAP account's unsolicited
/// fetch. The cursor now clamps at the buffer end, and the `flags()`
/// token scan terminates at CR/LF (an IMAP atom never contains them)
/// so a line-cut `\Seen` degrades to the real flag. The corpus also
/// re-locks the neighboring walls at the IMAP framing: malformed
/// frames (truncated literals, bare prefixes, past-`usize` digit
/// runs, truncated ENVELOPE, 500-deep BODYSTRUCTURE) reject or
/// saturate cleanly, the malformed-MIME mail corpus (truncated,
/// nested, illegal UTF-8, boundary bytes, NUL, bare CR/LF, broken
/// base64, kitchen sink, 5000-level nesting) parses zero-click with
/// no panic through envelope and MIME tree, and hostile
/// illegal-UTF-8 ENVELOPE literals flow through the tolerance layers
/// to a lossy-decoded envelope.
#[cfg(test)]
#[path = "CVE-2020-9818.rs"]
mod cve_2020_9818;

/// CVE-2006-2386 (Microsoft Outlook Express ≤ 6, CVSS 2.0 6.8,
/// MS06-076) address-book contact-record RCE regression (issue #29,
/// table 2 of `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code
/// execution): a crafted contact record in a mail-distributed Windows
/// Address Book (WAB) file executed code in Outlook Express'
/// address book parser. meli links no WAB engine, so the equivalent
/// surfaces the issue prescribes are meli's address book as a class:
/// the vobject vCard parser (`melib/src/utils/vobject`), the
/// production contacts load path (`vcard_folder` → `load_cards` →
/// `CardDeserializer` → `Card`) and the mail-carriage layer that
/// keeps `.vcf`/`.wab` attachments inert. [`cve_2006_2386`] locks
/// malformed contact entries — overlong fields, deep structures,
/// binary garbage — panic-free, bounded and deterministically
/// rejected on all three, and exposes two real gaps fixed with this
/// regression: one binary-garbage `.vcf` in the `vcard_folder` used
/// to abort the whole address book load and blank every contact
/// (CWE-754, now quarantined per file), and multi-card `.vcf` streams
/// loaded only their first contact (RFC 6350 streams, now complete).
#[cfg(test)]
#[path = "CVE-2006-2386.rs"]
mod cve_2006_2386;

/// CVE-2024-43604 (Microsoft Outlook for Android, CVSS 5.7, October
/// 2024 Patch Tuesday) local privilege-escalation regression (issue
/// #28, table 2 of `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code
/// execution): the Android attachment-path family — mail-controlled
/// attachment filenames carrying path elements out of the directory
/// the app assumed was fixed, planting files with the app's
/// privileges. meli runs on no Android sandbox, so the equivalent
/// surface — every place mail-controlled bytes become an on-disk path
/// component — is mapped and locked: the single/batch attachment
/// saves and the mbox export defaults sanitized only `/` away
/// (control characters, backslashes and `.`/`..` survived), and the
/// `.eml` export names derived from the attacker-authored
/// `Message-ID` reached `PathBuf::push` with **no sanitization at
/// all** — `save-attachment 0 <dir>` on `Message-ID:
/// <../../evil>` wrote outside the destination and an absolute
/// `Message-ID` replaced it outright, an arbitrary-file plant with
/// the user's privileges (the direct equivalent of the CVE's
/// escalation); meli's own forward-as-attachment was even capable of
/// *mailing out* such filenames. [`cve_2024_43604`] carries the
/// payload corpus (relative chains, absolute paths, RFC 2047-armored
/// traversals decoded by `Attachment::filename` before any sink sees
/// them, backslash chains, ANSI control names, the bare `..`) and
/// locks the walls: every sink now routes through
/// `sanitize_filename_component`/`eml_filename` (flat names,
/// generated fallbacks), the mailcap `%s` temp landing stays
/// sanitized under the meli temp root with degenerate hints guarded,
/// and every write stays `create_new` + `0o600` — regression-locked
/// end to end in `meli/src/mail/view/tests.rs`,
/// `meli/src/mailcap.rs` and `meli/src/types/helpers.rs`.
#[cfg(test)]
#[path = "CVE-2024-43604.rs"]
mod cve_2024_43604;

/// CVE-2006-6505 (Mozilla Thunderbird ≤ 1.5.0.8 / SeaMonkey ≤ 1.0.6,
/// MFSA 2006-74, Critical) mail-header heap-overflow RCE regression
/// (issue #34, table 2 of `SECURITY-CVE-RESEARCH.zh-CN.md` —
/// virus / code execution): processing an external message body
/// whose overlong `Content-Type` header and overlong RFC 2047
/// encoded-word headers overflowed a fixed heap buffer and ran
/// attacker code. meli's RFC 2047 decoder is safe Rust, so
/// [`cve_2006_6505`] locks the equivalent walls the issue
/// prescribes on `melib/src/email/parser`: oversized encoded words
/// (100 KiB–1 MiB B/Q words, charset-converting words, 20k-word
/// chains) decode to output bounded by a small multiple of input in
/// linear time (the CVE-2024-30103 `phrase()` linearity rework
/// re-locked), malformed words (truncated, oversized charset tag,
/// bad base64, bogus transfer encoding) degrade deterministically
/// without panicking, the oversized `Content-Type` carrier parses
/// with every parameter byte-identical (no cross-parameter
/// corruption — the overflow's observable face), and the
/// external-body URL stays inert text: the part is an opaque
/// `Other` leaf whose `decode()` yields only the local body bytes —
/// meli has no external-body fetcher (its only HTTP client is the
/// user-configured JMAP backend; patch retrieval is an explicit
/// user command). An immunity proof: no meli/melib gap was found
/// on this surface.
#[cfg(test)]
#[path = "CVE-2006-6505.rs"]
mod cve_2006_6505;

/// CVE-2020-9819 (iOS Mail, iOS 12–13.4.1) heap-corruption / DoS
/// regression (issue #32, table 2 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution family,
/// DoS face): processing a malicious mail corrupted the heap — the
/// zero-click sibling of CVE-2020-9818 (#31) on the memory-exhaustion
/// face, where a tiny message with giant *declared* sizes exhausts the
/// heap of a client that trusts declarations over arrived bytes. meli
/// links no proprietary mail engine, so the equivalent surface is the
/// issue-prescribed allocation strategy of `melib/src/email/parser.rs`
/// plus the receive paths, and the invariant — every allocation rides
/// bytes that actually arrived, declaration/content mismatch fails
/// closed — reads in meli's zero-copy nom model as immune by
/// construction. [`cve_2020_9819`] locks that layer by layer with the
/// prescribed deceptive-declaration corpus — MIME parts declaring
/// `usize::MAX`/TiB `Content-Length` and `message/external-body`
/// `size=` values over tiny actual bodies (footprint is a function of
/// actual bytes only; same-digit-count declarations `10^19` apart cost
/// byte-identically), a twenty-thousand-header in-part declaration
/// storm and an 8192-minimal-part explosion with the zero-object
/// empty delimiter mass (object count and retained bytes ride the
/// actual delimiters), the IMAP zero-click FETCH batch failing
/// closed on every literal declaration/content mismatch, and the
/// `Connection::MAX_SERVER_RESPONSE_SIZE`/`IO_BUF_SIZE` heap backstop
/// staying finite — and the result is **immune, no gap found**: no
/// production code needed changing. The corpus is disjoint from #27
/// (declared-size *arithmetic*), #30 (header *width* amplification
/// with real MB content) and #31 (malformed-bytes *panic/OOB* safety).
#[cfg(test)]
#[path = "CVE-2020-9819.rs"]
mod cve_2020_9819;

/// CVE-2008-0039 (Apple Mail, Mac OS X 10.4.11 / Server 10.4.11,
/// CVSS v2 6.8) file-URL application-launch RCE regression (issue
/// #33, table 2 of `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code
/// execution): APPLE-SA-2008-02-11 — 「An implementation issue exists
/// in Mail's handling of file:// URLs, which may allow arbitrary
/// applications to be launched without warning when a user clicks a
/// URL in a message」, fixed upstream by revealing the location in
/// Finder instead of launching (CWE-94). meli links no Launch
/// Services, Finder integration or AppleScript bridge, so the issue's
/// prescribed equivalent surface — the `meli/src/mail/view.rs` link
/// scheme whitelist, every place a mail-borne URL spelling can reach
/// an OS url launcher — is mapped and locked as an immunity proof on
/// the whole local-executable family (`.app` bundles, `.command`,
/// `.scpt`, `.sh`, directory, query, fragment, percent-encoded, any
/// casing, `file://localhost/…` and `file:///…`): [`cve_2008_0039`]
/// proves the `go_to_url` gate refuses every `file:` spelling behind
/// an explicit per-URL confirmation (on top of the deliberate
/// URL-mode keystroke sequence a terminal client already requires —
/// stronger than Apple's own fix), URL-mode extraction never yields a
/// launchable `file:` URL (the single-slash and backslash spellings
/// are not extracted at all — the issue's 「仅以纯文本显示，不可打开」),
/// the HTML mirror's `file:` hrefs die in `sanitize` casing and
/// percent-encoding included, the header-derived launch sites
/// (`List-Unsubscribe`/`List-Archive`) skip `file:` targets, and an
/// honest carrier keeps its `https`/`mailto` links one-keystroke
/// usable everywhere the family is refused. No gap was exposed on
/// this corpus; no production code needed changing.
#[cfg(test)]
#[path = "CVE-2008-0039.rs"]
mod cve_2008_0039;

/// CVE-2026-84641 (Thunderbird < 155, ESR < 140.15, 153.x < 153.2,
/// MFSA 2026-86/87/88, CVSS 7.5) IMAP `ID` response use-after-free →
/// heap-leak regression (issue #36, table 2 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution): a
/// malicious IMAP server answered the `ID` command (RFC 2971) with
/// a crafted parameter list that triggered a use-after-free in
/// Thunderbird's C++ parser and leaked heap memory — content that
/// could then be persisted into prefs.js. [`cve_2026_84641`] locks
/// meli's equivalent surface (`melib/src/imap/protocol_parser/
/// id_ext.rs` plus the response-assembly, log and `server_id`
/// persistence sites) with the issue-prescribed hostile corpus
/// (overlong keys/values, illegal UTF-8, unclosed parens, binary
/// garbage) and proves the memory-safety class immune — safe
/// checked Rust, every shape a deterministic typed error, illegal
/// UTF-8 rendered as a fixed placeholder, and the `prefs.js`-face
/// persistence (`serde_json::to_value`) holding only
/// `String::from_utf8`-validated ≤ 1024-octet text from a
/// zero-initialized read buffer — while exposing one real gap,
/// fixed with this regression: RFC 2971's `#(string SPACE nstring)`
/// allows *zero* pairs, but the parser demanded one, answering the
/// legal 「no information」 `* ID ()` form with a parse error and a
/// spurious "Consider turning ID use off" warning on every connect;
/// it now answers `Ok(None)`, regression-locked in
/// `melib/src/imap/protocol_parser/id_ext.rs` together with the
/// corpus here.
#[cfg(test)]
#[path = "CVE-2026-84641.rs"]
mod cve_2026_84641;

/// CVE-2026-14899 (Thunderbird < 153, ESR < 140.13; MFSA / OpenCVE,
/// CVSS 7.5) MIME-header off-by-one regression (issue #35, table 2
/// of `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution):
/// forwarding a mail with 「显示全部头」 enabled hit an off-by-one
/// in MIME header parsing that read one byte past the end of the
/// buffer, potentially crashing the client. meli links no
/// Gecko/XPCOM MIME parser, so the equivalent surface the issue
/// prescribes is the header parsing boundary itself —
/// `melib/src/email/headers` plus the `headers` module of
/// `melib/src/email/parser.rs` and their
/// 「receive → parse → display → forward」 consumers — driven by the
/// corpus family of header **name**, **value** and
/// **folding-whitespace** lengths at parsing-boundary values ±1
/// byte. [`cve_2026_14899`] locks that layer by layer — the corpus
/// parses panic-free with byte-precise slices, the exact
/// `(name, value, rest)` bytes of every canonical boundary shape
/// are asserted, the CVE's trigger (header iteration with all
/// headers shown + the forward/reply composers) is mapped immune —
/// and exposes one real gap, fixed with this regression:
/// `HeaderName::from_bytes` accepted the **empty** byte slice and
/// returned an empty header name, the length-0 boundary one below
/// the grammar's `field-name = 1*ftext` minimum of 1 (this CVE's
/// own ±1 class): an empty name could become a `HeaderMap` key and
/// a `": value"` malformed line through the composer's
/// `Draft::set_header`. The fix rejects the empty input at
/// `from_bytes` (every `TryFrom` spelling and serde route through
/// it), regression-locked in
/// `melib/src/email/headers/tests.rs::
/// test_email_headers_names_empty_name_is_invalid` together with
/// the corpus here.
#[cfg(test)]
#[path = "CVE-2026-14899.rs"]
mod cve_2026_14899;

/// CVE-2026-84640 (Thunderbird; MFSA 2026-86/87/88, same advisory
/// batch as CVE-2026-84641) mail-header one-byte out-of-bounds
/// read regression (issue #38, table 2 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution): a
/// maliciously constructed mail header made Thunderbird's parser
/// read one byte past the end of its header buffer. meli links no
/// Gecko/XPCOM header parser, so the equivalent surface the issue
/// prescribes is the header parsing boundary itself —
/// `melib/src/email/headers` plus the `headers` module of
/// `melib/src/email/parser.rs` and their 「receive → parse →
/// display」 consumers — driven by the issue's prescribed corpus
/// family: header byte sequences whose length sits exactly on a
/// parsing boundary — the empty value, the single-byte value, the
/// no-colon line and the input without a trailing newline.
/// [`cve_2026_84640`] locks the class layer by layer — no-colon
/// lines reject at the `field-name` scan guards, empty and
/// single-byte values cut exact byte slices with their one-past
/// neighbours rejecting, the exhaustive truncation sweep (every
/// prefix of canonical LF/CRLF/multipart mails) proves no parser
/// position consults the byte one past the buffer end, and the
/// receive → parse → display pipeline over a mail whose every
/// header sits on a boundary stays inert and byte-faithful. On
/// this corpus meli holds by construction — checked slicing leaves
/// the one-past read nowhere to land — so the class is locked
/// immune without needing a `melib` fix.
#[cfg(test)]
#[path = "CVE-2026-84640.rs"]
mod cve_2026_84640;

/// CVE-2026-84639 (Thunderbird, fixed in the MFSA 2026-86/87/88
/// batch, no CVSS score assigned) uninitialized-memory-use
/// regression (issue #37, table 2 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution): a
/// specific MIME body triggered an error path in Thunderbird's MIME
/// processing that used uninitialized memory (CWE-457), surfacing
/// never-written buffer contents. meli's whole e-mail decode path is
/// safe Rust (no `MaybeUninit`, no `set_len`), so the literal site
/// cannot exist — [`cve_2026_84639`] locks the issue-prescribed
/// equivalent surface, the MIME error paths of
/// `melib/src/email/parser.rs` + `Attachment::decode`, with the
/// prescribed corpus (truncated base64, bad quoted-printable,
/// illegal byte sequences): every error path echoes its input
/// verbatim or applies a fixed decode table / the fixed U+FFFD
/// replacement sequence — byte-exact snapshots, identical across two
/// independent parses and two decodes each, bounded by the proven
/// expansion factor; the error messages are exact static strings;
/// and a kitchen-sink mail combining every family is deterministic
/// end to end through envelope, MIME tree, decode, text extraction,
/// `Display`/`Debug` and every `ViewOptions` conversion mode — with
/// an honest carrier proving the decoder does its real work. The
/// result is **immune, no gap found**: no production code needed
/// changing.
#[cfg(test)]
#[path = "CVE-2026-84639.rs"]
mod cve_2026_84639;

/// CVE-2001-0473 (mutt < 1.2.5, no CVSS score assigned) IMAP
/// format-string remote-code-execution regression (issue #40, table
/// 2 of `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution):
/// a malicious IMAP server answered commands with response text
/// carrying printf format-string metacharacters, and mutt passed
/// that text to a printf-style error reporter *as the format
/// string* — `%s`/`%x` walked the stack, `%n` wrote attacker-chosen
/// pointers into it, and arbitrary commands executed (CWE-134).
/// [`cve_2001_0473`] maps the attack onto meli's
/// issue-prescribed equivalent surface — `melib/src/imap` response
/// text flowing into `melib/src/error` values, log records and
/// display — and locks it layer by layer as an immunity proof:
/// response text parses into `ResponseCode::Alert` byte-preserved
/// (metacharacter inventory constant, `%%` never collapsed, width
/// bombs never expanded, NUL/illegal-UTF-8 degraded only by the
/// fixed lossy decoder, Dovecot timing-suffix strips keep the
/// payload), and every downstream sink — the
/// `From<ImapResponse>` error embedding, the `BackendEvent::Notice`
/// and state.rs status format, the `imap_log!`/`tracing` log faces,
/// the BYE 「Offline」 face and the `Error` display chain — carries
/// the payload as a *value* argument of a compile-time literal
/// template; display is proven idempotent (no hop secondarily
/// formats), the `{`-family rides the framing as IMAP literal
/// *declarations* (lengths, never formats) that degrade to the
/// payload verbatim, untagged injections stay raw bytes, and the
/// whole corpus flows panic-free and deterministically. In Rust the
/// mutt primitive is inexpressible — no varargs, and
/// `format!`/`write!`/`tracing` accept no runtime format template —
/// so no gap exists to fix: no production code needed changing.
#[cfg(test)]
#[path = "CVE-2001-0473.rs"]
mod cve_2001_0473;

/// CVE-2014-9116 (mutt 1.5.23, CVSS v2 5.0) header-processing
/// heap-overflow regression (issue #41, table 2 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution): mutt's
/// `write_one_header` mishandled newline characters at the beginning
/// of a header, and 「a header with an empty body」 — the verbatim
/// trigger is an mbox mail whose header section is `From:\n` followed
/// by a bare-CR line — made `mutt_substrdup` receive a `begin` one
/// past its `end` at write-out time; the wrapped `-1` became
/// `SIZE_MAX`, and `memcpy` into the zero-size block overflowed the
/// heap (crash/DoS). [`cve_2014_9116`] maps the attack onto meli's
/// issue-prescribed equivalent surface — the 头区/正文边界 handling of
/// `melib/src/email/parser` — drives it with the trigger families
/// (the openwall PoC, 「空头」 empty-header-section mails in every
/// separator spelling, empty-body headers, bare-CR lines, separator
/// truncations) and locks the walls layer by layer, exposing two real
/// gaps, both fixed with this regression in
/// `melib/src/email/parser.rs`: `parser::mail`'s `many1` header list
/// rejected RFC 5322-valid 「空头」 mails outright — and since the
/// maildir receive path silently skips envelopes that fail
/// `Envelope::from_bytes`, such mails were invisible (this CVE's
/// availability face, one crash-proofed refusal away from mutt's
/// 「开头换行符处理不当」); and `headers_raw` — the `mutt_substrdup`
/// analogue — mis-cut both separator spellings: the CRLF form
/// truncated the last header line's terminator so `HeaderIterator`
/// dropped the header (an identical CRLF multipart mail lost its
/// `has_attachments` listing indicator), both spellings leaked the
/// empty line's bytes into the body, and the mixed spellings did not
/// split at all. The exact PoC mail itself fails closed — meli's
/// strict field-name grammar rejects its colon-less bare-CR line (the
/// CVE-2026-84640-locked class), so no header list and no write-out
/// exist to crash on — while every parseable corpus mail proves inert
/// through receive, display list, reply composer and mbox export.
#[cfg(test)]
#[path = "CVE-2014-9116.rs"]
mod cve_2014_9116;

/// CVE-1999-0940 (mutt, early versions, no CVSS assigned) MIME-parsing
/// buffer-overflow regression (issue #39, table 2 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution): a
/// malformed MIME mail overflowed a fixed buffer in mutt's MIME parser
/// and executed attacker commands — one of the earliest e-mail-client
/// overflow CVEs. meli's parser is checked Rust with no fixed-size
/// buffers, so [`cve_1999_0940`] locks the issue-prescribed corpus
/// family (boundary 缺失/重复, 部分头缺失, 坏 base64 边界, plus oversized
/// MIME tokens as the anti-mutt layer) as deterministic degradation
/// and bounded, byte-faithful parsing — and exposes two real gaps
/// behind the canonical RFC 5322 `CRLF` line ending, fixed with this
/// regression: the multipart twin scanners (`multipart_parts`/
/// `parts_f`) treated `\r\n` and `\n` delimiter terminators
/// asymmetrically, so `Envelope::has_attachments` (the 📎 listing
/// indicator, fed by `check_if_has_attachments_quick`) missed every
/// attachment living outside the first part of a CRLF mail and a
/// CRLF mail ending at its last non-closing delimiter lost all
/// already-scanned parts; and `headers_raw` returned the header
/// block of a CRLF part ending in a bare `\r` no `header_value()`
/// can terminate, silently dropping the part's last header — in
/// practice the `Content-Disposition` of the attachment part. Both
/// fixed in `melib/src/email/parser.rs`, regression-locked in
/// `melib/src/email/parser/tests.rs` together with the corpus here.
#[cfg(test)]
#[path = "CVE-1999-0940.rs"]
mod cve_1999_0940;

/// CVE-2023-4874 (mutt > 1.5.2, < 2.2.12, CVSS 4.3) NULL-pointer
/// dereference DoS regression (issue #43, table 2 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution): mutt
/// crashed when *viewing* a specially crafted mail whose structural
/// fields are missing (the drafting twin of the same advisory is
/// CVE-2023-4875). meli's memory model has no NULL to dereference —
/// a missing `From` is an empty `SmallVec`, a missing `Message-ID`
/// an allocated-but-empty `MessageID`, a missing `Subject` an empty
/// `Cow`, a missing `Date` the epoch timestamp — so
/// [`cve_2023_4874`] locks the issue-prescribed corpus family (无
/// Message-ID、无 From、空 body、无 Content-Type, down to the
/// zero-byte mail and the void-value `From: `/`Message-ID: `
/// spellings) as panic-free 「receive → parse → display」 with every
/// gap degrading to its documented default (an absent or void
/// Message-ID is synthesized from the envelope hash — the allocated
/// identifier mutt's `NULL` pointer never had; the absent
/// Content-Type degrades to the RFC 2045 `text/plain`). The full `EnvelopeView` rendering
/// (constructor display build, `draw` at real and degenerate
/// terminal sizes, sticky-header walk) is locked in
/// `meli/src/mail/view/tests.rs`
/// (`missing_structural_fields_mail_renders_degraded_in_envelope_view`)
/// — the view constructors need a `MainLoopHandler` from a mock
/// context, which this crate deliberately does not add as a
/// test-only dependency (same split as CVE-2024-21378).
#[cfg(test)]
#[path = "CVE-2023-4874.rs"]
mod cve_2023_4874;

/// CVE-2002-0833 (Eudora 5.1.1 / 5.0-J, Windows; no CVSS score
/// assigned) MIME boundary buffer-overflow regression (issue #45,
/// table 2 of `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code
/// execution): a multipart mail whose overlong `boundary=` parameter
/// overflowed Eudora's fixed-size MIME buffer and executed attacker
/// code. meli has no fixed-size buffer anywhere on the boundary path,
/// so [`cve_2002_0833`] locks the issue-prescribed corpus family —
/// overlong boundaries (>10 KB up to 1 MiB), quoted-string parameter
/// forms, control bytes inside the quoted boundary and
/// linear-whitespace runs — as bounded, byte-faithful, panic-free
/// parsing, and exposes two real gaps behind the 超长空白 face, both
/// fixed with this regression: RFC 2046 §5.1.1 allows a boundary
/// delimiter line to carry optional linear whitespace before its line
/// ending (`"--" boundary *LWSP CRLF`), but both twin scanners treated
/// a `--BOUND   <CRLF>` delimiter as a false boundary occurrence — a
/// whitespace-carrying first delimiter skipped the whole first part
/// and a mid delimiter ended the scan, silently dropping every part
/// after it (the 📎 listing indicator with it) on a legitimate,
/// RFC-valid mail; and the boundary *parameter value* grammar
/// (`bcharsnospace` ending) says trailing SP/HTAB is line junk, not
/// boundary bytes, but all three extraction sites kept the verbatim
/// value, so `boundary=BOUND   ` with honest `--BOUND` delimiters
/// parsed to zero parts. Fixed in `melib/src/email/parser.rs`
/// (`skip_lwsp` delimiter tolerance in both twin loops) and
/// `parser::attachments::multipart_boundary` (the shared normalized
/// extraction), regression-locked in
/// `melib/src/email/parser/tests.rs` together with the corpus here.
#[cfg(test)]
#[path = "CVE-2002-0833.rs"]
mod cve_2002_0833;

/// CVE-2022-1328 (mutt ≥ 0.94.13, < 2.2.3, CVSS 4.3, Tavis Ormandy
/// / Google Project Zero) uudecode out-of-bounds-read regression
/// (issue #42, table 2 of `SECURITY-CVE-RESEARCH.zh-CN.md` —
/// virus / code execution): a mail carried a uuencoded attachment
/// (`begin 644 name` … `end`) whose data line declared more decoded
/// bytes than the physical line held, and mutt's uudecode kept
/// consuming characters per the declaration — past the end of the
/// line into adjacent memory (CWE-125), surfacing the over-read
/// bytes in the decoded attachment. [`cve_2022_1328`] maps the
/// attack onto meli's issue-prescribed equivalent surface
/// (`melib/src/email/attachments.rs`: attachment type recognition
/// and decoding) and locks it layer by layer as an immunity proof:
/// the corpus is a genuine uuencode of a secret payload (verified by
/// an embedded bounded reference uudecode carrying the mutt 2.2.3
/// semantics — the declaration never licenses reading past the
/// actual span), every issue-prescribed malformation family
/// (「行尾无换行 / 截断 / 超长行 / 非法字符」) is exactly a
/// declared-vs-actual mismatch the bounded decoder refuses, and meli
/// never auto-uudecodes anything: the trigger header maps to
/// `ContentTransferEncoding::Other`'s verbatim passthrough of the
/// bytes that actually arrived, inline uuencoded text renders as
/// plain text through the ordinary `InlineText` branch, the secret
/// never surfaces in any decode/text/display surface, an exhaustive
/// truncation sweep proves decode output is exactly the actual bytes
/// at every cut — never longer, never past the (missing) line end —
/// the `begin` line's traversal name stays text and never migrates
/// into filename metadata, and the kitchen-sink mail (double
/// base64-wrapped uuencode included) parses deterministic and
/// panic-free end to end. meli has no uuencode decoder at all, so
/// no gap exists to fix: no production code needed changing.
#[cfg(test)]
#[path = "CVE-2022-1328.rs"]
mod cve_2022_1328;

/// CVE-2023-4875 (mutt >1.5.2, <2.2.12, CVSS v2 4.3) NULL-dereference
/// draft-composition regression (issue #44, table 2 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution): mutt
/// stored a draft's user headers after checking only that each field
/// "contains at least a colon", then handed the *entire* field to its
/// rfc2047 decoder and `safe_strdup`'d the result; the decoder's
/// base64 path skipped illegal characters instead of aborting, so
/// `=?utf-8?B?####?=` decoded to an empty string, the empty strdup
/// landed as a NULL `userhdrs` entry, and the compose write-out
/// crashed in `strchr(NULL, ':')` — composing from a specially
/// crafted draft message. [`cve_2023_4875`] maps the attack onto the
/// issue-prescribed 「从草稿继续撰写」 surface — draft load
/// (`Draft::from_str`), editor reload (`Draft::update`), draft re-open
/// (`Draft::edit`) and re-serialisation (`to_edit_string`,
/// `finalise`) — drives the mutt trigger family (RFC 2047 words of
/// illegal, empty and mixed base64 payloads, illegal Q-escapes,
/// unknown charsets, folds, the words planted in the standard
/// headers) plus the 坏头/坏 MIME 结构/缺失必填字段 families and every
/// byte-truncation of the canonical trigger draft, and locks the
/// walls layer by layer: the illegal-base64 decode *aborts* in meli's
/// `data_encoding` alphabet check (mutt's root fix) with the payload
/// kept as literal text; the compose path never decodes custom header
/// values at all, so the trigger words survive load → editor
/// round-trip → `finalise` byte-identical and the emptied-to-NULL
/// producer has no code to run in; malformed headers fail closed as
/// deterministic `ValueError`s; hostile MIME (150-level multiparts
/// past the 100-level cap, unclosed/empty/missing boundaries,
/// 500-level `message/rfc822` nests, garbage base64/QP bodies) stays
/// bounded; missing required fields default instead of crashing. One
/// real gap is exposed and fixed with this regression in
/// `melib/src/email/compose.rs`: `Draft::edit` swallowed any
/// `Envelope::headers` failure with `unwrap_or_else(|_| Vec::new())`
/// — one hostile non-UTF-8 header value silently wiped **every**
/// header (To/Subject/From gone) when a stored draft was resumed, the
/// silent data-loss neighbour of this CVE's availability face; it now
/// parses the header block with the exact grammar and verdicts of
/// `Draft::from_str` (`parser::mail` plus per-value UTF-8 conversion),
/// so the re-open entry reports the unparseable field as a
/// deterministic `ValueError` the composer surfaces as its "Failed to
/// open e-mail" notification, while RFC 5322-valid zero-header 「空头」
/// drafts keep loading with the default fields.
#[cfg(test)]
#[path = "CVE-2023-4875.rs"]
mod cve_2023_4875;

/// CVE-2003-0376 (Eudora 5.2.1, CVSS v2 5.0) "Attachment Converted"
/// buffer-overflow regression (issue #47, table 2 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution): Eudora
/// parsed the `Attachment Converted` line of a received mail and
/// copied its dot-piled quoted argument (`\B.A.A.A … .A.A.A`, 122
/// repetitions of `.A`) into a fixed-size buffer — crash with
/// `ACCESS_VIOLATION`, failed restart until the message was removed,
/// possibly controllable code execution. [`cve_2003_0376`] maps the
/// attack onto meli's equivalent surfaces — the marker line, the
/// MIME attachment filename, the mailcap `%s` temp landing and the
/// save-component sinks — over the verbatim bugtraq trigger shapes
/// (canonical `a…………exe`, `\B.A.A…` below/at/above the 122-repetition
/// crash threshold, all-dot and RFC 2047-armored piles, megabyte
/// names) and locks the walls layer by layer: the marker line is
/// inert body text (and its header spellings fail closed), dot piles
/// parse/surface/display verbatim and bounded, executable content
/// never auto-materializes (temp root untouched by reading) nor lands
/// with an execute bit, and opening stays gated on the explicit
/// `open_mailcap` shortcut. One real gap is exposed and fixed with
/// this regression in `meli/src/types/helpers.rs`: `File::
/// create_temp_file` left its sanitized name hint arbitrarily long,
/// so a name past `NAME_MAX` fell into the per-grapheme
/// `ENAMETOOLONG` retry loop — O(len²) work (measured: ≈109 s for a
/// 100 000-byte name in a release build), one hostile mail freezing
/// the client for minutes on the first mailcap open, the availability
/// face of this CVE; every mail-controlled name sink now
/// pre-truncates to a bounded component on a UTF-8 character boundary
/// while the per-grapheme fallback stays for exotic filesystems.
#[cfg(test)]
#[path = "CVE-2003-0376.rs"]
mod cve_2003_0376;
