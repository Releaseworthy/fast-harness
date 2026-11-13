//! The HTTP boundary. The core speaks only to [`Transport`]; each OS supplies
//! one implementation under `platform` (WinHTTP on Windows).

use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[non_exhaustive]
pub struct HttpRequest<'a> {
    /// `GET` or `POST`.
    pub method: &'a str,
    /// Absolute `http://` or `https://` URL.
    pub url: &'a str,
    pub headers: &'a [(&'a str, &'a str)],
    pub body: &'a [u8],
    /// Limit for connect and for each wait on response data; 0 means the transport default.
    pub timeout_ms: u32,
    /// Set from another thread to end the call in any state (connecting,
    /// waiting for the head or a body byte): send then returns
    /// HttpErrorKind::Cancelled within [CANCEL_BOUND_MS] (PLAN.md D66).
    /// A transport that ignores it is cancelled only between chunks.
    pub cancel: Option<&'a AtomicBool>,
}

impl<'a> HttpRequest<'a> {
    /// A request with the transport's default timeout.
    pub fn new(
        method: &'a str,
        url: &'a str,
        headers: &'a [(&'a str, &'a str)],
        body: &'a [u8],
    ) -> HttpRequest<'a> {
        HttpRequest {
            method,
            url,
            headers,
            body,
            timeout_ms: 0,
            cancel: None,
        }
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u32) -> HttpRequest<'a> {
        self.timeout_ms = timeout_ms;
        self
    }

    /// A request that cancel ends in any state (PLAN.md D66).
    pub fn with_cancel(mut self, cancel: &'a AtomicBool) -> HttpRequest<'a> {
        self.cancel = Some(cancel);
        self
    }

    /// Whether the caller asked to cancel this call.
    pub fn cancelled(&self) -> bool {
        self.cancel.is_some_and(|flag| flag.load(Ordering::Relaxed))
    }
}

/// How long a cancelled call may still block in a transport that honours
/// [HttpRequest::cancel]: the bound the tests hold UreqTransport and
/// ScriptedTransport to.
pub const CANCEL_BOUND_MS: u64 = 500;

/// Status and response headers. Header names are lowercase.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct HttpHead {
    pub status: u16,
    pub headers: Vec<(String, String)>,
}

impl HttpHead {
    pub fn new(status: u16, headers: Vec<(String, String)>) -> HttpHead {
        HttpHead { status, headers }
    }

    /// First header with this lowercase name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HttpErrorKind {
    /// Name resolution, connect or TLS failure: nothing was sent.
    Connect,
    Timeout,
    /// The connection broke after the request was sent.
    Io,
    InvalidRequest,
    /// [HttpRequest::cancel] was set: the call ended early.
    Cancelled,
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct HttpError {
    pub kind: HttpErrorKind,
    pub message: String,
}

impl HttpError {
    pub fn new(kind: HttpErrorKind, message: impl Into<String>) -> HttpError {
        HttpError {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for HttpError {}

/// Returned by the body callback to keep reading or end the transfer early.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Flow {
    Continue,
    Stop,
}

pub trait Transport: Send + Sync {
    /// Sends `req` and streams the response body through `on_chunk` as it
    /// arrives, on the calling thread. Any status is a success at this layer:
    /// the caller reads `head.status`. `Flow::Stop` ends the transfer and
    /// `send` still returns `Ok(head)`. Connections are reused across calls.
    fn send(
        &self,
        req: &HttpRequest<'_>,
        on_chunk: &mut dyn FnMut(&HttpHead, &[u8]) -> Flow,
    ) -> Result<HttpHead, HttpError>;
}

/// One canned response for [`ScriptedTransport`].
#[derive(Clone, Debug, Default)]
pub struct ScriptedResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub chunks: Vec<Vec<u8>>,
}

impl ScriptedResponse {
    /// A response whose body arrives in `chunk_size` byte pieces, which
    /// exercises parsers on arbitrary chunk boundaries (even inside UTF-8).
    pub fn new(status: u16, body: &str, chunk_size: usize) -> ScriptedResponse {
        let chunks = body
            .as_bytes()
            .chunks(chunk_size.max(1))
            .map(<[u8]>::to_vec)
            .collect();
        ScriptedResponse {
            status,
            headers: Vec::new(),
            chunks,
        }
    }

