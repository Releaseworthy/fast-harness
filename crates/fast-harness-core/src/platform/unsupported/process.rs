//! Fallback without process trees: a plain spawn, and `kill` stops only the
//! direct child (by pid, through the `kill` command on Unix). Processes the
//! child started may outlive it.

use std::process::{Child, Command, Stdio};

use crate::platform::PlatformError;

pub struct ProcessGroup {
    /// Pid of the direct child.
    pid: u32,
}

impl ProcessGroup {
    pub fn spawn(cmd: &mut Command) -> Result<(ProcessGroup, Child), PlatformError> {
        let child = cmd.spawn().map_err(|err| PlatformError::Os {
            code: err.raw_os_error().unwrap_or(0),
            message: err.to_string(),
        })?;
        Ok((ProcessGroup { pid: child.id() }, child))
    }

    /// Ends the group after the child is done. Without process trees there
    /// is nothing to release: leftover grandchildren keep running.
    pub fn close(self) {}

    /// Kills what is left of the group after the child was reaped. Without
    /// process trees nothing is known to be left, and the reaped pid may
    /// already belong to another process: nothing is done.
    pub fn kill_rest(&self) {}

    pub fn kill(&self) -> Result<(), PlatformError> {
        if !cfg!(unix) {
            return Err(PlatformError::Unsupported);
        }
        let status = Command::new("kill")
            .arg("-KILL")
            .arg(self.pid.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|err| PlatformError::Os {
                code: err.raw_os_error().unwrap_or(0),
                message: err.to_string(),
            })?;
        if status.success() {
            Ok(())
        } else {
            Err(PlatformError::Os {
                code: status.code().unwrap_or(-1),
                message: format!("kill -KILL {} failed", self.pid),
            })
        }
    }
}
