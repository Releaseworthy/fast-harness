//! `read` of a text file too large to load whole: lines are streamed from
//! the start and reading stops at the end of the lines the output can use.

use std::io::BufRead;

use super::args::cut_chars;

/// Bytes kept of one line: more than `max_chars` characters of UTF-8.
fn keep_bytes(max_chars: usize) -> usize {
    max_chars.saturating_mul(4).saturating_add(4)
}

/// One line without its `\n` (and `\r`) into `line`, at most `keep` bytes
/// of it. `None` at the end of input, else whether the line was whole.
fn next_line<R: BufRead>(
    reader: &mut R,
    line: &mut Vec<u8>,
    keep: usize,
) -> std::io::Result<Option<bool>> {
    line.clear();
    let mut any = false;
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            return Ok(any.then_some(true));
        }
        any = true;
        let newline = buf.iter().position(|byte| *byte == b'\n');
        let chunk = &buf[..newline.unwrap_or(buf.len())];
        let room = keep.saturating_sub(line.len());
        line.extend_from_slice(&chunk[..room.min(chunk.len())]);
        let cut = chunk.len() > room;
        if let Some(at) = newline {
            reader.consume(at + 1);
            if !cut && line.last() == Some(&b'\r') {
                line.pop();
            }
            return Ok(Some(true));
        }
        let used = buf.len();
        reader.consume(used);
        if cut {
            // The line goes on: its rest is left unread.
            return Ok(Some(false));
        }
    }
}

/// Skips to the end of the current line without keeping it.
fn skip_line<R: BufRead>(reader: &mut R, line: &mut Vec<u8>) -> std::io::Result<bool> {
    let mut any = false;
    while let Some(whole) = next_line(reader, line, 0)? {
        any = true;
        if whole {
            break;
        }
    }
    Ok(any)
}

/// Lines `offset..` of `reader`, at most `limit` of them and `room` bytes
/// (at least one line), each cut to `max_chars`. Reading stops after the
/// last line shown; a note says so when input remains, since the total line
/// count of the file (`size` bytes) is then unknown.
pub(super) fn prefix_lines<R: BufRead>(
    mut reader: R,
    (offset, limit, room, max_chars): (u64, u64, usize, usize),
    size: u64,
) -> Result<String, String> {
    let io = |err: std::io::Error| format!("cannot read the file: {err}");
    let mut line = Vec::new();
    for skipped in 0..offset - 1 {
        if !skip_line(&mut reader, &mut line).map_err(io)? {
            return Err(format!(
                "offset {offset} is past the end of the file ({skipped} lines)"
            ));
        }
    }
    let (mut out, mut end, mut stop) = (String::new(), offset - 1, false);
    while end - (offset - 1) < limit && !stop {
        let Some(whole) = next_line(&mut reader, &mut line, keep_bytes(max_chars)).map_err(io)?
        else {
            break;
        };
        let text = String::from_utf8_lossy(&line);
        let entry = format!("{}: {}\n", end + 1, cut_chars(&text, max_chars));
        if end >= offset && out.len() + entry.len() > room {
            stop = true;
            break;
        }
        out.push_str(&entry);
        end += 1;
        stop = !whole;
    }
    let more = stop || !reader.fill_buf().map_err(io)?.is_empty();
    if end < offset {
        return Err(format!("offset {offset} is past the end of the file"));
    }
    if more {
        out.push_str(&format!(
            "[lines {offset} to {end}; the file has {size} bytes and was read only this far; continue with offset {}]\n",
            end + 1
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::io::{BufReader, Cursor, Read};

    use super::prefix_lines;

    struct Counting<'a, R>(R, &'a Cell<u64>);

    impl<R: Read> Read for Counting<'_, R> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.0.read(buf)?;
            self.1.set(self.1.get() + n as u64);
            Ok(n)
        }
    }

    fn lines(count: usize) -> Vec<u8> {
        (1..=count)
            .map(|n| format!("line {n}\n"))
            .collect::<String>()
            .into_bytes()
    }

    #[test]
    fn reads_only_up_to_the_lines_the_room_can_hold() {
        let data = lines(2_000_000); // about 25 MB
        let size = data.len() as u64;
        let read = Cell::new(0);
        let reader = BufReader::new(Counting(Cursor::new(data), &read));
        let out = prefix_lines(reader, (1, 2000, 4_000, 2000), size).unwrap();
        assert!(out.starts_with("1: line 1\n"), "{out}");
        assert!(out.contains("read only this far; continue with offset"));
        assert!(read.get() < 64 * 1024, "read {} bytes", read.get());
    }

    #[test]
    fn a_huge_line_is_cut_without_reading_past_it() {
        let mut data = vec![b'x'; 20_000_000];
        data.extend_from_slice(b"\nnext\n");
        let read = Cell::new(0);
        let reader = BufReader::new(Counting(Cursor::new(data), &read));
        let out = prefix_lines(reader, (1, 10, 100_000, 50), 0).unwrap();
        assert!(out.starts_with(&format!("1: {}... [line cut]\n", "x".repeat(50))));
        assert!(out.contains("continue with offset 2"), "{out}");
        assert!(read.get() < 64 * 1024, "read {} bytes", read.get());
    }

    #[test]
    fn the_end_of_input_gives_no_note_and_crlf_is_dropped() {
        let reader = BufReader::new(Cursor::new(b"a\r\nb\r\nc".to_vec()));
        let out = prefix_lines(reader, (2, 10, 1_000, 2000), 7).unwrap();
        assert_eq!(out, "2: b\n3: c\n");
        let reader = BufReader::new(Cursor::new(b"a\n".to_vec()));
        let err = prefix_lines(reader, (3, 10, 1_000, 2000), 2).unwrap_err();
        assert!(err.contains("past the end"), "{err}");
    }
}
