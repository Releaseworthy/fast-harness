//! OS specific code. Everything outside this module is portable Rust.
//!
//! Every backend exports the same items (one file per feature):
//! - `secret`: `protect(&[u8]) -> Result<Vec<u8>>`, `unprotect(&[u8]) -> Result<Vec<u8>>`
//!   (data only the current OS user can decrypt: DPAPI on Windows).
//! - `random`: `random_fill(&mut [u8]) -> Result<()>` (the OS CSPRNG: BCryptGenRandom on Windows,
//!   `/dev/urandom` on other Unix-like systems).
//! - `open`: `open_url(&str) -> Result<()>` (the default browser),
//!   `open_folder(&Path) -> Result<()>` (Explorer).
//! - `file`: `copy_permissions(from, to) -> io::Result<()>` (Unix mode bits of a replaced file;
//!   a no-op on Windows).
//! - `process`: `ProcessGroup` (a process and every process it starts, killed together).
//! - `window`: `set_big_window_icon() -> Result<usize>` (the taskbar icon from the exe's icon
//!   group 1 on the calling thread's visible windows; a no-op returning 0 elsewhere) and
//!   `thread_window() -> Option<isize>` (the calling thread's visible unowned window).
//! - `folder`: `pick_folder(owner, title) -> Result<Option<PathBuf>>` (the modal "Select folder"
//!   dialog owned by window `owner`; `Ok(None)` when cancelled; IFileOpenDialog on Windows).
//!
//! An OS without a backend uses `unsupported`, whose items return
//! `PlatformError::Unsupported`, so the core still builds there (except `random_fill` on Unix).
//! `protect` stays `Unsupported` off Windows on purpose: no plaintext secrets (PLAN.md D9).

use std::fmt;
use std::path::{Path, PathBuf};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(not(windows))]
mod unsupported;
#[cfg(not(windows))]
pub use unsupported::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlatformError {
    Unsupported,
    Os { code: i32, message: String },
}

impl fmt::Display for PlatformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlatformError::Unsupported => f.write_str("not supported on this OS"),
            PlatformError::Os { code, message } => write!(f, "OS error {code}: {message}"),
        }
    }
}

impl std::error::Error for PlatformError {}

impl PlatformError {
    /// The calling thread's last OS error.
    pub fn last_os_error() -> PlatformError {
        let err = std::io::Error::last_os_error();
        PlatformError::Os {
            code: err.raw_os_error().unwrap_or(0),
            message: err.to_string(),
        }
    }
}

/// The user's home directory (`USERPROFILE` on Windows, `HOME` elsewhere).
pub fn home_dir() -> Option<PathBuf> {
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// `~/.fast-harness`, the root of config, accounts and sessions. `FAST_HARNESS_HOME` overrides it.
/// A `~/.nh` left by a build from before the rename moves there first ([`adopt_old_home`]).
pub fn app_home() -> Option<PathBuf> {
    match std::env::var_os("FAST_HARNESS_HOME").filter(|value| !value.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir)),
        None => home_dir().map(|home| adopt_old_home(&home)),
    }
}

/// `<home>/.fast-harness`. When only `<home>/.nh` exists (the folder of the builds named `nh`,
/// before 2026-11-12) it is renamed to `.fast-harness` first; if that rename fails (an old build
/// still holds a file) this run keeps using `.nh`. An existing `.fast-harness` always wins.
pub fn adopt_old_home(home: &Path) -> PathBuf {
    let new = home.join(".fast-harness");
    let old = home.join(".nh");
    if new.exists() || !old.is_dir() {
        return new;
    }
    match std::fs::rename(&old, &new) {
        Ok(()) => new,
        Err(_) => old,
    }
}
