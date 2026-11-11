//! The find tool (PLAN.md D30): ranking, the agent gate, resume, and the
//! Claude request bodies over `ScriptedTransport` (bodies in tiny chunks).

mod agent_support;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_support::*;
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::auth::Failover;
use fast_harness_core::events::TurnOutcome;
use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use fast_harness_core::llm::find;
use fast_harness_core::tools::{self, find as finder};
use fast_harness_core::trace::detail;
use fast_harness_core::types::{Block, Message};
use fast_harness_core::view;
use serde_json::{json, Value};

fn results(messages: &[Message]) -> Vec<(String, bool)> {
    let blocks = messages.iter().flat_map(|m| m.content.iter());
    blocks
        .filter_map(|block| match block {
            Block::ToolResult {
                content, is_error, ..
            } => Some((content.clone(), *is_error)),
            _ => None,
        })
        .collect()
}

fn names(indexes: &[usize], specs: &[fast_harness_core::llm::ToolSpec]) -> Vec<String> {
    indexes.iter().map(|&i| specs[i].name.clone()).collect()
}

#[test]
fn search_ranks_names_then_parameters_then_descriptions() {
    let registry = tools::builtin();
    let others = &registry.specs()[1..];
    let ranked = |query: &str| names(&finder::search(query, others), others);
    assert_eq!(ranked("shell")[0], "shell");
    assert_eq!(ranked("run a command")[0], "shell");
    assert_eq!(ranked("edit")[0], "edit");
    assert_eq!(ranked("glob pattern")[..2], ["glob", "grep"]);
    assert!(ranked("zzzz").is_empty());
    let spec = &registry.specs()[0];
    assert_eq!(spec.name, find::FIND_TOOLS);
    assert!(spec
        .description
        .contains("Tools: read, write, edit, shell, grep, glob."));
    assert!(!spec.description.contains("\"properties\""));
}

#[test]
fn find_answers_names_query_unknown_and_caps_at_five() {
    let registry = tools::builtin();
    let others = &registry.specs()[1..];
    let out = finder::run(&json!({"names": ["Read", "edit", "nope"]}), others);
    assert_eq!(
        out.content,
        "Loaded: read, edit\nYou can call them now.\nUnknown: nope."
    );
    let out = finder::run(&json!({"query": "file path pattern text command"}), others);
    assert_eq!(find::loaded_names(&out.content).len(), finder::MAX_RESULTS);
    let out = finder::run(&json!({"query": "zzzz"}), others);
    assert!(out.content.starts_with("No tool matched. Tools: read,"));
    assert!(find::loaded_names(&out.content).is_empty());
    assert!(finder::run(&json!({}), others).is_error);
}

fn agent(transport: &Arc<ScriptedTransport>, env: &Env) -> Agent {
    let parts = parts(transport, key_auth(), &env.cwd);
    Agent::new(parts, Arc::new(tools::builtin()))
}

fn run(agent: &Agent, conv: &mut Conversation) -> TurnOutcome {
    agent.run_turn(conv, "go", &AtomicBool::new(false), &mut |_| {})
}

fn body(transport: &ScriptedTransport, index: usize) -> Value {
    serde_json::from_str(&transport.requests()[index].body).unwrap()
}

