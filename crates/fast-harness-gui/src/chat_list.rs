//! The chat list (2026-11-14): the transcript draws through its own
//! virtual list with measured row heights (`virtual_list`), not Slint's
//! ListView, whose content height was the average height of the rows it
//! had laid out times the row count: the chat's rows run from 24 px to
//! about 2900 px, so the thumb changed size as the view moved, a drag of
//! the thumb jumped and a fold moved the view. This module is the chat's
//! side of it: its rows, estimates, properties and toggled lines; it also
//! attaches the Trace tab's list (`trace_list`).

use slint::{ComponentHandle, ModelRc};

use crate::virtual_list::{self, Host};
use crate::{MainWindow, MsgRow, Theme, TurnsUi};

/// A main window with its chat and trace lists attached.
pub fn new_window() -> Result<MainWindow, slint::PlatformError> {
    let ui = MainWindow::new()?;
    attach(&ui);
    crate::trace_list::attach(&ui);
    crate::reply_select::attach(&ui);
    Ok(ui)
}

/// Gives `ui`'s transcript its list: the models and the callbacks.
pub fn attach(ui: &MainWindow) {
    let list = virtual_list::attach::<Chat>(ui);
    let shared = list.clone();
    ui.on_chat_sync(move || shared.sync());
    let shared = list.clone();
    ui.on_chat_anchor(move |row, head, turn| shared.pin(row, head, turn));
    let shared = list.clone();
    ui.on_chat_measured(move |row, item, height| shared.measured(row, item, height));
    let shared = list.clone();
    ui.on_chat_go(move |item| shared.go(item));
    list.sync();
}

/// The chat's rows (`messages`), keyed by their transcript index.
pub struct Chat;

impl Host for Chat {
    type Row = MsgRow;
    fn rows(ui: &MainWindow) -> ModelRc<MsgRow> {
        ui.get_messages()
    }
    fn item(row: &MsgRow, _i: usize) -> i32 {
        row.item
    }
    fn shows(row: &MsgRow) -> bool {
        crate::rows::shows(row)
    }
    fn spacer(row: &MsgRow) -> bool {
        row.kind == "spacer"
    }
    fn estimate(row: &MsgRow, width: f32) -> f32 {
        estimate(row, width)
    }
    fn offset(ui: &MainWindow) -> f32 {
        ui.get_chat_offset()
    }
    fn set_offset(ui: &MainWindow, offset: f32) {
        ui.set_chat_offset(offset);
    }
    fn view(ui: &MainWindow) -> f32 {
        ui.get_chat_list_height()
    }
    fn pad(ui: &MainWindow) -> f32 {
        ui.get_chat_pad()
    }
    fn style(ui: &MainWindow) -> (f32, f32) {
        (ui.get_chat_width(), ui.global::<Theme>().get_font_m())
    }
    fn follow(ui: &MainWindow) -> bool {
        ui.get_transcript_follow()
    }
    fn set_follow(ui: &MainWindow, follow: bool) {
        ui.set_transcript_follow(follow);
    }
    fn top_row(ui: &MainWindow, item: i32) {
        ui.global::<TurnsUi>().set_top_row(item);
    }
    /// The step's head row, or the folded turn's line.
    fn line(rows: &[MsgRow], head: i32, turn: bool) -> Option<usize> {
        if turn {
            let line = |r: &MsgRow| r.turn_head == head && !r.turn_line.is_empty();
            rows.iter().position(line)
        } else {
            rows.iter().position(|r| r.item == head)
        }
    }
    /// A fold: a turn or step folded away (the row left the list), its
    /// thinking or tool output closed, its step or turn folded.
    fn collapsed(before: &MsgRow, after: Option<&MsgRow>) -> bool {
        let Some(after) = after else {
            return true;
        };
        (before.expanded && !after.expanded)
            || (!before.folded && after.folded)
            || (before.step_open && !after.step_open)
            || (!before.turn_hidden && after.turn_hidden)
    }
    fn set_models(ui: &MainWindow, tops: ModelRc<f32>, slots: ModelRc<i32>, probes: ModelRc<i32>) {
        ui.set_chat_tops(tops);
        ui.set_chat_slots(slots);
        ui.set_chat_probes(probes);
    }
}

/// A row's height before it is measured: from its kind and its text's
/// length at the list's width, never from other rows.
fn estimate(row: &MsgRow, width: f32) -> f32 {
    // A row that draws nothing (a folded step's row) is 0 px tall, and its
    // height never changes, so it may never report one.
    if !crate::rows::shows(row) {
        return 0.0;
    }
    let column = (width.min(860.0) - 80.0).max(200.0);
    let per_line = (column / 7.5).max(20.0);
    let lines = |text: &str| -> f32 {
        let count = |line: &str| (line.chars().count() as f32 / per_line).ceil().max(1.0);
        text.lines().map(count).sum()
    };
    let line = if row.step_summary.is_empty() || row.turn_hidden {
        0.0
    } else {
        30.0
    };
    let body = match row.kind.as_str() {
        "user" => 64.0 + 20.0 * lines(&row.text),
        "assistant" if row.text.is_empty() => 10.0,
        "assistant" => 36.0 + 20.0 * lines(&row.text),
        "tool" if row.folded => 10.0,
        "tool" => 52.0,
        "note" => 12.0 + 16.0 * lines(&row.text),
        _ => 28.0,
    };
    line + body
}
