//! The Trace page's log, read on a worker thread (D60, chunk 26B). It is
//! asked only when the page opens, on its Refresh, and after a turn of the
//! shown session while the page is shown. One read runs at a time; a request
//! made meanwhile starts when it ends. A result is dropped when the page is
//! no longer shown, the shown session changed or a newer read was asked.
//! The rows shown stay until the new ones arrive; another session's rows are
//! cleared at once with a "Loading" line.

use std::thread;

use fast_harness_core::session;

use crate::app::{with_app, App};
use crate::trace_rows::{self, Loaded};
use crate::MainWindow;

/// What the page asked and shows.
#[derive(Default)]
pub struct TraceLoad {
    /// The newest request.
    ticket: u64,
    /// A worker runs.
    running: bool,
    /// The session whose rows the page shows.
    shown: Option<String>,
}

pub const LOADING: &str = "Loading the session log…";

/// Asks a read of the shown session's log.
pub fn refresh(app: &mut App, ui: &MainWindow) {
    let Some(id) = app.runs.id.clone() else {
        app.trace_load.shown = None;
        return trace_rows::no_session(ui);
    };
    ui.set_trace_session(id.as_str().into());
    if app.trace_load.shown.as_deref() != Some(id.as_str()) {
        // Another session's rows must not stand under this one's name.
        app.trace_load.shown = None;
        ui.set_trace_message(LOADING.into());
        trace_rows::set_rows(ui, Vec::new(), Vec::new());
    }
    app.trace_load.ticket += 1;
    if !app.trace_load.running {
        start(app, ui, id, app.trace_load.ticket);
    }
}

fn start(app: &mut App, ui: &MainWindow, id: String, ticket: u64) {
    let path = session::log_path(&app.rt.home, &app.cwd, &id);
    let spawned = thread::Builder::new()
        .name("fast-harness-gui-trace".into())
        .spawn(move || {
            let result = trace_rows::load(&path);
            let _ = slint::invoke_from_event_loop(move || {
                with_app(|app, ui| loaded(app, ui, &id, ticket, result));
            });
        });
    match spawned {
        Ok(_) => app.trace_load.running = true,
        Err(err) => ui.set_trace_message(format!("Cannot start the trace read: {err}").into()),
    }
}

/// A read of session `id` ended (UI thread). True when it was shown.
pub fn loaded(
    app: &mut App,
    ui: &MainWindow,
    id: &str,
    ticket: u64,
    result: Result<Loaded, String>,
) -> bool {
    app.trace_load.running = false;
    let on_page = ui.get_page() == "trace";
    let current = app.runs.id.clone();
    if !on_page || current.is_none() {
        return false;
    }
    if ticket != app.trace_load.ticket || current.as_deref() != Some(id) {
        // A newer request (or another session) waits for this read.
        let (latest, id) = (app.trace_load.ticket, current.unwrap_or_default());
        start(app, ui, id, latest);
        return false;
    }
    app.trace_load.shown = Some(id.to_string());
    trace_rows::show(ui, result);
    true
}
