//! `<file>.lock`: a file made with `create_new` holding
//! `{"pid":..,"started":..}` (`platform::process_start`) while one process
//! changes `<file>`; removed when the change ends, also on a panic.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde::Deserialize;

use crate::platform;

/// Appended to the file name of the guarded file.
pub const LOCK_SUFFIX: &str = ".lock";
/// An unreadable lock (its writer may be between create and write) is stale
/// only after this.
const UNREADABLE_STALE: Duration = Duration::from_secs(10);
/// Without a liveness backend a lock of another process is held this long.
const BLIND_STALE: Duration = Duration::from_secs(600);
const RETRY: Duration = Duration::from_millis(15);

#[derive(Deserialize)]
struct Holder {
    pid: u32,
    started: u64,
}

/// `<path>.lock`: the file name plus `.lock`, in the same folder.
pub fn lock_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(LOCK_SUFFIX);
    path.with_file_name(name)
}

/// A lock written by `holder` (`pid`, `started`; `None` when its content is
/// unreadable) and last changed `age` ago is stale: its holder is gone (no
/// process with that pid and start time), or the content is unreadable and
/// older than 10 seconds. Where `platform::process_start` has no backend (it
/// returns `None` even for this process) a lock younger than 10 minutes
/// counts as held and an older one as stale.
pub fn stale(holder: Option<(u32, u64)>, age: Duration) -> bool {
    let Some((pid, started)) = holder else {
        return age > UNREADABLE_STALE;
    };
    if platform::process_start(std::process::id()).is_none() {
        return age > BLIND_STALE;
    }
    platform::process_start(pid) != Some(started)
}

/// Runs `f` while `<path>.lock` is held by this call. A lock held elsewhere
/// (another process, or another thread of this one) is waited for, checking
/// about every 15 ms; a stale one (see [`stale`]) is removed. After `wait` the
/// result is a `TimedOut` error naming the lock and its holder's pid.
pub fn locked<T>(path: &Path, wait: Duration, f: impl FnOnce() -> T) -> io::Result<T> {
    let lock = lock_path(path);
    let _guard = acquire(&lock, wait)?;
    Ok(f())
}

/// Removes the lock when dropped, so a panic in `f` frees it too.
struct Guard(PathBuf);

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn acquire(lock: &Path, wait: Duration) -> io::Result<Guard> {
    let start = Instant::now();
    loop {
        let failed = match create(lock) {
            Ok(guard) => return Ok(guard),
            Err(err) => err,
        };
        let busy = match failed.kind() {
            io::ErrorKind::AlreadyExists => true,
            // Windows: the name of a file being deleted is not free yet.
            io::ErrorKind::PermissionDenied => false,
            _ => return Err(failed),
        };
        if busy && remove_if_stale(lock) {
            continue;
        }
        if start.elapsed() >= wait {
            return Err(timed_out(lock, failed));
        }
        std::thread::sleep(RETRY);
    }
}

/// Makes `lock` with `create_new` and writes this process into it.
fn create(lock: &Path) -> io::Result<Guard> {
    if let Some(dir) = lock.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir)?;
    }
    let mut file = OpenOptions::new().write(true).create_new(true).open(lock)?;
    let guard = Guard(lock.to_path_buf());
    let pid = std::process::id();
    let started = platform::process_start(pid).unwrap_or(0);
    file.write_all(format!("{{\"pid\":{pid},\"started\":{started}}}").as_bytes())?;
    Ok(guard)
}

/// Removes `lock` when it is stale; true when it was (or is already gone).
fn remove_if_stale(lock: &Path) -> bool {
    let Ok(bytes) = fs::read(lock) else {
        return false;
    };
    if !stale(holder(&bytes), age(lock)) {
        return false;
    }
    remove_if_same(lock, &bytes);
    true
}

/// Removes `lock` only while it still holds `bytes`: a process that took the
/// lock after our read wrote other content (another pid or start time), so
/// its fresh lock stays. The window that remains: a lock taken by another
/// process between this re-read and the remove (microseconds) is removed
/// too, so that process and the next taker may overlap once. Taking needs no
/// such check: after `create_new` succeeds the lock is ours.
pub(crate) fn remove_if_same(lock: &Path, bytes: &[u8]) -> bool {
    if fs::read(lock).is_ok_and(|now| now == bytes) {
        return fs::remove_file(lock).is_ok();
    }
    false
}

fn holder(bytes: &[u8]) -> Option<(u32, u64)> {
    let holder: Holder = serde_json::from_slice(bytes).ok()?;
    Some((holder.pid, holder.started))
}

/// How long ago `path` was last written; zero when unknown (a young lock).
pub(crate) fn age(path: &Path) -> Duration {
    (fs::metadata(path).and_then(|meta| meta.modified()).ok())
        .and_then(|time| SystemTime::now().duration_since(time).ok())
        .unwrap_or_default()
}

fn timed_out(lock: &Path, last: io::Error) -> io::Error {
    let pid = fs::read(lock).ok().and_then(|bytes| holder(&bytes));
    let who = match pid {
        Some((pid, _)) => format!("pid {pid}"),
        None => format!("an unknown process ({last})"),
    };
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!("{} is held by {who}", lock.display()),
    )
}
