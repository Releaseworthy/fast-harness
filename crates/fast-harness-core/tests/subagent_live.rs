//! Subagent sessions are live (PLAN.md D79): stop, steer, pause and the
//! session lock of a running child.

mod agent_support;
mod routed_support;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_support::*;
use fast_harness_core::agent::{live, Agent, Conversation};
use fast_harness_core::events::{AgentEvent, TurnOutcome};
use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use fast_harness_core::session;
use routed_support::*;

/// One foreground child ("task L", 600 ms); `act` gets its id at its start.
fn live_child(name: &str, child: Vec<ScriptedResponse>, act: &dyn Fn(&Env, &str)) -> Live {
    let env = env(name);
    let transport = Routed::new(vec![
        ("task L", 600, child),
        (
            "go",
            0,
            vec![
                tool_reply(&[("t0", "find_tools", FIND_SUBAGENT)]),
                tool_reply(&[("t1", "subagent", r#"{"prompt":"task L"}"#)]),
                text_reply("root done"),
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
    let mut acted = false;
    let root = agent.run_turn(&mut conv, "go", &AtomicBool::new(false), &mut |e| {
        if let AgentEvent::Subagent {
            session,
            running: true,
        } = &e
        {
            assert!(live::running(session), "the child is live while it runs");
            act(&env, session);
            acted = true;
        }
    });
    assert!(acted, "the child started");
    let logs = child_logs(&env, &conv.id);
    assert_eq!(logs.len(), 1);
    let child = outcome(&logs[0].1);
    let bodies = transport.bodies.lock().unwrap().clone();
    let listed = session::top_level(session::list(&env.home, &env.cwd).unwrap());
    let listed = listed.into_iter().map(|s| s.id).collect();
    Live {
        root,
        child,
        bodies,
        listed,
        root_id: conv.id,
    }
}

struct Live {
    root: TurnOutcome,
    child: String,
    bodies: Vec<String>,
    listed: Vec<String>,
    root_id: String,
}

#[test]
fn stop_of_a_live_child_stops_only_it() {
    let child = vec![tool_reply(&[("c0", "find_tools", r#"{"names":["read"]}"#)])];
    let run = live_child("live-stop", child, &|_, id| assert!(live::stop(id)));
    assert_eq!(run.child, "cancelled");
    assert_eq!(run.root, TurnOutcome::Done, "the root went on");
    assert_eq!(run.listed, vec![run.root_id.clone()]);
}

#[test]
fn a_steer_and_a_pause_reach_the_live_child() {
    let child = vec![
        tool_reply(&[("c0", "find_tools", r#"{"names":["read"]}"#)]),
        text_reply("child done"),
    ];
    let run = live_child("live-steer", child, &|_, id| {
        assert!(live::steer(id, "use grep"))
    });
    assert_eq!(run.child, "done");
    let steered = run
        .bodies
        .iter()
        .filter(|b| b.contains("task L") && b.contains("use grep"));
    assert_eq!(
        steered.count(),
        1,
        "the child's second request carries the steer"
    );
    let child = vec![tool_reply(&[("c0", "find_tools", r#"{"names":["read"]}"#)])];
    let run = live_child("live-pause", child, &|_, id| assert!(live::pause(id)));
    assert_eq!(run.child, "paused");
    assert_eq!(run.root, TurnOutcome::Done);
}

#[test]
fn a_live_child_session_runs_no_second_turn() {
    let child = vec![
        tool_reply(&[("c0", "find_tools", r#"{"names":["read"]}"#)]),
        text_reply("child done"),
    ];
    let run = live_child("live-twice", child, &|env, id| {
        let mut conv = Conversation::resume(&env.home, &env.cwd, id).unwrap();
        let empty = Arc::new(ScriptedTransport::new(vec![]));
        let agent = Agent::new(
            parts(&empty, key_auth(), &env.cwd),
            Arc::new(Default::default()),
        );
        let mut errors = Vec::new();
        let again = agent.run_turn(&mut conv, "again", &AtomicBool::new(false), &mut |e| {
            if let AgentEvent::Error(text) = e {
                errors.push(text);
            }
        });
        assert_eq!(again, TurnOutcome::Failed);
        assert!(
            errors.iter().any(|e| e.contains("running a turn already")),
            "{errors:?}"
        );
    });
    assert_eq!(run.child, "done", "the parent's run of the child went on");
}
