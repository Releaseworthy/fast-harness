//! The chat [`Provider`] for `antigravity/<id>` models (PLAN.md D23, D24).
//!
//! One call is `POST {host}/v1internal:streamGenerateContent?alt=sse` with the
//! Google access token as a bearer, the Hub User-Agent and the envelope of
//! [`request::build`]. The Hub hosts are tried in order: the next one only
//! when the failure came before any answer byte and [`next_host_on`] says so
//! (404, 408, 5xx) or the connection failed; a timeout never moves on, since
//! another host would only start the same long generation again (the DSH
//! proxy `plugins/dsh-antigravity-switch/host.js` `contentCall`). The answer
//! is assembled by [`stream::Assembler`](super::stream::Assembler).
//!
//! [`next_host_on`]: super::hub::next_host_on

pub(crate) mod error;
mod run;

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use super::hub::Hub;
use super::{bare_id, request};
use crate::http::Transport;
use crate::llm::{Auth, ChatRequest, Completion, Delta, ErrorKind, LlmError, Provider};

/// Calls `streamGenerateContent?alt=sse` on the Hub hosts with an
/// [`Auth::Google`] credential.
pub struct Antigravity {
    pub hub: Arc<Hub>,
    /// `request.sessionId` of every call of this instance.
    session_id: String,
}

impl Default for Antigravity {
    fn default() -> Antigravity {
        Antigravity::new(Arc::new(Hub::default()))
    }
}

impl Antigravity {
    /// A provider on `hub` with a fresh random session id.
    pub fn new(hub: Arc<Hub>) -> Antigravity {
        Antigravity {
            hub,
            session_id: new_session_id(),
        }
    }

    /// The session id sent as `request.sessionId` (stable for this instance).
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
}

/// `-<decimal>` below 9e18 from 8 random bytes, the form the Antigravity
/// client sends (host.js `sessionOf`).
fn new_session_id() -> String {
    let mut bytes = [0u8; 8];
    if crate::platform::random_fill(&mut bytes).is_err() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let mixed = nanos ^ u64::from(std::process::id()).rotate_left(40);
        bytes = mixed.to_le_bytes();
    }
    format!("-{}", u64::from_le_bytes(bytes) % 9_000_000_000_000_000_000)
}

pub(crate) fn aborted() -> LlmError {
    LlmError::new(ErrorKind::Aborted, "cancelled")
}

impl Provider for Antigravity {
    fn id(&self) -> &str {
        "antigravity"
    }

    fn stream(
        &self,
        http: &dyn Transport,
        auth: &Auth,
        req: &ChatRequest<'_>,
        on_delta: &mut dyn FnMut(Delta<'_>),
    ) -> Result<Completion, LlmError> {
        if req.cancel.load(Ordering::Relaxed) {
            return Err(aborted());
        }
        let Auth::Google { access, project } = auth else {
            let message = "Antigravity needs a Google account credential";
            return Err(LlmError::new(ErrorKind::Auth, message));
        };
        let model = bare_id(req.model).ok_or_else(|| {
            let message = format!("{} is not an Antigravity model id", req.model);
            LlmError::new(ErrorKind::BadRequest, message)
        })?;
        let envelope = request::build(req, model, project, &self.session_id)?;
        let body = envelope.body.to_string();
        let bearer = format!("Bearer {access}");
        let user_agent = self.hub.user_agent(http);
        let headers = [
            ("Authorization", bearer.as_str()),
            ("Content-Type", "application/json"),
            ("Accept", "text/event-stream"),
            ("User-Agent", user_agent.as_str()),
        ];
        let call = run::Call {
            http,
            headers: &headers,
            body: body.as_bytes(),
            req,
            names: &envelope.names,
        };
        call.on_hosts(self.hub.hosts(), on_delta)
            .map_err(|err| error::redact(err, access))
    }
}
