//! Request headers and plain (non-streaming) calls.
//!
//! Headers follow pi-ai's `createClient` (bundled in DeepSeek Harness, read
//! 2026-11-11). OAuth mode adds Claude Code's identity: `user-agent:
//! claude-cli/<version>`, `x-app: cli` and the betas `claude-code-20250219`
//! and `oauth-2025-04-20`.

use super::error;
use crate::http::{Flow, HttpHead, HttpRequest, Transport};
use crate::llm::{Auth, ErrorKind, LlmError};

/// The `anthropic-version` pi-ai's SDK sends (PLAN.md fixes it too).
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
        // Never sent: eject_foreign stops it before any request.
        Auth::Google { .. } => {}
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
        Auth::ApiKey(_) | Auth::Google { .. } => Vec::new(),
    }
}

/// A credential of another provider: an Auth error instead of a request.
pub(crate) fn reject_foreign(auth: &Auth) -> Result<(), LlmError> {
    match auth {
        Auth::Google { .. } => Err(LlmError::new(
            ErrorKind::Auth,
            "a Google credential cannot call Anthropic",
        )),
        Auth::ApiKey(_) | Auth::OAuth(_) => Ok(()),
    }
}

/// `GET url` with the auth headers; the whole body (up to `MAX_BODY`).
pub(crate) fn get(
    http: &dyn Transport,
    url: &str,
    auth: &Auth,
) -> Result<(HttpHead, Vec<u8>), LlmError> {
    reject_foreign(auth)?;
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
        timeout_ms: 0,
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
