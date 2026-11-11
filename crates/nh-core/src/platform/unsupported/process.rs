use std::process::{Child, Command};

use crate::platform::PlatformError;

pub struct ProcessGroup {
    _private: (),
}

impl ProcessGroup {
    pub fn spawn(_cmd: &mut Command) -> Result<(ProcessGroup, Child), PlatformError> {
        Err(PlatformError::Unsupported)
    }

    pub fn kill(&self) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported)
    }
}
