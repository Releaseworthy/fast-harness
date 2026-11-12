//! base64 (standard and URL-safe), SHA-256, hex, PKCE material and random tokens.

use crate::platform::{self, PlatformError};
use std::fmt;

const STANDARD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const URL_SAFE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Why a base64 string could not be decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Base64Error {
    /// A character outside the alphabet (or misplaced padding), at this byte offset.
    InvalidChar(usize),
    /// The length cannot be a valid encoding.
    InvalidLength,
    /// The last character carries bits that an encoder never sets.
    NonCanonical,
}

impl fmt::Display for Base64Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Base64Error::InvalidChar(at) => write!(f, "invalid base64 character at {at}"),
            Base64Error::InvalidLength => f.write_str("invalid base64 length"),
            Base64Error::NonCanonical => f.write_str("non-canonical base64 tail"),
        }
    }
}

impl std::error::Error for Base64Error {}

/// Standard base64 with `=` padding (RFC 4648 section 4).
pub fn base64_encode(data: &[u8]) -> String {
    encode_with(data, STANDARD, true)
}

/// Decodes standard base64; padding is required.
pub fn base64_decode(text: &str) -> Result<Vec<u8>, Base64Error> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return Err(Base64Error::InvalidLength);
    }
    let pad = bytes
        .iter()
        .rev()
        .take(2)
        .take_while(|b| **b == b'=')
        .count();
    decode_with(&bytes[..bytes.len() - pad], STANDARD)
}

/// URL-safe base64 without padding (RFC 4648 section 5, as PKCE uses it).
pub fn base64url_encode(data: &[u8]) -> String {
    encode_with(data, URL_SAFE, false)
}

/// Decodes URL-safe base64 without padding.
pub fn base64url_decode(text: &str) -> Result<Vec<u8>, Base64Error> {
    decode_with(text.as_bytes(), URL_SAFE)
}

fn encode_with(data: &[u8], table: &[u8; 64], pad: bool) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let chars = chunk.len() + 1;
        for i in 0..4 {
            if i < chars {
                out.push(char::from(table[((n >> (18 - 6 * i)) & 63) as usize]));
            } else if pad {
                out.push('=');
            }
        }
    }
    out
}

fn decode_with(bytes: &[u8], table: &[u8; 64]) -> Result<Vec<u8>, Base64Error> {
    if bytes.len() % 4 == 1 {
        return Err(Base64Error::InvalidLength);
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3 + 2);
    for (index, chunk) in bytes.chunks(4).enumerate() {
        let mut n = 0u32;
        for (i, byte) in chunk.iter().enumerate() {
            let value = table
                .iter()
                .position(|c| c == byte)
                .ok_or(Base64Error::InvalidChar(index * 4 + i))?;
            n |= (value as u32) << (18 - 6 * i);
        }
        let len = chunk.len() - 1;
        let unused = n & (0x00ff_ffff >> (8 * len));
        if unused != 0 {
            return Err(Base64Error::NonCanonical);
        }
        out.extend_from_slice(&n.to_be_bytes()[1..1 + len]);
    }
    Ok(out)
}

/// Lowercase hex of `data`.
pub fn hex(data: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(data.len() * 2);
    for byte in data {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    out
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// SHA-256 digest of `data` (FIPS 180-4).
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut message = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        // Pad with zeros up to 8 bytes before a block boundary.
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());
    // The padding above makes the length a multiple of 64: no remainder.
    for block in message.as_chunks::<64>().0 {
        compress(&mut state, block);
    }
    let mut out = [0u8; 32];
    for (slot, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(state) {
        *slot = word.to_be_bytes();
    }
    out
}

fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for (i, word) in block.as_chunks::<4>().0.iter().enumerate() {
        w[i] = u32::from_be_bytes(*word);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for (k, wi) in K.iter().zip(w) {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ (!e & g);
        let t1 = h
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(*k)
            .wrapping_add(wi);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let t2 = s0.wrapping_add(maj);
        (h, g, f, e, d, c, b, a) = (g, f, e, d.wrapping_add(t1), c, b, a, t1.wrapping_add(t2));
    }
    for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *slot = slot.wrapping_add(value);
    }
}

/// A PKCE pair `(verifier, challenge)` for the S256 method (RFC 7636): the
/// verifier is 32 random bytes as base64url, the challenge base64url(sha256(verifier)).
pub fn pkce_pair() -> Result<(String, String), PlatformError> {
    let verifier = random_token(32)?;
    let challenge = base64url_encode(&sha256(verifier.as_bytes()));
    Ok((verifier, challenge))
}

/// `bytes` random bytes from the OS CSPRNG as base64url (for `state` and similar).
pub fn random_token(bytes: usize) -> Result<String, PlatformError> {
    let mut buf = vec![0u8; bytes];
    platform::random_fill(&mut buf)?;
    Ok(base64url_encode(&buf))
}
