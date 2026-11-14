//! Append-only bodies: each Claude request repeats the previous one byte for
//! byte (`cache_control` aside) and only adds at the end, within a turn and
//! at the next turn. A thinking block is bound to everything sent before it
//! (<https://platform.claude.com/docs/en/build-with-claude/preserved-thinking/prefix-checks>,
//! read 2026-11-15): a trailing effort message that moved on every call made
//! the API drop every earlier thinking block (`prefix_binding_mismatch`) and
//! miss the prompt cache at each new user turn.

use std::sync::atomic::AtomicBool;

use serde_json::{json, Value};

use super::super::live::LiveModels;
use super::super::models;
use super::build;
use crate::llm::{ChatRequest, ToolSpec};
use crate::types::{Block, Message, Role};

const MODEL: &str = "claude-opus-5-5";

fn assistant(thinking: &str, tail: Block) -> Message {
    let thinking = Block::Thinking {
        text: thinking.into(),
        signature: Some(format!("sig-{thinking}")),
    };
    Message::new(Role::Assistant, vec![thinking, tail]).with_model(MODEL)
}

fn call(id: &str) -> Block {
    Block::ToolCall {
        id: id.into(),
        name: "read".into(),
        arguments: r#"{"path":"a.txt"}"#.into(),
        signature: None,
    }
}

/// The history after each model call of two turns: turn 1 has two steps
/// (a tool call, then an answer), turn 2 starts with a new user message.
fn histories() -> Vec<Vec<Message>> {
    let mut history = vec![Message::user("read a.txt")];
    let mut out = vec![history.clone()];
    history.push(assistant("plan", call("toolu_1")));
    history.push(Message::new(
        Role::Tool,
        vec![Block::tool_result("toolu_1", "hello", false)],
    ));
    out.push(history.clone());
    history.push(assistant("answer", Block::text("It says hello.")));
    history.push(Message::user("thanks, now b.txt"));
    out.push(history);
    out
}

fn body(messages: &[Message], effort: Option<&str>) -> Value {
    let tools = [ToolSpec::new(
        "read",
        "Reads a file.",
        json!({"type": "object"}),
    )];
    let cancel = AtomicBool::new(false);
    let mut req = ChatRequest::new(MODEL, "Be brief.", messages, &tools, 4096, &cancel);
    req.effort = effort;
    let spec = models::find(MODEL);
    build(&req, spec, &LiveModels::default(), false).0
}

fn strip_cache(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("cache_control");
            map.values_mut().for_each(strip_cache);
        }
        Value::Array(list) => list.iter_mut().for_each(strip_cache),
        _ => {}
    }
}

/// The serialized prompt (`tools`, `system`, then each message) without
/// `cache_control`, and the index of the message holding the last breakpoint.
fn prompt(body: &Value) -> (String, Vec<String>, usize) {
    let messages = body["messages"].as_array().cloned().unwrap_or_default();
    let breakpoint = messages
        .iter()
        .rposition(|m| m.to_string().contains("cache_control"))
        .expect("a message breakpoint");
    let mut body = body.clone();
    strip_cache(&mut body);
    let head = format!("{}{}", body["tools"], body["system"]);
    let list = body["messages"].as_array().unwrap();
    (
        head,
        list.iter().map(Value::to_string).collect(),
        breakpoint,
    )
}

fn assert_append_only(effort: Option<&str>) {
    let bodies: Vec<Value> = histories().iter().map(|h| body(h, effort)).collect();
    for pair in bodies.windows(2) {
        let (head, before, mark) = prompt(&pair[0]);
        let (next_head, after, _) = prompt(&pair[1]);
        assert_eq!(head, next_head, "tools and system stay the same");
        let up_to_mark = before[..=mark].join(",");
        assert!(after.join(",").starts_with(&up_to_mark), "{effort:?}");
        // Everything sent before (the prefix of each new thinking block),
        // not only the cached part, comes back unchanged and in place.
        assert_eq!(after[..before.len()], before[..], "{effort:?}");
    }
}

#[test]
fn every_request_extends_the_previous_one() {
    for effort in [None, Some("high"), Some("low"), Some("max")] {
        assert_append_only(effort);
    }
}

#[test]
fn effort_follows_each_user_turn_and_the_breakpoint_stays_on_user() {
    let history = histories().pop().unwrap();
    let messages = body(&history, Some("low"))["messages"].clone();
    let roles: Vec<&str> = messages
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    let want = [
        "user",
        "system",
        "assistant",
        "user",
        "assistant",
        "user",
        "system",
    ];
    assert_eq!(roles, want);
    assert_eq!(messages[1]["output_config"], json!({"effort": "low"}));
    assert!(messages[5]["content"][0].get("cache_control").is_some());
    // The model default `high` is the top-level value: no message at all.
    let plain = body(&history, None)["messages"].clone();
    assert!(!plain.to_string().contains("\"system\""), "{plain}");
}
