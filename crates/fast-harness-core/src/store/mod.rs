//! Our own files under the home folder (PLAN.md D55, findings B3 and B4):
//! several processes (GUI, TUI, print mode, `serve --stdio`) share them.
//! [`write_atomic`] replaces a file whole (unique temporary name, `sync_all`,
//! rename), [`locked`] runs a closure while `<file>.lock` is held, and
//! [`update`] is the one way to read, change and write a file: inside the
//! lock it re-reads the current content, so no process overwrites another's
//! change with an older copy.
//!
//! Layer: imports only `platform` and std. User files go through
//! `tools::atomic` instead (a fallback to an in-place write, symlinks).

mod lock;

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub(crate) use lock::{age, remove_if_same};
pub use lock::{lock_path, locked, stale, LOCK_SUFFIX};

/// The default wait for a lock another process or thread holds.
pub const WAIT: Duration = Duration::from_secs(5);

/// Makes the temporary names of one process unique.
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// `.<name>.<pid>.<n>.tmp` in the folder of `target`, unique per process and
/// call; `None` when `target` has no file name.
pub fn temp_path(target: &Path) -> Option<PathBuf> {
    let name = target.file_name()?.to_string_lossy();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = format!(".{name}.{}.{n}.tmp", std::process::id());
    Some(target.with_file_name(temp))
}

/// Replaces `path` with `bytes`: the parent folder is created, a temporary
/// file next to it is written, synced and renamed over it, so a reader sees
/// the old or the new file, never half. On any error the temporary file goes
/// and the error is returned (no in-place fallback: these are our files).
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir)?;
    }
    let temp = temp_path(path).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} has no file name", path.display()),
        )
    })?;
    let written = write_new(&temp, bytes).and_then(|()| rename(&temp, path));
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written
}

fn write_new(temp: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(temp)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// How long a read or a rename refused with `PermissionDenied` is tried again.
const RENAME_WAIT: Duration = Duration::from_secs(2);

/// Reads a file another process may be replacing right now. On Windows an
/// open during a rename over the file can answer `PermissionDenied` (seen
/// on a CI runner, 2026-11-13: eight threads replacing and reading one
/// file); such a read is tried again for up to 2 s. Every reader of a file
/// written through [`write_atomic`] reads it here.
pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    retry(|| fs::read(path.as_ref()))
}

/// [`read`] as text.
pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    retry(|| fs::read_to_string(path.as_ref()))
}

fn retry<T>(mut op: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let start = Instant::now();
    let mut pause = Duration::from_millis(1);
    loop {
        match op() {
            Err(err)
                if err.kind() == io::ErrorKind::PermissionDenied
                    && start.elapsed() < RENAME_WAIT =>
            {
                std::thread::sleep(pause);
                pause = (pause * 2).min(Duration::from_millis(32));
            }
            done => return done,
        }
    }
}

/// A rename that retries on `PermissionDenied` for up to [`RENAME_WAIT`]:
/// Windows refuses a rename over a file that another rename is replacing
/// or a scanner holds open right now (100 ms was too short on a CI runner,
/// 2026-11-13). A read-only target never gets writable by waiting: its
/// error is returned at once.
fn rename(from: &Path, to: &Path) -> io::Result<()> {
    let start = Instant::now();
    let mut pause = Duration::from_millis(1);
    loop {
        match fs::rename(from, to) {
            Err(err)
                if err.kind() == io::ErrorKind::PermissionDenied
                    && start.elapsed() < RENAME_WAIT
                    && !read_only(to) =>
            {
                std::thread::sleep(pause);
                pause = (pause * 2).min(Duration::from_millis(32));
            }
            done => return done,
        }
    }
}

fn read_only(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|meta| meta.permissions().readonly())
}

/// Reads, changes and writes `path` under its lock: `f` gets the current
/// bytes (`None` when the file is missing) and returns the new bytes and a
/// result. An error of `f` writes nothing. Waits at most `wait` for the lock.
pub fn update<T>(
    path: &Path,
    wait: Duration,
    f: impl FnOnce(Option<Vec<u8>>) -> io::Result<(Vec<u8>, T)>,
) -> io::Result<T> {
    locked(path, wait, || {
        let current = match read(path) {
            Ok(bytes) => Some(bytes),
            Err(err) if err.kind() == io::ErrorKind::NotFound => None,
            Err(err) => return Err(err),
        };
        let (bytes, out) = f(current)?;
        write_atomic(path, &bytes)?;
        Ok(out)
    })?
}
