//! Command output made fit for the model: UTF-16 output decoded, binary
//! lines replaced by a short note, escape sequences taken out (`ansi`) and
//! progress-bar frames dropped (only the text after a line's last `\r`).

use super::ansi::strip_ansi;

/// Share of bad characters (control characters other than whitespace and
/// escapes, NUL, U+FFFD) above which a line counts as binary.
const BAD_SHARE: usize = 10;
/// Bad characters a line needs at least to count as binary.
const MIN_BAD: usize = 4;
/// Bad characters a whole output needs at least to count as binary: a few
/// odd characters in ordinary text keep it text.
const MIN_BAD_WHOLE: usize = 16;
/// U+FFFD in UTF-8: what pwsh prints for each byte it could not decode.
const REPLACEMENT_UTF8: [u8; 3] = [0xef, 0xbf, 0xbd];

/// `bytes` of shell or job output as clean text: an output that is binary
/// as a whole becomes one line `[binary output, N bytes]` (D89), binary
/// runs in text become `[binary output, N bytes omitted]`, escape codes
/// go, and each line keeps only its last `\r` frame.
pub(crate) fn clean_output(bytes: &[u8]) -> String {
    let decoded = utf16(bytes);
    let whole = match &decoded {
        Some(text) => is_binary(text, MIN_BAD_WHOLE),
        None => is_binary(&String::from_utf8_lossy(bytes), MIN_BAD_WHOLE),
    };
    if whole {
        return format!("[binary output, {} bytes]\n", source_bytes(bytes));
    }
    let text = match decoded {
        Some(text) => text,
        None => without_binary(bytes),
    };
    last_frames(&strip_ansi(&text))
}

/// The size of what the command printed before a lossy decode: pwsh
/// writes each byte it could not decode as U+FFFD, three bytes in UTF-8,
/// so each of those counts as one byte.
fn source_bytes(bytes: &[u8]) -> usize {
    let replaced = bytes.windows(3).filter(|w| *w == REPLACEMENT_UTF8).count();
    bytes.len() - 2 * replaced
}

/// The text of `bytes` when they look like UTF-16LE (a NUL after most
/// characters, none before them), as some Windows programs write.
fn utf16(bytes: &[u8]) -> Option<String> {
    let pairs = bytes.len() / 2;
    if pairs < 2 || !bytes.len().is_multiple_of(2) {
        return None;
    }
    let odd = bytes.iter().skip(1).step_by(2).filter(|b| **b == 0).count();
    let even = bytes.iter().step_by(2).filter(|b| **b == 0).count();
    if odd * 10 < pairs * 9 || even * 20 > pairs {
        return None;
    }
    let units = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u16::from_le_bytes(*p));
    let text: String = char::decode_utf16(units)
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect();
    Some(text.strip_prefix('\u{feff}').unwrap_or(&text).to_string())
}

/// `bytes` as text, each run of binary-looking lines replaced by a note.
fn without_binary(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    let mut omitted = 0usize;
    for line in bytes.split_inclusive(|b| *b == b'\n') {
        let text = String::from_utf8_lossy(line);
        if is_binary(&text, MIN_BAD) {
            omitted += line.len();
            continue;
        }
        note(&mut out, &mut omitted);
        out.push_str(&text);
    }
    note(&mut out, &mut omitted);
    out
}

/// Adds the note for `omitted` binary bytes, if any, on its own line.
fn note(out: &mut String, omitted: &mut usize) {
    if *omitted == 0 {
        return;
    }
    out.push_str(&format!("[binary output, {omitted} bytes omitted]\n"));
    *omitted = 0;
}

/// More than a tenth of `text` is bad characters, and at least `min_bad`.
fn is_binary(text: &str, min_bad: usize) -> bool {
    let (mut total, mut bad) = (0usize, 0usize);
    for c in text.chars() {
        total += 1;
        let control = c.is_control() && !matches!(c, '\t' | '\n' | '\r' | '\u{1b}' | '\u{c}');
        if control || c == char::REPLACEMENT_CHARACTER {
            bad += 1;
        }
    }
    bad >= min_bad && bad * BAD_SHARE > total
}

/// Each line cut to the text after its last `\r` (a terminal redraws the
/// line there): `\r\n` is a line break and a `\r` ending a line is no frame.
fn last_frames(text: &str) -> String {
    if !text.contains('\r') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let (body, newline) = match line.strip_suffix('\n') {
            Some(body) => (body, "\n"),
            None => (line, ""),
        };
        let body = body.trim_end_matches('\r');
        let frame = body.rsplit('\r').next().unwrap_or(body);
        out.push_str(frame);
        out.push_str(newline);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::clean_output;

    #[test]
    fn progress_frames_keep_only_the_last() {
        let out = "start\r\n 10%\r 50%\r100%\ndone\r\nlast\r";
        assert_eq!(clean_output(out.as_bytes()), "start\n100%\ndone\nlast");
        let pip = "\u{1b}[32m  1/3\u{1b}[0m\r  3/3 ok\r\n";
        assert_eq!(clean_output(pip.as_bytes()), "  3/3 ok\n");
    }

    /// A 100 000-byte file in pseudo-random bytes, as pwsh `Get-Content
    /// -Raw` prints it: each byte that is not ASCII became U+FFFD (3 bytes).
    fn pwsh_printed_binary() -> Vec<u8> {
        let mut seed = 0x2545_f491_u32;
        let mut out = Vec::new();
        for _ in 0..100_000 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            match (seed >> 24) as u8 {
                byte @ 0..=0x7f => out.push(byte),
                _ => out.extend("\u{fffd}".as_bytes()),
            }
        }
        out
    }

    /// Owner, 2026-11-15: short garbage lines leaked between the notes and
    /// the count was of the re-encoded bytes (100 KB read as ~190 KB).
    #[test]
    fn a_binary_output_is_one_line_with_the_original_size() {
        let printed = pwsh_printed_binary();
        assert!(printed.len() > 180_000, "{}", printed.len());
        assert_eq!(clean_output(&printed), "[binary output, 100000 bytes]\n");
        let mut raw = b"MZ\x90\0\x03\0\0\0".repeat(500);
        raw.extend(b"\nThis program cannot be run in DOS mode.\r\n");
        let out = clean_output(&raw);
        assert_eq!(out, format!("[binary output, {} bytes]\n", raw.len()));
        let ansi = "\u{1b}[31merror\u{1b}[0m: bell \u{7} and \u{fffd} here\nok\n".repeat(3);
        assert_eq!(
            clean_output(ansi.as_bytes()),
            ansi.replace("\u{1b}[31m", "").replace("\u{1b}[0m", "")
        );
    }

    #[test]
    fn binary_lines_become_a_note() {
        let mut bytes = "header line of ordinary text\n".repeat(200).into_bytes();
        bytes.extend((0u8..=255).cycle().take(300));
        bytes.extend(b"\ntail line\n");
        let out = clean_output(&bytes);
        assert!(out.contains("text\n[binary output, "), "{out}");
        assert!(out.ends_with(" bytes omitted]\ntail line\n"), "{out}");
        assert!(!out.contains('\u{fffd}') && !out.contains('\0'), "{out}");
        let text = "naïve café ✓ tab\there\n";
        assert_eq!(clean_output(text.as_bytes()), text);
        assert_eq!(clean_output(b"one \xff byte\n"), "one \u{fffd} byte\n");
    }

    #[test]
    fn utf16_output_is_decoded() {
        let bytes: Vec<u8> = "Ubuntu\r\nDebian\r\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(clean_output(&bytes), "Ubuntu\nDebian\n");
    }
}
