//! Opening a session's log on a worker thread (D60, gate G10): the parse of
//! the log and the transcript built from it never run on the UI thread. One
//! open runs at a time; an open asked meanwhile is remembered and the newest
//! starts when the running one ends, so the stale result is dropped. While a
//! session loads the view says so and no turn starts; an open that fails
//! shows its error and keeps the session the window shows.

use std::path::{Path, PathBuf};
use std::thread;

use fast_harness_core::view::opened::{self, Next, Opened};

use crate::app::{show_error, with_app, App};
use crate::runs::Run;
use crate::turn::error_text;
use crate::MainWindow;

/// The word the chat view shows while a session opens.
pub const LOADING: &str = "Loading the session…";

/// The window's opens: the newest ask and the one worker.
pub type Opens = opened::Opening;

/// Asks session `id`'s log opened on a worker; the shown run stays until the
/// result comes.
pub fn open(app: &mut App, ui: &MainWindow, id: &str) {
    ui.set_loading(true);
    if let Some(ask) = app.opens.ask(Some(id)) {
        start(app, ui, ask.id.unwrap_or_default(), app.cwd.clone(), ask.generation);
    }
}

/// Opens the shown session `id` again without the loading word: a running
/// subagent shown read-only follows its log this way (D74).
pub fn reload(app: &mut App, ui: &MainWindow, id: &str) {
    let _ = ui;
    if let Some(ask) = app.opens.ask(Some(id)) {
        start(app, ui, ask.id.unwrap_or_default(), app.cwd.clone(), ask.generation);
    }
}

/// A new session in `cwd`: nothing to parse, and a worker still running for
/// another session is dropped when it comes.
pub fn clear(app: &mut App, ui: &MainWindow) {
    app.opens.cancel();
    ui.set_loading(false);
}

/// Starts the worker of `generation`, which opens `id` in `cwd`.
fn start(app: &mut App, ui: &MainWindow, id: String, cwd: PathBuf, generation: u64) {
    let home = app.rt.home.clone();
    let working = crate::listings::Working::enter();
    let spawned = thread::Builder::new()
        .name("fast-harness-gui-open".into())
        .spawn(move || {
            let result = opened::load(&home, &cwd, &id);
            drop(working);
            let _ = slint::invoke_from_event_loop(move || {
                with_app(|app, ui| arrived(app, ui, generation, &cwd, result));
            });
        });
    if let Err(err) = spawned {
        app.opens.failed();
        ui.set_loading(false);
        crate::drafts::back_to_shown(app, ui);
        show_error(ui, format!("Cannot start the session read: {err}"));
    }
}

/// The worker of `generation` ended (UI thread): its result applies when it
/// is the newest, else the newest open starts instead.
pub fn arrived(
    app: &mut App,
    ui: &MainWindow,
    generation: u64,
    cwd: &Path,
    result: Result<Opened, String>,
) {
    match app.opens.arrive(generation) {
        Next::Apply => shown(app, ui, cwd, result),
        Next::Drop => {}
        Next::Start(newer) => {
            let id = newer.id.unwrap_or_default();
            start(app, ui, id, app.cwd.clone(), newer.generation);
        }
    }
}

/// A session opened (UI thread): its run replaces the shown one, with its
/// session in the service. A failure shows and keeps the shown session.
fn shown(app: &mut App, ui: &MainWindow, cwd: &Path, result: Result<Opened, String>) {
    let Opened { conv, view } = match result {
        Ok(opened) => opened,
        Err(err) => {
            ui.set_loading(false);
            crate::drafts::back_to_shown(app, ui);
            return show_error(ui, err);
        }
    };
    // The session's id, before the conversation goes into the service.
    let session_id = conv.id.clone();
    match app.service.open(conv, cwd) {
        Ok(session) => {
            // The window shows approval requests and answers them (D64).
            session.set_answering(true);
            let run = Run::new(cwd.to_path_buf(), Some(session), view);
            let dropped = app.runs.take(session_id.clone(), run);
            // A reload replaced the same session: nothing to close.
            if let Some(id) = dropped.filter(|id| *id != session_id) {
                app.service.close(&id);
            }
            ui.set_loading(false);
            app.show_run(ui);
        }
        Err(err) => {
            ui.set_loading(false);
            crate::drafts::back_to_shown(app, ui);
            show_error(ui, error_text(err));
        }
    }
}
