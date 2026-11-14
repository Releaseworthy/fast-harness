//! The turn navigator (D77) in a real (software rendered) window: hovering
//! a tick shows its card, a click brings the turn's user row to the top,
//! and the column sits in the gutter between the chat column and the bar.

use std::rc::Rc;
use std::time::Duration;

use fast_harness_core::events::AgentEvent;
use fast_harness_core::view::{Approvals, Transcript};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, PhysicalSize, Rgb8Pixel, VecModel};

use crate::mirror::Mirror;
use crate::turn_nav::Nav;
use crate::{MainWindow, MsgRow, TurnsUi};

pub const WIDTH: u32 = 1100;
pub const HEIGHT: u32 = 760;

pub struct TestPlatform(pub Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

/// A session of `turns` turns: a question, a step with thinking, words
/// and two tool calls, then an answer of varied length.
pub fn session(turns: usize) -> Transcript {
    let mut view = Transcript::new();
    for t in 1..=turns {
        view.push_user(format!(
            "Turn {t}: please check the build and explain what changed in the step summaries"
        ));
        view.apply(&AgentEvent::ThinkingDelta(
            "Planning which files to read.".into(),
        ));
        view.apply(&AgentEvent::TextDelta("Looking at the files first.".into()));
        for (id, name, args) in [
            ("a", "shell", r#"{"command":"cargo build"}"#),
            ("b", "read", r#"{"path":"src/main.rs"}"#),
        ] {
            let id = format!("{id}{t}");
            view.apply(&AgentEvent::ToolCall {
                id: id.clone(),
                name: name.into(),
                arguments: args.into(),
            });
            let content = "Finished dev profile in 2.1s".into();
            view.apply(&AgentEvent::ToolResult {
                id,
                content,
                is_error: false,
                duration_ms: 900,
            });
        }
        let answer = format!(
            "Answer {t}: the build passes and the step lines fold every tool call into one line. "
        );
        view.apply(&AgentEvent::TextDelta(answer.repeat(1 + (t * 5) % 7)));
    }
    view
}

/// The window with `view` shown as the app shows it (mirror and navigator).
pub fn show(ui: &MainWindow, view: &Transcript) -> Rc<VecModel<MsgRow>> {
    let model = Rc::new(VecModel::<MsgRow>::default());
    Mirror::new(false).reset_with(&view.items, &Approvals::default(), &*model);
    ui.set_messages(model.clone().into());
    Nav::default().show(ui, &view.items);
    model
}

pub fn window(width: u32, height: u32) -> (Rc<MinimalSoftwareWindow>, MainWindow) {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).expect("platform");
    window.set_size(PhysicalSize::new(width, height));
    let ui = crate::chat_list::new_window().expect("window");
    ui.show().expect("show");
    ui.set_page("chat".into());
    (window, ui)
}

/// Runs the animations, the change handlers and a frame for `ms` milliseconds.
pub fn frames(window: &MinimalSoftwareWindow, ms: u64) {
    let size = window.size();
    let mut buffer = vec![Rgb8Pixel { r: 0, g: 0, b: 0 }; (size.width * size.height) as usize];
    for _ in 0..ms.div_ceil(16) {
        std::thread::sleep(Duration::from_millis(16));
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut buffer, size.width as usize);
        });
    }
}

/// The centre of tick `index` in window coordinates.
pub fn tick(ui: &MainWindow, index: usize) -> LogicalPosition {
    let nav = ui.global::<TurnsUi>();
    let y = nav.get_nav_top() + nav.get_nav_step() * index as f32;
    LogicalPosition::new(nav.get_nav_x(), y)
}

fn click(window: &MinimalSoftwareWindow, position: LogicalPosition) {
    let button = PointerEventButton::Left;
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
}

