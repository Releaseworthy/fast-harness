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
fn a_step_folds_into_its_first_row_and_opens_on_toggle() {
    let (view, model) = (turn(), VecModel::<MsgRow>::default());
    let mut mirror = Mirror::new(false);
    mirror.reset_with(&view.items, &Approvals::default(), &model);
    let head = model.row_data(1).unwrap_or_default();
    assert_eq!(
        head.step_summary,
        "Thought, ran commands, read files, 1 failed"
    );
    assert_eq!(
        head.step_label,
        "Thought, ran 1 command, read 1 file, 1 failed"
    );
    assert!(head.step_failed && head.folded && !head.step_open);
    assert!(model
        .row_data(2)
        .is_some_and(|r| r.folded && r.step_summary.is_empty()));
    mirror.toggle_step(1, &model);
    let rows: Vec<MsgRow> = model.iter().collect();
    assert!(
        rows[1].step_open
            && rows[1..]
                .iter()
                .all(|r| r.rail && !r.folded && r.step_head == 1)
    );
    mirror.sync_with(&view.items, &Approvals::default(), &model, 0);
    assert!(
        model.row_data(2).is_some_and(|r| r.rail),
        "open survives a sync"
    );
    mirror.toggle_step(1, &model);
    assert!(model.row_data(3).is_some_and(|r| r.folded && !r.rail));
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
