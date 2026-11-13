//! Reads of a job's output that leave its read cursor alone (PLAN.md D72):
//! a view shows the newest output while the model's `job_output` still gets
//! every byte since its own last read.

use std::sync::atomic::{AtomicBool, Ordering};

use super::{Jobs, Reading};

/// Where a job's output stands: bytes received in all, and whether the job
/// ended with every byte in. A view waits for it to change.
pub type OutputMark = (u64, bool);

impl Jobs {
    /// The output of job `id` still in memory (the newest
    /// [`super::RING`] bytes) without moving the read cursor `read` uses:
    /// `skipped` counts the bytes that left the ring before it.
    pub fn peek(&self, id: &str) -> Result<Reading, String> {
        let job = self.job(id)?;
        let run = job.lock();
        let (skipped, bytes) = run.buffer.since(0);
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let info = job.info_of(&run);
        Ok(Reading {
            info,
            skipped,
            text,
        })
    }

    /// The [`OutputMark`] of job `id` now, `None` when there is no such job.
    pub fn output_mark(&self, id: &str) -> Option<OutputMark> {
        let job = self.job(id).ok()?;
        let run = job.lock();
        Some((run.buffer.total(), run.ended()))
    }

    /// Blocks until the [`OutputMark`] of job `id` differs from `seen`;
    /// `None` once `stop` is set and [`Jobs::wake`] is called, or when the
    /// job is gone. No timer: it sleeps on the job's condition variable.
    pub fn wait_output_or_stop(
        &self,
        id: &str,
        seen: OutputMark,
        stop: &AtomicBool,
    ) -> Option<OutputMark> {
        let job = self.job(id).ok()?;
        let mut run = job.lock();
        loop {
            // Read under the lock: `wake` takes it before it notifies.
            if stop.load(Ordering::SeqCst) {
                return None;
            }
            let mark = (run.buffer.total(), run.ended());
            if mark != seen {
                return Some(mark);
            }
            run = job.changed.wait(run).unwrap_or_else(|p| p.into_inner());
        }
    }
}
