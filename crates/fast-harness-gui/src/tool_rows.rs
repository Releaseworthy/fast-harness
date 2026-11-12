//! `view::ToolDetail` to the tool card rows (`ToolView` in ui/types.slint),
//! for the chat cards (src/rows.rs) and the Trace page (src/trace_rows.rs).

use fast_harness_core::trace::detail::{EntryKind, EntryTool};
use fast_harness_core::view::{tool_detail, Chip, Field, Line, ToolDetail, ToolKind};
use slint::{ModelRc, VecModel};

use crate::trace_rows::expanded;
use crate::{ChipRow, CodeLine, ToolChip, ToolField, ToolView};

/// A collapsed card shows this many code or text lines.
pub const PREVIEW_LINES: usize = 10;
/// A collapsed field shows this many lines.
pub const FIELD_LINES: usize = 3;
/// A drawn line is cut at this many characters (Copy takes all).
pub const LINE_CHARS: usize = 400;
/// Characters of chip text on one chip line, 4 more per chip for its
/// padding: about 470 px of the card at the extra-small size.
pub const CHIP_ROW_CHARS: usize = 72;

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

/// `text` cut at `max` characters with `…`; tabs as four spaces.
pub fn cut(text: &str, max: usize) -> String {
    let clean = text.replace('\t', "    ").replace('\r', "");
    if clean.chars().count() <= max {
        return clean;
    }
    let mut out: String = clean.chars().take(max).collect();
    out.push('…');
    out
}

/// The first `n` lines (each cut at [`LINE_CHARS`]), ` …` when more follow.
pub fn head(text: &str, n: usize) -> String {
    let lines: Vec<String> = text.lines().take(n).map(|l| cut(l, LINE_CHARS)).collect();
    let mut out = lines.join("\n");
    if text.lines().count() > n {
        out.push_str(" …");
    }
    out
}

/// File name and folder of a read, write or edit target; the other kinds
/// keep their whole target (shell gets its `$`).
pub fn split_target(kind: ToolKind, target: &str) -> (String, String) {
    match kind {
        ToolKind::Read | ToolKind::Write | ToolKind::Edit => {}
        ToolKind::Shell if !target.is_empty() => return (format!("$ {target}"), String::new()),
        _ => return (cut(target, LINE_CHARS), String::new()),
    }
    let path = target.trim_end_matches(['/', '\\']);
    match path.rfind(['/', '\\']) {
        Some(at) => {
            let dir = if at == 0 { &path[..1] } else { &path[..at] };
            (path[at + 1..].to_string(), dir.to_string())
        }
        None => (path.to_string(), String::new()),
    }
}

/// The chips in lines of at most [`CHIP_ROW_CHARS`] characters.
pub fn chip_rows(chips: &[Chip]) -> Vec<ChipRow> {
    let mut rows: Vec<Vec<ToolChip>> = Vec::new();
    let mut used = 0;
    for chip in chips {
        let width = chip.text.chars().count().min(CHIP_ROW_CHARS) + 4;
        if rows.is_empty() || used + width > CHIP_ROW_CHARS + 4 {
            rows.push(Vec::new());
            used = 0;
        }
        used += width;
        if let Some(row) = rows.last_mut() {
            row.push(ToolChip {
                text: cut(&chip.text, CHIP_ROW_CHARS).into(),
                tone: chip.tone.word().into(),
            });
        }
    }
    rows.into_iter()
        .map(|chips| ChipRow {
            chips: model(chips),
        })
        .collect()
}

fn number(n: Option<u64>) -> String {
    n.map(|n| n.to_string()).unwrap_or_default()
}

pub fn code_line(line: &Line) -> CodeLine {
    CodeLine {
        kind: line.kind.word().into(),
        old: number(line.old).into(),
        new: number(line.new).into(),
        text: cut(&line.text, LINE_CHARS).into(),
    }
}

pub fn field(field: &Field) -> ToolField {
    ToolField {
        label: field.label.as_str().into(),
        value: expanded(&field.value).into(),
        preview: head(&field.value, FIELD_LINES).into(),
    }
}

/// Digits of the widest line number; 0 when no line has one.
fn digits(lines: &[Line]) -> i32 {
    let widest = lines.iter().flat_map(|l| [l.old, l.new]).flatten().max();
    widest.map_or(0, |n| n.to_string().len() as i32)
}

/// What the Show all link says, when the card has more than its preview.
fn more_label(detail: &ToolDetail) -> Option<String> {
    let text_lines = detail.text.lines().count();
    if detail.lines.len() > PREVIEW_LINES {
        Some(format!("Show all {} lines", detail.lines.len()))
    } else if detail.lines.is_empty() && text_lines > PREVIEW_LINES {
        Some(format!("Show all {text_lines} lines"))
    } else {
        let long_field = detail
            .fields
            .iter()
            .any(|f| f.value.lines().count() > FIELD_LINES);
        long_field.then(|| "Show all".to_string())
    }
}

pub fn tool_view(name: &str, detail: &ToolDetail) -> ToolView {
    let (target, target_dir) = split_target(detail.kind, &detail.target);
    let lines: Vec<CodeLine> = detail.lines.iter().map(code_line).collect();
    let preview = lines.iter().take(PREVIEW_LINES).cloned().collect();
    let hidden = detail.lines_total.saturating_sub(detail.lines.len());
    let more = more_label(detail);
    ToolView {
        kind: detail.kind.word().into(),
        name: name.into(),
        target: target.into(),
        target_dir: target_dir.into(),
        chip_rows: model(chip_rows(&detail.chips)),
        fields: model(detail.fields.iter().map(field).collect()),
        lines: model(lines),
        lines_preview: model(preview),
        lines_note: if hidden > 0 {
            format!("{hidden} more lines not shown; Copy takes all").into()
        } else {
            Default::default()
        },
        two_numbers: detail.kind == ToolKind::Edit,
        digits: digits(&detail.lines),
        text: expanded(detail.text.trim_end()).into(),
        text_preview: head(detail.text.trim_end(), PREVIEW_LINES).into(),
        more: more.is_some(),
        more_label: more.unwrap_or_default().into(),
    }
}

/// The detail of a Trace entry: a call shows its arguments (the edit diff,
/// the written file), a result its output; write and edit results only
/// their message, since the call already shows the change.
pub fn entry_detail(kind: EntryKind, tool: &EntryTool, is_error: bool) -> ToolDetail {
    if kind != EntryKind::ToolResult {
        return tool_detail(&tool.name, &tool.arguments, None, false, None);
    }
    let output = tool.output.as_deref().unwrap_or("");
    let mut detail = tool_detail(&tool.name, &tool.arguments, Some(output), is_error, None);
    detail.fields.clear();
    if matches!(detail.kind, ToolKind::Write | ToolKind::Edit) {
        detail.lines.clear();
        detail.lines_total = 0;
        detail.text = output.to_string();
    }
    detail
}

#[cfg(test)]
#[path = "tool_rows_tests.rs"]
mod tests;
