//! Quota reads of the Google pool (PLAN.md D23): idle accounts at most every
//! [`GOOGLE_POLL_EVERY_MS`], the account just used [`TURN_READ_GAP_MS`] or more
//! after its previous read, and a 429 pauses an account's reads
//! [`QUOTA_BACKOFF_MS`], doubling up to [`QUOTA_BACKOFF_MAX_MS`].

use super::pool::{Backoff, GoogleAuth, State};
use super::usage_file::QuotaRead;
use super::{GoogleError, GoogleQuota};
use crate::http::Transport;

/// Shortest gap between two reads of an idle account.
pub const GOOGLE_POLL_EVERY_MS: u64 = 10 * 60_000;
/// Shortest gap between two reads of the account a turn used.
pub const TURN_READ_GAP_MS: u64 = 30_000;
/// First pause of an account's reads after a 429.
pub const QUOTA_BACKOFF_MS: u64 = 5 * 60_000;
/// Longest pause after repeated 429 answers.
pub const QUOTA_BACKOFF_MAX_MS: u64 = 60 * 60_000;

impl State {
    fn backed_off(&self, id: &str, now: u64) -> bool {
        self.backoff.get(id).is_some_and(|b| b.until > now)
    }

    /// Whether `id` may be read at `now`: not paused, and `gap` or more after its last read.
    pub(super) fn may_read(&self, id: &str, now: u64, gap: u64) -> bool {
        !self.backed_off(id, now)
            && self
                .reads
                .get(id)
                .is_none_or(|r| now >= r.read_at.saturating_add(gap))
    }

    fn due_now(&self, id: &str, now: u64) -> bool {
        if self.backed_off(id, now) {
            return false;
        }
        match self.due.get(id) {
            Some(at) if *at <= now => true,
            _ => self.may_read(id, now, GOOGLE_POLL_EVERY_MS),
        }
    }

    fn back_off(&mut self, id: &str, now: u64) {
        let step = self
            .backoff
            .get(id)
            .map_or(QUOTA_BACKOFF_MS, |b| b.step.saturating_mul(2))
            .min(QUOTA_BACKOFF_MAX_MS);
        let until = now.saturating_add(step);
        self.backoff.insert(id.to_string(), Backoff { until, step });
    }
}

impl GoogleAuth {
    /// Stores the result of a quota read of `id` made at `now`.
    pub(super) fn record_read(
        &self,
        state: &mut State,
        id: &str,
        result: Result<GoogleQuota, GoogleError>,
        now: u64,
    ) {
        state.due.remove(id);
        match result {
            Ok(quota) => {
                let read = QuotaRead {
                    quota,
                    read_at: now,
                };
                state.reads.insert(id.to_string(), read);
                state.backoff.remove(id);
                self.save_reads(state);
            }
            Err(GoogleError::Refused { status: 429, .. }) => {
                state.back_off(id, now);
                state
                    .errors
                    .insert(id.to_string(), "quota read limited (HTTP 429)".into());
            }
            Err(err) => {
                state.errors.insert(id.to_string(), err.to_string());
            }
        }
    }

    /// Reads the quota of every account that is due at `now` (see the module
    /// docs); returns how many reads were sent.
    pub fn poll(&self, http: &dyn Transport, now: u64) -> usize {
        let due: Vec<String> = {
            let state = self.lock();
            let accounts = self.pool.google_accounts();
            accounts
                .into_iter()
                .map(|a| a.id)
                .filter(|id| !state.dead.contains_key(id) && state.due_now(id, now))
                .collect()
        };
        due.iter().filter(|id| self.read_one(http, id, now)).count()
    }

    /// One quota read of `id`; false when no token was on hand.
    fn read_one(&self, http: &dyn Transport, id: &str, now: u64) -> bool {
        let token = {
            let mut state = self.lock();
            let token = self.token(&mut state, id, http, false);
            if let Err(err) = &token {
                state.due.remove(id);
                state.errors.insert(id.to_string(), err.clone());
            }
            token
        };
        let Ok((access, project)) = token else {
            return false;
        };
        // Without the lock: leases go on while Google answers.
        let result = self.api.retrieve_quota(http, &access, &project);
        let mut state = self.lock();
        self.record_read(&mut state, id, result, now);
        true
    }

    /// After a turn: schedules a read of the account it used, at `now` or
    /// [`TURN_READ_GAP_MS`] after that account's previous read, whichever is later.
    pub fn after_turn(&self, now: u64) {
        let mut state = self.lock();
        let Some(id) = state.last_used.clone() else {
            return;
        };
        let at = state
            .reads
            .get(&id)
            .map_or(now, |r| now.max(r.read_at.saturating_add(TURN_READ_GAP_MS)));
        state.due.insert(id, at);
    }
}
