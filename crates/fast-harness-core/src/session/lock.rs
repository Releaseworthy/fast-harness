//! `<id>.lock`: the process that runs a session's turn (PLAN.md D49). It is
//! written when a turn starts, rewritten at each step, removed when the turn
//! ends; a lock whose process no longer runs (pid and start time, through
//! `platform::process_start`) is stale and removed by the next reader.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::coord::lock_path;
use crate::platform;

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
    let text = fs::read_to_string(path).ok()?;
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
    /// Writes the lock of session log `log` for this process.
    pub fn take(log: &Path, id: &str, cwd: &str, turn: u32) -> io::Result<Held> {
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
        let held = Held {
            path: lock_path(log),
            lock,
        };
        held.write()?;
        Ok(held)
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

    /// A temporary file renamed over the lock, so readers never see half.
    fn write(&self) -> io::Result<()> {
        let tmp = self.path.with_extension("lock.tmp");
        let text = serde_json::to_string(&self.lock).map_err(io::Error::other)?;
        fs::write(&tmp, text)?;
        fs::rename(&tmp, &self.path)
    }
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
