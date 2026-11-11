//! Claude subscription OAuth, as DSH's claude-switch does it: browser sign-in
//! with PKCE and a loopback callback, code exchange, refresh and the profile
//! read. Endpoints, client id, scopes, headers and timeouts are claude-switch's
//! (`plugins/dsh-claude-switch/host.js`).

mod callback;

use crate::accounts::crypto::{pkce_pair, random_token};
use crate::accounts::model::now_unix_ms;
use crate::accounts::usage::plan_of;
use crate::http::{Flow, HttpError, HttpHead, HttpRequest, Transport};
use crate::platform::PlatformError;
use serde_json::{json, Value};
use std::fmt;
use std::io;
use std::net::{Ipv4Addr, TcpListener};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// The authorize page Claude Code's own login opens.
pub const AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
/// Code exchange and refresh.
pub const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
/// Account uuid, email, name and organization (plan).
pub const PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";
/// Claude Code's public OAuth client id.
pub const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
/// The scopes Claude Code's login requests.
pub const SCOPES: &str = "org:create_api_key user:profile user:inference \
user:sessions:claude_code user:mcp_servers user:file_upload";
/// Path of the loopback callback.
pub const CALLBACK_PATH: &str = "/callback";
/// `anthropic-beta` on OAuth API calls.
pub const OAUTH_BETA: &str = "oauth-2025-04-20";
/// `user-agent` on OAuth API calls.
pub const USER_AGENT: &str = "claude-cli/2.1.0 (external, cli)";
/// Bound on profile, usage and refresh calls.
pub const REQUEST_TIMEOUT_MS: u32 = 12_000;
/// Bound on the code exchange (claude-switch gives it 30 s).
pub const EXCHANGE_TIMEOUT_MS: u32 = 30_000;
/// A grant counts as expired this long before Anthropic's expiry.
pub const OAUTH_SKEW_MS: u64 = 5 * 60_000;
/// Largest response body read.
const MAX_BODY: usize = 1 << 20;

/// Errors of the OAuth calls. None carries a token.
#[derive(Debug)]
pub enum OAuthError {
    Platform(PlatformError),
    Io(io::Error),
    Http(HttpError),
    /// The endpoint answered an error status; `reason` is Anthropic's error code and text.
    Refused {
        status: u16,
        reason: Option<String>,
    },
    /// The refresh token is spent or revoked (`invalid_grant`, `invalid_client`,
    /// `unauthorized_client`): the account must sign in again.
    Dead(String),
    /// The answer carried no usable token or identity.
    Invalid(&'static str),
    /// Anthropic answered the sign-in with `error=<code>`.
    Denied(String),
    /// The callback had no code or a state of another attempt.
    Mismatch,
    Timeout,
    Cancelled,
}

impl fmt::Display for OAuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OAuthError::Platform(err) => write!(f, "sign-in: {err}"),
            OAuthError::Io(err) => write!(f, "sign-in callback: {err}"),
            OAuthError::Http(err) => write!(f, "OAuth request failed: {err}"),
            OAuthError::Refused { status, reason } => match reason {
                Some(reason) => write!(f, "Anthropic refused (HTTP {status}, {reason})"),
                None => write!(f, "Anthropic refused (HTTP {status})"),
            },
            OAuthError::Dead(reason) => {
                write!(f, "refresh token refused ({reason}): sign in again")
            }
            OAuthError::Invalid(what) => write!(f, "Anthropic returned no usable {what}"),
            OAuthError::Denied(code) => write!(f, "Anthropic refused the sign-in: {code}"),
            OAuthError::Mismatch => f.write_str("the sign-in answer did not match this attempt"),
            OAuthError::Timeout => f.write_str("the sign-in window expired"),
            OAuthError::Cancelled => f.write_str("the sign-in was cancelled"),
        }
    }
}

impl std::error::Error for OAuthError {}

impl From<PlatformError> for OAuthError {
    fn from(err: PlatformError) -> Self {
        OAuthError::Platform(err)
    }
}

impl From<io::Error> for OAuthError {
    fn from(err: io::Error) -> Self {
        OAuthError::Io(err)
    }
}

/// A token pair. `Debug` hides the tokens.
#[derive(Clone, PartialEq, Eq)]
pub struct Tokens {
    pub refresh: String,
    pub access: String,
    /// Unix milliseconds: Anthropic's expiry minus [`OAUTH_SKEW_MS`].
    pub expires_at: u64,
}

