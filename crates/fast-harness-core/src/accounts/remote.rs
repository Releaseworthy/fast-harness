//! Usage of an account nobody is using right now: `GET /api/oauth/usage`, as
//! claude-switch's `fetchUsage`.

use crate::accounts::oauth::{get_with_token, number};
use crate::accounts::usage::{usage_from_endpoint, Usage};
use crate::http::HttpError;
use serde_json::Value;
use std::fmt;

/// Subscription usage windows of the access token's account.
pub const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

/// Errors of the usage read. None carries a token.
#[derive(Debug)]
pub enum RemoteError {
    Http(HttpError),
    /// An error status; `retry_after_ms` is the `retry-after` header (a 429 often says 0).
    Status {
        status: u16,
        retry_after_ms: Option<u64>,
    },
    /// A success without a five-hour or seven-day window.
    Invalid,
}

impl RemoteError {
    /// True for a 429 answer.
    pub fn is_rate_limit(&self) -> bool {
        matches!(self, RemoteError::Status { status: 429, .. })
    }

    /// The server's `retry-after`, in milliseconds.
    pub fn retry_after_ms(&self) -> Option<u64> {
        match self {
            RemoteError::Status { retry_after_ms, .. } => *retry_after_ms,
            _ => None,
        }
    }
}

impl fmt::Display for RemoteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RemoteError::Http(err) => write!(f, "usage read failed: {err}"),
            RemoteError::Status { status, .. } => write!(f, "usage read answered HTTP {status}"),
            RemoteError::Invalid => f.write_str("usage read returned no usage windows"),
        }
    }
}

impl std::error::Error for RemoteError {}

/// Reads the usage windows of the account `access` belongs to.
pub fn fetch_usage(http: &dyn crate::http::Transport, access: &str) -> Result<Usage, RemoteError> {
    let (head, bytes) = get_with_token(http, USAGE_URL, access).map_err(RemoteError::Http)?;
    if !(200..300).contains(&head.status) {
        let retry_after_ms = head
            .header("retry-after")
            .and_then(|raw| number(&Value::String(raw.to_string())))
            .filter(|seconds| *seconds >= 0.0)
            .map(|seconds| (seconds * 1000.0) as u64);
        return Err(RemoteError::Status {
            status: head.status,
            retry_after_ms,
        });
    }
    let body: Value = serde_json::from_slice(&bytes).map_err(|_| RemoteError::Invalid)?;
    usage_from_endpoint(&body).ok_or(RemoteError::Invalid)
}
