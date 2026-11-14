//! Drag selection in the chat (the owner, 2026-11-15: "không thể bôi đen
//! chọn text được ở khung chat"): a press and drag over a prompt, a reply or
//! a tool card selects its text, the selection shows, and Ctrl+C copies it,
//! without Select mode. Each test has its own window (the platform is set
//! once per thread), so no selection is left over from another drag.

use std::cell::RefCell;
use std::rc::Rc;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Key, Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, PhysicalSize, Rgb8Pixel, VecModel};

use crate::fold_view_tests::shot;
use crate::real_child_tests::pump;
use crate::{MainWindow, MsgRow};

thread_local! {
    static CLIP: RefCell<String> = const { RefCell::new(String::new()) };
}

/// A platform whose clipboard the test reads.
struct ClipPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for ClipPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }

    fn set_clipboard_text(&self, text: &str, _: slint::platform::Clipboard) {
        CLIP.with(|c| *c.borrow_mut() = text.to_string());
    }

    fn clipboard_text(&self, _: slint::platform::Clipboard) -> Option<String> {
        Some(CLIP.with(|c| c.borrow().clone()))
    }
}

pub(crate) struct Chat {
    pub window: Rc<MinimalSoftwareWindow>,
    pub ui: MainWindow,
}

/// The chat at 1100x760 holding `rows`, laid out.
pub(crate) fn chat(rows: Vec<MsgRow>) -> Chat {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(ClipPlatform(window.clone()))).expect("platform");
    window.set_size(PhysicalSize::new(1100, 760));
    let ui = crate::chat_list::new_window().expect("window");
    ui.show().expect("show");
    ui.set_messages(ModelRc::new(VecModel::from(rows)));
    pump(&window, 400);
    Chat { window, ui }
}

/// The chat list's top in the window (under the title bar and the tabs).
const LIST_Y: f32 = 84.0;

pub(crate) fn row(item: i32, kind: &str, text: &str) -> MsgRow {
    MsgRow {
        item,
        kind: kind.into(),
        text: text.into(),
        shown: text.into(),
        blocks: crate::rows::reply_blocks(text),
        ..MsgRow::default()
    }
}

/// A tool card through the real mapping, opened.
fn tool(item: i32, name: &str, arguments: &str, output: Option<&str>) -> MsgRow {
    let tool = fast_harness_core::view::Item::Tool {
        id: format!("t{item}"),
        name: name.into(),
        arguments: arguments.into(),
        summary: String::new(),
        output: output.map(str::to_string),
        is_error: false,
        duration_ms: None,
        live: None,
    };
    MsgRow {
        item,
        expanded: true,
        ..crate::rows::msg_row(&tool)
    }
}

impl Chat {
    /// Row `item`'s top in the window.
    pub fn top(&self, item: i32) -> f32 {
        self.ui.set_chat_watch_item(-1);
        self.ui.set_chat_watch_y(-100_000.0);
        pump(&self.window, 16);
        self.ui.set_chat_watch_item(item);
        pump(&self.window, 48);
        LIST_Y + self.ui.get_chat_watch_y()
    }

    /// A left-button drag from `from` to `to` along `y`, hovering first.
    pub fn drag(&self, y: f32, from: f32, to: f32) {
        let window = &self.window;
        let button = PointerEventButton::Left;
        let at = |x: f32| LogicalPosition::new(x, y);
        window.dispatch_event(WindowEvent::PointerMoved { position: at(from) });
        pump(window, 16);
        window.dispatch_event(WindowEvent::PointerPressed {
            position: at(from),
            button,
        });
        pump(window, 32);
        for i in 1..=8 {
            let position = at(from + (to - from) * i as f32 / 8.0);
            window.dispatch_event(WindowEvent::PointerMoved { position });
            pump(window, 16);
        }
        window.dispatch_event(WindowEvent::PointerReleased {
            position: at(to),
            button,
        });
        pump(window, 64);
    }

    /// Ctrl+C: what it put on the clipboard (empty for nothing).
    pub fn copy(&self) -> String {
        CLIP.with(|c| c.borrow_mut().clear());
        let ctrl: slint::SharedString = Key::Control.into();
        let window = &self.window;
        window.dispatch_event(WindowEvent::KeyPressed { text: ctrl.clone() });
        window.dispatch_event(WindowEvent::KeyPressed { text: "c".into() });
        window.dispatch_event(WindowEvent::KeyReleased { text: "c".into() });
        window.dispatch_event(WindowEvent::KeyReleased { text: ctrl });
        pump(window, 32);
        CLIP.with(|c| c.borrow().clone())
    }

