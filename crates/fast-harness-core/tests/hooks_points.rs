//! Hooks at the other points (PLAN.md D44): UserPromptSubmit blocks,
//! SessionStart context, Stop continues, SubagentStop and PreCompact get
//! their events, project hooks load only when trusted, doctor lists them.

mod agent_support;
mod hooks_support;

use std::sync::Arc;

use agent_support::{env, text_reply};
use fast_harness_core::agent::Conversation;
use fast_harness_core::app::{doctor, load_settings};
use fast_harness_core::config::layers::{Layers, Source};
use fast_harness_core::config::{trust, Config};
use fast_harness_core::events::{AgentEvent, TurnOutcome};
use fast_harness_core::http::ScriptedTransport;
use fast_harness_core::tools::hooks::{HookEvent, Hooks};
use hooks_support::*;
use serde_json::json;

#[test]
fn user_prompt_submit_blocks_the_turn() {
    let env = env("hooks-prompt");
    let (seen, script) = capture(
        &env.root.0,
        "prompt",
        Some(r#"{"decision":"block","reason":"no secrets in prompts"}"#),
    );
    write_hooks(
        &env.home,
        json!({"UserPromptSubmit": [entry(None, script, 20_000)]}),
    );
    let transport = Arc::new(ScriptedTransport::new(vec![text_reply("never")]));
    let agent = agent(&env, &transport, Config::default());
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let (outcome, events) = turn(&agent, &mut conv, "the password is x");
    assert_eq!(outcome, TurnOutcome::Failed);
    assert!(transport.requests().is_empty(), "no model call");
    assert!(conv.messages.is_empty());
    let shown = events
        .iter()
        .any(|e| matches!(e, AgentEvent::Error(text) if text.contains("no secrets in prompts")));
    assert!(shown);
    let input = read_json(&seen);
    assert_eq!(input["event"], "UserPromptSubmit");
    assert_eq!(input["prompt"], "the password is x");
}

#[test]
fn session_start_context_joins_the_first_user_message() {
    let env = env("hooks-start");
    let script = answer(&env.root.0, "start", r#"{"context":"branch main"}"#);
    write_hooks(
        &env.home,
        json!({"SessionStart": [entry(None, script, 20_000)]}),
    );
    let transport = Arc::new(ScriptedTransport::new(vec![
        text_reply("a"),
        text_reply("b"),
    ]));
    let agent = agent(&env, &transport, Config::default());
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    turn(&agent, &mut conv, "go");
    turn(&agent, &mut conv, "again");
    let bodies = bodies(&transport);
    let first = bodies[0]["messages"][0]["content"].to_string();
    assert!(first.contains("go") && first.contains("[hook context] branch main"));
    assert!(!bodies[1]["messages"][2]
        .to_string()
        .contains("hook context"));
    assert_eq!(traced(&conv, "hook").len(), 1, "once per session");
}

#[test]
fn stop_continue_goes_on_with_a_user_text() {
    let env = env("hooks-stop");
    let (seen, script) = capture(&env.root.0, "stop", Some(r#"{"continue":"run the tests"}"#));
    write_hooks(&env.home, json!({"Stop": [entry(None, script, 20_000)]}));
    let transport = Arc::new(ScriptedTransport::new(vec![
        text_reply("first answer"),
        text_reply("second answer"),
    ]));
    let config = Config {
        max_steps: 2,
        ..Config::default()
    };
    let agent = agent(&env, &transport, config);
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let (outcome, _) = turn(&agent, &mut conv, "go");
    assert_eq!(outcome, TurnOutcome::StepLimit, "the step limit bounds it");
    let bodies = bodies(&transport);
    assert_eq!(bodies.len(), 2);
    let last = bodies[1]["messages"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    assert_eq!(last["role"], "user");
    assert!(last.to_string().contains("run the tests"));
    let input = read_json(&seen);
    assert_eq!(input["event"], "Stop");
    assert_eq!(input["answer"], "second answer");
    assert_eq!(input["stopHookActive"], true);
}

#[test]
fn subagent_stop_and_pre_compact_get_their_events() {
    let env = env("hooks-sub");
    let (sub_seen, sub) = capture(&env.root.0, "sub", None);
    let (stop_seen, stop) = capture(&env.root.0, "stop", None);
    let (compact_seen, compact) = capture(&env.root.0, "compact", None);
    write_hooks(
        &env.home,
        json!({"SubagentStop": [entry(None, sub, 20_000)], "Stop": [entry(None, stop, 20_000)],
            "PreCompact": [entry(None, compact, 20_000)]}),
    );
    let transport = Arc::new(ScriptedTransport::new(vec![
        text_reply("child answer"),
        text_reply("the summary"),
    ]));
    let agent = agent(&env, &transport, Config::default());
    let mut conv = Conversation::create_at(&env.home, &env.cwd, Some("parent"), 1).unwrap();
    assert_eq!(turn(&agent, &mut conv, "look").0, TurnOutcome::Done);
    let input = read_json(&sub_seen);
    assert_eq!(input["event"], "SubagentStop");
    assert_eq!(input["answer"], "child answer");
    assert!(
        !stop_seen.exists(),
        "a subagent session runs SubagentStop only"
    );
    assert_eq!(
        turn(&agent, &mut conv, "/compact keep paths").0,
        TurnOutcome::Done
    );
    let input = read_json(&compact_seen);
    assert_eq!(input["event"], "PreCompact");
    assert_eq!(input["trigger"], "manual");
    assert_eq!(input["instructions"], "keep paths");
    assert!(input["summaryRequest"]
        .as_str()
        .unwrap()
        .contains("[compaction]"));
}

#[test]
fn project_hooks_load_only_when_trusted_and_doctor_lists_them() {
    let env = env("hooks-trust");
    let project = env.cwd.join(".fast-harness");
    std::fs::create_dir_all(&project).unwrap();
    let hook = |name: &str| json!([{"command": name, "match": "shell"}]);
    write_hooks(&project, json!({"PreToolUse": hook("project-hook")}));
    write_hooks(&env.home, json!({"PreToolUse": hook("user-hook")}));
    let (hooks, _) = Hooks::load(&Layers::find(&env.home, &env.cwd));
    let names: Vec<&str> = hooks
        .entries()
        .iter()
        .map(|e| e.1.command.as_str())
        .collect();
    assert_eq!(
        names,
        ["user-hook"],
        "an untrusted project's hooks do not load"
    );
    trust::trust(&env.home, &env.cwd).unwrap();
    let (hooks, _) = Hooks::load(&Layers::find(&env.home, &env.cwd));
    let names: Vec<&str> = hooks
        .entries()
        .iter()
        .map(|e| e.1.command.as_str())
        .collect();
    assert_eq!(names, ["user-hook", "project-hook"], "both run, user first");
    let last = &hooks.entries()[1];
    assert_eq!(
        (last.0, hooks.source_of(&last.1)),
        (HookEvent::PreToolUse, Source::Project)
    );
    let settings = load_settings(&env.home, Some(&env.cwd), &|_| None).unwrap();
    let report = doctor(&settings);
    assert!(report.contains("hooks:"), "{report}");
    assert!(
        report.contains("PreToolUse match shell: project-hook (project"),
        "{report}"
    );
    std::fs::write(env.home.join("hooks.json"), r#"{"Nope": []}"#).unwrap();
    let (hooks, skipped) = Hooks::load(&Layers::user_only(&env.home));
    assert!(hooks.is_empty() && skipped[0].reason.contains("Nope"));
}
