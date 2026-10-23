/*
 * meli - MCP session: accounts, backends and tool implementations.
 *
 * Copyright 2026 Kyle Lee
 *
 * This file is part of meli.
 *
 * meli is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * meli is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with meli.  If not, see <http://www.gnu.org/licenses/>.
 */

//! The MCP server's account state.
//!
//! [`Session`] owns a [`Settings`] snapshot (loaded once from the user's
//! configuration file) and opens one [`AccountSession`] per account on
//! first use. Each account session wraps a melib [`MailBackend`] exactly
//! the way the TUI does — the same backend, the same flags, the same
//! maildir files — and caches the envelope list per mailbox.
//!
//! Credentials never pass through MCP: backends are created from the
//! account's own configuration, so the MCP surface has no account or
//! password tools by design.

use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    io::Write as _,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use indexmap::IndexMap;
use melib::{
    Address, Attachment, AttachmentBuilder, Collection, Draft, Envelope, EnvelopeHash, Error,
    ErrorKind, Flag, HeaderName, Mail, NetworkErrorKind, Result,
    backends::{
        BackendEventConsumer, Backends, EnvelopeHashBatch, FlagOp, MailBackend, MailboxHash,
        SpecialUsageMailbox, TagHash,
    },
    search::{Query, QueryTrait as _},
    utils::datetime,
};
use serde_json::{Value, json};

use super::{
    scoring::{self, ScoredUrl, Weights},
    urls,
};
use crate::{
    conf::composing::SendMail,
    conf::{
        self, AccountConf, Settings,
        mcp::{MCPSettings, MutationPermission},
    },
};

/// Attachments larger than this are refused by `get_attachment`.
const MAX_INLINE_ATTACHMENT_BYTES: usize = 5 * 1024 * 1024;
/// `get_emails` accepts at most this many ids per call.
const MAX_GET_EMAILS_IDS: usize = 500;
/// `score_urls` accepts at most this many URLs per call.
const MAX_SCORE_URLS: usize = 1000;
/// Page size bounds of `list_emails`/`search_emails`.
const MAX_PAGE_SIZE: usize = 100;

/// One mailbox of an account, as reported by the backend.
#[derive(Clone, Debug)]
struct MailboxEntry {
    hash: MailboxHash,
    path: String,
    name: String,
    special_use: SpecialUsageMailbox,
    /// Backend-reported `(unread, total)`, when available.
    counts: Option<(usize, usize)>,
}

/// Per-account backend state.
pub struct AccountSession {
    name: String,
    conf: AccountConf,
    backend: Box<dyn MailBackend>,
    mailboxes: Vec<MailboxEntry>,
    envelopes: HashMap<MailboxHash, Vec<Envelope>>,
    tag_index: BTreeMap<TagHash, String>,
    send_timestamps: VecDeque<Instant>,
}

impl AccountSession {
    fn open(name: &str, conf: AccountConf, backends: &Backends) -> Result<Self> {
        let creator = backends.get(&conf.account().format).map_err(|err| {
            Error::new(format!(
                "Account `{name}` has an invalid `format` setting: {err}"
            ))
            .set_kind(ErrorKind::Configuration)
        })?;
        let backend = creator(
            conf.account(),
            (Box::new(|_: &str| true) as Box<dyn Fn(&str) -> bool + Send + Sync>).into(),
            BackendEventConsumer::default(),
        )?;
        let Collection { tag_index, .. } = backend.collection();
        let tag_index = tag_index
            .read()
            .map_err(|_| Error::new("tag index poisoned"))?
            .clone();
        let mut ret = Self {
            name: name.to_string(),
            conf,
            backend,
            mailboxes: Vec::new(),
            envelopes: HashMap::new(),
            tag_index,
            send_timestamps: VecDeque::new(),
        };
        ret.refresh_mailboxes()?;
        Ok(ret)
    }

    fn refresh_mailboxes(&mut self) -> Result<()> {
        let mailboxes = smol::block_on(self.backend.mailboxes()?)?;
        let mut entries: Vec<MailboxEntry> = mailboxes
            .values()
            .map(|m| MailboxEntry {
                hash: m.hash(),
                path: m.path().to_string(),
                name: m.name().to_string(),
                special_use: m.special_usage(),
                counts: m.count().ok(),
            })
            .collect();
        // The backend does not apply [accounts.<name>.mailboxes.<path>]
        // special-use hints; meli's Account layer does that at reconcile
        // time, so do the same here: configuration hint first, then the
        // well-known-name heuristic.
        let conf_mailboxes = self.conf.account().mailboxes.clone();
        for entry in &mut entries {
            let conf_usage = conf_mailboxes
                .get(&entry.path)
                .or_else(|| conf_mailboxes.get(&entry.name))
                .and_then(|c| c.usage);
            if let Some(usage) = conf_usage {
                entry.special_use = usage;
            } else if let Some(detected) = SpecialUsageMailbox::detect_usage(&entry.name) {
                entry.special_use = detected;
            }
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        self.mailboxes = entries;
        Ok(())
    }

    fn ensure_envelopes(&mut self, mailbox_hash: MailboxHash) -> Result<()> {
        if self.envelopes.contains_key(&mailbox_hash) {
            return Ok(());
        }
        let mut stream = self.backend.fetch(mailbox_hash)?;
        let mut out: Vec<Envelope> = Vec::new();
        smol::block_on(async {
            use futures::StreamExt as _;
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(envs) => out.extend(envs),
                    Err(err) => return Err(err),
                }
            }
            Ok(())
        })?;
        out.sort_by(|a, b| {
            b.timestamp
                .cmp(&a.timestamp)
                .then_with(|| b.hash.cmp(&a.hash))
        });
        self.envelopes.insert(mailbox_hash, out);
        Ok(())
    }

    fn ensure_all_envelopes(&mut self) -> Result<()> {
        let hashes: Vec<MailboxHash> = self.mailboxes.iter().map(|m| m.hash).collect();
        for hash in hashes {
            self.ensure_envelopes(hash)?;
        }
        Ok(())
    }

    fn invalidate(&mut self, mailbox_hash: MailboxHash) {
        self.envelopes.remove(&mailbox_hash);
    }

    fn mailbox_by_hash(&self, hash: MailboxHash) -> Option<&MailboxEntry> {
        self.mailboxes.iter().find(|m| m.hash == hash)
    }

    fn resolve_mailbox(&self, path: Option<&str>) -> Result<&MailboxEntry> {
        if let Some(path) = path {
            self.mailboxes
                .iter()
                .find(|m| m.path.eq_ignore_ascii_case(path) || m.name.eq_ignore_ascii_case(path))
                .ok_or_else(|| {
                    Error::new(format!(
                        "mailbox `{path}` not found in account `{}`; call list_mailboxes for \
                         the available paths",
                        self.name
                    ))
                    .set_kind(ErrorKind::ValueError)
                })
        } else {
            self.default_mailbox()
        }
    }

    fn default_mailbox(&self) -> Result<&MailboxEntry> {
        self.mailboxes
            .iter()
            .find(|m| m.special_use == SpecialUsageMailbox::Inbox)
            .or_else(|| self.mailboxes.first())
            .ok_or_else(|| {
                Error::new(format!(
                    "account `{}` has no mailboxes; configure the account locally first",
                    self.name
                ))
                .set_kind(ErrorKind::Configuration)
            })
    }

    fn special_mailbox(&self, usage: SpecialUsageMailbox) -> Result<&MailboxEntry> {
        self.mailboxes
            .iter()
            .find(|m| m.special_use == usage)
            .ok_or_else(|| {
                Error::new(format!(
                    "account `{}` has no mailbox with special usage `{usage}`; set it in your \
                     meli configuration (see meli.conf(5) MAILBOXES)",
                    self.name
                ))
                .set_kind(ErrorKind::NotFound)
            })
    }

    /// Find an e-mail by opaque id, loading mailbox caches as needed.
    fn locate(&mut self, id: &str) -> Result<(MailboxHash, Envelope)> {
        let hash: u64 = id.parse().map_err(|_| {
            Error::new(format!("`{id}` is not a valid email_id")).set_kind(ErrorKind::ValueError)
        })?;
        let hash = EnvelopeHash(hash);
        let _ = self.ensure_all_envelopes();
        for (mailbox_hash, envs) in &self.envelopes {
            if let Some(env) = envs.iter().find(|e| e.hash == hash) {
                return Ok((*mailbox_hash, env.clone()));
            }
        }
        Err(Error::new(format!(
            "e-mail `{id}` not found in account `{}`; ids come from list_emails/search_emails \
             and are only valid within this server session",
            self.name
        ))
        .set_kind(ErrorKind::NotFound))
    }

    fn envelope_bytes(&mut self, hash: EnvelopeHash) -> Result<Vec<u8>> {
        smol::block_on(self.backend.envelope_bytes_by_hash(hash)?)
    }

    fn tags_of(&self, env: &Envelope) -> Vec<String> {
        env.tags
            .iter()
            .filter_map(|t| self.tag_index.get(t).cloned())
            .collect()
    }

    /// Audit-log a mutation. Never logs bodies, subjects or credentials.
    fn audit(&self, tool: &str, detail: &str) {
        tracing::info!(target: "mcp", "mutation `{tool}` on account `{}`: {detail}", self.name);
    }
}

/// The whole MCP server state.
pub struct Session {
    settings: Settings,
    backends: Backends,
    accounts: IndexMap<String, AccountSession>,
}

impl Session {
    /// Load the user's configuration (honouring `MELI_CONFIG` / `--config`,
    /// already handled by startup) and prepare the session.
    pub fn new() -> Result<Self> {
        let config_file: PathBuf = conf::get_config_file()?;
        if !config_file.exists() {
            return Err(Error::new(format!(
                "no meli configuration found at {}; MCP needs configured accounts (run `meli \
                 create-config`)",
                config_file.display()
            ))
            .set_kind(ErrorKind::Configuration));
        }
        let file_settings = conf::FileSettings::validate(config_file, false)?;
        let settings = Settings::from_file_settings(file_settings)?;
        Self::from_settings(settings)
    }

    /// Build a session from already-loaded settings (used by `meli mcp`
    /// and by tests).
    pub fn from_settings(settings: Settings) -> Result<Self> {
        settings.mcp.validate()?;
        Ok(Self {
            settings,
            backends: Backends::new(),
            accounts: IndexMap::new(),
        })
    }

    pub fn mcp_settings(&self) -> &MCPSettings {
        &self.settings.mcp
    }

    fn account_names(&self) -> Vec<String> {
        self.settings.accounts.keys().cloned().collect()
    }

