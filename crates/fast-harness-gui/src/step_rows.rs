//! Step summaries in the chat (PLAN.md D73): under `chatDetail: "summary"`
//! (the default) each run of tool calls and thinking between two texts
//! folds into one line on its first row ("Searched the code, read files,
//! ran commands, etc."); a click opens the run to its full rows with a rail
//! in the left gutter. The texts show at full width in order.

use std::collections::HashSet;

use fast_harness_core::view::approvals::WAITING;
use fast_harness_core::view::{summarize, Item};

use crate::MsgRow;

/// The step part of one row; the default changes nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Deco {
    /// The step's line, on its first row only.
    pub summary: String,
    /// The run's first verb (`StepSummary::kind`): the line's icon.
    pub kind: &'static str,
    /// The line with counts, for the accessible label and the tooltip.
    pub label: String,
    pub open: bool,
    /// An opened step's rows draw the faint rail that folds it again.
    pub rail: bool,
    /// The row of the step's line (for the rail's click).
    pub head: usize,
    pub running: bool,
    pub failed: bool,
    /// The row's detail is folded into the line (an assistant row keeps
    /// its text and folds its thinking).
    pub folded: bool,
    /// The row belongs to an open turn and draws the turn's rail (D80).
    pub turn_rail: bool,
    /// The turn's first row (after the user message) and its number.
    pub turn_head: usize,
    pub turn_no: usize,
    /// The folded turn's line, on its first row only.
    pub turn_line: String,
    /// The row's turn is folded: only its first row draws, as the line.
    pub turn_hidden: bool,
}

/// Writes `deco` into `row`.
pub fn apply(row: &mut MsgRow, deco: &Deco) {
    row.step_summary = deco.summary.as_str().into();
    row.step_label = deco.label.as_str().into();
    row.step_kind = deco.kind.into();
    row.step_open = deco.open;
    row.rail = deco.rail;
    row.step_head = i32::try_from(deco.head).unwrap_or(i32::MAX);
    row.step_running = deco.running;
    row.step_failed = deco.failed;
    row.folded = deco.folded;
    row.turn_rail = deco.turn_rail;
    row.turn_head = i32::try_from(deco.turn_head).unwrap_or(i32::MAX);
    row.turn_no = i32::try_from(deco.turn_no).unwrap_or(i32::MAX);
    row.turn_line = deco.turn_line.as_str().into();
    row.turn_hidden = deco.turn_hidden;
}

/// How the chat shows the steps.
pub struct Steps<'a> {
    pub items: &'a [Item],
    pub words: &'a [Option<&'static str>],
    /// `chatDetail: "full"`: no lines, nothing folded.
    pub full: bool,
    /// The first rows of the steps the user opened.
    pub open: &'a HashSet<usize>,
    /// The first rows of the turns the user folded (D80).
    pub closed: &'a HashSet<usize>,
}

/// The verbs a run's line names before "etc." (the owner's screenshot).
const VERBS: usize = 3;

/// The end (exclusive, past `start`) of the group starting at `start`: a
/// run of tool calls, thinking-only assistant rows and footers, plus the
/// next text's row when it thought first (its thinking joins the run, its
/// text stays). A user message, a note or a text row is a group alone.
pub fn run_end(items: &[Item], start: usize) -> usize {
    let member = |i: &Item| match i {
        Item::Tool { .. } | Item::StepFooter { .. } => true,
        Item::Assistant { text, .. } => text.trim().is_empty(),
        _ => false,
    };
    let mut end = start;
    while end < items.len() && member(&items[end]) {
        end += 1;
    }
    if let Some(Item::Assistant { thinking, .. }) = items.get(end) {
        if !thinking.trim().is_empty() {
            return end + 1;
        }
    }
    end.max(start + 1)
}

impl Steps<'_> {
    /// The decorations of rows `start..` (`start` begins a group: row 0,
    /// a user message or a turn's first row).
    pub fn from(&self, start: usize) -> Vec<Deco> {
        let mut out = Vec::with_capacity(self.items.len().saturating_sub(start));
        let mut index = start;
        while index < self.items.len() {
            let end = run_end(self.items, index);
            out.extend(self.run(index, end));
            index = end;
        }
        crate::turn_rows::decorate(self.items, self.words, start, &mut out, self.closed);
        out
    }

    /// One group, rows `start..end`: folded into one line by default (the
    /// owner, 2026-11-14), its rows with the rail while open.
    fn run(&self, start: usize, end: usize) -> Vec<Deco> {
        let mut out = vec![Deco::default(); end - start];
        let (items, words) = (&self.items[start..end], &self.words[start..end]);
        if self.full {
            return out;
        }
        // The line sits on the run's first row (a footer of the step before
        // keeps its usage under the line while the run is open).
        let head = 0;
        let summary = summarize(items.iter().zip(words.iter().copied()));
        let line = summary.brief_line(VERBS);
        if line.is_empty() {
            return out;
        }
        let open = self.open.contains(&(start + head));
        // The text row that ends the run keeps its text and draws no rail.
        let tail = matches!(items.last(), Some(Item::Assistant { text, .. }) if !text.trim().is_empty());
        let railed = if tail && items.len() - 1 > head { items.len() - 1 } else { items.len() };
        for deco in out[head..railed].iter_mut() {
            (deco.rail, deco.head) = (open, start + head);
        }
        if !open {
            for ((deco, item), word) in out.iter_mut().zip(items).zip(words) {
                deco.folded = match item {
                    Item::Tool { .. } => *word != Some(WAITING),
                    Item::Assistant { .. } | Item::StepFooter { .. } => true,
                    _ => false,
                };
            }
        }
        out[head] = Deco {
            summary: line,
            label: summary.counted_line(),
            kind: summary.kind,
            open,
            running: summary.running,
            failed: summary.failed > 0,
            ..out[head].clone()
        };
        out
    }
}
#[cfg(test)]
#[path = "step_rows_tests.rs"]
mod tests;
