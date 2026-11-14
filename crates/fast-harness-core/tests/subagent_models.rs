//! `subagent.models` and `subagent.guide` (PLAN.md D82) on the wire: the
//! `subagent` tool's `model` enum lists only enabled models, its description
//! ends with the guide, and a call for a disabled model is a tool error.

mod agent_support;
mod routed_support;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_support::*;
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::events::TurnOutcome;
use fast_harness_core::http::ScriptedTransport;
use routed_support::*;
use serde_json::Value;

/// The `subagent` tool as one request body lists it.
fn subagent_tool(body: &Value) -> Value {
    let tools = body["tools"].as_array().cloned().unwrap_or_default();
    let found = tools.into_iter().find(|t| t["name"] == "subagent");
    found.expect("the subagent tool is listed")
}

#[test]
fn only_enabled_models_are_offered_and_a_disabled_one_is_refused() {
    let env = env("sub-models");
    let call = r#"{"prompt":"fix it","model":"claude-opus-5-5"}"#;
    let responses = vec![
        tool_reply(&[("t0", "find_tools", FIND_SUBAGENT)]),
        tool_reply(&[("t1", "subagent", call)]),
        text_reply("parent done"),
    ];
    let transport = Arc::new(ScriptedTransport::new(responses));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let mut parts = parts(&transport, key_auth(), &env.cwd);
    let sub = &mut parts.config.subagent;
    sub.models.insert("claude-opus-5-5".into(), false);
    sub.guide = "Pick by task:\n- Haiku reads news.\n- Opus does hard work.".into();
    let agent = Agent::with_subagent(parts, &env.home, &conv.id);
    let outcome = agent.run_turn(&mut conv, "go", &AtomicBool::new(false), &mut |_| {});
    assert_eq!(outcome, TurnOutcome::Done);
    let requests = transport.requests();
    assert_eq!(requests.len(), 3, "no child request was made");
    let tool = subagent_tool(&json(&requests[1].body));
    let models = tool["input_schema"]["properties"]["model"]["enum"].clone();
    let models: Vec<&str> = models
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(!models.contains(&"claude-opus-5-5"), "{models:?}");
    assert!(models.contains(&"claude-haiku-5-5"), "{models:?}");
    let description = tool["description"].as_str().unwrap();
    // Other Opus models stay enabled, so the Opus line stays.
    assert!(
        description.ends_with("\n\nPick by task:\n- Haiku reads news.\n- Opus does hard work."),
        "{description:?}"
    );
    let last = &requests[2].body;
    assert!(
        last.contains("`claude-opus-5-5` is disabled for subagents"),
        "{last}"
    );
}
