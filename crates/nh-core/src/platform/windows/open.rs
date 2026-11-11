//! Opening a URL in the default browser (ShellExecuteW).

use crate::platform::PlatformError;

/// Opens an `https://` or `http://` URL in the user's default browser.
pub fn open_url(_url: &str) -> Result<(), PlatformError> {
    Err(PlatformError::Unsupported)
}
