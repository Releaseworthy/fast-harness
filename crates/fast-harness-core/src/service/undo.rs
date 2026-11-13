//! Undo of the last turn's file changes (PLAN.md D69): the session method
//! the views call, `/undo [force]`, and the JSON of `_fh/undo`.

use std::path::PathBuf;

use serde_json::{json, Value};

use super::{busy, ServiceError, Session};
use crate::agent::Conversation;
use crate::tools::{UndoInfo, UndoOutcome};

/// `Some(force)` for `/undo` and `/undo force`; `None` for any other text.
pub fn command(text: &str) -> Option<bool> {
    let rest = text.trim().strip_prefix("/undo")?;
    match rest.trim() {
        "" if rest.is_empty() || rest.starts_with(char::is_whitespace) => Some(false),
        "force" if rest.starts_with(char::is_whitespace) => Some(true),
        _ => None,
    }
}

/// Undoes the newest turn of `conv` with file changes not yet undone and
/// logs an `undo` record when files changed. Call it only between turns.
pub fn undo_conv(conv: &mut Conversation, force: bool) -> Result<UndoOutcome, String> {
    let outcome = conv.tool_session.undo().undo_last(force);
    // The record a replay shows, the same as the live note (view::undo).
    if let Some(data) = crate::view::undo::record(&outcome) {
        conv.trace
            .undo(data)
            .map_err(|err| format!("session log: {err}"))?;
    }
    Ok(outcome)
}

impl Session {
    /// Undoes the last turn's `write` and `edit` changes (D69); refused
    /// while a turn runs.
    pub fn undo_last_turn(&self, force: bool) -> Result<UndoOutcome, ServiceError> {
        let mut state = self.lock();
        let conv = state.conv.as_mut().ok_or_else(|| busy(&self.id))?;
        undo_conv(conv, force).map_err(ServiceError::Failed)
    }

    /// The turn the next undo would restore, with its file count.
    pub fn undo_info(&self) -> Option<UndoInfo> {
        self.undo.last()
    }
}

/// The result of `_fh/undo`.
pub fn outcome_json(outcome: &UndoOutcome) -> Value {
    let paths = |paths: &[PathBuf]| -> Vec<String> {
        paths.iter().map(|p| p.display().to_string()).collect()
    };
    json!({
        "turn": outcome.turn,
        "restored": paths(&outcome.restored),
        "removed": paths(&outcome.removed),
        "conflicts": paths(&outcome.conflicts),
        "refused": outcome.refused,
        "errors": outcome.errors,
        "forced": outcome.forced,
        "untracked": outcome.untracked,
        "text": outcome.text(),
    })
}

/// The result of `_fh/undo_info`: `null` when there is nothing to undo.
pub fn info_json(info: Option<UndoInfo>) -> Value {
    match info {
        Some(info) => json!({
            "turn": info.turn,
            "files": info.files,
            "notUndoable": info.not_undoable,
        }),
        None => Value::Null,
    }
}
