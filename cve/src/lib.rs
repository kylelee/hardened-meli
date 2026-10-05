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

/// CVE-2018-0950 (Microsoft Office Word / RTF OLE preview on Windows,
/// CVSS 6.5) remote-content information-disclosure regression (issue
/// #101, table 3 of `SECURITY-CVE-RESEARCH.zh-CN.md` — web/HTML
/// embedding): opening or previewing a mail whose RTF attachment
/// embeds an OLE preview object
/// (`\object\objemb{\*\objclass Word.Document.8}{\*\objdata …}`)
/// pointed at `\\attacker.example\share` made Office render remote
/// content and leak information (NTLM credentials / host info) with no
/// click on the object. meli embeds no RTF/OLE previewer, so
/// [`cve_2018_0950`] maps the issue's equivalent surfaces layer by
/// layer: the `application/rtf` part parses as an opaque
/// `ContentType::Other{tag:"application/rtf"}` attachment that only the
/// explicit `open_mailcap` / `open_attachment` user action can
/// materialize, the HTML mirror's `file:`/UNC/`<img src>`/`srcset`/CSS
/// `background:url()`/`@import` family dies in `sanitize` in every
/// embedding context, and the launch gate refuses the whole
/// `file:`/`smb:`/UNC/drive-path family while honest
/// http/https/mailto links stay usable. The recon exposed one real
/// gap, fixed with this regression: a bare UNC `href` has no WHATWG
/// scheme, so ammonia's `url_relative = PassThrough` kept it and the
/// `attribute_filter` trim early-return skipped its `is_safe_url`
/// re-validation — the sanitized output carried a live
/// `\\host\share` href that html2text rendered as an outbound link
/// footnote (the terminal mirror of Office's automatic dereference).
/// The fix rejects every Windows-path-shaped relative value and
/// validates `href` before the trim early-return, regression-locked in
/// `meli/src/mail/view/html_render.rs` together with the corpus here.
#[cfg(test)]
#[path = "CVE-2018-0950.rs"]
mod cve_2018_0950;

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

/// CVE-2003-0302 (Eudora 5.2.1, no CVSS score assigned) IMAP
/// literal integer-overflow regression (issue #46, table 2 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution): a
/// malicious IMAP server returned a huge literal size declaration
/// (`{4294967295}` and friends); Eudora parsed the declared octet
/// count into a signed 32-bit integer, and the sign/overflow error
/// crashed the client or possibly executed attacker code (the
/// signedness face of CWE-190). meli's IMAP literal path is
/// unsigned-checked Rust throughout — `digit1` grammar +
/// `usize::from_str` + nom's checked `take`, with no signed or
/// narrowing cast anywhere a declared size flows — so the
/// equivalent surface the issue prescribes
/// (`melib/src/imap/protocol_parser.rs`: [`literal`], the
/// string/astring token grammar, the ENVELOPE field parsers' literal
/// guard, the `fetch_response()` carriers and the line splitter's
/// saturating skip) is locked by [`cve_2003_0302`] on the issue's
/// verbatim corpus — `{4294967295}` and `{-1}` (the same
/// 0xFFFFFFFF bit pattern in its two spellings), the `2^16`/`2^31`/
/// `2^32` type boundaries, the 非数字 grammar class (`{abc}`, `{ 5}`,
/// `{5+}`, `{5}\n` …) and the 与实际数据不符 mismatch class — with
/// the honest literals still round-tripping as the carriers. The
/// result is **immune, no gap found**: the walls are issue #27's
/// saturating line-skip arithmetic, #31's framing cursor clamps and
/// #32's fail-closed mismatch policy, re-locked at this CVE's own
/// sign/grammar/mismatch corpus — a declared number saturates,
/// never wraps, never panics; no production code needed changing.
#[cfg(test)]
#[path = "CVE-2003-0302.rs"]
mod cve_2003_0302;

/// CVE-2007-3166 (Qualcomm Eudora 7.1.0.9, no CVSS score assigned)
/// IMAP FLAGS response buffer-overflow regression (issue #49,
/// table 2 of `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code
/// execution): a malicious IMAP server answered with an over-long
/// FLAGS response — a single flag atom past the client's fixed
/// buffer, or an explosion of flag tokens — and the copy into the
/// fixed-size buffer overflowed (CWE-120; user interaction
/// required). [`cve_2007_3166`] maps the attack onto the
/// issue-prescribed FLAGS 应答 face (`melib/src/imap/
/// protocol_parser.rs`: the [`flags`] list parser, its FETCH /
/// untagged / UID-FETCH-FLAGS / SELECT `* FLAGS (` and
/// `PERMANENTFLAGS` carriers, and the `split_rn` framing below them
/// all) and locks it layer by layer on the verbatim corpus — 单个
/// flag 超长 (1 KiB era-scale, 1 MiB, one byte past
/// `Connection::MAX_SERVER_RESPONSE_SIZE` — no fixed-size buffer
/// exists at any length) and 数量爆炸 (1 024 / 65 536 / 524 288
/// one-byte keywords, retained memory inside a documented 32×
/// response-byte bound) — and exposes one real gap, fixed with this
/// regression in `ImapLineIterator::next`: the literal-declaration
/// probe re-ran `find(CRLF)` over the line tail once per failing
/// `{` candidate, so a `{`-dense over-long FLAGS line framed in
/// O(N²) before any parser ran (measured ≈8 s at 32 KiB debug; the
/// transport cap admits 64 MiB lines) — one hostile response
/// hanging the watch/select/read loops of every IMAP account
/// (CWE-407, the availability twin of this CVE's overflow, the
/// same class issue #24 closed in `phrase()`). The probe now walks
/// the `{` candidates inside the already-found line — one pass —
/// with the literal semantics (issue #27's saturating skip, the
/// honest merges) unchanged, regression-locked in
/// `melib/src/imap/protocol_parser/tests.rs::
/// test_imap_line_iterator_brace_dense_line_is_linear` together
/// with the corpus here.
#[cfg(test)]
#[path = "CVE-2007-3166.rs"]
mod cve_2007_3166;

/// CVE-2007-2770 (Eudora 7.1, Windows; no CVSS score assigned) SMTP
/// response stack-overflow regression (issue #48, table 2 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution): a
/// malicious SMTP server returned an overlong reply and Eudora copied
/// it into a fixed-size stack buffer — arbitrary code executed when
/// the user clicked through the raised warning. [`cve_2007_2770`]
/// maps the attack onto the issue-prescribed surface, the server
/// response reading of `melib/src/smtp.rs` (`read_lines` and both
/// handshake call sites), and drives it over a real TCP SMTP
/// handshake with the prescribed corpus family — 超长应答行 单行
/// MB 级 (terminated and unterminated), 无 CRLF 结尾 (EOF-cut and
/// pumped past the production cap), 大量多行续应答 (endless
/// `220-`/`250-` lines), plus NUL/C0/DEL/ANSI/illegal-UTF-8 payloads
/// at MB scale — at the *production* 64 MiB response cap, not the
/// 128 KiB test-build one. The literal stack-copy primitive has no
/// target in meli (safe Rust, no fixed-size response buffer, heap
/// accumulation already walled by `MAX_SERVER_RESPONSE_SIZE` and the
/// per-read timeout), and the corpus exposes one real gap, fixed
/// with this regression in `read_lines` (CWE-407, the
/// quadratic-rescan class CVE-2024-30103 closed for header values):
/// the CRLF separator search restarted from the beginning of the
/// current line after every 1 KiB read, so an unterminated reply
/// line — this CVE's primary shape — made each chunk rescan the
/// whole accumulated prefix: ≈ 2.2 TB of scanning before the 64 MiB
/// cap fired, minutes of CPU per hostile connection (the alert-hang
/// face of the 2007 advisory). The fix keeps an incremental scan
/// cursor resuming one byte early — a `\r` at the cursor may have
/// met its `\n` in the newest chunk — so the read is one forward
/// pass while a separator split across read chunks is still found
/// (regression-locked in melib's `smtp.rs` tests together with the
/// corpus here).
#[cfg(test)]
#[path = "CVE-2007-2770.rs"]
mod cve_2007_2770;

/// CVE-2004-1944 (Qualcomm Eudora 6.1 / 6.0.3 for Windows, CVSS v2
/// 5.0) nested-MIME denial-of-service regression (issue #51, table 2
/// of `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution):
/// Paul Szabo's bugtraq PoC (BID 10137) crashed Eudora with a stack
/// overflow at 580 levels of nested `multipart/mixed` (570 was fine)
/// — a pure availability attack. [`cve_2004_1944`] locks meli's
/// equivalent surface, the multipart recursion of
/// `melib/src/email/attachments.rs`, with the verbatim generator at
/// both advisory thresholds and far past them: the C4 builder cap
/// keeps every spelling (bare-`\\n` verbatim, `CRLF` mirror,
/// shared-boundary self-similar piles, alternating subtypes,
/// thousands of levels) at a 100-level tree with the remaining
/// subtree as one opaque leaf — and exposes one real gap of the same
/// class, fixed with this regression: the reply path's recursive
/// re-parse (`Attachment::decode_rec_helper`'s inline
/// `message/rfc822` arm) recursed once per hop with no bound, each
/// hop handing `AttachmentBuilder::new` a fresh multipart budget, so
/// a mail nested ≈1700 `message/rfc822` levels deep (measured
/// pre-fix, 2 MiB stack) aborted the process the moment a reply was
/// drafted from it. The fix caps the hop recursion at
/// `MAX_RFC822_DECODE_NESTING_DEPTH` (8, matching the display path's
/// issue-#23 cap), regression-locked in
/// `melib/src/email/attachments.rs` together with the corpus here.
#[cfg(test)]
#[path = "CVE-2004-1944.rs"]
mod cve_2004_1944;

/// CVE-1999-0427 (Eudora 4.1, no CVSS score assigned; CVE candidate
/// proposed 1999-07-28) attachment-filename length denial-of-service
/// regression (issue #50, table 2 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution): one
/// mail carrying an attachment with a long enough filename performed
/// a denial of service on the client — the earliest of the
/// e-mail-client 「长度即拒绝服务」 family, where the name alone was
/// the weapon. [`cve_1999_0427`] drives the issue's over-length
/// corpus class (64 KiB+ filenames, plain / multibyte / RFC 2047
/// armored / RFC 2231 segmented / legacy `Content-Type name=`
/// spellings, up to 1 MiB) through the whole length axis of the
/// surfaces a mail-controlled attachment name travels: melib parsing
/// keeps the name byte-faithful at full length with no fixed-size
/// buffer to truncate or overrun (the 📎 receive indicator with it),
/// the attachment-tree line wraps losslessly and bounded through
/// both pager line breakers, and every landing sink (save
/// components, the mailcap `%s` temp hint) caps the name on a UTF-8
/// character boundary before it touches a filesystem. The
/// end-to-end meli faces (envelope view tree render, batch save) are
/// locked in `meli/src/mail/view/tests.rs`. The result is
/// **immune, no gap found**: the walls are issue #47's pre-landing
/// component cap and issue #24's linear `phrase()`, re-locked at
/// this CVE's own pure-length corpus; no production code needed
/// changing.
#[cfg(test)]
#[path = "CVE-1999-0427.rs"]
mod cve_1999_0427;

/// CVE-2015-8708 (Claws Mail 3.13.1, no CVSS score assigned) Japanese
/// character-set conversion stack-overflow regression (issue #53,
/// table 2 of `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code
/// execution): a stack-based buffer overflow in `conv_euctojis`
/// (`codeconv.c`) let a crafted e-mail crash the client through the
/// EUC-JP → ISO-2022-JP re-encoding direction — the incomplete-fix
/// **bypass** of its same-month sibling CVE-2015-8614 (issue #52),
/// whose advisory covered the `conv_jistoeuc`/`conv_euctojis`/
/// `conv_sjistoeuc` triple. [`cve_2015_8708`] locks meli's immune
/// mapping layer by layer on the issue-prescribed bypass-variant
/// corpus (混合非法转义 + 多字节边界组合: escape-mid-pair interrupts,
/// half-cut designators, CS2/CS3 invocations, unknown G1/G3
/// designators, C1 garbage, claim/byte-stream mismatches, every CTE
/// wrapping and every truncation boundary): the only conversion
/// funnel `decode_charset` maps every charset to a memory-safe
/// `encoding_rs` decoder whose output never exceeds its input and
/// never panics — exact known-answer verdicts included — and the
/// overflow's own re-encode direction has no code to run in (the
/// composer emits UTF-8 only, `charset="utf-8"` and `=?UTF-8?B?…?=`,
/// never a JIS designator), locked through the RFC 2047 header
/// paths, the full CTE × claim body matrix, the exhaustive prefix
/// sweeps and a stacked kitchen-sink mail. The result is **immune,
/// no gap found**: the Shift_JIS claim's Ascii-lossy degradation is
/// documented as the mapping, not a gap of this CVE's class; no
/// production code needed changing.
#[cfg(test)]
#[path = "CVE-2015-8708.rs"]
mod cve_2015_8708;

/// CVE-2015-8614 (Claws Mail < 3.13.1, no CVSS score assigned)
/// Japanese character-set conversion stack-overflow regression
/// (issue #52, table 2 of `SECURITY-CVE-RESEARCH.zh-CN.md` —
/// virus / code execution): a crafted e-mail drove Claws Mail's
/// `codeconv.c` converters (three functions around the
/// ISO-2022-JP / Shift_JIS / EUC-JP family) into multiple stack
/// buffer overflows via malformed ISO-2022-JP escape sequences and
/// half-cut Shift_JIS / EUC-JP multibyte sequences (CWE-121;
/// CVE-2015-8708 later bypassed the incomplete fix). meli's whole
/// conversion surface is one checked `encoding_rs` decoder call
/// per charset behind `decode_charset`, so
/// [`cve_2015_8614`] locks the issue-prescribed surfaces — body
/// decode, RFC 2047 encoded-word headers, `melib/src/text` display
/// layout — over the prescribed corpus (illegal escapes with
/// exhaustive escape-final/byte/pair sweeps, half-cut multibyte
/// families, MB-scale masses) as bounded, deterministic, U+FFFD-
/// replacing conversion — and exposes one real gap, fixed with
/// this regression: `Charset` had no Shift_JIS variant at all, so
/// every alias of the family (`shift_jis`, `Shift-JIS`, `sjis`,
/// `ms_kanji`, `windows-31j`, `cp932`, `x-sjis`, …) fell through
/// to the lossy ASCII default and an honestly labeled Shift_JIS
/// mail rendered as mojibake — the CVE-2015-8708 「variant bypass」
/// face. The fix maps the whole family onto `encoding_rs`'s WHATWG
/// Shift_JIS (Windows-31J) decoder in `melib/src/email/
/// attachment_types.rs` + `parser.rs`'s `decode_charset` (and the
/// mail view's force-charset selector), regression-locked in
/// `melib/src/email/parser/tests.rs::
/// test_charset_shift_jis_label_family_converts` together with the
/// corpus here.
#[cfg(test)]
#[path = "CVE-2015-8614.rs"]
mod cve_2015_8614;

/// CVE-2020-16094 (issue #55, table 2 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution): a
/// malicious IMAP server made Claws Mail ≤ 3.17.6 rebuild the
/// client-side directory tree from an unbounded chain of subfolders,
/// exhausting the client during the rebuild (CWE-674; CVSS 7.5).
/// [`cve_2020_16094`] maps the attack onto meli's issue-prescribed
/// surface — `melib/src/imap/protocol_parser.rs`'s
/// [`list_mailbox_result`] LIST/LSUB ingestion, the mailbox-map
/// construction it feeds (`ImapType::ingest_mailbox_list_line`), and
/// the mailbox-tree consumer in `meli/src/accounts` that is meli's
/// directory-tree equivalent — and locks it on the verbatim 「delimiter
/// 堆叠 10^5 层路径」 corpus plus the every-ancestor ladder Claws
/// Mail's rebuild consumed. Four real gaps it exposed are fixed with
/// this regression: the LIST hierarchy depth was unbounded (now
/// capped at [`MAX_MAILBOX_HIERARCHY_DEPTH`] = 20 levels with a clean
/// deterministic parse error, the 「畸形层级确定报错」 prescription);
/// `build_mailboxes_order` built the owned `MailboxNode` tree with no
/// depth limit of its own and one recursive call per hierarchy level
/// (now an explicit-stack arena rebuild that also refuses to nest
/// deeper than the cap whatever the backend feeds it — foreign
/// backends and the sqlite3 sync cache included — keeping every later
/// walk linear in the number of mailboxes, so the uncapped rebuild's
/// O(depth²) snapshot memory can no longer exhaust the system);
/// `Account::list_mailboxes` plus the tree's derive `Clone`/`Drop`
/// glue spent a frame per level on the sidebar snapshot and teardown
/// as well (now iterative, via [`flatten_mailbox_tree`] and manual
/// constant-stack `Clone`/`Drop`); and the first iterative rewrite
/// linked arena children in reversed sibling order, flipping deep
/// sidebar order and `has_sibling` bits (now pushed off the walk
/// stack in reverse so the deepest-first linking restores natural
/// order). In-crate twins: `melib/src/imap/protocol_parser/tests.rs::
/// test_imap_list_mailbox_result_hierarchy_depth_limit`.
#[cfg(test)]
#[path = "CVE-2020-16094.rs"]
mod cve_2020_16094;

/// CVE-2020-12641 (Roundcube Webmail `<= 1.4.3`, CVSS 9.8, CWE-78) OS
/// command-injection regression (issue #57, table 2 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution): the
/// `im_convert_path` / `im_identify_path` configuration values were
/// concatenated into a shell command line and run through the shell,
/// so a value carrying `;`, `|`, a backtick or `$(...)` executed an
/// arbitrary command with the server's privileges. The trusted field
/// was not the problem; completing a command-string template with
/// mail-controlled text and handing it to `sh -c` was. [`cve_2020_12641`]
/// maps that exactly onto meli's mailcap handler
/// (`meli/src/mailcap.rs`): the local, trusted mailcap command field
/// may legitimately carry pipes and command substitutions, while the
/// mail-controlled `%t` content-type tag, `%{param}` MIME parameter
/// and `%s`/`%F` temporary path are spliced into it. The regression
/// exposes and locks the real gap: the old `quote_shell_word` armor
/// only protects the bare context, so a template that opened `"..."`,
/// `'...'`, a backtick or a `$(...)` region re-activated the value's
/// shell syntax (or mangled it into a syntax error). The fix
/// classifies the quoting context at each substitution point
/// (`shell_quote_context`) and encodes for that exact region, refusing
/// undecidable templates (`ShellContext::Ambiguous`) instead of
/// guessing. Locked by `cve/src/CVE-2020-12641.rs` end to end and by
/// the in-crate twins in `meli/src/mailcap.rs`
/// (`expand_args_double_quoted_context_escapes`,
/// `expand_args_single_quoted_context_reopens_quote`,
/// `expand_args_backtick_context_escapes`,
/// `expand_args_ambiguous_context_fails_closed`).
#[cfg(test)]
#[path = "CVE-2020-12641.rs"]
mod cve_2020_12641;

