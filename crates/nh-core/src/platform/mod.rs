//! OS specific code. Everything outside this module is portable Rust.
//!
//! Every backend exports the same items (one file per feature):
//! - `secret`: `protect(&[u8]) -> Result<Vec<u8>>`, `unprotect(&[u8]) -> Result<Vec<u8>>`
//!   (data only the current OS user can decrypt: DPAPI on Windows).
//! - `random`: `random_fill(&mut [u8]) -> Result<()>` (the OS CSPRNG).
//! - `open`: `open_url(&str) -> Result<()>` (the default browser).
//! - `process`: `ProcessGroup` (a process and every process it starts, killed together).
//! - `window`: `set_big_window_icon() -> Result<usize>` (the taskbar icon from the exe's icon
//!   group 1 on the calling thread's visible windows; a no-op returning 0 elsewhere).
//!
//! An OS without a backend uses `unsupported`, whose items return
//! `PlatformError::Unsupported`, so the core still builds there.

use std::fmt;
use std::path::PathBuf;

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

/// `~/.nh`, the root of config, accounts and sessions. `NH_HOME` overrides it.
pub fn nh_home() -> Option<PathBuf> {
    match std::env::var_os("NH_HOME").filter(|value| !value.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir)),
        None => home_dir().map(|home| home.join(".nh")),
    }
}
