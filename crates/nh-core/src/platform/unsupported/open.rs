use crate::platform::PlatformError;

pub fn open_url(_url: &str) -> Result<(), PlatformError> {
    Err(PlatformError::Unsupported)
}
