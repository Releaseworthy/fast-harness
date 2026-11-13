use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::*;
use crate::agent::Inbox;
use crate::events::AgentEvent;

#[test]
fn controls_reach_only_the_running_child() {
    let (tx, mut inbox) = Inbox::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    assert!(!running("live-a"));
    let guard = run("live-a", cancel.clone(), tx);
    assert!(running("live-a"));
    assert!(!running("live-b"), "another id is not running");
    assert!(steer("live-a", "look at src"));
    assert!(pause("live-a"));
    assert_eq!(inbox.take_steers().as_deref(), Some("look at src"));
    assert!(inbox.take_pause());
    assert!(!stop("live-b"));
    assert!(stop("live-a"));
    assert!(cancel.load(Ordering::SeqCst));
    drop(guard);
    assert!(!running("live-a"));
    assert!(!steer("live-a", "late"));
}

#[test]
fn a_watch_sees_events_and_ends_with_the_child() {
    let (tx, _inbox) = Inbox::channel();
    let guard = run("live-w", Arc::default(), tx);
    let rx = watch("live-w").expect("running");
    publish("live-w", &AgentEvent::TextDelta("hi".into()));
    assert_eq!(rx.try_recv().ok(), Some(AgentEvent::TextDelta("hi".into())));
    drop(guard);
    assert!(rx.recv().is_err(), "the channel closes at the end");
    assert!(watch("live-w").is_none());
}

#[test]
fn a_second_turn_of_one_session_is_refused() {
    let first = enter("live-t").expect("free");
    let note = enter("live-t").err().expect("busy");
    assert!(note.contains("live-t"), "{note}");
    drop(first);
    assert!(enter("live-t").is_ok(), "free again");
}
