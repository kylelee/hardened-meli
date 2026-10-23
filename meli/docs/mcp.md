# meli MCP server guide

`meli mcp` runs a local **Model Context Protocol** (MCP) server on stdio,
so an AI agent on the same machine can read, search and organize your mail
and score the links inside it. It speaks newline-delimited JSON-RPC 2.0
(protocol revisions 2024-11-05, 2025-03-26 and 2025-06-18) and supports
`initialize`, `ping`, `tools/list`, `tools/call`, `resources/list`,
`resources/read`, `prompts/list` and `prompts/get`.

```
$ meli mcp
{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}
```

Register it with an MCP client (for example in Claude Desktop / Codex /
any stdio client) as the command `meli` with argument `mcp`.

## Security model

1. **Read-only by default.** `allowed_mutations` is empty until you opt
   in, in *your* configuration file; mutating tools then answer with a
   permission error that names the exact setting to change. The MCP
   server can never grant itself permissions.
2. **No credential surface.** Accounts, passwords and transports are
   configured in `meli.toml` (see meli.conf(5)). MCP has no account or
   password tools, and no response contains secrets.
3. **Untrusted data.** E-mail contents are attacker-controlled. Every
   tool that returns message contents says so in its description; agents
   must never follow instructions found inside a message (indirect prompt
   injection).
