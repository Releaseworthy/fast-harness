//! More of the chat's one selection (the owner, 2026-11-15): a reply
//! paragraph with a link, a markdown table's cells and a tool card's
//! header take part in a drag and Ctrl+C like any other text, and a plain
//! click on a link still opens it.

use std::cell::RefCell;
use std::rc::Rc;

use slint::platform::{PointerEventButton, WindowEvent};
use slint::LogicalPosition;

use crate::fold_view_tests::shot;
use crate::real_child_tests::pump;
use crate::text_select_tests::{chat, row, tool};

const LINKED: &str =
    "See [the build log](https://example.com/log) for the first failure of the scroll test.";

#[test]
fn a_drag_copies_a_paragraph_with_a_link() {
    let chat = chat(vec![row(0, "user", "Why?"), row(1, "assistant", LINKED)]);
    let y = chat.top(1) + 10.0;
    chat.drag(y, 309.0, 700.0);
    shot(&chat.window, "select-reply-link");
    let got = chat.copy();
    assert!(got.starts_with("See the build log for"), "copied {got:?}");
}

#[test]
fn a_click_on_a_link_opens_it_and_a_drag_does_not() {
    let chat = chat(vec![row(0, "user", "Why?"), row(1, "assistant", LINKED)]);
    let opened = Rc::new(RefCell::new(Vec::<String>::new()));
    let seen = opened.clone();
    chat.ui
        .on_open_url(move |url| seen.borrow_mut().push(url.to_string()));
    let y = chat.top(1) + 10.0;
    let at = LogicalPosition::new(LINK_X, y);
    let button = PointerEventButton::Left;
    chat.window
        .dispatch_event(WindowEvent::PointerMoved { position: at });
    pump(&chat.window, 16);
    chat.window
        .dispatch_event(WindowEvent::PointerPressed { position: at, button });
    pump(&chat.window, 32);
    // The window's release goes through the winit hook first.
    super::released(&chat.ui);
    chat.window
        .dispatch_event(WindowEvent::PointerReleased { position: at, button });
    pump(&chat.window, 64);
    assert_eq!(*opened.borrow(), ["https://example.com/log"]);
    chat.drag(y, LINK_X, LINK_X + 200.0);
    super::released(&chat.ui);
    assert_eq!(opened.borrow().len(), 1, "a drag opened {:?}", opened.borrow());
}

/// A point on "the build log".
const LINK_X: f32 = 360.0;

const TABLE: &str =
    "Sizes below.\n\n| Name | Size |\n|---|---|\n| alpha | 10 |\n| beta | 20 |";

#[test]
fn a_drag_copies_table_cells_by_tab_and_row() {
    let chat = chat(vec![row(0, "user", "Why?"), row(1, "assistant", TABLE)]);
    let top = chat.top(1);
    super::tests::drag(&chat, (317.0, top + HEAD_DY), (CELL2_X, top + HEAD_DY + 2.0 * ROW_STEP));
    shot(&chat.window, "select-table");
    let got = chat.copy();
    assert_eq!(got, "Name\tSize\nalpha\t10\nbeta\t2", "copied {got:?}");
}

/// The header row's middle below the reply's top, the row pitch and a
/// point at the start of the second column's text.
const HEAD_DY: f32 = 43.0;
const ROW_STEP: f32 = 22.0;
const CELL2_X: f32 = 733.0;

#[test]
fn a_drag_copies_a_tool_cards_header() {
    let chat = chat(vec![tool(0, "read", r#"{"path":"src/gui/a.rs"}"#, Some("1: let a = 1;"))]);
    let y = chat.top(0) + HEADER_DY;
    chat.drag(y, NAME_X, 760.0);
    shot(&chat.window, "select-tool-header");
    let got = chat.copy();
    assert_eq!(got, "read a.rs src/gui", "copied {got:?}");
}

/// The header's middle below the card's top and a point on the name's first letter.
const HEADER_DY: f32 = 35.0;
const NAME_X: f32 = 375.0;
