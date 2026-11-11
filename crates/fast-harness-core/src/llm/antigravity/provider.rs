//! The chat [`Provider`] for `antigravity/<id>` models.
//!
//! PLACEHOLDER: every call fails until the streaming port lands; it keeps the
//! type and its `Default` so the wiring can be written against it.

use std::sync::Arc;

use super::hub::Hub;
use crate::http::Transport;
use crate::llm::{Auth, ChatRequest, Completion, Delta, ErrorKind, LlmError, Provider};

/// Calls `streamGenerateContent?alt=sse` on the Hub hosts with an
/// [`Auth::Google`] credential.
pub struct Antigravity {
    pub hub: Arc<Hub>,
}

impl Default for Antigravity {
    fn default() -> Antigravity {
        Antigravity {
            hub: Arc::new(Hub::default()),
        }
    }
}

impl Provider for Antigravity {
    fn id(&self) -> &str {
        "antigravity"
    }

    fn stream(
        &self,
        _http: &dyn Transport,
        _auth: &Auth,
        _req: &ChatRequest<'_>,
        _on_delta: &mut dyn FnMut(Delta<'_>),
    ) -> Result<Completion, LlmError> {
        Err(LlmError::new(
            ErrorKind::BadRequest,
            "the Antigravity provider is not built yet",
        ))
    }
}
