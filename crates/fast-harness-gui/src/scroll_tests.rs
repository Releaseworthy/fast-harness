//! The transcript's follow mode driven through a real (software rendered)
//! window: a small wheel scroll up stops following at once, new output does not
//! pull the view back down, and Jump to latest follows again.

use std::rc::Rc;
use std::time::Duration;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model, PhysicalSize, Rgb8Pixel, VecModel};

use crate::rows::markdown;
use crate::{MainWindow, MsgRow};

const WIDTH: u32 = 1100;
const HEIGHT: u32 = 760;

struct TestPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn row(kind: &str, text: &str) -> MsgRow {
    MsgRow {
        kind: kind.into(),
        text: text.into(),
        shown: text.into(),
        body: markdown(text),
        ..MsgRow::default()
    }
}

/// Short questions and answers of very different heights, so the ListView's
/// height estimate changes as rows come into view.
fn chat() -> Vec<MsgRow> {
    (0..40)
        .map(|i| match i % 2 {
            0 => row("user", &format!("Question {i}")),
            _ => row(
                "assistant",
                &format!("Answer {i}.\n\n").repeat(1 + (i * 7) % 23),
            ),
        })
        .collect()
}

/// Runs the animations, the change handlers and a frame for `ms` milliseconds.
fn frames(window: &MinimalSoftwareWindow, ms: u64) {
    let mut buffer = vec![Rgb8Pixel { r: 0, g: 0, b: 0 }; (WIDTH * HEIGHT) as usize];
    for _ in 0..ms.div_ceil(16) {
        std::thread::sleep(Duration::from_millis(16));
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut buffer, WIDTH as usize);
        });
    }
}

#[test]
fn wheel_up_stops_following_and_output_does_not_pull_down() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).expect("platform");
    window.set_size(PhysicalSize::new(WIDTH, HEIGHT));
    let ui = MainWindow::new().expect("window");
    ui.show().expect("show");
    let rows = Rc::new(VecModel::from(chat()));
    ui.set_messages(rows.clone().into());
    ui.set_page("chat".into());
    frames(&window, 100);
    ui.invoke_to_bottom();
    frames(&window, 100);
    assert!(ui.get_transcript_follow(), "follows at the bottom");
    // A small step (a touchpad's pixel delta): the old rule kept following
    // within 32 px of the bottom, so the next output pulled the view down.
    let position = LogicalPosition::new(700.0, 300.0);
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerScrolled {
        position,
        delta_x: 0.0,
        delta_y: 20.0,
    });
    frames(&window, 300);
    assert!(
        !ui.get_transcript_follow(),
        "a wheel scroll up stops following"
    );
    // Streaming output grows the last row and adds one.
    let last = rows.row_count() - 1;
    rows.set_row_data(last, row("assistant", &"More output.\n\n".repeat(30)));
    rows.push(row("note", "Session saved."));
    frames(&window, 300);
    assert!(
        !ui.get_transcript_follow(),
        "new output does not pull the view down"
    );
    ui.invoke_to_bottom();
    frames(&window, 100);
    assert!(ui.get_transcript_follow(), "Jump to latest follows again");
}
