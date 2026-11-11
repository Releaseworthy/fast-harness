//! One typed method per event of the trace format (PLAN.md), each producing
//! exactly the data keys the format shows.

use crate::llm::{Timing, ToolSpec};
use crate::trace::TraceWriter;
use crate::types::{Message, StopReason, Usage};
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
    pub const TOOL_CALL: &str = "tool/call";
    pub const TOOL_RESULT: &str = "tool/result";
    pub const STEP_END: &str = "step/end";
    pub const TURN_END: &str = "turn/end";
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
    /// Wire capture name of this attempt, such as `t1-s1-a1`.
    pub wire: &'a str,
}

#[derive(Serialize)]
struct AssistantData<'a> {
    turn: u32,
    step: u32,
    message: &'a Message,
    usage: &'a Usage,
    stop: StopReason,
    timing: &'a Timing,
    account: &'a str,
}

fn value(data: impl Serialize) -> io::Result<Value> {
    Ok(serde_json::to_value(data)?)
}

impl TraceWriter {
    pub fn turn_start(&mut self, turn: u32) -> io::Result<u64> {
        self.emit(kind::TURN_START, json!({ "turn": turn }))
    }

    pub fn user_message(&mut self, turn: u32, message: &Message) -> io::Result<u64> {
        let data = json!({ "turn": turn, "message": value(message)? });
        self.emit(kind::USER_MESSAGE, data)
    }

    pub fn step_start(&mut self, turn: u32, step: u32) -> io::Result<u64> {
        self.emit(kind::STEP_START, json!({ "turn": turn, "step": step }))
    }

    /// Written only when the system prompt or the tool list changes.
    pub fn request_header(&mut self, system: &str, tools: &[ToolSpec]) -> io::Result<u64> {
        let tools: Vec<Value> = tools
            .iter()
            .map(|t| json!({ "name": t.name, "description": t.description, "schema": t.schema }))
            .collect();
        self.emit(
            kind::REQUEST_HEADER,
            json!({ "system": system, "tools": tools }),
        )
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
    ) -> io::Result<u64> {
        let data = AssistantData {
            turn,
            step,
            message,
            usage,
            stop,
            timing,
            account,
        };
        self.emit(kind::ASSISTANT_MESSAGE, value(data)?)
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
        let data = json!({ "turn": turn, "step": step, "message": value(message)?,
            "durationMs": duration_ms });
        self.emit(kind::TOOL_RESULT, data)
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
