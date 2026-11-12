//! One method per event of the trace format (docs/formats.md): each builds
//! the event's [`Record`] and writes it with [`TraceWriter::write`].

use crate::llm::{Timing, ToolSpec};
use crate::trace::record::{self, HeaderTool, Record};
use crate::trace::TraceWriter;
use crate::types::{Block, Message, StopReason, Usage};
use serde::Serialize;
use serde_json::Value;
use std::io;

/// Event type names.
pub mod kind {
    pub const TURN_START: &str = "turn/start";
    pub const USER_MESSAGE: &str = "user/message";
    pub const STEP_START: &str = "step/start";
    pub const REQUEST_HEADER: &str = "request/header";
    pub const REQUEST_CONTEXT: &str = "request/context";
    pub const ASSISTANT_ATTEMPT: &str = "assistant/attempt";
    pub const LLM_RETRY: &str = "llm/retry";
    pub const ACCOUNT_SWITCH: &str = "account/switch";
    pub const ASSISTANT_MESSAGE: &str = "assistant/message";
    pub const REQUEST_TRANSFORMATIONS: &str = "request/transformations";
    /// A compaction (PLAN.md D38): `session::replay` starts at the last one.
    pub const COMPACTION: &str = "compaction";
    pub const TOOL_CALL: &str = "tool/call";
    pub const TOOL_RESULT: &str = "tool/result";
    pub const STEP_END: &str = "step/end";
    pub const TURN_END: &str = "turn/end";
    /// A hook ran (PLAN.md D44): `event`, `command`, `ms`, `decision`.
    pub const HOOK: &str = "hook";
    /// A hook failed and was ignored: `event`, `command`, `ms`, `error`.
    pub const HOOK_ERROR: &str = "hook/error";
}

/// Data of `request/context`: one model call attempt (written as
/// [`record::Request`]).
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestContext<'a> {
    pub turn: u32,
    pub step: u32,
    pub attempt: u32,
    pub provider: &'a str,
    pub model: &'a str,
    /// Account id; never a credential.
    pub account: &'a str,
    pub effort: Option<&'a str>,
    /// Wire capture name of this attempt, such as `t1-s1-a1`; `None` (and no
    /// key in the log) when wire capture is off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wire: Option<&'a str>,
}

/// Bytes of a tool result block's content.
fn result_len(block: &Block) -> usize {
    match block {
        Block::ToolResult { content, .. } => content.len(),
        _ => 0,
    }
}

/// Entries of an `input_transformations` value that name
/// `prefix_binding_mismatch` (an array of entries, or one entry). The entry
/// shape is not documented, so any entry whose JSON holds the word counts.
pub fn prefix_binding_mismatches(transformations: &Value) -> u64 {
    let hit = |entry: &Value| entry.to_string().contains("prefix_binding_mismatch");
    match transformations {
        Value::Array(entries) => entries.iter().filter(|entry| hit(entry)).count() as u64,
        Value::Null => 0,
        other => u64::from(hit(other)),
    }
}

/// The `request/header` of `system` and `tools`, `context` left out when `None`.
pub fn header_data(
    system: &str,
    tools: &[ToolSpec],
    context: Option<&Value>,
) -> record::RequestHeader {
    let tools = tools.iter().map(|t| HeaderTool {
        name: t.name.clone(),
        description: Some(t.description.clone()),
        schema: Some(t.schema.clone()),
    });
    record::RequestHeader {
        system: system.to_string(),
        tools: tools.collect(),
        context: context.cloned(),
    }
}

impl TraceWriter {
    pub fn turn_start(&mut self, turn: u32) -> io::Result<u64> {
        let resume = false;
        self.write(&Record::TurnStart(record::TurnStart { turn, resume }))
    }

    pub fn user_message(&mut self, turn: u32, message: &Message) -> io::Result<u64> {
        let (message, kind) = (message.clone(), None);
        self.write(&Record::UserMessage(record::UserMessage {
            turn,
            message,
            kind,
        }))
    }

    /// A user text added inside a turn (version 8): `kind` `steer` or
    /// `context`; replay adds it to the message before it (D39).
    pub fn user_text(&mut self, turn: u32, message: &Message, kind: &str) -> io::Result<u64> {
        let (message, kind) = (message.clone(), Some(kind.to_string()));
        self.write(&Record::UserMessage(record::UserMessage {
            turn,
            message,
            kind,
        }))
    }

    /// The start of a turn that resumes the history without a user message.
    pub fn turn_resume(&mut self, turn: u32) -> io::Result<u64> {
        let resume = true;
        self.write(&Record::TurnStart(record::TurnStart { turn, resume }))
    }

    pub fn step_start(&mut self, turn: u32, step: u32) -> io::Result<u64> {
        self.write(&Record::StepStart(record::StepStart { turn, step }))
    }

    /// Written only when the system prompt or the tool list changes.
    pub fn request_header(&mut self, system: &str, tools: &[ToolSpec]) -> io::Result<u64> {
        self.request_header_data(header_data(system, tools, None))
    }

    /// `request/header` with the session's context stamp (PLAN.md D36).
    pub fn request_header_data(&mut self, header: record::RequestHeader) -> io::Result<u64> {
        self.write(&Record::RequestHeader(header))
    }

