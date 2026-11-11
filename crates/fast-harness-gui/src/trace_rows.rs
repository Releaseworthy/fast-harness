//! The Trace page: the timeline of the session log (`trace::detail`) and
//! the step table (`trace::report`), read again on every refresh.

use fast_harness_core::trace::detail::{entries, Entry, EntryKind};
use fast_harness_core::trace::report::{step_rows, StepRow};
use fast_harness_core::{session, trace};
use slint::{Model, ModelRc, SharedString, VecModel};

use crate::app::App;
use crate::format::{duration, tokens};
use crate::{MainWindow, TraceRow};

/// A body longer than this many lines or characters starts collapsed.
pub const PREVIEW_LINES: usize = 8;
pub const PREVIEW_CHARS: usize = 800;

/// What Show all draws at most; Copy still takes the whole body. The software
/// renderer lays a Text's lines out in `i16` physical pixels
/// (i-slint-core `textlayout.rs` `y += line_height`, i-slint-renderer-software
/// `lib.rs` `PhysicalLength = Length<i16, _>`): past 32 767 px the lines wrap
/// round and draw over each other, and the measured height wraps too. 300
/// lines plus 12 000 characters wrapped at 100 a line stay under 450 lines,
/// 450 x 20 px x 3 (300% scale) = 27 000 px.
pub const EXPANDED_LINES: usize = 300;
pub const EXPANDED_CHARS: usize = 12_000;

/// The `kind` word `ui/trace.slint` draws each entry by.
pub fn kind_word(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Turn => "turn",
        EntryKind::User => "user",
        EntryKind::Header => "header",
        EntryKind::Request => "request",
        EntryKind::Thinking => "thinking",
        EntryKind::Response => "response",
        EntryKind::ToolCall => "call",
        EntryKind::ToolResult => "result",
        EntryKind::Answer => "answer",
        EntryKind::Retry => "retry",
        EntryKind::Failure => "failure",
        EntryKind::Switch => "switch",
        EntryKind::TurnEnd => "end",
    }
}

pub fn is_long(body: &str) -> bool {
    body.lines().count() > PREVIEW_LINES || body.chars().count() > PREVIEW_CHARS
}

/// Long bodies start collapsed; the system prompt (Header) always does.
pub fn starts_collapsed(kind: EntryKind, body: &str) -> bool {
    !body.is_empty() && (kind == EntryKind::Header || is_long(body))
}

/// What a collapsed body shows: its first lines, cut at the character
/// limit, with `…` when anything was left out. The system prompt shows none.
pub fn preview(kind: EntryKind, body: &str) -> String {
    if kind == EntryKind::Header {
        return String::new();
    }
    let lines: Vec<&str> = body.lines().take(PREVIEW_LINES).collect();
    let head = lines.join("\n");
    let mut out: String = head.chars().take(PREVIEW_CHARS).collect();
    let cut = body.lines().count() > PREVIEW_LINES || head.chars().count() > PREVIEW_CHARS;
    if cut {
        out.push_str(" …");
    }
    out
}

/// What an expanded body shows: at most `EXPANDED_LINES` lines and
/// `EXPANDED_CHARS` characters, with a note of what was left out.
pub fn expanded(body: &str) -> String {
    let total = body.lines().count();
    let mut out = String::new();
    let (mut lines, mut chars, mut cut) = (0, 0, total > EXPANDED_LINES);
    for line in body.lines().take(EXPANDED_LINES) {
        let room = EXPANDED_CHARS.saturating_sub(chars);
        let len = line.chars().count();
        if lines > 0 {
            out.push('\n');
        }
        out.extend(line.chars().take(room));
        chars += len.min(room) + 1;
        lines += 1;
        if len >= room {
            cut |= len > room || lines < total;
            break;
        }
    }
    if cut {
        let all = body.chars().count();
        out.push_str(&format!(
            "\n... {} more lines, {all} characters in all (Copy takes all)",
            total - lines
        ));
    }
    out
}
pub fn entry_row(entry: &Entry) -> TraceRow {
    // Thinking often ends in blank lines: they would only stretch the block.
    let body = entry.body.trim_end();
    let long = starts_collapsed(entry.kind, body);
    let copy = if body.is_empty() {
        entry.title.clone()
    } else {
        format!("{}\n\n{}", entry.title, body)
    };
    TraceRow {
        kind: kind_word(entry.kind).into(),
        title: entry.title.as_str().into(),
        body: expanded(body).into(),
        preview: if long {
            preview(entry.kind, body).into()
        } else {
            SharedString::default()
        },
        long,
        expanded: false,
        error: entry.is_error || entry.kind == EntryKind::Failure,
        copy: copy.into(),
    }
}

/// The cells of the step table, in the column order of `ui/trace.slint`.
pub fn step_cells(row: &StepRow) -> Vec<SharedString> {
    let tools: Vec<&str> = row.tools.iter().map(|(name, _)| name.as_str()).collect();
    let cells = [
        row.turn.to_string(),
        row.step.to_string(),
        row.model.clone(),
        row.account.clone(),
        duration(row.ttfb_ms),
        duration(row.total_ms),
        tokens(row.usage.input),
        tokens(row.usage.output),
        tokens(row.usage.cache_read),
        tokens(row.usage.cache_write),
        tools.join(", "),
        row.retries.to_string(),
    ];
    cells.iter().map(|c| c.as_str().into()).collect()
}

fn set_rows(ui: &MainWindow, timeline: Vec<TraceRow>, steps: Vec<ModelRc<SharedString>>) {
    ui.set_trace_entries(ModelRc::new(VecModel::from(timeline)));
    ui.set_trace_steps(ModelRc::new(VecModel::from(steps)));
}

/// Reads the current session log into both views.
pub fn refresh(app: &App, ui: &MainWindow) {
    let Some(id) = app.conv_id.as_deref() else {
        ui.set_trace_session("".into());
        ui.set_trace_message("No session yet: send a message or open a session.".into());
        return set_rows(ui, Vec::new(), Vec::new());
    };
    ui.set_trace_session(id.into());
    let path = session::log_path(&app.rt.home, &app.cwd, id);
    match trace::read(&path) {
        Ok((_, events)) => {
            let timeline: Vec<TraceRow> = entries(&events).iter().map(entry_row).collect();
            let steps = step_rows(&events)
                .iter()
                .map(|r| ModelRc::new(VecModel::from(step_cells(r))))
                .collect();
            let note = if timeline.is_empty() {
                "Nothing in this session log yet."
            } else {
                ""
            };
            ui.set_trace_message(note.into());
            set_rows(ui, timeline, steps);
        }
        Err(err) => {
            ui.set_trace_message(format!("Cannot read {}: {err}", path.display()).into());
            set_rows(ui, Vec::new(), Vec::new());
        }
    }
}

/// Show all / Show less on one timeline entry.
pub fn toggle(ui: &MainWindow, index: i32) {
    let rows = ui.get_trace_entries();
    let Some(i) = usize::try_from(index).ok() else {
        return;
    };
    if let Some(mut row) = rows.row_data(i) {
        row.expanded = !row.expanded;
        rows.set_row_data(i, row);
    }
}

#[cfg(test)]
#[path = "trace_rows_tests.rs"]
mod tests;
