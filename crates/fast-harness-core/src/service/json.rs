//! The JSON form of agent events (PLAN.md D48, docs/service.md): one object
//! per [`AgentEvent`] with a `type` field and camelCase keys, written one
//! per line by `fast-harness -p --json`. The last line is a `result`.

use serde_json::{json, Value};

use crate::events::{AgentEvent, TurnOutcome};
use crate::types::Usage;

/// Version of the event schema, on the `session` line; a breaking change
/// of any event bumps it.
pub const SCHEMA: u32 = 1;

/// `done`, `cancelled`, `failed`, `step_limit`, `truncated`,
/// `context_overflow` or `paused`.
pub fn outcome_name(outcome: TurnOutcome) -> &'static str {
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

/// The Agent Client Protocol stop reason of an outcome; ours that ACP has
/// no word for start with `_` as ACP asks for custom values.
/// <https://agentclientprotocol.com/protocol/v2/prompt-lifecycle.md#stop-reasons>
pub fn stop_reason(outcome: TurnOutcome) -> &'static str {
    match outcome {
        TurnOutcome::Done => "end_turn",
        TurnOutcome::Cancelled => "cancelled",
        TurnOutcome::Failed => "error",
        TurnOutcome::StepLimit => "max_turn_requests",
        TurnOutcome::Truncated => "max_tokens",
        TurnOutcome::ContextOverflow => "_context_overflow",
        TurnOutcome::Paused => "_paused",
    }
}

/// The first line: the session the events belong to.
pub fn session_line(session: &str, cwd: &str) -> Value {
    json!({"type": "session", "schema": SCHEMA, "session": session, "cwd": cwd})
}

/// The last line: the outcome of the last turn and the usage of every turn.
pub fn result_line(session: &str, outcome: TurnOutcome, usage: &Usage, duration_ms: u64) -> Value {
    json!({"type": "result", "session": session, "outcome": outcome_name(outcome),
        "stopReason": stop_reason(outcome), "usage": usage, "durationMs": duration_ms})
}

/// The JSON object of one event.
pub fn event(event: &AgentEvent) -> Value {
    match event {
        AgentEvent::UserInput { text, steer } => {
            json!({"type": "user_input", "text": text, "steer": steer})
        }
        AgentEvent::TurnStart { turn } => json!({"type": "turn_start", "turn": turn}),
        AgentEvent::StepStart { turn, step } => {
            json!({"type": "step_start", "turn": turn, "step": step})
        }
        AgentEvent::Request {
            model,
            account,
            attempt,
        } => json!({"type": "request", "model": model, "account": account, "attempt": attempt}),
        AgentEvent::TextDelta(text) => json!({"type": "text", "text": text}),
        AgentEvent::ThinkingDelta(text) => json!({"type": "thinking", "text": text}),
        AgentEvent::ToolCallStart { id, name } => {
            json!({"type": "tool_call_start", "id": id, "name": name})
        }
        AgentEvent::ToolArgsDelta { id, chunk } => {
            json!({"type": "tool_args", "id": id, "chunk": chunk})
        }
        AgentEvent::ToolCall {
            id,
            name,
            arguments,
        } => json!({"type": "tool_call", "id": id, "name": name, "arguments": arguments}),
        AgentEvent::ToolResult {
            id,
            content,
            is_error,
            duration_ms,
        } => json!({"type": "tool_result", "id": id, "content": content,
            "isError": is_error, "durationMs": duration_ms}),
        other => tail(other),
    }
}

/// The events after the tool events.
fn tail(event: &AgentEvent) -> Value {
    match event {
        AgentEvent::Retry {
            attempt,
            delay_ms,
            error,
        } => json!({"type": "retry", "attempt": attempt, "delayMs": delay_ms, "error": error}),
        AgentEvent::AccountSwitch { from, to, reason } => {
            json!({"type": "account_switch", "from": from, "to": to, "reason": reason})
        }
        AgentEvent::UsageUpdate { account } => json!({"type": "usage_update", "account": account}),
        AgentEvent::Subagent { session, running } => {
            json!({"type": "subagent", "session": session, "running": running})
        }
        AgentEvent::SubagentProgress { session, line } => {
            json!({"type": "subagent_progress", "session": session, "line": line})
        }
        AgentEvent::Compaction {
            summary,
            before,
            after,
            auto,
        } => json!({"type": "compaction", "summary": summary, "before": before,
            "after": after, "auto": auto}),
        AgentEvent::TurnEnd {
            turn,
            usage,
            duration_ms,
            outcome,
        } => json!({"type": "turn_end", "turn": turn, "usage": usage,
            "durationMs": duration_ms, "outcome": outcome_name(*outcome)}),
        AgentEvent::Error(message) => json!({"type": "error", "message": message}),
        other => step_end(other),
    }
}

fn step_end(event: &AgentEvent) -> Value {
    match event {
        AgentEvent::StepEnd {
            turn,
            step,
            usage,
            context,
            window,
            timing,
            stop,
        } => json!({"type": "step_end", "turn": turn, "step": step, "usage": usage,
            "context": context, "window": window, "timing": timing, "stop": stop}),
        // Every variant is matched above; a new one shows up as `unknown`
        // until it gets its own line here (the schema test lists them all).
        _ => json!({"type": "unknown"}),
    }
}
