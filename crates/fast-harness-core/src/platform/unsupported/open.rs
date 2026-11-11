use std::path::Path;

use crate::platform::PlatformError;

pub fn open_url(_url: &str) -> Result<(), PlatformError> {
    Err(PlatformError::Unsupported)
}

pub fn open_folder(_path: &Path) -> Result<(), PlatformError> {
    Err(PlatformError::Unsupported)
}