    fn account(&mut self, name: &str) -> Result<&mut AccountSession> {
        if !self.accounts.contains_key(name) {
            let conf = self.settings.accounts.get(name).ok_or_else(|| {
                Error::new(format!(
                    "unknown account `{name}`; available accounts: {}",
                    self.account_names().join(", ")
                ))
                .set_kind(ErrorKind::ValueError)
            })?;
            let session = AccountSession::open(name, conf.clone(), &self.backends)?;
            self.accounts.insert(name.to_string(), session);
        }
        Ok(self.accounts.get_mut(name).unwrap())
    }

    fn require_mutation(&self, perm: MutationPermission) -> Result<()> {
        if self.settings.mcp.allows(perm) {
            Ok(())
        } else {
            Err(Error::new(format!(
                "permission denied: this tool needs mutation class `{perm}`. The MCP server is \
                 read-only until you add `{perm}` to `[mcp] allowed_mutations` in your meli \
                 configuration file (see meli.conf(5) MCP section)."
            ))
            .set_kind(ErrorKind::None))
        }
    }

    // ------------------------------------------------------------------
    // Tool implementations
    // ------------------------------------------------------------------

    pub fn tool_list_accounts(&mut self, _args: &Value) -> Result<Value> {
        let items: Vec<Value> = self
            .settings
            .accounts
            .iter()
            .map(|(name, conf)| {
                json!({
                    "account_name": name,
                    "account_type": conf.account().format,
                    "email_address": conf.account().identity,
                    "display_name": conf.account().display_name,
                    "can_receive": true,
                    "can_send": send_mail_enabled(&conf.conf.send_mail),
                    "read_only": conf.account().read_only,
                    "description":
                        "accounts and credentials are managed in the local meli configuration, \
                         never through MCP",
                })
            })
            .collect();
        Ok(json!({ "accounts": items }))
    }

    pub fn tool_list_mailboxes(&mut self, args: &Value) -> Result<Value> {
        #[derive(Deserialize)]
        struct Args {
            account: String,
        }
        let args: Args = serde_json::from_value(args.clone())?;
        let account = self.account(&args.account)?;
        let items: Vec<Value> = account
            .mailboxes
            .iter()
            .map(|m| {
                let counts = account
                    .envelopes
                    .get(&m.hash)
                    .map(|envs| {
                        let unread = envs
                            .iter()
                            .filter(|e| !e.flags.contains(Flag::SEEN))
                            .count();
                        (unread, envs.len())
                    })
                    .or(m.counts);
                json!({
                    "name": m.name,
                    "path": m.path,
                    "special_use": m.special_use.to_string().to_lowercase(),
                    "unread": counts.map(|c| c.0),
                    "total": counts.map(|c| c.1),
                })
            })
            .collect();
        Ok(json!({ "mailboxes": items }))
    }

    pub fn tool_list_emails(&mut self, args: &Value) -> Result<Value> {
        #[derive(Deserialize, Default)]
        struct Filters {
            from: Option<String>,
            to: Option<String>,
            subject: Option<String>,
            #[serde(rename = "since")]
            since: Option<String>,
            before: Option<String>,
            seen: Option<bool>,
            flagged: Option<bool>,
            answered: Option<bool>,
            has_attachment: Option<bool>,
            tags: Option<Vec<String>>,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            account: String,
            mailbox: Option<String>,
            #[serde(default)]
            filters: Filters,
            #[serde(default = "default_page")]
            page: usize,
            #[serde(default = "default_page_size")]
            page_size: usize,
            #[serde(default = "default_desc")]
            order: String,
        }
        fn default_page() -> usize {
            1
        }
        fn default_page_size() -> usize {
            20
        }
        fn default_desc() -> String {
            "desc".to_string()
        }
        let args: Args = serde_json::from_value(args.clone())?;
        if args.page == 0 {
            return Err(bad_request("page must be >= 1"));
        }
        if args.page_size == 0 || args.page_size > MAX_PAGE_SIZE {
            return Err(bad_request(&format!(
                "page_size must be in 1..={MAX_PAGE_SIZE}"
            )));
        }
        let asc = match args.order.as_str() {
            "asc" => true,
            "desc" => false,
            other => {
                return Err(bad_request(&format!(
                    "order must be \"asc\" or \"desc\", got `{other}`"
                )));
            }
        };
        let since = parse_date_opt(args.filters.since.as_deref(), "filters.since")?;
        let before = parse_date_opt(args.filters.before.as_deref(), "filters.before")?;
        let account = self.account(&args.account)?;
        let mailbox = account.resolve_mailbox(args.mailbox.as_deref())?.clone();
        account.ensure_envelopes(mailbox.hash)?;
        let envs: Vec<Envelope> = account
            .envelopes
            .get(&mailbox.hash)
            .cloned()
            .unwrap_or_default();
        let filtered: Vec<&Envelope> = envs
            .iter()
            .filter(|e| {
                let f = &args.filters;
                if let Some(from) = &f.from {
                    if !e
                        .from()
                        .iter()
                        .any(|a| a.to_string().to_lowercase().contains(&from.to_lowercase()))
                    {
                        return false;
                    }
                }
                if let Some(to) = &f.to {
                    if !e
                        .to()
                        .iter()
                        .any(|a| a.to_string().to_lowercase().contains(&to.to_lowercase()))
                    {
                        return false;
                    }
                }
                if let Some(subject) = &f.subject {
                    if !e.subject().to_lowercase().contains(&subject.to_lowercase()) {
                        return false;
                    }
                }
                if let Some(since) = since {
                    if e.timestamp < since {
                        return false;
                    }
                }
                if let Some(before) = before {
                    if e.timestamp > before {
                        return false;
                    }
                }
                if let Some(seen) = f.seen {
                    if e.flags.contains(Flag::SEEN) != seen {
                        return false;
                    }
                }
                if let Some(flagged) = f.flagged {
                    if e.flags.contains(Flag::FLAGGED) != flagged {
                        return false;
                    }
                }
                if let Some(answered) = f.answered {
                    if e.flags.contains(Flag::REPLIED) != answered {
                        return false;
                    }
                }
                if let Some(has_attachment) = f.has_attachment {
                    if e.has_attachments() != has_attachment {
                        return false;
                    }
                }
                if let Some(wanted) = &f.tags {
                    let have = account.tags_of(e);
                    if !wanted
                        .iter()
                        .any(|w| have.iter().any(|h| h.eq_ignore_ascii_case(w)))
                    {
                        return false;
                    }
                }
                true
            })
            .collect();
        let total = filtered.len();
        let items: Vec<Value> = paginate(&filtered, args.page, args.page_size, asc)
            .iter()
            .map(|e| envelope_summary(e, &mailbox.path, &account.tag_index))
            .collect();
        Ok(json!({
            "total": total,
            "page": args.page,
            "page_size": args.page_size,
            "mailbox": mailbox.path,
            "emails": items,
        }))
    }

