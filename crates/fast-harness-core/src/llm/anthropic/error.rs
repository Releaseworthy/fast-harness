//! Error mapping: HTTP status, JSON error bodies and SSE `error` events.
//!
//! Error body and event shape `{"type":"error","error":{"type":"...","message":"..."}}`
//! and "`overloaded_error` ... would normally correspond to an HTTP 529":
//! <https://platform.claude.com/docs/en/build-with-claude/streaming> (read 2026-11-11);
//! status list: <https://platform.claude.com/docs/en/api/errors>.

use serde_json::Value;

use crate::http::{HttpError, HttpHead};
use crate::llm::{ErrorKind, LlmError};

/// Longest raw body quoted in a message when it is not a JSON error.
const MAX_QUOTE: usize = 500;

pub(crate) fn kind_for_status(status: u16) -> ErrorKind {
    match status {
        401 | 403 => ErrorKind::Auth,
        429 => ErrorKind::RateLimit,
        529 => ErrorKind::Overloaded,
        500..=599 => ErrorKind::Server,
        400..=499 => ErrorKind::BadRequest,
        // A 1xx or 3xx reaching us is not a usable answer.
        _ => ErrorKind::Protocol,
    }
}

/// `type: message` from an `error` object, when it has a message.
fn describe(error: &Value) -> Option<String> {
    let message = error["message"].as_str()?;
    Some(match error["type"].as_str() {
        Some(kind) => format!("{kind}: {message}"),
        None => message.to_string(),
    })
}

/// A non-2xx response: the kind from the status, the message from the body.
pub(crate) fn from_http(head: HttpHead, body: &[u8]) -> LlmError {
    let parsed: Option<Value> = serde_json::from_slice(body).ok();
    let message = parsed
        .as_ref()
        .and_then(|json| describe(&json["error"]))
        .unwrap_or_else(|| {
            let text = String::from_utf8_lossy(body);
            let text = text.trim();
            if text.is_empty() {
                format!("HTTP {}", head.status)
            } else {
                text.chars().take(MAX_QUOTE).collect()
            }
        });
    let overflow = matches!(head.status, 400 | 413)
        && message.to_ascii_lowercase().contains("prompt is too long");
    LlmError {
        kind: match overflow {
            true => ErrorKind::ContextOverflow,
            false => kind_for_status(head.status),
        },
        status: Some(head.status),
        message,
        head: Some(head),
    }
}

/// An `error` event inside a 200 stream.
pub(crate) fn from_event(data: &Value) -> LlmError {
    let error = &data["error"];
    let kind = match error["type"].as_str() {
        Some("overloaded_error") => ErrorKind::Overloaded,
        Some("rate_limit_error") => ErrorKind::RateLimit,
        Some("api_error") => ErrorKind::Server,
        _ => ErrorKind::Protocol,
    };
    let message = describe(error).unwrap_or_else(|| "error event without a message".into());
    LlmError::new(kind, message)
}

pub(crate) fn from_transport(err: &HttpError) -> LlmError {
    LlmError::new(ErrorKind::Network, err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn statuses_map_to_kinds() {
        let cases = [
            (401, ErrorKind::Auth),
            (403, ErrorKind::Auth),
            (429, ErrorKind::RateLimit),
            (529, ErrorKind::Overloaded),
            (500, ErrorKind::Server),
            (503, ErrorKind::Server),
            (400, ErrorKind::BadRequest),
            (413, ErrorKind::BadRequest),
        ];
        for (status, kind) in cases {
            assert_eq!(kind_for_status(status), kind, "{status}");
        }
    }

    #[test]
    fn bodies_and_events() {
        let head = HttpHead {
            status: 502,
            headers: vec![],
        };
        let err = from_http(head.clone(), b"<html>bad gateway</html>");
        assert_eq!(
            (err.kind, err.message.as_str()),
            (ErrorKind::Server, "<html>bad gateway</html>")
        );
        assert_eq!(from_http(head, b"").message, "HTTP 502");
        let event =
            json!({"type": "error", "error": {"type": "invalid_request_error", "message": "m"}});
        let err = from_event(&event);
        assert_eq!(
            (err.kind, err.message.as_str()),
            (ErrorKind::Protocol, "invalid_request_error: m")
        );
    }
}
