//! The subagents pill (PLAN.md D74): subagent events stay out of the
//! transcript, and the rows merge the logs with the live events.

use std::collections::HashMap;

use fast_harness_core::events::AgentEvent;
use fast_harness_core::session::Child;

use super::{rows, take, Live};

fn child(id: &str, outcome: Option<&str>) -> Child {
    Child {
        id: id.into(),
        label: format!("label of {id}"),
        depth: 1,
        created_at: 1_000,
        last_time: 126_000,
        step: 7,
        tool: Some("grep".into()),
        outcome: outcome.map(str::to_string),
        unfinished: None,
    }
}

#[test]
fn subagent_events_are_taken_and_others_pass() {
    let mut live = HashMap::new();
    let start = AgentEvent::Subagent { session: "a".into(), running: true };
    assert_eq!(take(&mut live, &start), Some("a"));
    let line = "Review GUI · step 40 · grep".to_string();
    let progress = AgentEvent::SubagentProgress { session: "a".into(), line: line.clone() };
    assert_eq!(take(&mut live, &progress), Some("a"));
    assert_eq!(live["a"], Live { running: true, line });
    assert_eq!(take(&mut live, &AgentEvent::Error("x".into())), None);
}

#[test]
fn rows_take_the_state_from_the_live_events_and_the_rest_from_the_logs() {
    let mut live = HashMap::new();
    let line = "Review GUI · step 40 · grep".to_string();
    live.insert("a".to_string(), Live { running: true, line });
    let kids = [child("a", None), child("b", Some("done")), child("c", Some("failed")), child("d", None)];
    let rows = rows(&kids, &live);
    let marks: Vec<(&str, &str)> = rows.iter().map(|r| (r.mark.as_str(), r.word.as_str())).collect();
    assert_eq!(marks, [("running", "running"), ("finished", "finished"), ("failed", "failed"), ("stopped", "interrupted")]);
    assert_eq!((rows[0].label.as_str(), rows[0].progress.as_str(), rows[0].when.as_str()), ("Review GUI", "step 40 · grep", ""));
    assert_eq!((rows[1].label.as_str(), rows[1].progress.as_str(), rows[1].when.as_str()), ("label of b", "step 7 · grep", "2m 05s"));
}
