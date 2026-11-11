//! One call over the Hub hosts, and the state of one attempt while body
//! chunks arrive (timing, deltas, cancel, finish), as the Anthropic provider.

use std::sync::atomic::Ordering;
use std::time::Instant;

use super::super::hub::next_host_on;
use super::super::stream::Assembler;
use super::super::ToolNames;
use super::{aborted, error};
use crate::http::{Flow, HttpErrorKind, HttpHead, HttpRequest, Transport};
use crate::llm::{ChatRequest, Completion, Delta, ErrorKind, LlmError, Timing};
use crate::sse::SseDecoder;

/// `streamGenerateContent` with server-sent events.
const STREAM_PATH: &str = "/v1internal:streamGenerateContent?alt=sse";

/// Longest error body kept from a failed call.
const MAX_ERROR_BODY: usize = 64 * 1024;

fn elapsed_ms(start: Instant) -> u64 {
    start.elapsed().as_millis() as u64
}

fn success(status: u16) -> bool {
    (200..300).contains(&status)
}

/// The fixed parts of one call.
pub(super) struct Call<'a> {
    pub http: &'a dyn Transport,
    pub headers: &'a [(&'a str, &'a str)],
    pub body: &'a [u8],
    pub req: &'a ChatRequest<'a>,
    pub names: &'a ToolNames,
}

/// What one host gave.
enum Attempt {
    Done(Result<Completion, LlmError>),
    /// Failed before any answer byte in a way the next host may not share.
    Next(LlmError),
}

impl Call<'_> {
    /// Tries `hosts` in order until one gives a final result.
    pub fn on_hosts(
        &self,
        hosts: &[String],
        on_delta: &mut dyn FnMut(Delta<'_>),
    ) -> Result<Completion, LlmError> {
        let mut last = LlmError::new(ErrorKind::Network, "no Antigravity host is configured");
        for (index, host) in hosts.iter().enumerate() {
            if self.req.cancel.load(Ordering::Relaxed) {
                return Err(aborted());
            }
            let fallback = index + 1 < hosts.len();
            match self.attempt(host, fallback, &mut *on_delta) {
                Attempt::Done(result) => return result,
                Attempt::Next(err) => last = err,
            }
        }
        Err(last)
    }

    fn attempt(&self, host: &str, fallback: bool, on_delta: &mut dyn FnMut(Delta<'_>)) -> Attempt {
        let url = format!("{host}{STREAM_PATH}");
        let request = HttpRequest {
            method: "POST",
            url: &url,
            headers: self.headers,
            body: self.body,
            timeout_ms: 0,
        };
        let mut run = Run::new(self.req, self.names, on_delta, fallback);
        let sent = self
            .http
            .send(&request, &mut |head, chunk| run.on_chunk(head, chunk));
        if run.aborted || self.req.cancel.load(Ordering::Relaxed) {
            return Attempt::Done(Err(aborted()));
        }
        match sent {
            Ok(head) if fallback && !success(head.status) && next_host_on(head.status) => {
                Attempt::Next(error::from_http(head, &[]))
            }
            Ok(head) => Attempt::Done(run.finish(head)),
            Err(err) if fallback && run.answer_bytes == 0 && err.kind != HttpErrorKind::Timeout => {
                Attempt::Next(error::from_transport(&err))
            }
            Err(err) => Attempt::Done(Err(error::from_transport(&err))),
        }
    }
}

/// State of one attempt while body chunks arrive.
struct Run<'a, 'd> {
    req: &'a ChatRequest<'a>,
    on_delta: &'d mut dyn FnMut(Delta<'_>),
    start: Instant,
    timing: Timing,
    decoder: SseDecoder,
    assembler: Assembler<'a>,
    /// Body of a non-2xx response.
    error_body: Vec<u8>,
    /// Bytes of a 2xx body.
    answer_bytes: u64,
    /// A later host may still be tried.
    fallback: bool,
    failure: Option<LlmError>,
    aborted: bool,
    got_token: bool,
}

impl<'a, 'd> Run<'a, 'd> {
    fn new(
        req: &'a ChatRequest<'a>,
        names: &'a ToolNames,
        on_delta: &'d mut dyn FnMut(Delta<'_>),
        fallback: bool,
    ) -> Run<'a, 'd> {
        Run {
            req,
            on_delta,
            start: Instant::now(),
            timing: Timing::default(),
            decoder: SseDecoder::new(),
            assembler: Assembler::new(names),
            error_body: Vec::new(),
            answer_bytes: 0,
            fallback,
            failure: None,
            aborted: false,
            got_token: false,
        }
    }

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
        if !success(head.status) {
            if self.fallback && next_host_on(head.status) {
                return Flow::Stop;
            }
            self.error_body.extend_from_slice(chunk);
            return if self.error_body.len() > MAX_ERROR_BODY {
                Flow::Stop
            } else {
                Flow::Continue
            };
        }
        self.answer_bytes += chunk.len() as u64;
        let mut decoder = std::mem::take(&mut self.decoder);
        decoder.feed(chunk, &mut |_, data| self.on_event(data));
        self.decoder = decoder;
        if self.failure.is_some() || self.aborted {
            Flow::Stop
        } else {
            Flow::Continue
        }
    }

    fn on_event(&mut self, data: &str) {
        if self.failure.is_some() || self.aborted {
            return;
        }
        if self.req.cancel.load(Ordering::Relaxed) {
            self.aborted = true;
            return;
        }
        match self.assembler.handle(data, &mut *self.on_delta) {
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
        if !success(head.status) {
            self.error_body.truncate(MAX_ERROR_BODY);
            return Err(error::from_http(head, &self.error_body));
        }
        let mut decoder = std::mem::take(&mut self.decoder);
        decoder.finish(&mut |_, data| self.on_event(data));
        if self.aborted {
            return Err(aborted());
        }
        if let Some(err) = self.failure.take() {
            return Err(error::with_head(err, &head));
        }
        let (message, usage, stop) = self
            .assembler
            .finish()
            .map_err(|err| error::with_head(err, &head))?;
        self.timing.total_ms = elapsed_ms(self.start);
        Ok(Completion {
            message,
            usage,
            stop,
            timing: self.timing,
            head,
        })
    }
}
