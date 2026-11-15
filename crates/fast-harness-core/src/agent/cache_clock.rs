//! When the main session's message tail is cached for 1 hour (PLAN.md D88).
//!
//! The API does not count a hit on a 5 minute entry when a 1 hour
//! breakpoint after it misses (platform.claude.com prompt caching, "Mixing
//! different TTLs", 2026-11-15): a 1 hour breakpoint placed after a tool
//! loop cached for 5 minutes writes that loop again at the 1 hour price. So
//! a turn starts with a 1 hour tail only when the previous request started
//! at least [`QUICK_MS`] ago (its 5 minute entries may be gone), and the
//! 1 hour anchor sits only where a 1 hour tail was written.

use crate::llm::Cache;
use crate::trace::{kind, TraceEvent};
use crate::types::{Message, Role};

/// A turn that starts sooner after the previous request reads its 5 minute
/// entries (each lives 5 minutes from its last use).
pub(crate) const QUICK_MS: u64 = 4 * 60_000;

/// The start of the last answered request and where the last 1 hour tail
/// was written.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CacheClock {
    /// Unix milliseconds.
    last_request: Option<u64>,
    /// The tail sat on the user message after this many assistant messages.
    anchor: Option<usize>,
}

impl CacheClock {
    /// After a reopen: the start of the log's last request. Where its last
    /// 1 hour tail is is not known, so there is no anchor until the next one.
    pub(crate) fn from_events(events: &[TraceEvent]) -> CacheClock {
        let last = events
            .iter()
            .rev()
            .find(|e| e.kind == kind::REQUEST_CONTEXT);
        CacheClock {
            last_request: last.map(|e| e.time),
            anchor: None,
        }
    }

    /// `base` for a request starting at `now` (Unix milliseconds).
    pub(crate) fn cache(&self, base: Cache, now: u64) -> Cache {
        let idle = self
            .last_request
            .is_none_or(|t| now.saturating_sub(t) >= QUICK_MS);
        base.with_turn_start(idle, self.anchor)
    }

    /// Notes a request that started at `now` with `cache` over `messages`
    /// and got an answer.
    pub(crate) fn answered(&mut self, cache: &Cache, now: u64, messages: &[Message]) {
        self.last_request = Some(now);
        let opens_turn = messages.last().is_some_and(|m| m.role == Role::User);
        if cache.loop_5m && cache.messages_1h && cache.turn_start_1h && opens_turn {
            let answers = messages.iter().filter(|m| m.role == Role::Assistant);
            self.anchor = Some(answers.count());
        }
    }

    /// A compaction replaced the messages: the anchor names none of them.
    pub(crate) fn forget_anchor(&mut self) {
        self.anchor = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_starts_with_an_hour_only_after_an_idle_gap() {
        let base = Cache::new(true, true).with_loop_5m(true);
        let fresh = CacheClock::default();
        assert!(fresh.cache(base, 1_000).turn_start_1h);
        let mut clock = fresh;
        let messages = [Message::user("hi")];
        clock.answered(&clock.cache(base, 1_000), 1_000, &messages);
        assert_eq!(clock.anchor, Some(0));
        assert!(!clock.cache(base, 1_000 + QUICK_MS - 1).turn_start_1h);
        assert!(clock.cache(base, 1_000 + QUICK_MS).turn_start_1h);
        assert_eq!(clock.cache(base, 2_000).anchor, Some(0));
    }
}
