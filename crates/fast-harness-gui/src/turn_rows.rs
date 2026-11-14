//! Turn rails in the chat (PLAN.md D80): every row after a user message,
//! up to the next one, belongs to that turn and draws the turn's faint rail
//! on its left; a click on the rail folds the whole turn into one line on
//! its first row ("3 steps · <the answer's first line>"), a click on that
//! line opens it again. Turns are open by default, also under
//! `chatDetail: "full"`; a turn whose call waits for an approval never folds.

use std::collections::HashSet;

use fast_harness_core::view::approvals::WAITING;
use fast_harness_core::view::Item;

use crate::step_rows::Deco;
use crate::turn_nav::elide;

/// The answer part of a folded turn's line.
const ANSWER_CHARS: usize = 120;

/// The turn of the row before `start`: its first row (after the user
/// message) and its number (1-based), or `None` before the first message.
fn turn_before(items: &[Item], start: usize) -> Option<(usize, usize)> {
    let user = items[..start.min(items.len())]
        .iter()
        .rposition(|i| matches!(i, Item::User { .. }))?;
    let number = items[..=user]
        .iter()
        .filter(|i| matches!(i, Item::User { .. }))
        .count();
    Some((user + 1, number))
}

/// The first row of the turn holding row `index` (`index` itself after the
/// last user message before it), or `None`.
pub fn head_of(items: &[Item], index: usize) -> Option<usize> {
    turn_before(items, index + 1)
        .map(|(head, _)| head)
        .filter(|&head| head <= index)
}

/// The end (exclusive) of the turn starting at row `head`.
fn end_of(items: &[Item], head: usize) -> usize {
    (items[head..].iter())
        .position(|i| matches!(i, Item::User { .. }))
        .map_or(items.len(), |n| head + n)
}

/// The folded turn's line: its steps and the start of its last answer.
pub fn line(items: &[Item]) -> String {
    let steps = (items.iter())
        .filter(|i| matches!(i, Item::StepFooter { .. }))
        .count()
        .max(1);
    let plural = if steps == 1 { "" } else { "s" };
    let answer = items.iter().rev().find_map(|i| match i {
        Item::Assistant { text, .. } => text.lines().find(|l| !l.trim().is_empty()),
        _ => None,
    });
    match answer {
        Some(text) => format!("{steps} step{plural} · {}", elide(text.trim(), ANSWER_CHARS)),
        None => format!("{steps} step{plural}"),
    }
}

/// Writes the turn part of `decos` (rows `start..start + decos.len()`).
pub fn decorate(
    items: &[Item],
    words: &[Option<&'static str>],
    start: usize,
    decos: &mut [Deco],
    closed: &HashSet<usize>,
) {
    let mut turn = turn_before(items, start);
    let (mut folded, mut end) = (false, 0);
    if let Some((head, _)) = turn {
        folded = is_folded(items, words, head, closed);
        end = end_of(items, head);
    }
    for (offset, deco) in decos.iter_mut().enumerate() {
        let index = start + offset;
        if matches!(items.get(index), Some(Item::User { .. })) {
            let number = turn.map_or(1, |(_, n)| n + 1);
            turn = Some((index + 1, number));
            folded = is_folded(items, words, index + 1, closed);
            end = end_of(items, index + 1);
            continue;
        }
        let Some((head, number)) = turn else { continue };
        (deco.turn_head, deco.turn_no) = (head, number);
        deco.turn_rail = !folded;
        deco.turn_hidden = folded;
        // A folded turn's line sits on its last row, so the rows drawn
        // empty come before it: zero-height rows at the end of the list
        // left the frame blank under the line (Slint's list stops laying
        // out at the view's end and estimates the rest, 2026-11-14).
        if folded && index + 1 == end {
            deco.turn_line = line(&items[head..end_of(items, head)]);
        }
    }
}

/// Whether the turn starting at `head` shows folded: closed by the user
/// and no call in it waits for an approval.
fn is_folded(
    items: &[Item],
    words: &[Option<&'static str>],
    head: usize,
    closed: &HashSet<usize>,
) -> bool {
    if !closed.contains(&head) || head >= items.len() {
        return false;
    }
    let end = end_of(items, head).min(words.len());
    !words[head.min(end)..end].contains(&Some(WAITING))
}

#[cfg(test)]
#[path = "turn_rows_tests.rs"]
mod tests;
