//! The `fast-harness -p --json` schema (PLAN.md D48, docs/service.md): one
//! object per event with a stable `type` and camelCase keys.

use fast_harness_core::events::{AgentEvent, TurnOutcome};
use fast_harness_core::llm::Timing;
use fast_harness_core::service::json::{event, result_line, session_line, SCHEMA};
use fast_harness_core::types::{StopReason, Usage};
use serde_json::{json, Value};

fn usage() -> Usage {
    Usage::new(10, 5, 2, 1)
}

fn every_event() -> Vec<AgentEvent> {
    let (id, text) = (String::from("toolu_1"), String::from("x"));
    vec![
        AgentEvent::UserInput {
            text: text.clone(),
            steer: true,
        },
        AgentEvent::TurnStart { turn: 1 },
        AgentEvent::StepStart { turn: 1, step: 1 },
        AgentEvent::Request {
            model: "m".into(),
            account: "a".into(),
            attempt: 1,
        },
        AgentEvent::TextDelta(text.clone()),
        AgentEvent::ThinkingDelta(text.clone()),
        AgentEvent::ToolCallStart {
            id: id.clone(),
            name: "read".into(),
        },
        AgentEvent::ToolArgsDelta {
            id: id.clone(),
            chunk: "{".into(),
        },
        AgentEvent::ToolCall {
            id: id.clone(),
            name: "read".into(),
            arguments: "{}".into(),
        },
        AgentEvent::ToolResult {
            id,
            content: text.clone(),
            is_error: false,
            duration_ms: 3,
        },
        AgentEvent::Retry {
            attempt: 1,
            delay_ms: 500,
            error: text.clone(),
        },
        AgentEvent::AccountSwitch {
            from: "a".into(),
            to: "b".into(),
            reason: text.clone(),
        },
        AgentEvent::UsageUpdate {
            account: "a".into(),
        },
        AgentEvent::Subagent {
            session: "s".into(),
            running: true,
        },
        AgentEvent::SubagentProgress {
            session: "s".into(),
            line: text.clone(),
        },
        AgentEvent::StepEnd {
            turn: 1,
            step: 1,
            usage: usage(),
            context: 18,
            window: 200_000,
            timing: Timing::default(),
            stop: StopReason::End,
        },
        AgentEvent::Compaction {
            summary: text.clone(),
            before: 9,
            after: 3,
            auto: false,
        },
        AgentEvent::TurnEnd {
            turn: 1,
            usage: usage(),
            duration_ms: 7,
            outcome: TurnOutcome::Done,
        },
        AgentEvent::Error(text),
    ]
}

#[test]
fn every_event_has_its_own_type() {
    let types: Vec<Value> = every_event()
        .iter()
        .map(|e| event(e)["type"].clone())
        .collect();
    let want = json!([
        "user_input",
        "turn_start",
        "step_start",
        "request",
        "text",
        "thinking",
        "tool_call_start",
        "tool_args",
        "tool_call",
        "tool_result",
        "retry",
        "account_switch",
        "usage_update",
        "subagent",
        "subagent_progress",
        "step_end",
        "compaction",
        "turn_end",
        "error"
    ]);
    assert_eq!(Value::Array(types), want);
}

#[test]
fn events_carry_camel_case_fields() {
    let all = every_event();
    let result = event(&all[9]);
    assert_eq!(
        result,
        json!({"type": "tool_result", "id": "toolu_1", "content": "x", "isError": false,
            "durationMs": 3})
    );
    let step = event(&all[15]);
    assert_eq!(step["usage"]["inputTokens"], 10);
    assert_eq!(step["usage"]["cacheReadTokens"], 2);
    assert_eq!(step["stop"], "end");
    assert_eq!(step["window"], 200_000);
    assert_eq!(event(&all[17])["outcome"], "done");
    assert_eq!(event(&all[18]), json!({"type": "error", "message": "x"}));
}

#[test]
fn first_and_last_lines() {
    let first = session_line("s1", "C:\\work");
    assert_eq!(
        first,
        json!({"type": "session", "schema": SCHEMA, "session": "s1", "cwd": "C:\\work"})
    );
    let last = result_line("s1", TurnOutcome::Cancelled, &usage(), 42);
    assert_eq!(last["type"], "result");
    assert_eq!(last["outcome"], "cancelled");
    assert_eq!(last["stopReason"], "cancelled");
    assert_eq!(last["usage"]["outputTokens"], 5);
    assert_eq!(last["durationMs"], 42);
}
