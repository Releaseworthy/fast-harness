//! Google Antigravity accounts (PLAN.md D23 to D26), ported from DSH's
//! antigravity-switch:
//!
//! - `client`: the Google OAuth client, from the environment or `google-client.json`.
//! - `oauth`: browser sign-in (PKCE, loopback `/oauth-callback`), code exchange, refresh, userinfo.
//! - `project`: the Cloud Code project and plan (`loadCodeAssist`, `onboardUser`).
//! - `quota`: `retrieveUserQuotaSummary` groups with their 5 h and weekly windows.
//! - `signin`: the whole sign-in, stored as a `provider: google` account.
//!
//! Control calls go through [`crate::llm::antigravity::hub`].

pub mod client;
pub mod oauth;
pub mod project;
pub mod quota;
pub mod signin;

use crate::accounts::oauth::OAuthError;
use crate::accounts::store::StoreError;
use crate::http::HttpError;
use crate::llm::antigravity::hub::Hub;
use crate::platform::PlatformError;
use std::fmt;
use std::io;
use std::time::Duration;

pub use client::{client_from, load_client, save_client, GoogleClient};
pub use oauth::{begin_login, GoogleLoginFlow, GoogleUser};
pub use project::{ProjectInfo, FALLBACK_PROJECT_ID};
pub use quota::{parse_quota_summary, GoogleQuota, QuotaGroup, QuotaWindow};
pub use signin::{complete_sign_in, fetch_sign_in, save_sign_in, SignIn};

/// The Google endpoints and the Hub. `Default` is the real service; tests
/// point every URL at a scripted transport.
#[derive(Debug)]
pub struct GoogleApi {
    pub authorize_url: String,
    pub token_url: String,
    pub userinfo_url: String,
    pub hub: Hub,
    /// Pause between two `onboardUser` polls (5 s, as the plugin).
    pub onboard_wait: Duration,
}

impl Default for GoogleApi {
    fn default() -> Self {
        GoogleApi {
            authorize_url: oauth::AUTHORIZE_URL.into(),
            token_url: oauth::TOKEN_URL.into(),
            userinfo_url: oauth::USERINFO_URL.into(),
            hub: Hub::default(),
            onboard_wait: Duration::from_secs(5),
        }
    }
}

/// Errors of the Google account code. None carries a token or the client secret.
#[derive(Debug)]
pub enum GoogleError {
    /// No OAuth client is configured (PLAN.md D25).
    NoClient,
    /// `google-client.json` cannot be read or written; the text names the problem only.
    Client(String),
    Platform(PlatformError),
    Io(io::Error),
    Http(HttpError),
    /// Google answered an error status; `reason` is its error code and text.
    Refused {
        status: u16,
        reason: Option<String>,
    },
    /// The refresh token is spent or revoked: the account must sign in again.
    Dead(String),
    /// The answer carried no usable token, identity or JSON.
    Invalid(&'static str),
    /// Google answered the sign-in with `error=<code>`.
    Denied(String),
    /// The browser side of the sign-in failed (mismatch, timeout, cancel).
    Login(OAuthError),
    Store(StoreError),
}

impl fmt::Display for GoogleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GoogleError::NoClient => f.write_str(
                "no Google OAuth client: set FAST_HARNESS_GOOGLE_CLIENT_ID and \
FAST_HARNESS_GOOGLE_CLIENT_SECRET, or run `fast-harness accounts google-client --id <id>` \
and paste the client secret on stdin",
            ),
            GoogleError::Client(what) => write!(f, "Google OAuth client: {what}"),
            GoogleError::Platform(err) => write!(f, "Google sign-in: {err}"),
            GoogleError::Io(err) => write!(f, "Google sign-in: {err}"),
            GoogleError::Http(err) => write!(f, "Google request failed: {err}"),
            GoogleError::Refused { status, reason } => match reason {
                Some(reason) => write!(f, "Google refused (HTTP {status}, {reason})"),
                None => write!(f, "Google refused (HTTP {status})"),
            },
            GoogleError::Dead(reason) => {
                write!(
                    f,
                    "Google refused the refresh token ({reason}): sign in again"
                )
            }
            GoogleError::Invalid(what) => write!(f, "Google returned no usable {what}"),
            GoogleError::Denied(code) => write!(f, "Google refused the sign-in: {code}"),
            GoogleError::Login(err) => write!(f, "{err}"),
            GoogleError::Store(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for GoogleError {}

impl From<PlatformError> for GoogleError {
    fn from(err: PlatformError) -> Self {
        GoogleError::Platform(err)
    }
}

impl From<io::Error> for GoogleError {
    fn from(err: io::Error) -> Self {
        GoogleError::Io(err)
    }
}

impl From<HttpError> for GoogleError {
    fn from(err: HttpError) -> Self {
        GoogleError::Http(err)
    }
}

impl From<StoreError> for GoogleError {
    fn from(err: StoreError) -> Self {
        GoogleError::Store(err)
    }
}

impl From<OAuthError> for GoogleError {
    fn from(err: OAuthError) -> Self {
        match err {
            OAuthError::Denied(code) => GoogleError::Denied(code),
            OAuthError::Io(err) => GoogleError::Io(err),
            OAuthError::Platform(err) => GoogleError::Platform(err),
            other => GoogleError::Login(other),
        }
    }
}

/// A non-empty string.
fn text(value: &serde_json::Value) -> Option<String> {
    value
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
}
