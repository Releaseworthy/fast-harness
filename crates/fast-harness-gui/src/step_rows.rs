//! Step summaries in the chat (PLAN.md D73): under `chatDetail: "summary"`
//! each step's tool activity folds into one line on its first row; a click
//! opens the step to its full rows. Computed here once per step update.

use std::collections::HashSet;

use fast_harness_core::view::approvals::WAITING;
use fast_harness_core::view::{step_range, summarize, Item};

use crate::MsgRow;

/// The step part of one row; the default changes nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Deco {
    /// The step's line, on its first row only.
    pub summary: String,
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
}

/// Writes `deco` into `row`.
pub fn apply(row: &mut MsgRow, deco: &Deco) {
    row.step_summary = deco.summary.as_str().into();
    row.step_label = deco.label.as_str().into();
    row.step_open = deco.open;
    row.rail = deco.rail;
    row.step_head = i32::try_from(deco.head).unwrap_or(i32::MAX);
    row.step_running = deco.running;
    row.step_failed = deco.failed;
    row.folded = deco.folded;
}

/// How the chat shows the steps.
pub struct Steps<'a> {
    pub items: &'a [Item],
    pub words: &'a [Option<&'static str>],
    /// `chatDetail: "full"`: no lines, nothing folded.
    pub full: bool,
    /// The first rows of the steps the user opened.
    pub open: &'a HashSet<usize>,
}

impl Steps<'_> {
    /// The decorations of rows `start..` (`start` begins a step).
    pub fn from(&self, start: usize) -> Vec<Deco> {
        let mut out = Vec::with_capacity(self.items.len().saturating_sub(start));
        let mut index = start;
        while index < self.items.len() {
            let range = step_range(self.items, index);
            let end = range.end.max(index + 1);
            out.extend(self.step(range.start, end));
            index = end;
        }
        out
    }

    /// The decorations of one step, rows `start..end`.
    pub fn step(&self, start: usize, end: usize) -> Vec<Deco> {
        let mut out = vec![Deco::default(); end - start];
        let (items, words) = (&self.items[start..end], &self.words[start..end]);
        let head =
            (items.iter()).position(|i| matches!(i, Item::Assistant { .. } | Item::Tool { .. }));
        let Some(head) = head.filter(|_| !self.full) else {
            return out;
        };
        let summary = summarize(items.iter().zip(words.iter().copied()));
        let line = summary.line();
        if line.is_empty() {
            return out;
        }
        let open = self.open.contains(&(start + head));
        for deco in out.iter_mut().skip(head) {
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
