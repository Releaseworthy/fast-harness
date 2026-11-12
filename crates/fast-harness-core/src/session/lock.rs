//! `<id>.lock`: the process that runs a session's turn (PLAN.md D49). It is
//! written when a turn starts, rewritten at each step, removed when the turn
//! ends; a lock whose process no longer runs (pid and start time, through
//! `platform::process_start`) is stale and removed by the next reader.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::coord::lock_path;
use crate::platform;
use crate::store;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lock {
    pub pid: u32,
    /// The process start time (`platform::process_start`), against pid reuse.
    pub started: u64,
    /// The program's file name (`fast-harness.exe`, `fast-harness-gui.exe`).
    pub program: String,
    pub id: String,
    pub cwd: String,
    /// Unix milliseconds of the turn start.
    pub since: u64,
    /// `running`, `pausing` or `stopping`.
    pub state: String,
    pub turn: u32,
    /// The step running now (0 between turns).
    pub step: u32,
}

/// The lock at `path`, if it parses.
pub fn read(path: &Path) -> Option<Lock> {
    let text = crate::store::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// The process of `lock` still runs.
pub fn live(lock: &Lock) -> bool {
    platform::process_start(lock.pid) == Some(lock.started)
}

/// The lock of session log `log` when its process runs; a stale lock is
/// removed. An unreadable lock counts as not running and is kept.
pub fn live_lock(log: &Path) -> Option<Lock> {
    let path = lock_path(log);
    let lock = read(&path)?;
    if live(&lock) {
        return Some(lock);
    }
    let _ = fs::remove_file(&path);
    None
}

/// Waits until session log `log` has no live lock, checking every `every`;
/// false when `limit` passed first.
pub fn wait(log: &Path, every: Duration, limit: Option<Duration>) -> bool {
    let start = Instant::now();
    while live_lock(log).is_some() {
        if limit.is_some_and(|limit| start.elapsed() >= limit) {
            return false;
        }
        std::thread::sleep(every);
    }
    true
}

/// A running session found by [`running`].
#[derive(Clone, Debug)]
pub struct Running {
    pub lock: Lock,
    pub log: PathBuf,
}

/// Every session with a live lock under `<home>/sessions`, oldest turn first;
/// stale locks are removed on the way.
pub fn running(home: &Path) -> Vec<Running> {
    let mut found = Vec::new();
    for folder in sub_dirs(&home.join("sessions")) {
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("lock") {
                continue;
            }
            let log = path.with_extension("jsonl");
            if let Some(lock) = live_lock(&log) {
                found.push(Running { lock, log });
            }
        }
    }
    found.sort_by(|a, b| (a.lock.since, &a.lock.id).cmp(&(b.lock.since, &b.lock.id)));
    found
}

fn sub_dirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    (entries.flatten())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect()
}

/// The log of session `id` in any directory under `<home>/sessions`.
pub fn find(home: &Path, id: &str) -> Option<PathBuf> {
    if !super::valid_id(id) {
        return None;
    }
    (sub_dirs(&home.join("sessions")).into_iter())
        .map(|folder| folder.join(format!("{id}.jsonl")))
        .find(|log| log.is_file())
}

/// The lock this process holds; removed on drop (only while it is still ours).
pub struct Held {
    path: PathBuf,
    lock: Lock,
}

impl Held {
    /// Takes the lock of session log `log` for this process: the file is made
    /// with `create_new`, so of two processes starting a turn at once one
    /// wins. A live lock of another process (or an unreadable one younger
    /// than 10 seconds, see `store::stale`) is an `AlreadyExists` error whose
    /// message is [`busy_note`]; a stale one is removed and the create
    /// retried once. A lock of this very process is rewritten, as before.
    pub fn take(log: &Path, id: &str, cwd: &str, turn: u32) -> io::Result<Held> {
        let held = Held::new(log, id, cwd, turn);
        let text = serde_json::to_string(&held.lock).map_err(io::Error::other)?;
        for retry in [false, true] {
            match create(&held.path, &text) {
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
                done => return done.map(|()| held),
            }
            if held.claim_existing(retry)? {
                return Ok(held);
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("session {id}: its lock is taken"),
        ))
    }

    fn new(log: &Path, id: &str, cwd: &str, turn: u32) -> Held {
        let pid = std::process::id();
        let program = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let lock = Lock {
            pid,
            started: platform::process_start(pid).unwrap_or(0),
            program,
            id: id.to_string(),
            cwd: cwd.to_string(),
            since: crate::trace::now_ms(),
            state: "running".into(),
            turn,
            step: 0,
        };
        Held {
            path: lock_path(log),
            lock,
        }
    }

    /// The lock file exists: true when it is this process's own (rewritten
    /// as ours), an error when another process holds it (or `last`, the
    /// retry, found it again), false after a stale one was removed.
    fn claim_existing(&self, last: bool) -> io::Result<bool> {
        let Ok(bytes) = crate::store::read(&self.path) else {
            return Ok(false);
        };
        let found = serde_json::from_slice::<Lock>(&bytes).ok();
        if let Some(lock) = &found {
            if lock.pid == self.lock.pid && lock.started == self.lock.started {
                return self.write().map(|()| true);
            }
        }
        let holder = found.as_ref().map(|lock| (lock.pid, lock.started));
        if last || !store::stale(holder, store::age(&self.path)) {
            let note = match &found {
                Some(lock) => busy_note(&self.lock.id, lock),
                None => format!("session {} is starting in another process", self.lock.id),
            };
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, note));
        }
        store::remove_if_same(&self.path, &bytes);
        Ok(false)
    }

    /// Rewrites the lock when `state`, `turn` or `step` changed.
    pub fn update(&mut self, state: &str, turn: u32, step: u32) {
        let lock = &self.lock;
        if lock.state == state && lock.turn == turn && lock.step == step {
            return;
        }
        self.lock.state = state.to_string();
        (self.lock.turn, self.lock.step) = (turn, step);
        let _ = self.write();
    }

    pub fn lock(&self) -> &Lock {
        &self.lock
    }

    /// `store::write_atomic`, so readers never see half.
    fn write(&self) -> io::Result<()> {
        let text = serde_json::to_string(&self.lock).map_err(io::Error::other)?;
        store::write_atomic(&self.path, text.as_bytes())
    }
}

/// Makes `path` with `create_new` and writes `text` into the new handle; a
/// failed write removes the file again.
fn create(path: &Path, text: &str) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let written = file.write_all(text.as_bytes());
    if written.is_err() {
        drop(file);
        let _ = fs::remove_file(path);
    }
    written
}

/// The note of a turn refused because `lock` (another process) runs session
/// `id` (PLAN.md D49).
pub fn busy_note(id: &str, lock: &Lock) -> String {
    format!(
        "session {id} is running in {} (pid {}): wait for it (`fast-harness wait {id}`) or \
        reach it with `fast-harness send`",
        lock.program, lock.pid
    )
}

impl Drop for Held {
    fn drop(&mut self) {
        let ours = read(&self.path)
            .is_some_and(|lock| lock.pid == self.lock.pid && lock.started == self.lock.started);
        if ours {
            let _ = fs::remove_file(&self.path);
        }
    }
}
