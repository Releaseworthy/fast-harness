//! Phase 13 compaction (PLAN.md D38): thresholds per model, the automatic
//! compaction between steps and at turn start, the three-part history,
//! resume, overflow retry, the summary tool-call retry and `/compact`.

mod agent_support;

use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_support::*;
use fast_harness_core::agent::{Agent, AgentParts, Conversation, NO_TOOLS};
use fast_harness_core::config::compaction::{parse_trigger, ModelEntry, Trigger};
use fast_harness_core::config::Config;
use fast_harness_core::events::{AgentEvent, TurnOutcome};
use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use fast_harness_core::llm::Catalog;
use fast_harness_core::session::compacted::{CONTINUE, SUMMARY_PREFIX};
use fast_harness_core::tools;
use fast_harness_core::trace::kind;
use serde_json::{json, Value};

const SUMMARY: &str =
    "## Task\nfix the bug\n## Files\nsrc/a.rs\n## Durable facts\nuse pwsh on Windows";

fn summary_reply() -> ScriptedResponse {
    text_reply(SUMMARY)
}

fn overflow_reply() -> ScriptedResponse {
    let message = "prompt is too long: 1200000 tokens > 1000000 maximum";
    error_reply(400, "invalid_request_error", message)
}

fn bodies(transport: &ScriptedTransport) -> Vec<Value> {
    let requests = transport.requests();
    let parse = |body: &str| serde_json::from_str::<Value>(body).unwrap();
    requests
        .iter()
        .map(|request| parse(&request.body))
        .collect()
}

/// The messages of a Claude body without the trailing effort entry.
fn messages(body: &Value) -> Vec<Value> {
    let list = body["messages"].as_array().unwrap().iter();
    list.filter(|m| m["role"] != "system").cloned().collect()
}

