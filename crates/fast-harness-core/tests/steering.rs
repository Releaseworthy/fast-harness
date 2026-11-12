//! Steering, queue, pause and resume (PLAN.md D39): a steer between steps
//! lands after that step's tool results, a follow-up starts the next turn,
//! a pause ends the turn after its tools, and `resume_turn` continues a
//! paused, cancelled or crashed turn without a new user message.

mod agent_support;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use agent_support::{echo_registry, env, events, key_auth, parts, text_reply, tool_reply, Env};
use fast_harness_core::agent::{Agent, Conversation, Inbox, Input, MISSING_RESULT};
use fast_harness_core::events::{AgentEvent, TurnOutcome};
use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use fast_harness_core::session::{self, Unfinished};
use fast_harness_core::trace::kind;
use serde_json::{json, Value};

const CALL: &str = r#"{"text":"hi"}"#;

fn agent(transport: &Arc<ScriptedTransport>, env: &Env) -> Agent {
    Agent::new(parts(transport, key_auth(), &env.cwd), echo_registry())
}

/// Model call bodies without `cache_control` and without the trailing
/// Claude effort entry (`role: system`).
fn bodies(transport: &ScriptedTransport, path: &str) -> Vec<Value> {
    let requests = transport.requests();
    let model_calls = requests.iter().filter(|r| r.url.contains(path));
    model_calls
        .map(|r| strip(&serde_json::from_str(&r.body).unwrap()))
        .collect()
}

fn strip(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| key.as_str() != "cache_control")
                .map(|(key, item)| (key.clone(), strip(item)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .filter(|item| item["role"] != "system")
                .map(strip)
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Runs a turn whose sink sends `input` when it sees the first event
/// `when` matches (the step is still running then).
fn run_with(
    agent: &Agent,
    conv: &mut Conversation,
    input: Input,
    when: fn(&AgentEvent) -> bool,
) -> (TurnOutcome, Inbox, Vec<AgentEvent>) {
    let (tx, mut inbox) = Inbox::channel();
    let mut pending: Option<(Sender<Input>, Input)> = Some((tx, input));
    let mut seen = Vec::new();
    let cancel = AtomicBool::new(false);
    let outcome = agent.run_turn_with(conv, "go", &cancel, &mut inbox, &mut |event| {
        if when(&event) {
            if let Some((tx, input)) = pending.take() {
                tx.send(input).unwrap();
            }
        }
        seen.push(event);
    });
    (outcome, inbox, seen)
}

fn on_tool_call(event: &AgentEvent) -> bool {
    matches!(event, AgentEvent::ToolCall { .. })
}

fn on_text(event: &AgentEvent) -> bool {
    matches!(event, AgentEvent::TextDelta(_))
}

fn steer() -> Input {
    Input::Steer("also check the tests".into())
}

#[test]
fn steer_lands_after_the_tool_results_on_claude() {
    let env = env("steer-claude");
    let replies = vec![tool_reply(&[("toolu_1", "echo", CALL)]), text_reply("ok")];
    let transport = Arc::new(ScriptedTransport::new(replies));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let (outcome, mut inbox, seen) =
        run_with(&agent(&transport, &env), &mut conv, steer(), on_tool_call);
    assert_eq!(outcome, TurnOutcome::Done);
    assert!(inbox.leftover().is_empty());
    let steered = AgentEvent::UserInput {
        text: "also check the tests".into(),
        steer: true,
    };
    assert!(seen.contains(&steered));
    // Golden body. "In the user message containing tool results, the
    // `tool_result` blocks must come FIRST in the content array. Any text
    // must come AFTER all tool results."
    // https://platform.claude.com/docs/en/agents-and-tools/tool-use/handle-tool-calls
    let second = &bodies(&transport, "/v1/messages")[1];
    let want = json!([
        {"role": "user", "content": [{"type": "text", "text": "go"}]},
        {"role": "assistant", "content": [
            {"type": "tool_use", "id": "toolu_1", "name": "echo", "input": {"text": "hi"}}]},
        {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "toolu_1", "content": "hi", "is_error": false},
            {"type": "text", "text": "also check the tests"}]}
    ]);
    assert_eq!(second["messages"], want);
    let logged = events(&conv.log_path);
    let steer_event = logged.iter().find(|e| e.data["kind"] == "steer").unwrap();
    assert_eq!(steer_event.kind, kind::USER_MESSAGE);
    // A resume replays the steer into the same message.
    let resumed = Conversation::resume(&env.home, &env.cwd, &conv.id).unwrap();
    assert_eq!(resumed.messages, conv.messages);
}

