//! Process trees killed as one unit (a Job Object).

use std::process::{Child, Command};

use crate::platform::PlatformError;

/// A spawned process and every process it starts.
pub struct ProcessGroup {
    _private: (),
}

impl ProcessGroup {
    /// Spawns `cmd` inside a new group; processes it starts join the group.
    pub fn spawn(_cmd: &mut Command) -> Result<(ProcessGroup, Child), PlatformError> {
        Err(PlatformError::Unsupported)
    }

    /// Kills every process in the group.
    pub fn kill(&self) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported)
    }
}