fn first_text(message: &Value) -> String {
    message["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// Parts whose compaction threshold is 10 tokens (every step is above it).
fn small(transport: &Arc<ScriptedTransport>, cwd: &std::path::Path) -> AgentParts {
    let mut parts = parts(transport, key_auth(), cwd);
    parts.config.compaction.max_tokens = Some(10);
    parts
}

fn turn(agent: &Agent, conv: &mut Conversation, text: &str) -> (TurnOutcome, Vec<AgentEvent>) {
    let cancel = AtomicBool::new(false);
    let mut seen = Vec::new();
    let outcome = agent.run_turn(conv, text, &cancel, &mut |event| seen.push(event));
    (outcome, seen)
}

#[test]
fn thresholds_per_model_and_overrides() {
    let mut config = Config::default();
    let context_window = |model: &str| Catalog::default().context_window(model);
    assert_eq!(context_window("claude-opus-5-5"), 1_000_000);
    let defaults = config.compaction_for("anthropic");
    assert_eq!(defaults.threshold(1_000_000, 32_000), 500_000);
    let haiku = context_window("claude-haiku-4-5");
    assert_eq!(defaults.threshold(haiku, 32_000), 100_000);
    // The safety bound: 64K - 16K reserve - 32K output is below 50%.
    assert_eq!(defaults.threshold(64_000, 32_000), 16_000);
    assert_eq!(defaults.threshold(0, 32_000), 500_000);
    let models: BTreeMap<String, ModelEntry> = serde_json::from_value(json!({
        "claude-haiku-4-5": {"compaction": {"trigger": "70%"}},
        "local/*": {"compaction": {"maxTokens": 120000}},
    }))
    .unwrap();
    config.models = models;
    config.model = "claude-haiku-4-5".into();
    let resolved = config.compaction_for("anthropic");
    assert_eq!(resolved.threshold(200_000, 32_000), 140_000);
    config.model = "local/gemma-4".into();
    let resolved = config.compaction_for("local");
    assert_eq!(resolved.threshold(1_000_000, 32_000), 120_000);
    assert_eq!(parse_trigger("150k"), Some(Trigger::Tokens(150_000)));
    assert_eq!(parse_trigger("1.5M"), Some(Trigger::Tokens(1_500_000)));
    assert_eq!(parse_trigger(" 50% "), Some(Trigger::Percent(50)));
    assert_eq!(parse_trigger("soon"), None);
}

#[test]
fn config_json_reads_the_compaction_keys() {
    let env = env("compact-config");
    let text = r#"{"compaction": {"auto": false, "trigger": 150000, "model": "claude-haiku-4-5"}}"#;
    std::fs::write(env.home.join("config.json"), text).unwrap();
    let config = Config::load_with(&env.home, &|_| None).unwrap();
    let resolved = config.compaction_for("anthropic");
    assert!(!resolved.auto);
    assert_eq!(resolved.trigger, Trigger::Tokens(150_000));
    assert_eq!(resolved.model.as_deref(), Some("claude-haiku-4-5"));
    assert_eq!(resolved.effort.as_deref(), Some("low"));
}

#[test]
fn compacts_between_steps_with_the_three_part_history_and_resumes() {
    let env = env("compact-mid");
    let transport = Arc::new(ScriptedTransport::new(vec![
        tool_reply(&[("t1", "find_tools", r#"{"names":["read"]}"#)]),
        summary_reply(),
        text_reply("done"),
    ]));
    let agent = Agent::new(small(&transport, &env.cwd), Arc::new(tools::builtin()));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let (outcome, seen) = turn(&agent, &mut conv, "go");
    assert_eq!(outcome, TurnOutcome::Done);
    let bodies = bodies(&transport);
    assert_eq!(bodies.len(), 3);
    // The summary request: the same prefix, the old messages, one request.
    assert_eq!(bodies[1]["system"], bodies[0]["system"]);
    assert_eq!(bodies[1]["tools"], bodies[0]["tools"]);
    assert!(bodies[1].get("tool_choice").is_none());
    let asked = messages(&bodies[1]);
    assert_eq!(asked.len(), 4);
    assert!(first_text(&asked[3]).starts_with("[compaction] Summarize"));
    // The next step: summary, the found tools again, continue.
    let next = messages(&bodies[2]);
    assert_eq!(next.len(), 4, "{next:?}");
    assert!(first_text(&next[0]).starts_with(SUMMARY_PREFIX));
    assert!(first_text(&next[0]).contains("fix the bug"));
    assert_eq!(next[1]["content"][0]["name"], "find_tools");
    let reference = json!({"type": "tool_reference", "tool_name": "read"});
    assert!(next[2]["content"][0]["content"]
        .as_array()
        .unwrap()
        .contains(&reference));
    assert_eq!(first_text(&next[3]), CONTINUE);
    assert!(!bodies[2]["messages"].to_string().contains("\"thinking\""));
    let log = events(&conv.log_path);
    let event = first(&log, kind::COMPACTION);
    assert_eq!(event.data["loaded"], json!(["read"]));
    assert_eq!(event.data["facts"], "use pwsh on Windows");
    assert_eq!(
        (event.data["auto"].as_bool(), event.data["mid"].as_bool()),
        (Some(true), Some(true))
    );
    assert!(event.data["before"].as_u64().unwrap() > 10);
    assert!(seen
        .iter()
        .any(|e| matches!(e, AgentEvent::Compaction { auto: true, .. })));
    let streamed = |e: &AgentEvent| matches!(e, AgentEvent::TextDelta(t) if t.contains("fix"));
    assert!(!seen.iter().any(streamed), "the summary is not streamed");
    let id = conv.id.clone();
    drop(conv);
    let resumed = Conversation::resume(&env.home, &env.cwd, &id).unwrap();
    assert_eq!(resumed.messages.len(), 5);
    assert!(resumed.messages[0].text().starts_with(SUMMARY_PREFIX));
    assert_eq!(resumed.messages[4].text(), "done");
    assert!(resumed.last_context > 10 && !resumed.compacted);
}

#[test]
fn an_overflow_compacts_once_and_retries_the_step() {
    let env = env("compact-overflow");
    let transport = Arc::new(ScriptedTransport::new(vec![
        overflow_reply(),
        summary_reply(),
        text_reply("ok"),
    ]));
    let agent = Agent::new(parts(&transport, key_auth(), &env.cwd), echo_registry());
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let (outcome, seen) = turn(&agent, &mut conv, "go");
    assert_eq!(outcome, TurnOutcome::Done);
    assert!(!seen.iter().any(|e| matches!(e, AgentEvent::Error(_))));
    let bodies = bodies(&transport);
    let next = messages(&bodies[2]);
    assert_eq!(next.len(), 2);
    assert!(first_text(&next[0]).starts_with(SUMMARY_PREFIX));
    assert_eq!(first_text(&next[1]), CONTINUE);
}

#[test]
fn a_second_overflow_ends_the_turn() {
    let env = env("compact-overflow-twice");
    let transport = Arc::new(ScriptedTransport::new(vec![
        overflow_reply(),
        summary_reply(),
        overflow_reply(),
    ]));
    let agent = Agent::new(parts(&transport, key_auth(), &env.cwd), echo_registry());
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let (outcome, seen) = turn(&agent, &mut conv, "go");
    assert_eq!(outcome, TurnOutcome::ContextOverflow);
    let errors = seen.iter().filter(|e| matches!(e, AgentEvent::Error(_)));
    assert_eq!(errors.count(), 1);
}

#[test]
fn a_summary_that_calls_a_tool_is_asked_once_more() {
    let env = env("compact-tool");
    let transport = Arc::new(ScriptedTransport::new(vec![
        tool_reply(&[("t1", "echo", r#"{"text":"a"}"#)]),
        tool_reply(&[("s1", "echo", r#"{"text":"b"}"#)]),
        summary_reply(),
        text_reply("done"),
    ]));
    let agent = Agent::new(small(&transport, &env.cwd), echo_registry());
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let (outcome, seen) = turn(&agent, &mut conv, "go");
    assert_eq!(outcome, TurnOutcome::Done);
    let bodies = bodies(&transport);
    let retry = messages(&bodies[2]);
    let answered = &retry[retry.len() - 1]["content"][0];
    assert_eq!(answered["tool_use_id"], "s1");
    assert_eq!(answered["is_error"], true);
    assert!(answered.to_string().contains(NO_TOOLS));
    assert!(!seen
        .iter()
        .any(|e| matches!(e, AgentEvent::ToolCall { id, .. } if id == "s1")));
    let next = messages(&bodies[3]);
    assert_eq!(next.len(), 2);
    assert!(first_text(&next[0]).contains("fix the bug"));
}

#[test]
fn compacts_at_turn_start_and_on_the_compact_command() {
    let env = env("compact-start");
    let transport = Arc::new(ScriptedTransport::new(vec![
        text_reply("hi"),
        summary_reply(),
        text_reply("next answer"),
        summary_reply(),
    ]));
    let agent = Agent::new(small(&transport, &env.cwd), echo_registry());
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    assert_eq!(turn(&agent, &mut conv, "hello").0, TurnOutcome::Done);
    assert_eq!(turn(&agent, &mut conv, "more").0, TurnOutcome::Done);
    let next = messages(&bodies(&transport)[2]);
    assert_eq!(next.len(), 2);
    assert!(first_text(&next[0]).starts_with(SUMMARY_PREFIX));
    assert_eq!(first_text(&next[1]), "more");
    let (outcome, seen) = turn(&agent, &mut conv, "/compact keep the paths");
    assert_eq!(outcome, TurnOutcome::Done);
    let asked = messages(&bodies(&transport)[3]);
    assert!(first_text(&asked[asked.len() - 1]).ends_with("keep the paths"));
    assert_eq!(conv.messages.len(), 1);
    assert!(seen
        .iter()
        .any(|e| matches!(e, AgentEvent::Compaction { auto: false, .. })));
    let log = events(&conv.log_path);
    let users = log.iter().filter(|e| e.kind == kind::USER_MESSAGE);
    assert_eq!(users.count(), 2, "the command is not a user message");
}

#[test]
fn the_header_is_written_before_the_user_message() {
    let env = env("compact-header");
    let transport = Arc::new(ScriptedTransport::new(vec![text_reply("hi")]));
    let agent = Agent::new(parts(&transport, key_auth(), &env.cwd), echo_registry());
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    turn(&agent, &mut conv, "hello");
    let kinds = kinds(&conv.log_path);
    let at = |name: &str| kinds.iter().position(|k| k == name).unwrap();
    assert!(at(kind::REQUEST_HEADER) < at(kind::USER_MESSAGE));
}
