//! The live tail of a running tool's output (PLAN.md D58): the last lines of
//! its `ToolProgress` chunks, cleaned for a view. Its memory is bounded by
//! [`LIVE_LINES`] lines of at most [`LIVE_LINE_BYTES`] bytes, whatever the
//! tool prints; the `ToolResult` drops it.

/// Lines a view shows under a running tool.
pub const LIVE_LINES: usize = 8;
/// Bytes kept of one line (whole characters); the rest of a longer line is
/// dropped. With [`LIVE_LINES`] a tail shows at most 2 KB.
pub const LIVE_LINE_BYTES: usize = 256;

/// Where the cleaner is inside an escape sequence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Escape {
    #[default]
    None,
    /// After ESC.
    Start,
    /// A control sequence (`ESC [`), up to its final byte.
    Csi,
    /// A string (`ESC ]`, `ESC P`, ...), up to BEL or `ESC \`.
    Text,
    /// ESC inside a string.
    TextEsc,
}

/// The newest lines of a running tool's output. `\n` and `\r\n` end a
/// line, a lone `\r` starts the current line again (progress bars), escape
/// sequences and other control characters are dropped, and blank lines are
/// skipped.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LiveTail {
    /// Finished lines, oldest first; at most [`LIVE_LINES`].
    done: Vec<String>,
    /// The line being written.
    current: String,
    escape: Escape,
    /// The last character was `\r`: a `\n` next ends the line, anything
    /// else starts it again.
    carriage: bool,
}

impl LiveTail {
    /// Adds one chunk of output.
    pub fn push(&mut self, chunk: &str) {
        for ch in chunk.chars() {
            self.push_char(ch);
        }
    }

    /// The last [`LIVE_LINES`] lines, the line being written included.
    pub fn lines(&self) -> impl Iterator<Item = &str> {
        let current = (!self.current.trim().is_empty()).then_some(self.current.as_str());
        let all = self.done.iter().map(String::as_str).chain(current);
        let count = self.done.len() + usize::from(current.is_some());
        all.skip(count.saturating_sub(LIVE_LINES))
    }

    /// [`LiveTail::lines`] joined by `\n`.
    pub fn text(&self) -> String {
        self.lines().collect::<Vec<_>>().join("\n")
    }

    pub fn is_empty(&self) -> bool {
        self.lines().next().is_none()
    }

    fn push_char(&mut self, ch: char) {
        match self.escape {
            Escape::None => {}
            Escape::Start => {
                self.escape = match ch {
                    '[' => Escape::Csi,
                    ']' | 'P' | 'X' | '^' | '_' => Escape::Text,
                    '\u{1b}' => Escape::Start,
                    _ => Escape::None,
                };
                return;
            }
            Escape::Csi => {
                if ('\u{40}'..='\u{7e}').contains(&ch) {
                    self.escape = Escape::None;
                } else if ch == '\u{1b}' {
                    self.escape = Escape::Start;
                }
                return;
            }
            Escape::Text => {
                self.escape = match ch {
                    '\u{7}' => Escape::None,
                    '\u{1b}' => Escape::TextEsc,
                    _ => Escape::Text,
                };
                return;
            }
            Escape::TextEsc => {
                self.escape = Escape::None;
                return;
            }
        }
        self.plain(ch);
    }

    fn plain(&mut self, ch: char) {
        let carriage = std::mem::take(&mut self.carriage);
        match ch {
            '\n' => return self.end_line(),
            _ if carriage => self.current.clear(),
            _ => {}
        }
        match ch {
            '\r' => self.carriage = true,
            '\u{1b}' => self.escape = Escape::Start,
            '\t' => self.add(' '),
            c if c.is_control() => {}
            c => self.add(c),
        }
    }

    fn add(&mut self, ch: char) {
        if self.current.len() + ch.len_utf8() <= LIVE_LINE_BYTES {
            self.current.push(ch);
        }
    }

    /// Moves the current line to the finished ones, reusing the oldest
    /// line's buffer once [`LIVE_LINES`] are kept.
    fn end_line(&mut self) {
        if self.current.trim().is_empty() {
            self.current.clear();
            return;
        }
        let spare = if self.done.len() == LIVE_LINES {
            let mut oldest = self.done.remove(0);
            oldest.clear();
            oldest
        } else {
            String::new()
        };
        let line = std::mem::replace(&mut self.current, spare);
        self.done.push(line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tail(chunks: &[&str]) -> Vec<String> {
        let mut tail = LiveTail::default();
        for chunk in chunks {
            tail.push(chunk);
        }
        tail.lines().map(str::to_string).collect()
    }

    #[test]
    fn line_ends_and_restarts() {
        assert_eq!(tail(&["one", "\r\n", "two"]), ["one", "two"]);
        assert_eq!(tail(&["a\r", "\nb\n\n  \nc"]), ["a", "b", "c"]);
        assert_eq!(tail(&["10%\r20%\r", "30%"]), ["30%"]);
        assert_eq!(tail(&["x\ty\u{7}\u{0}z"]), ["x yz"]);
    }

    #[test]
    fn escape_sequences_are_dropped_across_chunks() {
        let chunks = ["\u{1b}[3", "1mred\u{1b}[0m ok", "\u{1b}]0;title\u{7}!"];
        assert_eq!(tail(&chunks), ["red ok!"]);
        assert_eq!(tail(&["\u{1b}]8;;u\u{1b}\\link\u{1b}=."]), ["link."]);
    }

    #[test]
    fn only_the_newest_lines_and_bytes_are_kept() {
        let many: String = (0..100).map(|i| format!("line {i}\n")).collect();
        let lines = tail(&[&many, "last"]);
        assert_eq!(lines.len(), LIVE_LINES);
        assert_eq!(lines.first().map(String::as_str), Some("line 93"));
        assert_eq!(lines.last().map(String::as_str), Some("last"));
        let long = "é".repeat(LIVE_LINE_BYTES);
        let kept = tail(&[&long]);
        assert_eq!(kept[0].len(), LIVE_LINE_BYTES);
        assert_eq!(kept[0].chars().count(), LIVE_LINE_BYTES / 2);
    }
}
