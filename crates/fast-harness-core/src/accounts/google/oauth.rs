//! Google sign-in as Antigravity does it (PLAN.md D23): browser authorize
//! with PKCE S256 and a loopback `/oauth-callback`, offline access, the code
//! exchange and refresh as form posts with the client id and secret, and the
//! v1 userinfo read.

use super::{text, GoogleApi, GoogleClient, GoogleError};
use crate::accounts::crypto::{pkce_pair, random_token};
use crate::accounts::model::now_unix_ms;
use crate::accounts::oauth::callback::{form_encode, wait_for_code};
use crate::accounts::oauth::{fetch, is_dead, number, Tokens};
use crate::http::{HttpRequest, Transport};
use serde_json::Value;
use std::fmt;
use std::net::{Ipv4Addr, TcpListener};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub const AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
/// Code exchange and refresh.
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
/// Answers `id`, `email`, `name`.
pub const USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v1/userinfo?alt=json";
/// The scopes Antigravity asks for.
pub const SCOPES: &str = "https://www.googleapis.com/auth/cloud-platform \
https://www.googleapis.com/auth/userinfo.email \
https://www.googleapis.com/auth/userinfo.profile \
https://www.googleapis.com/auth/cclog \
https://www.googleapis.com/auth/experimentsandconfigs";
/// Path of the loopback callback.
pub const CALLBACK_PATH: &str = "/oauth-callback";
/// A token counts as expired this long before Google's expiry.
pub const SKEW_MS: u64 = 5 * 60_000;
/// Bound on token and userinfo calls.
pub const REQUEST_TIMEOUT_MS: u32 = 30_000;
/// Lifetime assumed when the answer names none (Google's is one hour).
const DEFAULT_LIFETIME_S: f64 = 3600.0;

/// Who an access token belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoogleUser {
    /// Google's user id (digits).
    pub id: String,
    pub email: Option<String>,
    pub name: Option<String>,
}

/// One sign-in attempt: the PKCE verifier, the state and the loopback listener.
pub struct GoogleLoginFlow {
    listener: TcpListener,
    verifier: String,
    state: String,
    redirect_uri: String,
    url: String,
}

impl fmt::Debug for GoogleLoginFlow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GoogleLoginFlow")
            .field("redirect_uri", &self.redirect_uri)
            .finish_non_exhaustive()
    }
}

/// Starts a sign-in: PKCE pair, random state, a listener on 127.0.0.1 (any
/// free port) and the authorize URL.
pub fn begin_login(api: &GoogleApi, client: &GoogleClient) -> Result<GoogleLoginFlow, GoogleError> {
    let (verifier, challenge) = pkce_pair()?;
    let state = random_token(24)?;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://localhost:{port}{CALLBACK_PATH}");
    let url = authorize_url(api, &client.id, &redirect_uri, &challenge, &state);
    Ok(GoogleLoginFlow {
        listener,
        verifier,
        state,
        redirect_uri,
        url,
    })
}

/// The authorize URL with Antigravity's parameters.
pub fn authorize_url(
    api: &GoogleApi,
    client_id: &str,
    redirect_uri: &str,
    challenge: &str,
    state: &str,
) -> String {
    let params = [
        ("client_id", client_id),
        ("response_type", "code"),
        ("redirect_uri", redirect_uri),
        ("scope", SCOPES),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
        ("access_type", "offline"),
        ("prompt", "consent select_account"),
    ];
    format!("{}?{}", api.authorize_url, form(&params))
}

impl GoogleLoginFlow {
    /// The authorize URL to open in the browser.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// `http://localhost:<port>/oauth-callback`.
    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    /// The `state` this attempt expects back.
    pub fn state(&self) -> &str {
        &self.state
    }

    /// Waits for `GET /oauth-callback` and returns its code.
    pub fn wait(&self, timeout: Duration, cancel: &AtomicBool) -> Result<String, GoogleError> {
        wait_for_code(&self.listener, CALLBACK_PATH, &self.state, timeout, cancel)
            .map_err(GoogleError::from)
    }
}

