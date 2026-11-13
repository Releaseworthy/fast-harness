//! Approvals in the flow (PLAN.md D64): the gate sees a PreToolUse hook's
//! rewritten arguments and the call runs with them once it is allowed, and a
//! subagent's call asks the parent's gate and sink, named by the child, with
//! an answer that a run and a decline both honour.

mod agent_support;
mod hooks_support;

use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_support::{env, key_auth, parts, text_reply, tool_reply, Env};
use fast_harness_core::agent::{Agent, Answer, Approver, Conversation, Gate};
use fast_harness_core::config::approvals::Approvals;
use fast_harness_core::config::layers::Layers;
use fast_harness_core::config::Config;
use fast_harness_core::events::{AgentEvent, ApprovalRequest, TurnOutcome};
use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use fast_harness_core::tools::hooks::Hooks;
use fast_harness_core::tools::{self, Effect, Extensions};
use hooks_support::{bodies, capture, entry, read_json, write_hooks};
use serde_json::{json, Value};

/// The gate of a front end that shows requests and answers them.
fn answering() -> Arc<Gate> {
    let gate = Arc::new(Gate::default());
    gate.set_answering(true);
    gate
}

/// One turn whose sink gives each request the next answer of `plan`, as a
/// front end does; every event reaches `seen`.
fn approving_turn(
    agent: &Agent,
    conv: &mut Conversation,
    gate: &Arc<Gate>,
    plan: Vec<Answer>,
) -> (TurnOutcome, Vec<AgentEvent>) {
    let cancel = AtomicBool::new(false);
    let mut plan: VecDeque<Answer> = plan.into();
    let mut seen: Vec<AgentEvent> = Vec::new();
    let outcome = agent.run_turn(conv, "go", &cancel, &mut |event| {
        if let AgentEvent::ApprovalRequest(request) = &event {
            if let Some(answer) = plan.pop_front() {
                assert!(gate.answer(&request.id, answer), "{} waits", request.id);
            }
        }
        seen.push(event);
    });
    (outcome, seen)
}

fn requests(seen: &[AgentEvent]) -> Vec<ApprovalRequest> {
    (seen.iter())
        .filter_map(|event| match event {
            AgentEvent::ApprovalRequest(request) => Some(request.clone()),
            _ => None,
        })
        .collect()
}

/// The word of each answer, in order.
fn done_words(seen: &[AgentEvent]) -> Vec<String> {
    (seen.iter())
        .filter_map(|event| match event {
            AgentEvent::ApprovalDone { answer, .. } => Some(answer.clone()),
            _ => None,
        })
        .collect()
}

/// The arguments of a request as JSON.
fn args_of(request: &ApprovalRequest) -> Value {
    serde_json::from_str(&request.arguments).expect("the request carries JSON arguments")
}

/// The session id of the child the parent started.
fn child_session(seen: &[AgentEvent]) -> String {
    (seen.iter())
        .find_map(|event| match event {
            AgentEvent::Subagent {
                session,
                running: true,
            } => Some(session.clone()),
            _ => None,
        })
        .expect("the parent started a child")
}

/// An agent on the built-in tools with the hooks of `home`, asking.
fn hooked(env: &Env, transport: &Arc<ScriptedTransport>) -> Agent {
    let mut parts = parts(transport, key_auth(), &env.cwd);
    parts.config = Config {
        approvals: Approvals::Ask,
        ..Config::default()
    };
    let (hooks, skipped) = Hooks::load(&Layers::user_only(&env.home));
    assert!(skipped.is_empty(), "{skipped:?}");
    parts.extensions = Arc::new(Extensions::default().with_hooks(hooks));
    Agent::new(parts, Arc::new(tools::builtin()))
}

