//! `/reflect [apply]` in the GUI (PLAN.md D50): the job is prepared on the
//! UI thread and runs on its own; the text comes back in the note line.

use std::thread;

use crate::app::{show_error, with_app, App};
use crate::MainWindow;

pub fn start(app: &mut App, ui: &MainWindow, args: &str) {
    let job = match app.rt.reflect_job(&app.cwd, app.runs.id.as_deref(), args) {
        Ok(job) => job,
        Err(err) => return show_error(ui, err),
    };
    let spawned = thread::Builder::new()
        .name("fast-harness-reflect".into())
        .spawn(move || {
            let text = job().unwrap_or_else(|err| format!("reflect: {err}"));
            let _ = slint::invoke_from_event_loop(move || {
                with_app(|_, ui| ui.set_error_text(text.into()));
            });
        });
    match spawned {
        Ok(_) => ui.set_error_text("reflect is running".into()),
        Err(err) => show_error(ui, format!("reflect: {err}")),
    }
}
