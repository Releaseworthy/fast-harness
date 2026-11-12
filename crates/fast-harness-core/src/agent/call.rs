//! One step's model call: attempts, retries with backoff, account failover.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::agent::turn::{ms, traced, Res, Turn};
use crate::auth::{Failover, Lease};
use crate::events::{AgentEvent, TurnOutcome};
use crate::http::Transport;
use crate::llm::{Cache, ChatRequest, Completion, Delta, ErrorKind, LlmError, Timing};
use crate::trace::wire::WireCapture;
use crate::trace::RequestContext;

/// First retry delay; doubled per retry up to [`MAX_BACKOFF_MS`].
pub const FIRST_BACKOFF_MS: u64 = 1_000;
pub const MAX_BACKOFF_MS: u64 = 30_000;
/// Failover decisions accepted in one step, so a source that keeps
/// answering `Retry` cannot loop forever.
const MAX_FAILOVERS: u32 = 64;
/// How often a wait looks at the cancel flag.
const WAIT_SLICE_MS: u64 = 20;

/// Delay before retry number `retry` (0 based) without a `retry-after`.
pub fn backoff_ms(retry: u32) -> u64 {
    FIRST_BACKOFF_MS
        .saturating_mul(1u64 << retry.min(16))
        .min(MAX_BACKOFF_MS)
}

/// Sleeps `ms` milliseconds unless `cancel` is set first; false when cancelled.
pub fn wait(ms: u64, cancel: &AtomicBool) -> bool {
    let deadline = Instant::now() + Duration::from_millis(ms);
    loop {
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        let now = Instant::now();
        if now >= deadline {
            return true;
        }
        let slice = Duration::from_millis(WAIT_SLICE_MS).min(deadline - now);
        std::thread::sleep(slice);
    }
}

#[derive(Default)]
struct Attempts {
    attempt: u32,
    retries: u32,
    failovers: u32,
    /// Account and reason of a failover, until the next lease shows the result.
    switch: Option<(String, String)>,
}

enum Next {
    Again,
    End(TurnOutcome),
}

