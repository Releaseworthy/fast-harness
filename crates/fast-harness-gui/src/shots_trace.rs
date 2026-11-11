//! Sample Trace page data for the screenshots: a turn with a long system
//! prompt, thinking, a long tool output, a failed tool, a retry and a
//! switch, mapped through the same functions the page uses.

use fast_harness_core::trace::detail::{Entry, EntryKind};
use fast_harness_core::trace::report::StepRow;
use fast_harness_core::types::Usage;
use slint::{ModelRc, SharedString, VecModel};

use crate::trace_rows::{entry_row, step_cells};
use crate::TraceRow;

fn entry(kind: EntryKind, title: &str, body: &str, is_error: bool) -> Entry {
    Entry {
        kind,
        turn: 1,
        step: 1,
        title: title.into(),
        body: body.into(),
        is_error,
    }
}

fn long_output() -> String {
    (1..=14)
        .map(|i| {
            format!(
                "crates/fast-harness-gui/src/file_{i}.rs:{}: fn render_{i}()",
                i * 7
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn entries() -> Vec<TraceRow> {
    use EntryKind::*;
    let system = "You are fast-harness, a coding agent.\nUse the tools to read and change files.";
    let list = [
        entry(Turn, "Turn 1", "", false),
        entry(User, "User", "Why does the trace page not show what I sent?", false),
        entry(Header, "Request header · system prompt 9 214 chars · 6 tools: read, write, edit, bash, grep, glob", system, false),
        entry(Request, "Step 1 request · claude-opus-5-5 · effort high · account work@example.com · attempt 1", "", false),
        entry(Thinking, "Thinking", "The page reads trace::report only. The detail module already has the user message and the blocks; I should map those entries.", false),
        entry(Response, "Response", "I will read the page and the detail module first.", false),
        entry(ToolCall, "Tool call grep", "{\n  \"pattern\": \"fn render\",\n  \"path\": \"crates/fast-harness-gui\"\n}", false),
        entry(Answer, "Step 1 answer · stop tool_use · TTFB 850 ms · total 3200 ms · in 1200 · out 300 · cache read 80000 · cache write 4000", "", false),
        entry(ToolResult, "Tool result grep · 41 ms", &long_output(), false),
        entry(ToolResult, "Tool error bash · 2210 ms", "error[E0425]: cannot find value `palette` in this scope\n --> crates/fast-harness-gui/src/theme.rs:12:5", true),
        entry(Retry, "Retry 2 in 4000 ms", "Overloaded (529)", false),
        entry(Failure, "Attempt 1 failed after 30000 ms", "connection reset by peer", true),
        entry(Switch, "Account switch work@example.com → home@example.com · 5h limit reached", "", false),
        entry(TurnEnd, "Turn 1 done · 2 steps · 9400 ms · in 2400 · out 610 · cache read 160000 · cache write 4000", "", false),
    ];
    list.iter().map(entry_row).collect()
}

pub fn steps() -> Vec<ModelRc<SharedString>> {
    let row = |step: u64, tools: Vec<(String, u64)>, retries: u64| StepRow {
        turn: 1,
        step,
        model: "claude-opus-5-5".into(),
        account: "work@example.com".into(),
        ttfb_ms: 850 * step,
        total_ms: 3200 + 1000 * step,
        usage: Usage {
            input: 1200,
            output: 300,
            cache_read: 80_000,
            cache_write: 4000,
        },
        tools,
        retries,
    };
    let rows = [
        row(1, vec![("grep".into(), 41)], 0),
        row(2, vec![("bash".into(), 2210), ("read".into(), 5)], 1),
        row(3, Vec::new(), 0),
    ];
    rows.iter()
        .map(|r| ModelRc::new(VecModel::from(step_cells(r))))
        .collect()
}
