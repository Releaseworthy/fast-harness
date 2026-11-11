//! The tool calls of one step, run in parallel on scoped threads.

use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::time::Instant;

use crate::agent::turn::{ms, traced, Res, Turn};
use crate::agent::Agent;
use crate::events::AgentEvent;
use crate::tools::{ToolCtx, ToolOutput};
use crate::types::{Block, Message, Role};

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
        let results = execute(self.agent, &calls, self.cancel, &mut *self.sink);
        let mut blocks = Vec::with_capacity(calls.len());
        for (call, (output, duration_ms)) in calls.iter().zip(results) {
            let block = Block::ToolResult {
                call_id: call.id.clone(),
                content: output.content,
                is_error: output.is_error,
            };
            let message = Message {
                role: Role::Tool,
                content: vec![block.clone()],
                model: None,
            };
            traced(
                self.conv
                    .trace
                    .tool_result(self.turn, step, &message, duration_ms),
            )?;
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
    agent: &Agent,
    calls: &[Call],
    cancel: &AtomicBool,
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
                let output = run_one(agent, call, cancel);
                let _ = tx.send((index, output, ms(started)));
            });
        }
        drop(tx);
        for (index, output, duration_ms) in rx {
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

/// A panicking tool becomes an error result instead of ending the turn.
fn run_one(agent: &Agent, call: &Call, cancel: &AtomicBool) -> ToolOutput {
    let ctx = ToolCtx {
        cwd: &agent.parts.cwd,
        cancel,
    };
    let run = || agent.tools.run(&call.name, &call.arguments, &ctx);
    panic::catch_unwind(AssertUnwindSafe(run))
        .unwrap_or_else(|_| ToolOutput::err(format!("tool `{}` panicked", call.name)))
}
