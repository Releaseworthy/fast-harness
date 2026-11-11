//! Read-only access to claude-switch accounts in DSH's credential file.
//!
//! claude-switch keeps each account as the record `claude-switch/u-<uuid>` in
//! `~/.dsh/.credentials.yaml`. nh only reads it: it never writes the file and
//! never refreshes a linked token, because a Claude refresh token works once and
//! refreshing it would break DSH's copy (PLAN.md D8). Only the fields nh uses are
//! kept; refresh tokens and every other record are skipped.

mod yaml;

use crate::accounts::model::Account;
use crate::platform;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const RECORD_PREFIX: &str = "claude-switch/";

/// The access token DSH holds for a linked account. `Debug` hides it.
#[derive(Clone, PartialEq, Eq)]
pub struct LinkedToken {
    pub access: String,
    /// Unix milliseconds.
    pub expires_at: u64,
}

impl LinkedToken {
    /// True once `now_unix_ms` has reached the expiry time.
    pub fn is_expired(&self, now_unix_ms: u64) -> bool {
        now_unix_ms >= self.expires_at
    }
}

impl fmt::Debug for LinkedToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LinkedToken")
            .field("access", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// One claude-switch account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedAccount {
    /// The record id, `u-<uuid>`: the same as [`Account::oauth_id`] of the uuid.
    pub id: String,
    pub uuid: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub plan: Option<String>,
    pub token: LinkedToken,
}

#[derive(Debug)]
pub enum DshError {
    Io(io::Error),
    /// The file's `version` is not 1.
    UnsupportedVersion,
}

impl fmt::Display for DshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DshError::Io(err) => write!(f, "DSH credential file: {err}"),
            DshError::UnsupportedVersion => f.write_str("DSH credential file version is not 1"),
        }
    }
}

impl std::error::Error for DshError {}

/// DSH's home: `DSH_HOME`, else `~/.dsh`.
pub fn dsh_home() -> Option<PathBuf> {
    match std::env::var_os("DSH_HOME").filter(|value| !value.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir)),
        None => platform::home_dir().map(|home| home.join(".dsh")),
    }
}

/// `dsh_home()/.credentials.yaml`.
pub fn credentials_path() -> Option<PathBuf> {
    dsh_home().map(|home| home.join(".credentials.yaml"))
}

/// Every claude-switch OAuth account in the file, in file order. A missing file
/// has none. Records without an access token, a refresh token or an expiry are
/// skipped, as claude-switch's `grantOf` does.
pub fn read_linked(path: &Path) -> Result<Vec<LinkedAccount>, DshError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(DshError::Io(err)),
    };
    parse_linked(text.trim_start_matches('\u{feff}'))
}

/// The current token of the account with this uuid, read fresh from the file.
pub fn read_linked_token(path: &Path, uuid: &str) -> Result<Option<LinkedToken>, DshError> {
    let id = Account::oauth_id(uuid);
    Ok(read_linked(path)?
        .into_iter()
        .find(|account| account.id == id)
        .map(|account| account.token))
}

/// What one record yields while the file is walked.
#[derive(Default)]
struct Partial {
    record: String,
    oauth: bool,
    has_refresh: bool,
    access: Option<String>,
    expires: Option<u64>,
    uuid: Option<String>,
    email: Option<String>,
    name: Option<String>,
    plan: Option<String>,
}

/// The parser behind [`read_linked`], on the file text.
pub fn parse_linked(text: &str) -> Result<Vec<LinkedAccount>, DshError> {
    let mut records: Vec<Partial> = Vec::new();
    let mut version_ok = true;
    yaml::walk(text, &mut |path, raw| match path {
        ["version"] => version_ok = yaml::scalar(raw).as_deref() == Some("1"),
        ["records", record, "payload", field @ ..] => {
            let Some(id) = record.strip_prefix(RECORD_PREFIX) else {
                return;
            };
            if !id.starts_with("u-") {
                return;
            }
            let index = match records.iter().position(|p| p.record == *id) {
                Some(index) => index,
                None => {
                    records.push(Partial {
                        record: id.to_string(),
                        ..Partial::default()
                    });
                    records.len() - 1
                }
            };
            apply(&mut records[index], field, raw);
        }
        _ => {}
    });
    if !version_ok {
        return Err(DshError::UnsupportedVersion);
    }
    Ok(records.into_iter().filter_map(finish).collect())
}

/// Stores one payload field of a claude-switch record; others are ignored.
fn apply(partial: &mut Partial, field: &[&str], raw: &str) {
    match field {
        ["type"] => partial.oauth = yaml::scalar(raw).as_deref() == Some("oauth"),
        ["refresh"] => partial.has_refresh = text(raw).is_some(),
        ["access"] => partial.access = text(raw),
        ["expires"] => partial.expires = text(raw).and_then(|v| number(&v)),
        ["account", "uuid"] => partial.uuid = text(raw),
        ["account", "email"] => partial.email = text(raw),
        ["account", "name"] => partial.name = text(raw),
        ["account", "plan"] => partial.plan = text(raw),
        _ => {}
    }
}

/// A non-empty string value.
fn text(raw: &str) -> Option<String> {
    yaml::scalar(raw).filter(|value| !value.is_empty())
}

/// A non-negative whole number of milliseconds (integer or float notation).
fn number(value: &str) -> Option<u64> {
    if let Ok(n) = value.parse::<u64>() {
        return Some(n);
    }
    let float = value.parse::<f64>().ok()?;
    // Truncation is fine: the value is a millisecond timestamp.
    (float.is_finite() && float >= 0.0 && float < u64::MAX as f64).then_some(float as u64)
}

fn finish(partial: Partial) -> Option<LinkedAccount> {
    if !partial.oauth || !partial.has_refresh {
        return None;
    }
    let token = LinkedToken {
        access: partial.access?,
        expires_at: partial.expires?,
    };
    let uuid = match partial.uuid {
        Some(uuid) => uuid,
        None => partial.record.strip_prefix("u-")?.to_string(),
    };
    Some(LinkedAccount {
        id: partial.record,
        uuid,
        email: partial.email,
        name: partial.name,
        plan: partial.plan,
        token,
    })
}