    /// The window as drawn now.
    pub fn pixels(&self) -> Vec<Rgb8Pixel> {
        let size = self.window.size();
        let mut pixels = vec![Rgb8Pixel { r: 0, g: 0, b: 0 }; (size.width * size.height) as usize];
        self.window.request_redraw();
        self.window.draw_if_needed(|r| {
            r.render(&mut pixels, size.width as usize);
        });
        pixels
    }
}

/// The drag copied a non-empty piece of `whole`.
fn assert_piece(got: &str, whole: &str, what: &str) {
    assert!(
        got.len() > 5 && whole.contains(got),
        "{what}: the drag copied {got:?}"
    );
}

const PROMPT: &str =
    "Please read the build log and tell me which test failed first and why it failed";

#[test]
fn a_dragged_prompt_shows_its_selection() {
    let chat = chat(vec![row(0, "user", PROMPT)]);
    let y = chat.top(0) + 34.0;
    let before = chat.pixels();
    chat.drag(y, 640.0, 980.0);
    shot(&chat.window, "select-prompt");
    let after = chat.pixels();
    assert_piece(&chat.copy(), PROMPT, "prompt");
    // Pixels of the dragged line that the selection changed by a clear
    // margin (the old fill, `selection` on `bubble`, moved them by 28).
    let width = chat.window.size().width as usize;
    let changed = (y as usize - 5..y as usize + 5)
        .flat_map(|py| (650..970).map(move |px| py * width + px))
        .filter(|&i| {
            let (a, b) = (before[i], after[i]);
            a.r.abs_diff(b.r) as u32 + a.g.abs_diff(b.g) as u32 + a.b.abs_diff(b.b) as u32 > 60
        })
        .count();
    assert!(
        changed > 1500,
        "only {changed} px of the dragged span show the selection"
    );
}

fn reply_drag(text: &str, plain: &str, name: &str) {
    let chat = chat(vec![row(0, "user", "Why?"), row(1, "assistant", text)]);
    let y = chat.top(1) + 10.0;
    chat.drag(y, 330.0, 620.0);
    shot(&chat.window, name);
    assert_piece(&chat.copy(), plain, name);
}

#[test]
fn a_drag_copies_from_a_plain_reply_paragraph() {
    let text = "The first failure is the scroll test because the thumb moved before the rows were measured.";
    reply_drag(text, text, "select-reply");
}

#[test]
fn a_drag_copies_from_a_styled_reply_paragraph() {
    let text = "The first failure is **scroll_tests** in `src/scroll_tests.rs` because the thumb moved early.";
    let plain =
        "The first failure is scroll_tests in src/scroll_tests.rs because the thumb moved early.";
    reply_drag(text, plain, "select-reply-styled");
}

#[test]
fn a_link_in_a_reply_still_opens() {
    let text = "[The build log](https://example.com/log) has the first failure.";
    let chat = chat(vec![row(0, "user", "Why?"), row(1, "assistant", text)]);
    let opened = Rc::new(RefCell::new(String::new()));
    let seen = opened.clone();
    chat.ui
        .on_open_url(move |url| *seen.borrow_mut() = url.to_string());
    let y = chat.top(1) + 10.0;
    let at = LogicalPosition::new(LINK_X, y);
    let button = PointerEventButton::Left;
    chat.window
        .dispatch_event(WindowEvent::PointerMoved { position: at });
    pump(&chat.window, 16);
    chat.window.dispatch_event(WindowEvent::PointerPressed {
        position: at,
        button,
    });
    chat.window.dispatch_event(WindowEvent::PointerReleased {
        position: at,
        button,
    });
    pump(&chat.window, 64);
    assert_eq!(*opened.borrow(), "https://example.com/log");
}

/// A point on the reply's first words.
const LINK_X: f32 = 330.0;

const LINE: &str = "let measured = rows.iter().map(|r| r.height).sum::<f32>();";
const FIELD: &str = "Find the test that failed first in the build log and say why it failed";

#[test]
fn a_drag_copies_from_a_tool_line_and_a_field() {
    let args = serde_json::json!({ "prompt": FIELD }).to_string();
    let rows = vec![
        tool(
            0,
            "read",
            r#"{"path":"src/a.rs"}"#,
            Some(&format!("1: {LINE}")),
        ),
        tool(1, "subagent", &args, None),
    ];
    let chat = chat(rows);
    let y = chat.top(0) + CODE_DY;
    chat.drag(y, 360.0, 700.0);
    shot(&chat.window, "select-tool-line");
    assert_piece(&chat.copy(), LINE, "code line");
    let y = chat.top(1) + FIELD_DY;
    chat.drag(y, 430.0, 800.0);
    shot(&chat.window, "select-tool-field");
    assert_piece(&chat.copy(), FIELD, "field value");
}

/// The first code line and the field value below their card's top.
const CODE_DY: f32 = 97.0;
const FIELD_DY: f32 = 64.0;
