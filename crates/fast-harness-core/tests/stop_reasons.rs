//! Stop reasons (PLAN.md D36): `max_tokens` ends the turn `Truncated` and
//! continues once when a tool call was cut; `pause_turn` continues;
//! `model_context_window_exceeded` and a 400 "prompt is too long" end it
//! `ContextOverflow` without a retry.

mod agent_support;

use std::sync::atomic::AtomicBool;

use agent_support::{echo_registry, env, error_reply, events, key_auth, parts, text_reply};
use fast_harness_core::agent::Agent;
use fast_harness_core::agent::Conversation;
use fast_harness_core::events::{AgentEvent, TurnOutcome};
use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use fast_harness_core::trace::kind;
use serde_json::{json, Value};
use std::sync::Arc;

fn sse(events: &[Value]) -> String {
    let mut out = String::new();
    for event in events {
        let name = event["type"].as_str().unwrap();
        out.push_str(&format!("event: {name}\ndata: {event}\n\n"));
    }
    out
}

/// A reply with `blocks` (`text` or `(id, partial json)` tool calls) and `reason`.
fn reply(text: Option<&str>, call: Option<(&str, &str)>, reason: &str) -> ScriptedResponse {
    let mut events = vec![
        json!({"type": "message_start", "message": {"id": "m", "type": "message",
        "role": "assistant", "content": [], "model": "claude-opus-5-5",
        "usage": {"input_tokens": 10, "output_tokens": 1}}}),
    ];
    if let Some(text) = text {
        events.push(json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}));
        events.push(json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": text}}));
        events.push(json!({"type": "content_block_stop", "index": 0}));
    }
    if let Some((id, args)) = call {
        events.push(json!({"type": "content_block_start", "index": 1,
            "content_block": {"type": "tool_use", "id": id, "name": "echo", "input": {}}}));
        events.push(json!({"type": "content_block_delta", "index": 1,
            "delta": {"type": "input_json_delta", "partial_json": args}}));
        events.push(json!({"type": "content_block_stop", "index": 1}));
    }
    events.push(json!({"type": "message_delta", "delta": {"stop_reason": reason}, "usage": {"output_tokens": 5}}));
    events.push(json!({"type": "message_stop"}));
    ScriptedResponse::new(200, &sse(&events), 9)
}

/// Runs one turn on `responses`: the outcome, the errors shown, the request
/// bodies and the logged `turn/end` outcome word.
fn run(
    name: &str,
    responses: Vec<ScriptedResponse>,
) -> (TurnOutcome, Vec<String>, Vec<Value>, String) {
    let env = env(name);
    let transport = Arc::new(ScriptedTransport::new(responses));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let agent = Agent::new(parts(&transport, key_auth(), &env.cwd), echo_registry());
    let mut errors = Vec::new();
    let outcome = agent.run_turn(&mut conv, "go", &AtomicBool::new(false), &mut |event| {
        if let AgentEvent::Error(message) = event {
            errors.push(message);
        }
    });
    let bodies = transport
        .requests()
        .iter()
        .map(|r| serde_json::from_str(&r.body).unwrap())
        .collect();
    let logged = events(&conv.log_path);
    let end = logged.iter().find(|e| e.kind == kind::TURN_END).unwrap();
    (
        outcome,
        errors,
        bodies,
        end.data["outcome"].as_str().unwrap().to_string(),
    )
}

#[test]
fn max_tokens_ends_the_turn_truncated() {
    let (outcome, errors, bodies, word) = run(
        "stop-trunc",
        vec![reply(Some("half an ans"), None, "max_tokens")],
    );
    assert_eq!(
        (outcome, word.as_str(), bodies.len()),
        (TurnOutcome::Truncated, "truncated", 1)
    );
    assert!(errors.is_empty());
}

#[test]
fn a_cut_tool_call_continues_once() {
    let cut = || reply(None, Some(("toolu_1", r#"{"text":"ha"#)), "max_tokens");
    let (outcome, _, bodies, _) = run("stop-cut", vec![cut(), text_reply("done")]);
    assert_eq!((outcome, bodies.len()), (TurnOutcome::Done, 2));
    let results = last_message(&bodies[1]).to_string();
    assert!(
        results.contains("toolu_1") && results.contains("not valid JSON"),
        "{results}"
    );
    let cut2 = || reply(None, Some(("toolu_2", r#"{"te"#)), "max_tokens");
    let (outcome, _, bodies, _) = run("stop-cut-twice", vec![cut(), cut2(), text_reply("never")]);
    assert_eq!(
        (outcome, bodies.len()),
        (TurnOutcome::Truncated, 2),
        "only once per turn"
    );
}

#[test]
fn pause_turn_continues_the_turn() {
    let (outcome, _, bodies, _) = run(
        "stop-pause",
        vec![
            reply(Some("working"), None, "pause_turn"),
            text_reply("done"),
        ],
    );
    assert_eq!((outcome, bodies.len()), (TurnOutcome::Done, 2));
    let last = last_message(&bodies[1]).clone();
    assert_eq!(
        last["role"], "assistant",
        "the paused message goes back as is"
    );
}

/// A full window compacts once (PLAN.md D38) and goes on; when the summary
/// request fails too, the turn ends as an overflow.
#[test]
fn context_window_stop_compacts_then_is_an_overflow() {
    let full = || reply(Some("x"), None, "model_context_window_exceeded");
    let responses = vec![full(), text_reply("summary"), text_reply("done")];
    let (outcome, errors, bodies, _) = run("stop-window-ok", responses);
    assert_eq!(
        (outcome, bodies.len()),
        (TurnOutcome::Done, 3),
        "{errors:?}"
    );
    let failed = error_reply(400, "invalid_request_error", "bad field");
    let (outcome, errors, _, word) = run("stop-window", vec![full(), failed]);
    assert_eq!(
        (outcome, word.as_str()),
        (TurnOutcome::ContextOverflow, "context_overflow")
    );
    assert!(errors.last().unwrap().contains("context window"));
}

/// "prompt is too long" is not retried as it is: one compaction, then the
/// step again; a summary request that overflows too ends the turn.
#[test]
fn prompt_too_long_is_an_overflow_without_retry() {
    for status in [400, 413] {
        let too_long = || {
            error_reply(
                status,
                "invalid_request_error",
                "prompt is too long: 1000001 tokens > 1000000 maximum",
            )
        };
        let responses = vec![too_long(), too_long(), text_reply("never")];
        let (outcome, errors, bodies, _) = run("stop-long", responses);
        assert_eq!(
            (outcome, bodies.len()),
            (TurnOutcome::ContextOverflow, 2),
            "{status}"
        );
        let expected = format!("ContextOverflow (HTTP {status})");
        assert!(errors[0].contains(&expected), "{errors:?}");
        assert!(errors.last().unwrap().contains("context window"));
    }
    let other = error_reply(400, "invalid_request_error", "bad field");
    let (outcome, ..) = run("stop-bad", vec![other]);
    assert_eq!(outcome, TurnOutcome::Failed);
}

/// The last message of `body` before the trailing Claude effort entry.
fn last_message(body: &Value) -> &Value {
    let messages = body["messages"].as_array().unwrap();
    messages
        .iter()
        .rev()
        .find(|m| m["role"] != "system")
        .unwrap()
}