/// CVE-2002-1770 (Qualcomm Eudora 5.1 on Windows, no CVSS score
/// assigned) HTML-mail `file://` media-embedding script-execution
/// regression (issue #58, table 3 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — web/HTML embedding): an HTML
/// mail used a `t:video` tag to reference a Windows Media Player file
/// attached to the mail through a `file://` URL; Eudora handed the
/// markup to Internet Explorer, which resolved it into the
/// unsandboxed 「My Computer」 local zone and ran the WMV's embedded
/// JavaScript there. meli links no Microsoft HTML/script/media stack,
/// no Windows protocol or Local Machine zone handling, so the issue's
/// equivalent surface is the single HTML display pipeline —
/// [`sanitize`] (ammonia allowlist) followed by [`render`] (html2text
/// → plain terminal text) — and [`cve_2002_1770`] locks it as an
/// immunity proof on a full `multipart/mixed` corpus mail (text/html
/// body + `application/octet-stream` `.wmv` attachment): the
/// whitelist holds none of `object`/`embed`/`iframe`/`video`/
/// `source`/`param`/`t:video`, `file:` is not on the URL-scheme
/// allowlist (`http`/`https`/`mailto` only) for any spelling, casing
/// or quoting, so every carrier and reference dies in `sanitize`, the
/// rendered output is plain text with no file reference or executable
/// marker alive, and the media attachment stays an opaque,
/// never-materialized blob whose script never crosses into the
/// rendered text. No gap was exposed on this corpus; no production
/// code needed changing.
///
/// [`sanitize`]: meli::mail::view::html_render::sanitize
/// [`render`]: meli::mail::view::html_render::render
#[cfg(test)]
#[path = "CVE-2002-1770.rs"]
mod cve_2002_1770;

/// CVE-2024-37385 (Roundcube Webmail < 1.5.7, 1.6.x < 1.6.7, Windows
/// deployments only, CVSS 9.8, CWE-77) OS command-injection regression
/// (issue #56, table 2 of `SECURITY-CVE-RESEARCH.zh-CN.md` —
/// virus / code execution): the ImageMagick configuration paths
/// `im_convert_path`/`im_identify_path` were completed into a shell
/// command line, and **the fix for CVE-2020-12641 (issue #57) was
/// itself incomplete** — it rejected only backslash-leading paths, so
/// the forward-slash UNC spelling `//attacker.example/share/…` (plus
/// whitespace/type-juggling) passed the check and still injected.
/// meli's equivalent surface is the CVE-2020-12641 one: the mailcap
/// `%` substitutions of `meli/src/mailcap.rs`, where mail-controlled
/// bytes (`%t` type tag, `%{param}` MIME parameters, `%s`/`%F` temp
/// paths) join the local, trusted RFC 1524 shell templates — so this
/// CVE attacks *the context-aware encoding that first fix landed*.
/// [`cve_2024_37385`] locks it layer by layer over the
/// issue-prescribed metacharacter corpus (`;`、反引号、`$()`、引号逃逸
/// from malicious attachment names, parameters and type tags): exact
/// known answers against an independent POSIX single-quoting oracle,
/// actually-spawned `sh -c` runs whose stdout must stay byte-verbatim
/// and whose canary markers must stay absent, the CVE's own Windows
/// spellings (`//server/share`, `\\server\share`, `cmd.exe /c`,
/// `%SystemRoot%`, `^&`) as inert data on meli's POSIX surface (the
/// platform-precondition immunity mapping), the scanner's equivalent
/// quoting-region spellings (`$'`, `$"`), and the 「one argv element」
/// wall. It exposes **two real gaps of the exact "incomplete-fix"
/// class this CVE memorializes, both fixed with this regression**:
/// (1) the context scanner did not track here-documents *inside*
/// `$(...)`/backtick regions, so a `%{charset}` landing in such a body
/// was classified bare and armored with `'...'` — literal characters
/// in a here-document body, whose `$()`/backticks still expanded
/// (verified against a real shell pre-fix); a `<<` inside a command
/// substitution is lexically indistinguishable from an arithmetic
/// left shift, so the fix fails closed. (2) an arithmetic `$((...))`
/// region executes command substitutions and backticks in its
/// expression text *even inside quotes* (empirically: a `'x$(touch
/// …)x'` operand still substitutes before the arithmetic parses), so
/// the bare armor a `$((` body used to get let the payload inject —
/// and `$((` is lexically indistinguishable from a command
/// substitution containing a subshell, so no armor satisfies both
/// readings and the fix fails closed there too
/// (`ShellContext::Ambiguous`, the third-`$()`-level contract), while
/// closed `$((…))` regions and nested ordinary `$(...)` bodies keep
/// their bare armor. Locked together with the production newline wall
/// (mailcap continuations are stripped; a file can never deliver the
/// multi-line here-document shape) in `meli/src/mailcap.rs` and the
/// corpus here.
#[cfg(test)]
#[path = "CVE-2024-37385.rs"]
mod cve_2024_37385;

/// CVE-2002-1210 (Qualcomm Eudora 5.1.1 / 5.2, Windows; no CVSS score
/// assigned) predictable-attachment-path + frame-based local-file-read
/// regression (issue #59, table 3 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — 网页嵌入 / web/HTML embedding):
/// Eudora stored an attachment at a sender-predictable path, and a
/// malicious HTML mail used links/frames (`<a href="file://…">`,
/// `<iframe src="file://…">`, `<frameset><frame src=…>`) to load that
/// attachment through `file://` into the local browser context, where
/// the attachment's script could read and exfiltrate arbitrary local
/// files. meli links no browser engine, DOM, CSS/JS runtime or media
/// decoder, so the issue's three-layer equivalent surface is mapped and
/// locked by [`cve_2002_1210`]:
///
/// 1. **The rich-context half is immune by construction** — meli's only
///    HTML display pipeline is [`sanitize`] (ammonia allowlist) →
///    [`render`] (html2text → plain terminal text). The tag allowlist
///    holds no `iframe`/`frame`/`frameset`/`object`/`embed`/`script`
///    (removed with their content), the URL-scheme allowlist is only
///    `http`/`https`/`mailto` (every `file:` spelling — casing, drive
///    path, staging-dir path — is dropped), and the output is plain
///    text with no scripting engine to run the attachment script. No
///    embedding container or `file:` reference survives `sanitize`, the
///    `sanitize` output is a fixed point, and no attachment script
///    marker reaches rendered text.
/// 2. **The launch gate is immune** — even a surviving `file:` link is
///    held by the OS url-launcher gate
///    ([`is_default_launchable_scheme`] / [`url_scheme`]): `file:`
///    (any casing/spelling) is not in `DEFAULT_LAUNCHABLE_SCHEMES` and
///    needs an explicit per-URL confirmation, while an honest
///    `http://attacker.example/collect` stays launchable. Wall locked
///    by the CVE-2008-0039 corpus (issue #33).
/// 3. **The predictable-path half was a real gap** —
///    [`File::create_temp_file`] used to land a hint-bearing name
///    verbatim at `<temp_dir>/meli/<mail-controlled name>`, exactly the
///    Eudora precondition; the parallel issue-59-fix branch randomizes
///    the default landing component to
///    `stem_<32 lowercase hex UUID infix>.<ext>` and tightens the
///    `<temp_dir>/meli` staging directory to `0o700` (it used to follow
///    the process umask, usually `0o755`, letting other local users
///    list a victim's landed attachment names — this CVE's
///    information-leak face). The `temp_landing_path_is_unpredictable_and_private`
///    regression asserts that post-fix contract, so it is expected to
///    fail in this corpus-only worktree until issue-59-fix merges; the
///    remaining tests lock the two immune layers on the full
///    `multipart/mixed` corpus (HTML body with every carrier plus the
///    `eudora_leak.htm` attachment carrying the classic
///    `XMLHttpRequest` → `file://` → exfil script).
///
/// [`sanitize`]: meli::mail::view::html_render::sanitize
/// [`render`]: meli::mail::view::html_render::render
/// [`is_default_launchable_scheme`]: meli::mail::view::envelope::is_default_launchable_scheme
/// [`url_scheme`]: meli::mail::view::envelope::url_scheme
/// [`File::create_temp_file`]: meli::types::File::create_temp_file
/// [`cve_2002_1210`]: self::cve_2002_1210
#[cfg(test)]
#[path = "CVE-2002-1210.rs"]
mod cve_2002_1210;

/// CVE-2001-1326 (Qualcomm Eudora 5.1, Windows; no CVSS score assigned) HTML
/// 表单执行邮件内嵌附件 regression (issue #60, table 3 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — 网页嵌入 / web/HTML embedding): NVD
/// 记录，HTML 邮件里的「伪装成图片链接的表单」被激活（点击看似图片 / 链接
/// 的 submit 控件）后触发表单提交，Eudora 的表单处理把 `action` 解析到邮件
/// 附件的落盘路径，从而执行邮件内嵌附件。攻击链需要四环同时成立：(1) HTML
/// 邮件由内嵌 IE 引擎渲染且 `<form>` 存活；(2) submit 控件伪装成无害图片 /
/// 链接诱导激活；(3) 表单提交处理把 `action` 解析到本地附件路径并启动；
/// (4) 附件落盘路径可预测 / 可达。meli 是终端邮件客户端，唯一 HTML 显示管线
/// 是 [`sanitize`] (ammonia allowlist) → [`render`] (html2text → 纯终端文本)，
/// 四环都没有对应功能面：`sanitize` 的元素白名单不含
/// `form`/`input`/`button`/`select`/`textarea`/`isindex` 等任何交互元素
/// （元素删除、子文本保留，void 元素连同属性整体消失）；唯一 URL 承载属性是
/// `a[href]`，方案白名单只有 `http`/`https`/`mailto`，`action`/`formaction` 随
/// 元素一起消失，`file:`/`javascript:` 无处存活；html2text 之后没有 DOM / 表单
/// 引擎，全仓库（`meli/src`）不存在任何「HTML 表单提交」处理代码（UI 的
/// `FormWidget` 是联系人 / 撰写表单，与邮件 HTML 无关）；附件保持不透明
/// blob，阅读 / 渲染不落盘不执行，打开附件走 mailcap / launcher 的确认门
/// （issue #21/#33/#57 已锁）。[`cve_2001_1326`] 以完整 `multipart/mixed`
/// 语料（伪装成图片链接的 form 正文 + 携带 MZ 头与 payload 的可执行附件）
/// 覆盖 advisory 原形、各类 submit 控件、`action` 各 scheme / 拼写 / 大小写、
/// 嵌在白名单结构里与 math/svg mXSS 近亲，并断言 sanitize 后无任何交互容器 /
/// 无 `file:` / 无附件名、预言机全过且为不动点，render 输出纯文本无
/// `form`/`submit`/`action=` 痕迹，附件解码等于原字节且从不落盘。结论：
/// 免疫证明，未发现缺口，无需改动生产代码。
///
/// [`sanitize`]: meli::mail::view::html_render::sanitize
/// [`render`]: meli::mail::view::html_render::render
/// [`cve_2001_1326`]: self::cve_2001_1326
#[cfg(test)]
#[path = "CVE-2001-1326.rs"]
mod cve_2001_1326;

/// CVE-2002-2351（Qualcomm Eudora 5.1，Windows；NVD 未分配 CVSS 分数）
/// 附件名尾点绕过可执行附件告警 regression（issue #61，表 3 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — 网页嵌入 / web/HTML embedding）：
/// 附件名以 `.` 结尾（`evil.exe.`）或点号堆积（`evil.exe....`）时，
/// Eudora 的「可执行附件安全告警」被绕过——Win32 文件创建 API 会剥离
/// 最终路径组件的尾部点号 / 空格，文件实际落盘为 `evil.exe`，而告警
/// 检查看到的是带尾点的 `evil.exe.`，把扩展名判定为空 / 非 exe，检查与
/// 落盘不一致（check/use divergence）。
///
/// meli 等价面与结论：meli 全仓库（`meli/src`）没有「可执行附件告警」
/// 功能面，本 CVE 的字面「绕过告警」没有可绕过的对象；真正对应的是
/// **邮件控制附件名的落盘汇**——`File::create_temp_file`（mailcap `%s`、
/// open-with 默认应用、envelope.rs 临时落盘）与
/// [`sanitize_filename_component`] + [`unique_filename_component`]（单 /
/// 批量保存、`eml_filename`）。本次发现真实缺口：
/// [`sanitize_filename`] 原先不剥离尾部点号，`evil.exe.` 会原样带着尾点
/// 进入所有落盘汇；在按 Win32 语义归一化的文件系统（WSL `/mnt/c`
/// drvfs、Samba、部分 FUSE）上 OS 会创建为 `evil.exe`，check/use
/// divergence 原样重演。修复：管线末尾追加尾点剥离
/// （`meli/src/types/helpers.rs` 的 `strip_trailing_dots`），并让 192 字节
/// 截断后复剥一次，避免字节上限重新引入尾点；只剥尾点，中缀点号堆积
/// （CVE-2003-0376 契约）与前导点保留，全点名归一为 `""` 走生成名回退。
/// [`cve_2002_2351`] 以内嵌 `multipart/mixed` 语料逐层锁定：wire 真实性
/// （melib 逐字返回尾点名）、规范化剥离表、临时落盘（无尾点、`.exe`
/// 扩展名可见、`<tmp>/meli/`、`0o600` 无执行位、UUID 中缀随机）、孪生
/// 去重（`evil.exe` / `evil.exe.` 不碰撞不丢）、以及免疫映射（`MZ` 头
/// 可执行内容解析 / 显示不物化临时文件；打开附件是 `open_mailcap` +
/// `cmd_buf` 门控的用户显式动作；mailcap shell 上下文编码由 issue
/// #57/#56 锁定）。结论：一个真实缺口，已随本回归修复，生产代码改动
/// 仅限 `meli/src/types/helpers.rs`。
///
/// [`sanitize_filename`]: meli::types::sanitize_filename
/// [`sanitize_filename_component`]: meli::types::sanitize_filename_component
/// [`unique_filename_component`]: meli::types::unique_filename_component
/// [`cve_2002_2351`]: self::cve_2002_2351
#[cfg(test)]
#[path = "CVE-2002-2351.rs"]
mod cve_2002_2351;

/// CVE-2003-0336（Qualcomm Eudora 5.2.1，Windows；NVD 未分配 CVSS 分数）
/// 伪造 `Attachment Converted:` 标记裸 `CR` 注入 / 任意文件读取 regression
/// （issue #62，表 2 of `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code
/// execution family，任意文件读取面）：Eudora 在把 MIME 附件转存为本地
/// 文件后会向邮箱写一行 `Attachment Converted<CR>: "C:\path"` 元数据；其
/// 邮箱解析把裸 `CR`（回车 0x0D）当行终止符（mbox / 老式 Mac 行尾语义），
/// 于是攻击者在邮件里注入同样字节串时，伪造标记会被当成独立的头 / mbox
/// 元数据行，Eudora 随后把引号里的任意本地路径当「已转换附件」读取——任意
/// 文件读取；也可用 `Attachment Converted<CR>:` 名内 CR 拼写绕过只按 `\n`
/// 切分的检查（Paul Szabo 2003-05 bugtraq 系列，与同族 CVE-2003-0376 的
/// [`cve_2003_0376`] 同一份报告）。
///
/// meli 等价面与结论：meli 全仓库没有 Eudora 的 `Attachment Converted`
/// 转换步骤，也没有「邮箱元数据行 → 本地附件路径」这一层；`header_value()`
/// 只在 `LF`/`CRLF` 结束头值，裸 `CR` 留在头值内部，绝不切分新头，名内
/// CR 拼写被 `is_ctl_or_space!` 确定性拒绝；头值进 `Envelope` 时 `phrase`
/// 把中间 CR 折叠为单空格、首尾剥离，地址显示名另有 `sanitize_display_name`
/// 剥 C0，附件落盘名经 [`sanitize_filename_component`]。本次发现真实缺口：
/// `Envelope::set_message_id` 的 `msg_id` 语法失败回退分支
/// `MessageID::new(String::from_utf8_lossy(new_val))` 原样保留裸 `CR`，
/// 攻击邮件 `Message-ID: <a@b<CR>c>` 让结构字段 `message_id` 存入裸 CR，
/// `Draft::new_reply` 再把它原样写进回复的 `In-Reply-To` / `References`，
/// `finalise()` 的 CWE-93 chokepoint 因此永远拒绝发送——邮件控制的裸 CR
/// 是全部代码里唯一存活进外发头值生产者的点，与 Eudora 把裸 CR 当行终止
/// 符的根因同族（fail-closed 可用性破坏）。修复：`MessageID::new`
/// （`melib/src/email/address.rs`）在存储边界剥离 `CR`/`LF`，参照同文件
/// `sanitize_display_name` 的清洗先例；语法合法 `msg-id` 不含 CR/LF，正常
/// 路径逐字节不变。孪生单测
/// `melib/src/email/parser/tests.rs::test_email_envelope_message_id_strips_cr`。
/// [`cve_2003_0336`] 以内嵌攻击语料逐层锁定：L1 语法墙（名内 CR 确定性
/// 失败、CR 切分注入绝不诞生新头、裸 CR 行块只塌缩成一个头）、L2 无路径
/// 语义（正文位标记惰性、不派生附件 / 文件名、`<tmp>/meli` 不变、显示为
/// 折叠纯文本）、L3 外发墙与缺口回归（修复后回复头无 CR 且 `finalise` 为
/// `Ok`，手工含 CR 头值仍被拒绝）、L4 附件名面（wire 逐字保真，展示 /
/// 落盘组件剥控制字符且为单一组件）。
///
/// [`cve_2003_0376`]: self::cve_2003_0376
/// [`cve_2003_0336`]: self::cve_2003_0336
/// [`sanitize_filename_component`]: meli::types::sanitize_filename_component
#[cfg(test)]
#[path = "CVE-2003-0336.rs"]
mod cve_2003_0336;

/// CVE-2001-0677（Qualcomm Eudora 5.0.2，Windows；NVD 未分配 CVSS 分数）
/// `Attachment Converted` 头转发泄露本地文件 regression（issue #63，表 3 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — 网页嵌入 / web/HTML embedding）：Eudora
/// 转存 MIME 附件后会写一行携带目标本地路径的 `Attachment Converted` 元数据
/// 头（如 `"C:\path\secret.txt"`）；用户转发这样一封邮件时，Eudora 读取该头
/// 指向的本地文件并作为附件重新附上发出——用户转发即把本地文件回传给攻击者。
/// meli 没有 Eudora 的 `Attachment Converted` 转存面，唯一 HTML 无关的转发
/// 管线 `Composer::forward`（`meli/src/mail/compose.rs:604`）只搬运邮件自身
/// 字节：`inline` 模式附 `env.body_bytes(bytes)`，`as_attachment` 模式把整封
/// `bytes` 打成 `message/rfc822`、文件名取
/// `eml_filename(env.message_id())`。全仓库唯一的「读本地文件成附件」生产函数
/// `melib/src/email/compose.rs::attachment_from_file` 的调用点全部是用户显式
/// 的附件选择 / 添加附件动作，没有任何从收到的邮件字节推导路径的调用。
/// [`cve_2001_0677`] 以内嵌 `multipart/mixed` 语料逐层锁定为**免疫证明，未
/// 发现缺口，无需改动生产代码**：L1 头拼写变体 / 正文位标记只是
/// `other_headers` 惰性字符串，不派生附件 / 文件名、`<tmp>/meli` 不变；L2
/// 用真实 canary 文件证明转发两种模式都不读取、不附带、不改动头字段指向的
/// 文件，附件字节逐字节等于原邮件自身字节；L3 转发草稿头集合固定，伪造头不
/// 再生（`as_attachment` 中只存在于嵌套 `message/rfc822` 的惰性原始字节里）。
///
/// [`cve_2001_0677`]: self::cve_2001_0677
#[cfg(test)]
#[path = "CVE-2001-0677.rs"]
mod cve_2001_0677;

