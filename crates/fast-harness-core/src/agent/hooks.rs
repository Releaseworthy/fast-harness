//! `hooks.json` in the agent loop (PLAN.md D44). Each point builds the
//! event data, runs the matching hooks in order (user file first) and
//! merges their answers; a block stops the later hooks of that event. A
//! failed hook is traced as `hook/error`, shown as an error note and
//! ignored (fail open). Without hooks nothing here runs a process.

use std::sync::atomic::AtomicBool;
use std::time::Instant;

use serde_json::{json, Value};

use crate::agent::compact::request_text;
use crate::agent::turn::{traced, Res, Turn};
use crate::agent::Agent;
use crate::config::compaction::Compaction;
use crate::events::{AgentEvent, TurnOutcome};
use crate::tools::hooks::{self, Answer, HookEvent, HookRun};
use crate::tools::{ToolCtx, ToolOutput};
use crate::trace::kind;
use crate::types::Role;

/// `kind` of a traced user text added by a hook (context or continuation).
pub const KIND: &str = "hook";
/// The head of a hook's context text in the history.
pub const CONTEXT_HEAD: &str = "[hook context]";

/// One hook run, traced later by the turn's thread.
#[derive(Clone, Debug)]
pub(super) struct Record {
    event: HookEvent,
    command: String,
    ms: u64,
    /// The decision word, or why the hook was ignored.
    result: Result<String, String>,
}

/// The merged answers of the hooks of one event.
#[derive(Default)]
pub(super) struct Verdict {
    pub block: Option<String>,
    pub arguments: Option<Value>,
    pub output: Option<String>,
    pub context: Vec<String>,
    pub cont: Option<String>,
    pub records: Vec<Record>,
}

/// Runs the hooks of `event` (for tool events those matching `tool`).
pub(super) fn fire(
    agent: &Agent,
    session: &str,
    cancel: &AtomicBool,
    (event, tool): (HookEvent, Option<&str>),
    data: Value,
) -> Verdict {
    let mut verdict = Verdict::default();
    let ext = &agent.parts.extensions;
    if !ext.has_hooks() {
        return verdict;
    }
    let cwd = &agent.parts.cwd;
    let mut input = json!({"event": event.name(), "sessionId": session,
        "cwd": cwd.to_string_lossy()});
    if let (Value::Object(map), Value::Object(data)) = (&mut input, data) {
        map.extend(data);
    }
    for entry in ext.hooks.matching(event, tool) {
        let run = hooks::run(entry, &input, cwd, cancel);
        let mut command = entry.command.clone();
        for arg in &entry.args {
            command = format!("{command} {arg}");
        }
        if verdict.record(event, command, run, &mut input) {
            return verdict;
        }
    }
    // Native plugin hooks (D52) after the file entries, same input and answer.
    for hook in (ext.native_hooks.iter()).filter(|hook| hook.matches(event, tool)) {
        let started = Instant::now();
        let answer = hook.run(event, &input);
        let ms = started.elapsed().as_millis() as u64;
        let command = format!("native:{}", hook.name());
        if verdict.record(event, command, HookRun { ms, answer }, &mut input) {
            break;
        }
    }
    verdict
}

impl Verdict {
    /// Records one run and merges its answer; true on a block.
    fn record(
        &mut self,
        event: HookEvent,
        command: String,
        run: HookRun,
        input: &mut Value,
    ) -> bool {
        let result = match &run.answer {
            Ok(answer) => Ok(answer.decision()),
            Err(error) => Err(error.clone()),
        };
        self.records.push(Record {
            event,
            command,
            ms: run.ms,
            result,
        });
        match run.answer {
            Ok(answer) => self.take(event, answer, input),
            Err(_) => false,
        }
    }

    /// Merges one answer (keys of other events are ignored); a later hook
    /// sees rewritten arguments and output in its input. True on a block.
    fn take(&mut self, event: HookEvent, answer: Answer, input: &mut Value) -> bool {
        if let (HookEvent::PreToolUse, Some(args)) = (event, answer.arguments) {
            input["arguments"] = args.clone();
            self.arguments = Some(args);
        }
        if let (HookEvent::PostToolUse, Some(output)) = (event, answer.output) {
            input["output"] = json!(output);
            self.output = Some(output);
        }
        self.context.extend(answer.context);
        if matches!(event, HookEvent::Stop | HookEvent::SubagentStop) && answer.cont.is_some() {
            self.cont = answer.cont;
        }
        self.block = answer.block;
        self.block.is_some()
    }
}

/// One tool call with its PreToolUse and PostToolUse hooks: a block makes
/// the result an error with the reason, `arguments` replace the call's,
/// `output` replaces the tool's own before the budget. Returns the
/// output, the hook runs and the context texts.
pub(super) fn run_tool(
    agent: &Agent,
    (session, name, raw): (&str, &str, &str),
    ctx: &ToolCtx<'_>,
) -> (ToolOutput, Vec<Record>, Vec<String>) {
    let args: Value = match raw.trim() {
        "" => json!({}),
        text => serde_json::from_str(text).unwrap_or_else(|_| json!(text)),
    };
    let data = json!({"toolName": name, "arguments": args});
    let pre = fire(
        agent,
        session,
        ctx.cancel,
        (HookEvent::PreToolUse, Some(name)),
        data,
    );
    let (mut records, mut context) = (pre.records, pre.context);
    if let Some(reason) = pre.block {
        return (blocked(&reason), records, context);
    }
    let raw = pre
        .arguments
        .map_or_else(|| raw.to_string(), |args| args.to_string());
    if !agent.parts.extensions.has_post_tool(name) {
        return (agent.tools.run(name, &raw, ctx), records, context);
    }
    let output = agent.tools.run_with(name, &raw, ctx, &mut |args, output| {
        let data = json!({"toolName": name, "arguments": args, "output": output.content,
            "isError": output.is_error});
        let post = fire(
            agent,
            session,
            ctx.cancel,
            (HookEvent::PostToolUse, Some(name)),
            data,
        );
        records.extend(post.records);
        context.extend(post.context);
        match (post.block, post.output) {
            (Some(reason), _) => blocked(&reason),
            (None, Some(text)) => ToolOutput::new(text, output.is_error),
            (None, None) => output,
        }
    });
    (output, records, context)
}

