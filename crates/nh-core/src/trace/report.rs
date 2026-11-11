//! `nh trace`: one row per step built from the events of a session log.

use crate::llm::Timing;
use crate::trace::events::kind;
use crate::trace::TraceEvent;
use crate::types::Usage;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StepRow {
    pub turn: u64,
    pub step: u64,
    /// Account of the last attempt (or of the answer).
    pub account: String,
    pub model: String,
    /// Of the successful model call.
    pub ttfb_ms: u64,
    /// Total time of the successful model call.
    pub total_ms: u64,
    /// Sum over the step's answers.
    pub usage: Usage,
    /// `(name, duration_ms)` in call order.
    pub tools: Vec<(String, u64)>,
    /// Failed attempts (`assistant/attempt` events).
    pub retries: u64,
}

/// Rows in the order the steps started. Events naming a step that never
/// started open a row of their own; unknown events are ignored.
pub fn step_rows(events: &[TraceEvent]) -> Vec<StepRow> {
    let mut rows: Vec<StepRow> = Vec::new();
    let mut calls: HashMap<String, (usize, usize)> = HashMap::new();
    for event in events {
        let data = &event.data;
        let Some(index) = row_index(&mut rows, data) else {
            continue;
        };
        let row = &mut rows[index];
        match event.kind.as_str() {
            kind::REQUEST_CONTEXT => {
                row.account = text(data, "account");
                row.model = text(data, "model");
            }
            kind::ASSISTANT_ATTEMPT => row.retries += 1,
            kind::ASSISTANT_MESSAGE => add_answer(row, data),
            kind::TOOL_CALL => {
                calls.insert(text(data, "callId"), (index, row.tools.len()));
                row.tools.push((text(data, "name"), 0));
            }
            kind::TOOL_RESULT => set_durations(&mut rows, &calls, data),
            _ => {}
        }
    }
    rows
}

fn row_index(rows: &mut Vec<StepRow>, data: &Value) -> Option<usize> {
    let turn = data.get("turn")?.as_u64()?;
    let step = data.get("step")?.as_u64()?;
    if let Some(found) = rows.iter().rposition(|r| r.turn == turn && r.step == step) {
        return Some(found);
    }
    rows.push(StepRow {
        turn,
        step,
        ..StepRow::default()
    });
    Some(rows.len() - 1)
}

fn text(data: &Value, key: &str) -> String {
    data.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn add_answer(row: &mut StepRow, data: &Value) {
    let usage: Usage = data
        .get("usage")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    row.usage.add(&usage);
    let timing: Timing = data
        .get("timing")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    row.ttfb_ms = timing.ttfb_ms;
    row.total_ms = timing.total_ms;
    let account = text(data, "account");
    if !account.is_empty() {
        row.account = account;
    }
}

/// Every result block of the message gets the event's `durationMs`.
fn set_durations(rows: &mut [StepRow], calls: &HashMap<String, (usize, usize)>, data: &Value) {
    let duration = data.get("durationMs").and_then(Value::as_u64).unwrap_or(0);
    let blocks = data
        .pointer("/message/content")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    for block in blocks {
        let Some(id) = block.get("callId").and_then(Value::as_str) else {
            continue;
        };
        if let Some(&(row, tool)) = calls.get(id) {
            if let Some(entry) = rows.get_mut(row).and_then(|r| r.tools.get_mut(tool)) {
                entry.1 = duration;
            }
        }
    }
}

const HEADERS: [&str; 12] = [
    "turn",
    "step",
    "account",
    "model",
    "ttfb ms",
    "total ms",
    "input",
    "output",
    "cache read",
    "cache write",
    "retries",
    "tools",
];

/// Text columns, left aligned; every other column is a number, right aligned.
const LEFT: [usize; 3] = [2, 3, 11];

/// Aligned columns separated by two spaces, one row per step, then a totals row.
pub fn format_table(rows: &[StepRow]) -> String {
    let mut table: Vec<Vec<String>> = vec![HEADERS.iter().map(|h| h.to_string()).collect()];
    table.extend(rows.iter().map(row_cells));
    table.push(total_cells(rows));
    let widths: Vec<usize> = (0..HEADERS.len())
        .map(|col| {
            table
                .iter()
                .map(|r| r[col].chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut out = String::new();
    for cells in &table {
        let mut line = String::new();
        for (col, cell) in cells.iter().enumerate() {
            if col > 0 {
                line.push_str("  ");
            }
            let width = widths[col];
            if LEFT.contains(&col) {
                line.push_str(&format!("{cell:<width$}"));
            } else {
                line.push_str(&format!("{cell:>width$}"));
            }
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

fn row_cells(row: &StepRow) -> Vec<String> {
    let tools: Vec<String> = row
        .tools
        .iter()
        .map(|(name, ms)| format!("{name} {ms}ms"))
        .collect();
    vec![
        row.turn.to_string(),
        row.step.to_string(),
        row.account.clone(),
        row.model.clone(),
        row.ttfb_ms.to_string(),
        row.total_ms.to_string(),
        row.usage.input.to_string(),
        row.usage.output.to_string(),
        row.usage.cache_read.to_string(),
        row.usage.cache_write.to_string(),
        row.retries.to_string(),
        tools.join(", "),
    ]
}

fn total_cells(rows: &[StepRow]) -> Vec<String> {
    let mut usage = Usage::default();
    rows.iter().for_each(|r| usage.add(&r.usage));
    let total_ms: u64 = rows.iter().map(|r| r.total_ms).sum();
    let retries: u64 = rows.iter().map(|r| r.retries).sum();
    let calls: usize = rows.iter().map(|r| r.tools.len()).sum();
    let tool_ms: u64 = rows.iter().flat_map(|r| &r.tools).map(|t| t.1).sum();
    vec![
        "total".to_string(),
        rows.len().to_string(),
        String::new(),
        String::new(),
        String::new(),
        total_ms.to_string(),
        usage.input.to_string(),
        usage.output.to_string(),
        usage.cache_read.to_string(),
        usage.cache_write.to_string(),
        retries.to_string(),
        format!("{calls} calls {tool_ms}ms"),
    ]
}