/// CVE-1999-1016（Microsoft HTML 控件 / IE5 / Outlook Express 5 / Eudora
/// 4.x–5.x；NVD 未分配 CVSS 分数）HTML 表单资源耗尽 / 拒绝服务 regression
/// （issue #64，表 3 of `SECURITY-CVE-RESEARCH.zh-CN.md` — 网页嵌入 /
/// web/HTML embedding）：一封 HTML 邮件把巨型表单字段（`<input type=text
/// value="A…">` MB 级 value、`<textarea>`/`<select>`/`<button>` 巨型内容）
/// 放进 `<table><tr><td>` 单元格；内嵌 MS HTML 控件在打开邮件时为这些字段
/// 建立布局，海量内容让渲染线程跑到 100% CPU、客户端卡死（CWE-400 /
/// CWE-770，开放邮件即触发的可用性攻击）。
///
/// meli 等价面与结论：唯一 HTML 显示管线是 [`sanitize`]（ammonia 白名单）→
/// [`render`]（html2text → 纯终端文本）。**字面触发面免疫**：`input`/
/// `textarea`/`select`/`button`/`form` 都不在元素白名单里，也不在
/// `clean_content_tags`（只有 `script`/`style`），ammonia 按「元素删除」处理
/// ——void 元素 `input` 连同 `value=` 属性整体消失，`textarea`/`select`/
/// `button` 只留子文本，advisory 的巨型字段到不了渲染器。**两个真实缺口，
/// 均已随本回归修复**：
///
/// 1. **CWE-400 输入无上限**：[`render`] 原先对输入大小没有任何上限，>10 MiB
///    的 HTML 邮件（base64 在 wire 上膨胀 ~33%）会在打开邮件时、在阻塞
///    view job 线程上耗尽与输入成正比的 CPU / 内存（release 实测：10 MiB
///    纯嵌套表格 ≈ 3.3 s 最坏形态、10 万个小表 3.7 MB ≈ 790 ms、16 MiB
///    单格 ≈ 276 ms；100 MiB HTML 邮件 ≈ 33 s 100% CPU + ~1 GB 分配）。
///    修复：新增 [`MAX_HTML_RENDER_INPUT_BYTES`]（10 MiB），`render()` 在
///    lossy 解码后、`sanitize()` **之前**把超限输入截到 ≤ 上限的最大 UTF-8
///    字符边界，只对前缀 sanitize + render，再追加一行截断提示；截断在
///    sanitize 之前，故 sanitize 看到最终文档（未闭合 `<script>` 头吞掉余下
///    内容并被整体删除，无 parse differential），≤ 上限输入逐字节不变。
/// 2. **CWE-674 嵌套深度无界（比 100% CPU 更狠：进程 abort）**：html2text
///    0.17.1 用递归 `Drop` 胶水拆渲染树（gdb 实证 `RenderTable →
///    Vec<RenderTableRow> → RenderTableCell → Vec<RenderNode> → …` 链，
///    每表格嵌套层 ~700 B 栈），Rust 无法捕获栈溢出——过阈值即整个 meli
///    进程 abort。实测（默认 2 MiB tokio `spawn_blocking` view job 线程栈）：
///    `<table><tr><td>` 只开标签重复 5 000 次（**~75 KiB 邮件**）即崩，
///    2 000 层存活；字节上限约束输入大小而非嵌套深度，450 KiB 的深嵌套
///    邮件照样穿过 10 MiB 上限杀死进程。修复：新增
///    [`MAX_HTML_RENDER_NESTING_DEPTH`]（256 个字面开始标签深度），对
///    sanitize 后的规范化标记做引号感知扫描，在首个把深度推过上限的开始
///    标签处截断并追加第二条提示；256 比实测 abort 阈值（≥6 000 字面标签）
///    低 ≥23×，比合法邮件嵌套（生成器邮件 ≲30 层表格）高一个数量级。
///
/// 仓内孪生单测（`meli/src/mail/view/html_render.rs::tests`）：
/// `render_at_input_cap_has_no_truncation_notice`、
/// `render_over_input_cap_truncates_ascii_with_single_notice`、
/// `render_over_input_cap_cuts_on_cjk_char_boundary`、
/// `render_over_input_cap_with_invalid_utf8_does_not_panic`、
/// `render_truncation_cannot_smuggle_markup_past_sanitize`、
/// `render_deep_open_only_tables_do_not_abort`、
/// `render_nesting_depth_boundary_is_inclusive`、
/// `render_nesting_cap_ignores_void_elements_and_quoted_closers`、
/// `render_both_caps_compose_on_one_document`、
/// `render_over_cap_nested_tables_completes_within_watchdog`。
/// [`cve_1999_1016`] 以内嵌语料逐层锁定：L1 字面触发面免疫（8 MiB 级巨型
/// 字段死于 sanitize，sanitize / render 输出无表单标签 / `value=` 标记，
/// 且上限不误伤经典形态）；L2 资源面有界 + 崩溃面回归（深度上限内嵌套
/// 完成、**修复前 75 KiB 即 abort 进程**的只开标签深嵌套（5k / 30k /
/// ~699k 层）现在 `Ok` 返回且恰好一条深度提示、深度边界包含、void 元素与
/// 引号内假闭合不干扰计数、12 MiB 最坏形态 watchdog 内 `Ok` 且恰好一条
/// 字节提示无深度提示、双上限同击两条提示各一次）；L3 上限契约（恰好上限
/// 不截断、超限 CJK 字符边界截断、lossy + 上限组合、超限未闭合 script 无
/// 标记、提示纯文本、两个上限常量被引用并在移动时响亮失败）；L4 端到端
/// （base64 `text/html` 两封邮件——11 MiB 巨型最坏形态与 450 KiB 进程杀手
/// ——经 melib 解析、解码、render：有界、各恰好一条对应提示、无表单痕迹）。
/// 结论：一个字面触发面免疫 + 两个真实缺口（CWE-400 + CWE-674），均已随
/// 本回归修复，生产代码改动仅限 `meli/src/mail/view/html_render.rs`。
///
/// [`sanitize`]: meli::mail::view::html_render::sanitize
/// [`render`]: meli::mail::view::html_render::render
/// [`MAX_HTML_RENDER_INPUT_BYTES`]: meli::mail::view::html_render::MAX_HTML_RENDER_INPUT_BYTES
/// [`MAX_HTML_RENDER_NESTING_DEPTH`]: meli::mail::view::html_render::MAX_HTML_RENDER_NESTING_DEPTH
/// [`cve_1999_1016`]: self::cve_1999_1016
#[cfg(test)]
#[path = "CVE-1999-1016.rs"]
mod cve_1999_1016;

/// CVE-2007-4040（Outlook / Outlook Express，Windows；CVSS 8.8）URI 参数注入
/// regression（issue #65，表 3 of `SECURITY-CVE-RESEARCH.zh-CN.md` — 网页嵌入）：
/// 特定已注册 URI 携带 shell 元字符（`unknown:$(cmd)`、
/// `mailto:x?subject=;cmd`、含空格/引号的 URI）传入处理进程命令行，因客户端把
/// URI 拼进 shell 命令行而跨浏览器脚本 / 任意命令执行。meli 的「URI → 进程
/// 命令行」四个汇（`go_to_url` → `launch_url`、`List-Unsubscribe` URL、
/// `List-Archive`、相邻的 desktop-Exec `sh -c` 拼接）全部不经 shell 或带逐字符
/// 反斜杠 armor。[`cve_2007_4040`] 以内嵌 `multipart/alternative` 语料逐层锁
/// 定为**免疫证明，未发现缺口，无需改动生产代码**：scheme 确认门对元字符
/// corpus 的分类（unknown/`search-ms:`/无 scheme 必须确认，http/https/mailto
/// 任意大小写放行）、提取地图（无 `//` 的 `unknown:$(cmd)` 根本不提取、
/// `://` 拼写提取但落后于门、mailto 散文只产出落后于门的裸 email、白名单
/// scheme 的元字符 URL 直达 argv 契约层）、sanitize 只留白名单 href 且字节
/// 原样、`unsubscribe_action` 跳过未知 scheme 元字符选项并把白名单元字符
/// URL 逐字节带入 `OpenUrl`（有 mailto 时优先内部 `Send`）、畸形 mailto
/// fail-closed / 合法 mailto 的元字符只是惰性写信字段、以及
/// `desktop_exec_to_command` 的转义 armor。argv 单参数逐字节契约（`$(touch
/// marker)` 副作用预言机）由仓内孪生单测锁定在
/// `meli/src/mail/view/tests.rs`（`Context::new_mock` 为 `#[cfg(test)]`，
/// cve crate 无法驱动交互流）：`go_to_url_metacharacter_url_is_one_literal_argv_element`、
/// `go_to_url_unknown_scheme_metacharacters_need_confirmation_and_stay_literal`、
/// `list_archive_metacharacter_url_is_one_literal_argv_element`。
///
/// [`cve_2007_4040`]: self::cve_2007_4040
#[cfg(test)]
#[path = "CVE-2007-4040.rs"]
mod cve_2007_4040;

/// CVE-2007-2225（Outlook Express 5.5–6 / Windows Mail；MS07-034，NVD 未分配
/// CVSS 分数）MHTML 协议跨域信息泄露 regression（issue #66，表 3 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — 网页嵌入）：MHTML（`.mht`）协议处理器把
/// `mhtml:<URI>!<Content-Location>` 交给 IE 的 MHTML 解析器，该解析器对 UNC
/// 路径与 `Content-Disposition` 处理不当，允许构造的 MHTML URL 跨 IE 安全域
/// 读取本地 / 他域内容（CWE-200 类；同公告下 CVE-2007-2227 / CVE-2008-1448
/// 为同族）。meli 是终端客户端，没有 MHTML 协议处理器、没有 IE 安全域、没有
/// COM/URL moniker，字面的「`mhtml:` → IE 跨域读取」站点不存在；issue 指定的
/// 等价面是链接 scheme 白名单（`meli/src/mail/view/envelope.rs` 的
/// `is_default_launchable_scheme` / `url_scheme`，与
/// `meli/src/mail/view/html_render.rs` 的 `sanitize` ammonia `url_schemes`
/// 白名单），预期断言为白名单拒绝 `mhtml:`、其余仅纯文本显示。[`cve_2007_2225`]
/// 以内嵌 `multipart/alternative` 语料与 MHT 归档镜像语料逐层锁定为**免疫证明，
/// 未发现缺口，无需改动生产代码**：L1 语料是真实载体（melib 解析、双路负载
/// 逐字在场、`List-*` 头部逐字抵达）；L2 启动门对规范形态、跨域 `!` 定位、UNC、
/// 大小写、percent-编码、query、fragment 全拒、良性 `https`/`mailto` 全放；
/// L3 URL 模式提取从不保留 `mhtml:` 前缀（`file://` 余项落后于确认门、
/// `http://` 前缀剥离后是普通链接、裸 UNC/反斜杠主体不提取而只落到 `!` 后的
/// 普通 http(s) 尾巴，即「仅以纯文本显示」）；
/// L4 HTML 镜像的 `mhtml:` href 全部死于 sanitize、锚文本留为惰性散文；
/// L5 `unsubscribe_action` 跳过 `mhtml:` 选项落到合法 `https:`、`List-Archive`
/// 拒绝 `mhtml:`、`mailto:` 仍胜出为内部写信；L6 诚实载体每一层保持可用；
/// L7 内嵌 MHT 归档以 `Content-Disposition: attachment` 归类为惰性附件、不自动
/// 打开 / 执行，其 html 叶子经同一 sanitize 管道无 `mhtml:` 幸存。
///
/// [`cve_2007_2225`]: self::cve_2007_2225
#[cfg(test)]
#[path = "CVE-2007-2225.rs"]
mod cve_2007_2225;

/// CVE-2007-2227（Outlook Express 5.5–6 / Windows Mail；MS07-034，NVD 未分配
/// CVSS 分数）MHTML 协议跨域信息泄露 regression（issue #67，表 3 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — 网页嵌入）：与 CVE-2007-2225（issue #66）
/// 同属 MS07-034 家族（另见 CVE-2008-1448）。MHTML 协议处理器对 UNC 路径与
/// `Content-Disposition` 处理不当，跨 IE 安全域读取本地 / 他域内容；本 CVE 的
/// 攻击语料是 MHTML 文档内嵌的本地资源引用协议 `res:`（`res://…shdoclc.dll/
/// …htm` 指向系统 DLL 内嵌资源页）与 `mk:`（`mk:@MSITStore:…chm::/…htm` 经
/// HTML Help moniker 读取 `.chm`），以及时代同族 `its:` / `ms-its:`。meli 是
/// 终端客户端，没有 MHTML 协议处理器、没有 IE 安全域、没有 COM/URL moniker
/// （`res:`/`mk:` 依赖的 MSITStore），字面的「`mhtml:` → IE 跨域读取本地资源」
/// 站点不存在；issue 指定的等价面是链接 scheme 白名单
/// （`meli/src/mail/view/envelope.rs` 的 `is_default_launchable_scheme` /
/// `url_scheme`，与 `meli/src/mail/view/html_render.rs` 的 `sanitize` ammonia
/// `url_schemes` 白名单），预期断言为白名单拒绝 `res:`/`mk:` 等本地资源 scheme、
/// 其余仅纯文本显示。[`cve_2007_2227`] 以内嵌 `multipart/alternative` 语料与
/// 帮助文档镜像语料逐层锁定为**免疫证明，未发现缺口，无需改动生产代码**：L1
/// 语料是真实载体（melib 解析、双路负载逐字在场、`List-*` 头部逐字抵达）；
/// L2 启动门对 `res:`/`mk:`/`mhtml:`/`its:`/`ms-its:` 的规范形态、资源 ID 变体、
/// 大小写、percent-编码、query、fragment、复合变体全拒、裸 UNC 解析为无 scheme、
/// 良性 `https`/`http`/`mailto` 全放；L3 URL 模式提取只提取带 `://` 的
/// `res://…`（落后于确认门）、`mhtml:res://…` 剥前缀后与裸 `res://` 同类，
/// `mk:`/`its:`/`ms-its:`/裸 UNC 不提取而只落到 `mk:…http://` 的普通 http 余项，
/// 即「仅以纯文本显示」；L4 HTML 镜像的本地资源 href 全部死于 sanitize、锚文本
/// 留为惰性散文；L5 `unsubscribe_action` 跳过 `res:`/`mk:` 选项落到合法
/// `https:`、`List-Archive` 拒绝 `res:`、`mailto:` 仍胜出为内部写信；L6 诚实
/// 载体每一层保持可用；L7 内嵌帮助文档镜像与 base64 stub 以
/// `Content-Disposition: attachment` 归类为惰性附件、不自动打开 / 执行，其
/// html 叶子经同一 sanitize 管道无 `res:`/`mk:` 幸存，外联正文不含负载。
///
/// [`cve_2007_2227`]: self::cve_2007_2227
#[cfg(test)]
#[path = "CVE-2007-2227.rs"]
mod cve_2007_2227;

/// CVE-2008-1448（Outlook Express 5.5 SP2 / 6 SP1 与 Windows Mail；Microsoft
/// 公告 MS08-048，CVSS v2 7.1 HIGH AV:N/AC:M/Au:N/C:C/I:N/A:N，CWE-264，发布
/// 2008-08-12；aka "URL Parsing Cross-Domain Information Disclosure
/// Vulnerability"）MHTML 协议跨域信息泄露 regression（issue #68，表 3 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — 网页嵌入）：与 CVE-2007-2225（issue #66）、
/// CVE-2007-2227（issue #67）同属 MS07-034 / MS08-048 的 MHTML 跨域家族。MHTML
/// 协议处理器未给 UNC 共享路径分配正确的 IE 安全域，且未正确处理 MHTML URL
/// 重定向，远程攻击者借 `mhtml:` URI + 重定向绕过 IE 域限制读取任意文件；本
/// CVE 的特有向量是 UNC 安全域错配（`mhtml:file://\\host\share\…`）、MHTML URL
/// 重定向 + 构造 HTTP 头（`mhtml:http://…/redirect.mhtml!http://victim/…`）与
/// multipart 部件引用（`mhtml:http://x.example/a.html!cid`，`!` 后是
/// Content-Location / Content-ID 部件定位，配合 `Content-Disposition` 语义跨域
/// 读取）。meli 是终端客户端，没有 MHTML 协议处理器、没有 IE 安全域、没有
/// COM/URL moniker，字面的「`mhtml:` → IE 跨域读取」站点不存在；issue 指定的
/// 等价面是链接 scheme 白名单（`meli/src/mail/view/envelope.rs` 的
/// `is_default_launchable_scheme` / `url_scheme`，与
/// `meli/src/mail/view/html_render.rs` 的 `sanitize` ammonia `url_schemes`
/// 白名单 http/https/mailto），预期断言为白名单拒绝 `mhtml:` 的全部形态（含 `!`
/// 部件引用后缀）、无跨域读取通路、其余仅纯文本显示。[`cve_2008_1448`] 以内嵌
/// `multipart/alternative` 语料与「构造 multipart 部件引用」`.mht` 归档语料逐层
/// 锁定为**免疫证明，未发现缺口，无需改动生产代码**：L1 语料是真实载体（melib
/// 解析、双路负载逐字在场、`List-*` 头部逐字抵达）；L2 启动门对规范 `!cid` 部件
/// 引用、cid 值 / 他域部件变体、`!!` 双跳、UNC 错配、重定向、本地文件部件、
/// 大小写、percent-编码、query、fragment 全拒、裸 UNC 解析为无 scheme、良性
/// `https`/`http`/`mailto` 全放；L3 URL 模式提取从不保留 `mhtml:` 前缀
/// （`mhtml:http://…!cid` 剥前缀后是普通 `http://…!cid`、`!cid` 只是惰性路径
/// 字节，`mhtml:file://…` 余项落后于确认门、反斜杠主体 / 裸 UNC 不提取而只落到
/// `!` 后的普通 http(s) 尾巴）；L4 HTML 镜像的 `mhtml:` href 全部死于 sanitize，
/// `<meta http-equiv="refresh">` 重定向元素因不在 ammonia 标签白名单里整体死亡，
/// 锚文本留为惰性散文；L5 `unsubscribe_action` 跳过 `mhtml:` 选项落到合法
/// `https:`、`List-Archive` 拒绝 `mhtml:`、`mailto:` 仍胜出为内部写信；L6 诚实
/// 载体每一层保持可用；L7 内嵌 `.mht` 归档以 `Content-Disposition: attachment`
/// 归类为惰性附件、不自动打开 / 执行，其 html 叶经同一 sanitize 管道无 `mhtml:`
/// 幸存，Content-Location / Content-ID 头部字节永不流入链接面（对每个文本部件跑
/// URL 模式提取，没有任何链接值等于只存在于头部的受害者域 URL，证明无跨域读取
/// 通路），构造的 `Content-Disposition` 部件字节保持休眠附件。
///
/// [`cve_2008_1448`]: self::cve_2008_1448
#[cfg(test)]
#[path = "CVE-2008-1448.rs"]
mod cve_2008_1448;
/// CVE-2021-37746（Claws Mail < 3.18.0、Sylpheed ≤ 3.7.0；CVSS 6.1，NVD）链接
/// 校验不足 regression（issue #69，表 3 of `SECURITY-CVE-RESEARCH.zh-CN.md` —
/// 网页嵌入）：点击链接前的 URI 检查不充分，伪装链接（钓鱼）可达成本地资源。
/// issue 指定攻击样例三件套：伪装链接 `<a href="http://evil.example">
/// http://bank.example</a>`、Unicode 同形字符（西里尔 а U+0430）、U+202E 右向
/// 覆盖（RLO）。[`cve_2021_37746`] 以内嵌 `multipart/alternative` 语料逐层锁定
/// 为**免疫证明，未发现缺口，无需改动生产代码**：CVE 根因是「展示的链接文字」
/// 与「实际打开的 URI」分叉而点击前不校验；meli 链接面不存在这一分叉——可启动
/// 链接值永远是展示文本的逐字节子串（URL 模式 linkify 扫描对象就是 pager 显示
/// 的同一字符串，编号标记插在链接字节前），HTML 锚点的 href——唯一分叉点——被
/// html2text plain() 配置以编号脚注逐字节展示（防钓鱼可见性设计）。sanitize
/// 把伪装本地资源（file:）与 host 内 bidi 控制符（UTS #46/IDNA 拒绝）的 href
/// 整体杀死（伪装目标渲染前不可达），前缀 Cf 被 attribute trim 剥净，存活
/// href（同形字符 host、path 内 RLO——尾部 Cf 被 trim）逐字节可见可点；
/// mailto 脚注只产出裸 email 类链接；启动门按可见 scheme 分类（前缀 Cf、
/// file: 落逐 URL 确认门后）；诚实对照每层可用。展示字节恒等启动字节的 argv
/// 契约由仓内孪生单测 `go_to_url_disguised_lookalike_urls_are_one_literal_argv_element`
/// （`meli/src/mail/view/tests.rs`）锁定。meli 自身按逻辑序渲染、不做 bidi
/// 重排；双感知终端的视觉重排发生在 meli 之外，但展示与启动恒等——终端显示
/// 什么就启动什么。
///
/// [`cve_2021_37746`]: self::cve_2021_37746
#[cfg(test)]
#[path = "CVE-2021-37746.rs"]
mod cve_2021_37746;

