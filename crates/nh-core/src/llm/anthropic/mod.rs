//! Claude over the Anthropic Messages API: API key or subscription OAuth (chunk 1A).
//!
//! One call is `POST {base}/v1/messages?beta=true` with `stream: true` (the
//! path pi-ai's SDK uses for `beta.messages.create`). The response stream is
//! decoded by [`SseDecoder`] and assembled by `stream::Assembler`.

mod error;
mod models;
mod request;
mod stream;
mod thinking;
mod wire;

use std::sync::atomic::Ordering;
use std::time::Instant;

pub use models::{builtin_models, ModelInfo};

use crate::http::{Flow, HttpHead, HttpRequest, Transport};
use crate::llm::{Auth, ChatRequest, Completion, Delta, ErrorKind, LlmError, Provider, Timing};
use crate::sse::SseDecoder;
use crate::types::StopReason;

pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

/// Longest error body kept from a failed call.
const MAX_ERROR_BODY: usize = 64 * 1024;

#[derive(Clone, Debug)]
pub struct Anthropic {
    base_url: String,
}

impl Default for Anthropic {
    fn default() -> Anthropic {
        Anthropic::new(DEFAULT_BASE_URL)
    }
}

impl Anthropic {
    /// `base_url` without the `/v1` part, for example `https://api.anthropic.com`.
    pub fn new(base_url: &str) -> Anthropic {
        Anthropic {
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// The models this credential can use, merged with the built-in table.
    pub fn list_models(
        &self,
        http: &dyn Transport,
        auth: &Auth,
    ) -> Result<Vec<ModelInfo>, LlmError> {
        models::list(http, &self.base_url, auth)
    }
}

/// `GET https://api.anthropic.com/v1/models` merged with the built-in table.
pub fn list_models(http: &dyn Transport, auth: &Auth) -> Result<Vec<ModelInfo>, LlmError> {
    Anthropic::default().list_models(http, auth)
}

fn elapsed_ms(start: Instant) -> u64 {
    start.elapsed().as_millis() as u64
}

/// State of one call while body chunks arrive.
struct Run<'a, 'd> {
    req: &'a ChatRequest<'a>,
    on_delta: &'d mut dyn FnMut(Delta<'_>),
    start: Instant,
    timing: Timing,
    decoder: SseDecoder,
    assembler: stream::Assembler<'a>,
    /// Body of a non-2xx response.
    error_body: Vec<u8>,
    failure: Option<LlmError>,
    aborted: bool,
    got_token: bool,
}

impl Run<'_, '_> {
    fn on_chunk(&mut self, head: &HttpHead, chunk: &[u8]) -> Flow {
        if self.timing.chunks == 0 {
            self.timing.ttfb_ms = elapsed_ms(self.start);
        }
        self.timing.chunks += 1;
        self.timing.bytes += chunk.len() as u64;
        if self.req.cancel.load(Ordering::Relaxed) {
            self.aborted = true;
            return Flow::Stop;
        }
        if !(200..300).contains(&head.status) {
            self.error_body.extend_from_slice(chunk);
            return if self.error_body.len() > MAX_ERROR_BODY {
                Flow::Stop
            } else {
                Flow::Continue
            };
        }
        let mut decoder = std::mem::take(&mut self.decoder);
        decoder.feed(chunk, &mut |name, data| self.on_event(name, data));
        self.decoder = decoder;
        if self.failure.is_some() || self.assembler.done || self.aborted {
            Flow::Stop
        } else {
            Flow::Continue
        }
    }

    fn on_event(&mut self, name: &str, data: &str) {
        if self.failure.is_some() || self.assembler.done || self.aborted {
            return;
        }
        if self.req.cancel.load(Ordering::Relaxed) {
            self.aborted = true;
            return;
        }
        match self.assembler.handle(name, data, &mut *self.on_delta) {
            Ok(true) if !self.got_token => {
                self.got_token = true;
                self.timing.first_token_ms = elapsed_ms(self.start);
            }
            Ok(_) => {}
            Err(err) => self.failure = Some(err),
        }
    }

    /// Turns the finished transfer into the call's result.
    fn finish(mut self, head: HttpHead) -> Result<Completion, LlmError> {
        let attach = |mut err: LlmError, head: &HttpHead| {
            err.head = Some(head.clone());
            err
        };
        if self.aborted {
            return Err(attach(
                LlmError::new(ErrorKind::Aborted, "cancelled"),
                &head,
            ));
        }
        if !(200..300).contains(&head.status) {
            return Err(error::from_http(head, &self.error_body));
        }
        let mut decoder = std::mem::take(&mut self.decoder);
        decoder.finish(&mut |name, data| self.on_event(name, data));
        if let Some(err) = self.failure.take() {
            return Err(attach(err, &head));
        }
        if !self.assembler.done {
            let err = LlmError::new(ErrorKind::Protocol, "stream ended before message_stop");
            return Err(attach(err, &head));
        }
        self.timing.total_ms = elapsed_ms(self.start);
        Ok(Completion {
            usage: self.assembler.usage,
            stop: self.assembler.stop.unwrap_or(StopReason::Other),
            message: self.assembler.message(),
            timing: self.timing,
            head,
        })
    }
}

impl Provider for Anthropic {
    fn id(&self) -> &str {
        "anthropic"
    }

    fn stream(
        &self,
        http: &dyn Transport,
        auth: &Auth,
        req: &ChatRequest<'_>,
        on_delta: &mut dyn FnMut(Delta<'_>),
    ) -> Result<Completion, LlmError> {
        if req.cancel.load(Ordering::Relaxed) {
            return Err(LlmError::new(ErrorKind::Aborted, "cancelled"));
        }
        let oauth = matches!(auth, Auth::OAuth(_));
        let (body, betas) = request::build(req, models::find(req.model), oauth);
        let body = body.to_string();
        let mut owned = wire::headers(auth, &betas);
        owned.push(("content-type".into(), "application/json".into()));
        let headers: Vec<(&str, &str)> = owned
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let url = format!("{}/v1/messages?beta=true", self.base_url);
        let http_req = HttpRequest {
            method: "POST",
            url: &url,
            headers: &headers,
            body: body.as_bytes(),
            timeout_ms: 0,
        };
        let mut run = Run {
            req,
            on_delta,
            start: Instant::now(),
            timing: Timing::default(),
            decoder: SseDecoder::new(),
            assembler: stream::Assembler::new(req.tools, oauth),
            error_body: Vec::new(),
            failure: None,
            aborted: false,
            got_token: false,
        };
        match http.send(&http_req, &mut |head, chunk| run.on_chunk(head, chunk)) {
            Ok(head) => run.finish(head),
            Err(_) if run.aborted || req.cancel.load(Ordering::Relaxed) => {
                Err(LlmError::new(ErrorKind::Aborted, "cancelled"))
            }
            Err(err) => Err(error::from_transport(&err)),
        }
    }
}
