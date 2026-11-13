//! Subagents v2 (PLAN.md D40): preloaded tools, depth limit, the shared
//! prefix, model and effort, background children and cancel.

mod agent_support;
mod routed_support;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use agent_support::*;
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::events::{AgentEvent, TurnOutcome};
use fast_harness_core::http::ScriptedTransport;
use fast_harness_core::types::Role;
use routed_support::*;
use serde_json::Value;

/// `parts` with `maxDepth` 2: these tests nest children (the default 1
/// gives children no `subagent` tool).
fn deep(mut parts: fast_harness_core::agent::AgentParts) -> fast_harness_core::agent::AgentParts {
    parts.config.subagent.max_depth = 2;
    parts
}

#[test]
fn preloaded_tools_need_no_find_and_keep_the_parent_prefix() {
    let env = env("sub-preload");
    std::fs::write(env.cwd.join("a.txt"), "alpha").unwrap();
    let call = r#"{"prompt":"read a.txt","tools":["read"]}"#;
    let responses = vec![
        tool_reply(&[("t0", "find_tools", FIND_SUBAGENT)]),
        tool_reply(&[("t1", "subagent", call)]),
        tool_reply(&[("c1", "read", r#"{"path":"a.txt"}"#)]),
        text_reply("child read alpha"),
        text_reply("parent done"),
    ];
    let transport = Arc::new(ScriptedTransport::new(responses));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let agent = Agent::with_subagent(
        deep(parts(&transport, key_auth(), &env.cwd)),
        &env.home,
        &conv.id,
    );
    let outcome = agent.run_turn(&mut conv, "go", &AtomicBool::new(false), &mut |_| {});
    assert_eq!(outcome, TurnOutcome::Done);
    let requests = transport.requests();
    assert_eq!(requests.len(), 5, "the child made two calls, no find step");
    let (parent, child) = (json(&requests[0].body), json(&requests[2].body));
    // Same model, system and tool list: the child's prefix is the parent's.
    assert_eq!(parent["tools"], child["tools"]);
    assert_eq!(parent["system"], child["system"]);
    assert!(
        requests[2]
            .body
            .contains(r#"{"tool_name":"read","type":"tool_reference"}"#)
            || requests[2].body.contains(r#""type":"tool_reference""#)
    );
    let logs = child_logs(&env, &conv.id);
    let (header, events) = &logs[0];
    assert_eq!(header.depth, 1);
    let results: Vec<&Value> = events
        .iter()
        .filter(|e| e.kind == "tool/result")
        .map(|e| &e.data["message"]["content"][0])
        .collect();
    assert_eq!(results.len(), 2, "the preload result and the read");
    assert!(results[1]["content"].as_str().unwrap().contains("alpha"));
    assert!(!results[1]["is_error"].as_bool().unwrap_or(false));
    let resumed = Conversation::resume(&env.home, &env.cwd, &header.id).unwrap();
    assert_eq!(
        fast_harness_core::llm::find::found(&resumed.messages),
        ["read"]
    );
}

#[test]
fn a_child_at_max_depth_has_no_subagent_tool() {
    let env = env("sub-depth");
    let responses = vec![
        tool_reply(&[("t0", "find_tools", FIND_SUBAGENT)]),
        tool_reply(&[("t1", "subagent", r#"{"prompt":"leaf"}"#)]),
        text_reply("leaf done"),
        text_reply("parent done"),
    ];
    let transport = Arc::new(ScriptedTransport::new(responses));
    let mut parts = parts(&transport, key_auth(), &env.cwd);
    parts.config.subagent.max_depth = 1;
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let agent = Agent::with_subagent(parts, &env.home, &conv.id);
    agent.run_turn(&mut conv, "go", &AtomicBool::new(false), &mut |_| {});
    let requests = transport.requests();
    assert!(requests[0].body.contains(r#""name":"subagent""#));
    assert!(!requests[2].body.contains(r#""name":"subagent""#));
    assert!(!requests[2].body.contains(r#""name":"agent_output""#));
    assert_eq!(child_logs(&env, &conv.id)[0].0.depth, 1);
}

#[test]
fn max_depth_zero_turns_subagents_off() {
    let env = env("sub-off");
    let responses = vec![
        tool_reply(&[("t0", "find_tools", FIND_SUBAGENT)]),
        tool_reply(&[("t1", "subagent", r#"{"prompt":"leaf"}"#)]),
        text_reply("parent done"),
    ];
    let transport = Arc::new(ScriptedTransport::new(responses));
    let mut parts = parts(&transport, key_auth(), &env.cwd);
    parts.config.subagent.max_depth = 0;
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let agent = Agent::with_subagent(parts, &env.home, &conv.id);
    agent.run_turn(&mut conv, "go", &AtomicBool::new(false), &mut |_| {});
    let requests = transport.requests();
    assert_eq!(requests.len(), 3, "no child request");
    // The root keeps the tool (its list is frozen, D36); the call is refused.
    assert!(requests[2].body.contains("subagents are off at this depth"));
    assert!(requests[2].body.contains(r#""is_error":true"#));
    assert!(child_logs(&env, &conv.id).is_empty(), "no child session");
}
#[test]
fn model_and_effort_override_the_parent() {
    let env = env("sub-model");
    let sonnet = r#"{"prompt":"one","model":"claude-sonnet-5"}"#;
    let low = r#"{"prompt":"two","effort":"low"}"#;
    let google = r#"{"prompt":"three","model":"antigravity/gemini-3-pro"}"#;
    let responses = vec![
        tool_reply(&[("t0", "find_tools", FIND_SUBAGENT)]),
        tool_reply(&[("t1", "subagent", sonnet)]),
        text_reply("one done"),
        tool_reply(&[("t2", "subagent", low)]),
        text_reply("two done"),
        tool_reply(&[("t3", "subagent", google)]),
        text_reply("parent done"),
    ];
    let transport = Arc::new(ScriptedTransport::new(responses));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let agent = Agent::with_subagent(
        deep(parts(&transport, key_auth(), &env.cwd)),
        &env.home,
        &conv.id,
    );
    agent.run_turn(&mut conv, "go", &AtomicBool::new(false), &mut |_| {});
    let requests = transport.requests();
    let parent = json(&requests[0].body);
    let (one, two) = (json(&requests[2].body), json(&requests[4].body));
    assert_eq!(parent["model"], "claude-opus-5-5");
    assert_eq!(one["model"], "claude-sonnet-5");
    assert_eq!(two["model"], "claude-opus-5-5");
    // Opus 5.5 takes an effort change as a system message, keeping the
    // top-level `output_config` of the cached prefix.
    let low = r#""output_config":{"effort":"low"}"#;
    assert!(requests[4].body.contains(low));
    assert!(!requests[0].body.contains(low));
    assert!(two["tools"] == parent["tools"]);
    // A retired `antigravity/<id>` model (PLAN.md D53) is an error result.
    let last = conv
        .messages
        .iter()
        .rev()
        .find(|m| m.role == Role::Tool)
        .unwrap();
    assert!(
        format!("{last:?}").contains("branch `antigravity`"),
        "{last:?}"
    );
    // The popover's detail line (D79) reads them back from the child logs.
    let kids = fast_harness_core::session::children(&env.home, &env.cwd, &conv.id).unwrap();
    let seen: Vec<_> = kids
        .iter()
        .map(|k| (k.model.as_str(), k.effort.as_deref()))
        .collect();
    assert_eq!(
        &seen[..2],
        &[
            ("claude-sonnet-5", Some("high")),
            ("claude-opus-5-5", Some("low"))
        ]
    );
    assert!(kids[..2].iter().all(|k| k.tokens > 0), "{kids:?}");
}

#[test]
fn background_children_answer_in_order_of_completion() {
    let env = env("sub-bg");
    let a = r#"{"prompt":"task A","background":true}"#;
    let b = r#"{"prompt":"task B","background":true}"#;
    let transport = Routed::new(vec![
        ("task A", 1200, vec![text_reply("answer A")]),
        ("task B", 300, vec![text_reply("answer B")]),
        (
            "go",
            0,
            vec![
                tool_reply(&[("t0", "find_tools", FIND_SUBAGENT)]),
                tool_reply(&[("t1", "subagent", a), ("t2", "subagent", b)]),
                text_reply("waiting"),
                text_reply("got one"),
                text_reply("got two"),
                text_reply("spare"),
            ],
        ),
    ]);
    let mut parts = parts(
        &Arc::new(ScriptedTransport::new(vec![])),
        key_auth(),
        &env.cwd,
    );
    parts.transport = transport.clone();
    parts.config.subagent.max_depth = 2;
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let agent = Agent::with_subagent(parts, &env.home, &conv.id);
    let mut seen = Vec::new();
    let outcome = agent.run_turn(&mut conv, "go", &AtomicBool::new(false), &mut |e| {
        seen.push(e)
    });
    assert_eq!(outcome, TurnOutcome::Done);
    let users: String = conv
        .messages
        .iter()
        .filter(|m| m.role != Role::Assistant)
        .map(|m| format!("{m:?}"))
        .collect();
    let (pos_a, pos_b) = (
        users.find("answer A").unwrap(),
        users.find("answer B").unwrap(),
    );
    assert!(pos_b < pos_a, "B ended first");
    assert!(users.contains("finished] answer B"));
    let ends = seen
        .iter()
        .filter(|e| matches!(e, AgentEvent::Subagent { running: false, .. }))
        .count();
    assert_eq!(ends, 2);
    assert!(seen
        .iter()
        .any(|e| matches!(e, AgentEvent::SubagentProgress { .. })));
}

#[test]
fn cancel_reaches_every_descendant() {
    let env = env("sub-cancel");
    let bg = r#"{"prompt":"task A","background":true}"#;
    let fg = r#"{"prompt":"task G"}"#;
    let transport = Routed::new(vec![
        (
            "task A",
            0,
            vec![
                tool_reply(&[("a0", "find_tools", FIND_SUBAGENT)]),
                tool_reply(&[("a1", "subagent", fg)]),
                text_reply("A late"),
            ],
        ),
        (
            "task G",
            1500,
            vec![tool_reply(&[("g0", "find_tools", r#"{"names":["read"]}"#)])],
        ),
        (
            "go",
            0,
            vec![
                tool_reply(&[("t0", "find_tools", FIND_SUBAGENT)]),
                tool_reply(&[("t1", "subagent", bg)]),
                text_reply("waiting"),
            ],
        ),
    ]);
    let mut parts = parts(
        &Arc::new(ScriptedTransport::new(vec![])),
        key_auth(),
        &env.cwd,
    );
    parts.transport = transport.clone();
    parts.config.subagent.max_depth = 2;
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let agent = Agent::with_subagent(parts, &env.home, &conv.id);
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(700));
        flag.store(true, Ordering::Relaxed);
    });
    let mut started = 0;
    agent.run_turn(&mut conv, "go", &cancel, &mut |e| {
        if matches!(e, AgentEvent::Subagent { running: true, .. }) {
            started += 1;
        }
    });
    stopper.join().unwrap();
    assert_eq!(started, 2, "the child and the grandchild reached the root");
    let logs = child_logs(&env, &conv.id);
    assert_eq!(logs.len(), 2);
    let mut depths: Vec<u32> = logs.iter().map(|(h, _)| h.depth).collect();
    depths.sort();
    assert_eq!(depths, [1, 2]);
    for (header, events) in &logs {
        assert_eq!(outcome(events), "cancelled", "depth {}", header.depth);
    }
    assert_eq!(agent.children.as_ref().unwrap().running_background(), 0);
}

#[test]
fn the_view_keeps_one_note_per_child_session() {
    use fast_harness_core::view::{Item, Transcript};
    let mut view = Transcript::new();
    let progress = |line: &str| AgentEvent::SubagentProgress {
        session: "s1".into(),
        line: line.into(),
    };
    view.apply(&AgentEvent::Subagent {
        session: "s1".into(),
        running: true,
    });
    view.apply(&progress("look · step 2 · read"));
    view.apply(&AgentEvent::Subagent {
        session: "s2".into(),
        running: true,
    });
    view.apply(&AgentEvent::Subagent {
        session: "s1".into(),
        running: false,
    });
    let notes: Vec<&str> = view
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Note { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(notes, ["Subagent s1 finished", "Subagent s2 started"]);
}

/// A resumed session's child takes the session's frozen system prompt, not
/// the freshly rendered one its parts were built with (phase 15 fix, D41).
#[test]
fn a_resumed_sessions_child_takes_the_frozen_system() {
    let env = env("sub-frozen");
    let call = r#"{"prompt":"say hi"}"#;
    let responses = vec![
        text_reply("first turn"),
        tool_reply(&[("t0", "find_tools", FIND_SUBAGENT)]),
        tool_reply(&[("t1", "subagent", call)]),
        text_reply("child hi"),
        text_reply("parent done"),
    ];
    let transport = Arc::new(ScriptedTransport::new(responses));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let mut first = parts(&transport, key_auth(), &env.cwd);
    first.system = "FROZEN-SYSTEM".into();
    let agent = Agent::with_subagent(first, &env.home, &conv.id);
    let cancel = AtomicBool::new(false);
    assert_eq!(
        agent.run_turn(&mut conv, "one", &cancel, &mut |_| {}),
        TurnOutcome::Done
    );
    let mut resumed = Conversation::resume(&env.home, &env.cwd, &conv.id).unwrap();
    let mut fresh = parts(&transport, key_auth(), &env.cwd);
    fresh.system = "FRESH-SYSTEM".into();
    let agent = Agent::with_subagent(fresh, &env.home, &resumed.id);
    let outcome = agent.run_turn(&mut resumed, "two", &cancel, &mut |_| {});
    assert_eq!(outcome, TurnOutcome::Done);
    let requests = transport.requests();
    assert_eq!(requests.len(), 5);
    let child = json(&requests[3].body)["system"].to_string();
    assert!(child.contains("FROZEN-SYSTEM"), "{child}");
    assert!(!child.contains("FRESH-SYSTEM"));
}
