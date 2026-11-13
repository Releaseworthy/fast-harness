//! `shell` output as it arrives (PLAN.md D58): pipe chunks coalesced into
//! progress texts, at most one every [`LIVE_EVERY`], each holding at most
//! the newest [`LIVE_BYTES`] of output, cut only at a character border.

use std::time::{Duration, Instant};

/// The least time between two progress texts of one call.
pub(super) const LIVE_EVERY: Duration = Duration::from_millis(100);
/// Output bytes one progress text holds at most; when more arrived since
/// the last one, it holds the newest and says how many it skipped.
pub(super) const LIVE_BYTES: usize = 16 * 1024;
/// The longest UTF-8 sequence less one: an incomplete tail kept back.
const TAIL: usize = 3;

/// Receives the coalesced texts; `None` when nobody listens.
#[derive(Default)]
pub(super) struct Live<'r> {
    report: Option<&'r (dyn Fn(&str) + 'r)>,
    pending: Vec<u8>,
    /// Bytes dropped from the front of `pending` since the last text.
    skipped: usize,
    sent: Option<Instant>,
}

impl<'r> Live<'r> {
    pub(super) fn new(report: &'r (dyn Fn(&str) + 'r)) -> Live<'r> {
        Live {
            report: Some(report),
            pending: Vec::new(),
            skipped: 0,
            sent: None,
        }
    }

    pub(super) fn off() -> Live<'r> {
        Live::default()
    }

    /// Takes a chunk read at `now` and sends what is due.
    pub(super) fn push(&mut self, chunk: &[u8], now: Instant) {
        if self.report.is_none() {
            return;
        }
        self.pending.extend_from_slice(chunk);
        let over = self.pending.len().saturating_sub(LIVE_BYTES + TAIL);
        if over > 0 {
            self.pending.drain(..over);
            self.skipped += over;
        }
        self.flush(now);
    }

    /// Sends the complete characters held when [`LIVE_EVERY`] has passed
    /// since the last text; an incomplete character waits for the rest.
    pub(super) fn flush(&mut self, now: Instant) {
        let Some(report) = self.report else {
            return;
        };
        let early = (self.sent).is_some_and(|at| now.saturating_duration_since(at) < LIVE_EVERY);
        if self.pending.is_empty() || early {
            return;
        }
        if self.skipped > 0 {
            // A cut front may start inside a character.
            let inside = self.pending.iter().take(TAIL);
            let lead = inside.take_while(|byte| **byte & 0xC0 == 0x80).count();
            self.pending.drain(..lead);
            self.skipped += lead;
        }
        let end = complete_end(&self.pending);
        if end == 0 {
            return;
        }
        let text = String::from_utf8_lossy(&self.pending[..end]);
        match self.skipped {
            0 => report(&text),
            skipped => report(&format!("[{skipped} bytes of output skipped]\n{text}")),
        }
        self.pending.drain(..end);
        (self.skipped, self.sent) = (0, Some(now));
    }
}

/// The length of `bytes` without an incomplete UTF-8 sequence at its end.
pub(super) fn complete_end(bytes: &[u8]) -> usize {
    let len = bytes.len();
    for back in 1..=len.min(TAIL) {
        let byte = bytes[len - back];
        if byte & 0xC0 != 0x80 {
            let need = match byte {
                0xF0..=0xF7 => 4,
                0xE0..=0xEF => 3,
                0xC0..=0xDF => 2,
                _ => 1,
            };
            return if need > back { len - back } else { len };
        }
    }
    len
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    fn collect(chunks: &[(&[u8], u64)]) -> Vec<String> {
        let texts = RefCell::new(Vec::new());
        let report = |text: &str| texts.borrow_mut().push(text.to_string());
        let mut live = Live::new(&report);
        let start = Instant::now();
        for (chunk, at) in chunks {
            live.push(chunk, start + Duration::from_millis(*at));
        }
        drop(live);
        texts.into_inner()
    }

    #[test]
    fn a_character_split_across_reads_arrives_whole() {
        let check = "\u{2713}".as_bytes();
        let texts = collect(&[(b"a", 0), (&[b'b', check[0]], 200), (&check[1..], 400)]);
        assert_eq!(texts, ["a", "b", "\u{2713}"]);
    }

    #[test]
    fn texts_coalesce_to_one_per_interval() {
        let texts = collect(&[(b"1", 0), (b"2", 30), (b"3", 60), (b"4", 120), (b"5", 150)]);
        assert_eq!(texts, ["1", "234"], "5 waits for the next interval");
    }

    #[test]
    fn a_flood_sends_bounded_texts_with_the_newest_output() {
        let chunk = "\u{e9}".repeat(4096);
        let chunks: Vec<(&[u8], u64)> = (0..256).map(|i| (chunk.as_bytes(), i * 4)).collect();
        let texts = collect(&chunks);
        assert!(texts.len() <= 11, "{} texts", texts.len());
        for text in &texts {
            assert!(text.len() <= LIVE_BYTES + 64, "{}", text.len());
        }
        let last = texts.last().unwrap();
        assert!(last.starts_with('[') && last.contains("bytes of output skipped]\n\u{e9}"));
        let off = Live::off();
        assert!(off.report.is_none());
    }
}
