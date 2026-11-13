//! `/undo [force]` in the TUI (PLAN.md D69 "Views"): the text never reaches
//! the model; `Session::undo_last_turn` reads and writes files, so it runs
//! on a thread and its outcome comes back as `Msg::Undone`, shown with the
//! same notes a replay of the session log shows. The hint bar names the
//! turn the next undo restores.

use fast_harness_core::tools::{UndoInfo, UndoOutcome};
use fast_harness_core::view::NoteKind;

use super::app::{Msg, Tui};

/// The note of `/undo` typed while a turn runs (an undo waits for no turn).
pub const RUNNING: &str =
    "nothing was undone: a turn is running; send /undo after it ends (it is never sent to the model)";

impl Tui {
    /// `/undo` or `/undo force`: runs the undo; false for other text.
    pub(super) fn undo_command(&mut self, text: &str) -> bool {
        let Some(force) = fast_harness_core::app::undo_args(text) else {
            return false;
        };
        if self.current.running() || self.ui.loading {
            self.note(NoteKind::Error, RUNNING);
        } else {
            self.undo(force, false);
        }
        true
    }

    /// Undoes the shown session's last turn on a thread; `queued` goes on
    /// with the queue after it.
    pub(super) fn undo(&mut self, force: bool, queued: bool) {
        let (session, tx, id) = (self.current.clone(), self.tx.clone(), self.session.clone());
        let spawned = std::thread::Builder::new()
            .name("fast-harness-undo".into())
            .spawn(move || {
                let result = session
                    .undo_last_turn(force)
                    .map_err(|err| format!("{err:?}"));
                let _ = tx.send(Msg::Undone(id, result, queued));
            });
        self.ui.status = match spawned {
            Ok(_) => "undoing the last turn's file changes".into(),
            Err(_) => "cannot start the undo thread".into(),
        };
    }

    /// The outcome of an undo: its notes in the transcript of its session
    /// (the log keeps it for a session shown later), the hint, the queue.
    pub(super) fn undone(&mut self, id: &str, result: Result<UndoOutcome, String>, queued: bool) {
        if self.ui.status.starts_with("undoing") {
            self.ui.status.clear();
        }
        if id != self.session {
            return;
        }
        match result {
            Ok(outcome) => self.transcript.undone(&outcome),
            Err(err) => self.note(NoteKind::Error, format!("nothing was undone: {err}")),
        }
        self.transcript.revision += 1;
        self.ui.undo = self.current.undo_info();
        if queued && !self.ui.running {
            self.next_queued();
        }
    }
}

/// The hint bar's words for the next undo: `/undo turn 3 (2 files)`, or
/// why it cannot be undone.
pub fn hint(info: Option<&UndoInfo>) -> Option<String> {
    let info = info?;
    let files = match info.files {
        1 => "1 file".to_string(),
        n => format!("{n} files"),
    };
    Some(match &info.not_undoable {
        None => format!("/undo turn {} ({files})", info.turn),
        Some(_) => format!("turn {} ({files}) cannot be undone", info.turn),
    })
}