fn blocked(reason: &str) -> ToolOutput {
    ToolOutput::err(format!("blocked by a hook: {reason}"))
}

impl Turn<'_> {
    fn has_hooks(&self) -> bool {
        self.agent.parts.extensions.has_hooks()
    }

    /// Runs the hooks of `event`, traces them and keeps their context texts
    /// for the next model call.
    fn hook(&mut self, event: HookEvent, data: Value) -> Res<Verdict> {
        let (agent, session) = (self.agent, &self.conv.id);
        let mut verdict = fire(agent, session, self.cancel, (event, None), data);
        self.log_hooks(&verdict.records)?;
        self.hook_context.append(&mut verdict.context);
        Ok(verdict)
    }

    /// `hook` and `hook/error` events; a failure is also an error note.
    pub(super) fn log_hooks(&mut self, records: &[Record]) -> Res<()> {
        for record in records {
            let mut data = json!({"turn": self.turn, "event": record.event.name(),
                "command": record.command, "ms": record.ms});
            match &record.result {
                Ok(decision) => {
                    data["decision"] = json!(decision);
                    traced(self.conv.trace.emit(kind::HOOK, data))?;
                }
                Err(error) => {
                    data["error"] = json!(error);
                    traced(self.conv.trace.emit(kind::HOOK_ERROR, data))?;
                    let (event, command) = (record.event.name(), &record.command);
                    let note = format!("hook {event} `{command}` failed and was ignored: {error}");
                    (self.sink)(AgentEvent::Error(note));
                }
            }
        }
        Ok(())
    }

    /// SessionStart (the first turn of a new session) and UserPromptSubmit;
    /// `Some` when a hook blocked the prompt: the turn does not run.
    pub(super) fn prompt_hooks(&mut self, text: &str) -> Res<Option<TurnOutcome>> {
        if !self.has_hooks() {
            return Ok(None);
        }
        if self.conv.turn == 1 && self.conv.messages.is_empty() {
            self.hook(HookEvent::SessionStart, json!({"source": "new"}))?;
        }
        let verdict = self.hook(HookEvent::UserPromptSubmit, json!({"prompt": text}))?;
        let Some(reason) = verdict.block else {
            return Ok(None);
        };
        let note = format!("prompt blocked by a hook: {reason}");
        (self.sink)(AgentEvent::Error(note));
        Ok(Some(TurnOutcome::Failed))
    }

    /// The hooks' context texts at the tail, ahead of the next model call
    /// (after the last tool results or user text; earlier messages stay).
    pub(super) fn flush_hook_context(&mut self) -> Res<()> {
        if self.hook_context.is_empty() {
            return Ok(());
        }
        let texts = std::mem::take(&mut self.hook_context);
        let lines: Vec<String> = (texts.iter())
            .map(|text| format!("{CONTEXT_HEAD} {}", text.trim()))
            .collect();
        self.add_user_text(&lines.join("\n\n"), KIND)
    }

    /// Stop (SubagentStop in a subagent session) when the model ended the
    /// turn: a `continue` text or a block reason becomes the next user
    /// message and the turn goes on (true). The step limit bounds it.
    pub(super) fn stop_hook(&mut self) -> Res<bool> {
        if !self.has_hooks() {
            return Ok(false);
        }
        let event = match self.conv.depth {
            0 => HookEvent::Stop,
            _ => HookEvent::SubagentStop,
        };
        let last = self.conv.messages.last();
        let answer = last.filter(|m| m.role == Role::Assistant).map(|m| m.text());
        let data = json!({"answer": answer.unwrap_or_default(),
            "stopHookActive": self.stop_continued});
        let verdict = self.hook(event, data)?;
        let Some(text) = verdict.cont.or(verdict.block) else {
            return Ok(false);
        };
        self.stop_continued = true;
        self.add_user_text(&text, KIND)?;
        (self.sink)(AgentEvent::UserInput { text, steer: true });
        Ok(true)
    }

    /// PreCompact: `Err` when a hook blocked the compaction; else the
    /// instructions with the hooks' context texts appended.
    pub(super) fn pre_compact(
        &mut self,
        settings: &Compaction,
        instructions: Option<&str>,
        auto: bool,
    ) -> Res<Result<Option<String>, TurnOutcome>> {
        if !self.has_hooks() {
            return Ok(Ok(instructions.map(str::to_string)));
        }
        let data = json!({"trigger": if auto { "auto" } else { "manual" },
            "instructions": instructions, "summaryRequest": request_text(settings, instructions)});
        let (agent, session) = (self.agent, &self.conv.id);
        let verdict = fire(
            agent,
            session,
            self.cancel,
            (HookEvent::PreCompact, None),
            data,
        );
        self.log_hooks(&verdict.records)?;
        if let Some(reason) = verdict.block {
            let note = format!("compaction blocked by a hook: {reason}");
            (self.sink)(AgentEvent::Error(note));
            return Ok(Err(TurnOutcome::Failed));
        }
        let mut parts: Vec<String> = instructions.map(str::to_string).into_iter().collect();
        parts.extend(verdict.context);
        Ok(Ok((!parts.is_empty()).then(|| parts.join("\n\n"))))
    }
}
