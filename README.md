# meli  ![Established, created in 2017](https://img.shields.io/badge/Est.-2017-blue) ![Minimum Supported Rust Version](https://img.shields.io/badge/MSRV-1.85.0-blue) [![GitHub license](https://img.shields.io/github/license/meli/meli)](https://github.com/meli/meli/blob/master/COPYING) [![Crates.io](https://img.shields.io/crates/v/meli)](https://crates.io/crates/meli) [![IRC channel](https://img.shields.io/badge/irc.oftc.net-%23meli-blue)](ircs://irc.oftc.net:6697/%23meli)

**English** | [简体中文](./README.zh-CN.md)

https://github.com/user-attachments/assets/5cdbbc3e-9b49-46e4-ae37-be0a58cf300c

**A security-hardened and UX-optimized version of meli — BSD/Linux/macos terminal email client with support for multiple accounts and Maildir / mbox / notmuch / IMAP / JMAP / NNTP (Usenet).**

Hardened and based on <https://github.com/meli/meli>

## Highlights

This repository is a security-hardened and UX-optimized fork of meli: on top of the original it received a full code audit, hardening and refactoring, plus several real-world usability improvements. Four core highlights:

### 1. Security hardening: four lines of defense against e-mail attacks

A comprehensive code audit and refactor of the original meli fixed 18 audit findings (including 3 HIGH: mailcap command injection, mailto CRLF header injection, and RFC2047 display-name reply hijacking) and removed ~1500 lines of dead code while deduplicating logic. Against malformed/hostile e-mail content (such as QQ Mail's unescaped quoted Message-IDs, empty local-part sender addresses, and script-laden HTML bodies), four lines of defense were designed, layer upon layer:

1. **Parse tolerance**: when an IMAP ENVELOPE field fails strict parsing, it automatically falls back to raw-bytes parsing — any single malformed field can no longer abort the fetch of an entire mailbox;
2. **Ingestion sanitization**: address fields (From/Sender/Reply-To/To/Cc/Bcc) are validated and normalized before being written to the cache; fixable ones are automatically quoted and re-verified, unfixable ones are replaced with a safe placeholder — new data can never produce "poison rows";
3. **Visible quarantine**: legacy poisoned cache rows no longer trigger a whole-database reset; they are quarantined row by row into an `invalid_envelopes` table and shown as visible placeholder e-mails (with error-detail headers), self-healing after the server re-fetch — eliminating "one poison e-mail nukes the entire cache".
4. **Sanitize HTML**: HTML e-mail bodies are rendered by a built-in HTML renderer — the mail is sanitized with an allow-list (ammonia), keeping only safe structural tags and `http`/`https`/`mailto` links while stripping scripts, styles, event-handler attributes, comments and dangerous URL schemes (`javascript:`, `data:`), then converted to plain text (html2text) at the terminal's width, all in-process with no external dependency (the standalone sanitizer binary is gone, absorbed into `meli`), so hostile HTML mail can no longer smuggle scripts or tracking links into the rendered view.

### 2. UX: browse the whole mailbox with the arrow keys

Thread view navigation was fully strengthened: **Up/Down** move through the thread list while the mail body on the right switches live; **Left/Right** shift and enlarge focus between the thread-list and mail-view panes, forming a complete arrow-key navigation chain — the entire mailbox can be browsed with arrow keys alone.

### 3. UX: cache-first, near-instant startup

IMAP startup is now fully cache-first: the mail list and bodies are rendered immediately from the local sqlite cache (stale-while-revalidate, with network deltas syncing silently in the background); the mailbox folder list is persisted to the cache so startup no longer waits for an online check; combined with STATUS counter short-circuiting (RFC 4549) and MSN index persistence, full-scan commands are eliminated from startup. Measured on a real account (QQ Mail INBOX, ~5000 messages): warm-start wait dropped from minutes to seconds (median ~2.7 s, best ~1 s).

### 4. UI: entire interface rebuilt on ratatui, much better looks

The whole product interface was rebuilt with the beautiful [ratatui](https://ratatui.rs) library — layout solving, border rendering and dialog/OSD placement now run through ratatui's `Layout` and `Block` primitives, with terminal I/O migrated to crossterm — greatly improving the visual aesthetics.

**Table of contents**:

- [Highlights](#highlights)
- [Install](#install)
- [Build](#build)
  - [Cargo Compile-time Features](#cargo-compile-time-features)
- [Quick start](#quick-start)
  - [Supported E-mail backends](#supported-e-mail-backends)
  - [E-mail submission backends](#e-mail-submission-backends)
  - [Non-exhaustive list of features](#non-exhaustive-list-of-features)
  - [HTML Rendering](#html-rendering)
- [Documentation](#documentation)

## Install

- Cargo install by source code

  Install from git repository:
  ```sh
  cargo install --git https://github.com/kylelee/hardened-meli meli
  ```

### Runtime dependencies

HTML e-mail is rendered out of the box by a built-in renderer (ammonia
sanitizing + html2text at terminal width), so there is no required external
dependency.

## Build

Run `make` or `cargo build --release`.

See `make help` output for information on how to use the `Makefile`.

For detailed building instructions, see [`BUILD.md`](./BUILD.md)

For the upstream meli sync log, see [`SYNC.md`](./SYNC.md).

### Cargo Compile-time Features

`meli` supports opting in and out of features at compile time with cargo features.

The contents of the `default` feature are:

```toml
default = ["sqlite3", "notmuch", "smtp", "http", "dbus-notifications", "gpgme", "cli-docs", "jmap", "static"]
```

A list of all the features and a description for each follows:

| Feature flag                                                  | Dependencies                                                                                 | Notes                                                                                                                                                                                             |
|---------------------------------------------------------------|----------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| <a name="http-feature">`http`</a>                       | `melib` `http` feature                                                                       | Provides the HTTP client (via `melib`, used by the JMAP backend)                                                                   |
| <a name="notmuch-feature">`notmuch`</a>                       | `maildir` feature                                                                            | Provides the *notmuch* backend                                                                                                    |
| <a name="jmap-feature">`jmap`</a>                             | `http` feature, `url` crate with `serde` feature                                             | Provides the *JMAP* backend                                                                                                                                                                       |
| <a name="smtp-feature">`smtp`</a>                             | `tls` feature                                                                                | Integrated async *SMTP* client                                                                                                                                                                    |
| <a name="sqlite3-feature">`sqlite3`</a>                       | `rusqlite` crate with `bundled-full` feature                                                 | Used in caches                                                                                                                                                                                    |
| <a name="sqlite3-static-feature">`sqlite3-static`</a>         | `rusqlite` crate with `bundled-full` feature                                                 | Same as `sqlite3` feature but provided for consistency and in case `sqlite3` feature stops bundling libsqlite3 statically in the future.                                                          |
| <a name="smtp-trace-feature">`smtp-trace`</a>                 | `smtp` feature                                                                               | Connection trace logs on the `trace` logging level                                                                                                                                                |
| <a name="gpgme-feature">`gpgme`</a>                           |                                                                                              | *GPG* use by dynamically loading `libgpgme.so`                                                                                                                                                    |
| <a name="tls-static-feature">`tls-static`</a>                 | `native-tls` crate with `vendored` feature                                                   | Links with `OpenSSL` statically where it's used                                                                                                                                                   |
| <a name="http-static-feature">`http-static`</a>               | `isahc` crate with `static-curl` feature                                                     | Links with `curl` statically                                                                                                                                                                      |
| <a name="dbus-notifications-feature">`dbus-notifications`</a> | `notify-rust` dependency                                                                     | Uses DBus notifications                                                                                                                                                                           |
| <a name="dbus-static-feature">`dbus-static`</a>               | `notify-rust` dependency and enableds its `d_vendored` feature                               | Includes the dbus library statically.                                                                                                                                                             |
| <a name="cli-docs-feature">`cli-docs`</a>                     | `flate2` dependency                                                                          | Includes the manpage documentation compiled by either `mandoc` or `man` binary to plain text in `meli`'s command line. Embedded documentation can be viewed with the subcommand `meli man [PAGE]` |
| <a name="libz-static-feature">`libz-static`</a>               | `libz-sys` dependency and enables its `static` feature                                       | Allows for the transitive dependency libz (from `curl`) to be linked statically.                                                                                                                  |
| <a name="static-feature">`static`</a>                         | enables `tls-static`, `http-static`, `sqlite3-static`, `dbus-static`, `libz-static` features |                                                                                                                                                                                                   |

## Quick start

```sh
# Create configuration file in ${XDG_CONFIG_HOME}/meli/config.toml:
$ meli create-config
# Edit configuration in ${EDITOR} or ${VISUAL}:
$ meli edit-config
# Optionally, install manual pages if installed via cargo:
$ meli install-man
# Ready to go.
$ meli
# You can read any manual page with the CLI subcommand `man`:
$ meli man meli.7
# See help output for all options and subcommands.
$ meli --help
```

See a comprehensive tour of `meli` in the manual page [`meli(7)`](./meli/docs/meli.7).

See also the [Quickstart tutorial](https://meli-email.org/documentation.html#quick-start) online.

After installing `meli`, see `meli(1)`, `meli.conf(5)`, `meli(7)` and `meli-themes(5)` for documentation.
Sample configuration files can be found in the `meli/docs/samples/` subdirectory; theme files in `meli/themes/`.
Examples for configuration file settings can be found in `meli.conf.examples(5)`
Manual pages are also [hosted online](https://meli-email.org/documentation.html "meli documentation").
`meli` by default looks for a configuration file in this location: `${XDG_CONFIG_HOME}/meli/config.toml`.

You can run meli with arbitrary configuration files by setting the `${MELI_CONFIG}` environment variable to their locations, i.e.:

```sh
MELI_CONFIG=./test_config cargo run
```

See [`meli(7)`](./meli/docs/meli.7) for an extensive tutorial and [`meli.conf(5)`](./meli/docs/meli.conf.5) for all configuration values.

| Main view | Compact main view | Compose with embed terminal editor |
|-----------|-------------------|------------------------------------|
| ![Main view screenshot](./meli/docs/screenshots/main.webp "mail meli view screenshot") | ![Compact main view screenshot](./meli/docs/screenshots/compact.webp "compact main view screenshot") | ![Compose with embed terminal editor screenshot](./meli/docs/screenshots/compose.webp "composing view screenshot") |

### Supported E-mail backends

| Protocol      | Support    |
|---------------|------------|
| IMAP          | full       |
| Maildir       | full       |
| notmuch       | full[^0]   |
| mbox          | read-only  |
| JMAP          | functional |
| NNTP / Usenet | functional |

[^0]: there's no support for searching through all email directly, you'd have to
      create a mailbox with a notmuch query that returns everything and search
      inside that mailbox.

### E-mail submission backends

- SMTP
- Pipe to shell script
- Server-side submission when supported

### Non-exhaustive list of features

- TLS
- email threading support
- multithreaded, async operation
- optionally run your editor of choice inside meli, with an embedded
  xterm-compatible terminal emulator
- plain text configuration in TOML
- ability to open emails in UI tabs and switch to them
- optional sqlite3 index search
- override almost any setting per mailbox, per account
- contact list (+read-only vCard and mutt alias file support)
- forced UTF-8 (other encodings are read-only)
- configurable shortcuts
- theming: **dozens of themes built into the binary, ready out of the
  box** — the Zed editor's official theme family (Ayu, One, Gruvbox, from
  [zed.dev](https://zed.dev)) plus community-contributed theme packs
  (Catppuccin, Dracula, GitHub, Nord, Tokyo Night, Nightfox, Fleet, …),
  all compiled in; the `:toggle theme` picker live-previews the whole UI
  with the arrow keys and Enter saves the choice to the configuration
  file
- `NO_COLOR` support
- ascii-only drawing characters option
- view text/html attachments through the built-in HTML renderer (or an external command via `pager.html_filter`)
- text wrapping that measures display width (CJK characters count as two columns) and never splits an English word mid-word: URLs and hyphenated compounds get soft breaks after `/` and `-`, only a unit wider than the window is hard-cut losslessly with the `⤷` continuation marker, runs of blank lines longer than two collapse to two, and lines wrap to the actual window width, down to narrow terminals
- pipe attachments/mail to stuff
- use external attachment file picker instead of typing in an attachment's full path
- save all attachments of the viewed mail to ~/Downloads/meli-<subject> with one command or keystroke (default `C-s`)
- GPG signing, encryption, signing + encryption
- GPG signature verification

### HTML Rendering

HTML mail is rendered by the built-in renderer by default: meli sanitizes
it with an allow-list (ammonia, removing scripts and dangerous links) and
converts it to plain text (html2text) at the terminal's width — no external
dependency is required. For more details consult
[`meli.conf(5)`](./meli/docs/meli.conf.5).


## Documentation

See a comprehensive tour of `meli` in the manual page [`meli(7)`](./meli/docs/meli.7).

See also the [Quickstart tutorial](https://meli-email.org/documentation.html#quick-start) online.

After installing `meli`, see `meli(1)`, `meli.conf(5)`, `meli(7)` and `meli-themes(5)` for documentation.
Sample configuration files can be found in the `meli/docs/samples/` subdirectory; theme files in `meli/themes/`.
Manual pages are also [hosted online](https://meli-email.org/documentation.html "meli documentation").

`meli` by default looks for a configuration file in this location: `${XDG_CONFIG_HOME}/meli/config.toml`

You can run meli with arbitrary configuration files by setting the `${MELI_CONFIG}` environment variable to their locations, or use the `[-c, --config]` argument:

```sh
MELI_CONFIG=./test_config meli
```

or

```sh
meli -c ./test_config
```