/// CVE-2015-7609（Zimbra 网页邮箱；NVD 未分配 CVSS 分数；Fortinet 披露）邮件正文
/// XSS regression（issue #70，表 3 of `SECURITY-CVE-RESEARCH.zh-CN.md` — 网页嵌入 /
/// web/HTML embedding）：邮件内容未充分清洗即进入页面，攻击者在一封普通邮件里注入
/// 任意脚本——`<script>`（含 SVG/MathML 命名空间、未闭合吞尾、实体 / 大小写 / 属性
/// 混淆变体）、`<img src=x onerror=…>` 及 on* 全事件属性家族、`javascript:` URL
/// （href 载体与正文散文载体）。
///
/// meli 等价面映射（issue #70 明确要求）：meli 是终端邮件客户端，**没有 webmail
/// 功能面**——没有 HTTP 页面、会话 Cookie、浏览器 DOM 或 JavaScript 引擎。邮件
/// HTML 唯一的消费通路是邮件视图的显示管线 [`sanitize`]（ammonia 白名单：html5ever
/// 恰好一次解析 → 过滤树 → 带转义再序列化）→ [`render`]（html2text → 纯终端文本）：
/// `script`/`style` 属 ammonia `clean_content_tags` 默认集，连内容整体删除，
/// SVG/MathML 命名空间的 `script` 同样按本地名删除；`tag_attributes` 是整体替换，
/// 只保留 `a[href]`/`a[title]`，通用属性保持 ammonia 默认 `{lang,title}`，on*
/// 家族无处存活；`url_schemes` 只有 http/https/mailto，ammonia 对每个保留 href 用
/// WHATWG `Url::parse` 取 scheme（大小写归一、剥离 ASCII tab/LF/CR 与首尾 C0 /
/// 空格），meli 的 `attribute_filter` 在 trim 首尾 Cf / 控制符 / 空白后**重新校验**
/// href（CVE-2025-66376 硬化），前缀 Cf 隐藏的 `javascript:` 被整体丢弃而非被 trim
/// 激活；sanitize 输出再交给 html2text 降成终端文本，没有第二次“浏览器解析”可供
/// 碎片重组，也没有任何脚本 / 表单 / URL scheme 执行器。
///
/// [`cve_2015_7609`] 以内嵌 `multipart/alternative` 语料（plain + html 双叶）逐层
/// 锁定为**免疫证明，未发现缺口，无需改动生产代码**：L1 melib 解析出双叶且全部
/// 攻击字节逐字在场；L2 sanitize 输出无任何非白名单标签 / `on*` 属性 / 活的危险
/// scheme href 且为不动点；L3 render 里真实脚本体整体消失、事件属性值不出现、
/// 危险 scheme href 不生成脚注，正文散文 / 实体转义 / 注释切分留下的 `javascript:`
/// / `on*=` 只是惰性字面文本；L4 整封邮件端到端纯文本且诚实 `http`/`https`/`mailto`
/// 链接保持可用（sanitize 保留、render 脚注可见、`is_default_launchable_scheme`
/// 放行）。唯一记录性非对称点：`<noscript>` / 实体转义 / 双编码 / NUL 混淆标签名
/// 里的危险标记本来就不是脚本元素，render 解出的字面字符串不是活语义；
/// `java\u{200B}script:` 的内部 Cf 让 scheme 不成立（`url_scheme` 返回 `None`、
/// 确认门拒绝），最多作为惰性脚注展示。仓内孪生改动：无（免疫证明，未触碰生产代码）。
///
/// [`sanitize`]: meli::mail::view::html_render::sanitize
/// [`render`]: meli::mail::view::html_render::render
/// [`cve_2015_7609`]: self::cve_2015_7609
#[cfg(test)]
#[path = "CVE-2015-7609.rs"]
mod cve_2015_7609;

/// CVE-2008-2248（Outlook Web Access for Exchange Server 2003 SP2；CPE 还含
/// Exchange 2007 / 2007 SP1；Microsoft 公告 MS08-039，2008-07-08，CVSS v2 4.3
/// AV:N/AC:M/Au:N/C:N/I:P/A:N，CWE-79，BID 30078）OWA 网页邮箱邮件 HTML XSS
/// regression（issue #71，表 3 of `SECURITY-CVE-RESEARCH.zh-CN.md` — 网页嵌入 /
/// web/HTML embedding）：OWA 未充分清洗邮件 HTML 即把内容渲染进 webmail 浏览器
/// 页面，远程攻击者在一封普通邮件里注入任意 web 脚本 / HTML（NVD 原文 "via
/// unspecified HTML"），脚本在 OWA 源执行（会话 Cookie 窃取等）；同公告姊妹
/// 漏洞 CVE-2008-2247（via unspecified e-mail fields，BID 30130）是不同注入面。
///
/// meli 等价面映射（issue #71 明确要求）：meli 是终端邮件客户端，**没有 webmail
/// 功能面**——没有 HTTP 页面、会话 Cookie、浏览器 DOM 或 JavaScript 引擎。邮件
/// HTML 唯一的消费通路是邮件视图的显示管线 [`sanitize`]（ammonia 白名单：
/// html5ever 恰好一次解析 → 过滤树 → 带转义再序列化）→ [`render`]（html2text →
/// 纯终端文本）：`script`/`style` 属 `clean_content_tags` 连内容整体删除，
/// `iframe`/`object`/`embed`/`applet`/`frameset`/`bgsound`/`xml`/`isindex`/`form`/
/// `input`/`button`/`select`/`textarea`/`keygen`/`meta`/`base`/`link`/`video`/
/// `audio`/`source`/`track`/`details` 等非白名单元素连同 `src`/`data`/`action`/
/// `formaction`/`style`/`xlink:href`/`on*` 属性一起消失；`tag_attributes` 是整体
/// 替换（只留 `a[href]`/`a[title]`，通用属性 `{lang,title}`）；`url_schemes` 只有
/// http/https/mailto，ammonia 对每个保留 href 用 WHATWG `Url::parse` 取 scheme，
/// meli 的 `attribute_filter` 在 trim 首尾 Cf / 控制符 / 空白后**重新校验** href
/// （CVE-2025-66376 硬化）；sanitize 输出再交给 html2text 降成终端文本，没有
/// 第二次“浏览器解析”可供碎片重组，也没有任何脚本 / 表单 / CSS / URL scheme
/// 执行器。`<base href>` 重定基另由 MFSA-2005-11 的
/// `base_href_cannot_rebase_relative_urls` 锁定。
///
/// [`cve_2008_2248`] 以内嵌 `multipart/alternative` 语料（plain + html 双叶）逐层
/// 锁定为**免疫证明，未发现缺口，无需改动生产代码**。语料类别 A-J 与
/// CVE-2015-7609（issue #70，Zimbra；`cve/src/CVE-2015-7609.rs`）互补不重复：
/// A IE 嵌入 / 脚本宿主元素（iframe/object/embed/applet/frameset/frame/bgsound/
/// XML 数据岛/isindex/VML/条件注释）、B 导航 / meta 家族（refresh 到 javascript:、
/// UTF-7 charset 嗅探、base/link）、C 表单家族（action/formaction/autofocus
/// onfocus/onsubmit）、D CSS 脚本（白名单元素 style 属性 expression/-moz-binding/
/// behavior、style 元素 @import、未闭合吞尾）、E SVG/MathML 动画与属性注入
/// （animate/set/use/image/maction）、F HTML5 媒体 / autofocus 事件载体
/// （video/audio/source/track/details）、G scheme 混淆补（真实 LF/CR、`&Tab;`/
/// `&NewLine;`、无引号/单引号、等号前换行、斜杠分隔、`&#106;` 有/无分号）、
/// H RCDATA/raw-text（title/textarea/xmp/plaintext/listing）、I CDATA/PI/doctype、
/// J 惰性边界（scheme 内 NUL/全角、诚实 meta/base、相对/fragment）。分层：L1
/// melib 解析出双叶且全部攻击字节逐字在场（LF→CRLF 容忍）；L2 sanitize 输出无
/// 任何非白名单标签 / `on*` / 禁止属性 / 活的危险 scheme href 且为不动点；L3
/// render 里脚本 / 嵌入 / 表单元素整体消失、事件与 CSS 属性值不出现、危险 scheme
/// 不生成脚注，RCDATA / CDATA / 转义只解出惰性字面字符串，UTF-7 以字面 `+ADw-`
/// 字节交付，惰性边界的 `url_scheme` 为 `None` 且确认门拒绝；L4 整封攻击邮件
/// 端到端纯文本且诚实 `http`/`https`/`mailto` 链接保持可用（sanitize 保留、render
/// 脚注可见、`is_default_launchable_scheme` 放行）。实测记录：`&#106avascript:`
/// （无分号）与 `&#106;avascript:`（有分号）都被 html5ever 解码为 `javascript:`
/// 并被整体丢弃；`<![CDATA[<script>…` 在 HTML 上下文是覆盖到首个 `>` 的 bogus
/// comment，余下 `alert(1)]]>` 为惰性字面文本；`title`/`textarea`/`xmp`/`plaintext`
/// 内标记为惰性字面文本而 `listing` 是普通元素、内部真实 `<script>` 被整体删除；
/// scheme 内 NUL 被替换为 U+FFFD、全角 scheme 非 ASCII，两者最多作惰性脚注。
/// 仓内孪生改动：无（免疫证明，未触碰生产代码）。
///
/// [`sanitize`]: meli::mail::view::html_render::sanitize
/// [`render`]: meli::mail::view::html_render::render
/// [`cve_2008_2248`]: self::cve_2008_2248
#[cfg(test)]
#[path = "CVE-2008-2248.rs"]
mod cve_2008_2248;

/// CVE-2015-8864（Roundcube &lt; 1.0.9 / 1.1.x &lt; 1.1.5；NVD 未分配 CVSS 分数；
/// 同族 CVE-2016-4068）邮件内 SVG XSS regression（issue #72，表 3 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — 网页嵌入 / web/HTML embedding）：NVD 记录，
/// 攻击者在一封普通邮件里放入构造的 SVG 文档（正文内联或作为 `image/svg+xml`
/// 附件），Roundcube 未充分清洗即把邮件交给浏览器，SVG 命名空间里的脚本 / 事件
/// 属性 / 危险 URL 在 webmail 页面里执行；SVG 因自身命名空间与解析差异，是绕过
/// 「HTML 标签白名单」的经典载体。
///
/// meli 等价面映射（issue #72 明确要求）：meli 是终端邮件客户端，**没有 webmail
/// 功能面**——没有 HTTP 页面、会话 Cookie、浏览器 DOM、SVG 渲染器或 JavaScript
/// 引擎。邮件 HTML 唯一的消费通路是邮件视图的显示管线 [`sanitize`]（ammonia
/// 白名单：html5ever 恰好一次解析 → 过滤树 → 带转义再序列化）→ [`render`]
/// （html2text → 纯终端文本）：`script`/`style` 属 `clean_content_tags` 连内容
/// 整体删除，SVG foreign content 里的 `script` 仍按本地名命中；`svg`/
/// `foreignObject`/`animate`/`set`/`image`/`use`/`feImage` 等非白名单元素删除、
/// 仅保留再次清洗并转义的子文本；`tag_attributes` 是整体替换（只留
/// `a[href]`/`a[title]`，通用属性 `{lang,title}`），SVG 事件与动画属性无处存活；
/// `url_schemes` 只有 http/https/mailto，`xlink:href` / SVG `image href` / `use
/// href` / 动画 `values` 里的 `javascript:` 与 `data:image/svg+xml,…` 一并死亡，
/// meli 的 `attribute_filter` 在 trim 首尾 Cf / 控制符 / 空白后**重新校验** href
/// （CVE-2025-66376 硬化）；sanitize 输出再交给 html2text 降成终端文本，没有
/// 第二次“浏览器 / SVG 解析”可供碎片重组，也没有任何脚本 / URL scheme 执行器。
///
/// [`cve_2015_8864`] 以内嵌 `multipart/alternative` 语料（plain + html 双叶）与
/// 带 `image/svg+xml` 附件叶的 `multipart/mixed` 语料逐层锁定为**免疫证明，
/// 未发现缺口，无需改动生产代码**。语料与 CVE-2015-7609（issue #70，Zimbra）、
/// CVE-2008-2248（issue #71，OWA）互补不重复，SVG 专精深挖：a SVG 内 `script`
/// （issue 指定原形、大小写 / 属性 / 实体混淆、嵌套 SVG、`foreignObject`、
/// 未闭合吞尾）；b SVG 事件属性全家族（issue 指定 `<svg onload=…>` 原形与
/// `onbegin`/`onend`/`onrepeat`/`onactivate`/`onfocusin`/`onfocusout`/`onerror`/
/// `onanimation*`/`ontoggle` 等，挂 `<svg>` 根与 `<animate>`/`<set>`/`<image>`/
/// `<use>`/`<foreignObject>`/`<a xlink:href>` 载体）；c SVG 载体危险 URL
/// （`xlink:href="javascript:…"` 混淆、`<use xlink:href="data:image/svg+xml,…">`、
/// `<image href="javascript:…">`、动画 `values` 注入、HTML 锚点上的 SVG data
/// URL）；d SVG mXSS 近亲（`<svg><style>` 属性逃逸、SVG 内 CDATA、注释切分
/// `<s<!--c-->vg onload=…>`、`foreignObject` + `style`、`attributeName=onload`/
/// `xlink:href` 动画注入、`title` 突围）；e 实体转义 / 双编码文本与惰性 href
/// 边界。分层：L1 melib 解析出双叶 / 三叶且全部攻击字节逐字在场，`image/svg+xml`
/// 附件归类为 `ContentDisposition::Attachment` 惰性附件、其脚本字节绝不进入
/// HTML 显示管线；L2 sanitize 输出无任何非白名单 SVG 标签 / `xlink:href` /
/// `on*` / 禁止属性 / 活的危险 scheme href 且为不动点；L3 render 里真实 SVG
/// 脚本体整体消失、事件属性值不出现、危险 scheme 不生成脚注，SVG CDATA / 实体
/// 转义只解出惰性字面字符串（显式记录该非对称点），惰性边界的 `url_scheme` 为
/// `None` 且确认门拒绝；L4 整封攻击邮件端到端纯文本且诚实 `http`/`https`/`mailto`
/// 链接保持可用（sanitize 保留、render 脚注可见、`is_default_launchable_scheme`
/// 放行），`l4_webmail_script_pathway` 的注释记录全仓无 webmail / 浏览器 / SVG
/// 渲染器 / JS 引擎通路的映射依据。仓内孪生改动：无（免疫证明，未触碰生产代码）。
///
/// [`sanitize`]: meli::mail::view::html_render::sanitize
/// [`render`]: meli::mail::view::html_render::render
/// [`cve_2015_8864`]: self::cve_2015_8864
#[cfg(test)]
#[path = "CVE-2015-8864.rs"]
mod cve_2015_8864;

/// CVE-2016-4068（Roundcube &lt; 1.0.9 / 1.1.x &lt; 1.1.5；NVD 未分配 CVSS 分数；
/// 与 CVE-2015-8864 同族的两个漏洞之一）邮件内 SVG XSS regression（issue #73，
/// 表 3 of `SECURITY-CVE-RESEARCH.zh-CN.md` — 网页嵌入 / web/HTML embedding）：
/// NVD 记录，攻击者在一封普通邮件里放入构造的 SVG 文档（正文内联或作为
/// `image/svg+xml` 附件），Roundcube 未充分清洗即把邮件交给浏览器；issue #73
/// 指定的攻击面是同族变体——SVG 内 `<animate>`/`<set>` 修改 `href`，以及 SVG
/// `use` 外链等脚本注入形态。
///
/// meli 等价面映射（issue #73 明确要求）：meli 是终端邮件客户端，**没有 webmail
/// 功能面**——没有 HTTP 页面、会话 Cookie、浏览器 DOM、SVG 渲染器、JavaScript
/// 引擎，也没有 SMIL 动画引擎。邮件 HTML 唯一的消费通路是邮件视图的显示管线
/// [`sanitize`]（ammonia 白名单：html5ever 恰好一次解析 → 过滤树 → 带转义再
/// 序列化）→ [`render`]（html2text → 纯终端文本）：`animate`/`set`/
/// `animateTransform`/`animateMotion`/`discard`/`mpath`/`use` 都不在 24 项标签
/// 白名单里，且非白名单的 `<svg>` 被 ammonia 解包后其 SVG 命名空间子节点按
/// `check_expected_namespace` 连整棵子树（元素、文本与 `attributeName`/`values`/
/// `from`/`to`/`begin`/`dur`/`repeatCount`/`fill`/`xlink:href` 属性）一并删除；
/// 能保留 `href` 的只有 HTML 命名空间的 `<a>`，而 `tag_attributes` 整体替换只留
/// `href`/`title`，`href` 经 `url_schemes`（只有 http/https/mailto）与
/// `attribute_filter` trim 后重新校验（CVE-2025-66376 硬化）；`script`/`style`
/// 属 `clean_content_tags` 连内容整体删除；sanitize 输出再交给 html2text 降成
/// 终端文本，没有第二次「浏览器 / SVG / SMIL 解析」可供碎片重组，也没有任何脚本 /
/// URL scheme / 动画执行器。
///
/// [`cve_2016_4068`] 以内嵌 `multipart/alternative` 语料（plain + html 双叶）与
/// 带 `image/svg+xml` 附件叶的 `multipart/mixed` 语料逐层锁定为**免疫证明，
/// 未发现缺口，无需改动生产代码**。语料与 CVE-2015-8864（issue #72）互补不重复，
/// 专精 SMIL 动画与引用元素：a `<animate>`/`<set>` 修改 `href`（issue 指定原形、
/// `xlink:href` 目标、`values` 多值、`from`/`to` 组合、`begin`/`dur`/
/// `repeatCount`/`fill` 时序变体、`attributeName` 大小写混淆、动画
/// `xlink:href="#target"` 指定 `<a>`/`<image>`/`<use>` 目标、隐式父目标）；
/// b SVG `use` 外链（`external.svg#x` 跨文档、`//evil.example/x.svg#e` 协议相对、
/// `data:image/svg+xml` URL 编码与 base64、同文档 script 片段、use 嵌套 use、
/// HTML 正文裸 `use`）；c 组合与混淆（`<animate>`/`<set>` 挂 `<use>`/`<image>`/
/// `<a>`、`animateTransform`/`animateMotion`/`mpath`、SMIL `begin` 事件链、
/// `discard`、嵌套 `<svg>` 动画、未闭合吞尾）；d 惰性边界（相对 / 片段 `use`、
/// 良性 `opacity`/`x` 动画属性、协议相对锚点，元素死亡或 `url_scheme` 为 `None`
/// 且确认门拒绝）。分层：L1 melib 解析出双叶 / 三叶且全部攻击字节逐字在场
/// （LF→CRLF 容忍），`image/svg+xml` 附件归类为 `ContentDisposition::Attachment`
/// 惰性附件、其动画字节绝不进入 HTML 显示管线；L2 sanitize 输出无任何非白名单
/// SVG / SMIL 标签（含 `animateTransform`/`animateMotion`/`discard`/`mpath`）、
/// 无 `on*` / `attributeName`/`values`/`from`/`to`/`begin`/`dur`/`xlink:href`
/// 属性、无活的危险 scheme href 且为不动点；L3 render 里动画 / 引用元素整体
/// 消失、动画属性值不出现、危险 scheme 不生成脚注，惰性边界如实记录且
/// `is_default_launchable_scheme` 拒绝；L4 整封攻击邮件端到端纯文本且诚实
/// `http`/`https`/`mailto` 链接保持可用（sanitize 保留、render 脚注可见、确认门
/// 放行），`l4_no_smil_or_webmail_execution_pathway_exists` 的注释记录全仓无
/// webmail / 浏览器 / SVG 渲染器 / JS 引擎 / SMIL 动画引擎通路的映射依据。
/// 仓内孪生改动：无（免疫证明，未触碰生产代码）。
///
/// [`sanitize`]: meli::mail::view::html_render::sanitize
/// [`render`]: meli::mail::view::html_render::render
/// [`cve_2016_4068`]: self::cve_2016_4068
#[cfg(test)]
#[path = "CVE-2016-4068.rs"]
mod cve_2016_4068;