impl GoogleApi {
    /// Trades the callback code of `flow` for tokens.
    pub fn exchange_code(
        &self,
        http: &dyn Transport,
        client: &GoogleClient,
        flow: &GoogleLoginFlow,
        code: &str,
    ) -> Result<Tokens, GoogleError> {
        let fields = [
            ("client_id", client.id.as_str()),
            ("client_secret", client.secret.as_str()),
            ("code", code),
            ("grant_type", "authorization_code"),
            ("redirect_uri", flow.redirect_uri.as_str()),
            ("code_verifier", flow.verifier.as_str()),
        ];
        self.token_call(http, &fields, None)
    }

    /// Spends `refresh_token` for a new access token; an answer without
    /// `refresh_token` keeps the old one (Google does not rotate it).
    pub fn refresh(
        &self,
        http: &dyn Transport,
        client: &GoogleClient,
        refresh_token: &str,
    ) -> Result<Tokens, GoogleError> {
        let fields = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client.id.as_str()),
            ("client_secret", client.secret.as_str()),
        ];
        self.token_call(http, &fields, Some(refresh_token))
    }

    /// Who `access` belongs to.
    pub fn userinfo(&self, http: &dyn Transport, access: &str) -> Result<GoogleUser, GoogleError> {
        let bearer = format!("Bearer {access}");
        let headers = [
            ("authorization", bearer.as_str()),
            ("accept", "application/json"),
        ];
        let req = HttpRequest {
            method: "GET",
            url: &self.userinfo_url,
            headers: &headers,
            body: b"",
            timeout_ms: REQUEST_TIMEOUT_MS,
        };
        let (head, bytes) = fetch(http, &req)?;
        let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        let id = text(&body["id"]).or_else(|| body["id"].as_u64().map(|n| n.to_string()));
        match id {
            Some(id) if head.status == 200 => Ok(GoogleUser {
                id,
                email: text(&body["email"]),
                name: text(&body["name"]),
            }),
            _ if head.status != 200 => Err(refusal(head.status, &body)),
            _ => Err(GoogleError::Invalid("user id")),
        }
    }

    fn token_call(
        &self,
        http: &dyn Transport,
        fields: &[(&str, &str)],
        old_refresh: Option<&str>,
    ) -> Result<Tokens, GoogleError> {
        let body = form(fields).into_bytes();
        let headers = [
            ("content-type", "application/x-www-form-urlencoded"),
            ("accept", "application/json"),
        ];
        let req = HttpRequest {
            method: "POST",
            url: &self.token_url,
            headers: &headers,
            body: &body,
            timeout_ms: REQUEST_TIMEOUT_MS,
        };
        let (head, bytes) = fetch(http, &req)?;
        let json: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if head.status != 200 || text(&json["access_token"]).is_none() {
            return Err(match refusal(head.status, &json) {
                GoogleError::Refused {
                    reason: Some(reason),
                    ..
                } if is_dead(&reason) => GoogleError::Dead(reason),
                other => other,
            });
        }
        tokens_of(&json, old_refresh)
    }
}

/// The pair in a token answer.
fn tokens_of(json: &Value, old_refresh: Option<&str>) -> Result<Tokens, GoogleError> {
    let access = text(&json["access_token"]).ok_or(GoogleError::Invalid("token"))?;
    let refresh = text(&json["refresh_token"])
        .or_else(|| old_refresh.map(str::to_string))
        .ok_or(GoogleError::Invalid("refresh token"))?;
    let seconds = number(&json["expires_in"])
        .filter(|s| *s >= 0.0)
        .unwrap_or(DEFAULT_LIFETIME_S);
    let lifetime = (seconds * 1000.0).min(u64::MAX as f64) as u64;
    Ok(Tokens {
        refresh,
        access,
        expires_at: now_unix_ms()
            .saturating_add(lifetime)
            .saturating_sub(SKEW_MS),
    })
}

/// `Refused` with Google's `error[: error_description]`, at most 200 characters.
pub(crate) fn refusal(status: u16, json: &Value) -> GoogleError {
    let error = text(&json["error"]).or_else(|| text(&json["error"]["status"]));
    let description = text(&json["error_description"]).or_else(|| text(&json["error"]["message"]));
    let reason = [error, description]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(": ");
    GoogleError::Refused {
        status,
        reason: (!reason.is_empty()).then(|| reason.chars().take(200).collect()),
    }
}

/// `application/x-www-form-urlencoded` pairs.
fn form(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(key, value)| format!("{key}={}", form_encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}
