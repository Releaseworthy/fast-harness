//! The OS CSPRNG (BCryptGenRandom).

use crate::platform::PlatformError;
use windows_sys::Win32::Security::Cryptography::{
    BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
};

/// Fills `buf` with cryptographically secure random bytes.
pub fn random_fill(buf: &mut [u8]) -> Result<(), PlatformError> {
    // BCryptGenRandom takes a `u32` length, so large buffers go in pieces.
    for chunk in buf.chunks_mut(u32::MAX as usize) {
        // Fits: chunks are at most u32::MAX bytes.
        let len = chunk.len() as u32;
        // SAFETY: `chunk` is a valid writable buffer of exactly `len` bytes for
        // the call; a null algorithm handle is what
        // BCRYPT_USE_SYSTEM_PREFERRED_RNG requires.
        let status = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                chunk.as_mut_ptr(),
                len,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        if status < 0 {
            return Err(PlatformError::Os {
                code: status,
                message: format!("BCryptGenRandom failed (NTSTATUS {status:#010x})"),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_buffers_differ() {
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        random_fill(&mut a).unwrap();
        random_fill(&mut b).unwrap();
        assert_ne!(a, b);
        assert_ne!(a, [0u8; 32]);
    }

    #[test]
    fn empty_buffer_is_ok() {
        random_fill(&mut []).unwrap();
    }
}
