use crate::platform::PlatformError;

pub fn protect(_plain: &[u8]) -> Result<Vec<u8>, PlatformError> {
    Err(PlatformError::Unsupported)
}

pub fn unprotect(_blob: &[u8]) -> Result<Vec<u8>, PlatformError> {
    Err(PlatformError::Unsupported)
}
