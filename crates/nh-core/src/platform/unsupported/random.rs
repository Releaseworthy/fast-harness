use crate::platform::PlatformError;

pub fn random_fill(_buf: &mut [u8]) -> Result<(), PlatformError> {
    Err(PlatformError::Unsupported)
}
