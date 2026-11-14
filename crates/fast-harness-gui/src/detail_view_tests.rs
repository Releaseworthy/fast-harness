//! Step details inside an open run (the owner, 2026-11-15): each tool call
//! and each thinking is one line, folded by default; a click on the line
//! opens the detail one level in with its own rail in the gutter.

use std::rc::Rc;

use fast_harness_core::events::AgentEvent;
use fast_harness_core::view::{Approvals, Transcript};
use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model, Rgb8Pixel, VecModel};

use crate::mirror::{toggle, toggle_card, Mirror};
use crate::turn_nav_view_tests::{frames, window, WIDTH};
use crate::{MainWindow, MsgRow, TurnsUi};

fn call(view: &mut Transcript, id: &str, name: &str, arguments: &str, failed: bool) {
    view.apply(&AgentEvent::ToolCall {
        id: id.into(),
        name: name.into(),
        arguments: arguments.into(),
    });
    view.apply(&AgentEvent::ToolResult {
        id: id.into(),
        content: "fatal: not a git repository".into(),
        is_error: failed,
        duration_ms: 620,
    });
}

/// Rows: 0 user, 1 text, 2 grep (the run's line), 3 failed shell,
/// 4 thinking and the answer.
pub fn run_turn() -> Transcript {
    let mut view = Transcript::new();
    view.push_user("Check the repo.");
    view.apply(&AgentEvent::TextDelta("Let me look.".into()));
    call(&mut view, "a", "grep", r#"{"pattern":"budget"}"#, false);
    call(&mut view, "b", "shell", r#"{"command":"git log -5"}"#, true);
    view.apply(&AgentEvent::ThinkingDelta(
        "The folder is not a repo, so say so.".into(),
    ));
    view.apply(&AgentEvent::TextDelta("It is not a git repository.".into()));
    view
}

struct Chat {
    window: Rc<MinimalSoftwareWindow>,
    ui: MainWindow,
    model: Rc<VecModel<MsgRow>>,
}

fn open_run() -> Chat {
    let (window, ui) = window(WIDTH, 2000);
    let model = Rc::new(VecModel::<MsgRow>::default());
    let mut mirror = Mirror::new(false);
    mirror.reset_with(&run_turn().items, &Approvals::default(), &*model);
    ui.set_messages(model.clone().into());
    let rows = model.clone();
    ui.on_card_toggle(move |i| toggle_card(&rows, i as usize));
    let rows = model.clone();
    ui.on_toggle(move |i| toggle(&rows, i as usize));
    mirror.toggle_step(2, &*model);
    frames(&window, 200);
    Chat { window, ui, model }
}

impl Chat {
    fn row(&self, i: usize) -> MsgRow {
        self.model.row_data(i).expect("row")
    }
    /// Row `i`'s top and height on screen.
    fn top(&self, i: usize) -> (f32, f32) {
        let tops = self.ui.get_chat_tops();
        let at = |k: usize| tops.row_data(k).unwrap_or(0.0);
        let list = self.ui.global::<TurnsUi>().get_list_top();
        (list + at(i) - self.ui.get_chat_offset(), at(i + 1) - at(i))
    }
    fn left(&self) -> usize {
        self.ui.global::<TurnsUi>().get_column_left() as usize
    }
    fn click(&self, x: f32, y: f32) {
        let position = LogicalPosition::new(x, y);
        let button = PointerEventButton::Left;
        self.window
            .dispatch_event(WindowEvent::PointerMoved { position });
        self.window
            .dispatch_event(WindowEvent::PointerPressed { position, button });
        self.window
            .dispatch_event(WindowEvent::PointerReleased { position, button });
        self.window.dispatch_event(WindowEvent::PointerExited);
        frames(&self.window, 200);
    }
    fn pixels(&self) -> (Vec<Rgb8Pixel>, usize) {
        let size = self.window.size();
        let mut pixels = vec![Rgb8Pixel { r: 0, g: 0, b: 0 }; (size.width * size.height) as usize];
        self.window.request_redraw();
        self.window.draw_if_needed(|r| {
            r.render(&mut pixels, size.width as usize);
        });
        (pixels, size.width as usize)
    }
    /// At height `y`: the rail column (left + 7) and the gap right of it
    /// (left + 14) differ from the margin, the detail starts at left + 22.
    fn rail_and_indent(&self, y: f32) -> (bool, bool, bool) {
        let (pixels, width) = self.pixels();
        let (y, left) = (y as usize, self.left());
        let at = |x: usize| pixels[y * width + x];
        let margin = at(left - 4);
        let detail = (left + 22..left + 40).any(|x| at(x) != margin);
        (at(left + 7) != margin, at(left + 14) != margin, detail)
    }
}

#[test]
fn a_tool_call_in_an_open_run_is_one_line_and_opens_one_level_in_with_a_rail() {
    let chat = open_run();
    let shell = chat.row(3);
    assert!(
        shell.rail && !shell.folded && !shell.card_open,
        "a line by default"
    );
    let (y, height) = chat.top(3);
    assert!(height < 45.0, "the folded call is one line: {height}");
    chat.click(chat.left() as f32 + 120.0, y + 8.0 + 13.0);
    assert!(chat.row(3).card_open, "the click opens the card");
    let (y, height) = chat.top(3);
    assert!(height > 90.0, "the card shows: {height}");
    let (rail, gap, card) = chat.rail_and_indent(y + 8.0 + 26.0 + 4.0 + 12.0);
    assert!(rail && !gap && card, "rail {rail}, gap {gap}, card {card}");
    chat.click(chat.left() as f32 + 6.0, y + 60.0);
    assert!(!chat.row(3).card_open, "the rail folds it");
}

#[test]
fn thinking_in_an_open_run_is_one_line_and_opens_one_level_in_with_a_rail() {
    let chat = open_run();
    let answer = chat.row(4);
    assert!(!answer.folded && !answer.expanded && answer.thought.contains("not a repo"));
    let (y, folded) = chat.top(4);
    chat.click(chat.left() as f32 + 120.0, y + 8.0 + 13.0);
    assert!(chat.row(4).expanded, "the click opens the thinking");
    let (y, height) = chat.top(4);
    assert!(
        height > folded + 10.0,
        "the thinking shows: {folded} -> {height}"
    );
    let (rail, gap, text) = chat.rail_and_indent(y + 8.0 + 26.0 + 4.0 + 6.0);
    assert!(rail && !gap && text, "rail {rail}, gap {gap}, text {text}");
}
