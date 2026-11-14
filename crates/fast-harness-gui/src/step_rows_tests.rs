use super::*;
use crate::mirror::Mirror;
use fast_harness_core::events::AgentEvent;
use fast_harness_core::view::{Approvals, Transcript};
use slint::{Model, VecModel};

fn turn() -> Transcript {
    let mut view = Transcript::new();
    view.push_user("q");
    view.apply(&AgentEvent::ThinkingDelta("t".into()));
    view.apply(&AgentEvent::TextDelta("Looking.".into()));
    for (id, name) in [("a", "shell"), ("b", "read")] {
        view.apply(&AgentEvent::ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: "{}".into(),
        });
        view.apply(&AgentEvent::ToolResult {
            id: id.into(),
            content: "ok".into(),
            is_error: id == "b",
            duration_ms: 1,
        });
    }
    view
}

#[test]
fn a_run_folds_into_its_first_row_and_opens_on_toggle() {
    let (view, model) = (turn(), VecModel::<MsgRow>::default());
    let mut mirror = Mirror::new(false);
    mirror.reset_with(&view.items, &Approvals::default(), &model);
    // The text that thought first: its own "Thought" line, its text stays.
    let text = model.row_data(1).unwrap_or_default();
    assert_eq!((text.step_summary.as_str(), text.folded), ("Thought", true));
    let head = model.row_data(2).unwrap_or_default();
    assert_eq!(head.step_summary, "Ran commands, read files, 1 failed");
    assert_eq!(head.step_label, "Ran 1 command, read 1 file, 1 failed");
    assert_eq!(head.step_kind, "ran");
    assert!(head.step_failed && head.folded && !head.step_open);
    assert!(model
        .row_data(3)
        .is_some_and(|r| r.folded && r.step_summary.is_empty()));
    mirror.toggle_step(2, &model);
    let rows: Vec<MsgRow> = model.iter().collect();
    assert!(
        rows[2].step_open
            && rows[2..]
                .iter()
                .all(|r| r.rail && !r.folded && r.step_head == 2)
    );
    mirror.sync_with(&view.items, &Approvals::default(), &model, 0);
    assert!(
        model.row_data(3).is_some_and(|r| r.rail),
        "open survives a sync"
    );
    mirror.toggle_step(2, &model);
    assert!(model.row_data(3).is_some_and(|r| r.folded && !r.rail));
}

fn call(view: &mut Transcript, id: &str, name: &str) {
    view.apply(&AgentEvent::ToolCall {
        id: id.into(),
        name: name.into(),
        arguments: "{}".into(),
    });
    view.apply(&AgentEvent::ToolResult {
        id: id.into(),
        content: "ok".into(),
        is_error: false,
        duration_ms: 1,
    });
}

/// The owner's compact turn: text, line, text, line, text. Steps that only
/// called tools (and thought) merge into one line between two texts.
#[test]
fn a_turn_shows_texts_and_one_folded_line_per_run_by_default() {
    let mut view = Transcript::new();
    view.push_user("q");
    call(&mut view, "a", "grep");
    view.apply(&AgentEvent::ThinkingDelta("plan".into()));
    call(&mut view, "b", "read");
    call(&mut view, "c", "shell");
    call(&mut view, "d", "web_fetch");
    view.apply(&AgentEvent::TextDelta("Now edit budget.rs.".into()));
    call(&mut view, "e", "edit");
    view.apply(&AgentEvent::ThinkingDelta("done".into()));
    view.apply(&AgentEvent::TextDelta("All done.".into()));
    let model = VecModel::<MsgRow>::default();
    Mirror::new(false).reset_with(&view.items, &Approvals::default(), &model);
    let shown: Vec<String> = (model.iter().skip(1).filter(crate::rows::shows))
        .map(|r| match r.step_summary.as_str() {
            "" => r.text.to_string(),
            line => format!("[{line}]"),
        })
        .collect();
    assert_eq!(
        shown,
        [
            "[Searched the code, thought, read files, etc.]",
            "Now edit budget.rs.",
            "[Edited files, thought]",
            "All done.",
        ]
    );
    let rows: Vec<MsgRow> = model.iter().collect();
    assert!(rows.iter().all(|r| !r.step_open && !r.rail));
    assert!(rows.iter().filter(|r| r.kind == "tool").all(|r| r.folded));
}
#[test]
fn full_detail_has_no_lines_and_switches_in_place() {
    let (view, model) = (turn(), VecModel::<MsgRow>::default());
    let mut mirror = Mirror::new(true);
    mirror.reset_with(&view.items, &Approvals::default(), &model);
    assert!(model.iter().all(|r| r.step_summary.is_empty() && !r.folded));
    mirror.set_full(false, &model);
    assert!(model.row_data(2).is_some_and(|r| r.folded));
}

/// User, text, a finished grep, then `b` without its result.
fn unfinished() -> Transcript {
    let mut view = Transcript::new();
    view.push_user("q");
    view.apply(&AgentEvent::TextDelta("Looking.".into()));
    for id in ["a", "b"] {
        view.apply(&AgentEvent::ToolCall {
            id: id.into(),
            name: "shell".into(),
            arguments: "{}".into(),
        });
    }
    view.apply(&AgentEvent::ToolResult {
        id: "a".into(),
        content: "ok".into(),
        is_error: false,
        duration_ms: 1,
    });
    view
}

#[test]
fn a_call_waiting_for_an_answer_opens_its_run_and_keeps_its_card() {
    let view = unfinished();
    let mut words = vec![None; view.items.len()];
    words[3] = Some(WAITING);
    let none = HashSet::new();
    let steps = Steps { items: &view.items, words: &words, full: false, open: &none, closed: &none };
    let decos = steps.from(0);
    assert!(decos[2].open && decos[2].rail, "the run opens: {:?}", decos[2]);
    assert!(decos[2].lined, "the finished call is a line");
    assert!(!decos[3].lined && !decos[3].folded, "the waiting call keeps its card");
}

#[test]
fn a_running_call_in_an_open_run_keeps_its_card() {
    let view = unfinished();
    let words = vec![None; view.items.len()];
    let (open, none) = (HashSet::from([2]), HashSet::new());
    let steps = Steps { items: &view.items, words: &words, full: false, open: &open, closed: &none };
    let decos = steps.from(0);
    assert!(decos[2].lined && !decos[3].lined, "{:?}", &decos[2..]);
}