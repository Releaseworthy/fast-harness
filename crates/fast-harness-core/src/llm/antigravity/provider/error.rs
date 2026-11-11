//! Antigravity failures as [`LlmError`]s: HTTP statuses, Google's error body
//! `{"error": {"code", "message", "status", "details"}}` (also inside a 200
//! stream) and transport errors.
//!
//! Google's error model (google.rpc.Status with `ErrorInfo`, `RetryInfo`
//! `retryDelay` and `Help` links): <https://cloud.google.com/apis/design/errors>.
//! Antigravity's own details (`VALIDATION_REQUIRED` with `validation_url`,
//! `quotaResetDelay`) follow the live-verified DSH proxy
//! (`plugins/dsh-antigravity-switch/host.js` `errorDetails`).

use serde_json::Value;

use crate::http::{HttpError, HttpHead};
use crate::llm::{ErrorKind, LlmError};

/// Longest raw body quoted in a message when it is not a Google error.
const MAX_QUOTE: usize = 500;

/// The words `accounts::google::is_validation` looks for (`llm` sits below`n/// `accounts` and cannot call it; a test checks the two agree).
const VALIDATION_WORDS: &str = "validation required";

fn is_validation(message: &str) -> bool {
    message.to_lowercase().contains(VALIDATION_WORDS)
}

/// `ErrorInfo.reason` of Google's account verification request.
const VALIDATION_REASON: &str = "VALIDATION_REQUIRED";

/// The kind of an HTTP `status` (0 inside a stream) and Google `status` name.
pub(crate) fn kind_for(status: u16, google: &str) -> ErrorKind {
    if status == 429 || google == "RESOURCE_EXHAUSTED" {
        return ErrorKind::RateLimit;
    }
    match (status, google) {
        (401 | 403, _) => ErrorKind::Auth,
        (503 | 529, _) => ErrorKind::Overloaded,
        (408 | 500..=599, _) => ErrorKind::Server,
        (400..=499, _) => ErrorKind::BadRequest,
        (_, "UNAUTHENTICATED" | "PERMISSION_DENIED") => ErrorKind::Auth,
        (_, "UNAVAILABLE") => ErrorKind::Overloaded,
        (_, "INVALID_ARGUMENT" | "NOT_FOUND" | "FAILED_PRECONDITION") => ErrorKind::BadRequest,
        (_, "INTERNAL" | "UNKNOWN" | "DEADLINE_EXCEEDED") => ErrorKind::Server,
        // A 1xx or 3xx, or an unnamed error event, is not a usable answer.
        _ => ErrorKind::Protocol,
    }
}

/// The first object of an array body, else the body itself.
pub(crate) fn first_object(value: &Value) -> &Value {
    match value {
        Value::Array(items) => items.iter().find(|item| item.is_object()).unwrap_or(value),
        _ => value,
    }
}

/// A non-2xx response: the kind from the status, the message from the body.
pub(crate) fn from_http(head: HttpHead, body: &[u8]) -> LlmError {
    let parsed: Option<Value> = serde_json::from_slice(body).ok();
    let error = parsed
        .as_ref()
        .map(|json| &first_object(json)["error"])
        .filter(|error| error.is_object());
    if let Some(error) = error {
        return from_google(head.status, error, Some(head));
    }
    let text = String::from_utf8_lossy(body);
    let text = text.trim();
    let message = if text.is_empty() {
        format!("HTTP {}", head.status)
    } else {
        text.chars().take(MAX_QUOTE).collect()
    };
    LlmError {
        kind: kind_for(head.status, ""),
        status: Some(head.status),
        message,
        head: Some(head),
    }
}

/// An `error` object inside a 200 stream; its `code` stands for the status.
pub(crate) fn from_event(error: &Value) -> LlmError {
    let code = error["code"].as_u64().and_then(|c| u16::try_from(c).ok());
    from_google(code.unwrap_or(0), error, None)
}

fn from_google(status: u16, error: &Value, head: Option<HttpHead>) -> LlmError {
    let google = error["status"].as_str().unwrap_or("");
    let kind = kind_for(status, google);
    let mut message = describe(error, status);
    if status == 403 || google == "PERMISSION_DENIED" {
        if let Some(text) = validation(error, &message) {
            message = text;
        }
    }
    let mut head = head;
    if kind == ErrorKind::RateLimit {
        if let Some(seconds) = retry_delay_secs(error) {
            let head = head.get_or_insert_with(|| HttpHead {
                status,
                headers: Vec::new(),
            });
            head.headers
                .insert(0, ("retry-after".into(), format!("{seconds}")));
        }
    }
    LlmError {
        kind,
        status: (status != 0).then_some(status),
        message,
        head,
    }
}

