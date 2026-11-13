//! Request headers and plain (non-streaming) calls.
//!
//! Headers follow pi-ai's `createClient` (bundled in DeepSeek Harness, read
//! 2026-11-11). OAuth mode adds Claude Code's identity: `user-agent:
//! claude-cli/<version>`, `x-app: cli` and the betas `claude-code-20250219`
//! and `oauth-2025-04-20`.

use super::error;
use crate::http::{Flow, HttpHead, HttpRequest, Transport};
use crate::llm::{Auth, LlmError};

/// The `anthropic-version` pi-ai's SDK sends (docs/providers.md fixes it too).
pub(crate) const API_VERSION: &str = "2023-06-01";
/// The Claude Code release pi-ai presents itself as in OAuth mode.
pub(crate) const CLAUDE_CODE_VERSION: &str = "2.1.280";
pub(crate) const BETA_CLAUDE_CODE: &str = "claude-code-20250219";
pub(crate) const BETA_OAUTH: &str = "oauth-2025-04-20";
pub(crate) const BETA_INTERLEAVED_THINKING: &str = "interleaved-thinking-2025-05-14";
pub(crate) const BETA_SERVER_SIDE_FALLBACK: &str = "server-side-fallback-2026-07-01";
pub(crate) const BETA_MID_CONVERSATION_OUTPUT_CONFIG: &str =
    "mid-conversation-output-config-2026-07-01";
pub(crate) const BETA_THINKING_BINDING_CONTROLS: &str = "thinking-binding-controls-2026-08-01";

/// Largest non-streaming body kept (models list, error bodies).
pub(crate) const MAX_BODY: usize = 8 * 1024 * 1024;
/// Limit for connect and each wait of a GET (the model list, D66): nothing
/// cancels it, and the pickers start no second fetch while one runs, so it
/// must not inherit the 600 s read limit of a model call.
pub const GET_TIMEOUT_MS: u32 = 20_000;

/// Every header of one call, in send order. Secrets appear only here.
pub(crate) fn headers(auth: &Auth, betas: &[&str]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![
        ("accept".into(), "application/json".into()),
        (
            "anthropic-dangerous-direct-browser-access".into(),
            "true".into(),
        ),
    ];
    match auth {
        Auth::ApiKey(key) => {
            let agent = format!("fast-harness/{}", env!("CARGO_PKG_VERSION"));
            out.push(("user-agent".into(), agent));
            out.push(("x-api-key".into(), key.clone()));
        }
        Auth::OAuth(token) => {
            out.push((
                "user-agent".into(),
                format!("claude-cli/{CLAUDE_CODE_VERSION}"),
            ));
            out.push(("x-app".into(), "cli".into()));
            out.push(("authorization".into(), format!("Bearer {token}")));
        }
    }
    out.push(("anthropic-version".into(), API_VERSION.into()));
    if !betas.is_empty() {
        out.push(("anthropic-beta".into(), betas.join(",")));
    }
    out
}

/// The betas every OAuth call carries.
pub(crate) fn auth_betas(auth: &Auth) -> Vec<&'static str> {
    match auth {
        Auth::OAuth(_) => vec![BETA_CLAUDE_CODE, BETA_OAUTH],
        Auth::ApiKey(_) => Vec::new(),
    }
}

/// `GET url` with the auth headers; the whole body (up to `MAX_BODY`).
pub(crate) fn get(
    http: &dyn Transport,
    url: &str,
    auth: &Auth,
) -> Result<(HttpHead, Vec<u8>), LlmError> {
    let owned = headers(auth, &auth_betas(auth));
    let refs: Vec<(&str, &str)> = owned
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let req = HttpRequest {
        method: "GET",
        url,
        headers: &refs,
        body: b"",
        timeout_ms: GET_TIMEOUT_MS,
        cancel: None,
    };
    let mut body = Vec::new();
    let head = http
        .send(&req, &mut |_, chunk| {
            body.extend_from_slice(chunk);
            if body.len() > MAX_BODY {
                Flow::Stop
            } else {
                Flow::Continue
            }
        })
        .map_err(|err| error::from_transport(&err))?;
    Ok((head, body))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::http::HttpError;

    /// Keeps the timeout of each request.
    #[derive(Default)]
    struct Limits(Mutex<Vec<u32>>);

    impl Transport for Limits {
        fn send(
            &self,
            req: &HttpRequest<'_>,
            _: &mut dyn FnMut(&HttpHead, &[u8]) -> Flow,
        ) -> Result<HttpHead, HttpError> {
            self.0.lock().unwrap().push(req.timeout_ms);
            Ok(HttpHead::new(200, Vec::new()))
        }
    }

    #[test]
    fn a_get_never_waits_the_model_call_read_limit() {
        let limits = Limits::default();
        get(&limits, "http://x/v1/models", &Auth::ApiKey("k".into())).unwrap();
        assert_eq!(*limits.0.lock().unwrap(), vec![GET_TIMEOUT_MS]);
    }
}
