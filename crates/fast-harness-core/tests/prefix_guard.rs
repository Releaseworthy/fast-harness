//! `prefix_guard` (docs/efficiency-plan.md phase 10): over a scripted
//! multi-turn session on each provider, request N+1 repeats request N in
//! `tools`, `system` and every earlier message once `cache_control` is
//! removed, so the provider's prompt cache can hit.

mod agent_support;
mod antigravity_support;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_support::{echo_registry, env, key_auth, text_reply, tool_reply};
use fast_harness_core::agent::{Agent, AgentParts, Context, Conversation};
use fast_harness_core::auth::{AuthSource, StaticAuth};
use fast_harness_core::config::Config;
use fast_harness_core::events::TurnOutcome;
use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use fast_harness_core::llm::anthropic::Anthropic;
use fast_harness_core::llm::antigravity::provider::session_id_of;
use fast_harness_core::llm::Provider;
use serde_json::{json, Value};

const TURNS: usize = 3;

/// What changes between turns (docs/efficiency-plan.md phase 11).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Change {
    Nothing,
    /// A new AGENTS.md text before every later turn.
    AgentsMd,
    /// The date passes midnight before every later turn.
    Date,
}

struct Case {
    name: &'static str,
    provider: Arc<dyn Provider>,
    auth: Arc<dyn AuthSource>,
    model: &'static str,
    /// Part of the URL of a model call (other requests are skipped).
    path: &'static str,
    /// JSON pointers of the system prompt, the tool list and the messages.
    system: &'static str,
    tools: &'static str,
    messages: &'static str,
    /// The replies: a tool call then text in turn 1, text in later turns.
    responses: Vec<ScriptedResponse>,
}

fn claude() -> Case {
    let mut responses = vec![tool_reply(&[("toolu_1", "echo", r#"{"text":"hi"}"#)])];
    responses.extend((0..TURNS).map(|turn| text_reply(&format!("answer {turn}"))));
    Case {
        name: "anthropic",
        provider: Arc::new(Anthropic::new("http://scripted.invalid")),
        auth: key_auth(),
        model: "claude-opus-5-5",
        path: "/v1/messages",
        system: "/system",
        tools: "/tools",
        messages: "/messages",
        responses,
    }
}

fn antigravity() -> Case {
    use antigravity_support::{event, google, manifest, provider, sse, MODEL, PATH};
    let call = json!([{"functionCall": {"name": "echo", "args": {"text": "hi"}}}]);
    let mut responses = vec![manifest(), sse(&[event(call, Some("STOP"))])];
    for turn in 0..TURNS {
        let text = json!([{ "text": format!("answer {turn}") }]);
        responses.push(sse(&[event(text, Some("STOP"))]));
    }
    Case {
        name: "antigravity",
        provider: Arc::new(provider()),
        auth: Arc::new(StaticAuth::new("acct-g", "test", google())),
        model: MODEL,
        path: PATH,
        system: "/request/systemInstruction",
        tools: "/request/tools",
        messages: "/request/contents",
        responses,
    }
}

/// `value` without any `cache_control` key, at any depth.
fn strip(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| key.as_str() != "cache_control")
                .map(|(key, item)| (key.clone(), strip(item)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(strip).collect()),
        other => other.clone(),
    }
}

/// Runs [`TURNS`] turns, building the agent per turn as the front ends do
/// (`Runtime::agent`: the sources read each time, the session's frozen
/// system kept). Returns the model call bodies and the session id.
fn session(case: Case, change: Change) -> (Vec<Value>, String) {
    let env = env(&format!("prefix-{}-{change:?}", case.name));
    let transport = Arc::new(ScriptedTransport::new(case.responses));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let cancel = AtomicBool::new(false);
    for turn in 0..TURNS {
        if change == Change::AgentsMd && turn > 0 {
            std::fs::write(env.cwd.join("AGENTS.md"), format!("Rule {turn}.\n")).unwrap();
        }
        let date = match change {
            Change::Date => format!("2026-11-{}", 12 + turn),
            _ => "2026-11-12".to_string(),
        };
        let (context, _) = Context::read_at(&env.home, &env.cwd, case.model, &date);
        let parts = AgentParts {
            extensions: Default::default(),
            provider: case.provider.clone(),
            transport: transport.clone(),
            auth: case.auth.clone(),
            config: Config {
                model: case.model.into(),
                ..Config::default()
            },
            cwd: env.cwd.clone(),
            system: conv.system.clone().unwrap_or_else(|| context.system()),
        };
        let agent = Agent::new(parts, echo_registry()).with_context(context);
        let outcome = agent.run_turn(&mut conv, &format!("turn {turn}"), &cancel, &mut |_| {});
        assert_eq!(outcome, TurnOutcome::Done, "{} turn {turn}", case.name);
    }
    let bodies = transport
        .requests()
        .into_iter()
        .filter(|request| request.url.contains(case.path))
        .map(|request| strip(&serde_json::from_str(&request.body).unwrap()))
        .collect();
    (bodies, conv.id)
}

/// The messages at `pointer` without the trailing Claude effort entry
/// (`role: system`, the tail that may change on every call by design).
fn history<'a>(body: &'a Value, pointer: &str) -> &'a [Value] {
    let all = body.pointer(pointer).and_then(Value::as_array).unwrap();
    match all.split_last() {
        Some((last, rest)) if last["role"] == "system" => rest,
        _ => all,
    }
}

fn check(case: Case, change: Change) {
    let (name, pointers) = (case.name, [case.system, case.tools, case.messages]);
    let (bodies, id) = session(case, change);
    assert_eq!(bodies.len(), TURNS + 1, "{name}: model calls");
    if name == "antigravity" {
        let want = json!(session_id_of(&id));
        assert!(
            bodies.iter().all(|b| b["request"]["sessionId"] == want),
            "sessionId"
        );
    }
    let last = bodies
        .last()
        .unwrap()
        .pointer(pointers[2])
        .unwrap()
        .to_string();
    let updates = last.matches("[context update]").count();
    let want = match change {
        Change::Nothing => 0,
        _ => TURNS - 1,
    };
    assert_eq!(updates, want, "{name}: one context update per change");
    if change == Change::Date {
        assert!(
            last.contains("[context update] Today is 2026-11-14."),
            "{name}"
        );
    }
    for (n, pair) in bodies.windows(2).enumerate() {
        let (before, after) = (&pair[0], &pair[1]);
        for pointer in &pointers[..2] {
            let same = before.pointer(pointer) == after.pointer(pointer);
            assert!(same, "{name}: request {} changed {pointer}", n + 2);
        }
        let (old, new) = (history(before, pointers[2]), history(after, pointers[2]));
        assert!(new.len() > old.len(), "{name}: messages grow");
        assert_eq!(
            &new[..old.len()],
            old,
            "{name}: request {} rewrote history",
            n + 2
        );
    }
}

#[test]
fn prefix_guard_holds_without_changes() {
    check(claude(), Change::Nothing);
    check(antigravity(), Change::Nothing);
}

#[test]
fn prefix_guard_holds_when_agents_md_changes() {
    check(claude(), Change::AgentsMd);
    check(antigravity(), Change::AgentsMd);
}

#[test]
fn prefix_guard_holds_across_midnight() {
    check(claude(), Change::Date);
    check(antigravity(), Change::Date);
}