    pub fn tool_get_emails(&mut self, args: &Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            account: String,
            mailbox: Option<String>,
            email_ids: Vec<String>,
            #[serde(default)]
            mark_as_read: bool,
            #[serde(default)]
            body_offset: usize,
            #[serde(default = "default_max_body")]
            max_body_length: usize,
        }
        fn default_max_body() -> usize {
            20_000
        }
        let args: Args = serde_json::from_value(args.clone())?;
        if args.email_ids.is_empty() || args.email_ids.len() > MAX_GET_EMAILS_IDS {
            return Err(bad_request(&format!(
                "email_ids must contain 1..={MAX_GET_EMAILS_IDS} ids"
            )));
        }
        if args.max_body_length == 0 || args.max_body_length > 100_000 {
            return Err(bad_request("max_body_length must be in 1..=100000"));
        }
        if args.mark_as_read {
            self.require_mutation(MutationPermission::Organize)?;
        }
        let account = self.account(&args.account)?;
        let mailbox_hint = args.mailbox.as_deref().and_then(|p| {
            account
                .mailboxes
                .iter()
                .find(|m| m.path.eq_ignore_ascii_case(p) || m.name.eq_ignore_ascii_case(p))
                .map(|m| m.hash)
        });
        if let Some(hash) = mailbox_hint {
            account.ensure_envelopes(hash)?;
        }
        let mut items = Vec::with_capacity(args.email_ids.len());
        let mut failed = Vec::new();
        for id in &args.email_ids {
            match fetch_full_email(account, id, mailbox_hint) {
                Ok((path, mail)) => {
                    items.push(render_full_email(
                        id,
                        &mail,
                        &path,
                        args.body_offset,
                        args.max_body_length,
                    ));
                }
                Err(err) => failed.push(json!({"email_id": id, "error": err.to_string()})),
            }
        }
        if args.mark_as_read {
            // Group the ids by the mailbox they actually live in.
            let mut groups: HashMap<MailboxHash, Vec<EnvelopeHash>> = HashMap::new();
            for id in &args.email_ids {
                let (mailbox_hash, env) = account.locate(id)?;
                groups.entry(mailbox_hash).or_default().push(env.hash);
            }
            account.audit("get_emails", "mark_as_read");
            for (mailbox_hash, hashes) in groups {
                if let Some(batch) = non_empty_batch(hashes) {
                    smol::block_on(account.backend.set_flags(
                        batch,
                        mailbox_hash,
                        vec![FlagOp::Set(Flag::SEEN)],
                    )?)?;
                    account.invalidate(mailbox_hash);
                }
            }
        }
        Ok(json!({ "emails": items, "failed": failed }))
    }

    pub fn tool_get_thread(&mut self, args: &Value) -> Result<Value> {
        #[derive(Deserialize)]
        struct Args {
            account: String,
            email_id: String,
        }
        let args: Args = serde_json::from_value(args.clone())?;
        let account = self.account(&args.account)?;
        account.ensure_all_envelopes()?;
        let target_hash: u64 = args.email_id.parse().map_err(|_| {
            Error::new(format!("`{}` is not a valid email_id", args.email_id))
                .set_kind(ErrorKind::ValueError)
        })?;
        let target_hash = EnvelopeHash(target_hash);
        let mut all: Vec<(MailboxHash, Envelope)> = account
            .envelopes
            .iter()
            .flat_map(|(h, envs)| envs.iter().map(move |e| (*h, e.clone())))
            .collect();
        all.sort_by_key(|a| a.1.timestamp);
        let Some((_, target)) = all.iter().find(|(_, e)| e.hash == target_hash) else {
            return Err(Error::new(format!(
                "e-mail `{}` not found in account `{}`",
                args.email_id, args.account
            ))
            .set_kind(ErrorKind::NotFound));
        };
        let normalize = |mid: &str| {
            mid.trim()
                .trim_start_matches('<')
                .trim_end_matches('>')
                .to_lowercase()
        };
        // Seed with the target's ancestry, then close over replies.
        let mut mids: HashSet<String> = target
            .references()
            .iter()
            .map(|m| normalize(m.to_string().trim()))
            .collect();
        if let Some(irt) = target.in_reply_to() {
            for r in irt.refs() {
                mids.insert(normalize(r.to_string().trim()));
            }
        }
        mids.insert(normalize(target.message_id().to_string().trim()));
        for _ in 0..5 {
            let before = mids.len();
            for (_, e) in &all {
                let own = normalize(e.message_id().to_string().trim());
                if mids.contains(&own) {
                    continue;
                }
                if e.references()
                    .iter()
                    .any(|r| mids.contains(&normalize(r.to_string().trim())))
                {
                    mids.insert(own);
                }
            }
            if mids.len() == before {
                break;
            }
        }
        let items: Vec<Value> = all
            .iter()
            .filter(|(_, e)| mids.contains(&normalize(e.message_id().to_string().trim())))
            .map(|(mailbox_hash, e)| {
                let path = account
                    .mailbox_by_hash(*mailbox_hash)
                    .map(|m| m.path.clone())
                    .unwrap_or_default();
                json!({
                    "email_id": e.hash.to_string(),
                    "mailbox": path,
                    "message_id": e.message_id().to_string(),
                    "subject": e.subject(),
                    "from": address_strings(&e.from),
                    "date": timestamp_to_rfc3339(e.timestamp),
                    "in_reply_to": e.in_reply_to().and_then(|r| r.refs().first().map(|m| m.to_string())),
                })
            })
            .collect();
        Ok(json!({ "emails": items }))
    }

    pub fn tool_search_emails(&mut self, args: &Value) -> Result<Value> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            account: String,
            query: String,
            mailbox: Option<String>,
            #[serde(default = "default_page")]
            page: usize,
            #[serde(default = "default_page_size")]
            page_size: usize,
        }
        fn default_page() -> usize {
            1
        }
        fn default_page_size() -> usize {
            20
        }
        let args: Args = serde_json::from_value(args.clone())?;
        if args.page == 0 {
            return Err(bad_request("page must be >= 1"));
        }
        if args.page_size == 0 || args.page_size > MAX_PAGE_SIZE {
            return Err(bad_request(&format!(
                "page_size must be in 1..={MAX_PAGE_SIZE}"
            )));
        }
        let query = Query::try_from(args.query.as_str())
            .map_err(|err| bad_request(&format!("invalid query `{}`: {err}", args.query)))?;
        let needs_body = query_needs_body(&query);
        let account = self.account(&args.account)?;
        let mut matches: Vec<(MailboxHash, Envelope)> = Vec::new();
        let hashes: Vec<MailboxHash> = match &args.mailbox {
            Some(path) => vec![account.resolve_mailbox(Some(path))?.hash],
            None => account.mailboxes.iter().map(|m| m.hash).collect(),
        };
        for hash in hashes {
            account.ensure_envelopes(hash)?;
            let envs: Vec<Envelope> = account.envelopes.get(&hash).cloned().unwrap_or_default();
            for e in envs {
                let ok = if needs_body {
                    match account.envelope_bytes(e.hash) {
                        Ok(bytes) => {
                            let text = body_text_of_bytes(&bytes);
                            query_matches(&query, &e, Some(&text))
                        }
                        Err(_) => e.is_match(&query),
                    }
                } else {
                    e.is_match(&query)
                };
                if ok {
                    matches.push((hash, e));
                }
            }
        }
        matches.sort_by(|a, b| {
            b.1.timestamp
                .cmp(&a.1.timestamp)
                .then_with(|| b.1.hash.cmp(&a.1.hash))
        });
        let total = matches.len();
        let start = (args.page - 1) * args.page_size;
        let items: Vec<Value> = matches
            .iter()
            .skip(start)
            .take(args.page_size)
            .map(|(hash, e)| {
                let path = account
                    .mailbox_by_hash(*hash)
                    .map(|m| m.path.clone())
                    .unwrap_or_default();
                envelope_summary(e, &path, &account.tag_index)
            })
            .collect();
        Ok(json!({
            "total": total,
            "page": args.page,
            "page_size": args.page_size,
            "query": args.query,
            "emails": items,
        }))
    }

    pub fn tool_get_attachment(&mut self, args: &Value) -> Result<Value> {
        #[derive(Deserialize)]
        struct Args {
            account: String,
            email_id: String,
            filename: String,
        }
        let args: Args = serde_json::from_value(args.clone())?;
        if args.filename.contains('/')
            || args.filename.contains('\\')
            || args.filename.contains("..")
        {
            return Err(bad_request(
                "filename must be a plain attachment file name, not a path",
            ));
        }
        let account = self.account(&args.account)?;
        let (_, env) = account.locate(&args.email_id)?;
        let bytes = account.envelope_bytes(env.hash)?;
        let root = AttachmentBuilder::new(&bytes).build();
        let wanted = args.filename.as_str();
        let Some(att) = find_attachment(&root, wanted) else {
            return Err(Error::new(format!(
                "attachment `{wanted}` not found in e-mail `{}`",
                args.email_id
            ))
            .set_kind(ErrorKind::NotFound));
        };
        let data = att.decode(Default::default());
        if data.len() > MAX_INLINE_ATTACHMENT_BYTES {
            return Err(Error::new(format!(
                "attachment `{wanted}` is {} bytes, over the {} byte inline limit; save it \
                 locally from the TUI instead",
                data.len(),
                MAX_INLINE_ATTACHMENT_BYTES
            ))
            .set_kind(ErrorKind::ValueError));
        }
        Ok(json!({
            "filename": wanted,
            "mime": att.mime_type(),
            "size": data.len(),
            "blob_base64": base64_encode(&data),
        }))
    }

    pub fn tool_extract_urls(&mut self, args: &Value) -> Result<Value> {
        #[derive(Deserialize)]
        struct Args {
            account: String,
            email_id: String,
        }
        let args: Args = serde_json::from_value(args.clone())?;
        let account = self.account(&args.account)?;
        let (_, env) = account.locate(&args.email_id)?;
        let bytes = account.envelope_bytes(env.hash)?;
        let root = AttachmentBuilder::new(&bytes).build();
        let items = urls::extract_from_attachment(&root);
        Ok(json!({ "urls": items }))
    }

    pub fn tool_score_urls(&mut self, args: &Value) -> Result<Value> {
        #[derive(Deserialize)]
        struct Args {
            urls: Vec<String>,
        }
        let args: Args = serde_json::from_value(args.clone())?;
        if args.urls.len() > MAX_SCORE_URLS {
            return Err(bad_request(&format!(
                "at most {MAX_SCORE_URLS} urls per call"
            )));
        }
        let mcp = self.settings.mcp.clone();
        let weights = Weights::new(|name, default| mcp.signal_weight(name, default));
        let low = mcp.low_medium_threshold();
        let high = mcp.medium_high_threshold();
        let items: Vec<ScoredUrl> = args
            .urls
            .iter()
            .map(|u| scoring::score_url(u, None, &weights, low, high))
            .collect();
        Ok(json!({ "results": items }))
    }

    pub fn tool_scan_email_urls(&mut self, args: &Value) -> Result<Value> {
        #[derive(Deserialize)]
        struct Args {
            account: String,
            email_id: String,
        }
        let args: Args = serde_json::from_value(args.clone())?;
        let mcp = self.settings.mcp.clone();
        let weights = Weights::new(|name, default| mcp.signal_weight(name, default));
        let low = mcp.low_medium_threshold();
        let high = mcp.medium_high_threshold();
        let account = self.account(&args.account)?;
        let (_, env) = account.locate(&args.email_id)?;
        let bytes = account.envelope_bytes(env.hash)?;
        let root = AttachmentBuilder::new(&bytes).build();
        let extracted = urls::extract_from_attachment(&root);
        let items: Vec<ScoredUrl> = extracted
            .iter()
            .map(|u| {
                let mut scored =
                    scoring::score_url(&u.url, u.display_text.as_deref(), &weights, low, high);
                if u.obfuscated {
                    scored.signals.push(scoring::SignalHit {
                        name: "obfuscated_spelling",
                        weight: weights.get("obfuscated_spelling"),
                        detail: format!(
                            "the link was de-obfuscated from a defanged spelling to `{}`",
                            u.url
                        ),
                    });
                    scored.signals.sort_unstable_by(|a, b| {
                        b.weight.cmp(&a.weight).then_with(|| a.name.cmp(b.name))
                    });
                    scored.score = scored
                        .signals
                        .iter()
                        .map(|h| h.weight as u32)
                        .sum::<u32>()
                        .min(100) as u8;
                    scored.risk = scoring::Risk::of(scored.score, low, high);
                }
                scored
            })
            .collect();
        Ok(json!({ "urls": items }))
    }

    pub fn tool_set_flags(&mut self, args: &Value) -> Result<Value> {
        #[derive(Deserialize)]
        struct Flags {
            seen: Option<bool>,
            flagged: Option<bool>,
            answered: Option<bool>,
        }
        #[derive(Deserialize)]
        struct Args {
            account: String,
            mailbox: String,
            email_ids: Vec<String>,
            flags: Flags,
        }
        let args: Args = serde_json::from_value(args.clone())?;
        self.require_mutation(MutationPermission::Organize)?;
        let ops = flag_ops(args.flags.seen, args.flags.flagged, args.flags.answered);
        if ops.is_empty() {
            return Err(bad_request(
                "set at least one of flags.seen / flags.flagged / flags.answered",
            ));
        }
        let account = self.account(&args.account)?;
        let mailbox = account.resolve_mailbox(Some(&args.mailbox))?.clone();
        account.ensure_envelopes(mailbox.hash)?;
        let hashes = parse_ids(&args.email_ids, &account.envelopes[&mailbox.hash])?;
        let count = hashes.len();
        let Some(batch) = non_empty_batch(hashes) else {
            return Err(bad_request("email_ids must not be empty"));
        };
        account.audit("set_flags", &format!("{count} message(s)"));
        smol::block_on(account.backend.set_flags(batch, mailbox.hash, ops)?)?;
        account.invalidate(mailbox.hash);
        Ok(json!({ "updated": count, "mailbox": mailbox.path }))
    }

    pub fn tool_set_tags(&mut self, args: &Value) -> Result<Value> {
        #[derive(Deserialize)]
        struct Args {
            account: String,
            mailbox: String,
            email_ids: Vec<String>,
            add: Vec<String>,
            #[serde(default)]
            remove: Vec<String>,
        }
        let args: Args = serde_json::from_value(args.clone())?;
        self.require_mutation(MutationPermission::Organize)?;
        if args.add.is_empty() && args.remove.is_empty() {
            return Err(bad_request("add and remove cannot both be empty"));
        }
        let account = self.account(&args.account)?;
        if !account.backend.capabilities().supports_tags {
            return Err(Error::new(format!(
                "account `{}` does not support tags with the `{}` backend",
                args.account,
                account.conf.account().format
            ))
            .set_kind(ErrorKind::NotSupported));
        }
        let mailbox = account.resolve_mailbox(Some(&args.mailbox))?.clone();
        account.ensure_envelopes(mailbox.hash)?;
        let hashes = parse_ids(&args.email_ids, &account.envelopes[&mailbox.hash])?;
        let count = hashes.len();
        let Some(batch) = non_empty_batch(hashes) else {
            return Err(bad_request("email_ids must not be empty"));
        };
        let mut ops: Vec<FlagOp> = args
            .add
            .iter()
            .map(|t| FlagOp::SetTag(t.clone()))
            .chain(args.remove.iter().map(|t| FlagOp::UnSetTag(t.clone())))
            .collect();
        ops.dedup();
        account.audit("set_tags", &format!("{count} message(s)"));
        smol::block_on(account.backend.set_flags(batch, mailbox.hash, ops)?)?;
        account.invalidate(mailbox.hash);
        Ok(json!({ "updated": count, "mailbox": mailbox.path }))
    }

    pub fn tool_move_emails(&mut self, args: &Value) -> Result<Value> {
        #[derive(Deserialize)]
        struct Args {
            account: String,
            mailbox: String,
            email_ids: Vec<String>,
            to_mailbox: String,
        }
        let args: Args = serde_json::from_value(args.clone())?;
        self.require_mutation(MutationPermission::Organize)?;
        let account = self.account(&args.account)?;
        let from = account.resolve_mailbox(Some(&args.mailbox))?.clone();
        let to = account.resolve_mailbox(Some(&args.to_mailbox))?.clone();
        if from.hash == to.hash {
            return Err(bad_request("source and destination mailboxes are the same"));
        }
        account.ensure_envelopes(from.hash)?;
        let hashes = parse_ids(&args.email_ids, &account.envelopes[&from.hash])?;
        let count = hashes.len();
        let Some(batch) = non_empty_batch(hashes) else {
            return Err(bad_request("email_ids must not be empty"));
        };
        account.audit(
            "move_emails",
            &format!("{count} message(s) to `{}`", to.path),
        );
        smol::block_on(
            account
                .backend
                .copy_messages(batch, from.hash, to.hash, true)?,
        )?;
        account.invalidate(from.hash);
        account.invalidate(to.hash);
        Ok(json!({
            "moved": count,
            "from_mailbox": from.path,
            "to_mailbox": to.path,
        }))
    }

    pub fn tool_archive_emails(&mut self, args: &Value) -> Result<Value> {
        #[derive(Deserialize)]
        struct Args {
            account: String,
            mailbox: String,
            email_ids: Vec<String>,
        }
        let args: Args = serde_json::from_value(args.clone())?;
        self.require_mutation(MutationPermission::Organize)?;
        let account = self.account(&args.account)?;
        let archive = account
            .special_mailbox(SpecialUsageMailbox::Archive)?
            .clone();
        let move_args = json!({
            "account": args.account,
            "mailbox": args.mailbox,
            "email_ids": args.email_ids,
            "to_mailbox": archive.path,
        });
        self.tool_move_emails(&move_args)
    }

    pub fn tool_save_draft(&mut self, args: &Value) -> Result<Value> {
        #[derive(Deserialize)]
        struct Args {
            account: String,
            to: Vec<String>,
            #[serde(default)]
            cc: Vec<String>,
            #[serde(default)]
            bcc: Vec<String>,
            #[serde(default)]
            subject: String,
            body: String,
        }
        let args: Args = serde_json::from_value(args.clone())?;
        self.require_mutation(MutationPermission::Drafts)?;
        let mut draft = new_draft_for(&args.account, &self.settings)?;
        let _ = draft
            .set_header(HeaderName::TO, args.to.join(", "))
            .set_header(HeaderName::CC, args.cc.join(", "))
            .set_header(HeaderName::BCC, args.bcc.join(", "))
            .set_header(HeaderName::SUBJECT, args.subject.clone())
            .set_body(args.body);
        let message = draft.finalise()?;
        let account = self.account(&args.account)?;
        let drafts = account
            .special_mailbox(SpecialUsageMailbox::Drafts)?
            .clone();
        account.audit("save_draft", "1 draft");
        smol::block_on(account.backend.save(
            message.into_bytes(),
            drafts.hash,
            Some(Flag::DRAFT | Flag::SEEN),
        )?)?;
        account.invalidate(drafts.hash);
        Ok(json!({ "saved": true, "mailbox": drafts.path }))
    }

    pub fn tool_send_email(&mut self, args: &Value) -> Result<Value> {
        #[derive(Deserialize)]
        struct Args {
            account: String,
            to: Vec<String>,
            #[serde(default)]
            cc: Vec<String>,
            #[serde(default)]
            bcc: Vec<String>,
            subject: String,
            body: String,
            in_reply_to: Option<String>,
            #[serde(default)]
            references: Vec<String>,
        }
        let args: Args = serde_json::from_value(args.clone())?;
        self.require_mutation(MutationPermission::Send)?;
        if args.to.is_empty() {
            return Err(bad_request("`to` must contain at least one address"));
        }
        let mcp = self.settings.mcp.clone();
        let mut recipients = args.to.clone();
        recipients.extend(args.cc.iter().cloned());
        recipients.extend(args.bcc.iter().cloned());
        for recipient in &recipients {
            if !allowlist_matches(&mcp.send_recipient_allowlist, recipient) {
                return Err(Error::new(format!(
                    "recipient `{recipient}` is not allowed by [mcp] send_recipient_allowlist \
                     (empty list = all recipients denied). Ask the user to extend the allowlist \
                     in the meli configuration instead of working around it."
                ))
                .set_kind(ErrorKind::None));
            }
        }
        // Rate limit: per account, sliding one-minute window.
        let limit = mcp.send_rate_limit_per_minute;
        let now = Instant::now();
        let rate_gate = {
            let account = self.account(&args.account)?;
            while account
                .send_timestamps
                .front()
                .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60))
            {
                account.send_timestamps.pop_front();
            }
            if account.send_timestamps.len() >= limit {
                Err(Error::new(format!(
                    "send rate limit reached ({limit}/minute for account `{}`); retry in under                      a minute",
                    args.account
                ))
                .set_kind(ErrorKind::Network(NetworkErrorKind::None)))
            } else {
                Ok(())
            }
        };
        rate_gate?;
        let mut draft = new_draft_for(&args.account, &self.settings)?;
        let _ = draft
            .set_header(HeaderName::TO, args.to.join(", "))
            .set_header(HeaderName::CC, args.cc.join(", "))
            .set_header(HeaderName::BCC, args.bcc.join(", "))
            .set_header(HeaderName::SUBJECT, args.subject)
            .set_body(args.body);
        if let Some(irt) = &args.in_reply_to {
            draft.set_header(HeaderName::IN_REPLY_TO, angle_bracket(irt));
        }
        if !args.references.is_empty() {
            draft.set_header(
                HeaderName::REFERENCES,
                args.references
                    .iter()
                    .map(|r| angle_bracket(r))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        let identity = self
            .settings
            .accounts
            .get(&args.account)
            .map(|c| c.account().identity.clone())
            .unwrap_or_default();
        let fqdn = identity
            .rsplit('@')
            .next()
            .filter(|s| !s.is_empty() && *s != identity)
            .unwrap_or("localhost")
            .to_string();
        let message_id = melib::email::compose::random::gen_message_id(&fqdn);
        let _ = draft.set_header(HeaderName::MESSAGE_ID, message_id.clone());
        let message = draft.finalise()?;
        let account = self.account(&args.account)?;
        account.send_timestamps.push_back(now);
        let send_mail = account.conf.conf.send_mail.clone();
        account.audit("send_email", &format!("{} recipient(s)", recipients.len()));
        match send_mail {
            SendMail::ShellCommand(command) => {
                if command.is_empty() {
                    return Err(Error::new(
                        "send_mail shell command configuration value is empty",
                    ));
                }
                let mut child = Command::new("sh")
                    .args(["-c", &command])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .map_err(|err| {
                        Error::new(format!("could not start mailer command `{command}`: {err}"))
                            .set_kind(ErrorKind::Network(NetworkErrorKind::None))
                    })?;
                let Some(stdin) = child.stdin.as_mut() else {
                    return Err(Error::new("could not open stdin of mailer command"));
                };
                stdin.write_all(message.as_bytes()).map_err(|err| {
                    Error::new(format!("could not write message to mailer stdin: {err}"))
                })?;
                drop(child.stdin.take());
                let output = child
                    .wait()
                    .map_err(|err| Error::new(format!("could not wait on mailer: {err}")))?;
                if !output.success() {
                    return Err(Error::new(format!(
                        "mailer command `{command}` did not accept the message (exit status: \
                         {:?})",
                        output.code()
                    ))
                    .set_summary("Message not sent."));
                }
            }
            SendMail::ServerSubmission => {
                return Err(Error::new(
                    "send_mail = \"server_submission\" is not available through MCP; use an \
                     SMTP transport or shell command in the account configuration",
                )
                .set_kind(ErrorKind::NotSupported));
            }
            #[cfg(feature = "smtp")]
            SendMail::Smtp(smtp_conf) => {
                smol::block_on(async {
                    let mut conn = melib::smtp::SmtpConnection::new_connection(smtp_conf).await?;
                    conn.mail_transaction(&message, None).await
                })?;
            }
            #[cfg(not(feature = "smtp"))]
            SendMail::Smtp(_) => {
                return Err(Error::new(
                    "this build of meli has no SMTP support (feature `smtp` disabled); use a \
                     shell command transport",
                )
                .set_kind(ErrorKind::NotSupported));
            }
        }
        Ok(json!({ "accepted": true, "message_id": message_id }))
    }
}

