/*
 * meli - MCP (Model Context Protocol) server settings.
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

//! Settings of the `[mcp]` configuration section.
//!
//! These control the `meli mcp` server: which mutation classes local AI
//! agents may perform, the recipient allowlist and rate limit for outgoing
//! mail, and the URL scoring thresholds of the phishing detection engine.
//!
//! The default is **read-only**: `allowed_mutations` empty means every
//! mutating tool call is refused until the user opts in, in their own
//! configuration file.

use indexmap::IndexMap;
use melib::error::{Error, ErrorKind, Result};

/// A class of mutating operations the MCP server may perform.
///
/// Read-only tools are always available; these permissions only gate
/// mutations.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MutationPermission {
    /// Save messages into the account's `Drafts` mailbox (`save_draft`).
    /// Nothing leaves the machine.
    Drafts,
    /// Organize existing mail: set flags/tags, move or archive messages.
    Organize,
    /// Send mail through the account's `send_mail` transport
    /// (`send_email`).
    Send,
}

impl MutationPermission {
    pub const ALL: [Self; 3] = [Self::Drafts, Self::Organize, Self::Send];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Drafts => "drafts",
            Self::Organize => "organize",
            Self::Send => "send",
        }
    }
}

impl std::fmt::Display for MutationPermission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Tunables of the local (offline) URL risk scoring engine.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct UrlScoringSettings {
    /// Scores in `[0, low_medium_threshold)` are reported as `low`.
    pub low_medium_threshold: u8,
    /// Scores in `[low_medium_threshold, medium_high_threshold)` are
    /// reported as `medium`; anything above is `high`.
    pub medium_high_threshold: u8,
    /// Per-signal weight overrides, keyed by signal name. Unknown signal
    /// names are a configuration error; see the signal list in
    /// `meli/docs/mcp.md`.
    pub signal_weights: IndexMap<String, u8>,
}

impl Default for UrlScoringSettings {
    fn default() -> Self {
        Self {
            low_medium_threshold: 30,
            medium_high_threshold: 60,
            signal_weights: IndexMap::new(),
        }
    }
}

/// Settings of the `[mcp]` section.
///
/// # Examples
///
/// ```toml
/// [mcp]
/// allowed_mutations = ["organize"]
/// send_recipient_allowlist = ["*@example.org", "user@friend.tld"]
/// send_rate_limit_per_minute = 5
///
/// [mcp.url_scoring]
/// low_medium_threshold = 25
/// medium_high_threshold = 55
/// ```
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct MCPSettings {
    /// Mutation classes the MCP server may perform. Empty (the default)
    /// means read-only.
    pub allowed_mutations: Vec<MutationPermission>,
    /// Glob patterns (`*@example.org`) restricting the recipients
    /// `send_email` may deliver to. An empty list rejects every
    /// recipient: sending must be explicitly scoped by the user.
    pub send_recipient_allowlist: Vec<String>,
    /// Maximum number of `send_email` calls per account per minute.
    pub send_rate_limit_per_minute: usize,
    /// URL risk scoring thresholds and weight overrides.
    pub url_scoring: UrlScoringSettings,
}

impl Default for MCPSettings {
    fn default() -> Self {
        Self {
            allowed_mutations: Vec::new(),
            send_recipient_allowlist: Vec::new(),
            send_rate_limit_per_minute: 10,
            url_scoring: UrlScoringSettings::default(),
        }
    }
}

impl MCPSettings {
    /// Whether `perm` is granted.
    pub fn allows(&self, perm: MutationPermission) -> bool {
        self.allowed_mutations.contains(&perm)
    }

    /// Validate values that can only be checked once the whole section is
    /// deserialized.
    pub fn validate(&self) -> Result<()> {
        if self.low_medium_threshold() >= self.medium_high_threshold() {
            return Err(Error::new(
                "mcp.url_scoring: `low_medium_threshold` must be strictly smaller than \
                 `medium_high_threshold`",
            )
            .set_kind(ErrorKind::Configuration));
        }
        if self.send_rate_limit_per_minute == 0 {
            return Err(
                Error::new("mcp.send_rate_limit_per_minute must be at least 1")
                    .set_kind(ErrorKind::Configuration),
            );
        }
        for pattern in &self.send_recipient_allowlist {
            let valid = pattern.chars().all(|c| {
                c.is_ascii_alphanumeric() || matches!(c, '@' | '.' | '-' | '_' | '+' | '*')
            });
            if pattern.is_empty() || !valid {
                return Err(Error::new(format!(
                    "mcp.send_recipient_allowlist: invalid pattern `{pattern}` (expected an \
                     e-mail address with optional `*` wildcards)"
                ))
                .set_kind(ErrorKind::Configuration));
            }
        }
        let valid = crate::mcp::scoring::signal_names();
        for name in self.url_scoring.signal_weights.keys() {
            if !valid.contains(&name.as_str()) {
                return Err(Error::new(format!(
                    "mcp.url_scoring.signal_weights: unknown signal `{name}`; valid signals: \
                     {}",
                    valid.join(", ")
                ))
                .set_kind(ErrorKind::Configuration));
            }
        }
        Ok(())
    }

    /// Resolved scoring thresholds.
    pub fn low_medium_threshold(&self) -> u8 {
        self.url_scoring.low_medium_threshold
    }

    /// Resolved scoring thresholds.
    pub fn medium_high_threshold(&self) -> u8 {
        self.url_scoring.medium_high_threshold
    }

    /// Resolve the weight of signal `name`, preferring the configured
    /// override over the default.
    pub fn signal_weight(&self, name: &str, default: u8) -> u8 {
        self.url_scoring
            .signal_weights
            .get(name)
            .copied()
            .unwrap_or(default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_from_toml(s: &str) -> MCPSettings {
        toml::from_str(s).unwrap()
    }

    #[test]
    fn test_mcp_settings_default_is_read_only() {
        let s = MCPSettings::default();
        assert!(s.allowed_mutations.is_empty());
        assert!(!s.allows(MutationPermission::Organize));
        assert!(!s.allows(MutationPermission::Send));
        assert!(!s.allows(MutationPermission::Drafts));
        assert_eq!(s.send_rate_limit_per_minute, 10);
        assert_eq!(s.low_medium_threshold(), 30);
        assert_eq!(s.medium_high_threshold(), 60);
        s.validate().unwrap();
    }

    #[test]
    fn test_mcp_settings_parse_and_grant() {
        let s = settings_from_toml(
            r#"
allowed_mutations = ["organize", "drafts"]
send_recipient_allowlist = ["*@example.org"]
send_rate_limit_per_minute = 3

[url_scoring]
low_medium_threshold = 25
[url_scoring.signal_weights]
"ip_literal_host" = 50
"#,
        );
        assert!(s.allows(MutationPermission::Organize));
        assert!(s.allows(MutationPermission::Drafts));
        assert!(!s.allows(MutationPermission::Send));
        assert_eq!(s.signal_weight("ip_literal_host", 45), 50);
        assert_eq!(s.signal_weight("shortener_domain", 25), 25);
        s.validate().unwrap();
    }

    #[test]
    fn test_mcp_settings_reject_bad_thresholds() {
        let mut s = MCPSettings::default();
        s.url_scoring.low_medium_threshold = 60;
        s.url_scoring.medium_high_threshold = 60;
        assert!(s.validate().is_err());
        s.url_scoring.low_medium_threshold = 61;
        assert!(s.validate().is_err());
    }

    #[test]
    fn test_mcp_settings_reject_unknown_signal() {
        let s = settings_from_toml(
            r#"
[url_scoring.signal_weights]
"not_a_signal" = 5
"#,
        );
        let err = s.validate().unwrap_err();
        assert!(err.to_string().contains("not_a_signal"));
    }

    #[test]
    fn test_mcp_settings_reject_bad_allowlist_and_rate() {
        let s = settings_from_toml(r#"send_recipient_allowlist = ["bad pattern!"]"#);
        assert!(s.validate().is_err());
        let s = settings_from_toml(r#"send_rate_limit_per_minute = 0"#);
        assert!(s.validate().is_err());
    }
}
