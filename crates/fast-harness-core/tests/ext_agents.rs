//! Agent and command files and `tools.preload` (PLAN.md D42).

mod agent_support;

use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_support::*;
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::app::{Runtime, Slash};
use fast_harness_core::config::index::ExtIndex;
use fast_harness_core::config::layers::Layers;
use fast_harness_core::events::TurnOutcome;
use fast_harness_core::http::{ScriptedTransport, Transport};
use fast_harness_core::tools::Extensions;
use serde_json::Value;

fn put(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

#[test]
fn an_agent_type_opens_the_child_task_and_keeps_the_parent_system() {
    let env = env("ext-agent");
    let reviewer = "---\nname: reviewer\ndescription: Reviews code\ntools: [read]\n---\nYou are a careful reviewer.\n";
    put(&env.home.join("agents").join("reviewer.md"), reviewer);
    let ext = Extensions::new(ExtIndex::build(&Layers::user_only(&env.home)));
    let call = r#"{"prompt":"check a.txt","agent":"reviewer"}"#;
    let responses = vec![
        tool_reply(&[("t0", "find_tools", r#"{"names":["subagent"]}"#)]),
        tool_reply(&[("t1", "subagent", call)]),
        text_reply("looks fine"),
        text_reply("parent done"),
    ];
    let transport = Arc::new(ScriptedTransport::new(responses));
    let mut parts = parts(&transport, key_auth(), &env.cwd);
    parts.extensions = Arc::new(ext);
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let agent = Agent::with_subagent(parts, &env.home, &conv.id);
    let find = &agent.tools.specs()[0];
    assert!(find
        .description
        .contains("Agent types for the subagent tool's `agent`: reviewer."));
    let outcome = agent.run_turn(&mut conv, "go", &AtomicBool::new(false), &mut |_| {});
    assert_eq!(outcome, TurnOutcome::Done);
    let requests = transport.requests();
    assert_eq!(requests.len(), 4);
    let parent: Value = serde_json::from_str(&requests[0].body).unwrap();
    let child: Value = serde_json::from_str(&requests[2].body).unwrap();
    assert_eq!(parent["system"], child["system"], "same system prompt");
    let first = child["messages"][0]["content"].to_string();
    assert!(first.contains("You are a careful reviewer.") && first.contains("check a.txt"));
    assert!(
        requests[2].body.contains("preload_"),
        "the type's tools are preloaded"
    );
}

#[test]
fn a_command_file_expands_with_its_arguments() {
    let env = env("ext-command-md");
    let body = "---\ndescription: Review a path\n---\nReview $ARGUMENTS carefully.";
    put(&env.home.join("commands").join("review.md"), body);
    put(&env.home.join("commands").join("plain.md"), "Say hi.");
    let mut rt = Runtime::load_at(env.home.clone()).unwrap();
    rt.use_project(&env.cwd).unwrap();
    let expanded = Slash::Send("Review src/lib.rs carefully.".into());
    assert_eq!(rt.slash("/review src/lib.rs"), expanded);
    assert_eq!(
        rt.slash("/plain more"),
        Slash::Send("Say hi.\n\nmore".into())
    );
    assert_eq!(rt.slash("hello"), Slash::Send("hello".into()));
    let Slash::Help(help) = rt.slash("/help") else {
        panic!("help")
    };
    assert!(help.contains("/review: Review a path") && help.contains("/compact"));
}

#[test]
fn preload_lets_the_first_call_use_a_tool_without_a_find() {
    let env = env("ext-preload");
    put(
        &env.home.join("config.json"),
        r#"{"model": "local/m", "tools": {"preload": ["read"]}}"#,
    );
    let provider = r#"{"id": "local", "api": "anthropic-messages", "baseUrl": "http://127.0.0.1:9", "models": [{"id": "m"}]}"#;
    put(&env.home.join("providers").join("local.json"), provider);
    fs::write(env.cwd.join("a.txt"), "alpha").unwrap();
    let mut rt = Runtime::load_at(env.home.clone()).unwrap();
    rt.use_project(&env.cwd).unwrap();
    let mut conv = rt.open_conversation(&env.cwd, None, false).unwrap();
    let (mut agent, _) = rt.agent(&env.cwd, &conv).unwrap();
    assert_eq!(agent.preload, vec!["read".to_string()]);
    let responses = vec![
        tool_reply(&[("c1", "read", r#"{"path":"a.txt"}"#)]),
        text_reply("done"),
    ];
    let transport = Arc::new(ScriptedTransport::new(responses));
    let scripted: Arc<dyn Transport> = transport.clone();
    agent.parts.transport = scripted;
    let outcome = agent.run_turn(&mut conv, "read it", &AtomicBool::new(false), &mut |_| {});
    assert_eq!(outcome, TurnOutcome::Done);
    let requests = transport.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].body.contains("preload_"));
    assert!(
        !requests[1].body.contains("not loaded yet"),
        "read ran at once"
    );
    assert!(requests[1].body.contains("alpha"));
}
