//! Background shell jobs of one session (PLAN.md D68). `shell` with
//! `background: true` starts one; `job_output`, `job_kill` and `job_list`
//! read, stop and list them. A job's process tree is one
//! `platform::ProcessGroup`; one thread per pipe blocks on its reads and one
//! blocks on the exit, and readers wait on a condition variable: nothing
//! polls. Jobs end with their session: `Service::close` kills them, the
//! last handle to drop kills them, and on Windows the Job Object of each
//! dies with the process.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::job_buffer::Buffer;
pub use super::job_buffer::{LOG_CAP, RING};
use super::shell::shell_command;
use super::shell_live::complete_end;
use crate::platform::ProcessGroup;

#[path = "job_peek.rs"]
mod peek;
pub use peek::OutputMark;

/// Jobs of one session that may run at once.
pub const MAX_RUNNING: usize = 8;
/// Jobs one session keeps listed; the oldest ended ones go first.
const MAX_KEPT: usize = 32;
/// The longest a wait goes without looking at the cancel flag (the flag
/// cannot wake a condition variable).
const BACKSTOP: Duration = Duration::from_millis(200);
/// How long a kill waits for the exit it caused.
const KILL_WAIT: Duration = Duration::from_secs(5);

/// Where a job is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum JobState {
    Running,
    /// The command ended by itself.
    Exited,
    /// `job_kill` or the session's close ended it.
    Killed,
}

/// One row of a session's job list (what the views show).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct JobInfo {
    /// `job-<n>`, unique in the session.
    pub id: String,
    pub command: String,
    pub state: JobState,
    /// `None` while it runs, or when the OS gave none.
    pub exit_code: Option<i32>,
    pub started: SystemTime,
    /// When the shell exited; `None` while it runs.
    pub ended: Option<SystemTime>,
    /// Output bytes received in all.
    pub bytes: u64,
    /// The log file under the session folder, when there is one.
    pub log: Option<PathBuf>,
}

/// What one read returns.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Reading {
    pub info: JobInfo,
    /// New bytes that left the in-memory buffer before this read.
    pub skipped: u64,
    /// The new output (whole characters only while the job runs).
    pub text: String,
}

struct Job {
    id: String,
    command: String,
    started: SystemTime,
    group: ProcessGroup,
    run: Mutex<Run>,
    changed: Condvar,
    revision: Arc<Revision>,
}

#[derive(Default)]
struct Run {
    buffer: Buffer,
    /// Offset of the next byte a read returns.
    cursor: u64,
    /// `Some` once the shell exited: its exit code, if any.
    exit: Option<Option<i32>>,
    /// When the exit was recorded.
    ended_at: Option<SystemTime>,
    killed: bool,
    /// A kill was signalled; killed is set only when it ended the job.
    kill_asked: bool,
    /// Pipes not yet at their end.
    pipes: u8,
}

impl Run {
    /// Exited, and every byte it wrote is in the buffer.
    fn ended(&self) -> bool {
        self.exit.is_some() && self.pipes == 0
    }
}

/// A counter of job list changes and its condition variable.
#[derive(Default)]
struct Revision {
    count: Mutex<u64>,
    changed: Condvar,
}

impl Revision {
    fn lock(&self) -> MutexGuard<'_, u64> {
        self.count.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn bump(&self) {
        *self.lock() += 1;
        self.changed.notify_all();
    }
}

/// The jobs of one session (`ToolSession::jobs`, `Session::jobs`).
#[derive(Default)]
pub struct Jobs {
    table: Mutex<Table>,
    revision: Arc<Revision>,
}

#[derive(Default)]
struct Table {
    jobs: Vec<Arc<Job>>,
    next: u64,
}

impl std::fmt::Debug for Jobs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self.lock().jobs.len();
        f.debug_struct("Jobs").field("jobs", &count).finish()
    }
}

impl Jobs {
    fn lock(&self) -> MutexGuard<'_, Table> {
        self.table.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Starts `command` in `cwd` as a job logged under `log_dir`; returns
    /// its id. Fails past [`MAX_RUNNING`] running jobs.
    pub fn start(
        &self,
        command: &str,
        cwd: &Path,
        log_dir: Option<&Path>,
    ) -> Result<String, String> {
        if command.trim().is_empty() {
            return Err("command must not be empty".to_string());
        }
        let mut table = self.lock();
        let running = table.jobs.iter().filter(|job| job.lock().exit.is_none());
        if running.count() >= MAX_RUNNING {
            return Err(format!(
                "{MAX_RUNNING} background jobs are running in this session, the cap: \
                 stop one with job_kill first"
            ));
        }
        prune(&mut table.jobs);
        table.next += 1;
        let id = format!("job-{}", table.next);
        let job = spawn(&id, command, cwd, log_dir, self.revision.clone())?;
        table.jobs.push(job);
        drop(table);
        self.revision.bump();
        Ok(id)
    }

