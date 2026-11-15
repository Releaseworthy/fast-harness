//! After a compaction the context no longer holds a subagent's answer, so
//! a repeated `agent_output` returns it whole again (PLAN.md D90).

mod agent_support;
mod routed_support;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_support::*;
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::http::ScriptedTransport;
use fast_harness_core::types::Block;
use routed_support::*;

/// The text of the tool result of call `id`.
fn result_of(conv: &Conversation, id: &str) -> String {
    let blocks = conv.messages.iter().flat_map(|m| m.content.iter());
    let found = blocks.filter_map(|block| match block {
        Block::ToolResult {
            call_id, content, ..
        } if call_id == id => Some(content.clone()),
        _ => None,
    });
    found.last().unwrap_or_default()
}

#[test]
fn a_compaction_lets_agent_output_return_the_answer_again() {
    let env = env("sub-output-compact");
    let bg = r#"{"prompt":"task A","background":true}"#;
    let find = r#"{"names":["subagent","agent_output"]}"#;
    let output = r#"{"id":"bg1","wait":true}"#;
    let transport = Routed::new(vec![
        ("task A", 300, vec![text_reply("the long answer A")]),
        (
            "go",
            0,
            vec![
                tool_reply(&[("t0", "find_tools", find)]),
                tool_reply(&[("t1", "subagent", bg)]),
                tool_reply(&[("t2", "agent_output", output)]),
                tool_reply(&[("t3", "agent_output", output)]),
                text_reply("done"),
                text_reply("summary: we go on with bg1"),
                tool_reply(&[("t4", "agent_output", output)]),
                text_reply("done again"),
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
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let agent = Agent::with_subagent(parts, &env.home, &conv.id);
    let cancel = AtomicBool::new(false);
    agent.run_turn(&mut conv, "go", &cancel, &mut |_| {});
    assert_eq!(result_of(&conv, "t3"), "[already returned in call t2]");
    agent.run_turn(&mut conv, "/compact", &cancel, &mut |_| {});
    agent.run_turn(&mut conv, "go on", &cancel, &mut |_| {});
    let again = result_of(&conv, "t4");
    assert!(again.starts_with("the long answer A\n"), "{again}");
}
