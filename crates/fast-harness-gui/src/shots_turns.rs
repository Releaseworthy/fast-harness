//! The turn navigator (D77) and the step lines (D73) in the whole window,
//! for a visual check: `FAST_HARNESS_SHOTS_DIR=<dir> cargo test -p
//! fast-harness-gui turns_shots -- --ignored` writes, per theme (dark,
//! light), `turns-ticks` (eight turns, a middle turn at the top),
//! `turns-hover` (a tick's card), `steps-folded` (the default) and
//! `steps-open` (one step opened, its rail).

use std::path::PathBuf;

use fast_harness_core::theme::builtins;
use fast_harness_core::view::Approvals;
use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{Model, Rgb8Pixel, VecModel};

use crate::mirror::Mirror;
use crate::turn_nav::Nav;
use crate::turn_nav_view_tests::{frames, session, tick, window, HEIGHT, WIDTH};
use crate::{shots_png, theme, MainWindow, MsgRow};

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

fn click(window: &MinimalSoftwareWindow, ui: &MainWindow, index: usize) {
    let (position, button) = (tick(ui, index), PointerEventButton::Left);
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
    frames(window, 300);
}

/// The row of the second step line (turn 2's tool step).
fn second_step(model: &VecModel<MsgRow>) -> usize {
    let lines = (0..model.row_count())
        .filter(|&i| (model.row_data(i)).is_some_and(|r| !r.step_summary.is_empty()));
    lines.take(2).last().expect("a step line")
}

#[test]
#[ignore = "writes PNG screenshots for a visual check"]
fn turns_shots() {
    let (window, ui) = window(WIDTH, HEIGHT);
    let dir = out_dir();
    std::fs::create_dir_all(&dir).expect("shots directory");
    let view = session(8);
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
        Nav::default().show(&ui, &view.items);
        frames(&window, 100);
        click(&window, &ui, 3);
        save("turns-ticks");
        let position = tick(&ui, 5);
        window.dispatch_event(WindowEvent::PointerMoved { position });
        frames(&window, 200);
        save("turns-hover");
        window.dispatch_event(WindowEvent::PointerExited);
        frames(&window, 50);
        click(&window, &ui, 1);
        save("steps-folded");
        let head = second_step(&model);
        mirror.toggle_step(head, &*model);
        frames(&window, 100);
        click(&window, &ui, 1);
        save("steps-open");
    }
}