/// CVE-2024-37384（Roundcube &lt; 1.5.7 / 1.6.x &lt; 1.6.7；CVSS 6.1，NVD）偏好
/// 字段 XSS regression（issue #74，表 3 of `SECURITY-CVE-RESEARCH.zh-CN.md` —
/// 网页嵌入）：Roundcube 把用户偏好里的列表列设置原样拼进 webmail HTML 页面
/// 而不过滤，偏好字段因此成为脚本注入点。meli 无 webmail / 浏览器 / DOM / JS
/// 引擎，偏好 = TOML 配置值，「列设置」等价面 = listing 渲染配置 + terminal
/// 显示配置；TOML 值永不经过 HTML 引擎，终端攻击等价物 = 配置值携带终端转义
/// 序列（CSI / OSC / BEL / C1 / ST）落进 tty 字节流被解释成活指令。
/// [`cve_2024_37384`] 证明 listing / 鼠标标志面免疫：生产接缝
/// `CellBuffer::write_string` 把控制字符标成空格子，`draw_horizontal_segment`
/// 从不发射，剥离完整转义序列后的 tty 残渣无任何控制字节；并暴露、修复一个
/// 真缺口——`terminal.window_title` 曾被原样拼进 OSC 2 序列，标题里的 BEL/ST
/// 可提前终止 OSC 串，其后字节成为活的终端指令。修复 = 新增
/// `meli::terminal::sanitize_osc_payload`（过滤全部 `char::is_control`）+
/// `write_set_window_title`（净化后发射，空则不发射），由 `screen.rs` 仓内孪生
/// `window_title_control_bytes_cannot_break_out_of_osc2` /
/// `window_title_honest_value_is_unchanged` 锁定完整线序；诚实配置渲染不变。
///
/// [`sanitize_osc_payload`]: meli::terminal::sanitize_osc_payload
/// [`write_set_window_title`]: meli::terminal::write_set_window_title
/// [`cve_2024_37384`]: self::cve_2024_37384
#[cfg(test)]
#[path = "CVE-2024-37384.rs"]
mod cve_2024_37384;

/// CVE-2025-48700（Synacor Zimbra Collaboration Suite，Zimbra Classic UI；NVD 未分配
/// CVSS 分数；CISA KEV 2026-04-20 收录，在野利用）存储型 XSS regression（issue #75，
/// 表 3 of `SECURITY-CVE-RESEARCH.zh-CN.md` — 网页嵌入 / web/HTML embedding）：
/// KEV 原文记录攻击者可在受害会话内执行任意 JavaScript、可能触达敏感信息。
/// 与同族 CVE-2025-66376（CSS `@import` 标签切分 mXSS）互补，本语料聚焦 issue #75
/// 指定的**解析差异 / mXSS** 三大类：(1) math / `annotation-xml` 命名空间混淆；
/// (2) HTML / SVG / MathML 命名空间 `style` 文本重组；(3) 属性引号差异
/// （无引号 / 单引号 / 双引号 / 反引号 / 缺右引号吞尾 / 引号内 tab-LF-CR）。
///
/// meli 等价面映射（issue #75 明确要求）：meli 是终端邮件客户端，**没有 webmail
/// 功能面**——没有 HTTP 页面、会话 Cookie、浏览器 DOM、JavaScript 引擎，也没有把
/// 邮件 HTML 注入页面的服务端清洗器。邮件 HTML 唯一的消费通路是邮件视图的显示
/// 管线 [`sanitize`]（ammonia 白名单：html5ever **恰好一次**解析 → 过滤树 → 带转义
/// 再序列化）→ [`render`]（html2text → 纯终端文本）。mXSS 的成立前提是"清洗后的
/// 字符串被第二次交给 HTML 解析器"，而 meli 只解析一次、输出面向纯文本，攻击链在
/// 结构上即断：ammonia 的 `check_expected_namespace` 对 HTML/SVG/MathML 命名空间
/// 切换做白名单检查，不合法的切换连整棵子树删除；`script`/`style` 属
/// `clean_content_tags` 连内容整体删除；`tag_attributes` 整体替换只留
/// `a[href]`/`a[title]` 与通用 `{lang,title}`；`url_schemes` 只有 http/https/mailto，
/// `href` 经 `attribute_filter` trim 首尾不可见填充后**重新**校验（CVE-2025-66376
/// 时代硬化）；序列化时属性值内的 `&`/`"`/`<`/`>` 被转义，引号差异无法再生标签边界。
///
/// [`cve_2025_48700`] 以内嵌 `multipart/alternative` 语料（plain + html 双叶）
/// 逐层锁定为**免疫证明，未发现缺口，无需改动生产代码**。语料与 CVE-2025-66376
/// `TAG_SPLITTING_CORPUS` 的 12 个 payload **逐字不重复**（同族不同形），三大类各
/// ≥ 6、总数 45 个向量，每个向量都过独立惰性预言机（`scan_tags` 白名单扫描 +
/// 无 `on*`/禁止属性 + 无活的危险 scheme href + `sanitize` 不动点）。分层：L1 melib
/// 解析出双叶且全部攻击字节逐字在场（LF→CRLF 容忍），并自检三大类规模与命名唯一性；
/// L2 sanitize 输出无任何非白名单 math/SVG/style 标签、无 `on*`、无活的危险 scheme
/// href 且为不动点；L3 render 里 script/style 脚本体与危险 scheme 脚注整体消失，
/// 引号内嵌标记只解出惰性字面文本，惰性边界的 `url_scheme` 为 `None` 且确认门拒绝；
/// L4 整封攻击邮件端到端纯文本且诚实 `http`/`https`/`mailto` 链接保持可用
/// （sanitize 保留、render 脚注可见、`is_default_launchable_scheme` 放行），
/// `l4_no_webmail_script_pathway_exists` 的注释记录全仓无 webmail / 浏览器 / JS 引擎 /
/// 第二次 HTML 解析通路的映射依据。仓内孪生改动：无（免疫证明，未触碰生产代码）。
///
/// [`sanitize`]: meli::mail::view::html_render::sanitize
/// [`render`]: meli::mail::view::html_render::render
/// [`cve_2025_48700`]: self::cve_2025_48700
#[cfg(test)]
#[path = "CVE-2025-48700.rs"]
mod cve_2025_48700;

/// CVE-2026-73572（Synacor Zimbra Collaboration Suite，Zimbra Classic UI；NVD 未分配
/// CVSS 分数；来源 Zimbra 官方安全公告）**存储型 XSS** regression（issue #76，
/// 表 3 of `SECURITY-CVE-RESEARCH.zh-CN.md` 第 127 行 — 网页嵌入 / web/HTML
/// embedding）：报告原文记录攻击者在受害者预览恶意邮件附件时让附件内容执行脚本。
/// 与同族 CVE-2025-48700（正文 mXSS）、CVE-2025-66376（CSS `@import` 标签切分）互补，
/// 本语料聚焦 issue #76 指定的**附件预览面**四类：(1) `text/html` 附件带 `<script>` /
/// 事件属性 / `javascript:` / `data:` / `document.cookie` 窃取语义；(2)
/// `image/svg+xml` 附件带 `<script>` / `onload` / `<foreignObject>` / `xlink:href` /
/// SMIL `<animate>`；(3) inline `Other`/`OctetStream` 原样文本载体（含 ESC/CSI/OSC/
/// BEL/C1 终端控制序列）；(4) `multipart/mixed`、`multipart/related` 嵌套与
/// Content-Type 混写伪装（`image/svg+xml` 带 charset、附件名伪装 `.txt`/`.jpg`/`.png`）。
///
/// meli 等价面映射（issue #76 明确要求）：meli 是终端邮件客户端，**没有 webmail
/// 功能面**——没有 HTTP 页面、会话 Cookie、浏览器 DOM、JavaScript / SMIL 引擎，也
/// 没有把附件内容注入页面的预览器。附件消费路径逐环检查：`Content-Disposition:
/// attachment` 只以元数据进列表（`meli/src/mail/view/envelope.rs:321`）；inline
/// `text/html` 经 `ViewFilter::new_html`（`filters.rs:146`）→ [`sanitize`]（ammonia
/// 白名单恰好一次解析 → 过滤树 → 带转义序列化）→ [`render`]（html2text → 纯文本）；
/// inline `Other`/`OctetStream` 走原样纯文本（`filters.rs:405`），控制字符在网格层
/// 被 `CellBuffer::write_string`（`terminal/cells.rs:715`）标成空格子、发射器
/// `Screen::draw_horizontal_segment`（`screen.rs:582`）只在 `!c.empty()` 时写字符；
/// `image/svg+xml` 属 `ContentType::Other`，要么只显示元数据，要么在用户显式输入
/// 附件编号 + 按键后（`envelope.rs:2029` `open_attachment` / `envelope.rs:1858`
/// `open_mailcap` 均要求 `context.cmd_buf().is_some()`）交给本地桌面默认程序
/// （`envelope.rs:2072` `query_default_app` + `sh -c`），meli 自身从不解析 SVG。
///
/// 缺口检测结论：`query_default_app(attachment.mime_type())` 只把邮件可控 MIME
/// 字符串当查询键做相等比较（`melib/src/utils/xdg/mod.rs:239`），不拼接 shell；
/// `File::create_temp_file` 对邮件可控 filename 先 `sanitize_filename` 再截断
/// （`meli/src/types/helpers.rs:85`）；inline 原样文本分支的控制序列在网格层被
/// 消灭——本语料 `l2_inline_other_terminal_controls_never_reach_the_grid` 直接断言
/// 网格里 `cell.empty() || !cell.ch().is_control()`。**免疫证明，未发现缺口，无需
/// 改动生产代码**。
///
/// [`cve_2026_73572`] 以内嵌 `multipart/mixed` 附件语料逐层锁定：L1 melib 解析出
/// 多层附件树（HTML attachment 叶 / SVG attachment 叶 / ≥ 8 个 inline
/// Other/OctetStream 叶 / 嵌套 related 与 mixed），四类各 ≥ 6、总数 30 个向量逐字
/// 在场（LF→CRLF 容忍），并自检规模、命名唯一性与同族参考语料独有片段的逐字不重复；
/// L2 sanitize 输出无任何非白名单标签、无 `on*`、无活的危险 scheme href 且为不动点，
/// render 纯终端文本，inline 控制序列不进入网格；L3 `url_scheme` /
/// `is_default_launchable_scheme` 对 `javascript:`/`vbscript:`/`data:`/`file:`/`blob:`
/// 拒绝免确认启动、对 `http`/`https`/`mailto` 放行（诚实对照）；L4 整封攻击邮件逐
/// HTML 附件叶端到端纯文本且诚实链接保持可用，
/// `l4_no_webmail_or_svg_engine_pathway_exists` 的注释记录全仓无 webmail / 浏览器 /
/// SVG 渲染器 / JS / SMIL 引擎 / 第二次 HTML 解析通路的映射依据。仓内孪生改动：无
/// （免疫证明，未触碰生产代码）。
///
/// [`sanitize`]: meli::mail::view::html_render::sanitize
/// [`render`]: meli::mail::view::html_render::render
/// [`cve_2026_73572`]: self::cve_2026_73572
#[cfg(test)]
#[path = "CVE-2026-73572.rs"]
mod cve_2026_73572;

/// CVE-2021-29956（Mozilla Thunderbird 78.8.1–78.10.1；CVSS 4.3，CWE-312）**导入
/// 的 OpenPGP 私钥在未设置主密码时明文写入本地 keystore** regression（issue
/// #102）。Thunderbird 把用户导入的私钥存进自有 `key4.db`/secring 式 keystore，
/// 无 primary password 时不加口令保护，任何能读本地文件的攻击者扫描其
/// profile/数据目录即可原样窃取；投递形态是一封携带
/// `-----BEGIN PGP PRIVATE KEY BLOCK-----` 的「请导入新私钥」邮件。
///
/// meli 等价面映射：meli **没有私有 keystore**——唯一导入 API 是
/// [`Context::import_key`]（gpgme `gpgme_op_import`），私钥直接进 GnuPG 自己的
/// `GNUPGHOME/private-keys-v1.d/`，由 gpg-agent 掌管口令保护域；meli 生产代码
/// 没有任何 `import_key` 调用点（调用点只在 `#[cfg(test)]` 测试里），没有把
/// `application/pgp-keys` 解释成导入动作的分发（它落 `ContentType::Other` 不透明
/// 载荷），gpgme bindings 无 `gpgme_op_export`，[`Key`] 仅有
/// `secret(&self) -> bool` 布尔访问器、绝不返回字节。
///
/// [`cve_2021_29956`] 分层锁定为**免疫证明，未发现缺口，无需改动生产代码**：
/// L1 用 `Envelope::from_bytes` + 附件树解析攻击邮件（内联 armor + 真实
/// `application/pgp-keys` 附件），证明私钥字节只以不透明附件/正文形式可达；L2
/// 源码扫描证明无导入入口、无导出 API、Key 只有布尔句柄；L3 用真 gpgme 把攻击
/// 邮件里提取的私钥经唯一导入 API 导入，再递归扫描 scratch `XDG_CONFIG_HOME`/
/// `XDG_DATA_HOME`/`HOME`/cwd `log`/`<tmp>/meli` 证明私钥特征字节全空，阳性对照
/// 证明同一 needle 只落在 scratch `GNUPGHOME` 的 gpg-agent 域；L3d 证明 gpgme I/O
/// 回调只记录事件、不 dump 数据缓冲。草稿面依 CVE-2008-4491 的策略面记录（正文
/// armor 只能经用户显式动作进入草稿，属用户内容）。
///
/// [`Context::import_key`]: meli::melib::gpgme::Context::import_key
/// [`Key`]: meli::melib::gpgme::Key
/// [`cve_2021_29956`]: self::cve_2021_29956
#[cfg(test)]
#[path = "CVE-2021-29956.rs"]
mod cve_2021_29956;

/// CVE-2021-30858（WebKit；NVD 未分配 CVSS 分数；Apple 2021-09-13 紧急修复
/// iOS 14.8 / iPadOS 14.8 / Safari 14.1.2 等，WebKitGTK 同步收录 WSA-2021-0020；
/// Apple 确认在野利用，匿名研究者报告）**use-after-free → 任意代码执行**
/// regression（issue #77，表 4 of `SECURITY-CVE-RESEARCH.zh-CN.md` — 网页嵌入 /
/// web/HTML embedding）：Apple 公告原文 *"A use after free issue was addressed
/// with improved memory management"*，攻击面是 Apple Mail 的 HTML 渲染引擎，
/// 邮件 HTML 即投递面。与同族 CVE-2025-48700（正文 mXSS）、CVE-2015-8864（SVG
/// 脚本 / 事件）、CVE-2026-73572（附件预览存储型 XSS）、CVE-2025-66376
/// `TAG_SPLITTING_CORPUS`（CSS `@import` 标签切分）逐字不重复，本语料聚焦 issue
/// #77 指定的 UAF 触发型两类面：(1) DOM 操作差异——深度嵌套 / 错配嵌套、active
/// formatting elements 重建与 adoption agency、table foster parenting、
/// `form`/`template` 交互、`p`-in-body 自动闭合、raw text 吞尾；(2) 命名空间
/// 混淆——SVG / MathML foreign content、`annotation-xml`/`mtext`/`mglyph` 积分点、
/// `foreignObject`、CDATA / 注释切分、大小写与实体混淆。
///
/// meli 等价面映射（issue #77 明确要求）：meli 是终端邮件客户端，**没有 WebKit /
/// 浏览器 DOM / JavaScript 引擎 / webmail 页面**。邮件 HTML 唯一的消费通路是
/// 邮件视图的显示管线 [`sanitize`]（ammonia 白名单：html5ever **恰好一次**规范
/// 解析 → 过滤树 → 带转义再序列化）→ [`render`]（html2text → 纯终端文本）。
/// UAF→RCE 在结构上不可表达（安全 Rust、无内存不安全宿主对象、`script`/`style`
/// 连内容删除、`svg`/`math`/`foreignObject` 非白名单、`on*` 随元素死亡、
/// `url_schemes` 只有 http/https/mailto 且 `href` 经 trim 后重新校验）；唯一的
/// 内存安全等价面是无界递归（CWE-674），因为 Rust 无法 catch 栈溢出。
///
/// [`cve_2021_30858`] 以内嵌 `multipart/alternative` 语料（plain + html 双叶）
/// 逐层锁定为**免疫证明，未发现缺口，无需改动生产代码**。语料两大类各 ≥ 6、
/// 总数 46 个向量（DOM 差异 24 + 命名空间混淆 22），每个向量都过独立惰性预言机
/// （`scan_tags` 白名单扫描 + 无 `on*`/危险 scheme + `sanitize` 不动点）。分层：
/// L1 melib 解析出双叶且全部攻击字节逐字在场（LF→CRLF 容忍），并自检两类规模、
/// 命名唯一性与同族参考语料长片段的逐字不重复；L2 sanitize 输出无任何非白名单
/// script/style/SVG/MathML/form/select/template 标签、无 `on*`、无活的危险 scheme
/// href 且为不动点；L3 render 脚本体 / 事件属性值 / 危险 scheme 目标整体消失，
/// 不生成危险链接脚注，惰性边界 `url_scheme` 为 `None` 且
/// `is_default_launchable_scheme` 拒绝；L4 整封攻击邮件端到端纯文本且诚实
/// `http`/`https`/`mailto` 链接保持可用（sanitize 保留、render 脚注可见、确认门
/// 放行）。重点探针
/// `probe_sanitize_and_render_survive_deep_nesting_on_strict_stack` 在
/// `stack_size(2 MiB)` 线程（对齐 meli view 线程栈，同 issue #23 的 2 MiB 说明）
/// 上对 50000 层未闭合 `<b>` 链、`<table><tr><td>` 链、混合错配链与 balanced
/// `<div>` 移除链（后者只跑 `render`，其第一步即 `sanitize`），另加为限制 CI 时间
/// 收窄到 10000 层的 adoption agency 错配链跑 sanitize + render，全部存活、
/// 不 panic / 不栈溢出、输出纯文本。仓内孪生改动：无（免疫证明，未触碰生产代码）。
///
/// [`sanitize`]: meli::mail::view::html_render::sanitize
/// [`render`]: meli::mail::view::html_render::render
/// [`cve_2021_30858`]: self::cve_2021_30858
#[cfg(test)]
#[path = "CVE-2021-30858.rs"]
mod cve_2021_30858;

