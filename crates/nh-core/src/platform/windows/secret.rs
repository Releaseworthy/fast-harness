//! DPAPI: data only the current Windows user can decrypt.

use crate::platform::PlatformError;
use std::ptr;
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};

/// Encrypts `plain` for the current user (CryptProtectData).
pub fn protect(plain: &[u8]) -> Result<Vec<u8>, PlatformError> {
    let input = input_blob(plain)?;
    let mut output = empty_blob();
    // SAFETY: `input` points into `plain`, which outlives the call and is only
    // read; every optional pointer is null, which the API allows; `output` is a
    // valid out pointer that the API fills with a LocalAlloc buffer on success.
    let ok = unsafe {
        CryptProtectData(
            &input,
            ptr::null(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        return Err(PlatformError::last_os_error());
    }
    Ok(take_blob(output))
}

/// Decrypts a blob made by [`protect`] (CryptUnprotectData).
pub fn unprotect(blob: &[u8]) -> Result<Vec<u8>, PlatformError> {
    let input = input_blob(blob)?;
    let mut output = empty_blob();
    // SAFETY: same contract as in `protect`: `input` borrows `blob` for the
    // call, the description out pointer and every optional pointer are null, and
    // `output` receives a LocalAlloc buffer on success.
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            ptr::null_mut(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        return Err(PlatformError::last_os_error());
    }
    Ok(take_blob(output))
}

/// A read-only blob over `data`. DPAPI takes the length as a `u32`.
fn input_blob(data: &[u8]) -> Result<CRYPT_INTEGER_BLOB, PlatformError> {
    let len = u32::try_from(data.len()).map_err(|_| PlatformError::Os {
        code: 0,
        message: "data too large for DPAPI".into(),
    })?;
    Ok(CRYPT_INTEGER_BLOB {
        cbData: len,
        // The API never writes through the input pointer; the cast only fits its type.
        pbData: data.as_ptr() as *mut u8,
    })
}

fn empty_blob() -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    }
}

/// Copies an output blob into a `Vec` and frees the OS buffer.
fn take_blob(blob: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    if blob.pbData.is_null() {
        return Vec::new();
    }
    // SAFETY: on success DPAPI sets `pbData` to a buffer of `cbData` bytes that
    // stays valid until it is freed below and is not aliased elsewhere.
    let data = unsafe { std::slice::from_raw_parts(blob.pbData, blob.cbData as usize) }.to_vec();
    // SAFETY: `pbData` was allocated by DPAPI with LocalAlloc and is freed once.
    unsafe {
        LocalFree(blob.pbData.cast());
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let plain = b"test-access-1 round trip";
        let blob = protect(plain).unwrap();
        assert_ne!(blob.as_slice(), plain.as_slice());
        assert_eq!(unprotect(&blob).unwrap(), plain);
    }

    #[test]
    fn empty_round_trip() {
        let blob = protect(b"").unwrap();
        assert_eq!(unprotect(&blob).unwrap(), b"");
    }

    #[test]
    fn tampered_blob_fails() {
        let mut blob = protect(b"test-access-2").unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 0x55;
        assert!(unprotect(&blob).is_err());
        assert!(unprotect(b"not a dpapi blob").is_err());
    }
}
