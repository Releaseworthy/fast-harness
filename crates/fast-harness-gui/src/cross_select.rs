//! One selection across the chat's blocks (the owner, 2026-11-15: a drag
//! in a reply selected one paragraph only, Shift+click did not extend, tool
//! cards and step lines could not be selected). The state and the blocks'
//! side live in `ui/cross_select.slint` and the `Ghost` of
//! `ui/selectable.slint`; this side keeps the text of every block in the
//! range (for Ctrl+C, joined with newlines in block order) and re-presses
//! the mouse where a drag enters another block, so Slint's own hit test
//! puts the cursor at the exact character there (Slint 1.18.1 has no public
//! character-at-point query).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};

use crate::{CrossSelect, MainWindow};

/// The text of each block shown in the range, by (row, idx).
type Parts = Rc<RefCell<BTreeMap<(i32, i32), String>>>;

/// Connects the chat's selection to `ui`.
pub fn attach(ui: &MainWindow) {
    let parts: Parts = Rc::default();
    let global = ui.global::<CrossSelect>();
    let shared = parts.clone();
    global.on_put(move |row, idx, text| {
        shared.borrow_mut().insert((row, idx), text.to_string());
    });
    let shared = parts.clone();
    global.on_cleared(move || shared.borrow_mut().clear());
    let weak = ui.as_weak();
    global.on_copy(move || match weak.upgrade() {
        Some(ui) => joined(&parts.borrow(), &Range::of(&ui)).into(),
        None => Default::default(),
    });
    let weak = ui.as_weak();
    global.on_hand_off(move |px, py, mx, my| {
        let weak = weak.clone();
        // Deferred: the event that asked for it is still being delivered.
        slint::Timer::single_shot(Duration::ZERO, move || {
            if let Some(ui) = weak.upgrade() {
                hand_off(&ui, LogicalPosition::new(px, py), LogicalPosition::new(mx, my));
            }
        });
    });
}

/// Ends the press the drag holds and presses again at `press`, then moves
/// to `to`: the block under `press` takes the mouse and the focus.
fn hand_off(ui: &MainWindow, press: LogicalPosition, to: LogicalPosition) {
    let window = ui.window();
    let button = PointerEventButton::Left;
    window.dispatch_event(WindowEvent::PointerReleased { position: to, button });
    window.dispatch_event(WindowEvent::PointerPressed { position: press, button });
    if press != to {
        window.dispatch_event(WindowEvent::PointerMoved { position: to });
    }
    ui.global::<CrossSelect>().set_handing(false);
}

/// The selection's ends in block order: (row, idx, byte offset).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Range {
    pub first: (i32, i32, usize),
    pub last: (i32, i32, usize),
}

impl Range {
    /// The selection `ui` holds now.
    pub fn of(ui: &MainWindow) -> Range {
        let g = ui.global::<CrossSelect>();
        let offset = |v: i32| usize::try_from(v).unwrap_or(0);
        let anchor = (g.get_a_row(), g.get_a_idx(), offset(g.get_a_off()));
        let focus = (g.get_f_row(), g.get_f_idx(), offset(g.get_f_off()));
        Range::new(anchor, focus)
    }

    /// The range between two ends given in any order.
    pub fn new(a: (i32, i32, usize), b: (i32, i32, usize)) -> Range {
        if a <= b {
            Range { first: a, last: b }
        } else {
            Range { first: b, last: a }
        }
    }
}

/// The selected text of `parts` in `range`, one block per line.
pub fn joined(parts: &BTreeMap<(i32, i32), String>, range: &Range) -> String {
    let (first, last) = (range.first, range.last);
    let mut out = Vec::new();
    for (&(row, idx), text) in parts.range((first.0, first.1)..=(last.0, last.1)) {
        let start = if (row, idx) == (first.0, first.1) { first.2 } else { 0 };
        let end = if (row, idx) == (last.0, last.1) { last.2 } else { text.len() };
        out.push(slice(text, start, end));
    }
    out.join("\n")
}

/// `text[start..end]`, clamped to its length and char boundaries.
fn slice(text: &str, start: usize, end: usize) -> &str {
    let fit = |mut i: usize| {
        i = i.min(text.len());
        while !text.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let (start, end) = (fit(start), fit(end));
    text.get(start.min(end)..end).unwrap_or("")
}

#[cfg(test)]
#[path = "cross_select_tests.rs"]
mod tests;
