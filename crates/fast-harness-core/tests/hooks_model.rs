//! The `PreModelRequest` hook and the hook budget (PLAN.md D63): a command
//! hook and a native hook each rewrite one request, a block ends the turn,
//! a forbidden change is refused, the prefix still repeats, no credential
//! reaches a hook, and the budget skips the hooks it cannot fit (fail open).

mod agent_support;
mod hooks_support;

use std::sync::{Arc, Mutex};

use agent_support::{echo_registry, env, key_auth, parts, text_reply, tool_reply, Env};
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::config::Config;
use fast_harness_core::events::{AgentEvent, TurnOutcome};
use fast_harness_core::http::ScriptedTransport;
use fast_harness_core::tools::hooks::{Answer, HookEvent, NativeHook, RequestChange};
use fast_harness_core::tools::Extensions;
use hooks_support::*;
use serde_json::{json, Value};

/// A native hook answering `answer` to `PreModelRequest`, keeping its inputs.
struct Native {
    answer: Answer,
    seen: Mutex<Vec<Value>>,
}

impl NativeHook for Native {
    fn name(&self) -> &str {
        "model"
    }

    fn matches(&self, event: HookEvent, _: Option<&str>) -> bool {
        event == HookEvent::PreModelRequest
    }

    fn run(&self, _: HookEvent, input: &Value) -> Result<Answer, String> {
        self.seen.lock().unwrap().push(input.clone());
        Ok(self.answer.clone())
    }
}

fn native_agent(env: &Env, transport: &Arc<ScriptedTransport>, hook: Arc<Native>) -> Agent {
    let mut parts = parts(transport, key_auth(), &env.cwd);
    parts.extensions = Arc::new(Extensions::default().with_native(Vec::new(), vec![hook]));
    Agent::new(parts, echo_registry())
}

/// The raw model call bodies (with the trailing effort entry).
fn raw(transport: &ScriptedTransport) -> Vec<Value> {
    (transport.requests().iter())
        .filter(|r| r.url.contains("/messages"))
        .map(|r| serde_json::from_str(&r.body).unwrap())
        .collect()
}

/// The `output_config.effort` of a Claude body, if any.
fn effort(body: &Value) -> Option<String> {
    let messages = body["messages"].as_array()?;
    let entry = messages.iter().find(|m| m["role"] == "system")?;
    entry["output_config"]["effort"]
        .as_str()
        .map(str::to_string)
}