// ----------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------

/// Whether the account's transport can actually send (the default
/// `send_mail` is the `false` shell command, i.e. disabled).
fn send_mail_enabled(send_mail: &SendMail) -> bool {
    match send_mail {
        SendMail::ShellCommand(cmd) => cmd != "false",
        SendMail::ServerSubmission => true,
        #[cfg(feature = "smtp")]
        SendMail::Smtp(_) => true,
    }
}

fn bad_request(msg: &str) -> Error {
    Error::new(msg.to_string()).set_kind(ErrorKind::ValueError)
}

fn parse_date_opt(s: Option<&str>, field: &str) -> Result<Option<u64>> {
    match s {
        None => Ok(None),
        Some(s) => datetime::rfc3339_to_timestamp(s)
            .map(Some)
            .map_err(|_| bad_request(&format!("{field} must be an RFC3339 date, got `{s}`"))),
    }
}

fn paginate<'a>(
    envs: &[&'a Envelope],
    page: usize,
    page_size: usize,
    asc: bool,
) -> Vec<&'a Envelope> {
    let start = (page - 1) * page_size;
    // `envs` is stored newest-first; ascending order simply reverses.
    let iter = envs.iter().copied();
    let mut page_items: Vec<&Envelope> = if asc {
        iter.rev().collect()
    } else {
        iter.collect()
    };
    if start >= page_items.len() {
        page_items.clear();
    } else {
        page_items.drain(..start);
        page_items.truncate(page_size);
    }
    page_items
}

