//! The empty space at the end of the transcript while the session popup
//! shows (PLAN.md D81): one trailing row of kind "spacer", as tall as the
//! popup (`TranscriptView.pad`), so the list keeps its full height and the
//! last real row ends right at the popup's top. The mirror never sees it:
//! `count` stops before it and `mirror::Rows` inserts above it.

use slint::{Model, VecModel};

use crate::{MainWindow, MsgRow};

pub const KIND: &str = "spacer";

/// The rows before the spacer (all of them when there is none).
pub fn count(rows: &VecModel<MsgRow>) -> usize {
    let n = rows.row_count();
    let last = n.checked_sub(1).and_then(|i| rows.row_data(i));
    match last {
        Some(row) if row.kind == KIND => n - 1,
        _ => n,
    }
}

/// Adds the spacer at the end (`on`) or removes it.
pub fn set(rows: &VecModel<MsgRow>, on: bool) {
    let n = rows.row_count();
    let has = count(rows) < n;
    if on && !has {
        let row = MsgRow {
            kind: KIND.into(),
            ..Default::default()
        };
        rows.push(row);
    } else if !on && has {
        rows.remove(n - 1);
    }
}

/// The spacer follows the popup: an approval request or a question shown.
pub fn show(ui: &MainWindow, rows: &VecModel<MsgRow>) {
    let popup = !ui.get_approval().id.is_empty() || !ui.get_ask_id().is_empty();
    set(rows, popup);
}
