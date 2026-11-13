//! The model boundary: one streaming call in, one assembled message out.
//! Providers: Claude (`anthropic`) and the providers of `providers/*.json`
//! (`custom`, `openai`: models named `<provider id>/<id>`); `catalog` picks one by model id.

pub mod anthropic;
pub mod catalog;
pub mod custom;
pub mod find;
pub mod openai;

pub use catalog::Catalog;

use std::fmt;
use std::sync::atomic::AtomicBool;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::http::{HttpHead, Transport};
use crate::types::{Message, StopReason, Usage};

/// Credential for one call. Passed per call: the account layer may switch
/// accounts between two steps, or between two attempts of one step.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Auth {
    /// Sent as `x-api-key`.
    ApiKey(String),
    /// Claude subscription access token, sent as `Authorization: Bearer`.
    OAuth(String),
}

impl fmt::Debug for Auth {
    // Never print the secret itself.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Auth::ApiKey(_) => f.write_str("ApiKey(<redacted>)"),
            Auth::OAuth(_) => f.write_str("OAuth(<redacted>)"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema of the arguments object.
    pub schema: Value,
}

impl ToolSpec {
    pub fn new(name: impl Into<String>, description: impl Into<String>, schema: Value) -> ToolSpec {
        ToolSpec {
            name: name.into(),
            description: description.into(),
            schema,
        }
    }
}

#[non_exhaustive]
pub struct ChatRequest<'a> {
    pub model: &'a str,
    pub system: &'a str,
    pub messages: &'a [Message],
    /// Every tool of the agent. With [`find::FIND_TOOLS`] among them the
    /// others are deferred until a find result names them (PLAN.md D30).
    pub tools: &'a [ToolSpec],
    pub max_tokens: u32,
    /// Reasoning effort (`low`, `medium`, `high`, `max`), `off` to disable
    /// thinking, `None` for the model's default.
    pub effort: Option<&'a str>,
    /// Lifetimes of the prompt cache breakpoints.
    pub cache: Cache,
    /// The conversation id; a provider may derive a request session id from it
    /// (empty: the provider instance's own id).
    pub session: &'a str,
    /// Set from another thread to abort; the call then fails with `ErrorKind::Aborted`.
    pub cancel: &'a AtomicBool,
}

impl<'a> ChatRequest<'a> {
    /// A request with the model's default effort, 5 minute cache
    /// breakpoints and the provider's own session id.
    pub fn new(
        model: &'a str,
        system: &'a str,
        messages: &'a [Message],
        tools: &'a [ToolSpec],
        max_tokens: u32,
        cancel: &'a AtomicBool,
    ) -> ChatRequest<'a> {
        ChatRequest {
            model,
            system,
            messages,
            tools,
            max_tokens,
            effort: None,
            cache: Cache::default(),
            session: "",
            cancel,
        }
    }

    pub fn with_effort(mut self, effort: Option<&'a str>) -> ChatRequest<'a> {
        self.effort = effort;
        self
    }

    pub fn with_cache(mut self, cache: Cache) -> ChatRequest<'a> {
        self.cache = cache;
        self
    }

    pub fn with_session(mut self, session: &'a str) -> ChatRequest<'a> {
        self.session = session;
        self
    }
}

/// Prompt cache lifetimes (PLAN.md D36): `true` asks for 1 hour, `false` for
/// 5 minutes. The tools and system breakpoints share `system_1h`; a 1 hour
/// breakpoint must come before 5 minute ones, so `messages_1h` implies it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Cache {
    pub system_1h: bool,
    pub messages_1h: bool,
}

impl Cache {
    pub fn new(system_1h: bool, messages_1h: bool) -> Cache {
        Cache {
            system_1h,
            messages_1h,
        }
    }
}

/// The first and last line of the memory index block (PLAN.md D50).
pub const MEMORY_OPEN: &str = "<memory_index>";
pub const MEMORY_CLOSE: &str = "</memory_index>";

/// `system` split into its stable part and the memory index block that ends
/// it after a blank line (PLAN.md D50). Claude sends them as two system
/// blocks with a breakpoint each, so the stable block stays byte-identical
/// across the sessions of a workspace; other providers send the text whole.
pub fn split_memory(system: &str) -> (&str, Option<&str>) {
    let opener = format!("\n\n{MEMORY_OPEN}\n");
    match system.rfind(&opener) {
        Some(at) if system.ends_with(MEMORY_CLOSE) => (&system[..at], Some(&system[at + 2..])),
        _ => (system, None),
    }
}

/// Incremental output, delivered on the calling thread in arrival order.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Delta<'a> {
    Text(&'a str),
    Thinking(&'a str),
    /// A tool call block opened. `index` counts the tool calls of this message from 0.
    ToolStart {
        index: usize,
        id: &'a str,
        name: &'a str,
    },
    /// A piece of the raw JSON arguments of tool call `index`.
    ToolArgs {
        index: usize,
        chunk: &'a str,
    },
}

/// Wall-clock measurements of one call in milliseconds, counted from the
/// moment the request was handed to the transport.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct Timing {
    /// Until the first response body byte.
    #[serde(rename = "ttfbMs")]
    pub ttfb_ms: u64,
    /// Until the first text, thinking or tool delta.
    #[serde(rename = "firstTokenMs")]
    pub first_token_ms: u64,
    #[serde(rename = "totalMs")]
    pub total_ms: u64,
    /// Body chunks received from the transport.
    pub chunks: u32,
    /// Body bytes received.
    pub bytes: u64,
}

