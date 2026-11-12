//! One typed method per event of the trace format (PLAN.md), each producing
//! exactly the data keys the format shows.

use crate::llm::{Timing, ToolSpec};
use crate::trace::TraceWriter;
use crate::types::{Block, Message, StopReason, Usage};
use serde::Serialize;
use serde_json::{json, Value};
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

/// Data of `request/context`: one model call attempt.
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

#[derive(Serialize)]
struct AssistantData<'a> {
    turn: u32,
    step: u32,
    message: &'a Message,
    usage: &'a Usage,
    /// Prompt plus output tokens of this call (trace version 4).
    context: u64,
    /// The model's context window; 0 when unknown (trace version 4).
    window: u64,
    stop: StopReason,
    timing: &'a Timing,
    account: &'a str,
}

/// Bytes of a tool result block's content.
fn result_len(block: &Block) -> usize {
    match block {
        Block::ToolResult { content, .. } => content.len(),
        _ => 0,
    }
}

fn value(data: impl Serialize) -> io::Result<Value> {
    Ok(serde_json::to_value(data)?)
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

/// The data of a `request/header`: `{system, tools, context?}`, `context`
/// left out when `None`.
pub fn header_data(system: &str, tools: &[ToolSpec], context: Option<&Value>) -> Value {
    let tools: Vec<Value> = tools
        .iter()
        .map(|t| json!({ "name": t.name, "description": t.description, "schema": t.schema }))
        .collect();
    let mut data = json!({ "system": system, "tools": tools });
    if let Some(context) = context {
        data["context"] = context.clone();
    }
    data
}

impl TraceWriter {
    pub fn turn_start(&mut self, turn: u32) -> io::Result<u64> {
        self.emit(kind::TURN_START, json!({ "turn": turn }))
    }

    pub fn user_message(&mut self, turn: u32, message: &Message) -> io::Result<u64> {
        let data = json!({ "turn": turn, "message": value(message)? });
        self.emit(kind::USER_MESSAGE, data)
    }

    /// A user text added inside a turn (version 8): `kind` `steer` or
    /// `context`; replay adds it to the message before it (D39).
    pub fn user_text(&mut self, turn: u32, message: &Message, kind: &str) -> io::Result<u64> {
        let data = json!({ "turn": turn, "message": value(message)?, "kind": kind });
        self.emit(kind::USER_MESSAGE, data)
    }

    /// The start of a turn that resumes the history without a user message.
    pub fn turn_resume(&mut self, turn: u32) -> io::Result<u64> {
        self.emit(kind::TURN_START, json!({ "turn": turn, "resume": true }))
    }

    pub fn step_start(&mut self, turn: u32, step: u32) -> io::Result<u64> {
        self.emit(kind::STEP_START, json!({ "turn": turn, "step": step }))
    }

    /// Written only when the system prompt or the tool list changes.
    pub fn request_header(&mut self, system: &str, tools: &[ToolSpec]) -> io::Result<u64> {
        self.emit(kind::REQUEST_HEADER, header_data(system, tools, None))
    }

    /// `request/header` with the session's context stamp (PLAN.md D36), the
    /// `data` of [`header_data`].
    pub fn request_header_data(&mut self, data: Value) -> io::Result<u64> {
        self.emit(kind::REQUEST_HEADER, data)
    }

    pub fn request_context(&mut self, context: &RequestContext<'_>) -> io::Result<u64> {
        self.emit(kind::REQUEST_CONTEXT, value(context)?)
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
        let data = json!({ "turn": turn, "step": step, "attempt": attempt,
            "error": error, "timing": value(timing)? });
        self.emit(kind::ASSISTANT_ATTEMPT, data)
    }

    pub fn llm_retry(
        &mut self,
        turn: u32,
        step: u32,
        attempt: u32,
        delay_ms: u64,
        error: &str,
    ) -> io::Result<u64> {
        let data = json!({ "turn": turn, "step": step, "attempt": attempt,
            "delayMs": delay_ms, "error": error });
        self.emit(kind::LLM_RETRY, data)
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
        let data = json!({ "from": from, "to": to, "reason": reason, "holdUntil": hold_until });
        self.emit(kind::ACCOUNT_SWITCH, data)
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
        let data = AssistantData {
            turn,
            step,
            message,
            usage,
            context: usage.context(),
            window,
            stop,
            timing,
            account,
        };
        self.emit(kind::ASSISTANT_MESSAGE, value(data)?)
    }

    /// `request/transformations`: the server's `input_transformations` of
    /// one call and how many of them are `prefix_binding_mismatch`.
    pub fn request_transformations(
        &mut self,
        turn: u32,
        step: u32,
        transformations: &Value,
    ) -> io::Result<u64> {
        let data = json!({
            "turn": turn,
            "step": step,
            "transformations": transformations,
            "prefixBindingMismatch": prefix_binding_mismatches(transformations),
        });
        self.emit(kind::REQUEST_TRANSFORMATIONS, data)
    }

    pub fn tool_call(
        &mut self,
        turn: u32,
        step: u32,
        call_id: &str,
        name: &str,
        arguments: &str,
    ) -> io::Result<u64> {
        let data = json!({ "turn": turn, "step": step, "callId": call_id,
            "name": name, "arguments": arguments });
        self.emit(kind::TOOL_CALL, data)
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
        let mut data = json!({ "turn": turn, "step": step, "message": value(message)?,
            "durationMs": duration_ms, "bytes": bytes });
        if let Some(path) = spilled {
            data["spilled"] = json!(path);
        }
        self.emit(kind::TOOL_RESULT, data)
    }

    /// `compaction` with the data built by the agent (PLAN.md D38).
    pub fn compaction(&mut self, data: Value) -> io::Result<u64> {
        self.emit(kind::COMPACTION, data)
    }

    pub fn step_end(&mut self, turn: u32, step: u32, duration_ms: u64) -> io::Result<u64> {
        let data = json!({ "turn": turn, "step": step, "durationMs": duration_ms });
        self.emit(kind::STEP_END, data)
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
        let data = json!({ "turn": turn, "steps": steps, "usage": value(usage)?,
            "durationMs": duration_ms, "outcome": outcome });
        self.emit(kind::TURN_END, data)
    }
}
