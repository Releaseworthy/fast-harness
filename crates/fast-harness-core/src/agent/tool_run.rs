//! The tool calls of one step, run in parallel on scoped threads.

use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Instant;

use crate::agent::children::TICK;
use crate::agent::nested;
use crate::agent::turn::{ms, traced, Res, Turn};
use crate::agent::Agent;
use crate::events::AgentEvent;
use crate::llm::find::{self, FIND_TOOLS};
use crate::tools::{ToolCtx, ToolOutput, ToolSession};
use crate::types::{Block, Message, Role};

/// What the tool threads of one step share.
struct Run<'a> {
    agent: &'a Agent,
    /// `None` when the registry loads every tool from the start.
    loaded: Option<&'a [String]>,
    cancel: &'a AtomicBool,
    session: &'a ToolSession,
}

struct Call {
    id: String,
    name: String,
    arguments: String,
}

impl Turn<'_> {
    /// Runs the calls of the last assistant message. Traces every
    /// `tool/call` first, then every `tool/result` in call order, and adds
    /// one tool message holding all results.
    pub(super) fn run_tools(&mut self, step: u32) -> Res<()> {
        let calls: Vec<Call> = self
            .conv
            .messages
            .last()
            .map(|message| {
                let calls = message.tool_calls();
                calls
                    .map(|(id, name, arguments)| Call {
                        id: id.into(),
                        name: name.into(),
                        arguments: arguments.into(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        for call in &calls {
            let trace = &mut self.conv.trace;
            traced(trace.tool_call(self.turn, step, &call.id, &call.name, &call.arguments))?;
        }
        // Tools found by earlier steps; a find in this step loads for the next.
        let found = find::found(&self.conv.messages);
        let loaded = self.agent.tools.finds().then_some(found.as_slice());
        let shared = &self.conv.tool_session;
        let run = Run {
            agent: self.agent,
            loaded,
            cancel: self.cancel,
            session: shared,
        };
        let (mut outputs, durations): (Vec<ToolOutput>, Vec<u64>) =
            execute(&run, &calls, &mut *self.sink).into_iter().unzip();
        let args: Vec<&str> = calls.iter().map(|call| call.arguments.as_str()).collect();
        let cwd = &self.agent.parts.cwd;
        nested::attach(&self.conv.messages, cwd, &args, &mut outputs);
        let results = outputs.into_iter().zip(durations);
        let mut blocks = Vec::with_capacity(calls.len());
        for (call, (output, duration_ms)) in calls.iter().zip(results) {
            let block = Block::ToolResult {
                call_id: call.id.clone(),
                content: output.content,
                is_error: output.is_error,
            };
            let size = (output.bytes, output.spilled.as_deref());
            let message = Message {
                role: Role::Tool,
                content: vec![block.clone()],
                model: None,
            };
            traced(self.conv.trace.tool_result_sized(
                self.turn,
                step,
                &message,
                duration_ms,
                size,
            ))?;
            blocks.push(block);
        }
        self.conv.messages.push(Message {
            role: Role::Tool,
            content: blocks,
            model: None,
        });
        Ok(())
    }
}

/// Outputs and durations in call order. Events go out on this thread:
/// `ToolCall` for every call before any starts, `ToolResult` as each ends.
fn execute(
    run: &Run<'_>,
    calls: &[Call],
    sink: &mut dyn FnMut(AgentEvent),
) -> Vec<(ToolOutput, u64)> {
    for call in calls {
        sink(AgentEvent::ToolCall {
            id: call.id.clone(),
            name: call.name.clone(),
            arguments: call.arguments.clone(),
        });
    }
    let mut results: Vec<Option<(ToolOutput, u64)>> = vec![None; calls.len()];
    std::thread::scope(|scope| {
        let (tx, rx) = mpsc::channel();
        for (index, call) in calls.iter().enumerate() {
            let tx = tx.clone();
            scope.spawn(move || {
                let started = Instant::now();
                let output = match not_loaded(run.agent, &call.name, run.loaded) {
                    Some(error) => error,
                    None => run_one(run, call),
                };
                let _ = tx.send((index, output, ms(started)));
            });
        }
        drop(tx);
        loop {
            // Subagents' events reach the sink while their tool call runs.
            let (index, output, duration_ms) = match rx.recv_timeout(TICK) {
                Ok(done) => done,
                Err(RecvTimeoutError::Timeout) => {
                    run.agent.forward_events(sink);
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            };
            run.agent.forward_events(sink);
            sink(AgentEvent::ToolResult {
                id: calls[index].id.clone(),
                content: output.content.clone(),
                is_error: output.is_error,
                duration_ms,
            });
            results[index] = Some((output, duration_ms));
        }
    });
    results
        .into_iter()
        .map(|result| result.unwrap_or_else(|| (ToolOutput::err("the tool gave no result"), 0)))
        .collect()
}

/// The error for a known tool that no find has loaded yet (`loaded` is
/// `None` when the registry loads every tool from the start).
fn not_loaded(agent: &Agent, name: &str, loaded: Option<&[String]>) -> Option<ToolOutput> {
    let loaded = loaded?;
    let known = agent.tools.specs().iter().any(|spec| spec.name == name);
    if name == FIND_TOOLS || !known || loaded.iter().any(|found| found == name) {
        return None;
    }
    Some(ToolOutput::err(format!(
        "tool `{name}` is not loaded yet: call {FIND_TOOLS} with names [\"{name}\"] first, \
        then call it in a later step"
    )))
}

/// A panicking tool becomes an error result instead of ending the turn.
/// The budget of each call comes from `Config::tool_budget` (PLAN.md D37).
fn run_one(run: &Run<'_>, call: &Call) -> ToolOutput {
    let agent = run.agent;
    let ctx = ToolCtx {
        cwd: &agent.parts.cwd,
        cancel: run.cancel,
        budget: agent.parts.config.tool_budget(&call.name),
        call_id: &call.id,
        session: Some(run.session),
    };
    let go = || agent.tools.run(&call.name, &call.arguments, &ctx);
    panic::catch_unwind(AssertUnwindSafe(go))
        .unwrap_or_else(|_| ToolOutput::err(format!("tool `{}` panicked", call.name)))
}
