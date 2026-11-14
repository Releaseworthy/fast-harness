//! The Trace tab's timeline list (2026-11-14): the same virtual list with
//! measured row heights as the chat (`virtual_list`), so the thumb keeps
//! its size while the view moves (Slint's ListView estimated the content
//! height from the rows it had laid out: from about 26,000 px to 216,000 px
//! during one drag of the owner's log 6143, measured 35,417 px). The view is
//! `TraceTimeline` in `ui/trace.slint`; it talks through the `TraceList`
//! global, since the page sits under an `if`.

use slint::{ComponentHandle, ModelRc};

use crate::virtual_list::{self, Host};
use crate::{MainWindow, Theme, TraceList, TraceRow};

/// Gives `ui`'s trace timeline its list: the models and the callbacks.
pub fn attach(ui: &MainWindow) {
    let list = virtual_list::attach::<Trace>(ui);
    let global = ui.global::<TraceList>();
    let shared = list.clone();
    global.on_sync(move || shared.sync());
    let shared = list.clone();
    global.on_anchor(move |row, head, turn| shared.pin(row, head, turn));
    let shared = list.clone();
    global.on_measured(move |row, item, height| shared.measured(row, item, height));
    list.sync();
}

/// The timeline's entries (`trace-entries`), keyed by their index.
pub struct Trace;

impl Host for Trace {
    type Row = TraceRow;
    fn rows(ui: &MainWindow) -> ModelRc<TraceRow> {
        ui.get_trace_entries()
    }
    fn item(_row: &TraceRow, i: usize) -> i32 {
        i32::try_from(i).unwrap_or(i32::MAX)
    }
    fn estimate(row: &TraceRow, width: f32) -> f32 {
        estimate(row, width)
    }
    fn offset(ui: &MainWindow) -> f32 {
        ui.global::<TraceList>().get_offset()
    }
    fn set_offset(ui: &MainWindow, offset: f32) {
        ui.global::<TraceList>().set_offset(offset);
    }
    fn view(ui: &MainWindow) -> f32 {
        ui.global::<TraceList>().get_list_height()
    }
    fn style(ui: &MainWindow) -> (f32, f32) {
        let width = ui.global::<TraceList>().get_width();
        (width, ui.global::<Theme>().get_font_m())
    }
    /// Show less on an entry.
    fn collapsed(before: &TraceRow, after: Option<&TraceRow>) -> bool {
        after.is_none_or(|after| before.expanded && !after.expanded)
    }
    fn set_models(ui: &MainWindow, tops: ModelRc<f32>, slots: ModelRc<i32>, probes: ModelRc<i32>) {
        let global = ui.global::<TraceList>();
        global.set_tops(tops);
        global.set_slots(slots);
        global.set_probes(probes);
    }
}

/// An entry's height before it is measured: its title line and the text
/// it shows at the column's width.
fn estimate(row: &TraceRow, width: f32) -> f32 {
    if row.kind == "turn" {
        return 52.0;
    }
    if !row.tool.kind.is_empty() {
        return if row.expanded { 160.0 } else { 64.0 };
    }
    let shown = if row.long && !row.expanded {
        &row.preview
    } else {
        &row.body
    };
    let per_line = ((width.min(860.0) - 40.0).max(200.0) / 7.5).max(20.0);
    let count = |line: &str| (line.chars().count() as f32 / per_line).ceil().max(1.0);
    let lines: f32 = shown.lines().map(count).sum();
    let boxed = matches!(row.kind.as_str(), "call" | "result" | "header" | "user");
    32.0 + 18.0 * lines + if boxed { 20.0 } else { 0.0 }
}

#[cfg(test)]
#[path = "trace_list_tests.rs"]
mod tests;