#[test]
fn steer_after_a_final_answer_starts_the_next_step() {
    let env = env("steer-end");
    let transport = Arc::new(ScriptedTransport::new(vec![
        text_reply("first"),
        text_reply("second"),
    ]));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let (outcome, _, _) = run_with(&agent(&transport, &env), &mut conv, steer(), on_text);
    assert_eq!(outcome, TurnOutcome::Done);
    let second = &bodies(&transport, "/v1/messages")[1];
    let messages = second["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(
        messages[2]["content"],
        json!([{"type": "text", "text": "also check the tests"}])
    );
    let turns = events(&conv.log_path)
        .iter()
        .filter(|e| e.kind == kind::TURN_START)
        .count();
    assert_eq!(turns, 1, "same turn");
}

#[test]
fn follow_up_starts_a_new_turn() {
    let env = env("follow-up");
    let transport = Arc::new(ScriptedTransport::new(vec![
        text_reply("one"),
        text_reply("two"),
    ]));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let input = Input::FollowUp("next please".into());
    let (outcome, _, seen) = run_with(&agent(&transport, &env), &mut conv, input, on_text);
    assert_eq!(outcome, TurnOutcome::Done);
    let follow = AgentEvent::UserInput {
        text: "next please".into(),
        steer: false,
    };
    assert!(seen.contains(&follow));
    let logged = events(&conv.log_path);
    let ends: Vec<&str> = logged
        .iter()
        .filter(|e| e.kind == kind::TURN_END)
        .map(|e| e.data["outcome"].as_str().unwrap())
        .collect();
    assert_eq!(ends, ["done", "done"]);
    let second = &bodies(&transport, "/v1/messages")[1];
    assert_eq!(second["messages"][2]["content"][0]["text"], "next please");
}

