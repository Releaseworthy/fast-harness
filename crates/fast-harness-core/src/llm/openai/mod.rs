//! `openai-chat`: any OpenAI-compatible Chat Completions endpoint (PLAN.md
//! D41), defined in `providers/*.json`. One call is
//! `POST {baseUrl}/chat/completions` with `stream: true` and
//! `stream_options.include_usage`, the key as `Authorization: Bearer`
//! (none when the key is empty, as for a local server) plus the file's
//! `headers`. Model ids are `<provider id>/<model id>`; the part after the
//! first `/` goes on the wire.

mod error;
mod request;
mod stream;

use std::sync::atomic::Ordering;
use std::time::Instant;

use super::custom;
use crate::http::{Flow, HttpHead, HttpRequest, Transport};
use crate::llm::{Auth, ChatRequest, Completion, Delta, ErrorKind, LlmError, Provider, Timing};
use crate::sse::SseDecoder;

const MAX_ERROR_BODY: usize = 64 * 1024;

#[derive(Clone, Debug)]
pub struct OpenAiChat {
    id: String,
    base_url: String,
    headers: Vec<(String, String)>,
}

impl OpenAiChat {
    /// Provider `id` at `base_url` (up to and without `/chat/completions`).
    pub fn new(id: &str, base_url: &str, headers: Vec<(String, String)>) -> OpenAiChat {
        OpenAiChat {
            id: id.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            headers,
        }
    }

    /// The JSON body this provider would send for `req`.
    pub fn request_body(&self, req: &ChatRequest<'_>) -> String {
        let entry = custom::model(req.model);
        let efforts = entry
            .as_ref()
            .map(|m| m.efforts.clone())
            .unwrap_or_default();
        let max_output = entry.as_ref().and_then(|m| m.max_output);
        request::build(req, self.wire_model(req.model), max_output, &efforts).to_string()
    }

    /// The request headers: JSON, SSE, the bearer unless the key is empty,
    /// then the file's `headers`.
    fn headers<'h>(&'h self, bearer: &'h str, no_key: bool) -> Vec<(&'h str, &'h str)> {
        let mut headers = vec![
            ("content-type", "application/json"),
            ("accept", "text/event-stream"),
        ];
        if !no_key {
            headers.push(("authorization", bearer));
        }
        headers.extend(self.headers.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        headers
    }

    fn wire_model<'m>(&self, model: &'m str) -> &'m str {
        match model.split_once('/') {
            Some((prefix, rest)) if prefix == self.id => rest,
            _ => model,
        }
    }
}

struct Run<'a, 'd> {
    req: &'a ChatRequest<'a>,
    on_delta: &'d mut dyn FnMut(Delta<'_>),
    start: Instant,
    timing: Timing,
    decoder: SseDecoder,
    assembler: stream::Assembler,
    error_body: Vec<u8>,
    failure: Option<LlmError>,
    aborted: bool,
    got_token: bool,
}

impl Run<'_, '_> {
    fn on_chunk(&mut self, head: &HttpHead, chunk: &[u8]) -> Flow {
        if self.timing.chunks == 0 {
            self.timing.ttfb_ms = self.start.elapsed().as_millis() as u64;
        }
        self.timing.chunks += 1;
        self.timing.bytes += chunk.len() as u64;
        if self.req.cancel.load(Ordering::Relaxed) {
            self.aborted = true;
            return Flow::Stop;
        }
        if !(200..300).contains(&head.status) {
            self.error_body.extend_from_slice(chunk);
            return match self.error_body.len() > MAX_ERROR_BODY {
                true => Flow::Stop,
                false => Flow::Continue,
            };
        }
        let mut decoder = std::mem::take(&mut self.decoder);
        decoder.feed(chunk, &mut |_, data| self.on_event(data));
        self.decoder = decoder;
        match self.failure.is_some() || self.assembler.done || self.aborted {
            true => Flow::Stop,
            false => Flow::Continue,
        }
    }

    fn on_event(&mut self, data: &str) {
        if self.failure.is_some() || self.assembler.done || self.aborted {
            return;
        }
        match self.assembler.handle(data, &mut *self.on_delta) {
            Ok(true) if !self.got_token => {
                self.got_token = true;
                self.timing.first_token_ms = self.start.elapsed().as_millis() as u64;
            }
            Ok(_) => {}
            Err(err) => self.failure = Some(err),
        }
    }

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
        decoder.finish(&mut |_, data| self.on_event(data));
        if let Some(err) = self.failure.take() {
            return Err(attach(err, &head));
        }
        if !self.assembler.complete() {
            let err = error::protocol("stream ended before a finish reason or [DONE]".into());
            return Err(attach(err, &head));
        }
        self.timing.total_ms = self.start.elapsed().as_millis() as u64;
        Ok(Completion {
            usage: self.assembler.usage,
            stop: self.assembler.stop(),
            message: self.assembler.message(),
            timing: self.timing,
            head,
            transformations: None,
        })
    }
}

impl Provider for OpenAiChat {
    fn id(&self) -> &str {
        &self.id
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
        let Auth::ApiKey(key) = auth else {
            let message = format!("{} takes an API key (keyEnv)", self.id);
            return Err(LlmError::new(ErrorKind::Auth, message));
        };
        let body = self.request_body(req);
        let bearer = format!("Bearer {key}");
        let headers = self.headers(&bearer, key.is_empty());
        let url = format!("{}/chat/completions", self.base_url);
        let http_req =
            HttpRequest::new("POST", &url, &headers, body.as_bytes()).with_cancel(req.cancel);
        let mut run = Run {
            req,
            on_delta,
            start: Instant::now(),
            timing: Timing::default(),
            decoder: SseDecoder::new(),
            assembler: stream::Assembler::default(),
            error_body: Vec::new(),
            failure: None,
            aborted: false,
            got_token: false,
        };
        let redact = |mut err: LlmError| {
            if !key.is_empty() {
                err.message = err.message.replace(key.as_str(), "<redacted>");
            }
            err
        };
        match http.send(&http_req, &mut |head, chunk| run.on_chunk(head, chunk)) {
            Ok(head) => run.finish(head).map_err(redact),
            Err(_) if run.aborted || req.cancel.load(Ordering::Relaxed) => {
                Err(LlmError::new(ErrorKind::Aborted, "cancelled"))
            }
            Err(err) => Err(redact(error::from_transport(&err))),
        }
    }
}
