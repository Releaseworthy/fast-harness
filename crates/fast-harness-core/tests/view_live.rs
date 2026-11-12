//! A running tool's live output tail (PLAN.md D58): shown while it runs,
//! gone after its result, so a live transcript still equals its replay.

use fast_harness_core::events::AgentEvent;
use fast_harness_core::trace::TraceEvent;
use fast_harness_core::types::{Block, Message, Role};
use fast_harness_core::view::{Item, Transcript, LIVE_LINES};
use serde_json::json;

const ARGS: &str = r#"{"command":"Write-Output one"}"#;

fn progress(id: &str, chunk: &str) -> AgentEvent {
    AgentEvent::ToolProgress {
        id: id.into(),
        chunk: chunk.into(),
    }
}

fn result() -> AgentEvent {
    AgentEvent::ToolResult {
        id: "c1".into(),
        content: "one\ntwo".into(),
        is_error: false,
        duration_ms: 40,
    }
}

/// One turn with a shell call; `chunks` arrive between call and result.
fn live(chunks: &[&str], with_result: bool) -> Transcript {
    let mut view = Transcript::new();
    view.apply(&AgentEvent::TurnStart { turn: 1 });
    view.push_user("run it");
    view.apply(&AgentEvent::StepStart { turn: 1, step: 1 });
    view.apply(&AgentEvent::ToolCall {
        id: "c1".into(),
        name: "shell".into(),
        arguments: ARGS.into(),
    });
    for chunk in chunks {
        view.apply(&progress("c1", chunk));
    }
    if with_result {
        view.apply(&result());
    }
    view
}

fn tail(view: &Transcript) -> Vec<String> {
    view.items
        .iter()
        .find_map(|item| match item {
            Item::Tool { live, .. } => live.as_ref(),
            _ => None,
        })
        .map(|tail| tail.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

#[test]
fn chunks_become_the_tail_of_the_running_tool() {
    assert_eq!(tail(&live(&["one", "\r\n", "two"], false)), ["one", "two"]);
    assert_eq!(tail(&live(&["5%\r", "50%\r", "99%"], false)), ["99%"]);
    let colour = ["\u{1b}[1;31merror\u{1b}[0m: x\u{1b}]0;t\u{7}"];
    assert_eq!(tail(&live(&colour, false)), ["error: x"]);
    let many: Vec<String> = (0..20).map(|i| format!("l{i}\n")).collect();
    let refs: Vec<&str> = many.iter().map(String::as_str).collect();
    let kept = tail(&live(&refs, false));
    assert_eq!(kept.len(), LIVE_LINES);
    assert_eq!((kept[0].as_str(), kept[7].as_str()), ("l12", "l19"));
}

#[test]
fn unknown_and_finished_calls_ignore_progress() {
    let mut view = live(&[], true);
    let before = view.items.clone();
    view.apply(&progress("c1", "late"));
    view.apply(&progress("nope", "stray"));
    assert_eq!(view.items, before);
}

#[test]
fn the_result_drops_the_tail() {
    let with = live(&["one", "\r\n", "two", "\u{1b}[K\r"], true);
    let without = live(&[], true);
    assert_eq!(with.items, without.items);
    assert_eq!(with.state, without.state);
    assert!(tail(&with).is_empty());
}

fn event(seq: u64, kind: &str, data: serde_json::Value) -> TraceEvent {
    TraceEvent {
        kind: kind.into(),
        seq,
        time: 1000 + seq,
        data,
    }
}

/// The trace of the same turn: progress is never logged.
fn trace() -> Vec<TraceEvent> {
    let call = Block::ToolCall {
        id: "c1".into(),
        name: "shell".into(),
        arguments: ARGS.into(),
        signature: None,
    };
    let answer = Message::new(Role::Assistant, vec![call]);
    let results = Message::new(
        Role::Tool,
        vec![Block::tool_result("c1", "one\ntwo", false)],
    );
    vec![
        event(0, "turn/start", json!({"turn":1})),
        event(
            1,
            "user/message",
            json!({"turn":1,"message":Message::user("run it")}),
        ),
        event(2, "step/start", json!({"turn":1,"step":1})),
        event(
            3,
            "assistant/message",
            json!({"turn":1,"step":1,"message":answer}),
        ),
        event(
            4,
            "tool/call",
            json!({"turn":1,"step":1,"callId":"c1","name":"shell","arguments":ARGS}),
        ),
        event(
            5,
            "tool/result",
            json!({"turn":1,"step":1,"message":results,"durationMs":40}),
        ),
    ]
}

fn tools(view: &Transcript) -> Vec<&Item> {
    let is_tool = |item: &&Item| matches!(item, Item::Tool { .. });
    view.items.iter().filter(is_tool).collect()
}

#[test]
fn a_finished_call_equals_its_replay() {
    let live = live(&["one", "\r\n", "two", "\r\n"], true);
    let replay = Transcript::from_trace(&trace());
    assert_eq!(tools(&live), tools(&replay));
    assert_eq!(tools(&live).len(), 1);
}