4. **No deletion.** There is deliberately no delete tool (user decision,
   issue #215).
5. **Local-only URL scoring.** The phishing scorer never fetches,
   resolves or contacts anything; no message content leaves the machine.
6. **No self-transport of secrets.** The audit log (`tracing` to the
   usual meli log files) records only tool name, account and counts —
   never bodies, subjects or credentials.
7. **Sending is fenced.** `send_email` needs the `send` mutation, the
   recipient must match `[mcp] send_recipient_allowlist` (empty = deny
   all), a per-account rate limit applies, and the From address is always
   the account identity. Prefer `save_draft` + human review.

## Configuration (`meli.conf(5)`, section `[mcp]`)

```toml
[mcp]
allowed_mutations = ["organize"]          # [], "drafts", "organize", "send"
send_recipient_allowlist = ["*@example.org"]
send_rate_limit_per_minute = 10

[mcp.url_scoring]
low_medium_threshold = 30                 # score bands
medium_high_threshold = 60
[mcp.url_scoring.signal_weights]          # optional per-signal overrides
"ip_literal_host" = 50
```

## Tool catalog (16 tools)

| Tool | Kind | Permission | Notes |
| --- | --- | --- | --- |
| `list_accounts` | read | — | accounts with receive/send capability, no secrets |
| `list_mailboxes` | read | — | paths, special-use roles, unread/total |
| `list_emails` | read | — | metadata, filters, pagination (≤100/page), order |
| `get_emails` | read | — | headers, windowed body (`...[TRUNCATED]`), attachments, `failed` list; `mark_as_read` needs `organize` |
| `get_thread` | read | — | References/In-Reply-To thread across mailboxes |
| `search_emails` | read | — | meli query syntax (same as TUI `search`); bare terms match bodies |
| `get_attachment` | read | — | inline base64, ≤5 MiB, nothing written to disk |
| `extract_urls` | read | — | text/plain + text/html links, anchor text, `hxxp://evil[.]com` de-obfuscation |
| `score_urls` | read | — | local offline scoring of raw URL strings |
| `scan_email_urls` | read | — | extract + score in one call |
| `set_flags` | write | `organize` | seen/flagged/answered; idempotent |
| `set_tags` | write | `organize` | needs a tag-capable backend (notmuch/IMAP/JMAP) |
| `move_emails` | write | `organize` | mailbox to mailbox |
| `archive_emails` | write | `organize` | to the Archive special-use mailbox |
| `save_draft` | write | `drafts` | human review before anything is sent |
| `send_email` | write | `send` | allowlist + rate limit + fixed From; audit logged |

The catalog is static and locked by a snapshot test
(`meli/src/mcp/tools_snapshot.json`), so tool schemas cannot drift
silently between versions. All tools carry the four MCP annotations
(`readOnlyHint`, `destructiveHint`, `idempotentHint`, `openWorldHint`).

Two prompts are provided: `triage_inbox(account)` and
`phishing_review(account, mailbox?)`. One resource: `email://accounts`.

## URL risk scoring

Scoring is explainable by construction: a URL (plus its anchor display
text when scanned from HTML mail) is evaluated against the signals below;
matched weights sum up (capped at 100) and the total maps to a band
(`low < 30 ≤ medium < 60 ≤ high` by default). Results always include the
fired signals with weights and details — never a black-box number.

The signal set synthesizes established practice: lexical/host-based
features from the phishing-detection literature (encoded-IP hosts,
embedded credentials, non-standard ports, subdomain depth/randomness,
hyphen-heavy and abused TLDs, brand tokens planted in subdomains,
redirect parameters), homograph defence in the spirit of Unicode TS #39
(punycode labels via a local RFC 3492 decoder, mixed-script labels,
confusable skeletons against a brand table), and the strongest HTML-mail
cue: link text pointing at a different host than the `href`.

| Signal | Default weight |
| --- | --- |
| `scheme_dangerous` (javascript/data/vbscript/file) | 60 |
| `brand_homograph` (registered domain is a lookalike of a brand) | 60 |
| `href_text_mismatch` (anchor text host ≠ href host) | 55 |
| `ip_literal_host` | 45 |
| `credentials_in_url` (user:pass@) | 45 |
| `brand_in_subdomain` (e.g. `paypal.com.attacker.ru`) | 45 |
| `mixed_script_homograph` | 45 |
| `shortener_domain` | 25 |
| `obfuscated_ip_encoding` (`2130706433`, `0x7f000001`, `127.1`) | 20 |
| `suspicious_tld` (zip/mov/top/tk/...) | 20 |
| `random_looking_subdomain` | 20 |
| `scheme_non_web` | 15 |
| `excessive_subdomains` (≥5 labels) | 15 |
| `idn_punycode_label` | 15 |
| `nonstandard_port` | 15 |
| `tracker_domain` | 10 |
| `hyphen_heavy_host` | 10 |
| `long_url` (>120 chars) | 10 |
| `heavy_percent_encoding` | 10 |
| `redirect_param` | 10 |
| `obfuscated_spelling` (defanged `hxxp://evil[.]com` in the mail) | 15 |
| `tracking_params` (utm_/mc_eid/...) | 5 |
| `mailto_classified` / `unsubscribe_classified` | 0 (classification) |

Regression tests lock the engine: a phishing corpus (brand-in-subdomain,
encoded IP, homograph domain, IP+mismatch anchor, userinfo@IP, odd
scheme/port/TLD, `data:` URL) must all score `high`; a benign corpus
(GitHub, Wikipedia, search URLs, `mailto:`, newsletter unsubscribe links,
shorteners, vendor subdomains like `outlook.office365.com`) must all stay
`low`.

**Limitations, by design:** heuristics are local — no DNS, no WHOIS, no
page fetch (SSRF-safe). A convincing attack on a never-seen benign
domain with none of these traits can still score `low`; the engine is an
assistant for triage, not a verdict.

## External threat intelligence (opt-in, not yet enabled)

Issue #215 asked where external scoring would read from and whether it
leaks mail content. Answer: the only candidate design is a
cached-first VirusTotal URL-report lookup, which would send **only the
URL string itself** — never message contents, subjects or metadata —
 gated behind an explicit `[mcp]` API key configuration, disabled by
default, with local caching and rate limiting. No such integration is
compiled in today; if you want it, confirm on the issue and it will be
added with those constraints.

## Implementation notes

- Backends are created exactly like the TUI does (same `melib`
  `MailBackend`), so maildir/IMAP/JMAP/notmuch/mbox all work; accounts
  are opened lazily on first use.
- `email_id` values are backend-opaque (maildir ids are path hashes and
  change when a flag move renames the file — re-list after mutations);
  ids are only valid within one server session.
- `search_emails` evaluates meli's query syntax locally over the account
  cache (same parser as the TUI `search` command); `body:` terms are
  matched against the extracted body text.
- Tests live next to the code (`meli/src/mcp/*` and
  `meli/src/conf/mcp.rs`): protocol round-trips, catalog snapshot,
  scoring corpora, extraction fixtures and full maildir-backed session
  tests — all offline, no shell scripts.
