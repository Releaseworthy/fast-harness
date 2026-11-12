//! The OS CSPRNG off Windows: `/dev/urandom` on Unix (std only), `Unsupported` elsewhere.

use crate::platform::PlatformError;

/// Fills `buf` with cryptographically secure random bytes read from `/dev/urandom`.
#[cfg(unix)]
pub fn random_fill(buf: &mut [u8]) -> Result<(), PlatformError> {
    use std::io::Read;
    if buf.is_empty() {
        return Ok(());
    }
    let os_error = |err: std::io::Error| PlatformError::Os {
        code: err.raw_os_error().unwrap_or(0),
        message: format!("reading /dev/urandom failed: {err}"),
    };
    let mut file = std::fs::File::open("/dev/urandom").map_err(os_error)?;
    file.read_exact(buf).map_err(os_error)
}

#[cfg(not(unix))]
pub fn random_fill(_buf: &mut [u8]) -> Result<(), PlatformError> {
    Err(PlatformError::Unsupported)
}

#[cfg(all(test, unix))]
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
