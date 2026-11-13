//! The shown session's background jobs (PLAN.md D68 "Views", D72): a pill
//! above the composer while the session has jobs, and its popover with the
//! running and finished jobs and one job's output. The core's
//! `view::jobs::Watch` worker blocks on the session's jobs and posts each
//! new list through `slint::invoke_from_event_loop`; a switch re-targets it
//! and a list of an earlier session is dropped. The expanded job's output
//! comes from `view::job_tail::TailWatch`, a worker that reads it without
//! moving the model's `job_output` cursor, only while the popover is open.
//! Stop calls the same kill as `job_kill`, on a thread (it waits up to 5 s).

use std::collections::HashSet;
use std::time::Duration;

use fast_harness_core::tools::JobInfo;
use fast_harness_core::view::job_tail::{Tail, TailWatch};
use fast_harness_core::view::jobs::{JobRow, Mark};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::app::{with_app, App};
use crate::{JobChip, JobsUi, MainWindow};

/// The popover's state on the UI thread.
#[derive(Default)]
pub struct Panel {
    /// The shown session's rows, as the last list gave them.
    rows: Vec<JobRow>,
    /// Finished jobs Clear took off the list.
    cleared: HashSet<String>,
    /// The job whose output shows, and the worker reading it.
    expanded: String,
    tail: Option<TailWatch>,
    /// Tells a tail of the current worker from an earlier one's.
    generation: u64,
}

impl Panel {
    /// Ends the output worker (the popover closes, the window closes).
    pub fn stop(&mut self) {
        self.tail = None;
        self.expanded.clear();
        self.generation += 1;
    }

    /// The rows the popover lists: running, then finished not cleared.
    pub fn visible(&self) -> (Vec<JobRow>, Vec<JobRow>) {
        let shown = self.rows.iter().filter(|r| r.running() || !self.cleared.contains(&r.id));
        shown.cloned().partition(JobRow::running)
    }
}

/// `4s`, `2m 05s`, `1h 02m`: how long a finished job ran.
pub fn took_text(took: Option<Duration>) -> String {
    let Some(took) = took else {
        return String::new();
    };
    let secs = took.as_secs();
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m {:02}s", secs / 60, secs % 60),
        _ => format!("{}h {:02}m", secs / 3600, secs % 3600 / 60),
    }
}

/// The popover's model of `rows`: `mark` is the state's name, which picks
/// the glyph and the role; the word says it in text.
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
        when: took_text(row.took).into(),
    };
    rows.iter().map(chip).collect()
}

fn model(rows: &[JobRow]) -> ModelRc<JobChip> {
    ModelRc::new(VecModel::from(chips(rows)))
}

/// Shows the panel's rows; closes the popover when none is left, and hides
/// an output whose job left the list.
fn show(panel: &mut Panel, ui: &MainWindow) {
    let (running, finished) = panel.visible();
    let jobs = ui.global::<JobsUi>();
    jobs.set_running(model(&running));
    jobs.set_finished(model(&finished));
    let listed = running.iter().chain(&finished).any(|r| r.id == panel.expanded);
    if running.is_empty() && finished.is_empty() {
        jobs.set_open(false);
    }
    if !panel.expanded.is_empty() && (!listed || !jobs.get_open()) {
        collapse(panel, ui);
    }
}

fn collapse(panel: &mut Panel, ui: &MainWindow) {
    panel.stop();
    let jobs = ui.global::<JobsUi>();
    jobs.set_expanded("".into());
    jobs.set_output("".into());
    jobs.set_dropped(false);
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
        // Another session: its own list, nothing cleared or expanded.
        app.jobs_panel.cleared.clear();
        collapse(&mut app.jobs_panel, ui);
        app.jobs_panel.rows = rows;
        show(&mut app.jobs_panel, ui);
    }
}

/// A list from the worker: shown unless it is another session's.
pub fn arrived(app: &mut App, ui: &MainWindow, generation: u64, list: &[JobInfo]) {
    if let Some(rows) = app.jobs.arrived(generation, list) {
        app.jobs_panel.rows = rows;
        show(&mut app.jobs_panel, ui);
    }
}

/// Shows job `id`'s output, read by a worker, or hides it when it shows.
pub fn expand(app: &mut App, ui: &MainWindow, id: &str) {
    let open = app.jobs_panel.expanded == id;
    collapse(&mut app.jobs_panel, ui);
    let Some(session) = &app.runs.shown.session else {
        return;
    };
    if open || !ui.global::<JobsUi>().get_open() {
        return;
    }
    let panel = &mut app.jobs_panel;
    let generation = panel.generation;
    let post = move |tail: Tail| {
        let back = move || { with_app(|app, ui| tail_arrived(app, ui, generation, &tail)); };
        slint::invoke_from_event_loop(back).is_ok()
    };
    panel.tail = TailWatch::start(session.jobs(), id, post);
    panel.expanded = id.to_string();
    ui.global::<JobsUi>().set_expanded(id.into());
}

/// A tail from the output worker: shown unless an earlier worker sent it.
pub fn tail_arrived(app: &mut App, ui: &MainWindow, generation: u64, tail: &Tail) {
    if app.jobs_panel.generation != generation || app.jobs_panel.tail.is_none() {
        return;
    }
    let jobs = ui.global::<JobsUi>();
    jobs.set_output(tail.text.as_str().into());
    jobs.set_dropped(tail.dropped);
}

/// Clear: the finished jobs leave the list (they stay in the session).
pub fn clear(app: &mut App, ui: &MainWindow) {
    let panel = &mut app.jobs_panel;
    let finished = panel.rows.iter().filter(|r| !r.running());
    let ids: Vec<String> = finished.map(|r| r.id.clone()).collect();
    panel.cleared.extend(ids);
    show(panel, ui);
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

/// Wires the popover's callbacks.
pub fn connect(ui: &MainWindow) {
    let jobs = ui.global::<JobsUi>();
    jobs.on_kill(|id| { with_app(|app, _| kill(app, &id)); });
    jobs.on_expand(|id| { with_app(|app, ui| expand(app, ui, &id)); });
    jobs.on_clear(|| { with_app(clear); });
    jobs.on_closed(|| { with_app(|app, ui| collapse(&mut app.jobs_panel, ui)); });
}

#[cfg(test)]
#[path = "jobs_view_tests.rs"]
mod tests;