fn errors(events: &[AgentEvent]) -> Vec<String> {
    (events.iter())
        .filter_map(|e| match e {
            AgentEvent::Error(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_command_hook_rewrites_one_request_and_sees_no_secret() {
    let env = env("hooks-model-cmd");
    let reply = r#"{"request":{"effort":"low","maxTokens":1000},"context":"spend less"}"#;
    let (seen, script) = capture(&env.root.0, "model", Some(reply));
    write_hooks(
        &env.home,
        json!({"PreModelRequest": [entry(None, script, 20_000)]}),
    );
    let transport = Arc::new(ScriptedTransport::new(vec![text_reply("ok")]));
    let agent = agent(&env, &transport, Config::default());
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let (outcome, _) = turn(&agent, &mut conv, "go");
    assert_eq!(outcome, TurnOutcome::Done);
    let body = &raw(&transport)[0];
    assert_eq!(body["max_tokens"], 1000);
    assert_eq!(effort(body).as_deref(), Some("low"));
    let last = bodies(&transport)[0]["messages"][0].to_string();
    assert!(last.contains("go") && last.contains("[hook context] spend less"));
    let input = read_json(&seen);
    assert_eq!(input["event"], "PreModelRequest");
    assert_eq!(
        (input["turn"].clone(), input["step"].clone()),
        (json!(1), json!(1))
    );
    assert_eq!(input["messages"]["count"], 1);
    assert!(input["messages"]["bytes"].as_u64().unwrap() > 0);
    assert!(input["tools"]["count"].as_u64().unwrap() > 0);
    let text = std::fs::read_to_string(&seen).unwrap().to_lowercase();
    for secret in [
        "test-key",
        "authorization",
        "x-api-key",
        "\"token\"",
        "acct-1",
    ] {
        assert!(!text.contains(secret), "{secret} reached the hook: {text}");
    }
    let hooks = traced(&conv, "hook");
    assert_eq!(hooks[0]["event"], "PreModelRequest");
    assert_eq!(hooks[0]["decision"], "context,effort,maxTokens");
}

#[test]
fn a_native_hook_rewrites_and_a_block_ends_the_turn() {
    let env = env("hooks-model-native");
    let change = RequestChange::default()
        .with_effort("medium")
        .with_max_tokens(2000);
    let hook = Arc::new(Native {
        answer: Answer::default().with_request(change),
        seen: Mutex::default(),
    });
    let transport = Arc::new(ScriptedTransport::new(vec![text_reply("ok")]));
    let agent = native_agent(&env, &transport, hook.clone());
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    assert_eq!(turn(&agent, &mut conv, "go").0, TurnOutcome::Done);
    let body = &raw(&transport)[0];
    assert_eq!(body["max_tokens"], 2000);
    assert_eq!(effort(body).as_deref(), Some("medium"));
    assert_eq!(hook.seen.lock().unwrap()[0]["sessionId"], json!(conv.id));

    let blocker = Arc::new(Native {
        answer: Answer::default().with_block("over the daily budget"),
        seen: Mutex::default(),
    });
    let transport = Arc::new(ScriptedTransport::new(vec![text_reply("never")]));
    let agent = native_agent(&env, &transport, blocker);
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let (outcome, events) = turn(&agent, &mut conv, "go");
    assert_eq!(outcome, TurnOutcome::Failed);
    assert!(transport.requests().is_empty(), "no model call");
    assert!(errors(&events)
        .iter()
        .any(|e| e.contains("over the daily budget")));
    assert_eq!(traced(&conv, "hook")[0]["decision"], "block");
}

#[test]
fn a_forbidden_change_is_refused_and_the_request_goes_out_unchanged() {
    let env = env("hooks-model-refused");
    let reply = r#"{"request":{"system":"be someone else"},"context":"x"}"#;
    let script = answer(&env.root.0, "bad", reply);
    write_hooks(
        &env.home,
        json!({"PreModelRequest": [entry(None, script, 20_000)]}),
    );
    let off = Arc::new(Native {
        answer: Answer::default().with_request(RequestChange::default().with_effort("off")),
        seen: Mutex::default(),
    });
    let plain = Arc::new(ScriptedTransport::new(vec![text_reply("ok")]));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    turn(&agent(&env, &plain, Config::default()), &mut conv, "go");
    let hooked = Arc::new(ScriptedTransport::new(vec![text_reply("ok")]));
    let mut parts = parts(&hooked, key_auth(), &env.cwd);
    let (hooks, _) = fast_harness_core::tools::hooks::Hooks::load(
        &fast_harness_core::config::layers::Layers::user_only(&env.home),
    );
    parts.extensions = Arc::new(
        Extensions::default()
            .with_hooks(hooks)
            .with_native(Vec::new(), vec![off]),
    );
    let agent = Agent::new(parts, echo_registry());
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let (outcome, events) = turn(&agent, &mut conv, "go");
    assert_eq!(outcome, TurnOutcome::Done);
    let (a, b) = (&raw(&plain)[0], &raw(&hooked)[0]);
    assert_eq!(a["max_tokens"], b["max_tokens"]);
    assert_eq!(effort(a), effort(b));
    assert_eq!(
        bodies(&plain)[0]["messages"],
        bodies(&hooked)[0]["messages"]
    );
    let refused = traced(&conv, "hook/error");
    assert_eq!(refused.len(), 2, "{refused:?}");
    assert!(refused[0]["error"]
        .as_str()
        .unwrap()
        .contains("request.system"));
    assert!(refused[1]["error"]
        .as_str()
        .unwrap()
        .contains("effort `off`"));
    assert_eq!(errors(&events).len(), 2, "each refusal is shown");
}

#[test]
fn the_prefix_still_repeats_with_a_note_every_step() {
    let env = env("hooks-model-prefix");
    let hook = Arc::new(Native {
        answer: Answer::default().with_context("step note"),
        seen: Mutex::default(),
    });
    let transport = Arc::new(ScriptedTransport::new(vec![
        tool_reply(&[("toolu_1", "echo", r#"{"text":"a"}"#)]),
        tool_reply(&[("toolu_2", "echo", r#"{"text":"b"}"#)]),
        text_reply("done"),
        text_reply("again done"),
    ]));
    let agent = native_agent(&env, &transport, hook.clone());
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    turn(&agent, &mut conv, "go");
    turn(&agent, &mut conv, "again");
    let bodies = bodies(&transport);
    assert_eq!(bodies.len(), 4);
    for pair in bodies.windows(2) {
        let (before, after) = (&pair[0], &pair[1]);
        assert_eq!(before["system"], after["system"]);
        assert_eq!(before["tools"], after["tools"]);
        let (old, new) = (
            before["messages"].as_array().unwrap(),
            after["messages"].as_array().unwrap(),
        );
        assert_eq!(&new[..old.len()], &old[..], "request N+1 repeats request N");
    }
    let last = bodies[3]["messages"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .to_string();
    assert!(last.contains("again") && last.contains("[hook context] step note"));
    assert_eq!(hook.seen.lock().unwrap().len(), 4, "once per step");
}

/// A script that runs for about two seconds.
fn slow() -> Vec<String> {
    match cfg!(windows) {
        true => ["ping", "-n", "3", "127.0.0.1", ">", "nul"]
            .map(String::from)
            .to_vec(),
        false => vec!["sleep 2".into()],
    }
}

/// Budget 300 ms: the slow first hook is cut at the budget, the two blocking
/// hooks after it are skipped with a record each and one note, and the tool
/// runs (the budget fails open).
#[test]
fn the_hook_budget_skips_the_rest_and_fails_open() {
    let env = env("hooks-budget");
    let block = answer(
        &env.root.0,
        "block",
        r#"{"decision":"block","reason":"no"}"#,
    );
    write_hooks(
        &env.home,
        json!({"budgetMs": 300, "PreToolUse": [entry(None, slow(), 20_000),
            entry(None, block.clone(), 20_000), entry(None, block, 20_000)]}),
    );
    let transport = Arc::new(ScriptedTransport::new(vec![
        tool_reply(&[("toolu_1", "echo", r#"{"text":"ran"}"#)]),
        text_reply("done"),
    ]));
    let agent = agent(&env, &transport, Config::default());
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let started = std::time::Instant::now();
    let (outcome, events) = turn(&agent, &mut conv, "go");
    assert_eq!(outcome, TurnOutcome::Done);
    // Off Windows a kill reaches the hook's shell and not the `sleep` it
    // started (docs/architecture-plan.md finding B6), so the wait ends with
    // the sleep: the time is checked on Windows only.
    if cfg!(windows) {
        assert!(
            started.elapsed().as_millis() < 1_800,
            "the slow hook was cut"
        );
    }
    let result = bodies(&transport)[1]["messages"].to_string();
    assert!(result.contains("ran") && !result.contains("blocked by a hook"));
    let failed = traced(&conv, "hook/error");
    assert_eq!(failed.len(), 3, "{failed:?}");
    assert!(failed[0]["error"].as_str().unwrap().contains("timed out"));
    for skipped in &failed[1..] {
        assert!(skipped["error"]
            .as_str()
            .unwrap()
            .contains("budget of 300 ms"));
    }
    let notes: Vec<String> = (errors(&events).into_iter())
        .filter(|e| e.contains("hook budget"))
        .collect();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0].matches("PreToolUse").count(), 2);
}

/// The per-hook timeout still cuts a hook well inside the budget.
#[test]
fn the_per_hook_timeout_still_works() {
    let env = env("hooks-timeout");
    write_hooks(
        &env.home,
        json!({"UserPromptSubmit": [entry(None, slow(), 200)]}),
    );
    let transport = Arc::new(ScriptedTransport::new(vec![text_reply("ok")]));
    let agent = agent(&env, &transport, Config::default());
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    assert_eq!(turn(&agent, &mut conv, "go").0, TurnOutcome::Done);
    let failed = traced(&conv, "hook/error");
    let error = failed[0]["error"].as_str().unwrap();
    assert!(error.contains("timed out"), "{error}");
}