impl fmt::Debug for Tokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tokens")
            .field("refresh", &"<redacted>")
            .field("access", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Who an access token belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub uuid: String,
    pub email: Option<String>,
    pub name: Option<String>,
    /// `plan_of(organization)`: `Pro`, `Max 20x`, ...
    pub plan: Option<String>,
}

/// One sign-in attempt: the PKCE verifier, the state and the loopback listener.
pub struct LoginFlow {
    listener: TcpListener,
    verifier: String,
    state: String,
    redirect_uri: String,
    url: String,
}

impl fmt::Debug for LoginFlow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoginFlow")
            .field("redirect_uri", &self.redirect_uri)
            .finish_non_exhaustive()
    }
}

/// Starts a sign-in: PKCE pair, random state, a listener on 127.0.0.1 (any
/// free port) and the authorize URL, with claude-switch's parameters.
pub fn begin_login() -> Result<LoginFlow, OAuthError> {
    let (verifier, challenge) = pkce_pair()?;
    let state = random_token(24)?;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://localhost:{port}{CALLBACK_PATH}");
    let params = [
        ("code", "true"),
        ("client_id", CLIENT_ID),
        ("response_type", "code"),
        ("redirect_uri", redirect_uri.as_str()),
        ("scope", SCOPES),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("state", state.as_str()),
    ];
    let query: Vec<String> = params
        .iter()
        .map(|(key, value)| format!("{key}={}", callback::form_encode(value)))
        .collect();
    let url = format!("{AUTHORIZE_URL}?{}", query.join("&"));
    Ok(LoginFlow {
        listener,
        verifier,
        state,
        redirect_uri,
        url,
    })
}

impl LoginFlow {
    /// The authorize URL to open in the browser.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// `http://localhost:<port>/callback`.
    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    /// The `state` this attempt expects back.
    pub fn state(&self) -> &str {
        &self.state
    }

    /// Waits for `GET /callback` and returns its code. Other paths (such as
    /// `/favicon.ico`) get a 404 and the wait goes on.
    pub fn wait(&self, timeout: Duration, cancel: &AtomicBool) -> Result<String, OAuthError> {
        callback::wait_for_code(&self.listener, &self.state, timeout, cancel)
    }
}

/// Trades the callback code for tokens.
pub fn exchange_code(
    http: &dyn Transport,
    flow: &LoginFlow,
    code: &str,
) -> Result<Tokens, OAuthError> {
    let body = json!({
        "grant_type": "authorization_code",
        "client_id": CLIENT_ID,
        "code": code,
        "state": flow.state,
        "redirect_uri": flow.redirect_uri,
        "code_verifier": flow.verifier,
    });
    let (head, bytes) = post_token(http, &body, EXCHANGE_TIMEOUT_MS)?;
    if !ok(&head) {
        let reason = reason_of(&bytes);
        return Err(OAuthError::Refused {
            status: head.status,
            reason,
        });
    }
    tokens_from(&bytes, None)
}

/// Spends `refresh_token` for a new pair. Without `scope`, as claude-switch:
/// the grant keeps the scopes it was issued with.
pub fn refresh(http: &dyn Transport, refresh_token: &str) -> Result<Tokens, OAuthError> {
    let body = json!({
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
        "client_id": CLIENT_ID,
    });
    let (head, bytes) = post_token(http, &body, REQUEST_TIMEOUT_MS)?;
    if !ok(&head) {
        let reason = reason_of(&bytes);
        return Err(match reason {
            Some(reason) if head.status == 400 && is_dead(&reason) => OAuthError::Dead(reason),
            reason => OAuthError::Refused {
                status: head.status,
                reason,
            },
        });
    }
    tokens_from(&bytes, Some(refresh_token))
}

/// Reads who `access` belongs to.
pub fn profile(http: &dyn Transport, access: &str) -> Result<Profile, OAuthError> {
    let (head, bytes) = get_with_token(http, PROFILE_URL, access).map_err(OAuthError::Http)?;
    if !ok(&head) {
        let reason = reason_of(&bytes);
        return Err(OAuthError::Refused {
            status: head.status,
            reason,
        });
    }
    let body: Value = serde_json::from_slice(&bytes).map_err(|_| OAuthError::Invalid("profile"))?;
    let account = &body["account"];
    let uuid = text(&account["uuid"]).ok_or(OAuthError::Invalid("profile"))?;
    Ok(Profile {
        uuid,
        email: text(&account["email"]),
        name: text(&account["display_name"]).or_else(|| text(&account["full_name"])),
        plan: plan_of(&body["organization"]),
    })
}

