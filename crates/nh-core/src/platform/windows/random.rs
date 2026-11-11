//! The OS CSPRNG (BCryptGenRandom).

use crate::platform::PlatformError;

/// Fills `buf` with cryptographically secure random bytes.
pub fn random_fill(_buf: &mut [u8]) -> Result<(), PlatformError> {
    Err(PlatformError::Unsupported)
}
