//! The text encodings `read`, `edit` and `write` handle: UTF-8 with or
//! without a BOM and UTF-16 (LE or BE with a BOM, LE without one when its
//! zero bytes say so). Text is shown without the BOM; a write puts the
//! file's own encoding and BOM back.

use super::args::looks_binary;

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// How a text file is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Encoding {
    Utf8,
    Utf8Bom,
    Utf16Le { bom: bool },
    Utf16Be,
}

impl Encoding {
    /// A short name for result lines ("UTF-16LE"); `None` for plain UTF-8.
    pub(crate) fn label(self) -> Option<&'static str> {
        match self {
            Encoding::Utf8 => None,
            Encoding::Utf8Bom => Some("UTF-8 with BOM"),
            Encoding::Utf16Le { bom: true } => Some("UTF-16LE"),
            Encoding::Utf16Le { bom: false } => Some("UTF-16LE without BOM"),
            Encoding::Utf16Be => Some("UTF-16BE"),
        }
    }

    pub(crate) fn is_utf16(self) -> bool {
        matches!(self, Encoding::Utf16Le { .. } | Encoding::Utf16Be)
    }
}

/// The encoding of a file starting with `head`; `None` for a binary file
/// (a NUL byte in a file that is not UTF-16).
pub(crate) fn detect(head: &[u8]) -> Option<Encoding> {
    if head.starts_with(UTF8_BOM) {
        return Some(Encoding::Utf8Bom);
    }
    if head.starts_with(&[0xFF, 0xFE]) {
        return Some(Encoding::Utf16Le { bom: true });
    }
    if head.starts_with(&[0xFE, 0xFF]) {
        return Some(Encoding::Utf16Be);
    }
    if utf16le_without_bom(head) {
        return Some(Encoding::Utf16Le { bom: false });
    }
    (!looks_binary(head)).then_some(Encoding::Utf8)
}

/// Mostly ASCII text in UTF-16LE: every odd byte of the first 8 KB is NUL
/// for at least 90% of the pairs and no pair is two NULs.
fn utf16le_without_bom(head: &[u8]) -> bool {
    let (pairs, _) = head[..head.len().min(8 * 1024)].as_chunks::<2>();
    if pairs.len() < 2 {
        return false;
    }
    let mut ascii = 0;
    for pair in pairs {
        match pair {
            [0, 0] => return false,
            [_, 0] => ascii += 1,
            _ => {}
        }
    }
    ascii * 10 >= pairs.len() * 9
}

/// The UTF-16 code units of `body` (a trailing odd byte is left out).
fn units(body: &[u8], le: bool) -> Vec<u16> {
    let (pairs, _) = body.as_chunks::<2>();
    let unit = |pair: &[u8; 2]| match le {
        true => u16::from_le_bytes(*pair),
        false => u16::from_be_bytes(*pair),
    };
    pairs.iter().map(unit).collect()
}

/// The BOM bytes `encoding` starts a file with.
fn bom(encoding: Encoding) -> &'static [u8] {
    match encoding {
        Encoding::Utf8 | Encoding::Utf16Le { bom: false } => b"",
        Encoding::Utf8Bom => UTF8_BOM,
        Encoding::Utf16Le { bom: true } => &[0xFF, 0xFE],
        Encoding::Utf16Be => &[0xFE, 0xFF],
    }
}

/// The text of `bytes` without its BOM; `Err` when it does not decode
/// exactly (invalid UTF-8, an odd UTF-16 length or a lone surrogate).
pub(crate) fn decode(bytes: &[u8], encoding: Encoding) -> Result<String, String> {
    let body = &bytes[bom(encoding).len().min(bytes.len())..];
    if encoding.is_utf16() && !body.len().is_multiple_of(2) {
        return Err("odd byte count for UTF-16".into());
    }
    let utf16 = |le: bool| String::from_utf16(&units(body, le)).map_err(|e| e.to_string());
    match encoding {
        Encoding::Utf8 | Encoding::Utf8Bom => {
            String::from_utf8(body.to_vec()).map_err(|_| "invalid UTF-8".to_string())
        }
        Encoding::Utf16Le { .. } => utf16(true),
        Encoding::Utf16Be => utf16(false),
    }
}