    pub fn with_header(mut self, name: &str, value: &str) -> ScriptedResponse {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RecordedRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// In-memory transport for tests: replays responses in order and records requests.
#[derive(Default)]
pub struct ScriptedTransport {
    responses: Mutex<VecDeque<ScriptedResponse>>,
    requests: Mutex<Vec<RecordedRequest>>,
    /// (call index, chunks sent before the silence).
    silences: Mutex<Vec<(usize, usize)>>,
}

/// How long a scripted silence lasts when nobody cancels: then the call
/// fails with `Timeout`, as a real read limit would.
pub const SCRIPTED_SILENCE_MS: u64 = 10_000;

impl ScriptedTransport {
    pub fn new(responses: Vec<ScriptedResponse>) -> ScriptedTransport {
        ScriptedTransport {
            responses: Mutex::new(responses.into()),
            requests: Mutex::default(),
            silences: Mutex::default(),
        }
    }

    /// Call `call` (0-based) goes silent after `chunks` body chunks (0:
    /// before the first byte) until its [`HttpRequest::cancel`] is set, or
    /// for [`SCRIPTED_SILENCE_MS`], like a stream that stops sending.
    pub fn with_silence(self, call: usize, chunks: usize) -> ScriptedTransport {
        if let Ok(mut list) = self.silences.lock() {
            list.push((call, chunks));
        }
        self
    }

    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests
            .lock()
            .map(|list| list.clone())
            .unwrap_or_default()
    }

    fn silence_of(&self, call: usize) -> Option<usize> {
        let list = self.silences.lock().ok()?;
        list.iter().find(|(at, _)| *at == call).map(|(_, n)| *n)
    }

    /// Records `req`; returns its 0-based call index.
    fn record(&self, req: &HttpRequest<'_>) -> usize {
        let record = RecordedRequest {
            method: req.method.to_string(),
            url: req.url.to_string(),
            headers: req
                .headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: String::from_utf8_lossy(req.body).into_owned(),
        };
        match self.requests.lock() {
            Ok(mut list) => {
                list.push(record);
                list.len() - 1
            }
            Err(_) => 0,
        }
    }
}

/// A silent stream: nothing arrives until the call is cancelled or the
/// scripted limit passes. Rechecks the flag every 5 ms.
fn wait_silent(req: &HttpRequest<'_>) -> HttpError {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(SCRIPTED_SILENCE_MS) {
        if req.cancelled() {
            return HttpError::new(HttpErrorKind::Cancelled, "cancelled");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    HttpError::new(HttpErrorKind::Timeout, "scripted silence")
}

impl Transport for ScriptedTransport {
    fn send(
        &self,
        req: &HttpRequest<'_>,
        on_chunk: &mut dyn FnMut(&HttpHead, &[u8]) -> Flow,
    ) -> Result<HttpHead, HttpError> {
        let call = self.record(req);
        let silence = self.silence_of(call);
        let next = self
            .responses
            .lock()
            .ok()
            .and_then(|mut queue| queue.pop_front());
        let Some(response) = next else {
            let message = "no scripted response left".to_string();
            return Err(HttpError {
                kind: HttpErrorKind::Connect,
                message,
            });
        };
        let head = HttpHead {
            status: response.status,
            headers: response.headers,
        };
        for (index, chunk) in response.chunks.iter().enumerate() {
            if silence == Some(index) {
                return Err(wait_silent(req));
            }
            if on_chunk(&head, chunk) == Flow::Stop {
                return Ok(head);
            }
        }
        match silence {
            Some(at) if at >= response.chunks.len() => Err(wait_silent(req)),
            _ => Ok(head),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripted_transport_replays_and_records() {
        let response = ScriptedResponse::new(200, "abcdef", 4).with_header("x-id", "7");
        let transport = ScriptedTransport::new(vec![response]);
        let req = HttpRequest {
            method: "POST",
            url: "http://x/y",
            headers: &[("a", "b")],
            body: b"hi",
            timeout_ms: 0,
            cancel: None,
        };
        let mut seen = Vec::new();
        let head = transport
            .send(&req, &mut |head, chunk| {
                assert_eq!(head.status, 200);
                seen.push(chunk.to_vec());
                Flow::Continue
            })
            .unwrap();
        assert_eq!(seen, vec![b"abcd".to_vec(), b"ef".to_vec()]);
        assert_eq!(head.header("x-id"), Some("7"));
        assert_eq!(transport.requests()[0].body, "hi");
        let again = transport.send(&req, &mut |_, _| Flow::Continue);
        assert_eq!(again.unwrap_err().kind, HttpErrorKind::Connect);
    }

    #[test]
    fn stop_ends_the_transfer_early() {
        let transport = ScriptedTransport::new(vec![ScriptedResponse::new(200, "abcdef", 2)]);
        let req = HttpRequest {
            method: "GET",
            url: "http://x",
            headers: &[],
            body: b"",
            timeout_ms: 0,
            cancel: None,
        };
        let mut count = 0;
        let head = transport.send(&req, &mut |_, _| {
            count += 1;
            Flow::Stop
        });
        assert_eq!((head.unwrap().status, count), (200, 1));
    }
}
