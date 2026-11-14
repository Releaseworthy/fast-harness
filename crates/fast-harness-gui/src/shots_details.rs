//! Markdown replies and step details (the owner, 2026-11-15) in the whole
//! window: `FAST_HARNESS_SHOTS_DIR=<dir> cargo test -p fast-harness-gui
//! detail_shots -- --ignored` writes, per theme (dark, light),
//! `md-reply` (heading, list, code, table, quote), `run-lines` (an open
//! run, its calls folded to lines), `run-card` (one call opened) and
//! `run-thinking` (the thinking opened).

use fast_harness_core::events::AgentEvent;
use fast_harness_core::theme::builtins;
use fast_harness_core::view::{Approvals, Transcript};
use slint::VecModel;

use crate::detail_view_tests::run_turn;
use crate::mirror::{toggle, toggle_card, Mirror};
use crate::shots_turns::{out_dir, render};
use crate::turn_nav_view_tests::{frames, window, HEIGHT, WIDTH};
use crate::{shots_png, theme, MsgRow};

const REPLY: &str = "## Token budget\n\nThe step output now stops at **8 KB** and the \
rest goes to a `file`. See [D84](https://example.com).\n\n### Changes\n\n- `budget.rs`: \
the new limit\n- `formats.md`: the trace note\n  1. version bump\n  2. index line\n\n\
```rust\nconst STEP_OUTPUT_BYTES: usize = 8 * 1024;\n```\n\n| File | Lines | Note |\n\
|------|------:|------|\n| budget.rs | 42 | limit and test |\n| formats.md | 7 | **trace v4** |\n\
| PLAN.md | 1 | index |\n\n> The cache stays warm: nothing sent before changes.\n\n---\n\nDone.";

fn md_turn() -> Transcript {
    let mut view = Transcript::new();
    view.push_user("Summarise the token budget change.");
    view.apply(&AgentEvent::TextDelta(REPLY.into()));
    view
}

#[test]
#[ignore = "writes PNG screenshots for a visual check"]
fn detail_shots() {
    let (window, ui) = window(WIDTH, HEIGHT);
    let dir = out_dir();
    std::fs::create_dir_all(&dir).expect("shots directory");
    for name in ["Dark", "Light"] {
        let core = builtins().iter().find(|t| t.name == name).expect("theme");
        theme::apply(&ui, core, 0);
        let save = |shot: &str| {
            frames(&window, 200);
            let png = shots_png::encode(WIDTH, HEIGHT, &render(&window));
            let file = format!("{}-{shot}.png", name.to_lowercase());
            std::fs::write(dir.join(file), png).expect("write png");
        };
        let show = |view: &Transcript| {
            let model = std::rc::Rc::new(VecModel::<MsgRow>::default());
            let mut mirror = Mirror::new(false);
            mirror.reset_with(&view.items, &Approvals::default(), &*model);
            ui.set_messages(model.clone().into());
            (model, mirror)
        };
        show(&md_turn());
        save("md-reply");
        let (model, mut mirror) = show(&run_turn());
        mirror.toggle_step(2, &*model);
        save("run-lines");
        toggle_card(&model, 3);
        save("run-card");
        toggle_card(&model, 3);
        toggle(&model, 4);
        save("run-thinking");
    }
}
