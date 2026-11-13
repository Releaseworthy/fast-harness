//! The transcript notes of an undo (PLAN.md D69 "Views"): a front end shows
//! the outcome of `Session::undo_last_turn` with [`Transcript::undone`], and
//! a replay shows the trace's `undo` record with [`record_notes`]; an undo
//! that changed files reads the same both ways.

use std::path::PathBuf;

use super::{NoteKind, Transcript};
use crate::tools::undo::UNTRACKED;
use crate::tools::UndoOutcome;
use crate::trace::record::Undo;

/// The trace record of an outcome that changed files; `None` when the undo
/// was refused (nothing changed, nothing logged).
pub fn record(outcome: &UndoOutcome) -> Option<Undo> {
    let turn = outcome.turn.filter(|_| outcome.refused.is_none())?;
    let names = |paths: &[PathBuf]| paths.iter().map(|p| p.display().to_string()).collect();
    Some(Undo {
        turn,
        restored: names(&outcome.restored),
        removed: names(&outcome.removed),
        forced: outcome.forced,
    })
}

/// The line of an `undo` record: `undid the file changes of turn N: ...`.
pub fn record_text(undo: &Undo) -> String {
    let mut text = format!("undid the file changes of turn {}", undo.turn);
    if !undo.restored.is_empty() {
        text.push_str(&format!(": restored {}", undo.restored.join(", ")));
    }
    if !undo.removed.is_empty() {
        text.push_str(&format!("; removed {}", undo.removed.join(", ")));
    }
    if undo.forced {
        text.push_str(" (forced over later changes)");
    }
    text
}

/// What a replay shows for an `undo` record: its line and the note on the
/// changes an undo cannot see.
pub fn record_notes(undo: &Undo) -> Vec<(NoteKind, String)> {
    vec![
        (NoteKind::Info, record_text(undo)),
        (NoteKind::Info, UNTRACKED.to_string()),
    ]
}

/// The line of a refused undo; a conflict names the files and the way on.
pub fn refused_text(outcome: &UndoOutcome) -> Option<String> {
    let reason = outcome.refused.as_ref()?;
    let mut text = format!("nothing was undone: {reason}");
    if !outcome.conflicts.is_empty() {
        let names: Vec<String> = (outcome.conflicts.iter())
            .map(|p| p.display().to_string())
            .collect();
        text.push_str(&format!(
            "; changed since that turn: {} (/undo force or Undo anyway overwrites them)",
            names.join(", ")
        ));
    }
    Some(text)
}

/// The notes of an outcome as a front end shows it at once: a refusal
/// (with its conflicts) as an error, else what a replay shows, plus the
/// files that could not be put back.
pub fn notes(outcome: &UndoOutcome) -> Vec<(NoteKind, String)> {
    let mut notes = match (refused_text(outcome), record(outcome)) {
        (Some(text), _) => vec![(NoteKind::Error, text)],
        (None, Some(undo)) => {
            let mut notes = record_notes(&undo);
            notes.pop();
            notes
        }
        (None, None) => Vec::new(),
    };
    for error in &outcome.errors {
        notes.push((NoteKind::Error, format!("not put back: {error}")));
    }
    notes.push((NoteKind::Info, outcome.untracked.clone()));
    notes
}

impl Transcript {
    /// Shows the outcome of an undo the user asked for.
    pub fn undone(&mut self, outcome: &UndoOutcome) {
        for (kind, text) in notes(outcome) {
            self.note(kind, text);
        }
    }
}

#[cfg(test)]
#[path = "undo_tests.rs"]
mod tests;
