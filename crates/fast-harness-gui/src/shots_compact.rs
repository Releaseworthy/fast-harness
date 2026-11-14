//! The compact turn (the owner's DSH screenshot, 2026-11-14) in the whole
//! window: `FAST_HARNESS_SHOTS_DIR=<dir> cargo test -p fast-harness-gui
//! compact_shots -- --ignored` writes, per theme (dark, light),
//! `compact-turn` (text, folded run line, text, line, text, line, text)
//! and `compact-open` (the middle run opened, its rail in the gutter).

use fast_harness_core::events::AgentEvent;
use fast_harness_core::theme::builtins;
use fast_harness_core::view::{Approvals, Transcript};
use slint::{Model, VecModel};

use crate::mirror::Mirror;
use crate::shots_turns::{out_dir, render};
use crate::turn_nav_view_tests::{frames, window, HEIGHT, WIDTH};
use crate::{shots_png, theme, MsgRow};

fn call(view: &mut Transcript, id: &str, name: &str, arguments: &str) {
    view.apply(&AgentEvent::ToolCall {
        id: id.into(),
        name: name.into(),
        arguments: arguments.into(),
    });
    view.apply(&AgentEvent::ToolResult {
        id: id.into(),
        content: "Finished dev profile in 2.1s".into(),
        is_error: false,
        duration_ms: 900,
    });
}

/// One turn as the owner's screenshot shows it.
fn compact_turn() -> Transcript {
    let mut view = Transcript::new();
    view.push_user("Trim the token budget and write the decision.");
    view.apply(&AgentEvent::TextDelta(
        "Let me look at the budget code.".into(),
    ));
    call(&mut view, "a", "grep", r#"{"pattern":"budget"}"#);
    call(&mut view, "b", "read", r#"{"path":"src/budget.rs"}"#);
    call(&mut view, "c", "shell", r#"{"command":"cargo test"}"#);
    call(&mut view, "d", "find_tools", r#"{"query":"web"}"#);
    view.apply(&AgentEvent::TextDelta("Now edit budget.rs.".into()));
    call(&mut view, "e", "edit", r#"{"path":"src/budget.rs"}"#);
    call(&mut view, "f", "grep", r#"{"pattern":"stepOutputBytes"}"#);
    call(&mut view, "g", "read", r#"{"path":"docs/formats.md"}"#);
    view.apply(&AgentEvent::TextDelta(
        "Now write D84 and the PLAN index line.".into(),
    ));
    call(
        &mut view,
        "h",
        "shell",
        r#"{"command":"cargo test --workspace"}"#,
    );
    view.apply(&AgentEvent::ThinkingDelta("All green.".into()));
    view.apply(&AgentEvent::TextDelta(
        "The core token changes are done, tested and committed on branch \
         `token-diet-turn` (90cef47). The scrollbar work is still running in \
         a subagent in the `repos/fast-harness-gui-wt` worktree."
            .into(),
    ));
    view
}

#[test]
#[ignore = "writes PNG screenshots for a visual check"]
fn compact_shots() {
    let (window, ui) = window(WIDTH, HEIGHT);
    let dir = out_dir();
    std::fs::create_dir_all(&dir).expect("shots directory");
    let view = compact_turn();
    for name in ["Dark", "Light"] {
        let core = builtins().iter().find(|t| t.name == name).expect("theme");
        theme::apply(&ui, core, 0);
        let save = |shot: &str| {
            let png = shots_png::encode(WIDTH, HEIGHT, &render(&window));
            let file = format!("{}-{shot}.png", name.to_lowercase());
            std::fs::write(dir.join(file), png).expect("write png");
        };
        let model = std::rc::Rc::new(VecModel::<MsgRow>::default());
        let mut mirror = Mirror::new(false);
        mirror.reset_with(&view.items, &Approvals::default(), &*model);
        ui.set_messages(model.clone().into());
        frames(&window, 200);
        save("compact-turn");
        let lines: Vec<usize> = (0..model.row_count())
            .filter(|&i| (model.row_data(i)).is_some_and(|r| !r.step_summary.is_empty()))
            .collect();
        assert_eq!(lines.len(), 3, "one line per run");
        mirror.toggle_step(lines[1], &*model);
        frames(&window, 200);
        save("compact-open");
    }
}
