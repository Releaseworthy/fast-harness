//! Incremental Server-Sent Events decoder (chunk 1A).
//!
//! Follows the WHATWG event stream rules
//! (<https://html.spec.whatwg.org/multipage/server-sent-events.html#event-stream-interpretation>):
//! lines end with LF, CRLF or CR; `:` starts a comment; `data` lines of one
//! event are joined with `\n`; an event without an `event` field is named
//! `message`; a blank line dispatches the event. Bytes may be split anywhere,
//! even inside a UTF-8 character or between the CR and LF of one line end.

/// Feed it body bytes as they arrive; it calls back once per complete event
/// with `(event name, data)`.
#[derive(Debug, Default)]
pub struct SseDecoder {
    /// Bytes of the current, unfinished line.
    line: Vec<u8>,
    event: String,
    data: String,
    has_data: bool,
    /// The previous byte was a CR, so a following LF belongs to the same line end.
    after_cr: bool,
    /// Still at the start of the stream, where a UTF-8 BOM is dropped.
    at_start: bool,
}

const BOM: &[u8] = b"\xEF\xBB\xBF";

impl SseDecoder {
    pub fn new() -> SseDecoder {
        SseDecoder {
            at_start: true,
            ..SseDecoder::default()
        }
    }

    /// Decodes `bytes`, calling `on_event(name, data)` for each event completed by them.
    pub fn feed(&mut self, bytes: &[u8], on_event: &mut dyn FnMut(&str, &str)) {
        for &byte in bytes {
            if self.after_cr {
                self.after_cr = false;
                if byte == b'\n' {
                    continue;
                }
            }
            match byte {
                b'\n' => self.end_line(on_event),
                b'\r' => {
                    self.after_cr = true;
                    self.end_line(on_event);
                }
                _ => {
                    self.line.push(byte);
                    if self.at_start && self.line.len() == BOM.len() {
                        self.at_start = false;
                        if self.line == BOM {
                            self.line.clear();
                        }
                    }
                }
            }
        }
    }

    /// Ends the stream: a last line without a line end and a last event
    /// without a blank line are still delivered.
    pub fn finish(&mut self, on_event: &mut dyn FnMut(&str, &str)) {
        if !self.line.is_empty() {
            self.end_line(on_event);
        }
        self.dispatch(on_event);
        self.after_cr = false;
    }

    fn end_line(&mut self, on_event: &mut dyn FnMut(&str, &str)) {
        self.at_start = false;
        let raw = std::mem::take(&mut self.line);
        if raw.is_empty() {
            self.dispatch(on_event);
            return;
        }
        let line = String::from_utf8_lossy(&raw);
        if line.starts_with(':') {
            return;
        }
        let (field, value) = match line.find(':') {
            Some(at) => {
                let value = &line[at + 1..];
                (&line[..at], value.strip_prefix(' ').unwrap_or(value))
            }
            None => (line.as_ref(), ""),
        };
        match field {
            "event" => self.event = value.to_string(),
            "data" => {
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.has_data = true;
            }
            // `id` and `retry` do not matter to a one-shot stream.
            _ => {}
        }
    }

    fn dispatch(&mut self, on_event: &mut dyn FnMut(&str, &str)) {
        let event = std::mem::take(&mut self.event);
        let data = std::mem::take(&mut self.data);
        if std::mem::take(&mut self.has_data) {
            let name = if event.is_empty() { "message" } else { &event };
            on_event(name, &data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(input: &[u8], chunk: usize) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut decoder = SseDecoder::new();
        let mut push = |name: &str, data: &str| out.push((name.to_string(), data.to_string()));
        for piece in input.chunks(chunk) {
            decoder.feed(piece, &mut push);
        }
        decoder.finish(&mut push);
        out
    }

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn every_line_end_and_chunk_size() {
        let text = "event: a\ndata: x\n\nevent: b\r\ndata: é😀\r\n\r\nevent: c\rdata:z\r\r";
        let want = pairs(&[("a", "x"), ("b", "é😀"), ("c", "z")]);
        for chunk in 1..=text.len() {
            assert_eq!(decode(text.as_bytes(), chunk), want, "chunk {chunk}");
        }
    }

    #[test]
    fn multi_line_data_comments_and_default_name() {
        let text = ": keep-alive\ndata: one\ndata:two\nid: 7\nretry: 5\n\n:x\n\n";
        assert_eq!(
            decode(text.as_bytes(), 3),
            pairs(&[("message", "one\ntwo")])
        );
    }

    #[test]
    fn event_without_data_is_dropped_and_tail_is_flushed() {
        let text = "event: lonely\n\nevent: last\ndata: {\"a\":1}";
        assert_eq!(decode(text.as_bytes(), 2), pairs(&[("last", "{\"a\":1}")]));
    }

    #[test]
    fn bom_empty_data_and_field_without_colon() {
        let text = "\u{FEFF}data\n\ndata: \n\nevent: e\ndata: a:b\n\n";
        let want = pairs(&[("message", ""), ("message", ""), ("e", "a:b")]);
        for chunk in [1, 2, 3, 7] {
            assert_eq!(decode(text.as_bytes(), chunk), want);
        }
    }

    #[test]
    fn invalid_utf8_does_not_panic() {
        let out = decode(b"data: \xFF\xFE\n\n", 1);
        assert_eq!(out.len(), 1);
    }
}