    pub fn request_context(&mut self, context: &RequestContext<'_>) -> io::Result<u64> {
        let request = record::Request {
            turn: context.turn,
            step: context.step,
            attempt: context.attempt,
            provider: context.provider.to_string(),
            model: context.model.to_string(),
            account: context.account.to_string(),
            effort: Some(context.effort.map(str::to_string)),
            wire: context.wire.map(|wire| Some(wire.to_string())),
        };
        self.write(&Record::RequestContext(request))
    }

    /// A failed attempt, written before `llm/retry` or `account/switch`.
    pub fn assistant_attempt(
        &mut self,
        turn: u32,
        step: u32,
        attempt: u32,
        error: &str,
        timing: &Timing,
    ) -> io::Result<u64> {
        let (error, timing) = (error.to_string(), *timing);
        self.write(&Record::AssistantAttempt(record::AssistantAttempt {
            turn,
            step,
            attempt,
            error,
            timing,
        }))
    }

    pub fn llm_retry(
        &mut self,
        turn: u32,
        step: u32,
        attempt: u32,
        delay_ms: u64,
        error: &str,
    ) -> io::Result<u64> {
        let error = error.to_string();
        self.write(&Record::LlmRetry(record::LlmRetry {
            turn,
            step,
            attempt,
            delay_ms,
            error,
        }))
    }

    /// `reason` is the error kind that caused the switch (`RateLimit`, ...)
    /// or a word such as `Manual`.
    pub fn account_switch(
        &mut self,
        from: &str,
        to: &str,
        reason: &str,
        hold_until: Option<u64>,
    ) -> io::Result<u64> {
        self.write(&Record::AccountSwitch(record::AccountSwitch {
            from: from.to_string(),
            to: to.to_string(),
            reason: reason.to_string(),
            hold_until: Some(hold_until),
        }))
    }

    #[allow(clippy::too_many_arguments)] // mirrors the event's data keys one to one
    pub fn assistant_message(
        &mut self,
        turn: u32,
        step: u32,
        message: &Message,
        usage: &Usage,
        stop: StopReason,
        timing: &Timing,
        account: &str,
        window: u64,
    ) -> io::Result<u64> {
        self.write(&Record::AssistantMessage(record::AssistantMessage {
            turn,
            step,
            message: message.clone(),
            usage: *usage,
            context: Some(usage.context()),
            window: Some(window),
            stop: stop_word(stop),
            timing: *timing,
            account: Some(account.to_string()),
        }))
    }

    /// `request/transformations`: the server's `input_transformations` of
    /// one call and how many of them are `prefix_binding_mismatch`.
    pub fn request_transformations(
        &mut self,
        turn: u32,
        step: u32,
        transformations: &Value,
    ) -> io::Result<u64> {
        let data = record::RequestTransformations {
            turn,
            step,
            transformations: transformations.clone(),
            prefix_binding_mismatch: prefix_binding_mismatches(transformations),
        };
        self.write(&Record::RequestTransformations(data))
    }

    pub fn tool_call(
        &mut self,
        turn: u32,
        step: u32,
        call_id: &str,
        name: &str,
        arguments: &str,
    ) -> io::Result<u64> {
        self.write(&Record::ToolCall(record::ToolCall {
            turn,
            step,
            call_id: call_id.to_string(),
            name: name.to_string(),
            arguments: arguments.to_string(),
        }))
    }

    /// `message` has role `tool`; a subagent result names the child session in its content.
    pub fn tool_result(
        &mut self,
        turn: u32,
        step: u32,
        message: &Message,
        duration_ms: u64,
    ) -> io::Result<u64> {
        let bytes = message.content.iter().map(result_len).sum();
        self.tool_result_sized(turn, step, message, duration_ms, (bytes, None))
    }

    /// `tool/result` with `bytes` (the full output size) and `spilled` (the
    /// spill file, left out when the result was not cut; trace version 6).
    pub fn tool_result_sized(
        &mut self,
        turn: u32,
        step: u32,
        message: &Message,
        duration_ms: u64,
        (bytes, spilled): (usize, Option<&str>),
    ) -> io::Result<u64> {
        self.write(&Record::ToolResult(record::ToolResult {
            turn,
            step,
            message: message.clone(),
            duration_ms,
            bytes: Some(bytes as u64),
            spilled: spilled.map(str::to_string),
        }))
    }

    /// `compaction` as built by the agent (PLAN.md D38).
    pub fn compaction(&mut self, data: record::Compaction) -> io::Result<u64> {
        self.write(&Record::Compaction(data))
    }

    pub fn step_end(&mut self, turn: u32, step: u32, duration_ms: u64) -> io::Result<u64> {
        self.write(&Record::StepEnd(record::StepEnd {
            turn,
            step,
            duration_ms,
        }))
    }

    /// `outcome` is `done`, `error`, `cancelled` or another short word.
    pub fn turn_end(
        &mut self,
        turn: u32,
        steps: u32,
        usage: &Usage,
        duration_ms: u64,
        outcome: &str,
    ) -> io::Result<u64> {
        self.write(&Record::TurnEnd(record::TurnEnd {
            turn,
            steps,
            usage: *usage,
            duration_ms,
            outcome: outcome.to_string(),
        }))
    }
}

/// The logged word of a stop reason (`end`, `tool_use`, ...).
fn stop_word(stop: StopReason) -> String {
    match serde_json::to_value(stop) {
        Ok(Value::String(word)) => word,
        _ => "other".to_string(),
    }
}
