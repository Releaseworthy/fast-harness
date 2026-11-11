use crate::platform::PlatformError;
use std::path::PathBuf;

/// No folder dialog outside Windows yet.
pub fn pick_folder(_owner: isize, _title: &str) -> Result<Option<PathBuf>, PlatformError> {
    Err(PlatformError::Unsupported)
}
