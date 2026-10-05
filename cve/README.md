<!-- SPDX-License-Identifier: EUPL-1.2 OR GPL-3.0-or-later -->

# cve

Security CVE research and regression workspace of the
[hardened-meli](https://github.com/kylelee/hardened-meli) fork.

## What lives here

- `SECURITY-CVE-RESEARCH.md` /
  [`SECURITY-CVE-RESEARCH.zh-CN.md`](./SECURITY-CVE-RESEARCH.zh-CN.md) —
  the source-verified survey of CVEs delivered through e-mail or directly
  attacking mail clients and their rendering path: tracking and privacy
  leaks, malware and code execution, web/HTML embedding, e-mail-reachable
  browser engines, and protocol/crypto trust boundaries.
- `src/` — the crate scaffolding for CVE-driven regression tests that
  target the `meli` and `melib` crates without adding test-only
  dependencies to the shipped binaries. A regression is one CVE (or CVE
  family) plus its verbatim attack payload plus an inertness or
  round-trip assertion against the hardened code path (the
  CVE-2025-66376 tag-splitting corpus locking down meli's built-in HTML
  sanitizer is the model).

## Usage

Run the regressions:

```sh
cargo test -p cve
```

The crate is a plain workspace member: lint with `make lint`, format with
`make fmt` (both cover the whole workspace, including `cve`).

## Attack-simulation task board (issue #6)

One Gitea issue per CVE of the research report (plus MFSA-2005-11), each
carrying the CVE background, the mapped meli attack surface, payload
construction hints, expected assertions, and the acceptance criteria for a
`src/<cve-id>.rs` regression corpus. CVE-2025-66376 is already covered by
issue #5's tag-splitting corpus and has no duplicate issue.

The first per-CVE corpus has landed: MFSA-2005-11 (issue #13, cookie
tracking) in [`src/MFSA-2005-11.rs`](./src/MFSA-2005-11.rs) — 20
auto-load beacon vectors plus a full tracking-spam mail locking the
sanitize → html2text pipeline's no-remote-loading immunity.

**2026-10-05 second batch (+78 issues, #101–#178)**: the externally supplied
KIMI research summary was cross-checked against NVD API 2.0 (all 50 new IDs
verified, CVSS backfilled), deduplicated against the 28-ID 查缺补漏 round and
the existing board, and merged into both research reports (see the reports'
section 10). Issues #101–#178 cover that batch plus the 28 查缺补漏 CVEs that
had no issue yet. CVE-2019-10732's Table 5 row was subsumed by the Table 1
decrypt-oracle family row; CVE-2021-32066 (Net::IMAP library, not a mail
client) is prose-only in the trends section and intentionally has no issue.

- **Table 1 — tracking & privacy** (17): MFSA-2005-11 (#13), CVE-2005-2512 (#14), CVE-2006-1045 (#15), CVE-2008-3068 (#16), CVE-2008-4491 (#17), CVE-2017-17688 (#18), CVE-2017-17689 (#19), CVE-2026-0818 (#20), CVE-2018-0950 (#101), CVE-2021-29956 (#102), CVE-2021-31855 (#103), CVE-2018-19516 (#126), CVE-2019-10732 (#104), CVE-2019-10734 (#127), CVE-2019-10735 (#128), CVE-2019-10740 (#129), CVE-2019-10741 (#130)
- **Table 2 — malware & code execution** (67): CVE-2023-23397 (#21), CVE-2024-21413 (#22), CVE-2024-21378 (#23), CVE-2024-30103 (#24), CVE-2025-21361 (#25), CVE-2025-47176 (#26), CVE-2026-70329 (#27), CVE-2024-43604 (#28), CVE-2006-2386 (#29), CVE-2006-1305 (#30), CVE-2020-9818 (#31), CVE-2020-9819 (#32), CVE-2008-0039 (#33), CVE-2006-6505 (#34), CVE-2026-14899 (#35), CVE-2026-84641 (#36), CVE-2026-84639 (#37), CVE-2026-84640 (#38), CVE-1999-0940 (#39), CVE-2001-0473 (#40), CVE-2014-9116 (#41), CVE-2022-1328 (#42), CVE-2023-4874 (#43), CVE-2023-4875 (#44), CVE-2002-0833 (#45), CVE-2003-0302 (#46), CVE-2003-0376 (#47), CVE-2007-2770 (#48), CVE-2007-3166 (#49), CVE-1999-0427 (#50), CVE-2004-1944 (#51), CVE-2015-8614 (#52), CVE-2015-8708 (#53), CVE-2012-4507 (#54), CVE-2020-16094 (#55), CVE-2024-37385 (#56), CVE-2020-16947 (#105), CVE-2009-0587 (#106), CVE-2000-0481 (#107), CVE-2025-49113 (#137), CVE-2003-0300 (#138), CVE-2005-0667 (#139), CVE-2005-2549 (#140), CVE-2005-2550 (#141), CVE-2008-1108 (#142), CVE-2024-38173 (#131), CVE-2023-35636 (#132), CVE-2023-36763 (#133), CVE-2023-35619 (#134), CVE-2000-0567 (#135), CVE-2001-0145 (#136), CVE-2021-32055 (#108), CVE-2018-14349 (#143), CVE-2018-14350 (#144), CVE-2018-14351 (#145), CVE-2018-14352 (#146), CVE-2018-14353 (#147), CVE-2018-14354 (#148), CVE-2018-14355 (#149), CVE-2018-14356 (#150), CVE-2018-14357 (#151), CVE-2018-14358 (#152), CVE-2018-14359 (#153), CVE-2018-14360 (#154), CVE-2018-14361 (#155), CVE-2018-14362 (#156), CVE-2018-14363 (#157)
- **Table 3 — web/HTML embedding** (41): CVE-2020-12641 (#57), CVE-2002-1770 (#58), CVE-2002-1210 (#59), CVE-2001-1326 (#60), CVE-2002-2351 (#61), CVE-2003-0336 (#62), CVE-2001-0677 (#63), CVE-1999-1016 (#64), CVE-2007-4040 (#65), CVE-2007-2225 (#66), CVE-2007-2227 (#67), CVE-2008-1448 (#68), CVE-2021-37746 (#69), CVE-2015-7609 (#70), CVE-2008-2248 (#71), CVE-2015-8864 (#72), CVE-2016-4068 (#73), CVE-2024-37384 (#74), CVE-2025-48700 (#75), CVE-2026-73572 (#76), CVE-2002-2086 (#109), CVE-2002-1649 (#110), CVE-2016-7966 (#111), CVE-2016-7967 (#112), CVE-2016-7968 (#113), CVE-2022-29360 (#114), CVE-2024-42009 (#115), CVE-2024-42010 (#116), CVE-2024-45516 (#117), CVE-2000-0621 (#158), CVE-2001-0999 (#159), CVE-2001-1088 (#160), CVE-2020-11880 (#161), CVE-2020-35730 (#162), CVE-2023-43770 (#163), CVE-2023-5631 (#164), CVE-2024-42008 (#165), CVE-2024-27443 (#166), CVE-2025-27915 (#167), CVE-2025-30349 (#168), CVE-2021-44026 (#169)
- **Table 4 — e-mail-reachable browser engines** (9): CVE-2021-30858 (#77), CVE-2023-4863 (#78), CVE-2023-41061 (#79), CVE-2023-41064 (#80), CVE-2026-8091 (#81), CVE-2010-0249 (#82), CVE-2016-3714 (#118), CVE-2021-30761 (#170), CVE-2021-30860 (#171)
- **Table 5 — protocol & crypto trust boundaries** (22): CVE-2007-1268 (#83), CVE-2024-49393 (#84), CVE-2024-49394 (#85), CVE-2024-49395 (#86), CVE-2009-1390 (#87), CVE-2009-3765 (#88), CVE-2009-3766 (#89), CVE-2020-15917 (#90), CVE-2007-1265 (#119), CVE-2014-8878 (#120), CVE-2017-9604 (#121), CVE-2020-15954 (#122), CVE-2021-38373 (#123), CVE-2024-50624 (#124), CVE-2018-12020 (#125), CVE-2018-12019 (#172), CVE-2021-29969 (#173), CVE-2016-10727 (#174), CVE-2020-28896 (#175), CVE-2020-15047 (#176), CVE-2021-38372 (#177), CVE-2021-39272 (#178)