/// CVE-2023-4863（libwebp；CVSS 8.8；2023-09 在野利用；Chrome / Firefox /
/// Thunderbird 共用同一份 libwebp）**VP8L 无损位流 Huffman 编码表构建堆缓冲区
/// 溢出** regression（issue #78，表 4 of `SECURITY-CVE-RESEARCH.zh-CN.md` —
/// e-mail-reachable browser engines）：`ReadHuffmanCodes` /
/// `BuildHuffmanTable` 对 attacker 控制的 code-length 符号表写出越界，含
/// WebP 图片的邮件只要被打开就在进程内解码并触发。meli 是终端邮件客户端，
/// **没有浏览器引擎、没有 DOM / JS、也没有任何内嵌图片解码器**，本 CVE 的
/// 触发原语在结构上不存在。
///
/// [`cve_2023_4863`] 以内嵌「等价结构」恶意 WebP 语料（真 `RIFF` + 小端 size +
/// `WEBP` 魔数的 VP8 / VP8L / VP8X+ALPH+ANIM+ANMF 容器，与截断畸形变体，共
/// 20+ 向量；投递形态覆盖 `Content-Disposition: attachment` 叶、inline
/// `image/webp`、伪装 `application/octet-stream` + `.webp` 文件名、
/// `multipart/related` + HTML `<img src="cid:…">` 引用、嵌套
/// `message/rfc822`）逐层锁定为**免疫证明，未发现缺口，未触碰生产代码**。
/// L1 melib 解析出精确的 `multipart/mixed` 附件树，全部语料字节在 wire 上逐字
/// 在场，`image/webp` 归类 `ContentType::Other { tag: b"image/webp" }`、
/// `is_text() == false`，伪装 `application/octet-stream` + `.webp` 文件名按
/// Content-Type 归类（`ContentType::Other { tag: b"application/octet-stream" }`，
/// 不看扩展名；`ContentType::OctetStream` 是程序化构造形），且
/// [`Attachment::decode`] 只做 base64 / quoted-printable 传输编码反转、其余字节
/// 原样返回，无任何 WebP 结构解释；L2 WebP 叶进入 `AttachmentDisplay::Attachment`
/// 元数据条目（文件名 / 大小 / MIME），`multipart/related` 里的 `cid:` / `data:`
/// / 远程 WebP 引用在 [`sanitize`] 里整体死亡（`img`/`object`/`embed`/`iframe`/
/// `source` 非白名单，scheme 只有 http/https/mailto），[`render`] 输出纯终端文本；
/// L3 打开 WebP 只经 `open_mailcap` / `open_attachment` 两条汇，两者都以
/// `context.cmd_buf().is_some()`（先输入附件编号）为前置，命中后字节只落入
/// `<temp_dir>/meli/` 的 `0o600` 随机名临时文件并交给进程外程序，语料同时断言
/// 整个 workspace 清单无任何内嵌图片解码依赖（`l3_workspace_has_no_in_process_
/// image_decoder_dependency`）；L4 整封攻击邮件端到端解析 + 渲染纯文本，合法
/// 内容交付、WebP 字节 / 引用不出现，诚实 `http`/`https`/`mailto` 链接保持可用。
/// WebP 门禁的仓内孪生回归为 `meli/src/mail/view/tests.rs` 的
/// `cve_2023_4863_webp_attachments_render_as_metadata_only` 与
/// `cve_2023_4863_open_attachment_requires_explicit_attachment_number`。
///
/// [`Attachment::decode`]: meli::melib::Attachment::decode
/// [`sanitize`]: meli::mail::view::html_render::sanitize
/// [`render`]: meli::mail::view::html_render::render
/// [`cve_2023_4863`]: self::cve_2023_4863
#[cfg(test)]
#[path = "CVE-2023-4863.rs"]
mod cve_2023_4863;

/// CVE-2023-41061（Apple Wallet / PassKit；Apple 公告原文 *"A validation issue
/// was addressed with improved logic"*；2023-09 在野利用；与 CVE-2023-41064 组成
/// BLASTPASS 零点击链，经 iMessage 附件投递，Citizen Lab / Lookout 归因）**附件
/// 校验绕过 → 任意代码执行** regression（issue #79，表 4 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md`）：iOS/iPadOS/watchOS 的 Wallet 对伪造 Apple
/// Wallet `.pass` 附件的校验被绕过——`.pass` 本质是一个 ZIP 归档（`pass.json` /
/// `manifest.json` / `logo.png` / `zh-CN.lproj/pass.strings` 等条目），Wallet 在
/// 检查 SHA1 清单与结构约束时逻辑不完善，恶意 `.pass` 被当作合法凭证处理；链上
/// 再配合 CVE-2023-41064（ImageIO 处理 `.pass` 内嵌恶意 PNG 的缓冲区溢出）完成
/// 代码执行。攻击面对邮件客户端附件处理同等适用。
///
/// meli 等价面映射（issue #79 明确要求）：meli 是终端邮件客户端，**没有 Wallet /
/// PassKit 解析器、没有 ZIP 归档解析器、也没有任何内嵌图片解码器**。本 CVE 的触发
/// 原语在结构上不存在。`Content-Disposition: attachment` 叶只以元数据进列表；
/// `.pass` / `application/zip` / `application/octet-stream` 在 melib 解析路径
/// 按 Content-Type 归为 `ContentType::Other { tag }`、`is_text() == false`（不看
/// 扩展名），[`Attachment::decode`] 只做 base64 / quoted-printable / 8bit 传输编码
/// 反转，不做任何 ZIP 记录头 / EOCD / PassKit / PNG 结构解释；inline `Other` 原样
/// 文本分支只把已到达字节当纯文本；`multipart/related` 的 `<img src="cid:…">` /
/// `data:application/vnd.apple.pass;…` / 远程 `https://evil79.example/x.pass` /
/// `<object>` / `<embed>` / `<iframe>` 引用在 [`sanitize`]（ammonia 白名单，`img` /
/// `object` / `embed` / `iframe` / `source` 非白名单，URL scheme 只有
/// http/https/mailto）里整体死亡；打开 `.pass` 只有 `open_mailcap` 与
/// `open_attachment` 两条汇，两者都以 `context.cmd_buf().is_some()`（先输入附件
/// 编号）为前置，命中后字节经 `sanitize_filename` 压平路径穿越再落入
/// `<temp_dir>/meli/` 的 `0o600` 随机名临时文件、交给进程外程序。
///
/// [`cve_2023_41061`] 以内嵌「等价结构」伪造 `.pass` 语料（真 `PK\x03\x04` 本地
/// 文件头 + `PK\x01\x02` 中央目录 + `PK\x05\x06` EOCD 的 stored 容器，逐条内嵌
/// 恶意 `pass.json`（http `webServiceURL` 信标 / `backFields` script 事件载荷 /
/// `authenticationToken` / 畸形 barcodes）、伪造 40 位十六进制 SHA1 的
/// `manifest.json`、IHDR 声明尺寸与实际数据失配的畸形 PNG、`zh-CN.lproj/
/// pass.strings` 本地化嵌套目录；外加 `PK\x06\x07` ZIP64 EOCD locator 混淆与
/// 截断 / 篡改 EOCD 变体，共 18 个 ZIP 向量；投递形态覆盖正确申报
/// `application/vnd.apple.pass`、伪装 `application/zip` 与
/// `application/octet-stream` + `.pass` 文件名、RFC 2231 `filename*=UTF-8''…%2Epass`
/// 与 `filename*0=`/`filename*1=` 分段拼写、inline vs attachment、双扩展
/// `boarding79.pass.txt`、`text/plain` 伪装、路径穿越 `../../evil79.pass`、
/// `multipart/related` + `<img src="cid:pass79@evil79.example">`、嵌套
/// `message/rfc822`，共 11 个投递向量；另有 7 个 HTML 引用向量）逐层锁定为
/// **免疫证明，未发现缺口，未触碰生产代码**。
///
/// L1 melib 解析出精确的 `multipart/mixed` 附件树（19 个根部件），全部语料字节在
/// wire 上逐字在场，`application/vnd.apple.pass` / `application/zip` /
/// `application/octet-stream` 归类 `ContentType::Other`、`is_text() == false`，
/// [`Attachment::decode`] 只做传输编码反转（8bit 原样、base64 `UEsDBA==` →
/// `PK\x03\x04`、quoted-printable 同已知答案），无任何 ZIP / PassKit / PNG 结构
/// 解释；L2 `.pass` 叶只以元数据条目显示，HTML 引用在 [`sanitize`] 里整体死亡且为
/// 不动点，inline `Other` 原样分支惰性；L3 源码扫描锁定 `open_mailcap` /
/// `open_attachment` 的 `cmd_buf().is_some()` 前置，路径穿越文件名经
/// `sanitize_filename` / `sanitize_filename_component` 不产生目录分量，
/// `l3_workspace_has_no_zip_or_archive_parser_dependency` 断言全仓清单无任何
/// ZIP / 归档解析依赖，`l3_flate2_is_only_used_outside_attachment_paths` 断言
/// `flate2` 的使用点仅为协议连接层（`melib/src/utils/connections.rs`）、本地
/// manpage 资产（`meli/src/manpages.rs`）、可选补丁抓取工具
///（`melib/src/utils/patch_retrieve.rs`）与测试代码（`meli/src/sqlite3/tests.rs`），
/// `melib/src/email/` 下零引用；L4 整封攻击邮件端到端解析 + 渲染纯文本，合法内容
/// （`LEGITIMATE-MARKER-79`）交付、`.pass` 字节 / 引用不出现，诚实
/// `http`/`https`/`mailto` 链接保持可用（sanitize 保留 href、render 脚注可见、
/// 确认门放行）。仓内孪生改动：无（免疫证明，未触碰生产代码）。
///
/// [`Attachment::decode`]: meli::melib::Attachment::decode
/// [`sanitize`]: meli::mail::view::html_render::sanitize
/// [`cve_2023_41061`]: self::cve_2023_41061
#[cfg(test)]
#[path = "CVE-2023-41061.rs"]
mod cve_2023_41061;

/// CVE-2023-41064（Apple ImageIO；Apple 公告原文 *"A buffer overflow issue was
/// addressed with improved memory management"*；2023-09 在野利用；与
/// CVE-2023-41061 组成 BLASTPASS 零点击链，经 iMessage 附件投递，Citizen Lab /
/// Lookout 披露，修复于 iOS 16.6.1）**图片缓冲区溢出 → 任意代码执行** regression
/// （issue #80，表 4 of `SECURITY-CVE-RESEARCH.zh-CN.md` — e-mail-reachable
/// browser engines）：ImageIO 解析恶意图片时对**元数据头**缺乏边界检查——
/// IFD0 偏移 / entry count / 声明的 ImageWidth / ImageLength / StripOffsets /
/// StripByteCounts / EXIF IFD 指针都可被声明为远超实际数据容量的值，越界读 /
/// 写破坏堆内存；BLASTPASS 链中触发点正是 `.pass` 归档内嵌的 TIFF 形态图片。
///
/// meli 等价面映射（issue #80 明确要求）：meli 是终端邮件客户端，**没有浏览器
/// 引擎、没有 ImageIO、也没有任何内嵌图片解码器**。本 CVE 的触发原语在结构上
/// 不存在。`Content-Disposition: attachment` 叶只以元数据进列表；`image/tiff` /
/// `image/heic` / `image/heif` / `image/jpeg` 与伪装成
/// `application/octet-stream` 的图片部件在 melib 解析路径按 Content-Type 归为
/// `ContentType::Other { tag }`、`is_text() == false`（不看扩展名），
/// [`Attachment::decode`] 只做 base64 / quoted-printable / 8bit 传输编码反转，
/// 不做任何 TIFF IFD / ISO-BMFF box / JPEG 段结构解释；inline `Other` 原样文本
/// 分支只把已到达字节当纯文本；`multipart/related` 的 `<img src="cid:…">` /
/// `data:image/tiff|heic|jpeg;…` / 远程 `https://evil80.example/x.tiff` /
/// `<object>` / `<embed>` / `<iframe>` / `<picture><source>` / `data:` 锚点引用在
/// [`sanitize`]（ammonia 白名单，`img` / `object` / `embed` / `iframe` / `source`
/// 非白名单，URL scheme 只有 http/https/mailto）里整体死亡；打开图片只有
/// `open_mailcap`（`meli/src/mail/view/envelope.rs:1859`）与 `open_attachment`
/// （同文件 `:2030`）两条汇，两者都以 `context.cmd_buf().is_some()`（先输入附件
/// 编号）为前置，命中后字节经 `sanitize_filename` 压平路径穿越再落入
/// `<temp_dir>/meli/` 的 `0o600` 随机名临时文件、交给进程外程序。
///
/// [`cve_2023_41064`] 以内嵌「等价结构」畸形图片语料（真 `II*\0` / `MM\0*` TIFF
/// 魔数 + IFD 记录；真 ISO-BMFF `ftyp` + `meta` + `iloc`/`iinf`/`infe`/`iprp`/
/// `ipco` 子盒；真 JPEG `FFD8` SOI + APP0/APP1/SOF0/SOF2/DQT/DHT/SOS 段，逐族
/// 内嵌「声明尺寸 / 偏移 / 长度远超实际数据」的越界元数据头形态）逐层锁定为
/// **免疫证明，未发现缺口，未触碰生产代码**。语料共 26 个向量：TIFF 8（双字节序、
/// IFD0 偏移 0xFFFFFFFF、entry count 0xFFFF、ImageWidth/ImageLength=0xFFFFFFFF、
/// StripOffsets/StripByteCounts 越界、EXIF IFD 指针越界（BLASTPASS 等价形态）、
/// IFD 头后截断、IFD 链自环）、HEIF/HEIC 9（`iloc` v2 extent u64 越界、
/// construction_method=1 指向 `idat`、`meta` size=0、size=1 + 64 位 largesize
/// 越界、`iinf` entry_count 0xFFFF、`infe` 截断、`pitm` 指向不存在条目、
/// `meta`+`mdat`、`ftyp` 后截断）、JPEG 9（APP0/JFIF 长度失配、APP1/Exif 内嵌
/// 越界 TIFF 头、SOF0/SOF2 声明 0xFFFF×0xFFFF、DQT/DHT 符号计数失配、SOS 前
/// 截断、缺 EOI、SOS 后无扫描数据）；投递形态 16（正确申报四种图片 MIME、伪装
/// `application/octet-stream` + `.tiff`/`.heic`/`.jpg` 文件名、base64 / quoted-printable
/// 传输编码、RFC 2231 `filename*=UTF-8''board80%2Etiff` 与 `filename*0=`/
/// `filename*1=` 分段拼写、inline vs attachment、双扩展 `board80.tiff.txt`、
/// `text/plain` 伪装、路径穿越 `../../evil80.tiff`、`multipart/related` +
/// `<img src="cid:image80@evil80.example">`、嵌套 `message/rfc822`）；另有 10 个
/// HTML 引用向量。所有语料字节均以 8bit 形式逐字上线（固定投递部件 +
/// `corpus-*.tiff/.heic/.jpg` 附件）。
///
/// L1 melib 解析出精确的 `multipart/mixed` 附件树（`16 + 其余向量` 个根部件），
/// 全部语料字节在 wire 上逐字在场，四种图片 MIME 归类 `ContentType::Other`、
/// `is_text() == false`，base64 / quoted-printable 部件只做传输编码反转（已知答案
/// `SUkqAA==` → `II*\0`）；L2 图片叶只以元数据条目显示，HTML 引用在 [`sanitize`]
/// 里整体死亡且为不动点，inline `Other` 原样分支惰性；L3 源码扫描锁定
/// `open_mailcap` / `open_attachment` 的 `cmd_buf().is_some()` 前置，路径穿越
/// 文件名经 `sanitize_filename` / `sanitize_filename_component` 不产生目录分量，
/// `l3_workspace_has_no_in_process_image_decoder_dependency` 断言全仓清单无任何
/// 内嵌图片解码依赖（`image` / `tiff` / `libheif` / `jpeg` / `png` / `gif` /
/// `webp` / exif 类 crate 全查）；L4 整封攻击邮件端到端解析 + 渲染纯文本，合法
/// 内容（`LEGITIMATE-MARKER-80`）交付、图片字节 / 引用不出现，诚实
/// `http`/`https`/`mailto` 链接保持可用（sanitize 保留 href、render 脚注可见、
/// 确认门放行）。图片类附件显示 / 开门禁的仓内孪生回归为
/// `meli/src/mail/view/tests.rs` 的
/// `cve_2023_4863_webp_attachments_render_as_metadata_only` 与
/// `cve_2023_4863_open_attachment_requires_explicit_attachment_number`。仓内孪生
/// 改动：无（免疫证明，未触碰生产代码）。
///
/// [`Attachment::decode`]: meli::melib::Attachment::decode
/// [`sanitize`]: meli::mail::view::html_render::sanitize
/// [`cve_2023_41064`]: self::cve_2023_41064
#[cfg(test)]
#[path = "CVE-2023-41064.rs"]
mod cve_2023_41064;

