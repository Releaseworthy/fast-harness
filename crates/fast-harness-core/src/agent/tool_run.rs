//! The tool calls of one step, run by effect on scoped threads (PLAN.md D54).

use std::ops::Range;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Instant;

use crate::agent::approval;
use crate::agent::children::TICK;
use crate::agent::hooks::{self, Record};
use crate::agent::nested;
use crate::agent::turn::{ms, traced, Res, Turn};
use crate::agent::Agent;
use crate::events::AgentEvent;
use crate::llm::find::{self, FIND_TOOLS};
use crate::tools::{Progress, ToolCtx, ToolOutput, ToolSession};
use crate::types::{Block, Message, Role};

/// What the tool threads of one step share.
struct Run<'a> {
    agent: &'a Agent,
    /// `None` when the registry loads every tool from the start.
    loaded: Option<&'a [String]>,
    cancel: &'a AtomicBool,
    session: &'a ToolSession,
    /// The session id given to hooks (PLAN.md D44).
    session_id: &'a str,
}

/// One call's output, duration, hook runs and hook context texts.
type Done = (ToolOutput, u64, Vec<Record>, Vec<String>);

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
        let calls = last_calls(&self.conv.messages);
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
            session_id: &self.conv.id,
        };
        let done = execute(&run, &calls, &mut *self.sink);
        let mut outputs = Vec::with_capacity(done.len());
        let mut durations = Vec::with_capacity(done.len());
        for (output, duration_ms, records, mut context) in done {
            self.log_hooks(&records)?;
            self.hook_context.append(&mut context);
            outputs.push(output);
            durations.push(duration_ms);
        }
        let args: Vec<&str> = calls.iter().map(|call| call.arguments.as_str()).collect();
        let cwd = &self.agent.parts.cwd;
        let gate = self.agent.parts.extensions.trust_home.as_deref();
        nested::attach(&self.conv.messages, cwd, gate, &args, &mut outputs);
        self.push_results(step, &calls, outputs, durations)
    }

    /// Traces every `tool/result` in call order and adds one tool message
    /// holding all results.
    fn push_results(
        &mut self,
        step: u32,
        calls: &[Call],
        outputs: Vec<ToolOutput>,
        durations: Vec<u64>,
    ) -> Res<()> {
        let results = outputs.into_iter().zip(durations);
        let mut blocks = Vec::with_capacity(calls.len());
        for (call, (output, duration_ms)) in calls.iter().zip(results) {
            let block = Block::ToolResult {
                call_id: call.id.clone(),
                content: output.text().into_owned(),
                is_error: output.is_error(),
            };
            let size = (output.bytes(), output.spilled());
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

/// The output of a call that a cancel kept from starting (PLAN.md D54).
pub(crate) const CANCELLED: &str = "cancelled before it ran";

/// The tool calls of the last message, in call order.
fn last_calls(messages: &[Message]) -> Vec<Call> {
    let Some(message) = messages.last() else {
        return Vec::new();
    };
    (message.tool_calls())
        .map(|(id, name, arguments)| Call {
            id: id.into(),
            name: name.into(),
            arguments: arguments.into(),
        })
        .collect()
}

/// Outputs and durations in call order. Events go out on this thread:
/// `ToolCall` for every call before any starts, `ToolResult` as each ends.
/// The calls run by effect (PLAN.md D54): a maximal run of consecutive
/// parallel calls is one batch started together; any other call runs alone,
/// after everything before it has finished.
fn execute(run: &Run<'_>, calls: &[Call], sink: &mut dyn FnMut(AgentEvent)) -> Vec<Done> {
    for call in calls {
        sink(AgentEvent::ToolCall {
            id: call.id.clone(),
            name: call.name.clone(),
            arguments: call.arguments.clone(),
        });
    }
    let mut results: Vec<Option<Done>> = vec![None; calls.len()];
    let mut start = 0;
    while start < calls.len() {
        let end = batch_end(run, calls, start);
        run_batch(run, calls, start..end, &mut results, sink);
        start = end;
    }
    let missing = || {
        (
            ToolOutput::err("the tool gave no result"),
            0,
            Vec::new(),
            Vec::new(),
        )
    };
    (results.into_iter())
        .map(|result| result.unwrap_or_else(missing))
        .collect()
}

/// The end of the batch starting at `start`: past the parallel calls that
/// follow it, or just past `start` when that call is not parallel.
fn batch_end(run: &Run<'_>, calls: &[Call], start: usize) -> usize {
    let parallel = |call: &Call| run.agent.tools.parallel(&call.name);
    if !calls.get(start).is_some_and(parallel) {
        return start + 1;
    }
    let rest = calls[start..].iter().position(|call| !parallel(call));
    rest.map_or(calls.len(), |count| start + count)
}

/// Runs the calls in `range` together on scoped threads, or none of them
/// when the turn is cancelled (each gets [`CANCELLED`]).
fn run_batch(
    run: &Run<'_>,
    calls: &[Call],
    range: Range<usize>,
    results: &mut [Option<Done>],
    sink: &mut dyn FnMut(AgentEvent),
) {
    if run.cancel.load(Ordering::Relaxed) {
        for index in range {
            let done = (ToolOutput::err(CANCELLED), 0, Vec::new(), Vec::new());
            finish(calls, index, done, results, sink);
        }
        return;
    }
    std::thread::scope(|scope| {
        let (tx, rx) = mpsc::channel();
        for index in range {
            let (tx, call) = (tx.clone(), &calls[index]);
            scope.spawn(move || {
                let started = Instant::now();
                // An unbounded send: never blocks on the agent thread, and a
                // closed channel (cannot happen in the scope) is ignored.
                let progress = |text: &str| {
                    let _ = tx.send(Msg::Progress(index, text.to_string()));
                };
                let emit = |event: AgentEvent| {
                    let _ = tx.send(Msg::Event(event));
                };
                let (output, records, context) = match not_loaded(run.agent, &call.name, run.loaded)
                {
                    Some(error) => (error, Vec::new(), Vec::new()),
                    None => run_one(run, call, &progress, &emit),
                };
                let done = (output, ms(started), records, context);
                let _ = tx.send(Msg::Done(index, done));
            });
        }
        drop(tx);
        collect(run, calls, &rx, results, sink);
    });
}

/// What a tool thread sends the agent thread. One channel keeps the order:
/// a call's progress is sent before its result (PLAN.md D58).
enum Msg {
    Progress(usize, String),
    /// An approval request or its end (PLAN.md D64).
    Event(AgentEvent),
    Done(usize, Done),
}

/// Takes the results of one batch as they end, sending each progress text
/// as it comes and forwarding subagent events every `TICK`.
fn collect(
    run: &Run<'_>,
    calls: &[Call],
    rx: &mpsc::Receiver<Msg>,
    results: &mut [Option<Done>],
    sink: &mut dyn FnMut(AgentEvent),
) {
    loop {
        // Subagents' events reach the sink while their tool call runs.
        match rx.recv_timeout(TICK) {
            Ok(Msg::Progress(index, chunk)) if results[index].is_none() => {
                let id = calls[index].id.clone();
                sink(AgentEvent::ToolProgress { id, chunk });
            }
            Ok(Msg::Progress(..)) => {}
            Ok(Msg::Event(event)) => sink(event),
            Ok(Msg::Done(index, done)) => {
                run.agent.forward_events(sink);
                finish(calls, index, done, results, sink);
            }
            Err(RecvTimeoutError::Timeout) => run.agent.forward_events(sink),
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// Sends `ToolResult` for call `index` and keeps its result.
fn finish(
    calls: &[Call],
    index: usize,
    done: Done,
    results: &mut [Option<Done>],
    sink: &mut dyn FnMut(AgentEvent),
) {
    sink(AgentEvent::ToolResult {
        id: calls[index].id.clone(),
        content: done.0.text().into_owned(),
        is_error: done.0.is_error(),
        duration_ms: done.1,
    });
    results[index] = Some(done);
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
/// The budget of each call comes from `Config::tool_budget` (PLAN.md D37);
/// PreToolUse and PostToolUse hooks wrap it (D44), and the approval gate
/// sees the arguments the PreToolUse hooks leave (D64).
fn run_one(
    run: &Run<'_>,
    call: &Call,
    progress: &Progress<'_>,
    emit: &dyn Fn(AgentEvent),
) -> (ToolOutput, Vec<Record>, Vec<String>) {
    let agent = run.agent;
    let ctx = ToolCtx::new(&agent.parts.cwd, run.cancel)
        .with_budget(agent.parts.config.tool_budget(&call.name))
        .with_call_id(&call.id)
        .with_session(run.session)
        .with_progress(progress);
    let gate = |raw: &str| approval::check(agent, (&call.id, &call.name, raw), run.cancel, emit);
    let go = || match !agent.parts.extensions.has_hooks() {
        true => {
            let output = gate(&call.arguments)
                .unwrap_or_else(|| agent.tools.run(&call.name, &call.arguments, &ctx));
            (output, Vec::new(), Vec::new())
        }
        false => {
            let names = (run.session_id, call.name.as_str(), call.arguments.as_str());
            hooks::run_tool(agent, names, &ctx, &gate)
        }
    };
    panic::catch_unwind(AssertUnwindSafe(go)).unwrap_or_else(|_| {
        let error = ToolOutput::err(format!("tool `{}` panicked", call.name));
        (error, Vec::new(), Vec::new())
    })
}
