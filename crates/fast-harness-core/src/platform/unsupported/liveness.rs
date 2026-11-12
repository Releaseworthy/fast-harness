//! Process liveness without a backend: `/proc/<pid>` where it exists (Linux;
//! every lock looks stale elsewhere), and a plain spawn with null stdio.

use std::process::{Child, Command, Stdio};

use crate::platform::PlatformError;

/// `Some(0)` while `/proc/<pid>` exists (no start time without a backend).
pub fn process_start(pid: u32) -> Option<u64> {
    std::path::Path::new(&format!("/proc/{pid}"))
        .exists()
        .then_some(0)
}

/// Starts `cmd` with null stdio, in its own process group on Unix.
pub fn spawn_detached(cmd: &mut Command) -> Result<Child, PlatformError> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn().map_err(|err| PlatformError::Os {
        code: err.raw_os_error().unwrap_or(0),
        message: err.to_string(),
    })
}
