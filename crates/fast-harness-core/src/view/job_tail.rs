//! The output of one background job as a view shows it (PLAN.md D72): a
//! bounded tail of what is still in memory, read with `Jobs::peek` so the
//! model's `job_output` cursor stays where it is, and [`TailWatch`], the
//! worker that re-reads it after each change. No timer: the worker sleeps
//! on the job's condition variable, and waits [`TAIL_EVERY`] after a post so
//! a chatty job wakes the UI a few times a second at most.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::tools::{JobState, Jobs};

/// Lines a tail keeps at most: the newest.
pub const TAIL_LINES: usize = 400;
/// Bytes a tail keeps at most: the newest (the software renderer lays text
/// out in i16 pixels, so the panel's text stays well under that).
pub const TAIL_BYTES: usize = 32 * 1024;
/// The least time between two posts of one worker.
pub const TAIL_EVERY: Duration = Duration::from_millis(150);

/// The newest output of a job.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tail {
    /// At most [`TAIL_LINES`] lines and [`TAIL_BYTES`] bytes.
    pub text: String,
    /// Earlier output was dropped (here or from the job's memory).
    pub dropped: bool,
    pub running: bool,
}

/// The tail of `text`, of which `skipped` earlier bytes are gone already.
pub fn tail(text: &str, skipped: u64, running: bool) -> Tail {
    let mut start = text.len().saturating_sub(TAIL_BYTES);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let mut kept = &text[start..];
    let body = kept.strip_suffix('\n').unwrap_or(kept);
    if let Some((index, _)) = body.rmatch_indices('\n').nth(TAIL_LINES - 1) {
        kept = &kept[index + 1..];
    }
    Tail {
        text: kept.to_string(),
        dropped: skipped > 0 || kept.len() < text.len(),
        running,
    }
}

/// The tail of job `id` now; `None` when there is no such job.
pub fn read(jobs: &Jobs, id: &str) -> Option<Tail> {
    let reading = jobs.peek(id).ok()?;
    let running = reading.info.state == JobState::Running;
    Some(tail(&reading.text, reading.skipped, running))
}

/// The worker re-reading one job's tail; dropping it ends the worker.
pub struct TailWatch {
    jobs: Arc<Jobs>,
    stop: Arc<AtomicBool>,
}

impl TailWatch {
    /// Starts the worker for job `id`: `post(tail)` runs on the worker with
    /// the tail now and after each change; it returns false when the UI is
    /// gone, which ends the worker. `None` when the thread cannot start.
    pub fn start(
        jobs: Arc<Jobs>,
        id: &str,
        post: impl Fn(Tail) -> bool + Send + 'static,
    ) -> Option<TailWatch> {
        let stop = Arc::new(AtomicBool::new(false));
        let (worker_jobs, worker_stop, id) = (jobs.clone(), stop.clone(), id.to_string());
        let spawned = thread::Builder::new()
            .name("fast-harness-job-tail".into())
            .spawn(move || run(&worker_jobs, &id, &worker_stop, post));
        spawned.ok()?;
        Some(TailWatch { jobs, stop })
    }
}

impl Drop for TailWatch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.jobs.wake();
    }
}

fn run(jobs: &Jobs, id: &str, stop: &AtomicBool, post: impl Fn(Tail) -> bool) {
    let Some(mut seen) = jobs.output_mark(id) else {
        return;
    };
    loop {
        // The mark before the read: a change during it wakes the next wait.
        let Some(tail) = read(jobs, id) else {
            return;
        };
        if stop.load(Ordering::SeqCst) || !post(tail) {
            return;
        }
        thread::sleep(TAIL_EVERY);
        match jobs.wait_output_or_stop(id, seen, stop) {
            Some(mark) => seen = mark,
            None => return,
        }
    }
}

#[cfg(test)]
#[path = "job_tail_tests.rs"]
mod tests;
