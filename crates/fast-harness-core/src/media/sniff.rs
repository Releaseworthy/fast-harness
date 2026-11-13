//! The type and size of an image from its first bytes: PNG, JPEG, GIF and
//! WebP, read by hand (no image crate, PLAN.md D70).

/// The media type the bytes start with, if it is one of the four.
pub fn media_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// The file name extension of a media type.
pub fn extension(media_type: &str) -> &'static str {
    match media_type {
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "png",
    }
}

/// Width and height in pixels, `None` when the header cannot be read.
pub fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let size = match media_type(bytes)? {
        "image/png" => png(bytes),
        "image/gif" => gif(bytes),
        "image/webp" => webp(bytes),
        _ => jpeg(bytes),
    }?;
    (size.0 > 0 && size.1 > 0).then_some(size)
}

fn be16(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 2)?;
    Some(u32::from(b[0]) << 8 | u32::from(b[1]))
}

fn le16(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 2)?;
    Some(u32::from(b[1]) << 8 | u32::from(b[0]))
}

fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 4)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn le24(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 3)?;
    Some(u32::from(b[2]) << 16 | u32::from(b[1]) << 8 | u32::from(b[0]))
}

/// IHDR is the first chunk: width and height at bytes 16 and 20.
fn png(bytes: &[u8]) -> Option<(u32, u32)> {
    (bytes.get(12..16)? == b"IHDR").then_some(())?;
    Some((be32(bytes, 16)?, be32(bytes, 20)?))
}

/// The logical screen size after the signature.
fn gif(bytes: &[u8]) -> Option<(u32, u32)> {
    Some((le16(bytes, 6)?, le16(bytes, 8)?))
}

/// `VP8 ` (lossy), `VP8L` (lossless) or `VP8X` (extended) at byte 12.
fn webp(bytes: &[u8]) -> Option<(u32, u32)> {
    match bytes.get(12..16)? {
        b"VP8 " => {
            (bytes.get(23..26)? == [0x9D, 0x01, 0x2A]).then_some(())?;
            Some((le16(bytes, 26)? & 0x3FFF, le16(bytes, 28)? & 0x3FFF))
        }
        b"VP8L" => {
            (*bytes.get(20)? == 0x2F).then_some(())?;
            let b = bytes.get(21..25)?;
            let bits = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            Some(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1))
        }
        b"VP8X" => Some((le24(bytes, 24)? + 1, le24(bytes, 27)? + 1)),
        _ => None,
    }
}

/// Walks the markers to the first start-of-frame (SOF0 to SOF15 but DHT,
/// JPG and DAC): height then width.
fn jpeg(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    loop {
        while *bytes.get(at)? != 0xFF {
            at += 1;
        }
        while *bytes.get(at)? == 0xFF {
            at += 1;
        }
        let marker = *bytes.get(at)?;
        at += 1;
        if matches!(marker, 0xD0..=0xD9 | 0x01) {
            continue;
        }
        let length = be16(bytes, at)? as usize;
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            return Some((be16(bytes, at + 5)?, be16(bytes, at + 3)?));
        }
        at += length.max(2);
    }
}

#[cfg(test)]
#[path = "sniff_tests.rs"]
mod tests;
