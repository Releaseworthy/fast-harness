//! The real HTTP transport: `ureq` with `rustls` and the OS certificate store (chunk 1A).
//! A request with a cancel flag runs on its own thread so the cancel ends
//! it in any state (`call`, `watch`, PLAN.md D66).

mod call;
mod watch;

use std::io::{self, Read};
use std::time::Duration;

use ureq::config::AutoHeaderValue;
use ureq::http;
use ureq::tls::{RootCerts, TlsConfig};
use ureq::unversioned::resolver::DefaultResolver;
use ureq::Agent;

pub use call::{live_calls, POLL_MS};

use crate::http::{Flow, HttpError, HttpErrorKind, HttpHead, HttpRequest, Transport};

/// Connect limit when `HttpRequest::timeout_ms` is 0.
pub const DEFAULT_CONNECT_TIMEOUT_MS: u32 = 30_000;
/// Limit for each wait on response bytes (headers or one body read) when
/// `timeout_ms` is 0. Long: a thinking model can be silent for minutes,
/// though Anthropic sends `ping` events in between. A cancel does not wait
/// for it (D66).
pub const DEFAULT_READ_TIMEOUT_MS: u32 = 600_000;
/// Size of one body read handed to `on_chunk`.
pub const CHUNK_BYTES: usize = 16 * 1024;

/// `ureq` 3 transport. One `Agent` per value, so connections are pooled and
/// reused across calls; share one `UreqTransport` across threads.
pub struct UreqTransport {
    agent: Agent,
}

impl Default for UreqTransport {
    fn default() -> UreqTransport {
        UreqTransport::new()
    }
}

impl UreqTransport {
    pub fn new() -> UreqTransport {
        let tls = TlsConfig::builder()
            .root_certs(RootCerts::PlatformVerifier)
            .build();
        let config = Agent::config_builder()
            .http_status_as_error(false)
            .tls_config(tls)
            // Callers send their own user-agent; ureq adds none of its own.
            .user_agent(AutoHeaderValue::None)
            .timeout_connect(Some(millis(DEFAULT_CONNECT_TIMEOUT_MS)))
            .timeout_per_read(Some(millis(DEFAULT_READ_TIMEOUT_MS)))
            .build();
        // ureq's default connector and resolver; the connector only keeps
        // each socket's shutdown handle for a cancel (D66).
        let connector = watch::WatchedConnector::default();
        UreqTransport {
            agent: Agent::with_parts(config, connector, DefaultResolver::default()),
        }
    }
}

fn run<S: ureq::AsSendBody>(
    agent: &Agent,
    request: http::Request<S>,
    timeout_ms: u32,
) -> Result<http::Response<ureq::Body>, HttpError> {
    let request = if timeout_ms == 0 {
        request
    } else {
        agent
            .configure_request(request)
            .timeout_connect(Some(millis(timeout_ms)))
            .timeout_recv_response(Some(millis(timeout_ms)))
            .timeout_per_read(Some(millis(timeout_ms)))
            .build()
    };
    agent.run(request).map_err(map_ureq_error)
}

impl Transport for UreqTransport {
    fn send(
        &self,
        req: &HttpRequest<'_>,
        on_chunk: &mut dyn FnMut(&HttpHead, &[u8]) -> Flow,
    ) -> Result<HttpHead, HttpError> {
        match req.cancel {
            Some(cancel) => call::send(&self.agent, req, cancel, on_chunk),
            None => send_inline(&self.agent, req, on_chunk),
        }
    }
}

/// The request on the calling thread; `req.cancel` is not read.
fn send_inline(
    agent: &Agent,
    req: &HttpRequest<'_>,
    on_chunk: &mut dyn FnMut(&HttpHead, &[u8]) -> Flow,
) -> Result<HttpHead, HttpError> {
    let mut builder = http::Request::builder().method(req.method).uri(req.url);
    for (name, value) in req.headers {
        builder = builder.header(*name, *value);
    }
    let response = if req.body.is_empty() {
        run(agent, builder.body(()).map_err(invalid)?, req.timeout_ms)?
    } else {
        run(
            agent,
            builder.body(req.body).map_err(invalid)?,
            req.timeout_ms,
        )?
    };
    let (parts, body) = response.into_parts();
    let head = HttpHead {
        status: parts.status.as_u16(),
        headers: parts
            .headers
            .iter()
            .map(|(name, value)| {
                let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
                (name.as_str().to_ascii_lowercase(), value)
            })
            .collect(),
    };
    read_body(body.into_reader(), &head, on_chunk)?;
    Ok(head)
}

