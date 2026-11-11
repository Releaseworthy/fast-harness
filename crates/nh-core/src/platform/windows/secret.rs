//! DPAPI: data only the current Windows user can decrypt.

use crate::platform::PlatformError;

/// Encrypts `plain` for the current user (CryptProtectData).
pub fn protect(_plain: &[u8]) -> Result<Vec<u8>, PlatformError> {
    Err(PlatformError::Unsupported)
}

/// Decrypts a blob made by [`protect`] (CryptUnprotectData).
pub fn unprotect(_blob: &[u8]) -> Result<Vec<u8>, PlatformError> {
    Err(PlatformError::Unsupported)
}