/// CVE-2026-8091（Firefox / Thunderbird 150；CVSS 9.8；MFSA 2026-45 批次；Gecko
/// 引擎共享同一修复）**Audio/Video: Playback 组件边界条件错误（incorrect boundary
/// conditions）→ 任意代码执行** regression（issue #81，表 4 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — e-mail-reachable browser engines）：Gecko
/// 的音视频回放栈在解析 / 解复用媒体容器（MP4 / ISO-BMFF、WebM / Matroska、
/// Ogg、WAV、MP3）时，对容器声明的边界——box / element / chunk / page / frame 的
/// 长度与计数——缺乏一致校验：size=0 延伸到 EOF 的 box、size=1 + 64 位 largesize
/// 的越界声明、采样表里远超实际数据的 entry_count、EBML VINT 越界长度、Ogg
/// segment lacing 与数据失配、RIFF 块长度撒谎、MP3 帧头 bitrate / sampling 花招，
/// 任一即可让解码侧缓冲区边界计算失配并越界读写。攻击面正是**邮件内嵌 HTML5
/// 音视频**：`<video>` / `<audio>` / `<source>` / `<track>` / `<picture>` 及
/// `on*` 事件属性一旦被引擎渲染，media element 就在进程内解复用 / 解码附件字节。
///
/// meli 等价面映射（issue #81 明确要求）：meli 是终端邮件客户端，**没有浏览器
/// 引擎、没有 DOM / JS / media element、也没有任何内嵌音视频解码器 / 解复用器**。
/// 本 CVE 的触发原语在结构上不存在。`video/mp4` / `video/webm` / `audio/ogg` /
/// `audio/webm` / `audio/mpeg` / `audio/wav` / `audio/x-wav` 与伪装成
/// `application/octet-stream` 的媒体部件在 melib 解析路径按 Content-Type 归为
/// `ContentType::Other { tag }`、`is_text() == false`（不看扩展名），
/// [`Attachment::decode`] 只做 base64 / quoted-printable / 8bit 传输编码反转，
/// 不做任何 ISO-BMFF box / EBML element / Ogg page / RIFF chunk / MPEG frame
/// 结构解释；inline `Other` 原样文本分支只把已到达字节当纯文本；`multipart/related`
/// 的 `<video src="cid:…">` / `<video><source src>` / `<audio controls autoplay>` /
/// `<track>` / `<picture><source srcset>` / `data:video|audio;…` / 远程
/// `https://evil81.example/x.mp4` / `blob:` / `file:` / `onerror`/`onload`/
/// `oncanplay`/`onloadeddata` 在 [`sanitize`]（ammonia 白名单，标签集只有文本 /
/// 结构元素——`video`/`audio`/`source`/`track`/`picture` 一概非白名单；属性白名单
/// 只有 `a` 的 href/title；`url_schemes` 仅 http/https/mailto）里整体死亡；打开
/// 媒体只有 `open_mailcap`（`meli/src/mail/view/envelope.rs:1859`）与
/// `open_attachment`（同文件 `:2030`）两条汇，两者都以 `context.cmd_buf().is_some()`
/// （先输入附件编号）为前置，命中后字节经 `sanitize_filename` 压平路径穿越再落入
/// `<temp_dir>/meli/` 的 `0o600` 随机名临时文件、交给进程外程序。
///
/// [`cve_2026_8091`] 以内嵌「等价结构」畸形媒体语料（真 `ftyp` + `moov`/`mdat`
/// box 树 + `stsz`/`stsc`/`stco` 采样表；真 EBML `1A45DFA3` 头 + `18538067`
/// Segment + Void/Info 元素；真 `OggS` 页头 + segment lacing 表；真
/// `RIFF`/`WAVE`/`fmt `/`data` 块；真 MPEG 帧头 + `ID3` 头，逐族内嵌「声明长度 /
/// 计数远超实际数据」的边界错误形态）逐层锁定为 **免疫证明，未发现缺口，未触碰
/// 生产代码**。语料共 27 个容器向量：MP4 8（moov size=0 到 EOF、size=1 + 64 位
/// largesize=u64::MAX、mdat size=0xFFFFFFFF、stsz sample_count=0xFFFFFFFF、stsc
/// entry_count=0xFFFFFFFF、stco chunk offset 越界、moov 中途截断、ftyp 首盒 size
/// 越界）、WebM 6（EBML 头长度越界、Segment 未知长度、Segment 长度越界、Void 长度
/// 越界、Info 长度失配、长度 VINT 标记后截断）、Ogg 5（version 非 0、lacing 总和
/// 超过数据、page_segments 越界、页头截断、lacing 继续位无后续页）、WAV 4
///（RIFF size 越界、fmt 块长度失配、data 块长度越界、data 头后截断）、MP3 4
///（帧头后截断、bitrate/sampling 失配、ID3v2 syncsafe size 越界、sampling 保留
/// 索引）；投递形态 21（正确申报七种媒体 MIME、伪装 `application/octet-stream` +
/// `.mp4`/`.ogg`/`.webm`/`.mp3`/`.wav` 文件名、base64 / quoted-printable 传输编码、
/// RFC 2231 `filename*=UTF-8''clip81%2Emp4` 与 `filename*0=`/`filename*1=` 分段
/// 拼写、inline vs attachment、双扩展 `clip81.mp4.txt`、`text/plain` 伪装、路径
/// 穿越 `../../evil81.mp4`、`multipart/related` + `<video poster src="cid:…">`、
/// 嵌套 `message/rfc822`）；另有 19 个 HTML5 媒体嵌入向量。所有语料字节均以 8bit
/// 形式逐字上线（固定投递部件 + `corpus-*.mp4/.ogg/.webm/.mp3/.wav` 附件）。
///
/// L1 melib 解析出精确的 `multipart/mixed` 附件树（21 + 其余向量个根部件），全部
/// 语料字节在 wire 上逐字在场，七种媒体 MIME 归类 `ContentType::Other`、
/// `is_text() == false`，base64 / quoted-printable 部件只做传输编码反转（已知答案
/// `T2dnUw==` → `OggS`、`GkXfow==` → EBML 魔数、`//s=` → MPEG 同步字）；L2 媒体
/// 叶只以元数据条目显示，HTML 媒体引用 / `on*` 事件属性在 [`sanitize`] 里整体死亡
/// 且为不动点，inline `Other` 原样分支惰性；L3 源码扫描锁定 `open_mailcap` /
/// `open_attachment` 的 `cmd_buf().is_some()` 前置，路径穿越文件名经
/// `sanitize_filename` / `sanitize_filename_component` 不产生目录分量，
/// `l3_workspace_has_no_in_process_media_decoder_dependency` 断言全仓清单无任何
/// 内嵌音视频解码 / 解复用依赖（ffmpeg / gstreamer / symphonia / rodio / lewton /
/// ogg / vorbis / minimp4 / mp4 / cpal / audiopus 等全查）；L4 整封攻击邮件端到端
/// 解析 + 渲染纯文本，合法内容（`LEGITIMATE-MARKER-81`）交付、媒体字节 / 引用不
/// 出现，诚实 http/https/mailto 链接保持可用（sanitize 保留 href、render 脚注可见、
/// 确认门放行）。媒体类附件显示 / 开门禁的仓内孪生回归沿用
/// `meli/src/mail/view/tests.rs` 的
/// `cve_2023_4863_webp_attachments_render_as_metadata_only` 与
/// `cve_2023_4863_open_attachment_requires_explicit_attachment_number`。仓内孪生
/// 改动：无（免疫证明，未触碰生产代码）。
///
/// [`Attachment::decode`]: meli::melib::Attachment::decode
/// [`sanitize`]: meli::mail::view::html_render::sanitize
/// [`render`]: meli::mail::view::html_render::render
/// [`cve_2026_8091`]: self::cve_2026_8091
#[cfg(test)]
#[path = "CVE-2026-8091.rs"]
mod cve_2026_8091;

/// CVE-2010-0249（Internet Explorer 6 / mshtml.dll；CVSS 9.3 v2；NVD）**无效指针
/// 引用（invalid pointer reference）→ 远程代码执行** regression（issue #82）：
/// Operation Aurora（极光行动）——攻击者向 Google、Adobe 等公司员工发送鱼叉邮件，
/// 正文链接指向托管 IE6 漏洞利用页的服务器（在野样本 URL 形如
/// `http://<evil-host>/a.asp?ZhangChen`）。受害者点击后，mshtml.dll 解析特制
/// HTML/JS：脚本创建 / 删除 DOM 元素留下悬空指针，再用
/// `unescape('%u0c0c%u0c0c')` 堆喷射铺到 `0x0C0C0C0C` 并跳入 shellcode。微软以
/// 2010 年 1 月紧急带外补丁 MS10-002 修复。
///
/// meli 等价面映射（issue #82）：meli 是终端邮件客户端，**没有浏览器引擎、JS
/// 引擎、DOM、ActiveX/COM 宿主**，本 CVE 的触发原语在结构上不存在。等价面 =
/// ①链接显示 / 打开路径：linkify（[`ViewOptions::convert`]）只产生显示用 `Link`
/// 值、不发起网络；打开必须是显式用户动作（URL 模式输入链接编号 + `go_to_url`），
/// 非 `http`/`https`/`mailto` 还要过确认对话框
/// （[`is_default_launchable_scheme`]）；Aurora 链接本身是普通 http URL，放行正是
/// 设计内的显式用户动作语义。②渲染路径无预取 / 无自动打开：`sanitize`/`render`
/// 永不发起网络，`url_launcher` 只出现在 `go_to_url` / `List-Unsubscribe` /
/// `List-Archive` 三个用户动作汇点。③利用页字节形态在 [`sanitize`] 里整体死亡：
/// `<script>`（unescape 堆喷射、createElement/removeChild churn、fromCharCode+eval、
/// cid/https src）、`<object classid="CLSID:…">`、`<embed>`、
/// `<iframe src="javascript:…">`、`<meta refresh>`、CSS `expression(...)`、`on*`
/// 事件属性、`javascript:`/`vbscript:`/`data:text/html` href 全部不在 ammonia
/// 白名单。
///
/// [`cve_2010_0249`] 共 19 条 HTML 利用向量 + 14 条投递形态向量（inline
/// `text/html`、attachment + `Aurora82.html`/`.htm`、伪装
/// `application/octet-stream` + `.html`、base64 / quoted-printable、RFC 2231
/// `filename*=UTF-8''Aurora82%2Ehtml` 与 `filename*0=`/`filename*1=` 分段、双扩展
/// `policy82.html.txt`、`text/plain` 伪装、路径穿越 `../../evil82.html`、
/// `multipart/related` + `<script src="cid:…">`、嵌套 `message/rfc822`），整封
/// `multipart/mixed` 攻击邮件经 melib 解析出精确附件树，全部语料字节在 wire 上
/// 逐字在场。
///
/// L1 melib 解析出精确 `multipart/mixed` 附件树（`[0]` multipart/alternative + 11
/// 个投递部件），`text/plain`/`text/html` 归类正确、`is_text()` 正确，base64 /
/// quoted-printable 只做传输编码反转（已知答案 `PHNjcmlwdD4=` → `<script>`、
/// `=3Cscript=3E` → `<script>`）；L2 每条 HTML 向量 / 完整利用页在 [`sanitize`] 里
/// 整体死亡且为不动点，[`render`] 输出纯终端文本，`%u0c0c` / CLSID / createElement
/// 标记不出现；L3 源码扫描锁定 `Command::new(url_launcher)` 只出现在三个用户动作
/// 汇点、`html_render` 无 `url_launcher`/`Command::new`/`reqwest` 引用，
/// `l3_workspace_has_no_js_or_browser_engine_dependency` 断言全仓清单无任何 JS /
/// 浏览器引擎依赖（v8 / deno_core / quickjs / boa / javascriptcore / webkit2gtk /
/// wry / tauri / servo / headless_chrome / chromiumoxide / fantoccini / thirtyfour /
/// webdriver 等全查；ammonia / html5ever 只是 HTML 解析 / 清理器，不执行脚本）；
/// L4 整封攻击邮件端到端解析 + 渲染纯文本，合法内容（`LEGITIMATE-MARKER-82`）交付、
/// 脚本 / 喷射字节不出现，诚实 http/https/mailto 链接保持可用（sanitize 保留 href、
/// render 脚注可见、门禁放行）。仓内孪生回归：`meli/src/mail/view/tests.rs` 的
/// `go_to_url_cancel_does_not_launch` 与
/// `go_to_url_non_default_scheme_requires_confirmation`。仓内孪生改动：无（免疫证明，
/// 未触碰生产代码）。
///
/// [`Attachment::decode`]: meli::melib::Attachment::decode
/// [`sanitize`]: meli::mail::view::html_render::sanitize
/// [`render`]: meli::mail::view::html_render::render
/// [`ViewOptions::convert`]: meli::mail::view::ViewOptions::convert
/// [`is_default_launchable_scheme`]: meli::mail::view::envelope::is_default_launchable_scheme
/// [`cve_2010_0249`]: self::cve_2010_0249
#[cfg(test)]
#[path = "CVE-2010-0249.rs"]
mod cve_2010_0249;

/// CVE-2007-1268（mutt ≤ 1.5.13；NVD 未分配 CVSS 分数）**GnuPG 签名状态
/// 误判** regression（issue #83，表 5 协议信任边界 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md`）：mutt 没有正确使用 `--status-fd`，
/// 多部分 OpenPGP 邮件里未签名部分的篡改无法被辨别，伪造内容继承了真实
/// 签名的已验证状态。[`cve_2007_1268`] 用一次性 Ed25519 密钥构造**真实
/// 部分签名**攻击邮件（真 detached 签名 over meli 送验字节 + 攻击者伪造
/// 的未签名兄弟部分），分层锁定 meli 的等价面：L1 信任边界
/// （`extract_unverified_signature`：恰好 2 部分 / micalg / protocol /
/// 送验字节逐字节等于被签 part 的 raw，走私与畸形容器一律 fail-closed，
/// cleartext armor 尾部走私不路由）；L2 状态解读 fail-closed
/// （`signatures_into_error`：空签名列表必须是 `Err` —— 本 issue 暴露并
/// 修复的 fail-open 缺口，CLI 后端 JSON `{"signatures":[]}` 旧版会落成
/// `SignedVerified { comment: None }`；任何一条坏状态整体 `BAD
/// signature`）；L3 真实 gpgme 引擎端到端（导入语料公钥：真签名 over
/// 被签字节 → good；篡改内容 / 拼接未签名兄弟字节 → BAD；垃圾 armor →
/// Err）；L4 显示域隔离（`Signed*` 包装节点与 pager notice 只套签名
/// 子树，仓内孪生单测在 `meli/src/mail/view/tests.rs`：
/// `partial_signature_open_scopes_signed_marker_to_signed_part`、
/// `partial_signature_filter_notice_stays_off_the_unsigned_sibling`）。
/// 结论：结构性免疫 + 一个 fail-open 缺口已修复（空签名列表 → `Err`，
/// 与 gpgme 后端一致）。
///
/// [`cve_2007_1268`]: self::cve_2007_1268
#[cfg(test)]
#[path = "CVE-2007-1268.rs"]
mod cve_2007_1268;

/// CVE-2024-49393（mutt 1.14.0–2.2.12 / neomutt ≤ 2024-04-25；CVSS 6.5）**签名未
/// 覆盖 To/Cc 收件人头** regression（issue #84，表 5 协议信任边界 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md`）：外层 RFC 5322 的 `To`/`Cc` 不在
/// `multipart/signed` 被签 part 内，中间人可把 `eve@mitm.example` 追加进 `Cc`
/// 破坏机密性；NeoMutt 的修复方向是 Memory Hole / protected header fields，
/// 而本仓库完全没有该支持，meli 的等价面是签名验证域（只认被签 part 的 raw）
/// 与信封头显示语义的隔离。[`cve_2024_49393`] 用一次性 Ed25519 密钥
/// `B2611F2F94BD8B4F282E48BCEDC8569B5D2FEE15` 构造**真实签名**机密邮件 + 两个
/// 只改外层头的 MITM 变体，分层锁定：L1 送验字节逐字节等于被签 part raw、头
/// 篡改不改变验证输入、外层头从不进入、走私收件人 part fail-closed；L2
/// `Signed*` 包装只承载自己的被签子树、`Envelope` 的 `To`/`Cc` 始终来自外层头
/// （`signatures_into_error` 的 fail-closed 已由 #83 锁定，不重复）；L3 真实
/// gpgme 端到端（原件与两个头篡改变体验证结果完全一致为 good——签名对 To/Cc
/// 毫无约束力；篡改正文/把 To 塞进被签 part → BAD；垃圾 armor → Err）；L4 显示
/// 域隔离（仓内孪生单测在 `meli/src/mail/view/tests.rs`：
/// `tampered_recipient_headers_keep_signature_notice_on_the_signed_container`、
/// `tampered_recipient_headers_are_read_from_the_outer_envelope`、
/// `signature_notice_and_header_band_have_no_intersection`）。结论：结构性免疫，
/// 未发现显示语义缺口，未触碰生产代码。
///
/// [`cve_2024_49393`]: self::cve_2024_49393
#[cfg(test)]
#[path = "CVE-2024-49393.rs"]
mod cve_2024_49393;

/// CVE-2024-49394（mutt 1.14.0–2.2.12 / neomutt ≤ 2024-04-25；CVSS 5.3）**签名未
/// 覆盖 In-Reply-To/References 线程头** regression（issue #85，表 5 协议信任
/// 边界 of `SECURITY-CVE-RESEARCH.zh-CN.md`）：NVD 原文指出外层 `In-Reply-To`
/// 不受密码学签名保护，中间人可复用一封未加密但已签名的邮件冒充原发件人；
/// 本仓库没有 Memory Hole / protected headers，meli 的等价面是**线程构建
/// （`melib/src/thread.rs`）与线程视图显示语义**。[`cve_2024_49394`] 用一次性
/// Ed25519 密钥 `E130F3621F763BCC0D402F929FB76B8416849528` 构造真实签名回复 +
/// 三个只改外层线程头的重放变体，分层锁定：L1 送验字节逐字节等于被签 part
/// raw、四种变体的 `(signed_part, signature)` 完全相同、把 `In-Reply-To` 塞进
/// 被签 part 会改变送验字节；L2 线程归属语义——genuine 挂声明的 x-root，
/// 同 Message-ID 的 V1/V3 重放不能改挂/合并线程组/重复计数/劫持
/// `node.message`，新 Message-ID 的 V2 诚实地成为挂到 y-root 的独立节点，
/// V1 先到则先见者落位（诚实局限已记录）；L3 真实 gpgme 端到端（genuine 与
/// 三个线程头变体验证结果完全一致为 good——签名对线程头毫无约束力；篡改
/// 正文/把 `In-Reply-To` 塞进被签 part → BAD；垃圾 armor → Err）；L4 显示域
/// 隔离（外层线程头解析、被签子树逐字节不变、`Thread`/`ThreadNode` 无签名
/// 字段；仓内孪生单测在 `meli/src/mail/view/thread.rs`：
/// `cve_2024_49394_replayed_signed_reply_stays_in_its_thread`、
/// `cve_2024_49394_thread_headings_carry_no_signature_status`）。结论：**一个
/// 真实缺口已修复**——同 Message-ID 重放触发 `insert` 幂等早退，首见落位
/// 不可变（`melib/src/thread.rs` 的 `insert_internal`）；其余为结构性免疫。
///
/// [`cve_2024_49394`]: self::cve_2024_49394
#[cfg(test)]
#[path = "CVE-2024-49394.rs"]
mod cve_2024_49394;

/// CVE-2024-49395（mutt / neomutt；CVSS 5.3，CWE-1230）**PGP 加密未隐藏收件人
/// key ID，密文可反推 Bcc 收件人** regression（issue #86，表 5 协议信任边界 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md`）：NVD 原文 *"In mutt and neomutt, PGP
/// encryption does not use the --hidden-recipient mode which may leak the Bcc
/// email header field by inferring from the recipients info."* OpenPGP 为每个
/// 收件人生成 PKESK 包，包体带 8 字节加密子钥 key ID；Bcc 收件人必须在加密集合
/// 里才能解密，但其 key ID 若可见，任一 To/Cc 收件人都能反查身份、推断 Bcc。
///
/// 本仓库勘查确认两个真实缺口并全部修复：
///
/// 1. **缺口 A（核心面）**：`melib/src/gpgme/mod.rs` 的 `Context::encrypt` 只设
///    `NO_ENCRYPT_TO|NO_COMPRESS|ALWAYS_TRUST`，从不设
///    `GPGME_ENCRYPT_THROW_KEYIDS`；修复为 `hidden_recipients==true` 时追加该
///    flag（gpgme 无 per-recipient hidden 选项，全量 `--throw-keyids` 是 mutt
///    `--hidden-recipient` 的最强等价面）。
/// 2. **缺口 B（外层头直泄）**：`Draft::finalise` 保留 `Bcc:` 头，SMTP
///    `mail_transaction` 把含该头的原文发给所有收件人；修复为在解析信封之后、
///    写 DATA/BDAT 之前用 `melib::smtp::strip_bcc_headers` 删除 `Bcc` 头（及折叠
///    续行），RCPT 集合仍含 Bcc（RFC 5322 §3.5）。
///
/// 触发链路：`send_draft_async`（加密分支）→ `draft_has_bcc_recipients` →
/// `encrypt_filter(..., hidden_recipients)` → `PGPBackend::encrypt` →
/// gpgme flags 追加 `GPGME_ENCRYPT_THROW_KEYIDS`。CLI 后端以
/// `HIDDEN_RECIPIENTS=1` 环境变量约定同一语义。
///
/// [`cve_2024_49395`] 用两把一次性收件人密钥（To 子钥
/// `0x09339244E8A5C674`、Bcc 子钥 `0xF64F5EA77C73C35D`）分层锁定：
/// (a) `draft_has_bcc_recipients` 有/无/仅空白三态；
/// (b) `strip_bcc_headers` 后攻击邮件不含任何 `Bcc` 头行、删除行数恰为 Bcc
/// 头+续行、其余字节逐一致，且 `Envelope::from_bytes(原文).bcc()` 非空——信封
/// RCPT 来源在 strip 之前不受影响；
/// (c) 真实 gpgme 端到端——本地 PKESK 解析器证明 `hidden_recipients=false` 时
/// 密文 key ID 集合恰为 To/Bcc 两把子钥（攻击复现），真实发送路径
/// `encrypt_filter(..., true)` 后所有 key ID 全零、不含两把子钥 ID、密文不含
/// `bcc-49395`，且隐藏密文仍能用私钥解出与输入逐字节一致的明文。
/// 结论：**两个真实缺口均已修复**，纯逻辑与真 gpgme 端到端双重回归锁定。
///
/// [`cve_2024_49395`]: self::cve_2024_49395
#[cfg(test)]
#[path = "CVE-2024-49395.rs"]
mod cve_2024_49395;