fn flag_strings(flags: Flag) -> Vec<&'static str> {
    let mut out = Vec::new();
    if flags.contains(Flag::SEEN) {
        out.push("seen");
    }
    if flags.contains(Flag::FLAGGED) {
        out.push("flagged");
    }
    if flags.contains(Flag::REPLIED) {
        out.push("answered");
    }
    if flags.contains(Flag::DRAFT) {
        out.push("draft");
    }
    if flags.contains(Flag::PASSED) {
        out.push("passed");
    }
    if flags.contains(Flag::TRASHED) {
        out.push("trashed");
    }
    out
}

fn address_strings(addrs: &[Address]) -> Vec<String> {
    addrs.iter().map(|a| a.to_string()).collect()
}

fn envelope_summary(
    env: &Envelope,
    mailbox_path: &str,
    tag_index: &BTreeMap<melib::backends::TagHash, String>,
) -> Value {
    json!({
        "email_id": env.hash.to_string(),
        "mailbox": mailbox_path,
        "message_id": env.message_id().to_string(),
        "subject": env.subject(),
        "from": address_strings(&env.from),
        "to": address_strings(&env.to),
        "date": timestamp_to_rfc3339(env.timestamp),
        "timestamp": env.timestamp,
        "flags": flag_strings(env.flags),
        "tags": env.tags.iter().filter_map(|t| tag_index.get(t).cloned()).collect::<Vec<_>>(),
        "has_attachment": env.has_attachments(),
    })
}

fn timestamp_to_rfc3339(ts: u64) -> String {
    datetime::timestamp_to_string(ts, Some(datetime::formats::RFC3339_DATETIME), true)
}

/// Read the raw bytes and full parse of one e-mail by opaque id.
fn fetch_full_email(
    account: &mut AccountSession,
    id: &str,
    mailbox_hint: Option<MailboxHash>,
) -> Result<(String, Mail)> {
    let hash: u64 = id
        .parse()
        .map_err(|_| bad_request(&format!("`{id}` is not a valid email_id")))?;
    let hash = EnvelopeHash(hash);
    // Locate without re-loading everything when a hint matches.
    let found: Option<(MailboxHash, Envelope)> = mailbox_hint.and_then(|mh| {
        account
            .envelopes
            .get(&mh)
            .and_then(|envs| envs.iter().find(|e| e.hash == hash))
            .map(|e| (mh, e.clone()))
    });
    let (mailbox_hash, env) = match found {
        Some(found) => found,
        None => {
            let _ = account.locate(id)?;
            account
                .envelopes
                .iter()
                .find_map(|(mh, envs)| {
                    envs.iter()
                        .find(|e| e.hash == hash)
                        .map(|e| (*mh, e.clone()))
                })
                .ok_or_else(|| {
                    Error::new(format!("e-mail `{id}` not found")).set_kind(ErrorKind::NotFound)
                })?
        }
    };
    let path = account
        .mailbox_by_hash(mailbox_hash)
        .map(|m| m.path.clone())
        .unwrap_or_default();
    let bytes = account.envelope_bytes(env.hash)?;
    let mail = Mail::new(bytes, None)?;
    Ok((path, mail))
}