impl Timing {
    /// The three times in milliseconds; `chunks` and `bytes` are 0.
    pub fn new(ttfb_ms: u64, first_token_ms: u64, total_ms: u64) -> Timing {
        Timing {
            ttfb_ms,
            first_token_ms,
            total_ms,
            chunks: 0,
            bytes: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Completion {
    pub message: Message,
    pub usage: Usage,
    pub stop: StopReason,
    pub timing: Timing,
    /// Status and response headers; the rate-limit headers feed the account layer.
    pub head: HttpHead,
    /// Claude `message_start.message.input_transformations`, when the server
    /// reported changes to the request; traced as `request/transformations`.
    pub transformations: Option<Value>,
}

impl Completion {
    /// A completion with zero timing, an empty head (status 0) and no
    /// transformations; set those fields afterwards when known.
    pub fn new(message: Message, usage: Usage, stop: StopReason) -> Completion {
        Completion {
            message,
            usage,
            stop,
            timing: Timing::default(),
            head: HttpHead::default(),
            transformations: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ErrorKind {
    /// 401 or 403: the credential is wrong, expired or not allowed.
    Auth,
    /// 429: rate or quota limit of this credential.
    RateLimit,
    /// 529 or an `overloaded_error` event.
    Overloaded,
    /// Other 5xx.
    Server,
    /// Other 4xx: the same request cannot succeed.
    BadRequest,
    /// Transport failure before or during the stream.
    Network,
    Aborted,
    /// The stream ended early or carried data that cannot be interpreted.
    Protocol,
    /// A 400 or 413 "prompt is too long": the request does not fit the
    /// model's context window (the turn ends `ContextOverflow`).
    ContextOverflow,
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct LlmError {
    pub kind: ErrorKind,
    pub status: Option<u16>,
    pub message: String,
    pub head: Option<HttpHead>,
}

impl LlmError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> LlmError {
        LlmError {
            kind,
            status: None,
            message: message.into(),
            head: None,
        }
    }

    pub fn with_status(mut self, status: u16) -> LlmError {
        self.status = Some(status);
        self
    }

    pub fn with_head(mut self, head: HttpHead) -> LlmError {
        self.head = Some(head);
        self
    }

    /// True when sending the same request again with the same credential can succeed.
    pub fn retryable(&self) -> bool {
        use ErrorKind::{Network, Overloaded, Protocol, Server};
        matches!(self.kind, Overloaded | Server | Network | Protocol)
    }

    /// The server's `retry-after` header (seconds) in milliseconds.
    pub fn retry_after_ms(&self) -> Option<u64> {
        let seconds = self
            .head
            .as_ref()?
            .header("retry-after")?
            .trim()
            .parse::<f64>()
            .ok()?;
        (seconds >= 0.0).then_some((seconds * 1000.0) as u64)
    }
}

impl fmt::Display for LlmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.status {
            Some(status) => write!(f, "{:?} (HTTP {status}): {}", self.kind, self.message),
            None => write!(f, "{:?}: {}", self.kind, self.message),
        }
    }
}

impl std::error::Error for LlmError {}

pub trait Provider: Send + Sync {
    /// Short stable id written to the trace, for example `anthropic`.
    fn id(&self) -> &str;

    /// Runs one streaming call to completion. `on_delta` sees every increment;
    /// the returned `Completion` holds the fully assembled message.
    fn stream(
        &self,
        http: &dyn Transport,
        auth: &Auth,
        req: &ChatRequest<'_>,
        on_delta: &mut dyn FnMut(Delta<'_>),
    ) -> Result<Completion, LlmError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_retry_rules() {
        let mut err = LlmError::new(ErrorKind::Overloaded, "busy");
        assert!(err.retryable());
        assert_eq!(err.retry_after_ms(), None);
        err.head = Some(HttpHead {
            status: 529,
            headers: vec![("retry-after".into(), "1.5".into())],
        });
        err.status = Some(529);
        assert_eq!(err.retry_after_ms(), Some(1500));
        assert_eq!(err.to_string(), "Overloaded (HTTP 529): busy");
        for kind in [
            ErrorKind::Auth,
            ErrorKind::RateLimit,
            ErrorKind::BadRequest,
            ErrorKind::Aborted,
        ] {
            assert!(!LlmError::new(kind, "x").retryable(), "{kind:?}");
        }
    }

    #[test]
    fn auth_debug_hides_secrets() {
        assert_eq!(
            format!("{:?}", Auth::OAuth("sk-secret".into())),
            "OAuth(<redacted>)"
        );
        let timing = Timing {
            ttfb_ms: 1,
            first_token_ms: 2,
            total_ms: 3,
            chunks: 4,
            bytes: 5,
        };
        assert_eq!(
            serde_json::to_string(&timing).unwrap(),
            r#"{"ttfbMs":1,"firstTokenMs":2,"totalMs":3,"chunks":4,"bytes":5}"#
        );
    }
}