    fn job(&self, id: &str) -> Result<Arc<Job>, String> {
        let table = self.lock();
        let found = table.jobs.iter().find(|job| job.id == id.trim()).cloned();
        found.ok_or_else(|| format!("no background job `{}` in this session", id.trim()))
    }

    /// The output of job `id` since the last read. Waits up to `wait` for
    /// new output (`until_end`: for the end), or until `cancel` is set.
    pub fn read(
        &self,
        id: &str,
        wait: Duration,
        until_end: bool,
        cancel: Option<&AtomicBool>,
    ) -> Result<Reading, String> {
        let job = self.job(id)?;
        let deadline = Instant::now() + wait;
        let mut run = job.wait_until(deadline, cancel, |run| {
            run.ended() || (!until_end && has_whole_text(run))
        });
        let (skipped, bytes) = run.buffer.since(run.cursor);
        let end = match run.ended() {
            true => bytes.len(),
            false => complete_end(&bytes),
        };
        run.cursor += skipped + end as u64;
        let text = String::from_utf8_lossy(&bytes[..end]).into_owned();
        let info = job.info_of(&run);
        Ok(Reading {
            info,
            skipped,
            text,
        })
    }

    /// Kills job `id` and its whole process tree; waits briefly for the exit.
    pub fn kill(&self, id: &str) -> Result<JobInfo, String> {
        let job = self.job(id)?;
        job.signal_kill();
        drop(job.wait_until(Instant::now() + KILL_WAIT, None, |run| run.exit.is_some()));
        Ok(job.info())
    }

    /// Kills every running job and waits briefly for their exits (the
    /// session closes).
    pub fn kill_all(&self) {
        let jobs = self.lock().jobs.clone();
        jobs.iter().for_each(|job| job.signal_kill());
        let deadline = Instant::now() + KILL_WAIT;
        for job in &jobs {
            drop(job.wait_until(deadline, None, |run| run.exit.is_some()));
        }
    }

    /// The jobs, oldest first.
    pub fn list(&self) -> Vec<JobInfo> {
        let jobs = self.lock().jobs.clone();
        jobs.iter().map(|job| job.info()).collect()
    }

    /// Counts the changes of the list: a start, an end, a kill.
    pub fn revision(&self) -> u64 {
        *self.revision.lock()
    }

    /// Blocks until [`Jobs::revision`] differs from `since` or `timeout`
    /// passes; returns the revision. A view's thread waits here.
    pub fn wait_change(&self, since: u64, timeout: Duration) -> u64 {
        let deadline = Instant::now() + timeout;
        let mut count = self.revision.lock();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if *count != since || left.is_zero() {
                return *count;
            }
            count = match self.revision.changed.wait_timeout(count, left) {
                Ok((guard, _)) => guard,
                Err(poison) => poison.into_inner().0,
            };
        }
    }

    /// [`Jobs::wait_change`] with no timeout that also ends, with `None`,
    /// once `stop` is set and [`Jobs::wake`] is called: a view's worker
    /// waits here for as long as its session is shown (no timer).
    pub fn wait_change_or_stop(&self, since: u64, stop: &AtomicBool) -> Option<u64> {
        let mut count = self.revision.lock();
        loop {
            // Read under the lock: `wake` takes it before it notifies, so a
            // stop set after this check still wakes the wait below.
            if stop.load(Ordering::SeqCst) {
                return None;
            }
            if *count != since {
                return Some(*count);
            }
            count = (self.revision.changed.wait(count)).unwrap_or_else(|p| p.into_inner());
        }
    }

    /// Wakes every [`Jobs::wait_change_or_stop`] and
    /// [`Jobs::wait_output_or_stop`] so it reads its stop flag.
    pub fn wake(&self) {
        let count = self.revision.lock();
        self.revision.changed.notify_all();
        drop(count);
        let jobs = self.lock().jobs.clone();
        for job in jobs {
            let _run = job.lock();
            job.changed.notify_all();
        }
    }
}

impl Drop for Jobs {
    fn drop(&mut self) {
        self.lock().jobs.iter().for_each(|job| job.signal_kill());
    }
}

/// Drops the oldest ended jobs past [`MAX_KEPT`] - 1.
fn prune(jobs: &mut Vec<Arc<Job>>) {
    while jobs.len() >= MAX_KEPT {
        match jobs.iter().position(|job| job.lock().exit.is_some()) {
            Some(index) => drop(jobs.remove(index)),
            None => return,
        }
    }
}