/// CVE-2009-1390（mutt 1.5.19，OpenSSL/GnuTLS；CWE-295）**TLS 服务端证书链
/// 校验不完整** regression（issue #87，表 5 协议信任边界 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md`）：NVD 原文指出，mutt 1.5.19 只要证书链中
/// **一张**证书能独立被接受（本身即信任锚，或由信任锚直接签发）就放行整条
/// 连接，未要求叶子到信任锚的整条链逐节有效，中间人可据此伪装可信服务器。
///
/// meli 的 TLS 校验全部委托 `native-tls`（Linux=OpenSSL 系统信任库 + 全链 +
/// 主机名校验）：`melib/src/imap/connection.rs`、`melib/src/nntp/connection.rs`、
/// `melib/src/smtp.rs` 三处 `TlsConnector::builder()` 都是默认校验器，只有账号
/// 配置显式打开 `danger_accept_invalid_certs` 时才追加危险开关；
/// `connector.connect(&path, socket)` 把服务器主机名交给 verifier；JMAP 的 isahc
/// 客户端（`melib/src/jmap/connection.rs`、`melib/src/jmap/eventsource.rs`）三个
/// `danger_*` 同门控；四个账号配置默认全是 `false`。workspace 无 rustls /
/// webpki，也无 `verify_callback`/`SSL_CTX`/`add_root_certificate` 等自定义校验面。
///
/// **为何单元级 mock**：`native-tls`/`openssl` 不是 melib 公开 API，`cve` 依赖
/// 只有 `meli`，issue 约束「不新增 test-only 依赖」，melib 亦无 TLS 服务器实现，
/// 故按 issue 允许的「真实证书语料 + 单元级 mock」做免疫证明。语料是 openssl
/// 离线生成的真实 ECDSA P-256 链：受信根 / 诚实中间 / 诚实叶、攻击者 Rogue CA /
/// 攻击叶、以及与受信根同 Subject DN 不同密钥的 evil twin CA / 伪造 issuer 叶；
/// 三种攻击链 A1=[攻击叶,受信根]、A2=[攻击叶,诚实中间,受信根]、
/// A3=[伪造 issuer 叶,受信根] 由测试内组装。
///
/// [`cve_2009_1390`] 分层锁定：(a) 语料真实性——诚实链 DN 逐级相连、攻击叶 SAN
/// 均为 `imap.victim.example`、A3 叶 issuer TLV 字节级等于受信根 subject TLV 而
/// 三个自签 CA 的 SPKI 互异（DN 可伪造、密钥不可），`openssl verify` 转录逐字
/// 内嵌（诚实链 `OK`；A1/A2 error 20；A3 error 7）；(b) 攻击复现——mutt 式
/// 「任一证书可独立验证即接受」谓词对诚实链与 A1/A2/A3 全部接受，证明攻击原语
/// 成立；(c) 核心断言——整链连通谓词（签名邻接表 + issuer DN TLV 相等）接受诚实
/// 链、拒绝 A1/A2/A3，即「部分可信链被拒绝」；(d) L3 源码 / manifest 扫描锁定
/// 三处 connector 的默认校验器 + 门控 danger + 主机名、账号配置默认 false、
/// 无自定义证书校验面、TLS 栈为 native-tls，以及可直达的
/// [`SmtpSecurity::default`] danger=false。
/// 结论：**免疫证明，未发现缺口，未触碰生产代码**。
///
/// [`cve_2009_1390`]: self::cve_2009_1390
/// [`SmtpSecurity::default`]: meli::melib::smtp::SmtpSecurity
#[cfg(test)]
#[path = "CVE-2009-1390.rs"]
mod cve_2009_1390;

/// CVE-2009-3765（mutt 1.5.19 / 1.5.20，OpenSSL；CWE-310，CVSS v2 6.8）**证书
/// CN 内嵌 NUL 截断主机名比较** regression（issue #88，表 5 协议信任边界 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md`）：NVD 原文指出 mutt 不正确处理 X.509
/// subject CN 中的 `'\0'`，攻击者可用**合法 CA 签发**的伪造证书 MITM 冒充任意
/// SSL 服务器；同族 CVE-2009-3766 更进一层——mutt（OpenSSL）完全不校验证书
/// CN 与主机名匹配，任意合法签发的证书即可冒充。本回归同时覆盖「链有效但名字
/// 不匹配」（3766 面）与「CN 内嵌 `\0` 截断比较绕过」（3765 面）。
///
/// meli 等价面：issue 指定的 `melib/src/utils/connections.rs` 只是传输层包装
///（`Connection::Tls` 包 `native_tls::TlsStream`，无任何 CN / 主机名逻辑）；
/// 主机名校验策略全在 native-tls（Linux=OpenSSL 的 `X509_check_host` 语义）
/// 内部，入口在三处 `connector.connect(&path, socket)`——
/// `melib/src/imap/connection.rs`（path=`server_conf.server_hostname`）、
/// `melib/src/nntp/connection.rs`、`melib/src/smtp.rs`；三处
/// `TlsConnector::builder()` 都是默认校验器，只有账号配置显式打开
/// `danger_accept_invalid_certs` 时才追加危险开关，全仓无
/// `danger_accept_invalid_hostnames`；JMAP 的 isahc 客户端
///（`melib/src/jmap/connection.rs`、`eventsource.rs`）三个 `danger_*`
///（certs/hosts/revoked）由同一账号开关门控；四个账号配置默认全是 `false`，
/// `SmtpSecurity::default` 亦然。workspace 无 rustls / webpki，也无
/// `X509_check_host`/`verify_callback`/自定义 CN 解析代码。
///
/// **为何单元级 mock**：`native-tls`/`openssl` 不是 melib 公开 API，`cve` 依赖
/// 只有 `meli`，issue 约束「不新增 test-only 依赖」，melib 亦无 TLS 服务器实现，
/// 故按 issue 允许的「真实证书语料 + 单元级 mock」做免疫证明。语料是 openssl
/// 离线生成的真实 ECDSA P-256 证书（受信根 + 四张由它直接真实签名的叶子：
/// L_MATCH 正确基线、A1 名字不匹配、A2 的 CN 内嵌 `\0`、A3 的 SAN dNSName 内嵌
/// `\0` 且 CN 干净），`openssl verify` 转录逐字内嵌（无主机名校验四叶全 `OK`；
/// 带 `-verify_hostname` 时 L_MATCH `OK`、A1/A2/A3 全 `error 62 hostname
/// mismatch`）。
///
/// [`cve_2009_3765`] 分层锁定：(a) 语料真实性——四叶 issuer TLV 逐字节等于受信
/// 根 subject TLV，A2 的 CN 恰为 `imap.victim.example\0.attacker.example`
///（37 字节、无 SAN），A3 的 SAN 内嵌 `\0` 而 CN 干净，SPKI 互异，DER 长度与
/// `asn1parse` NUL 证据命中；(b) 攻击复现——旧 mutt 三面缺陷谓词（完全不比主机
/// 名 / CN 截断到首个 `\0` / SAN 全量比较失败后回退 CN）对 L_MATCH/A1/A2/A3
/// 全部接受；(c) 核心断言——RFC 6125 正确语义只接受 L_MATCH，A1/A2/A3 全拒绝，
/// SAN 在场即权威且内嵌 `\0` 的名字（含参考主机名）永不匹配；(d) L3 源码 /
/// manifest 扫描锁定三处 connector 的默认校验器 + 门控 danger + 主机名、
/// `utils/connections.rs` 仅传输包装、账号配置默认 false、无自定义主机名校验面、
/// TLS 栈为 native-tls，以及可直达的 [`SmtpSecurity::default`] danger=false。
/// 结论：**免疫证明，未发现缺口，未触碰生产代码**。
///
/// [`cve_2009_3765`]: self::cve_2009_3765
/// [`SmtpSecurity::default`]: meli::melib::smtp::SmtpSecurity
#[cfg(test)]
#[path = "CVE-2009-3765.rs"]
mod cve_2009_3765;

/// CVE-2009-3766（mutt 1.5.19 / 1.5.20，OpenSSL；CWE-295 / CWE-310）**TLS 服务器
/// 证书主机名完全不校验** regression（issue #89，表 5 协议信任边界 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md`）：NVD 原文指出 mutt 使用 OpenSSL 时未正确
/// 校验证书 CN 字段，中间人可用**合法 CA 签发**、名字完全无关的证书冒充任意
/// SSL 服务器。它与同族 CVE-2009-3765 的差别是「根本不比」而非「截断后比」；
/// issue #89 指定攻击样例形态为 CN 内嵌 NUL 截断（`good.example\0evil.example`
/// 伪装匹配 `good.example`），并要求「含 NUL 的 CN/SAN 一律判定不匹配、拒绝」，
/// 语料域名 `good.example` / `evil.example` 与 issue #88 完全独立。
///
/// meli 等价面：issue 指定的 `melib/src/utils/connections.rs` 只是传输层包装
///（`Connection::Tls` 包 `native_tls::TlsStream`，无任何 CN / 主机名逻辑）；
/// 主机名校验策略全在 native-tls（Linux=OpenSSL 的 `X509_check_host` 语义）
/// 内部，入口在三处 `connector.connect(&path, socket)`——
/// `melib/src/imap/connection.rs`（path=`server_conf.server_hostname`）、
/// `melib/src/nntp/connection.rs`、`melib/src/smtp.rs`（path=`server_conf.hostname`）；
/// 三处 `TlsConnector::builder()` 都是默认校验器，只有账号配置显式打开
/// `danger_accept_invalid_certs` 时才追加危险开关，全仓无
/// `danger_accept_invalid_hostnames`；JMAP 的 isahc 客户端
///（`melib/src/jmap/connection.rs`、`eventsource.rs`）三个 `danger_*`
///（certs/hosts/revoked）由同一账号开关门控；四个账号配置默认全是 `false`，
/// `SmtpSecurity::default` 亦然。workspace 无 rustls / webpki，也无
/// `X509_check_host`/`verify_callback`/自定义 CN 解析代码。
///
/// **为何单元级 mock**：`native-tls`/`openssl` 不是 melib 公开 API，`cve` 依赖
/// 只有 `meli`，issue 约束「不新增 test-only 依赖」，melib 亦无 TLS 服务器实现，
/// 故按 issue 允许的「真实证书语料 + 单元级 mock」做免疫证明。语料是 openssl
/// 离线生成的真实 ECDSA P-256 证书（受信根 + 四张由它直接真实签名的叶子：
/// L_MATCH 正确基线 `good.example`、A1 名字不匹配 `evil.example`、A2 的 CN 内嵌
/// `\0`（`good.example\0evil.example`，25 字节、无 SAN）、A3 的 SAN dNSName 内嵌
/// `\0` 且 CN 干净），`openssl verify` 转录逐字内嵌（无主机名校验四叶全 `OK`；
/// 带 `-verify_hostname good.example` 时 L_MATCH `OK`、A1/A2/A3 全 `error 62
/// hostname mismatch`）。
///
/// [`cve_2009_3766`] 分层锁定：(a) 语料真实性——四叶 issuer TLV 逐字节等于受信
/// 根 subject TLV，A2 的 CN 恰为 `good.example\0evil.example`（25 字节、无 SAN），
/// A3 的 SAN 内嵌 `\0` 而 CN 干净，SPKI 互异，DER 长度与 `asn1parse` NUL 证据
/// 命中；(b) 攻击复现——旧 mutt 三面缺陷谓词（完全不比主机名 / CN 截断到首个
/// `\0` / SAN 全量比较失败后回退 CN）对 L_MATCH/A1/A2/A3 全部接受；(c) 核心
/// 断言——RFC 6125 正确语义只接受 L_MATCH，A1/A2/A3 全拒绝，**NUL 不截断比较**：
/// 含 NUL 的 CN/SAN 永不匹配（即使其 NUL 前缀恰等于 `good.example`），SAN 在场
/// 即权威且绝不回退 CN；(d) L3 源码 / manifest 扫描锁定三处 connector 的默认
/// 校验器 + 门控 danger + 主机名、`utils/connections.rs` 仅传输包装、账号配置
/// 默认 false、无自定义主机名校验面、TLS 栈为 native-tls，以及可直达的
/// [`SmtpSecurity::default`] danger=false。
/// 结论：**免疫证明，未发现缺口，未触碰生产代码**。
///
/// [`cve_2009_3766`]: self::cve_2009_3766
/// [`SmtpSecurity::default`]: meli::melib::smtp::SmtpSecurity
#[cfg(test)]
#[path = "CVE-2009-3766.rs"]
mod cve_2009_3766;

/// CVE-2020-15917（Claws Mail < 3.17.6，CVSS 9.8，NVD；CWE-345/CWE-346）
/// STARTTLS 明文后缀注入 regression（issue #90，表 5 协议信任边界 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md`）：MITM 在 IMAP `STARTTLS` 的 tagged OK
///（`M1 OK Begin TLS negotiation now\r\n`）之后向同一 TCP 连接注入明文响应
/// 行；Claws Mail `session.c` 的会话缓冲跨 TLS 升级存活，残留明文被当作升级
/// 后的「安全」响应解析，中间人据此在 TLS 会话内伪造 CAPABILITY/认证/SELECT
/// 结果。注入变体：黏连同段、分段到达、夹在 greeting 与 tagged OK 之间、或
/// 在客户端 ClientHello 之后继续注入明文。
///
/// meli 等价面：issue 指定 `melib/src/imap/connection.rs` 的 STARTTLS 升级
///（等价面 SMTP / NNTP 同型）。`if server_conf.use_starttls` 块里的协商缓冲
/// 是**块内局部变量**，看到 tagged `M1 OK` break 后整体丢弃；TLS 升级直接包裹
/// 裸 socket，内核缓冲里的残留明文只会进 TLS 记录层 → 握手失败（fail-closed）；
/// 升级后首个响应必来自 TLS 会话内（IMAP/NNTP 重发 CAPABILITY，SMTP 重发
/// EHLO，post-TLS 读取用全新缓冲）。因此后缀注入类在 meli 上不可能成为升级后
/// 的响应。
///
/// 本次攻击模拟暴露并修复了一个真实缺口（交付物 A）：IMAP / NNTP 的 TLS 握手
/// 块在 `AsyncWrapper::into_inner()` 之后没有恢复阻塞模式（async-io 2.x 的
/// `into_inner` 文档明确要求调用方 `set_nonblocking(false)`；SMTP 早有此调用），
/// 于是 native-tls 返回 `HandshakeError::WouldBlock` 中间流后，无超时的
/// `loop { handshake() }` 在非阻塞 fd 上**热自旋（100% CPU）**——敌意服务器
/// 应答 tagged OK 后保持沉默即可让客户端永久烧 CPU。修复：恢复阻塞模式、施加
/// 账号读写超时，并在 WouldBlock 循环里加握手截止时间（`ErrorKind::TimedOut`）。
///
/// [`cve_2020_15917`] 分层锁定：(a) 攻击复现——Claws Mail 式跨升级会话缓冲对
/// S1 语料逐行吐出注入的 `* CAPABILITY …` / `M2 OK …` / `* 3 EXISTS`；(b) meli
/// 语义核心断言——按 425-533 行真实算法复刻，每个变体要么协商 Err，要么升级且
/// 携带过升级的缓冲恒为空，OK 前未打标脏行必须协商失败；(c) 行为级攻击——本地
/// 敌意服务器打真实 `ImapStream::new_connection`（黏连 / 分段 / OK 前 /
/// ManageSieve / 沉默 MITM 五场景），客户端须进入 TLS（服务器读到 `0x16 0x03`）、
/// TLS 流内不得含明文 IMAP 命令、注入后被 drop 必须 Err、沉默服务器须在账号
/// 超时内以 `TimedOut` 返回而非热自旋到对端 drop；(d) L3 源码 / 工程扫描锁定
/// imap / nntp / smtp 三处升级块的阻塞恢复 + 超时 + deadline + 升级后重发命令，
/// 且 `ImapStream` 不含 `read_buffer`/`Vec<u8>` 跨升级残留缓冲。
/// 结论：**后缀注入类免疫；沉默服务器缺口发现并修复**。
///
/// [`cve_2020_15917`]: self::cve_2020_15917
/// [`ImapStream::new_connection`]: meli::melib::imap::ImapStream::new_connection
#[cfg(test)]
#[path = "CVE-2020-15917.rs"]
mod cve_2020_15917;

/// Pager OSC 8 hyperlink URL sanitization regression (issue #97,
/// follow-up to CVE-2024-37384 / issue #74): the pager renders text
/// mail bodies and any URL it `linkify`-extracts becomes the OSC 8
/// payload via `Screen::draw_horizontal_segment`. Previously the URL
/// was interpolated **raw** into `\x1b]8;...;{url}\x07`, so a mail
/// body URL containing BEL/ST would early-terminate the OSC 8
/// sequence and turn the trailing bytes into live terminal commands
/// (`ESC[2J`, OSC 52 clipboard write, RIS, ...). The fix mirrors the
/// `window_title` fix one-for-one: route the URL through
/// [`sanitize_osc_payload`] (strips all `char::is_control` - C0/DEL/C1)
/// before interpolating it; when the sanitized URL is empty, emit no
/// OSC 8 at all. The link **text** still renders - the pager uses the
/// link text regardless - so the visual layout is unchanged; only the
/// OSC 8 control sequence is sanitized.
///
/// [`osc8_pager_hyperlink_sanitization`] drives the corpus through
/// the production `Hyperlink::write_start_sanitized` path and
/// through the production `Screen::<Tty>::draw_horizontal_segment`
/// emitter: every URL with control bytes produces an OSC 8 with a
/// body that is the URL minus its control bytes (and exactly one BEL
/// terminator); a control-only URL emits no OSC 8 at all; an honest
/// URL is byte-identical to the unsanitized path. Fix in
/// [`crate::cve_2024_37384`] (issue #74) for the configuration-side
/// precedent.
///
/// [`sanitize_osc_payload`]: meli::terminal::sanitize_osc_payload
#[cfg(test)]
#[path = "OSC8-pager-hyperlink.rs"]
mod osc8_pager_hyperlink_sanitization;

/// CVE-2021-31855（KMail / Messagelib ≤ 5.17.0；CVSS 3.1 6.5，CWE-312
/// 「Cleartext Storage of Sensitive Information」）**删除已解密邮件附件时把
/// 明文正文回传服务器** regression（issue #103）。KMail 的
/// `ViewerPrivate::deleteAttachment` 在查看远端（IMAP）存储、已解密的加密
/// 邮件时，用户删除某个附件会让处理函数把**解密后的 session 明文正文**
/// 重新组装并写回服务器；能读取服务端存储者因此拿到本应只存在于受害者
/// 会话内的明文。
///
/// meli 的等价面只在 composer：从已存储邮件进入 composer 的唯一入口是
/// view 的 `edit` 快捷键 → `Composer::edit` → [`Draft::edit`]，附件删除是
/// `ComposerTabAction::RemoveAttachment` 的
/// `self.draft.attachments_mut().remove(idx)`，回传是 [`Draft::finalise`] →
/// `save_draft`/`save_special` → IMAP `APPEND`。对 `multipart/encrypted`，
/// 解析层的 `body.text()` 为空、`body.attachments()` 是线上部件 raw 的克隆，
/// 解析层从不解密；解密只发生在 view-filter 层并只构造显示用 `Attachment`，
/// 绝不进入 `Draft`——所以删除附件后的序列化仍是原样密文。
///
/// [`cve_2021_31855`] 用 gpg 2.4.9 离线生成的真实 PGP/MIME 语料（cv25519
/// 子钥，PKESK v3 + MDC 保护 SEIPD v1，CRC-24 与包形状在测试内重算；解密
/// inner 实体是 `multipart/mixed`：含 SECRET 标记的 text/plain 正文 + 含
/// 机密附件名标记的附件 part）分层锁定：L1 解析面 `text()`/`decode_rec()`
/// 无明文、L2 `Draft::edit` 只装载线上密文 raw、L3 逐个 `remove(idx)` 及删空
/// 变体的 `finalise()` 输出无任何明文标记且未删 armor 逐字保留、L4 源码扫描
/// 证明写回面只承载调用方 bytes。结论：**免疫证明，未发现缺口，未触碰生产
/// 代码**；CVE 的「删除附件 → 明文正文回传」原语在 meli 中不可表达。
///
/// [`Draft::edit`]: meli::melib::Draft::edit
/// [`Draft::finalise`]: meli::melib::Draft::finalise
#[cfg(test)]
#[path = "CVE-2021-31855.rs"]
mod cve_2021_31855;