/// Find, then call the found tool; calls before the find fail.
fn find_then_read() -> Vec<ScriptedResponse> {
    vec![
        tool_reply(&[
            ("toolu_1", "find_tools", r#"{"names":["read","edit"]}"#),
            ("toolu_2", "read", r#"{"path":"a.txt"}"#),
            ("toolu_3", "nope", "{}"),
        ]),
        tool_reply(&[("toolu_4", "read", r#"{"path":"a.txt"}"#)]),
        text_reply("done"),
    ]
}

#[test]
fn a_turn_finds_then_calls_the_found_tool() {
    let env = env("find-turn");
    std::fs::write(env.cwd.join("a.txt"), "hello find").unwrap();
    let transport = Arc::new(ScriptedTransport::new(find_then_read()));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    assert_eq!(run(&agent(&transport, &env), &mut conv), TurnOutcome::Done);
    let out = results(&conv.messages);
    assert!(out[0].0.starts_with("Loaded: read, edit") && !out[0].1);
    assert!(
        out[1].1 && out[1].0.contains("not loaded yet"),
        "{:?}",
        out[1]
    );
    assert!(out[2].1 && out[2].0.contains("unknown tool `nope`"));
    assert!(!out[3].1 && out[3].0.contains("hello find"), "{:?}", out[3]);
    check_claude_bodies(&transport);
    let text = detail::format_text(&detail::entries(&events(&conv.log_path)));
    assert!(text.contains("Tool call find_tools: read, edit"), "{text}");
    assert!(text.contains("Loaded: read, edit"));
    let summary = view::tool_summary("find_tools", r#"{"names":["read","edit"]}"#);
    assert_eq!(summary, "read, edit");
    assert_eq!(
        view::tool_summary("find_tools", r#"{"query":"run"}"#),
        "\"run\""
    );
    drop(env);
}

/// The tools array never changes; only find_tools is loaded and carries
/// the cache breakpoint; the find result becomes tool_reference blocks.
fn check_claude_bodies(transport: &ScriptedTransport) {
    let (first, second, third) = (body(transport, 0), body(transport, 1), body(transport, 2));
    assert_eq!(first["tools"], second["tools"]);
    assert_eq!(first["tools"], third["tools"]);
    let tools = first["tools"].as_array().unwrap();
    assert_eq!(tools[0]["name"], "find_tools");
    assert_eq!(tools[0]["cache_control"], json!({"type": "ephemeral"}));
    assert!(tools[0].get("defer_loading").is_none());
    for tool in &tools[1..] {
        assert_eq!(tool["defer_loading"], json!(true), "{tool}");
        assert!(tool.get("cache_control").is_none());
    }
    let result = &second["messages"][2]["content"][0];
    assert_eq!(result["tool_use_id"], "toolu_1");
    assert_eq!(
        result["content"],
        json!([
            {"type": "tool_reference", "tool_name": "read"},
            {"type": "tool_reference", "tool_name": "edit"},
            {"type": "text", "text": "Loaded: read, edit\nYou can call them now."}
        ])
    );
    // The not-loaded error stays a plain string.
    assert!(second["messages"][2]["content"][1]["content"].is_string());
    assert_eq!(
        third["messages"][2]["content"][0]["content"],
        result["content"]
    );
}

#[test]
fn oauth_references_use_the_sent_tool_names() {
    let env = env("find-oauth");
    std::fs::write(env.cwd.join("a.txt"), "hello oauth").unwrap();
    let responses = vec![
        tool_reply(&[("toolu_1", "find_tools", r#"{"query":"read file"}"#)]),
        tool_reply(&[("toolu_2", "Read", r#"{"path":"a.txt"}"#)]),
        text_reply("done"),
    ];
    let transport = Arc::new(ScriptedTransport::new(responses));
    let auth = FakeAuth::new(vec!["acct-1"], Failover::Fail);
    let agent = Agent::new(
        parts(&transport, auth, &env.cwd),
        Arc::new(tools::builtin()),
    );
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    assert_eq!(run(&agent, &mut conv), TurnOutcome::Done);
    let out = results(&conv.messages);
    assert!(
        !out[1].1 && out[1].0.contains("hello oauth"),
        "{:?}",
        out[1]
    );
    let second = body(&transport, 1);
    let tools = second["tools"].as_array().unwrap();
    assert!(tools.iter().any(|t| t["name"] == "Read"));
    let content = &second["messages"][2]["content"][0]["content"];
    assert_eq!(
        content[0],
        json!({"type": "tool_reference", "tool_name": "Read"})
    );
    drop(env);
}

#[test]
fn resume_rebuilds_the_found_set() {
    let env = env("find-resume");
    std::fs::write(env.cwd.join("a.txt"), "hello resume").unwrap();
    let responses = vec![
        tool_reply(&[("toolu_1", "find_tools", r#"{"names":["read"]}"#)]),
        text_reply("found"),
    ];
    let transport = Arc::new(ScriptedTransport::new(responses));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    assert_eq!(run(&agent(&transport, &env), &mut conv), TurnOutcome::Done);
    let responses = vec![
        tool_reply(&[("toolu_2", "read", r#"{"path":"a.txt"}"#)]),
        text_reply("done"),
    ];
    let transport = Arc::new(ScriptedTransport::new(responses));
    let mut resumed = Conversation::resume(&env.home, &env.cwd, &conv.id).unwrap();
    assert_eq!(find::found(&resumed.messages), ["read"]);
    assert_eq!(
        run(&agent(&transport, &env), &mut resumed),
        TurnOutcome::Done
    );
    let out = results(&resumed.messages);
    assert!(
        !out[1].1 && out[1].0.contains("hello resume"),
        "{:?}",
        out[1]
    );
    let first = body(&transport, 0);
    let content = &first["messages"][2]["content"][0]["content"];
    assert_eq!(
        content[0],
        json!({"type": "tool_reference", "tool_name": "read"})
    );
    drop(env);
}
