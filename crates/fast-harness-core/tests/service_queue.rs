//! What the service keeps around a turn for a front end (PLAN.md D39,
//! D59): steers, follow-ups, cancel with a queue, pause and resume, drop.

mod agent_support;
mod service_support;

use std::sync::Arc;

use agent_support::{env, text_reply, tool_reply, Env};
use fast_harness_core::events::{AgentEvent, TurnOutcome};
use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use fast_harness_core::service::{Service, Session, Start};
use fast_harness_core::session::Unfinished;
use service_support::{watch, Gate, TestHost, Watch};

/// An echo call that sleeps 300 ms: the window to steer, queue or pause.
fn slow_tool() -> ScriptedResponse {
    tool_reply(&[("c1", "echo", r#"{"text":"one","sleepMs":300}"#)])
}

fn open(name: &str, script: Vec<ScriptedResponse>) -> (Env, Service, Arc<Session>) {
    let dir = env(name);
    let service = Service::new(Arc::new(TestHost {
        home: dir.home.clone(),
        transport: Arc::new(ScriptedTransport::new(script)),
        gate: Arc::new(Gate::default()),
    }));
    let session = service.create(&dir.cwd).unwrap();
    (dir, service, session)
}

/// Starts "hi" and waits until the slow tool runs.
fn in_tool_step(service: &Service, session: &Arc<Session>) -> Watch {
    let turn = watch(service, session, Start::Text("hi".into()));
    turn.until(|e| matches!(e, AgentEvent::ToolCall { .. }));
    turn
}

#[test]
fn a_steer_during_a_tool_step_is_read_by_the_turn() {
    let (_dir, service, session) = open("queue-steer", vec![slow_tool(), text_reply("ok")]);
    let turn = in_tool_step(&service, &session);
    assert!(session.steer("use b.rs".into()));
    assert_eq!(session.status().steering, ["use b.rs"]);
    turn.until(|e| matches!(e, AgentEvent::UserInput { steer: true, .. }));
    let end = turn.end();
    assert_eq!(end.outcome, TurnOutcome::Done);
    let status = session.status();
    assert!(status.steering.is_empty() && status.queued.is_empty());
    assert!(!status.running);
}

#[test]
fn a_follow_up_waits_for_the_end_and_a_steer_after_it_queues() {
    let (_dir, service, session) = open("queue-follow", vec![slow_tool(), text_reply("ok")]);
    let turn = in_tool_step(&service, &session);
    session.queue("then docs".into());
    assert!(
        session.take_next().is_none(),
        "nothing starts while running"
    );
    assert_eq!(turn.end().outcome, TurnOutcome::Done);
    assert!(!session.steer("late".into()), "no turn runs: it queues");
    assert_eq!(session.status().queued, ["then docs", "late"]);
    assert_eq!(session.take_next().as_deref(), Some("then docs"));
    assert_eq!(session.status().queued, ["late"]);
}

#[test]
fn cancel_with_a_queue_keeps_the_unused_texts_first() {
    let (_dir, service, session) = open("queue-cancel", vec![slow_tool()]);
    let turn = in_tool_step(&service, &session);
    session.queue("later".into());
    assert!(session.cancel());
    assert!(session.status().stopping);
    let end = turn.end();
    assert_eq!(end.outcome, TurnOutcome::Cancelled);
    let status = session.status();
    assert!(!status.stopping && !status.running);
    assert_eq!(status.unfinished, Some(Unfinished::Cancelled));
    let mut expected = end.leftover.clone();
    expected.push("later".into());
    assert_eq!(status.queued, expected);
    assert_eq!(session.drop_queue(), expected);
    assert!(session.status().queued.is_empty());
}

#[test]
fn pause_parks_the_turn_and_resume_ends_it() {
    let script = vec![slow_tool(), text_reply("resumed")];
    let (_dir, service, session) = open("queue-pause", script);
    let turn = in_tool_step(&service, &session);
    assert!(session.pause());
    assert!(session.status().pausing);
    assert_eq!(turn.end().outcome, TurnOutcome::Paused);
    let status = session.status();
    assert_eq!(status.unfinished, Some(Unfinished::Paused));
    assert!(!status.pausing && session.can_resume());
    let resumed = watch(&service, &session, Start::Resume);
    assert!(session.status().unfinished.is_none());
    assert_eq!(resumed.end().outcome, TurnOutcome::Done);
    assert!(session.status().unfinished.is_none() && !session.can_resume());
}

#[test]
fn queued_texts_are_removed_by_index_and_dropped() {
    let (_dir, _service, session) = open("queue-drop", Vec::new());
    for text in ["a", "b", "c"] {
        session.queue(text.into());
    }
    assert_eq!(session.remove_queued(1).as_deref(), Some("b"));
    assert!(session.remove_queued(5).is_none());
    assert!(!session.pause(), "no turn runs");
    assert_eq!(session.drop_queue(), ["a", "c"]);
}
