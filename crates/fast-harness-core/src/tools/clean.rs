//! Command output made fit for the model: UTF-16 output decoded, binary
//! lines replaced by a short note, escape sequences taken out (`ansi`) and
//! progress-bar frames dropped (only the text after a line's last `\r`).

use super::ansi::strip_ansi;

/// Share of bad characters (control characters other than whitespace and
/// escapes, NUL, U+FFFD) above which a line counts as binary.
const BAD_SHARE: usize = 10;
/// Bad characters a line needs at least to count as binary.
const MIN_BAD: usize = 4;

/// `bytes` of shell or job output as clean text: binary runs become
/// `[binary output, N bytes omitted]`, escape codes go, and each line keeps
/// only its last `\r` frame.
pub(crate) fn clean_output(bytes: &[u8]) -> String {
    let text = match utf16(bytes) {
        Some(text) => text,
        None => without_binary(bytes),
    };
    last_frames(&strip_ansi(&text))
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
        if is_binary(&text) {
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

fn is_binary(line: &str) -> bool {
    let (mut total, mut bad) = (0usize, 0usize);
    for c in line.chars() {
        total += 1;
        let control = c.is_control() && !matches!(c, '\t' | '\n' | '\r' | '\u{1b}' | '\u{c}');
        if control || c == char::REPLACEMENT_CHARACTER {
            bad += 1;
        }
    }
    bad >= MIN_BAD && bad * BAD_SHARE > total
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

    #[test]
    fn binary_lines_become_a_note() {
        let mut bytes = b"header\n".to_vec();
        bytes.extend((0u8..=255).cycle().take(3000));
        bytes.extend(b"\ntail line\n");
        let out = clean_output(&bytes);
        assert!(out.starts_with("header\n[binary output, "), "{out}");
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
