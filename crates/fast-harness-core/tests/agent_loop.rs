//! The turn loop over `ScriptedTransport`: trace sequence, parallel tools,
//! retries, failover, waits and cancel.

mod agent_support;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agent_support::*;
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::auth::Failover;
use fast_harness_core::events::{AgentEvent, TurnOutcome};
use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use fast_harness_core::types::{Block, Role};

fn run(
    agent: &Agent,
    conv: &mut Conversation,
    cancel: &AtomicBool,
) -> (TurnOutcome, Vec<AgentEvent>) {
    let mut seen = Vec::new();
    let outcome = agent.run_turn(conv, "go", cancel, &mut |e| seen.push(e));
    (outcome, seen)
}

fn setup(
    name: &str,
    responses: Vec<ScriptedResponse>,
) -> (Env, Arc<ScriptedTransport>, Conversation) {
    let env = env(name);
    let transport = Arc::new(ScriptedTransport::new(responses));
    let conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    (env, transport, conv)
}

#[test]
fn two_step_turn_writes_the_exact_trace() {
    let responses = vec![
        tool_reply(&[("toolu_1", "echo", r#"{"text":"hi"}"#)]),
        text_reply("done"),
        text_reply("again"),
    ];
    let (env, transport, mut conv) = setup("two-step", responses);
    let agent = Agent::new(parts(&transport, key_auth(), &env.cwd), echo_registry());
    let cancel = AtomicBool::new(false);
    let (outcome, seen) = run(&agent, &mut conv, &cancel);
    assert_eq!(outcome, TurnOutcome::Done);
    let expected = [
        "turn/start",
        "user/message",
        "step/start",
        "request/header",
        "request/context",
        "assistant/message",
        "tool/call",
        "tool/result",
        "step/end",
        "step/start",
        "request/context",
        "assistant/message",
        "step/end",
        "turn/end",
    ];
    assert_eq!(kinds(&conv.log_path), expected);
    assert!(matches!(
        seen.last(),
        Some(AgentEvent::TurnEnd {
            outcome: TurnOutcome::Done,
            ..
        })
    ));
    assert!(seen.contains(&AgentEvent::TextDelta("done".into())));
    let roles: Vec<Role> = conv.messages.iter().map(|m| m.role).collect();
    assert_eq!(
        roles,
        [Role::User, Role::Assistant, Role::Tool, Role::Assistant]
    );
    check_two_step_trace(&conv);
    assert!(transport.requests()[1].body.contains("tool_result"));
    // A second turn with the same prompt and tools writes no new request/header.
    let (outcome, _) = run(&agent, &mut conv, &cancel);
    assert_eq!(outcome, TurnOutcome::Done);
    let kinds = kinds(&conv.log_path);
    assert_eq!(kinds.iter().filter(|k| *k == "request/header").count(), 1);
    assert_eq!(conv.turn, 2);
    drop(env);
}

fn check_two_step_trace(conv: &Conversation) {
    let events = events(&conv.log_path);
    let contexts: Vec<_> = events
        .iter()
        .filter(|e| e.kind == "request/context")
        .collect();
    // Wire capture is off by default: no `wire` key and no `.wire` folder.
    assert!(contexts[1].data.get("wire").is_none());
    assert_eq!(contexts[1].data["step"], 2);
    assert_eq!(contexts[1].data["account"], "acct-1");
    let result = first(&events, "tool/result");
    assert_eq!(result.data["message"]["content"][0]["content"], "hi");
    let end = first(&events, "turn/end");
    assert_eq!(
        (end.data["steps"].as_u64(), end.data["outcome"].as_str()),
        (Some(2), Some("done"))
    );
    assert_eq!(end.data["usage"]["inputTokens"], 20);
    assert!(!conv.wire_dir.exists());
}

#[test]
fn wire_capture_on_writes_the_wire_files() {
    let (env, transport, mut conv) = setup("wire-on", vec![text_reply("ok")]);
    let mut parts = parts(&transport, key_auth(), &env.cwd);
    parts.config.wire = true;
    let agent = Agent::new(parts, echo_registry());
    let (outcome, _) = run(&agent, &mut conv, &AtomicBool::new(false));
    assert_eq!(outcome, TurnOutcome::Done);
    let events = events(&conv.log_path);
    assert_eq!(first(&events, "request/context").data["wire"], "t1-s1-a1");
    for suffix in ["request.json", "head.json", "response.txt"] {
        let file = conv.wire_dir.join(format!("t1-s1-a1.{suffix}"));
        assert!(file.is_file(), "{}", file.display());
    }
    drop(env);
}

#[test]
fn parallel_tool_calls_keep_call_order() {
    let calls = [
        ("toolu_1", "echo", r#"{"text":"one","sleepMs":300}"#),
        ("toolu_2", "echo", r#"{"text":"two","sleepMs":10}"#),
        ("toolu_3", "echo", r#"{"text":"three","sleepMs":120}"#),
    ];
    let (env, transport, mut conv) = setup("parallel", vec![tool_reply(&calls), text_reply("ok")]);
    let agent = Agent::new(parts(&transport, key_auth(), &env.cwd), echo_registry());
    let (outcome, seen) = run(&agent, &mut conv, &AtomicBool::new(false));
    assert_eq!(outcome, TurnOutcome::Done);
    let finished: Vec<&str> = seen
        .iter()
        .filter_map(|e| match e {
            AgentEvent::ToolResult { id, .. } => Some(id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        finished,
        ["toolu_2", "toolu_3", "toolu_1"],
        "results arrive as tools finish"
    );
    let traced: Vec<String> = events(&conv.log_path)
        .iter()
        .filter(|e| e.kind == "tool/result")
        .map(|e| {
            e.data["message"]["content"][0]["callId"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(traced, ["toolu_1", "toolu_2", "toolu_3"]);
    let tool_message = &conv.messages[2];
    let texts: Vec<&str> = tool_message
        .content
        .iter()
        .map(|b| match b {
            Block::ToolResult { content, .. } => content.as_str(),
            _ => "",
        })
        .collect();
    assert_eq!(texts, ["one", "two", "three"]);
    drop(env);
}

#[test]
fn overloaded_is_retried_then_succeeds() {
    let busy = error_reply(529, "overloaded_error", "busy").with_header("retry-after", "0");
    let (env, transport, mut conv) = setup("retry", vec![busy, text_reply("fine")]);
    let agent = Agent::new(parts(&transport, key_auth(), &env.cwd), echo_registry());
    let (outcome, seen) = run(&agent, &mut conv, &AtomicBool::new(false));
    assert_eq!(outcome, TurnOutcome::Done);
    let expected = [
        "turn/start",
        "user/message",
        "step/start",
        "request/header",
        "request/context",
        "assistant/attempt",
        "llm/retry",
        "request/context",
        "assistant/message",
        "step/end",
        "turn/end",
    ];
    assert_eq!(kinds(&conv.log_path), expected);
    let events = events(&conv.log_path);
    let retry = first(&events, "llm/retry");
    assert_eq!(retry.data["delayMs"], 0);
    assert!(retry.data["error"].as_str().unwrap().contains("Overloaded"));
    let contexts: Vec<_> = events
        .iter()
        .filter(|e| e.kind == "request/context")
        .collect();
    assert_eq!(contexts[1].data["attempt"], 2);
    assert!(contexts[1].data.get("wire").is_none());
    assert!(seen.iter().any(|e| matches!(
        e,
        AgentEvent::Retry {
            attempt: 1,
            delay_ms: 0,
            ..
        }
    )));
    drop(env);
}

#[test]
fn rate_limit_switches_to_another_account() {
    let limited = error_reply(429, "rate_limit_error", "slow down");
    let (env, transport, mut conv) = setup("switch", vec![limited, text_reply("fine")]);
    let auth = FakeAuth::new(vec!["acct-a", "acct-b"], Failover::Retry);
    let agent = Agent::new(parts(&transport, auth, &env.cwd), echo_registry());
    let (outcome, seen) = run(&agent, &mut conv, &AtomicBool::new(false));
    assert_eq!(outcome, TurnOutcome::Done);
    let events = events(&conv.log_path);
    let switch = first(&events, "account/switch");
    assert_eq!(switch.data["from"], "acct-a");
    assert_eq!(switch.data["to"], "acct-b");
    assert_eq!(switch.data["reason"], "RateLimit");
    assert_eq!(
        first(&events, "assistant/message").data["account"],
        "acct-b"
    );
    let kinds = kinds(&conv.log_path);
    let at = kinds.iter().position(|k| k == "account/switch").unwrap();
    assert_eq!(kinds[at - 1], "assistant/attempt");
    assert!(seen
        .iter()
        .any(|e| matches!(e, AgentEvent::AccountSwitch { to, .. } if to == "acct-b")));
    drop(env);
}

#[test]
fn failover_wait_beyond_the_limit_fails() {
    let limited = error_reply(429, "rate_limit_error", "quota");
    let (env, transport, mut conv) = setup("wait", vec![limited]);
    let auth = FakeAuth::new(vec!["acct-a"], Failover::Wait(10_000));
    let mut parts = parts(&transport, auth, &env.cwd);
    parts.config.max_wait_ms = 1_000;
    let agent = Agent::new(parts, echo_registry());
    let started = Instant::now();
    let (outcome, seen) = run(&agent, &mut conv, &AtomicBool::new(false));
    assert_eq!(outcome, TurnOutcome::Failed);
    assert!(started.elapsed() < Duration::from_secs(5));
    let kinds = kinds(&conv.log_path);
    assert_eq!(
        &kinds[kinds.len() - 3..],
        ["assistant/attempt", "step/end", "turn/end"]
    );
    let events = events(&conv.log_path);
    assert_eq!(first(&events, "turn/end").data["outcome"], "failed");
    assert!(seen
        .iter()
        .any(|e| matches!(e, AgentEvent::Error(m) if m.contains("RateLimit"))));
    drop(env);
}

#[test]
fn step_limit_ends_the_turn() {
    let calls = [("toolu_1", "echo", r#"{"text":"x"}"#)];
    let (env, transport, mut conv) = setup("limit", vec![tool_reply(&calls)]);
    let mut parts = parts(&transport, key_auth(), &env.cwd);
    parts.config.max_steps = 1;
    let agent = Agent::new(parts, echo_registry());
    let (outcome, _) = run(&agent, &mut conv, &AtomicBool::new(false));
    assert_eq!(outcome, TurnOutcome::StepLimit);
    assert_eq!(
        first(&events(&conv.log_path), "turn/end").data["outcome"],
        "step_limit"
    );
    drop(env);
}

#[test]
fn cancel_during_a_stream_ends_cancelled() {
    let (env, transport, mut conv) = setup("cancel", vec![text_reply("a long answer here")]);
    let agent = Agent::new(parts(&transport, key_auth(), &env.cwd), echo_registry());
    let cancel = AtomicBool::new(false);
    let mut seen = Vec::new();
    let outcome = agent.run_turn(&mut conv, "go", &cancel, &mut |e| {
        if matches!(e, AgentEvent::TextDelta(_)) {
            cancel.store(true, Ordering::Relaxed);
        }
        seen.push(e);
    });
    assert_eq!(outcome, TurnOutcome::Cancelled);
    let kinds = kinds(&conv.log_path);
    assert!(!kinds.contains(&"assistant/message".to_string()));
    assert_eq!(
        first(&events(&conv.log_path), "turn/end").data["outcome"],
        "cancelled"
    );
    assert!(matches!(
        seen.last(),
        Some(AgentEvent::TurnEnd {
            outcome: TurnOutcome::Cancelled,
            ..
        })
    ));
    drop(env);
}

#[test]
fn cancel_wakes_a_retry_wait() {
    let busy = error_reply(529, "overloaded_error", "busy").with_header("retry-after", "60");
    let (env, transport, mut conv) = setup("cancel-wait", vec![busy]);
    let agent = Agent::new(parts(&transport, key_auth(), &env.cwd), echo_registry());
    let cancel = AtomicBool::new(false);
    let started = Instant::now();
    let outcome = std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(Duration::from_millis(150));
            cancel.store(true, Ordering::Relaxed);
        });
        run(&agent, &mut conv, &cancel).0
    });
    assert_eq!(outcome, TurnOutcome::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(kinds(&conv.log_path).contains(&"llm/retry".to_string()));
    drop(env);
}