/// The second request's last message: the tool results, no user text.
fn assert_resumed_on_results(transport: &ScriptedTransport, result: &str) {
    let second = &bodies(transport, "/v1/messages")[1];
    let last = second["messages"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    let want = json!({"role": "user", "content": [{"type": "tool_result",
        "tool_use_id": "toolu_1", "content": result, "is_error": result != "hi"}]});
    assert_eq!(last, want);
}

fn tool_then_text() -> Vec<ScriptedResponse> {
    vec![tool_reply(&[("toolu_1", "echo", CALL)]), text_reply("done")]
}

#[test]
fn pause_ends_after_the_tools_and_resume_turn_goes_on() {
    let env = env("pause");
    let transport = Arc::new(ScriptedTransport::new(tool_then_text()));
    let agent = agent(&transport, &env);
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let (outcome, _, _) = run_with(&agent, &mut conv, Input::Pause, on_tool_call);
    assert_eq!(outcome, TurnOutcome::Paused);
    assert_eq!(transport.requests().len(), 1, "no second model call");
    let mut conv = Conversation::resume(&env.home, &env.cwd, &conv.id).unwrap();
    assert_eq!(conv.unfinished, Some(Unfinished::Paused));
    let cancel = AtomicBool::new(false);
    let outcome = agent.resume_turn(&mut conv, &cancel, &mut Inbox::empty(), &mut |_| {});
    assert_eq!(outcome, TurnOutcome::Done);
    assert_resumed_on_results(&transport, "hi");
    let logged = events(&conv.log_path);
    let resume = logged
        .iter()
        .filter(|e| e.kind == kind::TURN_START)
        .nth(1)
        .unwrap();
    assert_eq!(resume.data, json!({"turn": 2, "resume": true}));
    let users = logged
        .iter()
        .filter(|e| e.kind == kind::USER_MESSAGE)
        .count();
    assert_eq!(users, 1, "resume adds no user message");
    let reopened = Conversation::resume(&env.home, &env.cwd, &conv.id).unwrap();
    assert_eq!(reopened.unfinished, None);
}

/// Keeps the log lines up to the first event of `cut` kinds after a
/// `tool/call`: a crash while the tools ran.
fn crash_log(path: &std::path::Path) {
    let text = std::fs::read_to_string(path).unwrap();
    let kept: Vec<&str> = text
        .lines()
        .take_while(|line| !line.contains("\"tool/result\""))
        .collect();
    std::fs::write(path, kept.join("\n") + "\n").unwrap();
}

#[test]
fn a_crashed_turn_is_interrupted_and_resumes() {
    let env = env("crash");
    let transport = Arc::new(ScriptedTransport::new(tool_then_text()));
    let agent = agent(&transport, &env);
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let (outcome, _, _) = run_with(&agent, &mut conv, Input::Pause, on_tool_call);
    assert_eq!(outcome, TurnOutcome::Paused);
    crash_log(&conv.log_path);
    let listed = session::list(&env.home, &env.cwd).unwrap();
    assert_eq!(listed[0].unfinished, Some(Unfinished::Interrupted));
    let mut conv = Conversation::resume(&env.home, &env.cwd, &conv.id).unwrap();
    assert_eq!(conv.unfinished, Some(Unfinished::Interrupted));
    let cancel = AtomicBool::new(false);
    let outcome = agent.resume_turn(&mut conv, &cancel, &mut Inbox::empty(), &mut |_| {});
    assert_eq!(outcome, TurnOutcome::Done);
    assert_resumed_on_results(&transport, MISSING_RESULT);
}

#[test]
fn a_cancelled_turn_resumes_in_memory() {
    let env = env("cancel");
    let transport = Arc::new(ScriptedTransport::new(tool_then_text()));
    let agent = agent(&transport, &env);
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let cancel = AtomicBool::new(false);
    let outcome = agent.run_turn(&mut conv, "go", &cancel, &mut |event| {
        if on_tool_call(&event) {
            cancel.store(true, Ordering::SeqCst);
        }
    });
    assert_eq!(outcome, TurnOutcome::Cancelled);
    let reopened = Conversation::resume(&env.home, &env.cwd, &conv.id).unwrap();
    assert_eq!(reopened.unfinished, Some(Unfinished::Cancelled));
    cancel.store(false, Ordering::SeqCst);
    let outcome = agent.resume_turn(&mut conv, &cancel, &mut Inbox::empty(), &mut |_| {});
    assert_eq!(outcome, TurnOutcome::Done);
    assert_eq!(transport.requests().len(), 2);
}

#[test]
fn a_finished_turn_has_nothing_to_resume() {
    let env = env("nothing");
    let transport = Arc::new(ScriptedTransport::new(vec![text_reply("done")]));
    let agent = agent(&transport, &env);
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let cancel = AtomicBool::new(false);
    assert_eq!(
        agent.run_turn(&mut conv, "go", &cancel, &mut |_| {}),
        TurnOutcome::Done
    );
    let reopened = Conversation::resume(&env.home, &env.cwd, &conv.id).unwrap();
    assert_eq!(reopened.unfinished, None);
    let mut errors = Vec::new();
    let outcome = agent.resume_turn(&mut conv, &cancel, &mut Inbox::empty(), &mut |event| {
        errors.push(event)
    });
    assert_eq!(outcome, TurnOutcome::Failed);
    assert_eq!(errors.len(), 1);
    assert_eq!(transport.requests().len(), 1);
}
