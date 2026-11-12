//! Hooks around tool calls (PLAN.md D44): no file spawns nothing,
//! PreToolUse blocks and rewrites, PostToolUse replaces the output before
//! the budget, context lands at the tail, failed hooks are ignored.

mod agent_support;
mod hooks_support;

use std::sync::Arc;

use agent_support::{env, text_reply, tool_reply};
use fast_harness_core::agent::Conversation;
use fast_harness_core::config::layers::Layers;
use fast_harness_core::config::Config;
use fast_harness_core::events::{AgentEvent, TurnOutcome};
use fast_harness_core::http::ScriptedTransport;
use fast_harness_core::tools::hooks::Hooks;
use hooks_support::*;
use serde_json::{json, Value};

const CALL: &str = r#"{"text":"hi"}"#;

fn tool_then_text() -> Arc<ScriptedTransport> {
    Arc::new(ScriptedTransport::new(vec![
        tool_reply(&[("c1", "echo", CALL)]),
        text_reply("done"),
    ]))
}

/// The tool message of the second request.
fn tool_message(transport: &ScriptedTransport) -> Value {
    bodies(transport)[1]["messages"][2].clone()
}

#[test]
fn no_hooks_file_spawns_nothing() {
    let env = env("hooks-none");
    let (hooks, skipped) = Hooks::load(&Layers::user_only(&env.home));
    assert!(hooks.is_empty() && skipped.is_empty() && hooks.files.is_empty());
    let transport = tool_then_text();
    let agent = agent(&env, &transport, Config::default());
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let (outcome, _) = turn(&agent, &mut conv, "go");
    assert_eq!(outcome, TurnOutcome::Done);
    assert!(tool_message(&transport).to_string().contains("hi"));
    assert!(traced(&conv, "hook").is_empty() && traced(&conv, "hook/error").is_empty());
}

#[test]
fn pre_tool_use_blocks_the_call() {
    let env = env("hooks-block");
    let (seen, script) = capture(
        &env.root.0,
        "pre",
        Some(r#"{"decision":"block","reason":"no echo today"}"#),
    );
    let pre = entry(Some("read|ech?"), script, 20_000);
    let other = entry(Some("mcp__*"), answer(&env.root.0, "x", "{}"), 20_000);
    write_hooks(&env.home, json!({"PreToolUse": [pre, other]}));
    let transport = tool_then_text();
    let agent = agent(&env, &transport, Config::default());
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    turn(&agent, &mut conv, "go");
    let message = tool_message(&transport).to_string();
    assert!(
        message.contains("blocked by a hook: no echo today"),
        "{message}"
    );
    assert!(message.contains(r#""is_error":true"#), "{message}");
    let input = read_json(&seen);
    assert_eq!(input["event"], "PreToolUse");
    assert_eq!(input["toolName"], "echo");
    assert_eq!(input["arguments"]["text"], "hi");
    assert_eq!(input["sessionId"], json!(conv.id));
    let hooks = traced(&conv, "hook");
    assert_eq!(hooks.len(), 1, "the mcp__* entry does not match: {hooks:?}");
    assert_eq!(hooks[0]["event"], "PreToolUse");
    assert_eq!(hooks[0]["decision"], "block");
    assert!(hooks[0]["command"].as_str().unwrap().len() > 2);
    assert!(hooks[0]["ms"].is_u64());
}

#[test]
fn pre_tool_use_rewrites_the_arguments() {
    let env = env("hooks-rewrite");
    let script = answer(&env.root.0, "pre", r#"{"arguments":{"text":"rewritten"}}"#);
    write_hooks(
        &env.home,
        json!({"PreToolUse": [entry(None, script, 20_000)]}),
    );
    let transport = tool_then_text();
    let agent = agent(&env, &transport, Config::default());
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    turn(&agent, &mut conv, "go");
    let message = tool_message(&transport).to_string();
    assert!(message.contains("rewritten") && !message.contains("is_error\":true"));
    // The model's call stays as it was sent.
    let calls = traced(&conv, "tool/call");
    assert_eq!(calls[0]["arguments"], CALL);
}

#[test]
fn post_tool_use_replaces_the_output_before_the_budget() {
    let env = env("hooks-output");
    let long = "y".repeat(20_000);
    let (seen, script) = capture(
        &env.root.0,
        "post",
        Some(&json!({"output": long}).to_string()),
    );
    write_hooks(
        &env.home,
        json!({"PostToolUse": [entry(Some("echo"), script, 20_000)]}),
    );
    let transport = tool_then_text();
    let agent = agent(&env, &transport, Config::default());
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    turn(&agent, &mut conv, "go");
    let input = read_json(&seen);
    assert_eq!(
        (&input["event"], &input["output"]),
        (&json!("PostToolUse"), &json!("hi"))
    );
    let message = tool_message(&transport).to_string();
    assert!(
        message.contains("yyyy") && !message.contains(r#""hi""#),
        "{message}"
    );
    assert!(
        message.len() < 9_000,
        "budget applied after: {}",
        message.len()
    );
    assert!(message.contains("full output") || message.contains("output cut"));
}

#[test]
fn context_lands_at_the_tail_and_the_prefix_stays() {
    let env = env("hooks-context");
    let script = answer(&env.root.0, "post", r#"{"context":"remember the hook"}"#);
    write_hooks(
        &env.home,
        json!({"PostToolUse": [entry(None, script, 20_000)]}),
    );
    let transport = tool_then_text();
    let agent = agent(&env, &transport, Config::default());
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    turn(&agent, &mut conv, "go");
    let bodies = bodies(&transport);
    let (first, second) = (&bodies[0], &bodies[1]);
    assert_eq!(first["system"], second["system"]);
    assert_eq!(first["tools"], second["tools"]);
    let (old, new) = (
        first["messages"].as_array().unwrap(),
        second["messages"].as_array().unwrap(),
    );
    assert_eq!(&new[..old.len()], &old[..], "earlier messages unchanged");
    let tail = new.last().unwrap()["content"].as_array().unwrap();
    assert_eq!(tail[0]["type"], "tool_result");
    let last = tail.last().unwrap();
    assert_eq!(last["text"], "[hook context] remember the hook");
}

#[test]
fn a_timeout_and_bad_json_fail_open() {
    let env = env("hooks-fail-open");
    let slow = match cfg!(windows) {
        true => vec!["ping".into(), "-n".into(), "30".into(), "127.0.0.1".into()],
        false => vec!["sleep 30".to_string()],
    };
    let bad = answer(&env.root.0, "bad", "not json");
    let hooks = json!({"PreToolUse": [entry(None, slow, 300), entry(None, bad, 20_000)]});
    write_hooks(&env.home, hooks);
    let transport = tool_then_text();
    let agent = agent(&env, &transport, Config::default());
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let started = std::time::Instant::now();
    let (outcome, seen) = turn(&agent, &mut conv, "go");
    assert!(started.elapsed().as_secs() < 15);
    assert_eq!(outcome, TurnOutcome::Done);
    let message = tool_message(&transport).to_string();
    assert!(
        message.contains("hi") && !message.contains("is_error\":true"),
        "{message}"
    );
    let errors = traced(&conv, "hook/error");
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(errors[0]["error"].as_str().unwrap().contains("timed out"));
    assert!(errors[1]["error"].as_str().unwrap().contains("not JSON"));
    let notes = seen
        .iter()
        .filter(|e| matches!(e, AgentEvent::Error(t) if t.contains("ignored")));
    assert_eq!(notes.count(), 2);
}