impl Turn<'_> {
    /// The completion and the lease that produced it, or how the turn ends.
    pub(super) fn call_model(
        &mut self,
        step: u32,
    ) -> Res<Result<(Completion, Lease), TurnOutcome>> {
        let agent = self.agent;
        let parts = &agent.parts;
        let mut state = Attempts::default();
        loop {
            state.attempt += 1;
            let lease = match parts.auth.lease(&*parts.transport) {
                Ok(lease) => lease,
                Err(err) => {
                    (self.sink)(AgentEvent::Error(format!("no credential: {err}")));
                    return Ok(Err(TurnOutcome::Failed));
                }
            };
            self.note_switch(&mut state, &lease)?;
            let wire = format!("t{}-s{step}-a{}", self.turn, state.attempt);
            self.trace_context(step, state.attempt, &lease, &wire)?;
            let started = Instant::now();
            let err = match self.stream_once(&lease, &wire) {
                Ok(completion) => return Ok(Ok((completion, lease))),
                Err(err) => err,
            };
            let timing = Timing {
                total_ms: ms(started),
                ..Timing::default()
            };
            let text = err.to_string();
            let trace = &mut self.conv.trace;
            traced(trace.assistant_attempt(self.turn, step, state.attempt, &text, &timing))?;
            if let Next::End(outcome) = self.on_error(step, &mut state, &lease, &err)? {
                if matches!(outcome, TurnOutcome::Failed | TurnOutcome::ContextOverflow) {
                    (self.sink)(AgentEvent::Error(text));
                }
                return Ok(Err(outcome));
            }
        }
    }

    fn note_switch(&mut self, state: &mut Attempts, lease: &Lease) -> Res<()> {
        let Some((from, reason)) = state.switch.take() else {
            return Ok(());
        };
        if from == lease.account {
            return Ok(());
        }
        traced(
            self.conv
                .trace
                .account_switch(&from, &lease.account, &reason, None),
        )?;
        (self.sink)(AgentEvent::AccountSwitch {
            from,
            to: lease.account.clone(),
            reason,
        });
        Ok(())
    }

    fn trace_context(&mut self, step: u32, attempt: u32, lease: &Lease, wire: &str) -> Res<()> {
        let agent = self.agent;
        let parts = &agent.parts;
        let context = RequestContext {
            turn: self.turn,
            step,
            attempt,
            provider: parts.provider.id(),
            model: &parts.config.model,
            account: &lease.account,
            effort: parts.config.effort.as_deref(),
            wire: parts.config.wire.then_some(wire),
        };
        traced(self.conv.trace.request_context(&context))?;
        (self.sink)(AgentEvent::Request {
            model: parts.config.model.clone(),
            account: lease.account.clone(),
            attempt,
        });
        Ok(())
    }

    fn stream_once(&mut self, lease: &Lease, wire_name: &str) -> Result<Completion, LlmError> {
        let agent = self.agent;
        let parts = &agent.parts;
        // Raw wire capture only when the config asks for it (off by default).
        let capture = parts
            .config
            .wire
            .then(|| WireCapture::new(&*parts.transport, &self.conv.wire_dir, wire_name));
        let http: &dyn Transport = match &capture {
            Some(capture) => capture,
            None => &*parts.transport,
        };
        let (system_1h, messages_1h) = parts.config.cache_hours();
        // The session's frozen system and tools (PLAN.md D36).
        let conv = &*self.conv;
        let req = ChatRequest {
            model: &parts.config.model,
            system: conv.system.as_deref().unwrap_or(&parts.system),
            messages: &conv.messages,
            tools: conv.tools.as_deref().unwrap_or(agent.tools.specs()),
            max_tokens: parts.config.max_tokens,
            effort: parts.config.effort.as_deref(),
            cache: Cache {
                system_1h,
                messages_1h,
            },
            session: &conv.id,
            cancel: self.cancel,
        };
        let sink = &mut *self.sink;
        let mut ids: Vec<String> = Vec::new();
        let mut on_delta = |delta: Delta<'_>| forward(delta, &mut ids, sink);
        parts
            .provider
            .stream(http, &lease.auth, &req, &mut on_delta)
    }

    fn on_error(
        &mut self,
        step: u32,
        state: &mut Attempts,
        lease: &Lease,
        err: &LlmError,
    ) -> Res<Next> {
        let agent = self.agent;
        let config = &agent.parts.config;
        if err.kind == ErrorKind::Aborted || self.cancel.load(Ordering::Relaxed) {
            return Ok(Next::End(TurnOutcome::Cancelled));
        }
        match err.kind {
            ErrorKind::ContextOverflow => Ok(Next::End(TurnOutcome::ContextOverflow)),
            ErrorKind::RateLimit | ErrorKind::Auth => self.failover(step, state, lease, err),
            _ if err.retryable() && state.retries < config.retries => {
                let delay = err
                    .retry_after_ms()
                    .unwrap_or_else(|| backoff_ms(state.retries));
                state.retries += 1;
                if delay > config.max_wait_ms {
                    return Ok(Next::End(TurnOutcome::Failed));
                }
                self.wait_retry(step, state.attempt, delay, err)
            }
            _ => Ok(Next::End(TurnOutcome::Failed)),
        }
    }

    fn failover(
        &mut self,
        step: u32,
        state: &mut Attempts,
        lease: &Lease,
        err: &LlmError,
    ) -> Res<Next> {
        let agent = self.agent;
        let parts = &agent.parts;
        state.failovers += 1;
        if state.failovers > MAX_FAILOVERS {
            return Ok(Next::End(TurnOutcome::Failed));
        }
        let reason = format!("{:?}", err.kind);
        let decision = parts
            .auth
            .on_failure(lease, err, &parts.config.model, &*parts.transport);
        match decision {
            Failover::Retry => {
                state.switch = Some((lease.account.clone(), reason));
                Ok(Next::Again)
            }
            Failover::Wait(delay) if delay <= parts.config.max_wait_ms => {
                state.switch = Some((lease.account.clone(), reason));
                self.wait_retry(step, state.attempt, delay, err)
            }
            Failover::Wait(_) | Failover::Fail => Ok(Next::End(TurnOutcome::Failed)),
        }
    }

    fn wait_retry(&mut self, step: u32, attempt: u32, delay: u64, err: &LlmError) -> Res<Next> {
        let error = err.to_string();
        traced(
            self.conv
                .trace
                .llm_retry(self.turn, step, attempt, delay, &error),
        )?;
        (self.sink)(AgentEvent::Retry {
            attempt,
            delay_ms: delay,
            error,
        });
        Ok(match wait(delay, self.cancel) {
            true => Next::Again,
            false => Next::End(TurnOutcome::Cancelled),
        })
    }
}

/// Turns a provider delta into an event; `ids` maps tool call indexes to ids.
fn forward(delta: Delta<'_>, ids: &mut Vec<String>, sink: &mut dyn FnMut(AgentEvent)) {
    match delta {
        Delta::Text(text) => sink(AgentEvent::TextDelta(text.into())),
        Delta::Thinking(text) => sink(AgentEvent::ThinkingDelta(text.into())),
        Delta::ToolStart { index, id, name } => {
            if ids.len() <= index {
                ids.resize(index + 1, String::new());
            }
            ids[index] = id.to_string();
            sink(AgentEvent::ToolCallStart {
                id: id.into(),
                name: name.into(),
            });
        }
        Delta::ToolArgs { index, chunk } => sink(AgentEvent::ToolArgsDelta {
            id: ids.get(index).cloned().unwrap_or_default(),
            chunk: chunk.into(),
        }),
    }
}