/// [`decode`] that never fails: bad bytes become U+FFFD (for `read`).
pub(crate) fn decode_lossy(bytes: &[u8], encoding: Encoding) -> String {
    let body = &bytes[bom(encoding).len().min(bytes.len())..];
    match encoding {
        Encoding::Utf8 | Encoding::Utf8Bom => String::from_utf8_lossy(body).into_owned(),
        Encoding::Utf16Le { .. } => String::from_utf16_lossy(&units(body, true)),
        Encoding::Utf16Be => String::from_utf16_lossy(&units(body, false)),
    }
}

/// `text` stored as `encoding`, its BOM first.
pub(crate) fn encode(text: &str, encoding: Encoding) -> Vec<u8> {
    let mut out = bom(encoding).to_vec();
    match encoding {
        Encoding::Utf8 | Encoding::Utf8Bom => out.extend_from_slice(text.as_bytes()),
        Encoding::Utf16Le { .. } => text
            .encode_utf16()
            .for_each(|u| out.extend_from_slice(&u.to_le_bytes())),
        Encoding::Utf16Be => text
            .encode_utf16()
            .for_each(|u| out.extend_from_slice(&u.to_be_bytes())),
    }
    out
}

/// A text file for `edit`: its bytes, encoding and exact text.
pub(crate) fn read_text_file(
    path_arg: &str,
    path: &std::path::Path,
) -> Result<(Vec<u8>, Encoding, String), String> {
    let bytes = std::fs::read(path).map_err(|err| format!("cannot read {path_arg}: {err}"))?;
    let encoding = detect(&bytes)
        .ok_or_else(|| format!("{path_arg} looks like a binary file; edit refused"))?;
    let text = decode(&bytes, encoding).map_err(|err| {
        let name = encoding.label().unwrap_or("UTF-8");
        format!("{path_arg} is not valid {name} text ({err}); edit refused")
    })?;
    Ok((bytes, encoding, text))
}

/// `content` with CRLF line ends when the file it replaces (`old`) used
/// them and `content` has none of its own.
pub(crate) fn keep_line_ends(old: &str, content: &str) -> Option<String> {
    let crlf = old.contains("\r\n") && !content.contains('\r') && content.contains('\n');
    crlf.then(|| content.replace('\n', "\r\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le(text: &str, with_bom: bool) -> Vec<u8> {
        encode(text, Encoding::Utf16Le { bom: with_bom })
    }

    #[test]
    fn detects_boms_utf16_without_bom_and_binary() {
        assert_eq!(detect(b"\xEF\xBB\xBFhi"), Some(Encoding::Utf8Bom));
        assert_eq!(
            detect(&le("hi\r\n", true)),
            Some(Encoding::Utf16Le { bom: true })
        );
        let be = encode("hi", Encoding::Utf16Be);
        assert_eq!(&be[..2], [0xFE, 0xFF]);
        assert_eq!(detect(&be), Some(Encoding::Utf16Be));
        let plain = le("key=value\r\nother=1\r\n", false);
        assert_eq!(detect(&plain), Some(Encoding::Utf16Le { bom: false }));
        assert_eq!(detect(b"MZ\x90\x00\x03\x00\x00\x00\x04\x00"), None);
        assert_eq!(detect(b"plain"), Some(Encoding::Utf8));
    }

    #[test]
    fn decode_and_encode_round_trip_every_encoding() {
        let text = "Xin chào\r\nđ€😀\r\n";
        for encoding in [
            Encoding::Utf8,
            Encoding::Utf8Bom,
            Encoding::Utf16Le { bom: true },
            Encoding::Utf16Le { bom: false },
            Encoding::Utf16Be,
        ] {
            let bytes = encode(text, encoding);
            assert_eq!(decode(&bytes, encoding).unwrap(), text, "{encoding:?}");
        }
        assert!(decode(b"\xFF\xFEa", Encoding::Utf16Le { bom: true }).is_err());
        assert_eq!(
            decode_lossy(b"\xFF\xFE\x00\xD8", Encoding::Utf16Le { bom: true }),
            "\u{FFFD}"
        );
    }
}
