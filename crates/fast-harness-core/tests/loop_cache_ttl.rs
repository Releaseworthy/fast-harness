//! The main session's cache lifetimes (PLAN.md D88): with `cacheTtl: auto`
//! in the TUI or GUI, tools and system keep 1 hour, a turn's first step
//! caches its messages for 1 hour, a step inside the tool loop for 5
//! minutes behind a 1 hour breakpoint on the turn's user message; an
//! explicit `cacheTtl: "1h"` keeps every breakpoint at 1 hour.

mod agent_support;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_support::{env, key_auth, parts, text_reply, tool_reply, Echo};
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::config::CacheTtl;
use fast_harness_core::http::ScriptedTransport;
use fast_harness_core::tools::Registry;
use serde_json::Value;

/// Two turns: the first calls `echo` once, then answers.
fn sent(ttl: Option<CacheTtl>) -> Vec<Value> {
    let env = env(&format!("loop-ttl-{ttl:?}"));
    let echo = r#"{"text":"hi"}"#;
    let script = vec![
        tool_reply(&[("c1", "echo", echo)]),
        text_reply("done"),
        text_reply("again"),
    ];
    let transport = Arc::new(ScriptedTransport::new(script));
    let mut parts = parts(&transport, key_auth(), &env.cwd);
    parts.config.interactive = true;
    parts.config.cache_ttl = ttl;
    let mut tools = Registry::new();
    tools.add(Box::new(Echo));
    let agent = Agent::new(parts, Arc::new(tools));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let cancel = AtomicBool::new(false);
    for text in ["one", "two"] {
        agent.run_turn(&mut conv, text, &cancel, &mut |_| {});
    }
    (transport.requests().iter())
        .filter(|r| r.url.contains("/messages"))
        .map(|r| serde_json::from_str(&r.body).unwrap())
        .collect()
}

/// Each breakpoint in prompt order (tools, system, messages): `(where,
/// ttl)`, `where` the message index or -1 before the messages.
fn breakpoints(body: &Value) -> Vec<(i64, String)> {
    let mut out = Vec::new();
    let mut walk = |at: i64, value: &Value| {
        let blocks = value.as_array().into_iter().flatten();
        let content = |b: &Value| b["content"].as_array().cloned().unwrap_or_default();
        for block in blocks.flat_map(|b| [vec![b.clone()], content(b)].concat()) {
            if let Some(cache) = block.get("cache_control") {
                let ttl = cache["ttl"].as_str().unwrap_or("5m").to_string();
                out.push((at, ttl));
            }
        }
    };
    walk(-1, &body["tools"]);
    walk(-1, &body["system"]);
    let messages = body["messages"].as_array().cloned().unwrap_or_default();
    for (index, message) in messages.iter().enumerate() {
        walk(index as i64, &Value::Array(vec![message.clone()]));
    }
    out
}

fn ttls(body: &Value) -> Vec<String> {
    breakpoints(body).into_iter().map(|(_, ttl)| ttl).collect()
}

#[test]
fn auto_caches_the_tool_loop_for_5_minutes_and_turn_starts_for_an_hour() {
    let bodies = sent(None);
    assert_eq!(bodies.len(), 3);
    for body in &bodies {
        let marks = breakpoints(body);
        assert!(marks.len() <= 4, "{marks:?}");
        let order = ttls(body);
        let first_5m = order.iter().position(|t| t == "5m").unwrap_or(order.len());
        assert!(order[first_5m..].iter().all(|t| t == "5m"), "{order:?}");
    }
    // Step 1: the first user message, 1 hour.
    assert_eq!(breakpoints(&bodies[0]).last(), Some(&(0, "1h".into())));
    // Step 2 answers the tool result: 5 minutes, the turn's message 1 hour.
    let loop_marks = breakpoints(&bodies[1]);
    let last = loop_marks.last().map(|m| m.1.as_str());
    assert_eq!(last, Some("5m"), "{loop_marks:?}");
    assert!(loop_marks.contains(&(0, "1h".into())), "{loop_marks:?}");
    // Turn 2 starts: 1 hour on its message and on turn 1's.
    let next = breakpoints(&bodies[2]);
    assert_eq!(next.last().map(|m| m.1.as_str()), Some("1h"), "{next:?}");
    assert!(next.contains(&(0, "1h".into())), "{next:?}");
}

#[test]
fn an_explicit_hour_keeps_every_breakpoint_at_an_hour() {
    for body in sent(Some(CacheTtl::Long)) {
        assert!(ttls(&body).iter().all(|t| t == "1h"), "{body}");
    }
    for body in sent(Some(CacheTtl::Short)) {
        assert!(ttls(&body).iter().all(|t| t == "5m"), "{body}");
    }
}
