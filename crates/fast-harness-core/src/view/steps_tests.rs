use super::*;
use crate::events::AgentEvent;
use crate::llm::Timing;
use crate::types::{StopReason, Usage};
use crate::view::Transcript;

fn call(view: &mut Transcript, id: &str, name: &str) {
    view.apply(&AgentEvent::ToolCall {
        id: id.into(),
        name: name.into(),
        arguments: "{}".into(),
    });
}

fn result(view: &mut Transcript, id: &str, is_error: bool) {
    view.apply(&AgentEvent::ToolResult {
        id: id.into(),
        content: "ok".into(),
        is_error,
        duration_ms: 1,
    });
}

fn line(view: &Transcript, words: &[(&str, &'static str)]) -> StepSummary {
    let word = |item: &Item| match item {
        Item::Tool { id, .. } => (words.iter()).find(|(w, _)| w == id).map(|(_, w)| *w),
        _ => None,
    };
    summarize(view.items.iter().map(|i| (i, word(i))))
}

#[test]
fn counts_verbs_in_order_with_plurals() {
    let mut view = Transcript::new();
    view.apply(&AgentEvent::ThinkingDelta("hmm".into()));
    for (id, name) in [("a", "shell"), ("b", "read"), ("c", "shell"), ("d", "grep")] {
        call(&mut view, id, name);
        result(&mut view, id, false);
    }
    for (id, name) in [("e", "subagent"), ("f", "job_output"), ("g", "mcp_call")] {
        call(&mut view, id, name);
        result(&mut view, id, false);
    }
    let s = line(&view, &[]);
    assert_eq!(
        s.line(),
        "Thought, ran commands, read files, searched the code, coordinated subagents, \
         checked jobs, called tools"
    );
    assert_eq!(
        s.counted_line(),
        "Thought, ran 2 commands, read 1 file, searched 1 time, created 1 subagent, \
         checked 1 job, called 1 tool"
    );
    assert!(!s.running && s.failed == 0);
}

#[test]
fn every_verb_has_its_words() {
    let mut view = Transcript::new();
    let names = [
        "write",
        "edit",
        "glob",
        "find_tools",
        "find_tools",
        "web_search",
        "web_fetch",
    ];
    for (i, name) in names.iter().enumerate() {
        call(&mut view, &i.to_string(), name);
        result(&mut view, &i.to_string(), false);
    }
    let s = line(&view, &[]);
    assert_eq!(
        s.text,
        "Edited files, searched the code, loaded tools, searched the web, fetched pages"
    );
    assert_eq!(
        s.counted,
        "Edited 2 files, searched 1 time, loaded 2 tools, searched the web 1 time, fetched 1 page"
    );
}

#[test]
fn errors_and_gate_ends_are_words() {
    let mut view = Transcript::new();
    for id in ["a", "b", "c"] {
        call(&mut view, id, "shell");
    }
    result(&mut view, "a", true);
    result(&mut view, "b", true);
    result(&mut view, "c", false);
    let s = line(&view, &[("b", "declined")]);
    assert_eq!(s.line(), "Ran commands, 1 failed, 1 not run");
    assert_eq!(s.counted_line(), "Ran 3 commands, 1 failed, 1 not run");
}

#[test]
fn a_running_call_names_the_latest_activity() {
    let mut view = Transcript::new();
    call(&mut view, "a", "read");
    result(&mut view, "a", false);
    view.apply(&AgentEvent::ToolCall {
        id: "b".into(),
        name: "shell".into(),
        arguments: r#"{"command":"cargo test"}"#.into(),
    });
    let s = line(&view, &[]);
    assert!(s.running);
    assert!(s
        .line()
        .starts_with("Read files, ran commands, running shell"));
    let waiting = line(&view, &[("b", crate::view::approvals::WAITING)]);
    assert!(waiting
        .line()
        .ends_with("running shell, waiting for your answer"));
}

#[test]
fn a_text_only_step_has_no_line() {
    let mut view = Transcript::new();
    view.apply(&AgentEvent::TextDelta("hi".into()));
    assert_eq!(line(&view, &[]).line(), "");
}

#[test]
fn ranges_end_at_footers_and_user_messages() {
    let mut view = Transcript::new();
    view.push_user("q");
    view.apply(&AgentEvent::ThinkingDelta("t".into()));
    call(&mut view, "a", "read");
    view.footer(1, 1, Usage::default(), Timing::default(), StopReason::End);
    call(&mut view, "b", "read");
    let items = &view.items;
    assert_eq!(step_range(items, 0), 0..1);
    assert_eq!(step_range(items, 1), 1..4);
    assert_eq!(step_range(items, 3), 1..4);
    assert_eq!(step_range(items, 4), 4..5);
    assert_eq!(step_range(items, 5), 4..5);
}