/// A GET with a bearer token and Claude Code's OAuth headers.
pub(crate) fn get_with_token(
    http: &dyn Transport,
    url: &str,
    access: &str,
) -> Result<(HttpHead, Vec<u8>), HttpError> {
    let bearer = format!("Bearer {access}");
    let headers = [
        ("authorization", bearer.as_str()),
        ("content-type", "application/json"),
        ("anthropic-beta", OAUTH_BETA),
        ("user-agent", USER_AGENT),
    ];
    let req = HttpRequest {
        method: "GET",
        url,
        headers: &headers,
        body: b"",
        timeout_ms: REQUEST_TIMEOUT_MS,
    };
    fetch(http, &req)
}

/// Sends `req` and collects up to [`MAX_BODY`] bytes of the answer.
pub(crate) fn fetch(
    http: &dyn Transport,
    req: &HttpRequest<'_>,
) -> Result<(HttpHead, Vec<u8>), HttpError> {
    let mut body = Vec::new();
    let head = http.send(req, &mut |_, chunk| {
        let room = MAX_BODY.saturating_sub(body.len());
        body.extend_from_slice(&chunk[..chunk.len().min(room)]);
        if body.len() >= MAX_BODY {
            Flow::Stop
        } else {
            Flow::Continue
        }
    })?;
    Ok((head, body))
}

fn post_token(
    http: &dyn Transport,
    body: &Value,
    timeout_ms: u32,
) -> Result<(HttpHead, Vec<u8>), OAuthError> {
    let bytes = body.to_string().into_bytes();
    let headers = [
        ("content-type", "application/json"),
        ("accept", "application/json"),
    ];
    let req = HttpRequest {
        method: "POST",
        url: TOKEN_URL,
        headers: &headers,
        body: &bytes,
        timeout_ms,
    };
    fetch(http, &req).map_err(OAuthError::Http)
}

fn ok(head: &HttpHead) -> bool {
    (200..300).contains(&head.status)
}

/// The pair in a token answer; a refresh answer without `refresh_token` keeps `old_refresh`.
fn tokens_from(bytes: &[u8], old_refresh: Option<&str>) -> Result<Tokens, OAuthError> {
    let body: Value = serde_json::from_slice(bytes).map_err(|_| OAuthError::Invalid("token"))?;
    let access = text(&body["access_token"]).ok_or(OAuthError::Invalid("token"))?;
    let refresh = text(&body["refresh_token"])
        .or_else(|| old_refresh.map(str::to_string))
        .ok_or(OAuthError::Invalid("token"))?;
    let expires_in = number(&body["expires_in"])
        .filter(|seconds| *seconds >= 0.0)
        .ok_or(OAuthError::Invalid("token"))?;
    let lifetime = (expires_in * 1000.0).min(u64::MAX as f64) as u64;
    let expires_at = now_unix_ms()
        .saturating_add(lifetime)
        .saturating_sub(OAUTH_SKEW_MS);
    Ok(Tokens {
        refresh,
        access,
        expires_at,
    })
}

/// claude-switch's `oauthErrorOf`: `<code>: <description>`, at most 200 characters.
fn reason_of(bytes: &[u8]) -> Option<String> {
    let body: Value = serde_json::from_slice(bytes).ok()?;
    let nested = body["error"].as_object();
    let code = nested
        .and_then(|error| error.get("type").and_then(text))
        .or_else(|| text(&body["error"]))
        .or_else(|| text(&body["code"]));
    let description = nested
        .and_then(|error| error.get("message").and_then(text))
        .or_else(|| text(&body["error_description"]))
        .or_else(|| text(&body["description"]));
    let reason = [code, description]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(": ");
    (!reason.is_empty()).then(|| reason.chars().take(200).collect())
}

/// `DEAD_REFRESH`: `/^(invalid_grant|invalid_client|unauthorized_client)\b/`.
fn is_dead(reason: &str) -> bool {
    ["invalid_grant", "invalid_client", "unauthorized_client"]
        .iter()
        .any(|code| {
            reason.strip_prefix(code).is_some_and(|rest| {
                !rest
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
            })
        })
}

/// A non-empty string.
fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
}

/// A finite number, or a string holding one.
pub(crate) fn number(value: &Value) -> Option<f64> {
    let parsed = match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    };
    parsed.filter(|n| n.is_finite())
}