#[test]
fn the_gate_shows_a_hook_rewrite_and_the_tool_runs_with_it() {
    let env = env("approvals-hook-rewrite");
    let reply = r#"{"arguments":{"path":"hook.txt","content":"from the hook"}}"#;
    let (hook_saw, script) = capture(&env.root.0, "pre", Some(reply));
    write_hooks(
        &env.home,
        json!({"PreToolUse": [entry(Some("write"), script, 20_000)]}),
    );
    let original = r#"{"path":"model.txt","content":"from the model"}"#;
    let transport = Arc::new(ScriptedTransport::new(vec![
        tool_reply(&[("f1", "find_tools", r#"{"names":["write"]}"#)]),
        tool_reply(&[("c1", "write", original)]),
        text_reply("done"),
    ]));
    let mut agent = hooked(&env, &transport);
    let gate = answering();
    agent.set_approver(Approver::new(Arc::clone(&gate)));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let (outcome, seen) = approving_turn(&agent, &mut conv, &gate, vec![Answer::Once]);
    assert_eq!(outcome, TurnOutcome::Done);
    let hook_input = read_json(&hook_saw);
    assert_eq!(
        hook_input["arguments"]["path"], "model.txt",
        "the hook reads the model's own call"
    );
    let asked = requests(&seen);
    assert_eq!(asked.len(), 1, "only the write asks: {asked:?}");
    let request = &asked[0];
    assert_eq!(
        (request.call_id.as_str(), request.tool.as_str()),
        ("c1", "write")
    );
    assert_eq!(request.effect, Effect::Write);
    let args = args_of(request);
    assert_eq!(
        (args["path"].as_str(), args["content"].as_str()),
        (Some("hook.txt"), Some("from the hook")),
        "the request shows the rewritten arguments"
    );
    assert!(request.summary.ends_with("hook.txt"), "{}", request.summary);
    assert!(!request.arguments.contains("model.txt"));
    assert_eq!(request.agent, None, "the session itself asks");
    assert_eq!(done_words(&seen), ["once"]);
    assert_eq!(
        std::fs::read_to_string(env.cwd.join("hook.txt")).unwrap(),
        "from the hook",
        "the allowed call ran with the rewritten arguments"
    );
    assert!(!env.cwd.join("model.txt").exists(), "not the model's");
}

/// The parent's two steps and the child's two; the child is described
/// (`description` is its label) as `writer`, writes `sub.txt`, then says
/// `answer`.
fn child_script(answer: &str) -> Vec<ScriptedResponse> {
    let call = r#"{"prompt":"write the file","description":"writer","tools":["write"]}"#;
    let write = r#"{"path":"sub.txt","content":"from the child"}"#;
    vec![
        tool_reply(&[("t0", "find_tools", r#"{"names":["subagent"]}"#)]),
        tool_reply(&[("t1", "subagent", call)]),
        tool_reply(&[("c1", "write", write)]),
        text_reply(answer),
        text_reply("parent done"),
    ]
}

/// A parent whose children ask through `gate`, the parent turn's own gate.
fn parent(
    env: &Env,
    transport: &Arc<ScriptedTransport>,
    conv: &Conversation,
    gate: &Arc<Gate>,
) -> Agent {
    let mut parts = parts(transport, key_auth(), &env.cwd);
    parts.config = Config {
        approvals: Approvals::Ask,
        ..Config::default()
    };
    let mut agent = Agent::with_subagent(parts, &env.home, &conv.id);
    agent.set_approver(Approver::new(Arc::clone(gate)));
    agent
}

#[test]
fn a_childs_call_reaches_the_parents_gate_and_runs_when_allowed() {
    let env = env("approvals-child-allow");
    let transport = Arc::new(ScriptedTransport::new(child_script("child wrote it")));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let gate = answering();
    let agent = parent(&env, &transport, &conv, &gate);
    let (outcome, seen) = approving_turn(&agent, &mut conv, &gate, vec![Answer::Once]);
    assert_eq!(outcome, TurnOutcome::Done);
    let named = format!("writer ({})", child_session(&seen));
    let asked = requests(&seen);
    assert_eq!(
        asked.len(),
        1,
        "the parent's own calls never ask: {asked:?}"
    );
    let request = &asked[0];
    assert_eq!(request.tool, "write");
    assert_eq!(request.effect, Effect::Write);
    assert_eq!(args_of(request)["path"], "sub.txt");
    assert_eq!(request.agent.as_deref(), Some(named.as_str()));
    assert_eq!(done_words(&seen), ["once"]);
    assert_eq!(
        std::fs::read_to_string(env.cwd.join("sub.txt")).unwrap(),
        "from the child"
    );
    let bodies = bodies(&transport);
    assert!(
        bodies
            .iter()
            .any(|b| b["messages"].to_string().contains("to sub.txt")),
        "the child read its write"
    );
    assert!(bodies.last().unwrap()["messages"]
        .to_string()
        .contains("child wrote it"));
}

#[test]
fn a_declined_child_call_runs_nothing_and_the_child_reads_it() {
    let env = env("approvals-child-decline");
    let answer = "I could not write it";
    let transport = Arc::new(ScriptedTransport::new(child_script(answer)));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let gate = answering();
    let agent = parent(&env, &transport, &conv, &gate);
    let plan = vec![Answer::No(Some("not this one".into()))];
    let (outcome, seen) = approving_turn(&agent, &mut conv, &gate, plan);
    assert_eq!(outcome, TurnOutcome::Done, "the child still answers");
    let asked = requests(&seen);
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(asked[0]
        .agent
        .as_deref()
        .is_some_and(|agent| agent.starts_with("writer (")));
    assert_eq!(done_words(&seen), ["declined"]);
    assert!(!env.cwd.join("sub.txt").exists(), "the call did not run");
    let bodies = bodies(&transport);
    let declined = "not run: the user declined this call: not this one";
    assert!(
        bodies
            .iter()
            .any(|b| b["messages"].to_string().contains(declined)),
        "the child saw the declined result"
    );
    assert!(bodies.last().unwrap()["messages"]
        .to_string()
        .contains(answer));
}
