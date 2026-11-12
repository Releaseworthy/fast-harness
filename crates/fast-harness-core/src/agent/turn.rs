//! One turn: user message, steps, turn end.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use crate::agent::inbox::{self, Inbox};
use crate::agent::{compact, repair, Agent, Conversation};
use crate::auth::Lease;
use crate::events::{AgentEvent, TurnOutcome};
use crate::llm::catalog::context_window;
use crate::llm::Completion;
use crate::types::{Block, Message, StopReason, Usage};

/// The error shown when the prompt no longer fits the context window.
pub const OVERFLOW_NOTE: &str = "the conversation no longer fits the model's context window, even after a compaction: start a new session";

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
    /// Steers, follow-ups and pauses from the front end (PLAN.md D39).
    pub inbox: &'a mut Inbox,
    pub sink: &'a mut dyn FnMut(AgentEvent),
    pub turn: u32,
    pub steps: u32,
    pub usage: Usage,
    /// The one automatic continuation after a tool call cut at max_tokens was used.
    pub continued: bool,
    /// The one compaction after a context overflow was used (PLAN.md D38).
    pub overflowed: bool,
    /// An automatic compaction failed this turn: no second try.
    pub compact_failed: bool,
    /// A compaction's summary request runs: no deltas to the sink.
    pub quiet: bool,
    /// Model and effort of the summary request while it runs.
    pub request: Option<(String, Option<String>)>,
    /// The error text of a context overflow, shown when compaction cannot help.
    pub last_error: Option<String>,
}

/// How a turn begins: with a user text, or by resuming a history that ends
/// with tool results or user text (PLAN.md D39).
#[derive(Clone, Copy)]
pub(super) enum Start<'t> {
    Text(&'t str),
    Resume,
}

/// What a turn runs on besides the agent and the conversation.
pub(super) struct Io<'a> {
    pub cancel: &'a AtomicBool,
    pub inbox: &'a mut Inbox,
    pub sink: &'a mut dyn FnMut(AgentEvent),
}

pub(super) fn run(
    agent: &Agent,
    conv: &mut Conversation,
    start: Start<'_>,
    io: Io<'_>,
) -> TurnOutcome {
    let started = Instant::now();
    conv.turn += 1;
    let mut turn = Turn {
        agent,
        turn: conv.turn,
        conv,
        cancel: io.cancel,
        inbox: io.inbox,
        sink: io.sink,
        steps: 0,
        usage: Usage::default(),
        continued: false,
        overflowed: false,
        compact_failed: false,
        quiet: false,
        request: None,
        last_error: None,
    };
    let outcome = turn.run(start).unwrap_or_else(|message| {
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
        TurnOutcome::Truncated => "truncated",
        TurnOutcome::ContextOverflow => "context_overflow",
        TurnOutcome::Paused => "paused",
    }
}

impl Turn<'_> {
    fn run(&mut self, start: Start<'_>) -> Res<TurnOutcome> {
        let text = match start {
            Start::Text(text) => Some(text),
            Start::Resume => None,
        };
        let trace = &mut self.conv.trace;
        traced(match text {
            Some(_) => trace.turn_start(self.turn),
            None => trace.turn_resume(self.turn),
        })?;
        (self.sink)(AgentEvent::TurnStart { turn: self.turn });
        repair(&mut self.conv.messages);
        if let Some(instructions) = text.and_then(compact::command) {
            return self.manual_compact(instructions);
        }
        let agent = self.agent;
        let specs = agent.tools.specs();
        let note = (self.conv).freeze(&agent.parts.system, specs, agent.context.as_ref());
        // The header (with the new context stamp) goes first: a crash before
        // the user message then cannot repeat the context update on resume.
        traced(self.conv.header_if_changed())?;
        // A resume is mid-task: a compaction then asks to continue it.
        if let Some(outcome) = self.auto_compact(text.is_none())? {
            return Ok(outcome);
        }
        match text {
            Some(text) => {
                let message = user_message(note, text);
                traced(self.conv.trace.user_message(self.turn, &message))?;
                self.conv.messages.push(message);
            }
            None => {
                if let Some(note) = note {
                    self.add_user_text(note.trim_end(), inbox::CONTEXT)?;
                }
            }
        }
        self.steps_loop()
    }

    /// Steps until one ends the turn. At each boundary (tool results
    /// complete): cancel, step limit, pause, compaction, then steers.
    fn steps_loop(&mut self) -> Res<TurnOutcome> {
        loop {
            if self.cancel.load(Ordering::Relaxed) {
                return Ok(TurnOutcome::Cancelled);
            }
            if self.steps >= self.agent.parts.config.max_steps {
                return Ok(TurnOutcome::StepLimit);
            }
            if self.inbox.take_pause() {
                // A steer that came with the pause is kept for the resume.
                self.apply_steers()?;
                return Ok(TurnOutcome::Paused);
            }
            // Between steps the tool results are complete (PLAN.md D38).
            if self.steps > 0 {
                if let Some(outcome) = self.auto_compact(true)? {
                    return Ok(outcome);
                }
            }
            self.apply_steers()?;
            self.steps += 1;
            if let Some(outcome) = self.step()? {
                return Ok(outcome);
            }
        }
    }
    /// One model call and its tools; `Some` ends the turn.
    fn step(&mut self) -> Res<Option<TurnOutcome>> {
        let (turn, step, started) = (self.turn, self.steps, Instant::now());
        traced(self.conv.trace.step_start(turn, step))?;
        (self.sink)(AgentEvent::StepStart { turn, step });
        traced(self.conv.header_if_changed())?;
        let mut called = self.call_model(step)?;
        if matches!(called, Err(TurnOutcome::ContextOverflow)) {
            called = self.after_overflow(step)?;
        }
        let outcome = match called {
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
        let cut = done.stop == StopReason::MaxTokens && has_cut_call(&done.message);
        self.conv.messages.push(done.message);
        match done.stop {
            StopReason::ToolUse | StopReason::PauseTurn if has_calls => self.run_tools(step)?,
            // Once per turn: the cut call gets an error result and the model
            // sends it again (PLAN.md D36).
            StopReason::MaxTokens if cut && !self.continued => {
                self.continued = true;
                self.run_tools(step)?;
            }
            StopReason::MaxTokens => return Ok(Some(TurnOutcome::Truncated)),
            StopReason::PauseTurn => {}
            StopReason::ContextWindow => return self.window_full(),
            // A steer that came during the last step starts the next one.
            _ if self.apply_steers()? => {}
            _ => return Ok(Some(TurnOutcome::Done)),
        }
        Ok(None)
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
        self.conv.last_context = done.usage.context();
        self.conv.compacted = false;
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

/// The user message of a turn: the context update first when there is one.
fn user_message(note: Option<String>, text: &str) -> Message {
    let mut message = Message::user(text);
    if let Some(note) = note {
        let text = format!("{note}\n\n");
        message.content.insert(0, Block::Text { text });
    }
    message
}

/// True when a tool call of message holds arguments that are not complete JSON.
fn has_cut_call(message: &Message) -> bool {
    message
        .tool_calls()
        .any(|(_, _, args)| serde_json::from_str::<serde_json::Value>(args).is_err())
}
