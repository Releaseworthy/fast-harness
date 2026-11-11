//! Shared fixtures of the Antigravity provider tests: a Hub on fake hosts,
//! SSE bodies of wrapped Google answers split into tiny chunks, one call.
#![allow(dead_code)]

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use fast_harness_core::http::{RecordedRequest, ScriptedResponse, ScriptedTransport};
use fast_harness_core::llm::antigravity::hub::Hub;
use fast_harness_core::llm::antigravity::Antigravity;
use fast_harness_core::llm::{Auth, ChatRequest, Completion, LlmError, Provider};
use fast_harness_core::types::Message;
use serde_json::{json, Value};

pub const MANIFEST: &str = "https://manifest.example/latest-arm64-mac.yml";
pub const DAILY: &str = "https://daily.example";
pub const SANDBOX: &str = "https://sandbox.example";
pub const PATH: &str = "/v1internal:streamGenerateContent?alt=sse";
/// A fake token: no real credential ever appears in tests.
pub const TOKEN: &str = "fake-access-token-7f3a";
pub const MODEL: &str = "antigravity/gemini-test";

pub fn provider() -> Antigravity {
    Antigravity::new(Arc::new(Hub::new(MANIFEST, &[DAILY, SANDBOX])))
}

pub fn google() -> Auth {
    Auth::Google {
        access: TOKEN.into(),
        project: "fake-project".into(),
    }
}

/// The Hub manifest every fresh provider reads first for its User-Agent.
pub fn manifest() -> ScriptedResponse {
    ScriptedResponse::new(200, "version: 2.30.0\npath: x.zip\n", 4)
}

/// `{"response": answer, "traceId": ...}` as Antigravity wraps each event.
pub fn wrap(answer: Value) -> Value {
    json!({"response": answer, "traceId": "trace-1"})
}

/// One wrapped event with `parts` and an optional finish reason.
pub fn event(parts: Value, finish: Option<&str>) -> Value {
    let mut candidate = json!({"content": {"role": "model", "parts": parts}});
    if let Some(reason) = finish {
        candidate["finishReason"] = json!(reason);
    }
    wrap(json!({"candidates": [candidate]}))
}

/// An SSE body of `events`, delivered 3 bytes at a time.
pub fn sse(events: &[Value]) -> ScriptedResponse {
    let body: String = events
        .iter()
        .map(|event| format!("data: {event}\r\n\r\n"))
        .collect();
    ScriptedResponse::new(200, &body, 3)
}

/// A Google error body with `status`.
pub fn google_error(status: u16, error: Value) -> ScriptedResponse {
    ScriptedResponse::new(status, &json!({ "error": error }).to_string(), 5)
}

pub struct Outcome {
    pub result: Result<Completion, LlmError>,
    pub deltas: Vec<String>,
    pub requests: Vec<RecordedRequest>,
}

impl Outcome {
    pub fn ok(&self) -> &Completion {
        match &self.result {
            Ok(done) => done,
            Err(err) => panic!("call failed: {err}"),
        }
    }

    /// The error, checked to never carry the token.
    pub fn err(&self) -> &LlmError {
        let err = self.result.as_ref().unwrap_err();
        assert!(!format!("{err} {err:?}").contains(TOKEN), "token leaked");
        err
    }
}

/// One call of `MODEL` with `auth`; `cancel_on_text` sets the cancel flag
/// on the first text delta.
pub fn call_with(responses: Vec<ScriptedResponse>, auth: &Auth, cancel_on_text: bool) -> Outcome {
    let http = ScriptedTransport::new(responses);
    let cancel = AtomicBool::new(false);
    let messages = [Message::user("hi")];
    let req = ChatRequest {
        model: MODEL,
        system: "",
        messages: &messages,
        tools: &[],
        max_tokens: 1024,
        effort: None,
        cache_long: false,
        cancel: &cancel,
    };
    let mut deltas = Vec::new();
    let result = provider().stream(&http, auth, &req, &mut |delta| {
        let text = format!("{delta:?}");
        if cancel_on_text && text.starts_with("Text") {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        deltas.push(text);
    });
    Outcome {
        result,
        deltas,
        requests: http.requests(),
    }
}

pub fn call(responses: Vec<ScriptedResponse>) -> Outcome {
    call_with(responses, &google(), false)
}

/// `manifest` then `answer`: the usual single-host call.
pub fn answer(events: &[Value]) -> Outcome {
    call(vec![manifest(), sse(events)])
}
