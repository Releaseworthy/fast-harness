//! Error mapping of OpenAI-compatible servers: the status gives the kind,
//! the body `{"error": {"message", "type", "code"}}` the message; a 400 or
//! 413 about the context length is [`ErrorKind::ContextOverflow`].

use serde_json::Value;

use crate::http::{HttpError, HttpErrorKind, HttpHead};
use crate::llm::{ErrorKind, LlmError};

const MAX_QUOTE: usize = 500;

/// Phrases servers use for a prompt over the window (OpenAI
/// `context_length_exceeded`, "maximum context length", llama.cpp
/// "exceeds the available context size", Anthropic-style wording).
const OVERFLOW: [&str; 6] = [
    "context_length_exceeded",
    "maximum context length",
    "context length",
    "context size",
    "prompt is too long",
    "too many tokens",
];

fn kind_for_status(status: u16) -> ErrorKind {
    match status {
        401 | 403 => ErrorKind::Auth,
        429 => ErrorKind::RateLimit,
        503 | 529 => ErrorKind::Overloaded,
        500..=599 => ErrorKind::Server,
        400..=499 => ErrorKind::BadRequest,
        _ => ErrorKind::Protocol,
    }
}

pub(crate) fn protocol(message: String) -> LlmError {
    LlmError::new(ErrorKind::Protocol, message)
}

fn describe(error: &Value) -> Option<String> {
    let message = error["message"].as_str()?;
    let kind = error["code"].as_str().or(error["type"].as_str());
    Some(match kind {
        Some(kind) => format!("{kind}: {message}"),
        None => message.to_string(),
    })
}

fn overflow(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    OVERFLOW.iter().any(|phrase| lower.contains(phrase))
}

/// A non-2xx response.
pub(crate) fn from_http(head: HttpHead, body: &[u8]) -> LlmError {
    let parsed: Option<Value> = serde_json::from_slice(body).ok();
    let message = parsed
        .as_ref()
        .and_then(|json| describe(&json["error"]))
        .unwrap_or_else(|| {
            let text = String::from_utf8_lossy(body);
            match text.trim() {
                "" => format!("HTTP {}", head.status),
                text => text.chars().take(MAX_QUOTE).collect(),
            }
        });
    let kind = match matches!(head.status, 400 | 413) && overflow(&message) {
        true => ErrorKind::ContextOverflow,
        false => kind_for_status(head.status),
    };
    LlmError {
        kind,
        status: Some(head.status),
        message,
        head: Some(head),
    }
}

/// An `error` object inside a 200 stream.
pub(crate) fn from_event(chunk: &Value) -> LlmError {
    let message = describe(&chunk["error"]).unwrap_or_else(|| chunk["error"].to_string());
    let status = chunk["error"]["code"]
        .as_u64()
        .and_then(|c| u16::try_from(c).ok());
    let kind = match (overflow(&message), status) {
        (true, _) => ErrorKind::ContextOverflow,
        (false, Some(status)) => kind_for_status(status),
        (false, None) => ErrorKind::Server,
    };
    LlmError::new(kind, message)
}

pub(crate) fn from_transport(err: &HttpError) -> LlmError {
    let kind = match err.kind {
        HttpErrorKind::InvalidRequest => ErrorKind::BadRequest,
        _ => ErrorKind::Network,
    };
    LlmError::new(kind, err.to_string())
}