fn render_full_email(
    id: &str,
    mail: &Mail,
    mailbox_path: &str,
    offset: usize,
    max: usize,
) -> Value {
    let body_text = extract_body_text(&mail.body());
    let total_chars = body_text.chars().count();
    let skipped = offset.min(total_chars);
    let taken: String = body_text.chars().skip(skipped).take(max).collect();
    let truncated = skipped + taken.chars().count() < total_chars;
    let mut window = taken;
    if truncated {
        window.push_str("...[TRUNCATED]");
    }
    let headers: serde_json::Map<String, Value> =
        melib::email::parser::headers::headers(mail.bytes())
            .map(|(_, list)| {
                list.iter()
                    .map(|(name, value)| {
                        (
                            name.to_string(),
                            Value::String(String::from_utf8_lossy(value).trim().to_string()),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
    let mut attachments = Vec::new();
    collect_attachments(&mail.body(), &mut attachments, 0);
    json!({
        "email_id": id,
        "mailbox": mailbox_path,
        "headers": headers,
        "message_id": mail.message_id().to_string(),
        "subject": mail.subject(),
        "from": address_strings(&mail.from),
        "to": address_strings(&mail.to),
        "date": timestamp_to_rfc3339(mail.timestamp),
        "body_text": window,
        "body_length": total_chars,
        "body_offset": skipped,
        "truncated": truncated,
        "attachments": attachments,
        "in_reply_to": mail.in_reply_to().and_then(|r| r.refs().first().map(|m| m.to_string())),
        "references": mail.references().iter().map(|m| m.to_string()).collect::<Vec<_>>(),
    })
}

/// Extract the human-readable body: prefer inline `text/plain`, fall back
/// to sanitized `text/html` rendered to text.
pub fn extract_body_text(root: &Attachment) -> String {
    #[derive(Default)]
    struct Parts {
        plain: Vec<String>,
        html: Vec<String>,
    }
    fn walk(part: &Attachment, parts: &mut Parts, depth: usize) {
        if depth > 10 {
            return;
        }
        if part.content_disposition.kind.is_attachment() {
            return;
        }
        match &part.content_type {
            melib::email::attachment_types::ContentType::Text { .. } => {
                let text = String::from_utf8_lossy(&part.decode(Default::default())).into_owned();
                if part.content_type.is_text_html() {
                    parts.html.push(text);
                } else {
                    parts.plain.push(text);
                }
            }
            melib::email::attachment_types::ContentType::Multipart { parts: sub, .. } => {
                for p in sub {
                    walk(p, parts, depth + 1);
                }
            }
            _ => {}
        }
    }
    let mut parts = Parts::default();
    walk(root, &mut parts, 0);
    if !parts.plain.is_empty() {
        parts.plain.join("\n\n")
    } else if !parts.html.is_empty() {
        parts
            .html
            .iter()
            .map(|h| {
                let sanitized = ammonia::clean(h);
                html2text::from_read(sanitized.as_bytes(), 100).unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    } else {
        String::new()
    }
}

/// Body text straight from raw bytes (used by search for body queries).
fn body_text_of_bytes(bytes: &[u8]) -> String {
    let root = AttachmentBuilder::new(bytes).build();
    extract_body_text(&root)
}

#[derive(Serialize)]
struct AttachmentMeta {
    filename: String,
    mime: String,
    size: usize,
}

fn collect_attachments(part: &Attachment, out: &mut Vec<AttachmentMeta>, depth: usize) {
    if depth > 10 || out.len() >= 100 {
        return;
    }
    match &part.content_type {
        melib::email::attachment_types::ContentType::Multipart { parts, .. } => {
            for sub in parts {
                collect_attachments(sub, out, depth + 1);
            }
        }
        melib::email::attachment_types::ContentType::MessageRfc822 => {
            // The attached message counts as one attachment; its MIME
            // subtree belongs to the attached mail.
            out.push(AttachmentMeta {
                filename: part
                    .filename()
                    .map(|f| f.to_string())
                    .unwrap_or_else(|| "attached-message.eml".to_string()),
                mime: part.mime_type(),
                size: part.raw().len(),
            });
        }
        melib::email::attachment_types::ContentType::Text { .. } => {
            if part.content_disposition.kind.is_attachment() || part.filename().is_some() {
                out.push(AttachmentMeta {
                    filename: part
                        .filename()
                        .map(|f| f.to_string())
                        .unwrap_or_else(|| "unnamed.txt".to_string()),
                    mime: part.mime_type(),
                    size: part.raw().len(),
                });
            }
        }
        _ => {
            out.push(AttachmentMeta {
                filename: part
                    .filename()
                    .map(|f| f.to_string())
                    .unwrap_or_else(|| "unnamed.bin".to_string()),
                mime: part.mime_type(),
                size: part.raw().len(),
            });
        }
    }
}

fn find_attachment<'a>(root: &'a Attachment, filename: &str) -> Option<&'a Attachment> {
    fn node<'a>(part: &'a Attachment, filename: &str, depth: usize) -> Option<&'a Attachment> {
        if depth > 10 {
            return None;
        }
        match &part.content_type {
            melib::email::attachment_types::ContentType::Multipart { parts, .. } => {
                parts.iter().find_map(|p| node(p, filename, depth + 1))
            }
            _ => {
                let matches = part
                    .filename()
                    .is_some_and(|f| f.eq_ignore_ascii_case(filename))
                    || (matches!(
                        part.content_type,
                        melib::email::attachment_types::ContentType::MessageRfc822
                    ) && filename.eq_ignore_ascii_case("attached-message.eml"));
                matches.then_some(part)
            }
        }
    }
    node(root, filename, 0)
}

fn flag_ops(seen: Option<bool>, flagged: Option<bool>, answered: Option<bool>) -> Vec<FlagOp> {
    let mut ops = Vec::new();
    if let Some(v) = seen {
        ops.push(if v {
            FlagOp::Set(Flag::SEEN)
        } else {
            FlagOp::UnSet(Flag::SEEN)
        });
    }
    if let Some(v) = flagged {
        ops.push(if v {
            FlagOp::Set(Flag::FLAGGED)
        } else {
            FlagOp::UnSet(Flag::FLAGGED)
        });
    }
    if let Some(v) = answered {
        ops.push(if v {
            FlagOp::Set(Flag::REPLIED)
        } else {
            FlagOp::UnSet(Flag::REPLIED)
        });
    }
    ops
}

fn parse_ids(ids: &[String], envs: &[Envelope]) -> Result<Vec<EnvelopeHash>> {
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let hash: u64 = id
            .parse()
            .map_err(|_| bad_request(&format!("`{id}` is not a valid email_id")))?;
        let hash = EnvelopeHash(hash);
        if !envs.iter().any(|e| e.hash == hash) {
            return Err(Error::new(format!(
                "e-mail `{id}` not found in this mailbox; call list_emails for valid ids"
            ))
            .set_kind(ErrorKind::NotFound));
        }
        out.push(hash);
    }
    if out.is_empty() {
        return Err(bad_request("email_ids must not be empty"));
    }
    Ok(out)
}

fn non_empty_batch(hashes: Vec<EnvelopeHash>) -> Option<EnvelopeHashBatch> {
    let first = hashes.first().copied()?;
    Some(EnvelopeHashBatch {
        first,
        rest: hashes[1..].to_vec(),
    })
}

fn new_draft_for(account: &str, settings: &Settings) -> Result<Draft> {
    let conf = settings.accounts.get(account).ok_or_else(|| {
        Error::new(format!("unknown account `{account}`")).set_kind(ErrorKind::ValueError)
    })?;
    let mut draft = Draft::default();
    let from = conf.account().main_identity_address().to_string();
    draft.set_header(HeaderName::FROM, from);
    Ok(draft)
}

fn angle_bracket(mid: &str) -> String {
    let trimmed = mid.trim();
    if trimmed.starts_with('<') && trimmed.ends_with('>') {
        trimmed.to_string()
    } else {
        format!("<{trimmed}>")
    }
}

fn allowlist_matches(patterns: &[String], address: &str) -> bool {
    let address = address.trim().to_lowercase();
    // Allow bare addresses and "Name <addr>" forms.
    let bare = address
        .rsplit('<')
        .next()
        .unwrap_or(&address)
        .trim_end_matches('>')
        .to_string();
    patterns.iter().any(|p| {
        let p = p.to_lowercase();
        glob_match(&p, &bare) || glob_match(&p, &address)
    })
}

/// Tiny glob: `*` matches any run of characters.
fn glob_match(pattern: &str, s: &str) -> bool {
    let (first, rest) = match pattern.split_once('*') {
        Some((first, rest)) => (first, rest),
        None => return pattern == s,
    };
    if !s.starts_with(first) {
        return false;
    }
    let s = &s[first.len()..];
    if rest.is_empty() {
        return true;
    }
    if let Some(last) = rest.rsplit_once('*') {
        // Multiple stars: match the final segment as a suffix and scan.
        let (_, tail) = last;
        if !s.ends_with(tail) {
            return false;
        }
        let mid = rest.strip_suffix(tail).unwrap_or(rest);
        return glob_match(mid, s.strip_suffix(tail).unwrap_or(s));
    }
    s.contains(rest) && s.ends_with(rest)
}

fn query_needs_body(q: &Query) -> bool {
    use Query::*;
    match q {
        Body(_) | AllText(_) => true,
        And(a, b) | Or(a, b) => query_needs_body(a) || query_needs_body(b),
        Not(a) => query_needs_body(a),
        _ => false,
    }
}

fn query_matches(q: &Query, env: &Envelope, body: Option<&str>) -> bool {
    use Query::*;
    match q {
        Body(needle) | AllText(needle) => body.is_some_and(|b| {
            b.to_lowercase().contains(&needle.to_lowercase())
                || env
                    .subject()
                    .to_lowercase()
                    .contains(&needle.to_lowercase())
        }),
        And(a, b) => query_matches(a, env, body) && query_matches(b, env, body),
        Or(a, b) => query_matches(a, env, body) || query_matches(b, env, body),
        Not(a) => !query_matches(a, env, body),
        other => env.is_match(other),
    }
}

fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use melib::{backends::prelude::AccountSettings, utils::datetime};
    use tempfile::TempDir;

    use super::*;
    use crate::conf::mcp::{MCPSettings, MutationPermission};

    fn test_settings(dir: &TempDir, mcp: MCPSettings) -> Settings {
        // Maildir layout: the root *is* the INBOX mailbox; Archive and
        // Drafts are child mailboxes (special usage comes from their
        // names, like meli's own maildir discovery).
        let root = dir.path().join("INBOX");
        let archive = root.join("Archive");
        let drafts = root.join("Drafts");
        for d in [&root, &archive, &drafts] {
            for sub in ["cur", "new", "tmp"] {
                std::fs::create_dir_all(d.join(sub)).unwrap();
            }
        }
        let account_settings = AccountSettings {
            name: "test".to_string(),
            root_mailbox: root.display().to_string(),
            format: "maildir".to_string(),
            identity: "agent@localhost".to_string(),
            extra_identities: vec![],
            read_only: false,
            display_name: Some("Test Agent".to_string()),
            subscribed_mailboxes: vec![],
            mailboxes: indexmap::IndexMap::new(),
            manual_refresh: true,
            contacts: None,
            extra: indexmap::IndexMap::new(),
        };
        let mut account_conf: AccountConf = account_settings.into();
        account_conf.account.name = "test".to_string();
        let mut settings = Settings::default();
        settings.accounts.insert("test".to_string(), account_conf);
        settings.mcp = mcp;
        settings
    }

    fn session(settings: Settings) -> Session {
        Session::from_settings(settings).expect("session")
    }

    fn write_mail(dir: &TempDir, mailbox: &str, seq: usize, secs: u64, content: &str) {
        // `mailbox` is the mailbox path as meli reports it; the INBOX is
        // the maildir root itself.
        let base = if mailbox == "INBOX" {
            dir.path().join("INBOX")
        } else {
            dir.path().join("INBOX").join(mailbox)
        };
        let dir = base.join("new");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{secs}.{seq:03}.mcp-test")), content).unwrap();
    }

    fn simple_mail(seq: usize, from: &str, subject: &str, body: &str) -> String {
        let secs = 1_700_000_000u64 + (seq as u64) * 3600;
        let date = datetime::timestamp_to_string(secs, Some(datetime::formats::RFC822_DATE), true);
        format!(
            "From: {from}\r\nTo: agent@localhost\r\nSubject: {subject}\r\nDate: {date}\r\n\
             Message-ID: <msg-{seq}@mcp.test>\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n\
             {body}\r\n"
        )
    }

    fn list_inbox(sess: &mut Session) -> Value {
        let out = sess
            .tool_list_emails(&json!({"account": "test"}))
            .expect("list_emails works");
        out["emails"].clone()
    }

    #[test]
    fn test_accounts_and_mailboxes() {
        let dir = TempDir::new().unwrap();
        let settings = test_settings(&dir, MCPSettings::default());
        let mut sess = session(settings);
        let accounts = sess.tool_list_accounts(&Value::Null).expect("accounts");
        let accounts = accounts["accounts"].as_array().unwrap().clone();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0]["account_name"], json!("test"));
        assert_eq!(accounts[0]["account_type"], json!("maildir"));
        assert_eq!(accounts[0]["email_address"], json!("agent@localhost"));
        assert_eq!(accounts[0]["can_send"], json!(false));
        // No secret-bearing fields in the serialized response.
        let serialized = serde_json::to_string(&accounts).unwrap().to_lowercase();
        assert!(!serialized.contains("password"));
        assert!(!serialized.contains("secret"));
        assert!(!serialized.contains("token"));

        let mailboxes = sess
            .tool_list_mailboxes(&json!({"account": "test"}))
            .expect("mailboxes");
        let paths: Vec<&str> = mailboxes["mailboxes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["path"].as_str().unwrap())
            .collect();
        assert!(paths.contains(&"INBOX"), "{paths:?}");
        assert!(paths.contains(&"INBOX/Archive"), "{paths:?}");
        assert!(paths.contains(&"INBOX/Drafts"), "{paths:?}");

        let unknown = sess.tool_list_mailboxes(&json!({"account": "nope"}));
        assert!(unknown.is_err());
        assert!(unknown.unwrap_err().to_string().contains("unknown account"));
    }

    #[test]
    fn test_list_emails_pagination_filters_order() {
        let dir = TempDir::new().unwrap();
        for i in 0..25 {
            let from = if i % 2 == 0 {
                "alice@example.org"
            } else {
                "bob@example.org"
            };
            write_mail(
                &dir,
                "INBOX",
                i,
                1_700_000_000 + (i as u64) * 3600,
                &simple_mail(i, from, &format!("report {i:02}"), &format!("body {i}")),
            );
        }
        let settings = test_settings(&dir, MCPSettings::default());
        let mut sess = session(settings);

        let page1 = sess
            .tool_list_emails(&json!({"account": "test", "page_size": 10, "page": 1}))
            .unwrap();
        let page2 = sess
            .tool_list_emails(&json!({"account": "test", "page_size": 10, "page": 2}))
            .unwrap();
        let page3 = sess
            .tool_list_emails(&json!({"account": "test", "page_size": 10, "page": 3}))
            .unwrap();
        assert_eq!(page1["total"], json!(25));
        let mut ids: Vec<String> = Vec::new();
        for page in [&page1, &page2, &page3] {
            assert_eq!(
                page["emails"].as_array().unwrap().len().min(10),
                page["emails"].as_array().unwrap().len()
            );
            for e in page["emails"].as_array().unwrap() {
                ids.push(e["email_id"].as_str().unwrap().to_string());
            }
        }
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(ids.len(), 25, "no duplication across pages");
        assert_eq!(unique.len(), 25, "no overlap across pages");
        // Default order: newest first.
        let subjects: Vec<&str> = page1["emails"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["subject"].as_str().unwrap())
            .collect();
        assert_eq!(subjects[0], "report 24", "{subjects:?}");
        // Filters.
        let filtered = sess
            .tool_list_emails(&json!({
                "account": "test",
                "filters": {"from": "alice", "subject": "report"},
                "page_size": 100,
            }))
            .unwrap();
        assert_eq!(filtered["total"], json!(13));
        // Ascending order.
        let asc = sess
            .tool_list_emails(&json!({
                "account": "test",
                "order": "asc",
                "page_size": 1,
            }))
            .unwrap();
        assert_eq!(asc["emails"][0]["subject"], json!("report 00"));
        // Bounds.
        sess.tool_list_emails(&json!({"account": "test", "page_size": 0}))
            .unwrap_err();
        sess.tool_list_emails(&json!({"account": "test", "page_size": 101}))
            .unwrap_err();
        sess.tool_list_emails(&json!({"account": "test", "page": 0}))
            .unwrap_err();
        // Unknown mailbox.
        sess.tool_list_emails(&json!({"account": "test", "mailbox": "Nope"}))
            .unwrap_err();
    }

    #[test]
    fn test_get_emails_windowing_attachments_and_failures() {
        let dir = TempDir::new().unwrap();
        let long_body = "x".repeat(500);
        write_mail(
            &dir,
            "INBOX",
            1,
            1_700_000_000,
            &format!(
                "From: rich@example.org\r\nTo: agent@localhost\r\nSubject: big\r\n\
                 Date: Wed, 15 Nov 2023 00:00:00 +0000\r\nMessage-ID: <big@mcp.test>\r\n\
                 Content-Type: text/plain; charset=utf-8\r\n\r\n{long_body}\r\n"
            ),
        );
        let attachment_mail = "From: docs@example.org\r\n\
To: agent@localhost\r\nSubject: with attachment\r\n\
Date: Wed, 15 Nov 2023 01:00:00 +0000\r\nMessage-ID: <att@mcp.test>\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"BOUND\"\r\n\r\n\
--BOUND\r\n\
Content-Type: text/plain; charset=utf-8\r\n\r\n\
see attachment\r\n\
--BOUND\r\n\
Content-Type: application/pdf\r\n\
Content-Disposition: attachment; filename=\"invoice.pdf\"\r\n\
Content-Transfer-Encoding: base64\r\n\r\n\
aGVsbG8gd29ybGQ=\r\n\
--BOUND--\r\n";
        write_mail(&dir, "INBOX", 2, 1_700_003_600, attachment_mail);
        let settings = test_settings(&dir, MCPSettings::default());
        let mut sess = session(settings);
        let emails = list_inbox(&mut sess);
        let id_big = emails[1]["email_id"].as_str().unwrap().to_string();
        let id_att = emails[0]["email_id"].as_str().unwrap().to_string();

        let first = sess
            .tool_get_emails(&json!({
                "account": "test",
                "email_ids": [id_big],
                "max_body_length": 200,
            }))
            .unwrap();
        let e = &first["emails"][0];
        // 500 x's plus the trailing CRLF of the raw body.
        assert_eq!(e["body_length"], json!(502));
        assert_eq!(e["truncated"], json!(true));
        assert!(e["body_text"].as_str().unwrap().ends_with("...[TRUNCATED]"));
        let second = sess
            .tool_get_emails(&json!({
                "account": "test",
                "email_ids": [id_big],
                "body_offset": 200,
                "max_body_length": 200,
            }))
            .unwrap();
        let mid = &second["emails"][0];
        assert_eq!(mid["body_offset"], json!(200));
        // Windowed reads reassemble the whole body (500 x's).
        let whole = sess
            .tool_get_emails(&json!({
                "account": "test",
                "email_ids": [id_big],
                "max_body_length": 100000,
            }))
            .unwrap();
        let body = whole["emails"][0]["body_text"].as_str().unwrap();
        assert_eq!(body.chars().filter(|c| *c == 'x').count(), 500);
        assert!(!whole["emails"][0]["truncated"].as_bool().unwrap());

        let att = sess
            .tool_get_emails(&json!({"account": "test", "email_ids": [id_att]}))
            .unwrap();
        let attachments = att["emails"][0]["attachments"].as_array().unwrap().clone();
        assert_eq!(attachments.len(), 1, "{attachments:?}");
        assert_eq!(attachments[0]["filename"], json!("invoice.pdf"));
        assert_eq!(attachments[0]["mime"], json!("application/pdf"));

        // get_attachment returns the exact bytes.
        let got = sess
            .tool_get_attachment(&json!({
                "account": "test",
                "email_id": id_att,
                "filename": "invoice.pdf",
            }))
            .unwrap();
        assert_eq!(got["size"], json!(11));
        assert_eq!(got["blob_base64"], json!("aGVsbG8gd29ybGQ="));
        // Path traversal is refused.
        sess.tool_get_attachment(&json!({
            "account": "test",
            "email_id": id_att,
            "filename": "../../etc/passwd",
        }))
        .unwrap_err();

        // failed list: one good id + one bogus id.
        let mixed = sess
            .tool_get_emails(&json!({"account": "test", "email_ids": [id_big, "999999"]}))
            .unwrap();
        assert_eq!(mixed["emails"].as_array().unwrap().len(), 1);
        assert_eq!(mixed["failed"].as_array().unwrap().len(), 1);
        assert_eq!(mixed["failed"][0]["email_id"], json!("999999"));

        // mark_as_read is a mutation: refused under the read-only default.
        let marked = sess
            .tool_get_emails(&json!({
                "account": "test",
                "email_ids": [id_big],
                "mark_as_read": true,
            }))
            .unwrap_err();
        assert!(marked.to_string().contains("allowed_mutations"));
    }

    #[test]
    fn test_get_thread_reconstruction() {
        let dir = TempDir::new().unwrap();
        write_mail(
            &dir,
            "INBOX",
            1,
            1_700_000_000,
            "From: root@example.org\r\nTo: agent@localhost\r\nSubject: root topic\r\n\
             Date: Wed, 15 Nov 2023 00:00:00 +0000\r\nMessage-ID: <root@mcp.test>\r\n\
             Content-Type: text/plain\r\n\r\nstart\r\n",
        );
        write_mail(
            &dir,
            "INBOX",
            2,
            1_700_000_010,
            "From: replier@example.org\r\nTo: agent@localhost\r\nSubject: Re: root topic\r\n\
             Date: Wed, 15 Nov 2023 00:00:10 +0000\r\nMessage-ID: <reply@mcp.test>\r\n\
             In-Reply-To: <root@mcp.test>\r\nReferences: <root@mcp.test>\r\n\
             Content-Type: text/plain\r\n\r\nreply\r\n",
        );
        write_mail(
            &dir,
            "Archive",
            3,
            1_700_000_020,
            "From: fork@example.org\r\nTo: agent@localhost\r\nSubject: Re: root topic (fork)\r\n\
             Date: Wed, 15 Nov 2023 00:00:20 +0000\r\nMessage-ID: <fork@mcp.test>\r\n\
             In-Reply-To: <root@mcp.test>\r\nReferences: <root@mcp.test>\r\n\
             Content-Type: text/plain\r\n\r\nfork\r\n",
        );
        // Unrelated message: must not join.
        write_mail(
            &dir,
            "INBOX",
            4,
            1_700_000_030,
            "From: other@example.org\r\nTo: agent@localhost\r\nSubject: different\r\n\
             Date: Wed, 15 Nov 2023 00:00:30 +0000\r\nMessage-ID: <other@mcp.test>\r\n\
             Content-Type: text/plain\r\n\r\nunrelated\r\n",
        );
        let settings = test_settings(&dir, MCPSettings::default());
        let mut sess = session(settings);
        let archive = sess
            .tool_list_emails(&json!({"account": "test", "mailbox": "INBOX/Archive"}))
            .unwrap();
        let fork_id = archive["emails"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["subject"] == json!("Re: root topic (fork)"))
            .unwrap()["email_id"]
            .as_str()
            .unwrap()
            .to_string();
        let thread = sess
            .tool_get_thread(&json!({"account": "test", "email_id": fork_id}))
            .unwrap();
        let subjects: Vec<&str> = thread["emails"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["subject"].as_str().unwrap())
            .collect();
        assert_eq!(
            subjects,
            ["root topic", "Re: root topic", "Re: root topic (fork)"],
            "{subjects:?}"
        );
        let in_reply_tos: Vec<Option<&str>> = thread["emails"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["in_reply_to"].as_str())
            .collect();
        assert_eq!(in_reply_tos[1], Some("root@mcp.test"));
        assert_eq!(in_reply_tos[2], Some("root@mcp.test"));
    }

    #[test]
    fn test_search_emails_body_and_headers() {
        let dir = TempDir::new().unwrap();
        write_mail(
            &dir,
            "INBOX",
            1,
            1_700_000_000,
            &simple_mail(
                1,
                "alice@example.org",
                "quarterly report",
                "the numbers are up",
            ),
        );
        write_mail(
            &dir,
            "INBOX",
            2,
            1_700_003_600,
            &simple_mail(2, "bob@example.org", "lunch", "do you like sushi"),
        );
        let settings = test_settings(&dir, MCPSettings::default());
        let mut sess = session(settings);

        let by_from = sess
            .tool_search_emails(&json!({"account": "test", "query": "from:alice"}))
            .unwrap();
        assert_eq!(by_from["total"], json!(1));
        assert_eq!(by_from["emails"][0]["subject"], json!("quarterly report"));

        let combined = sess
            .tool_search_emails(
                &json!({"account": "test", "query": "from:alice and subject:report"}),
            )
            .unwrap();
        assert_eq!(combined["total"], json!(1));

        let none = sess
            .tool_search_emails(
                &json!({"account": "test", "query": "from:alice and subject:lunch"}),
            )
            .unwrap();
        assert_eq!(none["total"], json!(0));

        // Bare literal matches the body (meli query semantics).
        let body = sess
            .tool_search_emails(&json!({"account": "test", "query": "sushi"}))
            .unwrap();
        assert_eq!(body["total"], json!(1));
        assert_eq!(body["emails"][0]["subject"], json!("lunch"));

        sess.tool_search_emails(&json!({"account": "test", "query": "bogus_syntax ((("}))
            .unwrap_err();
    }

    #[test]
    fn test_extract_and_scan_urls_via_session() {
        let dir = TempDir::new().unwrap();
        write_mail(
            &dir,
            "INBOX",
            1,
            1_700_000_000,
            "From: phish@example.org\r\nTo: agent@localhost\r\nSubject: verify\r\n\
             Date: Wed, 15 Nov 2023 00:00:00 +0000\r\nMessage-ID: <phish@mcp.test>\r\n\
             Content-Type: text/html; charset=utf-8\r\n\r\n\
             <a href=\"http://paypal.com.secure-login.example.ru/\">https://paypal.com/login</a> \
             and hxxp://evil[.]com/x\r\n",
        );
        let settings = test_settings(&dir, MCPSettings::default());
        let mut sess = session(settings);
        let emails = list_inbox(&mut sess);
        let id = emails[0]["email_id"].as_str().unwrap().to_string();
        let extracted = sess
            .tool_extract_urls(&json!({"account": "test", "email_id": id}))
            .unwrap();
        let urls = extracted["urls"].as_array().unwrap().clone();
        assert!(
            urls.iter()
                .any(|u| u["url"] == json!("http://paypal.com.secure-login.example.ru/")),
            "{urls:?}"
        );
        assert!(
            urls.iter()
                .any(|u| u["url"] == json!("http://evil.com/x") && u["obfuscated"] == json!(true)),
            "{urls:?}"
        );

        let scanned = sess
            .tool_scan_email_urls(&json!({"account": "test", "email_id": id}))
            .unwrap();
        let results = scanned["urls"].as_array().unwrap().clone();
        let phish = results
            .iter()
            .find(|u| u["url"].as_str().unwrap().contains("paypal"))
            .unwrap();
        assert_eq!(phish["risk"], json!("high"), "{phish:?}");
        let signals: Vec<&str> = phish["signals"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect();
        assert!(signals.contains(&"brand_in_subdomain"), "{signals:?}");
        assert!(signals.contains(&"href_text_mismatch"), "{signals:?}");
        // The de-obfuscated link carries its own signal.
        let deobfuscated = results
            .iter()
            .find(|u| u["url"] == json!("http://evil.com/x"))
            .unwrap();
        let deobfuscated_signals: Vec<&str> = deobfuscated["signals"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect();
        assert!(
            deobfuscated_signals.contains(&"obfuscated_spelling"),
            "{deobfuscated_signals:?}"
        );
    }

    #[test]
    fn test_set_flags_permission_and_effect() {
        let dir = TempDir::new().unwrap();
        write_mail(
            &dir,
            "INBOX",
            1,
            1_700_000_000,
            &simple_mail(1, "a@example.org", "s", "b"),
        );
        // Read-only by default.
        let settings = test_settings(&dir, MCPSettings::default());
        let mut sess = session(settings);
        let emails = list_inbox(&mut sess);
        let id = emails[0]["email_id"].as_str().unwrap().to_string();
        assert_eq!(emails[0]["flags"].as_array().unwrap().len(), 0);
        let denied = sess
            .tool_set_flags(&json!({
                "account": "test",
                "mailbox": "INBOX",
                "email_ids": [id],
                "flags": {"seen": true},
            }))
            .unwrap_err();
        assert!(denied.to_string().contains("organize"), "{}", denied);
        drop(sess);

        // With `organize` the flag lands and is visible.
        let mcp = MCPSettings {
            allowed_mutations: vec![MutationPermission::Organize],
            ..MCPSettings::default()
        };
        let settings = test_settings(&dir, mcp);
        let mut sess = session(settings);
        sess.tool_set_flags(&json!({
            "account": "test",
            "mailbox": "INBOX",
            "email_ids": [id],
            "flags": {"seen": true, "flagged": true},
        }))
        .unwrap();
        let emails = list_inbox(&mut sess);
        let flags = emails[0]["flags"].as_array().unwrap().clone();
        assert!(flags.contains(&json!("seen")), "{flags:?}");
        assert!(flags.contains(&json!("flagged")), "{flags:?}");
        // Idempotent replay: a maildir flag change renames the file, which
        // changes its opaque id, so take the fresh id from the new listing.
        let fresh_id = emails[0]["email_id"].as_str().unwrap().to_string();
        sess.tool_set_flags(&json!({
            "account": "test",
            "mailbox": "INBOX",
            "email_ids": [fresh_id],
            "flags": {"seen": true},
        }))
        .unwrap();
        let emails = list_inbox(&mut sess);
        let flags = emails[0]["flags"].as_array().unwrap().clone();
        assert!(flags.contains(&json!("seen")), "{flags:?}");
        // Id from another mailbox is rejected.
        sess.tool_set_flags(&json!({
            "account": "test",
            "mailbox": "INBOX/Archive",
            "email_ids": [id],
            "flags": {"seen": true},
        }))
        .unwrap_err();
    }

    #[test]
    fn test_move_and_archive() {
        let dir = TempDir::new().unwrap();
        write_mail(
            &dir,
            "INBOX",
            1,
            1_700_000_000,
            &simple_mail(1, "a@example.org", "old", "b"),
        );
        write_mail(
            &dir,
            "INBOX",
            2,
            1_700_003_600,
            &simple_mail(2, "a@example.org", "keep", "b"),
        );
        let mcp = MCPSettings {
            allowed_mutations: vec![MutationPermission::Organize],
            ..MCPSettings::default()
        };
        let settings = test_settings(&dir, mcp);
        let mut sess = session(settings);
        let emails = list_inbox(&mut sess);
        let old_id = emails[1]["email_id"].as_str().unwrap().to_string();
        sess.tool_move_emails(&json!({
            "account": "test",
            "mailbox": "INBOX",
            "email_ids": [old_id],
            "to_mailbox": "INBOX/Archive",
        }))
        .unwrap();
        let inbox = sess.tool_list_emails(&json!({"account": "test"})).unwrap();
        assert_eq!(inbox["total"], json!(1));
        assert_eq!(inbox["emails"][0]["subject"], json!("keep"));
        let archive = sess
            .tool_list_emails(&json!({"account": "test", "mailbox": "INBOX/Archive"}))
            .unwrap();
        assert_eq!(archive["total"], json!(1));
        assert_eq!(archive["emails"][0]["subject"], json!("old"));

        // Archive tool: moves into the Archive special-use mailbox.
        let keep_id = inbox["emails"][0]["email_id"].as_str().unwrap().to_string();
        sess.tool_archive_emails(&json!({
            "account": "test",
            "mailbox": "INBOX",
            "email_ids": [keep_id],
        }))
        .unwrap();
        let archive = sess
            .tool_list_emails(&json!({"account": "test", "mailbox": "INBOX/Archive"}))
            .unwrap();
        assert_eq!(archive["total"], json!(2));
        // The maildir file really moved (counts stay consistent).
        let new_count = std::fs::read_dir(dir.path().join("INBOX").join("Archive").join("new"))
            .unwrap()
            .count();
        let cur_count = std::fs::read_dir(dir.path().join("INBOX").join("Archive").join("cur"))
            .unwrap()
            .count();
        assert_eq!(new_count + cur_count, 2);
    }

    #[test]
    fn test_save_draft() {
        let dir = TempDir::new().unwrap();
        let mcp = MCPSettings {
            allowed_mutations: vec![MutationPermission::Drafts],
            ..MCPSettings::default()
        };
        let settings = test_settings(&dir, mcp);
        let mut sess = session(settings);
        // Denied without the drafts permission.
        {
            let read_only = session(test_settings(&dir, MCPSettings::default()));
            let mut read_only = read_only;
            read_only
                .tool_save_draft(&json!({
                    "account": "test",
                    "to": ["friend@example.org"],
                    "subject": "hi",
                    "body": "hello",
                }))
                .unwrap_err();
        }
        let saved = sess
            .tool_save_draft(&json!({
                "account": "test",
                "to": ["friend@example.org"],
                "cc": ["other@example.org"],
                "subject": "hello from mcp",
                "body": "line one\nline two",
            }))
            .unwrap();
        assert_eq!(saved["saved"], json!(true));
        assert_eq!(saved["mailbox"], json!("INBOX/Drafts"));
        let drafts = sess
            .tool_list_emails(&json!({"account": "test", "mailbox": "INBOX/Drafts"}))
            .unwrap();
        assert_eq!(drafts["total"], json!(1));
        let id = drafts["emails"][0]["email_id"]
            .as_str()
            .unwrap()
            .to_string();
        let full = sess
            .tool_get_emails(
                &json!({"account": "test", "mailbox": "INBOX/Drafts", "email_ids": [id]}),
            )
            .unwrap();
        let e = &full["emails"][0];
        assert_eq!(e["subject"], json!("hello from mcp"));
        assert_eq!(e["from"], json!(["Test Agent <agent@localhost>"]));
        assert!(e["body_text"].as_str().unwrap().contains("line one"));
        // Header injection is refused by the draft finaliser (CWE-93).
        sess.tool_save_draft(&json!({
            "account": "test",
            "to": ["friend@example.org"],
            "subject": "bad\r\nBcc: evil@example.org",
            "body": "x",
        }))
        .unwrap_err();
    }

    #[test]
    fn test_send_email_allowlist_rate_limit_and_transport() {
        let dir = TempDir::new().unwrap();
        let outbox = dir.path().join("sent");
        let mcp = MCPSettings {
            allowed_mutations: vec![MutationPermission::Send],
            send_recipient_allowlist: vec!["*@example.org".to_string()],
            send_rate_limit_per_minute: 1,
            ..MCPSettings::default()
        };
        let mut settings = test_settings(&dir, mcp.clone());
        settings.accounts.get_mut("test").unwrap().conf.send_mail =
            SendMail::ShellCommand(format!("cat > {}", outbox.display()));
        let mut sess = session(settings);

        // Recipient outside the allowlist: denied with guidance.
        let denied = sess
            .tool_send_email(&json!({
                "account": "test",
                "to": ["someone@evil.example"],
                "subject": "no",
                "body": "x",
            }))
            .unwrap_err();
        assert!(denied.to_string().contains("allowlist"), "{}", denied);

        // Allowed recipient: the transport receives the message.
        let sent = sess
            .tool_send_email(&json!({
                "account": "test",
                "to": ["friend@example.org"],
                "subject": "hello",
                "body": "from mcp",
                "in_reply_to": "root@mcp.test",
            }))
            .unwrap();
        assert_eq!(sent["accepted"], json!(true));
        assert!(sent["message_id"].as_str().unwrap().starts_with('<'));
        let mut written = String::new();
        std::fs::File::open(&outbox)
            .unwrap()
            .read_to_string(&mut written)
            .unwrap();
        assert!(
            written.contains("From: Test Agent <agent@localhost>"),
            "{written}"
        );
        assert!(written.contains("To: friend@example.org"), "{written}");
        assert!(written.contains("Subject: hello"), "{written}");
        assert!(
            written.contains("In-Reply-To: <root@mcp.test>"),
            "{written}"
        );
        assert!(written.contains("Message-ID: "), "{written}");

        // Rate limit: one send per minute under this configuration.
        let limited = sess
            .tool_send_email(&json!({
                "account": "test",
                "to": ["friend@example.org"],
                "subject": "again",
                "body": "x",
            }))
            .unwrap_err();
        assert!(limited.to_string().contains("rate limit"), "{}", limited);

        // Empty allowlist denies everyone.
        let mut none = mcp.clone();
        none.send_recipient_allowlist = vec![];
        let settings = test_settings(&dir, none);
        let mut sess2 = session(settings);
        sess2
            .tool_send_email(&json!({
                "account": "test",
                "to": ["friend@example.org"],
                "subject": "hi",
                "body": "x",
            }))
            .unwrap_err();
        // A failing transport reports an error, not success.
        let mut failing = test_settings(&dir, {
            let mut m = mcp;
            m.send_rate_limit_per_minute = 10;
            m
        });
        failing.accounts.get_mut("test").unwrap().conf.send_mail =
            SendMail::ShellCommand("false".to_string());
        let mut sess3 = session(failing);
        let failed = sess3
            .tool_send_email(&json!({
                "account": "test",
                "to": ["friend@example.org"],
                "subject": "hi",
                "body": "x",
            }))
            .unwrap_err();
        assert!(failed.to_string().contains("not accept"), "{}", failed);
    }

    #[test]
    fn test_extract_body_text_prefers_plain() {
        let dir = TempDir::new().unwrap();
        write_mail(
            &dir,
            "INBOX",
            1,
            1_700_000_000,
            "From: x@example.org\r\nTo: agent@localhost\r\nSubject: alt\r\n\
             Date: Wed, 15 Nov 2023 00:00:00 +0000\r\nMessage-ID: <alt@mcp.test>\r\n\
             MIME-Version: 1.0\r\n\
             Content-Type: multipart/alternative; boundary=\"AB\"\r\n\r\n\
             --AB\r\nContent-Type: text/plain\r\n\r\nPLAIN WIN\r\n\
             --AB\r\nContent-Type: text/html\r\n\r\n<html><body>HTML loses</body></html>\r\n\
             --AB--\r\n",
        );
        let settings = test_settings(&dir, MCPSettings::default());
        let mut sess = session(settings);
        let emails = list_inbox(&mut sess);
        let id = emails[0]["email_id"].as_str().unwrap().to_string();
        let full = sess
            .tool_get_emails(&json!({"account": "test", "email_ids": [id]}))
            .unwrap();
        assert!(
            full["emails"][0]["body_text"]
                .as_str()
                .unwrap()
                .contains("PLAIN WIN")
        );
    }

    #[test]
    fn test_missing_account_errors_list_available() {
        let dir = TempDir::new().unwrap();
        let settings = test_settings(&dir, MCPSettings::default());
        let mut sess = session(settings);
        let err = sess
            .tool_list_emails(&json!({"account": "ghost"}))
            .unwrap_err();
        assert!(err.to_string().contains("ghost"), "{}", err);
        assert!(err.to_string().contains("test"), "{}", err);
    }
}