fn details(error: &Value) -> &[Value] {
    error["details"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn type_is(detail: &Value, name: &str) -> bool {
    detail["@type"].as_str().is_some_and(|t| t.contains(name))
}

/// `STATUS (REASON): message`, with the parts Google gave.
fn describe(error: &Value, status: u16) -> String {
    let message = error["message"].as_str().unwrap_or("").trim();
    let reason = details(error)
        .iter()
        .find(|d| type_is(d, "google.rpc.ErrorInfo"))
        .and_then(|d| d["reason"].as_str());
    let mut label = error["status"].as_str().unwrap_or("").to_string();
    if let Some(reason) = reason {
        label = format!("{label} ({reason})").trim().to_string();
    }
    match (label.is_empty(), message.is_empty()) {
        (true, true) => format!("HTTP {status}"),
        (true, false) => message.to_string(),
        (false, true) => label,
        (false, false) => format!("{label}: {message}"),
    }
}

/// The verification URL Google names in the details.
fn detail_url(error: &Value) -> Option<String> {
    let keys = ["validation_url", "validationUrl", "appeal_url", "appealUrl"];
    let details = details(error);
    let from_info = details.iter().find_map(|d| {
        let meta = &d["metadata"];
        keys.iter().find_map(|key| meta[*key].as_str())
    });
    let from_help = || {
        details
            .iter()
            .filter(|d| type_is(d, "google.rpc.Help"))
            .filter_map(|d| d["links"].as_array())
            .flatten()
            .find_map(|link| link["url"].as_str())
    };
    from_info
        .or_else(from_help)
        .filter(|url| url.starts_with("http"))
        .map(str::to_string)
}

/// A message that `accounts::google::{is_validation, validation_url}` read, when Google
/// asks for account verification.
fn validation(error: &Value, message: &str) -> Option<String> {
    let by_reason = details(error)
        .iter()
        .any(|d| d["reason"].as_str() == Some(VALIDATION_REASON));
    let url = detail_url(error);
    if !by_reason && !is_validation(message) {
        return None;
    }
    match url {
        Some(url) => Some(format!("{VALIDATION_WORDS}: {url} ; {message}")),
        None if is_validation(message) => None,
        None => Some(format!("{VALIDATION_WORDS}; {message}")),
    }
}

/// `RetryInfo.retryDelay`, else `ErrorInfo.metadata.quotaResetDelay`, in seconds.
fn retry_delay_secs(error: &Value) -> Option<f64> {
    let details = details(error);
    let retry = details
        .iter()
        .filter(|d| type_is(d, "google.rpc.RetryInfo"))
        .find_map(|d| d["retryDelay"].as_str().and_then(parse_duration_secs));
    retry.or_else(|| {
        details
            .iter()
            .find_map(|d| d["metadata"]["quotaResetDelay"].as_str())
            .and_then(parse_duration_secs)
    })
}

/// `12.5s`, `500ms` or `1h2m3.5s` in seconds; `None` for anything else.
pub fn parse_duration_secs(text: &str) -> Option<f64> {
    let mut rest = text.trim();
    let mut total = 0.0;
    if rest.is_empty() {
        return None;
    }
    while !rest.is_empty() {
        let end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        let number: f64 = rest[..end].parse().ok()?;
        rest = &rest[end..];
        let (scale, unit) = if rest.starts_with("ms") {
            (0.001, 2)
        } else {
            match rest.chars().next() {
                Some('h') => (3600.0, 1),
                Some('m') => (60.0, 1),
                Some('s') => (1.0, 1),
                _ => return None,
            }
        };
        total += number * scale;
        rest = &rest[unit..];
    }
    Some(total)
}

/// `err` with `head`'s status and headers after its own (the synthetic
/// `retry-after` of an error event stays first).
pub(crate) fn with_head(mut err: LlmError, head: &HttpHead) -> LlmError {
    let mut merged = head.clone();
    if let Some(own) = err.head.take() {
        merged.headers.splice(0..0, own.headers);
    }
    err.head = Some(merged);
    err
}

pub(crate) fn from_transport(err: &HttpError) -> LlmError {
    LlmError::new(ErrorKind::Network, err.to_string())
}

/// `err` without `secret` in its message.
pub(crate) fn redact(mut err: LlmError, secret: &str) -> LlmError {
    if !secret.is_empty() && err.message.contains(secret) {
        err.message = err.message.replace(secret, "<redacted>");
    }
    err
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_and_kinds() {
        assert_eq!(parse_duration_secs("12.5s"), Some(12.5));
        assert_eq!(parse_duration_secs("500ms"), Some(0.5));
        assert_eq!(parse_duration_secs("1h2m3s"), Some(3723.0));
        assert_eq!(parse_duration_secs("12"), None);
        assert_eq!(parse_duration_secs(""), None);
        assert_eq!(kind_for(0, "RESOURCE_EXHAUSTED"), ErrorKind::RateLimit);
        assert_eq!(kind_for(404, ""), ErrorKind::BadRequest);
        assert_eq!(kind_for(502, ""), ErrorKind::Server);
        assert_eq!(kind_for(0, "UNAVAILABLE"), ErrorKind::Overloaded);
        assert_eq!(kind_for(0, ""), ErrorKind::Protocol);
    }
}
