//! One selection across the chat's blocks (the owner, 2026-11-15): a drag
//! over several paragraphs and list items of a reply, a Shift+click, and a
//! drag over a tool card's detail line and code lines each copy every
//! block in between, joined with newlines in block order.

use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::LogicalPosition;

use super::{joined, Range};
use crate::fold_view_tests::shot;
use crate::real_child_tests::pump;
use crate::text_select_tests::{chat, row, tool, Chat};

const P1: &str = "First paragraph alpha words here.";
const P2: &str = "Second paragraph beta words.";
const LI1: &str = "item gamma one";
const LI2: &str = "item delta two";

fn reply() -> String {
    format!("{P1}\n\n{P2}\n\n- {LI1}\n- {LI2}")
}

fn at(x: f32, y: f32) -> LogicalPosition {
    LogicalPosition::new(x, y)
}

/// A left-button drag from `from` to `to` in 12 moves, hovering first.
fn drag(chat: &Chat, from: (f32, f32), to: (f32, f32)) {
    let window = &chat.window;
    let button = PointerEventButton::Left;
    window.dispatch_event(WindowEvent::PointerMoved { position: at(from.0, from.1) });
    pump(window, 16);
    window.dispatch_event(WindowEvent::PointerPressed { position: at(from.0, from.1), button });
    pump(window, 32);
    for i in 1..=12 {
        let f = i as f32 / 12.0;
        let position = at(from.0 + (to.0 - from.0) * f, from.1 + (to.1 - from.1) * f);
        window.dispatch_event(WindowEvent::PointerMoved { position });
        pump(window, 32);
    }
    window.dispatch_event(WindowEvent::PointerReleased { position: at(to.0, to.1), button });
    pump(window, 64);
}

/// A click at `x, y`, with Shift held when `shift`.
fn click(chat: &Chat, x: f32, y: f32, shift: bool) {
    let window = &chat.window;
    let button = PointerEventButton::Left;
    if shift {
        window.dispatch_event(WindowEvent::KeyPressed { text: Key::Shift.into() });
    }
    window.dispatch_event(WindowEvent::PointerMoved { position: at(x, y) });
    pump(window, 16);
    window.dispatch_event(WindowEvent::PointerPressed { position: at(x, y), button });
    window.dispatch_event(WindowEvent::PointerReleased { position: at(x, y), button });
    pump(window, 48);
    if shift {
        window.dispatch_event(WindowEvent::KeyReleased { text: Key::Shift.into() });
    }
    pump(window, 300);
}

/// Copied text: the first line ends `first`, then every full block, then
/// a non-empty start of `last`.
fn assert_spans(got: &str, first: &str, full: &[&str], last: &str) {
    let lines: Vec<&str> = got.lines().collect();
    assert_eq!(lines.len(), full.len() + 2, "copied {got:?}");
    assert!(!lines[0].is_empty() && first.ends_with(lines[0]), "first part of {got:?}");
    assert_eq!(&lines[1..=full.len()], full, "middle of {got:?}");
    let tail = lines[full.len() + 1];
    assert!(!tail.is_empty() && last.starts_with(tail), "last part of {got:?}");
}

#[test]
fn a_drag_selects_across_paragraphs_and_list_items() {
    let chat = chat(vec![row(0, "user", "Why?"), row(1, "assistant", &reply())]);
    let top = chat.top(1);
    drag(&chat, (360.0, top + 10.0), (390.0, top + LI1_DY));
    shot(&chat.window, "cross-select-reply");
    assert_spans(&chat.copy(), P1, &[P2], LI1);
}

#[test]
fn a_shift_click_extends_over_blocks() {
    let chat = chat(vec![row(0, "user", "Why?"), row(1, "assistant", &reply())]);
    let top = chat.top(1);
    click(&chat, 360.0, top + 10.0, false);
    click(&chat, 380.0, top + LI2_DY, true);
    assert_spans(&chat.copy(), P1, &[P2, LI1], LI2);
}

