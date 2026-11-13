//! The background jobs strip of the views (PLAN.md D68 "Views"): the rows
//! both front ends draw from `Jobs::list`, and [`Watch`], the one worker per
//! front end that blocks on the shown session's jobs and posts each new list
//! to the UI thread. No timer: the worker sleeps on the jobs' condition
//! variable until a start, an end or a kill, or until its watch is dropped.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use crate::tools::{JobInfo, JobState, Jobs};

/// The longest shortened command, in characters, before the ellipsis.
pub const COMMAND_CHARS: usize = 40;

/// What a row's mark says (each comes with a word: never colour alone).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Running,
    /// Ended with exit code 0, or with none.
    Exited,
    /// Ended with another exit code.
    Failed,
    Killed,
}

/// One job as the strip shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobRow {
    /// `job-<n>`: what `job_kill` takes.
    pub id: String,
    /// The first line of the command, cut at [`COMMAND_CHARS`].
    pub command: String,
    /// `running`, `exited 3`, `exited` or `killed`.
    pub word: String,
    pub mark: Mark,
    /// How long it ran, once it ended (D72).
    pub took: Option<std::time::Duration>,
}

impl JobRow {
    pub fn running(&self) -> bool {
        self.mark == Mark::Running
    }

    /// `job-1 ping -t localhost running`: the row as one plain text.
    pub fn text(&self) -> String {
        format!("{} {} {}", self.id, self.command, self.word)
    }
}

/// The rows of `list`, oldest first.
pub fn rows(list: &[JobInfo]) -> Vec<JobRow> {
    list.iter().map(row).collect()
}

fn row(info: &JobInfo) -> JobRow {
    let (word, mark) = match (info.state, info.exit_code) {
        (JobState::Running, _) => ("running".to_string(), Mark::Running),
        (JobState::Killed, _) => ("killed".to_string(), Mark::Killed),
        (_, Some(0)) => ("exited 0".to_string(), Mark::Exited),
        (_, Some(code)) => (format!("exited {code}"), Mark::Failed),
        (_, None) => ("exited".to_string(), Mark::Exited),
    };
    JobRow {
        id: info.id.clone(),
        command: short_command(&info.command),
        word,
        mark,
        took: (info.ended).and_then(|end| end.duration_since(info.started).ok()),
    }
}

/// The first non-empty line of `command`, its spaces collapsed, cut at
/// [`COMMAND_CHARS`] characters with `…`.
pub fn short_command(command: &str) -> String {
    let line = command.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let words: Vec<&str> = line.split_whitespace().collect();
    let joined = words.join(" ");
    let more = command.trim().lines().count() > 1;
    match joined.chars().count() > COMMAND_CHARS {
        true => joined.chars().take(COMMAND_CHARS - 1).collect::<String>() + "…",
        false if more => joined + " …",
        false => joined,
    }
}

/// The worker watching one session's jobs; dropping it ends the worker.
pub struct Watch {
    jobs: Arc<Jobs>,
    stop: Arc<AtomicBool>,
    generation: u64,
}

impl Watch {
    /// Starts the worker for `jobs` from revision `since` (read before the
    /// list the view shows now). `post(generation, list)` runs on the
    /// worker after each change; it returns false when the UI is gone, which
    /// ends the worker. `None` when the thread cannot start.
    pub fn start(
        jobs: Arc<Jobs>,
        since: u64,
        generation: u64,
        post: impl Fn(u64, Vec<JobInfo>) -> bool + Send + 'static,
    ) -> Option<Watch> {
        let stop = Arc::new(AtomicBool::new(false));
        let (worker_jobs, worker_stop) = (jobs.clone(), stop.clone());
        let spawned = thread::Builder::new()
            .name("fast-harness-jobs".into())
            .spawn(move || run(&worker_jobs, &worker_stop, since, generation, post));
        spawned.ok()?;
        Some(Watch {
            jobs,
            stop,
            generation,
        })
    }

    /// The generation its lists carry.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// It watches `jobs` (the same session's).
    pub fn watches(&self, jobs: &Arc<Jobs>) -> bool {
        Arc::ptr_eq(&self.jobs, jobs)
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.jobs.wake();
    }
}

fn run(
    jobs: &Jobs,
    stop: &AtomicBool,
    mut seen: u64,
    generation: u64,
    post: impl Fn(u64, Vec<JobInfo>) -> bool,
) {
    while let Some(revision) = jobs.wait_change_or_stop(seen, stop) {
        seen = revision;
        if !post(generation, jobs.list()) {
            return;
        }
    }
}

/// The shown session's watch on the UI thread: which generation is
/// current, so a list from an earlier session is dropped.
#[derive(Default)]
pub struct Follow {
    watch: Option<Watch>,
    next: u64,
}

impl Follow {
    /// Watches `jobs` (or nothing) from now on; returns the rows to show at
    /// once. Keeps the running worker when it already watches `jobs`.
    pub fn target(
        &mut self,
        jobs: Option<Arc<Jobs>>,
        post: impl Fn(u64, Vec<JobInfo>) -> bool + Send + 'static,
    ) -> Option<Vec<JobRow>> {
        let Some(jobs) = jobs else {
            self.watch = None;
            return Some(Vec::new());
        };
        if self.watch.as_ref().is_some_and(|w| w.watches(&jobs)) {
            return None;
        }
        self.watch = None;
        self.next += 1;
        let since = jobs.revision();
        let list = jobs.list();
        self.watch = Watch::start(jobs, since, self.next, post);
        Some(rows(&list))
    }

    /// The rows of a list posted by generation `generation`, or `None` when
    /// it came from a watch that was replaced or stopped.
    pub fn arrived(&self, generation: u64, list: &[JobInfo]) -> Option<Vec<JobRow>> {
        let current = self.watch.as_ref().map(Watch::generation);
        (current == Some(generation)).then(|| rows(list))
    }

    /// Ends the worker (the window or the terminal closes).
    pub fn stop(&mut self) {
        self.watch = None;
    }
}

#[cfg(test)]
#[path = "jobs_tests.rs"]
mod tests;