#[test]
fn hovering_a_tick_shows_its_card() {
    let (window, ui) = window(WIDTH, HEIGHT);
    show(&ui, &session(8));
    frames(&window, 100);
    let nav = ui.global::<TurnsUi>();
    assert!(nav.get_nav_shown(), "eight turns show the navigator");
    assert_eq!(nav.get_hovered(), -1);
    window.dispatch_event(WindowEvent::PointerMoved {
        position: tick(&ui, 2),
    });
    frames(&window, 50);
    assert_eq!(nav.get_hovered(), 2, "the third tick is hovered");
    let away = LogicalPosition::new(200.0, 300.0);
    window.dispatch_event(WindowEvent::PointerMoved { position: away });
    frames(&window, 50);
    assert_eq!(nav.get_hovered(), -1, "the card goes with the pointer");
}

#[test]
fn a_click_brings_the_turn_to_the_top() {
    let (window, ui) = window(WIDTH, HEIGHT);
    let view = session(8);
    show(&ui, &view);
    frames(&window, 100);
    ui.invoke_to_bottom();
    frames(&window, 100);
    let nav = ui.global::<TurnsUi>();
    let rows = crate::turn_nav::turns(&view.items);
    for index in [1, 5, 0, 3] {
        click(&window, tick(&ui, index));
        frames(&window, 300);
        assert_eq!(
            nav.get_top_row(),
            rows[index].row as i32,
            "turn {}",
            index + 1
        );
        assert!(
            !ui.get_transcript_follow(),
            "a jump to a turn stops following"
        );
    }
}

#[test]
fn the_navigator_sits_between_the_column_and_the_bar() {
    let (window, ui) = window(WIDTH, HEIGHT);
    show(&ui, &session(8));
    frames(&window, 100);
    let nav = ui.global::<TurnsUi>();
    let (left, right) = (nav.get_nav_x() - 12.0, nav.get_nav_x() + 8.0);
    let bar = ui.get_transcript_thumb_x() - 7.0;
    assert!(
        left >= nav.get_column_right(),
        "ticks {left} inside the column {}",
        nav.get_column_right()
    );
    assert!(
        right <= bar,
        "ticks end at {right}, the bar starts at {bar}"
    );
    // One turn: no navigator.
    show(&ui, &session(1));
    frames(&window, 50);
    assert!(!nav.get_nav_shown(), "one turn has nothing to navigate");
}

#[test]
fn the_chat_margins_are_symmetric_and_shrink_first() {
    let (window, ui) = window(WIDTH, HEIGHT);
    show(&ui, &session(3));
    frames(&window, 100);
    let nav = ui.global::<TurnsUi>();
    // The sidebar is open: the chat starts right of it.
    let wide = nav.get_column_right();
    window.set_size(PhysicalSize::new(700, HEIGHT));
    frames(&window, 100);
    let narrow = nav.get_column_right();
    assert!(narrow < wide, "the column shrinks with the window");
    assert!(
        !nav.get_nav_shown() || narrow + 4.0 <= nav.get_nav_x() - 12.0,
        "no overlap when narrow"
    );
}

/// Two turns: one with a one-word reply, one with a long final reply.
fn short_and_long() -> Transcript {
    let mut view = Transcript::new();
    view.push_user("Short one");
    view.apply(&AgentEvent::TextDelta("Done.".into()));
    view.push_user("Long one");
    let long = "The build passes and every step folds into one line. ".repeat(20);
    view.apply(&AgentEvent::TextDelta(long));
    view
}

fn card_height(window: &MinimalSoftwareWindow, ui: &MainWindow, index: usize) -> f32 {
    let position = tick(ui, index);
    window.dispatch_event(WindowEvent::PointerMoved { position });
    frames(window, 50);
    assert_eq!(ui.global::<TurnsUi>().get_hovered(), index as i32);
    ui.global::<TurnsUi>().get_card_height()
}

#[test]
fn the_card_is_as_tall_as_its_texts() {
    let (window, ui) = window(WIDTH, HEIGHT);
    show(&ui, &short_and_long());
    frames(&window, 100);
    let short = card_height(&window, &ui, 0);
    let long = card_height(&window, &ui, 1);
    assert!(short > 30.0 && short < 80.0, "a one-line reply: {short}");
    assert!(long >= short + 20.0, "three lines {long} vs one {short}");
    assert!(
        long < short + 60.0,
        "at most three lines: {long} vs {short}"
    );
}