/// The first list item and the second below the reply's top.
const LI1_DY: f32 = 62.0;
const LI2_DY: f32 = 84.0;

const CODE: [&str; 3] = [
    "let first = rows.iter().count();",
    "let second = first * 2;",
    "let third = second + first;",
];

#[test]
fn a_drag_selects_across_a_tool_cards_code_lines() {
    let output = format!("1: {}\n2: {}\n3: {}", CODE[0], CODE[1], CODE[2]);
    let chat = chat(vec![tool(0, "read", r#"{"path":"src/a.rs"}"#, Some(&output))]);
    let top = chat.top(0);
    drag(&chat, (380.0, top + CODE_DY), (420.0, top + CODE_DY + 2.0 * CODE_STEP));
    shot(&chat.window, "cross-select-tool");
    assert_spans(&chat.copy(), CODE[0], &[CODE[1]], CODE[2]);
}

/// The first code line below the card's top, and the line pitch.
const CODE_DY: f32 = 97.0;
const CODE_STEP: f32 = 18.0;

#[test]
fn joined_takes_the_ends_parts_and_whole_blocks_between() {
    let parts = [((1, 0), "abc def"), ((1, 1), "ghi"), ((2, -1), "jkl mno")]
        .into_iter()
        .map(|(k, v)| (k, v.to_string()))
        .collect();
    let range = Range::new((2, -1, 3), (1, 0, 4));
    assert_eq!(joined(&parts, &range), "def\nghi\njkl");
    let one = Range::new((1, 0, 6), (1, 0, 1));
    assert_eq!(joined(&parts, &one), "bc de");
}

/// A tool row as the chat shows it in a step: its detail line, card open.
fn detailed(output: &str) -> crate::MsgRow {
    crate::MsgRow {
        detail_line: true,
        card_open: true,
        ..tool(0, "read", r#"{"path":"src/a.rs"}"#, Some(output))
    }
}

#[test]
fn a_drag_from_a_detail_line_selects_into_its_code() {
    let output = format!("1: {}\n2: {}\n3: {}", CODE[0], CODE[1], CODE[2]);
    let chat = chat(vec![detailed(&output)]);
    let top = chat.top(0);
    drag(&chat, (DETAIL_X, top + DETAIL_DY), (420.0, top + DETAIL_CODE_DY));
    shot(&chat.window, "cross-select-detail");
    let got = chat.copy();
    let lines: Vec<&str> = got.lines().collect();
    assert_eq!(lines.len(), 2, "copied {got:?}");
    assert!("src/a.rs".ends_with(lines[0]) && !lines[0].is_empty(), "copied {got:?}");
    assert!(CODE[0].starts_with(lines[1]) && !lines[1].is_empty(), "copied {got:?}");
}

/// A point on the detail line's target, and the first code line.
const DETAIL_X: f32 = 371.0;
const DETAIL_DY: f32 = 20.0;
const DETAIL_CODE_DY: f32 = 127.0;

#[test]
fn a_click_on_a_detail_line_still_toggles_and_a_drag_does_not() {
    let output = format!("1: {}", CODE[0]);
    let chat = chat(vec![detailed(&output)]);
    let toggles = std::rc::Rc::new(std::cell::Cell::new(0));
    let seen = toggles.clone();
    chat.ui.on_card_toggle(move |_| seen.set(seen.get() + 1));
    let top = chat.top(0);
    drag(&chat, (DETAIL_X, top + DETAIL_DY), (DETAIL_X + 12.0, top + DETAIL_DY));
    assert_eq!(toggles.get(), 0, "a drag on the line toggled it");
    assert!(!chat.copy().is_empty(), "the drag selected nothing");
    // The first click clears the selection, the next one toggles.
    click(&chat, DETAIL_X, top + DETAIL_DY, false);
    click(&chat, DETAIL_X, top + DETAIL_DY, false);
    assert_eq!(toggles.get(), 1, "a click after the selection did not toggle");
}

const AFTER: &str = "After the code beta words.";

/// A reply with a fenced code block between two paragraphs.
fn code_reply() -> String {
    format!("{P1}\n\n```\n{}\n{}\n```\n\n{AFTER}", CODE[0], CODE[1])
}

#[test]
fn a_drag_selects_across_a_replys_code_block() {
    let chat = chat(vec![row(0, "user", "Why?"), row(1, "assistant", &code_reply())]);
    let top = chat.top(1);
    drag(&chat, (360.0, top + 10.0), (390.0, top + AFTER_DY));
    shot(&chat.window, "cross-select-code-block");
    assert_spans(&chat.copy(), P1, &[CODE[0], CODE[1]], AFTER);
}

/// The paragraph after the code block below the reply's top.
const AFTER_DY: f32 = 100.0;

/// The pixels on the band `y +- 6` that differ from the band's most
/// common colour (its background): the ink of the text drawn there.
fn ink(chat: &Chat, y: f32) -> usize {
    let pixels = chat.pixels();
    let width = chat.window.size().width as usize;
    let rows = (y as usize - 6)..(y as usize + 6);
    let band: Vec<_> = rows.flat_map(|r| pixels[r * width..(r + 1) * width].to_vec()).collect();
    let mut counts = std::collections::HashMap::new();
    for p in &band {
        *counts.entry((p.r, p.g, p.b)).or_insert(0usize) += 1;
    }
    let bg = counts.into_iter().max_by_key(|(_, n)| *n).map(|(c, _)| c).unwrap_or_default();
    let far = |p: &slint::Rgb8Pixel| {
        let d = |a: u8, b: u8| (a as i32 - b as i32).abs();
        d(p.r, bg.0) + d(p.g, bg.1) + d(p.b, bg.2) > 60
    };
    band.iter().filter(|p| far(p)).count()
}

#[test]
fn a_shift_click_that_shrinks_the_selection_keeps_the_dropped_items_text() {
    let chat = chat(vec![row(0, "user", "Why?"), row(1, "assistant", &reply())]);
    let top = chat.top(1);
    let before = ink(&chat, top + LI1_DY);
    drag(&chat, (360.0, top + 10.0), (390.0, top + LI2_DY));
    sweep(&chat, (390.0, top + LI2_DY), (380.0, top + P2_DY));
    click(&chat, 380.0, top + P2_DY, true);
    sweep(&chat, (380.0, top + P2_DY), (700.0, top + 200.0));
    shot(&chat.window, "cross-select-shrink");
    let after = ink(&chat, top + LI1_DY);
    assert!(after * 5 >= before * 4, "the dropped item lost its text: ink {after} of {before}");
    assert_spans(&chat.copy(), P1, &[], P2);
}

/// The second paragraph below the reply's top.
const P2_DY: f32 = 36.0;

const ASK: &str = "Please read the build log and tell me which test failed first";

#[test]
fn a_drag_selects_from_a_user_message_into_the_reply() {
    let chat = chat(vec![row(0, "user", ASK), row(1, "assistant", &reply())]);
    let (user, top) = (chat.top(0), chat.top(1));
    drag(&chat, (ASK_X, user + 34.0), (390.0, top + 10.0));
    shot(&chat.window, "cross-select-user");
    assert_spans(&chat.copy(), ASK, &[], P1);
}

/// A point inside the user's bubble text.
const ASK_X: f32 = 700.0;

/// Moves the pointer from `from` to `to` in 10 steps, no button.
fn sweep(chat: &Chat, from: (f32, f32), to: (f32, f32)) {
    for i in 0..=10 {
        let f = i as f32 / 10.0;
        let position = at(from.0 + (to.0 - from.0) * f, from.1 + (to.1 - from.1) * f);
        chat.window.dispatch_event(WindowEvent::PointerMoved { position });
        pump(&chat.window, 32);
    }
}
