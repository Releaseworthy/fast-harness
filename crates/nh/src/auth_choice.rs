//! Where print mode gets its credential. For now a fixed one from the
//! environment; the account pool replaces this function later.

use std::sync::Arc;

use nh_core::auth::{AuthSource, StaticAuth};
use nh_core::llm::Auth;

fn env_value(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// `ANTHROPIC_API_KEY`, else `NH_OAUTH_TOKEN`, else an error naming both.
pub fn auth_source() -> Result<Arc<dyn AuthSource>, String> {
    if let Some(key) = env_value("ANTHROPIC_API_KEY") {
        let auth = StaticAuth::new("env-api-key", "ANTHROPIC_API_KEY", Auth::ApiKey(key));
        return Ok(Arc::new(auth));
    }
    if let Some(token) = env_value("NH_OAUTH_TOKEN") {
        let auth = StaticAuth::new("env-oauth", "NH_OAUTH_TOKEN", Auth::OAuth(token));
        return Ok(Arc::new(auth));
    }
    Err(
        "no credential: set ANTHROPIC_API_KEY (API key) or NH_OAUTH_TOKEN (subscription token)"
            .into(),
    )
}
