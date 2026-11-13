//! Undo of the last turn's file changes in the window (PLAN.md D69
//! "Views"): a row above the composer while the shown session has a turn to
//! undo, read from `Session::undo_info` at every `refresh_run` (turn end,
//! switch, after an undo; no polling); "Undo last turn" and `/undo` run
//! `Session::undo_last_turn` on a thread (it reads and writes files) and
//! the outcome lands in that session's transcript as the notes a replay
//! shows. A conflict offers "Undo anyway" (force) as a second click.

use std::path::PathBuf;
use std::sync::mpsc::Sender;

use fast_harness_core::tools::{UndoInfo, UndoOutcome};
use fast_harness_core::view::{Item, NoteKind};

use crate::app::{show_error, with_app, App};
use crate::{MainWindow, UndoBar};

/// `/undo` sent while a turn runs: it is never a steer.
pub const RUNNING: &str =
    "Nothing was undone: a turn is running. Undo after it ends (/undo is never sent to the model).";

/// The undo state of the window: one undo at a time, and the conflict of
/// the last refused one.
#[derive(Default)]
pub struct UndoState {
    pub running: bool,
    pub conflict: Option<Conflict>,
    /// Where the workers' outcomes go instead of the event loop (tests).
    pub results: Option<Sender<Undone>>,
}

/// An undo's session id, outcome and whether the queue goes on after it.
pub type Undone = (String, Result<UndoOutcome, String>, bool);

/// Files changed since turn `turn` of `session`, which refused an undo.
pub struct Conflict {
    pub session: String,
    pub turn: u32,
    pub files: String,
}

/// The row of `info`; `conflict` shows when it is of the same turn.
pub fn bar(info: Option<&UndoInfo>, conflict: Option<&Conflict>) -> UndoBar {
    let Some(info) = info else {
        return UndoBar::default();
    };
    let files = match info.files {
        1 => "1 file".to_string(),
        n => format!("{n} files"),
    };
    let turn = info.turn;
    let blocked = (info.not_undoable.as_ref())
        .map(|why| format!("Turn {turn} ({files}) cannot be undone: {why}"));
    let conflict = (conflict.filter(|c| c.turn == turn))
        .map(|c| format!("Conflict: changed since turn {turn}: {}", c.files));
    UndoBar {
        turn: i32::try_from(turn).unwrap_or(i32::MAX),
        summary: format!("Turn {turn} changed {files} with write or edit").into(),
        blocked: blocked.unwrap_or_default().into(),
        conflict: conflict.unwrap_or_default().into(),
    }
}

/// Shows the shown session's row (hidden while an undo runs).
pub fn show(app: &App, ui: &MainWindow) {
    let session = app.runs.shown.session.as_ref();
    let info = session.and_then(|s| s.undo_info()).filter(|_| !app.undo.running);
    let conflict = (app.undo.conflict.as_ref())
        .filter(|c| session.is_some_and(|s| s.id == c.session));
    ui.set_undo(bar(info.as_ref(), conflict));
}

/// `/undo [force]` or a click: the shown session's last turn.
pub fn typed(app: &mut App, ui: &MainWindow, force: bool) {
    let Some(id) = app.runs.shown.session.as_ref().map(|s| s.id.clone()) else {
        return show_error(ui, "Nothing was undone: no turn of this session changed files.");
    };
    if app.runs.shown.busy {
        return show_error(ui, RUNNING);
    }
    start(app, ui, &id, force, false);
}

/// Undoes session `id`'s last turn on a thread; `queued` goes on with its
/// queue after it.
pub fn start(app: &mut App, ui: &MainWindow, id: &str, force: bool, queued: bool) {
    let Some(session) = app.runs.get_mut(id).and_then(|run| run.session.clone()) else {
        return;
    };
    if app.undo.running {
        return show_error(ui, "An undo is running.");
    }
    let (tag, results) = (id.to_string(), app.undo.results.clone());
    let spawned = std::thread::Builder::new()
        .name("fast-harness-gui-undo".into())
        .spawn(move || {
            let result = session.undo_last_turn(force).map_err(crate::turn::error_text);
            match results {
                Some(results) => drop(results.send((tag, result, queued))),
                None => post((tag, result, queued)),
            }
        });
    match spawned {
        Ok(_) => app.undo.running = true,
        Err(_) => show_error(ui, "Cannot start the undo thread."),
    }
    show(app, ui);
}

/// Hands an outcome to the UI thread; an ended event loop drops it.
fn post((id, result, queued): Undone) {
    let back = move || {
        with_app(|app, ui| finished(app, ui, &id, result, queued));
    };
    let _ = slint::invoke_from_event_loop(back);
}

/// The outcome of session `id`'s undo: its notes, the conflict, the row.
pub fn finished(app: &mut App, ui: &MainWindow, id: &str, result: Result<UndoOutcome, String>, queued: bool) {
    app.undo.running = false;
    app.undo.conflict = result.as_ref().ok().and_then(|o| conflict(id, o));
    if let Some(run) = app.runs.get_mut(id) {
        match &result {
            Ok(outcome) => run.view.undone(outcome),
            Err(err) => run.view.items.push(Item::Note {
                kind: NoteKind::Error,
                text: format!("nothing was undone: {err}"),
            }),
        }
    }
    if app.runs.is_shown(id) {
        app.sync_rows();
        app.refresh_run(ui);
        ui.invoke_to_bottom();
    }
    if queued {
        crate::turn::next_after_undo(app, ui, id);
    }
}

fn conflict(id: &str, outcome: &UndoOutcome) -> Option<Conflict> {
    let turn = outcome.turn.filter(|_| outcome.refused.is_some())?;
    if outcome.conflicts.is_empty() {
        return None;
    }
    let names: Vec<String> = (outcome.conflicts.iter()).map(PathBuf::as_path).map(|p| p.display().to_string()).collect();
    let files = names.join(", ");
    Some(Conflict { session: id.to_string(), turn, files })
}

/// Wires the row's buttons.
pub fn connect(ui: &MainWindow) {
    ui.on_undo_turn(|force| {
        with_app(|app, ui| typed(app, ui, force));
    });
}

#[cfg(test)]
#[path = "undo_view_tests.rs"]
mod tests;
