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
