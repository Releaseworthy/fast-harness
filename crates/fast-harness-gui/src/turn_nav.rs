//! The turn navigator's model (PLAN.md D77): one tick per user turn of the
//! shown transcript, with the prompt's first line and the start of the
//! final reply for the hover card. Built from the view items after each
//! transcript sync and handed to the window only when it changed.

use fast_harness_core::view::Item;
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::{MainWindow, TurnTick, TurnsUi};

/// The card's title: one line, at most this many characters.
pub const PROMPT_CHARS: usize = 90;
/// The card's preview: about three lines of the reply.
pub const PREVIEW_CHARS: usize = 180;
/// A longer session is sampled down to this many ticks.
pub const MAX_TICKS: usize = 60;

/// One user turn: its row and the card's two texts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Turn {
    /// The turn's number, from 1.
    pub number: usize,
    /// The user row (rows and items are one to one, `mirror`).
    pub row: usize,
    pub prompt: String,
    pub preview: String,
}

/// `text` on one line (whitespace runs become one space), cut to `max`
/// characters with an ellipsis.
pub fn elide(text: &str, max: usize) -> String {
    // A long reply grows on every delta: only its start is read.
    let head: String = text.chars().take(max.saturating_mul(4)).collect();
    let flat = head.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let cut: String = flat.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

/// The first line with words, shortened.
fn first_line(text: &str) -> String {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    elide(line, PROMPT_CHARS)
}

/// Every user turn of `items`, with the start of its final reply text.
pub fn turns(items: &[Item]) -> Vec<Turn> {
    let mut out: Vec<Turn> = Vec::new();
    for (row, item) in items.iter().enumerate() {
        match item {
            Item::User { text, .. } => out.push(Turn {
                number: out.len() + 1,
                row,
                prompt: first_line(text),
                preview: String::new(),
            }),
            Item::Assistant { text, .. } if !text.trim().is_empty() => {
                // The last text of the turn wins: the card previews the
                // final reply (D80), not the words before the first tool.
                if let Some(turn) = out.last_mut() {
                    turn.preview = elide(text, PREVIEW_CHARS);
                }
            }
            _ => {}
        }
    }
    out
}

/// At most `max` turns, evenly spread, the first and the last kept.
pub fn sample(turns: Vec<Turn>, max: usize) -> Vec<Turn> {
    let n = turns.len();
    if n <= max || max < 2 {
        return turns;
    }
    let picks: Vec<usize> = (0..max).map(|k| k * (n - 1) / (max - 1)).collect();
    let mut out = Vec::with_capacity(max);
    for (index, turn) in turns.into_iter().enumerate() {
        if picks.binary_search(&index).is_ok() {
            out.push(turn);
        }
    }
    out
}

/// The ticks last handed to the window.
#[derive(Default)]
pub struct Nav {
    shown: Vec<Turn>,
}

impl Nav {
    /// Hands the ticks of `items` to the window when they changed.
    pub fn show(&mut self, ui: &MainWindow, items: &[Item]) {
        let ticks = sample(turns(items), MAX_TICKS);
        if ticks == self.shown {
            return;
        }
        let model: Vec<TurnTick> = ticks.iter().map(tick).collect();
        ui.global::<TurnsUi>()
            .set_turns(ModelRc::new(VecModel::from(model)));
        self.shown = ticks;
    }
}

fn tick(turn: &Turn) -> TurnTick {
    let int = |n: usize| i32::try_from(n).unwrap_or(i32::MAX);
    TurnTick {
        number: int(turn.number),
        row: int(turn.row),
        prompt: turn.prompt.as_str().into(),
        preview: turn.preview.as_str().into(),
    }
}

#[cfg(test)]
#[path = "turn_nav_tests.rs"]
mod tests;
