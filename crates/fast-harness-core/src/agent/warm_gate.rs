//! Parallel subagents share one cached prefix (PLAN.md D40, 2026-11-15):
//! children of one session that start together on the same model would
//! each write the same system and tool prefix (about 1.7k tokens apiece).
//! The first child sends its first request alone; the others wait until a
//! response to it arrives, at most [`WARM_CAP`], and then read its cache.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// The longest wait for the first child's first response.
pub(super) const WARM_CAP: Duration = Duration::from_secs(3);

/// How long a written prefix counts as warm (the 5 minute cache, less a
/// margin).
const FRESH: Duration = Duration::from_secs(240);

/// How often a waiting child looks at its cancel flag.
const TICK: Duration = Duration::from_millis(100);

enum State {
    /// A child sent its first request at this time and waits for a reply.
    Leading(Instant),
    /// A reply came at this time: the prefix is cached.
    Warm(Instant),
}

/// The prefix state per model of one session's children.
#[derive(Default)]
pub struct WarmGate {
    models: Mutex<HashMap<String, State>>,
    changed: Condvar,
}

impl WarmGate {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, State>> {
        self.models.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Called before a child's first request on `model`. `true`: this child
    /// leads and must call [`WarmGate::warmed`] once a response arrives (or
    /// it ends). `false`: the prefix is warm, or the leader's reply did not
    /// come within `cap`, or `cancel` was set.
    pub fn enter(&self, model: &str, cap: Duration, cancel: &AtomicBool) -> bool {
        let start = Instant::now();
        let mut models = self.lock();
        loop {
            match models.get(model) {
                Some(State::Warm(at)) if at.elapsed() < FRESH => return false,
                Some(State::Leading(since)) if since.elapsed() < cap => {
                    let waited = start.elapsed();
                    if waited >= cap || cancel.load(Ordering::Relaxed) {
                        return false;
                    }
                    let limit = (cap - waited).min(TICK);
                    models = match self.changed.wait_timeout(models, limit) {
                        Ok((guard, _)) => guard,
                        Err(poisoned) => poisoned.into_inner().0,
                    };
                }
                // Cold, stale or a leader past the cap: this child leads.
                _ => {
                    models.insert(model.to_string(), State::Leading(Instant::now()));
                    return true;
                }
            }
        }
    }

    /// A response to the leader's first request on `model` arrived: the
    /// waiting children go.
    pub fn warmed(&self, model: &str) {
        self.lock()
            .insert(model.to_string(), State::Warm(Instant::now()));
        self.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn the_first_child_leads_and_the_others_wait_for_its_reply() {
        let gate = Arc::new(WarmGate::default());
        let never = AtomicBool::new(false);
        let cap = Duration::from_secs(60);
        assert!(gate.enter("m", cap, &never), "the first child leads");
        let order = Arc::new(Mutex::new(Vec::new()));
        let waiter = {
            let (gate, order) = (gate.clone(), order.clone());
            std::thread::spawn(move || {
                let led = gate.enter("m", cap, &AtomicBool::new(false));
                order.lock().unwrap().push("second sends");
                led
            })
        };
        assert!(
            gate.enter("other", cap, &never),
            "another model has its own prefix"
        );
        order.lock().unwrap().push("first reply");
        gate.warmed("m");
        assert!(!waiter.join().unwrap(), "the second child does not lead");
        assert_eq!(*order.lock().unwrap(), ["first reply", "second sends"]);
        assert!(!gate.enter("m", cap, &never), "a warm prefix: no wait");
    }

    #[test]
    fn a_leader_without_a_reply_holds_nobody_past_the_cap() {
        let gate = WarmGate::default();
        let never = AtomicBool::new(false);
        assert!(gate.enter("m", Duration::ZERO, &never));
        assert!(
            gate.enter("m", Duration::ZERO, &never),
            "past the cap: leads itself"
        );
        let cancelled = AtomicBool::new(true);
        let cap = Duration::from_secs(60);
        assert!(!gate.enter("m", cap, &cancelled), "a cancel ends the wait");
    }
}
