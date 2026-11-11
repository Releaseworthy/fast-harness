//! The account record and its secret.

use crate::accounts::crypto::{hex, sha256};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

/// Where an account comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountKind {
    /// OAuth sign-in done by nh; nh refreshes its tokens.
    Own,
    /// Imported from DSH's claude-switch, read-only (PLAN.md D8).
    Linked,
    /// An Anthropic API key.
    ApiKey,
}

/// One account as listed in `accounts.json` (without its secret).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    /// `u-<uuid>` for OAuth accounts, `k-<first 8 hex of sha256(key)>` for API keys.
    pub id: String,
    pub kind: AccountKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// Unix milliseconds.
    pub added_at: u64,
}

impl Account {
    /// The id of an OAuth account: `u-` and the uuid, normalised like
    /// claude-switch's `recordIdFor` (lowercase, only `[a-z0-9-]` kept).
    pub fn oauth_id(uuid: &str) -> String {
        let clean: String = uuid
            .to_lowercase()
            .chars()
            .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
            .collect();
        format!("u-{clean}")
    }

    /// The id of an API key account: `k-` and the first 8 hex digits of sha256(key).
    pub fn api_key_id(key: &str) -> String {
        let digest = hex(&sha256(key.as_bytes()));
        format!("k-{}", &digest[..8])
    }
}

/// The credential of an account. `Debug` never shows the values.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Secret {
    /// OAuth tokens of an own account.
    Own {
        refresh: String,
        access: String,
        /// Unix milliseconds when `access` expires.
        #[serde(rename = "expiresAt")]
        expires_at: u64,
    },
    /// An API key.
    ApiKey { key: String },
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Secret::Own { expires_at, .. } => f
                .debug_struct("Own")
                .field("refresh", &"<redacted>")
                .field("access", &"<redacted>")
                .field("expires_at", expires_at)
                .finish(),
            Secret::ApiKey { .. } => f
                .debug_struct("ApiKey")
                .field("key", &"<redacted>")
                .finish(),
        }
    }
}

/// The current time in Unix milliseconds (0 if the clock is before 1970).
pub fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}
