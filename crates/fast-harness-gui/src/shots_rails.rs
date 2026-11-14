//! D80 in the whole window, for a visual check: `FAST_HARNESS_SHOTS_DIR=<dir>
//! cargo test -p fast-harness-gui rails_shots -- --ignored` writes, per theme
//! (dark, light), `rails-open` (turn rail, an open step's rail, the
//! indents), `rails-folded` (turn 2 folded to one line), `rails-card` (the
//! navigator card of a long reply), `rails-150` (the window at 150 %) and
//! `settings-limits` (Settings with the UI scale and the subagent limits).

use std::path::PathBuf;
use std::rc::Rc;

use fast_harness_core::theme::builtins;
use fast_harness_core::view::Approvals;
use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, Model, Rgb8Pixel, VecModel};

use crate::mirror::Mirror;
use crate::turn_nav::{turns, Nav};
use crate::turn_nav_view_tests::{frames, session, tick, window, HEIGHT, WIDTH};
use crate::{limits, shots_png, theme, LimitsUi, MainWindow, MsgRow};

fn render(window: &MinimalSoftwareWindow) -> Vec<u8> {
    let mut buffer = vec![Rgb8Pixel { r: 0, g: 0, b: 0 }; (WIDTH * HEIGHT) as usize];
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut buffer, WIDTH as usize);
        });
    }
    buffer.iter().flat_map(|p| [p.r, p.g, p.b]).collect()
}

fn out_dir() -> PathBuf {
    std::env::var_os("FAST_HARNESS_SHOTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("fast-harness-gui-shots"))
}

fn go(window: &MinimalSoftwareWindow, ui: &MainWindow, index: usize) {
    let (position, button) = (tick(ui, index), PointerEventButton::Left);
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
    window.dispatch_event(WindowEvent::PointerExited);
    frames(window, 300);
}

/// The row of the second step line (turn 2's tool step).
fn second_step(model: &VecModel<MsgRow>) -> usize {
    let lines = (0..model.row_count())
        .filter(|&i| (model.row_data(i)).is_some_and(|r| !r.step_summary.is_empty()));
    lines.take(2).last().expect("a step line")
}

fn settings(window: &MinimalSoftwareWindow, ui: &MainWindow, save: &dyn Fn(&str)) {
    crate::dialogs::open(ui, "settings");
    frames(window, 100);
    // Scrolled to the Subagents limits and the UI scale.
    let position = slint::LogicalPosition::new(WIDTH as f32 / 2.0, HEIGHT as f32 / 2.0);
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerScrolled {
        position,
        delta_x: 0.0,
        delta_y: -360.0,
    });
    frames(window, 300);
    let g = ui.global::<LimitsUi>();
    g.set_scale(150);
    g.set_max_parallel(4);
    g.set_max_depth(1);
    save("settings-limits");
    subagent_models(window, ui, position);
    save("settings-subagent-models");
    crate::dialogs::close(ui);
}

/// Settings > Subagent models (D82): three models, Opus off, the default
/// guide with an unsaved edit, scrolled into view.
fn subagent_models(
    window: &MinimalSoftwareWindow,
    ui: &MainWindow,
    position: slint::LogicalPosition,
) {
    let row = |id: &str, name: &str, on: bool| crate::SpawnModel {
        id: id.into(),
        label: format!("{name} ({id})").into(),
        on,
    };
    let g = ui.global::<crate::SpawnUi>();
    let rows = vec![
        row("claude-opus-5-5", "Claude Opus 5.5", false),
        row("claude-sonnet-5-5", "Claude Sonnet 5.5", true),
        row("claude-haiku-5-5", "Claude Haiku 5.5", true),
    ];
    g.set_models(slint::ModelRc::new(VecModel::from(rows)));
    let guide = fast_harness_core::config::DEFAULT_SUBAGENT_GUIDE;
    g.set_saved(guide.into());
    g.set_guide(format!("{guide}\n- Fable: long creative writing.").into());
    window.dispatch_event(WindowEvent::PointerScrolled {
        position,
        delta_x: 0.0,
        delta_y: -420.0,
    });
    frames(window, 300);
}

#[test]
#[ignore = "writes PNG screenshots for a visual check"]
fn rails_shots() {
    let (window, ui) = window(WIDTH, HEIGHT);
    let dir = out_dir();
    std::fs::create_dir_all(&dir).expect("shots directory");
    let view = session(8);
    let heads: Vec<usize> = turns(&view.items).iter().map(|t| t.row).collect();
    for name in ["Dark", "Light"] {
        let core = builtins().iter().find(|t| t.name == name).expect("theme");
        theme::apply(&ui, core, 0);
        let save = |shot: &str| {
            let png = shots_png::encode(WIDTH, HEIGHT, &render(&window));
            let file = format!("{}-{shot}.png", name.to_lowercase());
            std::fs::write(dir.join(file), png).expect("write png");
        };
        let model = Rc::new(VecModel::<MsgRow>::default());
        let mut mirror = Mirror::new(false);
        mirror.reset_with(&view.items, &Approvals::default(), &*model);
        ui.set_messages(model.clone().into());
        Nav::default().show(&ui, &view.items);
        frames(&window, 100);
        mirror.toggle_step(second_step(&model), &*model);
        frames(&window, 100);
        go(&window, &ui, 1);
        save("rails-open");
        limits::apply_scale(ui.window(), 1.5);
        frames(&window, 100);
        go(&window, &ui, 1);
        save("rails-150");
        limits::apply_scale(ui.window(), 1.0);
        frames(&window, 100);
        mirror.toggle_turn(heads[1], &*model);
        frames(&window, 100);
        go(&window, &ui, 0);
        save("rails-folded");
        let position = tick(&ui, 5);
        window.dispatch_event(WindowEvent::PointerMoved { position });
        frames(&window, 200);
        save("rails-card");
        window.dispatch_event(WindowEvent::PointerExited);
        settings(&window, &ui, &save);
    }
}
