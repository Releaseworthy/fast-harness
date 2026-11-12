//! One turn: user message, steps, turn end.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use crate::agent::{repair, Agent, Conversation};
use crate::auth::Lease;
use crate::events::{AgentEvent, TurnOutcome};
use crate::llm::catalog::context_window;
use crate::llm::Completion;
use crate::types::{Message, StopReason, Usage};

/// Turn-internal result: `Err` is a session log failure that ends the turn.
pub(super) type Res<T> = Result<T, String>;

pub(super) fn traced<T>(result: io::Result<T>) -> Res<T> {
    result.map_err(|err| format!("session log: {err}"))
}

pub(super) fn ms(since: Instant) -> u64 {
    since.elapsed().as_millis() as u64
}

pub(super) struct Turn<'a> {
    pub agent: &'a Agent,
    pub conv: &'a mut Conversation,
    pub cancel: &'a AtomicBool,
    pub sink: &'a mut dyn FnMut(AgentEvent),
    pub turn: u32,
    pub steps: u32,
    pub usage: Usage,
}

pub(super) fn run(
    agent: &Agent,
    conv: &mut Conversation,
    text: &str,
    cancel: &AtomicBool,
    sink: &mut dyn FnMut(AgentEvent),
) -> TurnOutcome {
    let started = Instant::now();
    conv.turn += 1;
    let mut turn = Turn {
        agent,
        turn: conv.turn,
        conv,
        cancel,
        sink,
        steps: 0,
        usage: Usage::default(),
    };
    let outcome = turn.run(text).unwrap_or_else(|message| {
        (turn.sink)(AgentEvent::Error(message));
        TurnOutcome::Failed
    });
    turn.finish(outcome, started)
}

pub(super) fn outcome_word(outcome: TurnOutcome) -> &'static str {
    match outcome {
        TurnOutcome::Done => "done",
        TurnOutcome::Cancelled => "cancelled",
        TurnOutcome::Failed => "failed",
        TurnOutcome::StepLimit => "step_limit",
    }
}

impl Turn<'_> {
    fn run(&mut self, text: &str) -> Res<TurnOutcome> {
        traced(self.conv.trace.turn_start(self.turn))?;
        (self.sink)(AgentEvent::TurnStart { turn: self.turn });
        repair(&mut self.conv.messages);
        let message = Message::user(text);
        traced(self.conv.trace.user_message(self.turn, &message))?;
        self.conv.messages.push(message);
        loop {
            if self.cancel.load(Ordering::Relaxed) {
                return Ok(TurnOutcome::Cancelled);
            }
            if self.steps >= self.agent.parts.config.max_steps {
                return Ok(TurnOutcome::StepLimit);
            }
            self.steps += 1;
            if let Some(outcome) = self.step()? {
                return Ok(outcome);
            }
        }
    }

    /// One model call and its tools; `Some` ends the turn.
    fn step(&mut self) -> Res<Option<TurnOutcome>> {
        let agent = self.agent;
        let (turn, step, started) = (self.turn, self.steps, Instant::now());
        traced(self.conv.trace.step_start(turn, step))?;
        (self.sink)(AgentEvent::StepStart { turn, step });
        let specs = agent.tools.specs();
        traced(self.conv.header_if_changed(&agent.parts.system, specs))?;
        let outcome = match self.call_model(step)? {
            Ok((completion, lease)) => self.finish_step(step, completion, &lease)?,
            Err(outcome) => Some(outcome),
        };
        traced(self.conv.trace.step_end(turn, step, ms(started)))?;
        Ok(outcome)
    }

    fn finish_step(
        &mut self,
        step: u32,
        mut done: Completion,
        lease: &Lease,
    ) -> Res<Option<TurnOutcome>> {
        let turn = self.turn;
        // Provenance for every provider: the full model id of this call.
        done.message.model = Some(self.agent.parts.config.model.clone());
        self.agent.parts.auth.observe(lease, &done.head);
        (self.sink)(AgentEvent::UsageUpdate {
            account: lease.account.clone(),
        });
        self.record(turn, step, &done, &lease.account)?;
        let has_calls = done.message.tool_calls().next().is_some();
        self.conv.messages.push(done.message);
        if done.stop == StopReason::ToolUse && has_calls {
            self.run_tools(step)?;
            return Ok(None);
        }
        Ok(Some(TurnOutcome::Done))
    }

    /// Traces the answer (and the server's input transformations), adds its
    /// usage to the turn and emits `StepEnd` with the context and window.
    fn record(&mut self, turn: u32, step: u32, done: &Completion, account: &str) -> Res<()> {
        let window = context_window(&self.agent.parts.config.model);
        let trace = &mut self.conv.trace;
        if let Some(changes) = &done.transformations {
            traced(trace.request_transformations(turn, step, changes))?;
        }
        traced(trace.assistant_message(
            turn,
            step,
            &done.message,
            &done.usage,
            done.stop,
            &done.timing,
            account,
            window,
        ))?;
        self.usage.add(&done.usage);
        (self.sink)(AgentEvent::StepEnd {
            turn,
            step,
            usage: done.usage,
            context: done.usage.context(),
            window,
            timing: done.timing,
            stop: done.stop,
        });
        Ok(())
    }

    fn finish(&mut self, outcome: TurnOutcome, started: Instant) -> TurnOutcome {
        let duration_ms = ms(started);
        let word = outcome_word(outcome);
        let written =
            self.conv
                .trace
                .turn_end(self.turn, self.steps, &self.usage, duration_ms, word);
        if let Err(err) = written {
            (self.sink)(AgentEvent::Error(format!("session log: {err}")));
        }
        (self.sink)(AgentEvent::TurnEnd {
            turn: self.turn,
            usage: self.usage,
            duration_ms,
            outcome,
        });
        outcome
    }
}
