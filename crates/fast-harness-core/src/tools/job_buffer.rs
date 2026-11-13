//! The output of one background job (PLAN.md D68): the newest [`RING`]
//! bytes in memory, and under the session folder a log file holding the
//! first [`LOG_CAP`] bytes, so a read that fell behind the ring still has
//! somewhere to look.

use std::collections::VecDeque;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

/// Output bytes a job keeps in memory: the newest.
pub const RING: usize = 64 * 1024;
/// Output bytes a job's log file keeps at most: the first.
pub const LOG_CAP: u64 = 16 * 1024 * 1024;

#[derive(Debug, Default)]
pub(super) struct Buffer {
    ring: VecDeque<u8>,
    /// Bytes received in all.
    total: u64,
    /// Open until [`LOG_CAP`] is reached or a write fails.
    log: Option<File>,
    /// The log file, when one could be created.
    pub(super) log_path: Option<PathBuf>,
    logged: u64,
}

impl Buffer {
    /// A buffer logging to `log_path` (`None`: memory only). The log is a
    /// stream appended as output arrives, not a settings file, so it is
    /// written in place (not through `store`).
    pub(super) fn new(log_path: Option<PathBuf>) -> Buffer {
        let log = log_path.as_ref().and_then(|path| {
            std::fs::create_dir_all(path.parent()?).ok()?;
            File::create(path).ok()
        });
        let log_path = log.as_ref().and(log_path);
        Buffer {
            log,
            log_path,
            ..Buffer::default()
        }
    }

    pub(super) fn push(&mut self, chunk: &[u8]) {
        self.total += chunk.len() as u64;
        self.ring.extend(chunk);
        let over = self.ring.len().saturating_sub(RING);
        self.ring.drain(..over);
        let room = LOG_CAP.saturating_sub(self.logged) as usize;
        let kept = room.min(chunk.len());
        if let Some(log) = &mut self.log {
            if kept == 0 || log.write_all(&chunk[..kept]).is_err() {
                self.log = None;
            } else {
                self.logged += kept as u64;
            }
        }
    }

    pub(super) fn total(&self) -> u64 {
        self.total
    }

    /// The bytes from offset `cursor` on: how many of them already left the
    /// ring, and the rest.
    pub(super) fn since(&self, cursor: u64) -> (u64, Vec<u8>) {
        let start = self.total - self.ring.len() as u64;
        let from = cursor.clamp(start, self.total);
        let skipped = from.saturating_sub(cursor);
        let offset = (from - start) as usize;
        (skipped, self.ring.range(offset..).copied().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_keeps_the_newest_bytes_and_counts_the_rest() {
        let mut buffer = Buffer::new(None);
        buffer.push(b"abc");
        assert_eq!(buffer.since(0), (0, b"abc".to_vec()));
        assert_eq!(buffer.since(2), (0, b"c".to_vec()));
        assert_eq!(buffer.since(3), (0, Vec::new()));
        buffer.push(&vec![b'x'; RING]);
        let (skipped, bytes) = buffer.since(1);
        assert_eq!((skipped, bytes.len()), (2, RING));
        assert_eq!(buffer.total(), RING as u64 + 3);
    }
}
