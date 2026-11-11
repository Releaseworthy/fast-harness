//! Usage of idle accounts from the usage endpoint, paced as claude-switch
//! paces it: each account at most every `USAGE_EVERY_MS`, `READ_GAP_MS`
//! between two calls, and a 429 pausing that account 5 minutes, doubling up
//! to 60 minutes.

use super::{AccountPool, State};
use crate::accounts::model::{now_unix_ms, AccountKind, Provider};
use crate::accounts::remote::fetch_usage;
use crate::http::Transport;
use crate::llm::Auth;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

/// An account's usage is read again once its last read is this old.
pub const USAGE_EVERY_MS: u64 = 10 * 60_000;
/// Pause between two usage-endpoint calls.
pub const READ_GAP_MS: u64 = 3_000;
/// The first 429 pauses that account's reads this long; each further one doubles it.
const BACKOFF_MS: u64 = 5 * 60_000;
const BACKOFF_MAX_MS: u64 = 60 * 60_000;

/// An account's usage-endpoint pause after a 429.
#[derive(Clone, Copy, Debug)]
pub(super) struct Backoff {
    until: u64,
    pause_ms: u64,
}

impl State {
    /// Own Anthropic accounts not read for `USAGE_EVERY_MS` and not paused.
    fn due_for_poll(&self, now: u64) -> Vec<String> {
        self.store
            .list()
            .into_iter()
            .filter(|a| a.kind == AccountKind::Own && a.provider == Provider::Anthropic)
            .filter(|a| {
                self.snapshot
                    .read_at(&a.id)
                    .is_none_or(|at| now.saturating_sub(at) >= USAGE_EVERY_MS)
            })
            .filter(|a| self.backoff.get(&a.id).is_none_or(|b| b.until <= now))
            .map(|a| a.id.clone())
            .collect()
    }

    fn back_off(&mut self, id: &str, now: u64, retry_after_ms: Option<u64>) {
        let pause_ms = self
            .backoff
            .get(id)
            .map_or(BACKOFF_MS, |b| (b.pause_ms * 2).min(BACKOFF_MAX_MS));
        let wait = pause_ms.max(retry_after_ms.unwrap_or(0));
        let until = now.saturating_add(wait);
        self.backoff
            .insert(id.to_string(), Backoff { until, pause_ms });
    }
}

impl AccountPool {
    /// Reads the usage of every idle account that is due, one call every
    /// `READ_GAP_MS` (see [`AccountPool::set_read_gap`]). Returns how many
    /// accounts were read. The pool is not locked during the calls.
    pub fn poll_usage(&self, http: &dyn Transport) -> usize {
        let due = self.lock().due_for_poll(now_unix_ms());
        let gap = Duration::from_millis(self.read_gap_ms.load(Ordering::Relaxed));
        let mut read = 0;
        let mut first = true;
        for id in due {
            let auth = self.lock().token_for(&id, http, false);
            let Some(Auth::OAuth(access)) = auth else {
                continue;
            };
            if !first {
                thread::sleep(gap);
            }
            first = false;
            let result = fetch_usage(http, &access);
            let mut state = self.lock();
            let now = now_unix_ms();
            match result {
                Ok(usage) => {
                    state.snapshot.set(&id, usage, now);
                    state.backoff.remove(&id);
                    read += 1;
                }
                Err(err) if err.is_rate_limit() => state.back_off(&id, now, err.retry_after_ms()),
                Err(_) => {}
            }
        }
        if read > 0 {
            let _ = self.lock().snapshot.save(now_unix_ms(), true);
        }
        read
    }
}
