//! The shown session's background jobs (PLAN.md D68 "Views"): one line
//! above the input while the session has jobs, fed by the core's
//! `view::jobs::Watch` worker through `Msg::Jobs`, and `/kill <job-id>`.

use fast_harness_core::tools::JobInfo;
use fast_harness_core::view::jobs::{JobRow, Mark};
use fast_harness_core::view::NoteKind;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::app::{Msg, Tui};
use super::theme::Styles;

impl Tui {
    /// Watches the shown session's jobs (after every switch, `shown`).
    pub(super) fn follow_jobs(&mut self) {
        let tx = self.tx.clone();
        let post = move |generation, list| tx.send(Msg::Jobs(generation, list)).is_ok();
        if let Some(rows) = self.jobs.target(Some(self.current.jobs()), post) {
            self.ui.jobs = rows;
        }
    }

    /// A list from the worker: shown unless it is another session's.
    pub(super) fn jobs_arrived(&mut self, generation: u64, list: Vec<JobInfo>) {
        if let Some(rows) = self.jobs.arrived(generation, &list) {
            self.ui.jobs = rows;
        }
    }

    /// `/kill <job-id>`: the same kill as `job_kill`, on a thread (it waits
    /// up to 5 s for the exit); the strip shows the end. False for other
    /// text.
    pub(super) fn kill_command(&mut self, text: &str) -> bool {
        let Some(id) = text.trim().strip_prefix("/kill") else {
            return false;
        };
        let id = id.trim().to_string();
        let jobs = self.current.jobs();
        let found = jobs.list().into_iter().find(|job| job.id == id);
        match found {
            None => self.note(NoteKind::Error, format!("no background job `{id}` here")),
            Some(_) => {
                let spawned = std::thread::Builder::new()
                    .name("fast-harness-kill".into())
                    .spawn(move || drop(jobs.kill(&id)));
                self.ui.status = match spawned {
                    Ok(_) => "killing the job and its process tree".into(),
                    Err(_) => "cannot start the kill thread".into(),
                };
            }
        }
        true
    }
}

/// The mark and the style of a row's state (the word says it too).
fn mark(row: &JobRow, st: &Styles) -> (&'static str, Style) {
    match row.mark {
        Mark::Running => (st.icons.pending, st.accent()),
        Mark::Exited => (st.icons.ok, st.success()),
        Mark::Failed => (st.icons.error, st.error()),
        Mark::Killed => (st.icons.not_run, st.warning()),
    }
}

/// The strip: `jobs ○ job-1 ping -t localhost running  ✓ job-2 … exited 0`;
/// `None` when the session has no jobs.
pub fn line(rows: &[JobRow], st: &Styles) -> Option<Line<'static>> {
    if rows.is_empty() {
        return None;
    }
    let mut spans = vec![Span::styled("jobs", st.dim())];
    for row in rows {
        let (glyph, style) = mark(row, st);
        spans.push(Span::styled(format!("  {glyph} "), style));
        let id = st.base().add_modifier(Modifier::BOLD);
        spans.push(Span::styled(row.id.clone(), id));
        spans.push(Span::styled(format!(" {} ", row.command), st.dim()));
        spans.push(Span::styled(row.word.clone(), style));
    }
    Some(Line::from(spans))
}