impl Job {
    fn lock(&self) -> MutexGuard<'_, Run> {
        self.run.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn info(&self) -> JobInfo {
        self.info_of(&self.lock())
    }

    fn info_of(&self, run: &Run) -> JobInfo {
        let state = match (run.exit, run.killed) {
            (None, _) => JobState::Running,
            (Some(_), true) => JobState::Killed,
            (Some(_), false) => JobState::Exited,
        };
        JobInfo {
            id: self.id.clone(),
            command: self.command.clone(),
            state,
            exit_code: run.exit.flatten(),
            started: self.started,
            ended: run.ended_at,
            bytes: run.buffer.total(),
            log: run.buffer.log_path.clone(),
        }
    }

    /// Kills the process tree of a running job (an ended one stays as it is).
    fn signal_kill(&self) {
        let mut run = self.lock();
        if run.exit.is_none() {
            run.kill_asked = true;
            drop(run);
            let _ = self.group.kill();
        }
    }

    /// Waits on the condition variable until `done`, the deadline or the
    /// cancel flag; returns the state locked.
    fn wait_until(
        &self,
        deadline: Instant,
        cancel: Option<&AtomicBool>,
        done: impl Fn(&Run) -> bool,
    ) -> MutexGuard<'_, Run> {
        let mut run = self.lock();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let cancelled = cancel.is_some_and(|flag| flag.load(Ordering::Relaxed));
            if done(&run) || left.is_zero() || cancelled {
                return run;
            }
            let slice = if cancel.is_some() {
                left.min(BACKSTOP)
            } else {
                left
            };
            run = match self.changed.wait_timeout(run, slice) {
                Ok((guard, _)) => guard,
                Err(poison) => poison.into_inner().0,
            };
        }
    }
}

fn spawn(
    id: &str,
    command: &str,
    cwd: &Path,
    log_dir: Option<&Path>,
    revision: Arc<Revision>,
) -> Result<Arc<Job>, String> {
    let mut cmd = shell_command(command);
    cmd.current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let (group, mut child) =
        ProcessGroup::spawn(&mut cmd).map_err(|err| format!("cannot start the shell: {err}"))?;
    // A resumed session numbers its jobs from 1 again: the time keeps logs apart.
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let log = log_dir.map(|dir| dir.join(format!("{id}-{stamp}.log")));
    let mut pipes: Vec<Box<dyn Read + Send>> = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        pipes.push(Box::new(stdout));
    }
    if let Some(stderr) = child.stderr.take() {
        pipes.push(Box::new(stderr));
    }
    let run = Run {
        buffer: Buffer::new(log),
        pipes: pipes.len() as u8,
        ..Run::default()
    };
    let job = Arc::new(Job {
        id: id.to_string(),
        command: command.to_string(),
        started: SystemTime::now(),
        group,
        run: Mutex::new(run),
        changed: Condvar::new(),
        revision,
    });
    pipes
        .into_iter()
        .for_each(|pipe| read_into(pipe, job.clone()));
    wait_exit(child, job.clone());
    Ok(job)
}

/// Appends what `source` produces to the job's buffer from a thread that
/// blocks on the pipe.
fn read_into(mut source: Box<dyn Read + Send>, job: Arc<Job>) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match source.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => job.lock().buffer.push(&buf[..n]),
            }
            job.changed.notify_all();
        }
        let mut run = job.lock();
        run.pipes = run.pipes.saturating_sub(1);
        drop(run);
        job.changed.notify_all();
    });
}

/// Records the exit from a thread that blocks on it, then ends what the
/// command left running so the pipes close.
fn wait_exit(mut child: Child, job: Arc<Job>) {
    std::thread::spawn(move || {
        let code = child.wait().ok().and_then(|status| status.code());
        job.group.kill_rest();
        let mut run = job.lock();
        run.killed = run.kill_asked && crate::platform::ended_by_kill(code);
        run.exit = Some(code);
        run.ended_at = Some(SystemTime::now());
        drop(run);
        job.changed.notify_all();
        job.revision.bump();
    });
}

/// New output holds a whole character (a partial UTF-8 sequence alone is
/// not worth waking a reader for), or bytes left the ring.
fn has_whole_text(run: &Run) -> bool {
    if run.buffer.total() <= run.cursor {
        return false;
    }
    let (skipped, bytes) = run.buffer.since(run.cursor);
    skipped > 0 || complete_end(&bytes) > 0
}
