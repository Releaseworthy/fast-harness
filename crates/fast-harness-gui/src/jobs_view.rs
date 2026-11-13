//! The shown session's background jobs (PLAN.md D68 "Views"): a strip above
//! the composer while the session has jobs. The core's `view::jobs::Watch`
//! worker blocks on the session's jobs and posts each new list through
//! `slint::invoke_from_event_loop`; a switch re-targets it and a list of an
//! earlier session is dropped. Stop on a running job calls the same kill as
//! `job_kill`, on a thread (it waits up to 5 s for the exit).

use fast_harness_core::tools::JobInfo;
use fast_harness_core::view::jobs::{JobRow, Mark};
use slint::{ModelRc, VecModel};

use crate::app::{with_app, App};
use crate::{JobChip, MainWindow};

/// The strip's model of `rows`: `mark` is the state's name, which picks the
/// glyph and the role; the word says it in text.
pub fn chips(rows: &[JobRow]) -> Vec<JobChip> {
    let chip = |row: &JobRow| JobChip {
        id: row.id.as_str().into(),
        command: row.command.as_str().into(),
        word: row.word.as_str().into(),
        mark: match row.mark {
            Mark::Running => "running",
            Mark::Exited => "exited",
            Mark::Failed => "failed",
            Mark::Killed => "killed",
        }
        .into(),
    };
    rows.iter().map(chip).collect()
}

fn show(ui: &MainWindow, rows: &[JobRow]) {
    ui.set_jobs(ModelRc::new(VecModel::from(chips(rows))));
}

/// Watches the shown session's jobs: none until the session is opened in
/// the service (a session gets jobs only through a turn of this window).
pub fn follow(app: &mut App, ui: &MainWindow) {
    let jobs = app.runs.shown.session.as_ref().map(|s| s.jobs());
    let post = |generation, list: Vec<JobInfo>| {
        let back = move || {
            with_app(|app, ui| arrived(app, ui, generation, &list));
        };
        slint::invoke_from_event_loop(back).is_ok()
    };
    if let Some(rows) = app.jobs.target(jobs, post) {
        show(ui, &rows);
    }
}

/// A list from the worker: shown unless it is another session's.
pub fn arrived(app: &mut App, ui: &MainWindow, generation: u64, list: &[JobInfo]) {
    if let Some(rows) = app.jobs.arrived(generation, list) {
        show(ui, &rows);
    }
}

/// Stop on job `id` of the shown session.
pub fn kill(app: &App, id: &str) {
    let Some(session) = &app.runs.shown.session else {
        return;
    };
    let (jobs, id) = (session.jobs(), id.to_string());
    let spawned = std::thread::Builder::new()
        .name("fast-harness-gui-kill".into())
        .spawn(move || drop(jobs.kill(&id)));
    drop(spawned);
}

/// Wires Stop of the strip.
pub fn connect(ui: &MainWindow) {
    ui.on_kill_job(|id| {
        with_app(|app, _| kill(app, &id));
    });
}

#[cfg(test)]
#[path = "jobs_view_tests.rs"]
mod tests;