fn read_body(
    mut reader: impl Read,
    head: &HttpHead,
    on_chunk: &mut dyn FnMut(&HttpHead, &[u8]) -> Flow,
) -> Result<(), HttpError> {
    let mut buf = vec![0u8; CHUNK_BYTES];
    loop {
        let read = match reader.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(read) => read,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(map_io_error(&err)),
        };
        if on_chunk(head, &buf[..read]) == Flow::Stop {
            return Ok(());
        }
    }
}

fn millis(ms: u32) -> Duration {
    Duration::from_millis(u64::from(ms))
}

fn invalid(err: http::Error) -> HttpError {
    HttpError {
        kind: HttpErrorKind::InvalidRequest,
        message: err.to_string(),
    }
}

fn map_io_error(err: &io::Error) -> HttpError {
    let kind = match err.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => HttpErrorKind::Timeout,
        io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound => HttpErrorKind::Connect,
        _ => HttpErrorKind::Io,
    };
    HttpError {
        kind,
        message: err.to_string(),
    }
}

fn map_ureq_error(err: ureq::Error) -> HttpError {
    use ureq::Error as E;
    let kind = match &err {
        E::Timeout(_) => HttpErrorKind::Timeout,
        E::Io(io_err) => return map_io_error(io_err),
        E::HostNotFound | E::ConnectionFailed | E::Tls(_) | E::Rustls(_) | E::Pem(_) => {
            HttpErrorKind::Connect
        }
        E::Http(_) | E::BadUri(_) | E::InvalidProxyUrl => HttpErrorKind::InvalidRequest,
        _ => HttpErrorKind::Io,
    };
    HttpError {
        kind,
        message: err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_errors_map_to_kinds() {
        let timeout = io::Error::new(io::ErrorKind::TimedOut, "slow");
        assert_eq!(map_io_error(&timeout).kind, HttpErrorKind::Timeout);
        let reset = io::Error::new(io::ErrorKind::ConnectionReset, "reset");
        assert_eq!(map_io_error(&reset).kind, HttpErrorKind::Io);
        assert_eq!(
            map_ureq_error(ureq::Error::HostNotFound).kind,
            HttpErrorKind::Connect
        );
    }

    #[test]
    fn bad_url_is_invalid_request() {
        let transport = UreqTransport::new();
        let req = HttpRequest {
            method: "GET",
            url: "not a url",
            headers: &[],
            body: b"",
            timeout_ms: 1000,
            cancel: None,
        };
        let err = transport
            .send(&req, &mut |_, _| Flow::Continue)
            .unwrap_err();
        assert_eq!(err.kind, HttpErrorKind::InvalidRequest);
    }

    #[test]
    fn body_is_read_in_chunks_until_stop() {
        let data = vec![7u8; CHUNK_BYTES * 2 + 5];
        let head = HttpHead::default();
        let mut sizes = Vec::new();
        read_body(&data[..], &head, &mut |_, chunk| {
            sizes.push(chunk.len());
            Flow::Continue
        })
        .unwrap();
        assert_eq!(sizes, vec![CHUNK_BYTES, CHUNK_BYTES, 5]);
        let mut count = 0;
        read_body(&data[..], &head, &mut |_, _| {
            count += 1;
            Flow::Stop
        })
        .unwrap();
        assert_eq!(count, 1);
    }

    /// Run by hand: `cargo test -p fast-harness-core net::tests::live -- --ignored`.
    #[test]
    #[ignore]
    fn live_anthropic_models_without_credentials_is_401() {
        let transport = UreqTransport::new();
        let req = HttpRequest {
            method: "GET",
            url: "https://api.anthropic.com/v1/models",
            headers: &[("anthropic-version", "2023-06-01")],
            body: b"",
            timeout_ms: 0,
            cancel: None,
        };
        let mut body = Vec::new();
        let head = transport
            .send(&req, &mut |_, chunk| {
                body.extend_from_slice(chunk);
                Flow::Continue
            })
            .unwrap();
        assert_eq!(head.status, 401);
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["type"], "error");
        assert!(head.headers.iter().all(|(k, _)| *k == k.to_lowercase()));
    }
}
