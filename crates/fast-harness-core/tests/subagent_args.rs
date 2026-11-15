//! The `subagent` call's answer budget and `tools` names (2026-11-15):
//! a long final answer reaches the parent whole, and `tools` takes the
//! names in any letter case (Claude OAuth shows `Read`, the registry `read`).

mod agent_support;
mod routed_support;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_support::*;
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::events::TurnOutcome;
use fast_harness_core::http::ScriptedTransport;
use routed_support::*;

/// Runs a parent whose second step calls `subagent` with `call`; the child
/// and the parent then take `rest`. Returns the transport and the env.
fn run(
    tag: &str,
    call: &str,
    rest: Vec<fast_harness_core::http::ScriptedResponse>,
) -> (Arc<ScriptedTransport>, Env, String) {
    let env = env(tag);
    let mut responses = vec![
        tool_reply(&[("t0", "find_tools", FIND_SUBAGENT)]),
        tool_reply(&[("t1", "subagent", call)]),
    ];
    responses.extend(rest);
    let transport = Arc::new(ScriptedTransport::new(responses));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let agent = Agent::with_subagent(parts(&transport, key_auth(), &env.cwd), &env.home, &conv.id);
    let outcome = agent.run_turn(&mut conv, "go", &AtomicBool::new(false), &mut |_| {});
    assert_eq!(outcome, TurnOutcome::Done);
    (transport, env, conv.id)
}

#[test]
fn a_20_kb_answer_reaches_the_parent_whole() {
    let answer = format!("{}END-OF-ANSWER", "abcdefghij".repeat(2_000));
    let rest = vec![text_reply(&answer), text_reply("parent done")];
    let (transport, _env, _) = run(
        "sub-long",
        r#"{"prompt":"report","description":"long"}"#,
        rest,
    );
    let requests = transport.requests();
    assert_eq!(requests.len(), 4);
    let body = &requests[3].body;
    assert!(body.contains(&answer), "the whole answer, not an 8 KB head");
    assert!(!body.contains("output cut") && !body.contains("read it with offset"));
}

#[test]
fn tools_take_any_letter_case_and_name_the_valid_ones_on_error() {
    let call = r#"{"prompt":"look","description":"case","tools":["Read","GREP"]}"#;
    let rest = vec![text_reply("child done"), text_reply("parent done")];
    let (transport, env, parent) = run("sub-case", call, rest);
    let requests = transport.requests();
    assert_eq!(requests.len(), 4, "the child ran");
    assert!(!requests[3].body.contains("unknown"));
    let child = &child_logs(&env, &parent)[0].0.id;
    let resumed = Conversation::resume(&env.home, &env.cwd, child).unwrap();
    let found = fast_harness_core::llm::find::found(&resumed.messages);
    assert_eq!(
        found,
        ["read", "grep"],
        "preloaded as the registry spells them"
    );

    let call = r#"{"prompt":"look","tools":["Read","nope"]}"#;
    let (transport, _env, _) = run("sub-unknown", call, vec![text_reply("parent done")]);
    let requests = transport.requests();
    assert_eq!(requests.len(), 3, "no child request");
    let body = &requests[2].body;
    assert!(body.contains("unknown nope"), "{body}");
    assert!(
        body.contains("read, ") && body.contains("grep"),
        "the valid names"
    );
}
