//! The pool as an [`AuthSource`]: leases, live usage and failover.
//!
//! The pool's mutex and the store lock are held while a token refreshes
//! (`pool/sync.rs`), so one single-use refresh token is never spent twice at
//! once, in this process or another (claude-switch's `refreshGrant` runs one
//! refresh per token at a time for the same reason).

use super::sync::{usable, Refreshed};
use super::{now_ms, AccountPool, State};
use crate::accounts::model::{now_unix_ms, AccountKind, Provider, Secret};
use crate::accounts::order::{
    candidates, hold_for_limit, Candidate, BROKEN_HOLD_MS, RETRY_INTERVAL_MS,
};
use crate::accounts::usage::{current_usage, usage_from_headers, Usage};
use crate::auth::{AuthError, AuthSource, Failover, Lease};
use crate::http::{HttpHead, Transport};
use crate::llm::{Auth, ErrorKind, LlmError};

/// An own access token expiring sooner than this is refreshed before use
/// (claude-switch `REFRESH_MARGIN_MS`).
pub const REFRESH_MARGIN_MS: u64 = 60_000;

/// Live usage from headers; model windows only the endpoint reports are kept
/// from the previous read.
fn merged(previous: Option<&Usage>, mut fresh: Usage, now: i64) -> Usage {
    if fresh.extra.is_empty() {
        if let Some(previous) = previous {
            fresh.extra = current_usage(previous, now).extra;
        }
    }
    fresh
}

impl State {
    /// A credential for `id`, or `None` when it is unusable now. An own token
    /// is refreshed when it expires within the margin or when `force` is set;
    /// a refused refresh holds the account for `BROKEN_HOLD_MS`.
    pub(super) fn token_for(
        &mut self,
        id: &str,
        http: &dyn Transport,
        force: bool,
    ) -> Option<Auth> {
        let account = self.store.get(id)?;
        match account.kind {
            AccountKind::Own => self.own_token(id, http, force),
            AccountKind::ApiKey => match self.store.secret(id) {
                Ok(Some(Secret::ApiKey { key })) => Some(Auth::ApiKey(key)),
                _ => None,
            },
        }
    }

    fn own_token(&mut self, id: &str, http: &dyn Transport, force: bool) -> Option<Auth> {
        let Ok(Some(Secret::Own {
            refresh,
            access,
            expires_at,
        })) = self.store.secret(id)
        else {
            return None;
        };
        let now = now_unix_ms();
        let usable = usable(expires_at);
        if self.dead.contains(id) {
            return (usable && !force).then_some(Auth::OAuth(access));
        }
        if !force && usable {
            return Some(Auth::OAuth(access));
        }
        match self.refresh_own(id, &refresh, http) {
            Refreshed::Token(token) => Some(Auth::OAuth(token)),
            Refreshed::Dead => {
                self.dead.insert(id.to_string());
                self.hold(id, now_ms() + BROKEN_HOLD_MS, "refresh token refused");
                None
            }
            Refreshed::Failed if !force && expires_at > now => Some(Auth::OAuth(access)),
            Refreshed::Failed => None,
        }
    }
    fn candidate_list(&self) -> Vec<Candidate> {
        self.store
            .list()
            .into_iter()
            .filter(|account| account.provider == Provider::Anthropic)
            .map(|account| Candidate {
                id: account.id.clone(),
                plan: account.plan.clone(),
                usage: self.snapshot.usage(&account.id).cloned(),
            })
            .collect()
    }

    fn lease_of(&self, id: &str, auth: Auth) -> Lease {
        let account = self.store.get(id);
        let label = account
            .and_then(|a| a.email.clone().or_else(|| a.name.clone()))
            .unwrap_or_else(|| id.to_string());
        Lease {
            account: id.to_string(),
            label,
            auth,
        }
    }

    /// Makes `id` active on disk. A failed write leaves the pointer as it
    /// was and shows as the pool's warning; the next lease tries again.
    fn make_active(&mut self, id: &str) {
        if self.store.active().map(|a| a.id.as_str()) == Some(id) {
            return;
        }
        if let Err(err) = self.change(|store| store.set_active(id)) {
            self.save_error = Some(err.to_string());
        }
    }

    /// Installs the first candidate with a usable credential (`Retry`); with
    /// none, waits for the soonest hold, at most `RETRY_INTERVAL_MS`.
    fn fail_over(&mut self, current: &str, model: &str, http: &dyn Transport) -> Failover {
        let now = now_ms();
        let order = candidates(&self.candidate_list(), current, &self.holds(), model, now);
        for id in order {
            if self.token_for(&id, http, false).is_some() {
                self.make_active(&id);
                return Failover::Retry;
            }
        }
        let soonest = self.holds().soonest(now);
        let wait = soonest.map_or(RETRY_INTERVAL_MS, |until| {
            (until - now).clamp(1, RETRY_INTERVAL_MS)
        });
        Failover::Wait(u64::try_from(wait).unwrap_or(1))
    }

    /// Records usage from `head` for `id`; returns false when it has none.
    fn record_headers(&mut self, id: &str, head: &HttpHead) -> bool {
        let Some(fresh) = usage_from_headers(&head.headers) else {
            return false;
        };
        let usage = merged(self.snapshot.usage(id), fresh, now_ms());
        self.snapshot.set(id, usage, now_unix_ms());
        true
    }
}

impl AccountPool {
    fn on_limit(
        &self,
        lease: &Lease,
        err: &LlmError,
        model: &str,
        http: &dyn Transport,
    ) -> Failover {
        let mut state = self.lock();
        if let Some(head) = &err.head {
            state.record_headers(&lease.account, head);
        }
        let until = hold_for_limit(state.snapshot.usage(&lease.account), model, now_ms());
        state.hold(&lease.account, until, "rate limit");
        let _ = state.snapshot.save(now_unix_ms(), true);
        state.fail_over(&lease.account, model, http)
    }

    fn on_auth(&self, lease: &Lease, model: &str, http: &dyn Transport) -> Failover {
        let mut state = self.lock();
        let id = lease.account.as_str();
        let kind = state.store.get(id).map(|a| a.kind);
        match kind {
            Some(AccountKind::ApiKey) => return Failover::Fail,
            // One forced refresh per failure streak; a second refusal holds the account.
            Some(AccountKind::Own)
                if state.forced.insert(id.to_string())
                    && state.token_for(id, http, true).is_some() =>
            {
                return Failover::Retry;
            }
            _ => {}
        }
        state.hold(id, now_ms() + BROKEN_HOLD_MS, "credentials refused");
        state.fail_over(id, model, http)
    }
}

impl AuthSource for AccountPool {
    /// The active account (when not held), else the best candidate, else the
    /// held active account again (after a `Wait` the call is simply retried).
    fn lease(&self, http: &dyn Transport) -> Result<Lease, AuthError> {
        let mut state = self.lock();
        state.fresh();
        let now = now_ms();
        let active = state.store.active().map(|a| a.id.clone());
        let current = active.clone().unwrap_or_default();
        let held = state.holds().held_until(&current, now).is_some();
        let mut order: Vec<String> = active.iter().filter(|_| !held).cloned().collect();
        order.extend(candidates(
            &state.candidate_list(),
            &current,
            &state.holds(),
            "",
            now,
        ));
        order.extend(active.iter().filter(|_| held).cloned());
        for id in order {
            if let Some(auth) = state.token_for(&id, http, false) {
                state.make_active(&id);
                return Ok(state.lease_of(&id, auth));
            }
        }
        Err(AuthError {
            message: "no Claude account is usable: sign in or add a key".into(),
        })
    }

    fn observe(&self, lease: &Lease, head: &HttpHead) {
        let mut state = self.lock();
        if (200..300).contains(&head.status) {
            state.forced.remove(&lease.account);
        }
        if state.record_headers(&lease.account, head) {
            let _ = state.snapshot.save(now_unix_ms(), false);
        }
    }

    fn on_failure(
        &self,
        lease: &Lease,
        err: &LlmError,
        model: &str,
        http: &dyn Transport,
    ) -> Failover {
        match err.kind {
            ErrorKind::RateLimit => self.on_limit(lease, err, model, http),
            ErrorKind::Auth => self.on_auth(lease, model, http),
            _ => Failover::Fail,
        }
    }

    fn warning(&self) -> Option<String> {
        self.save_error()
    }
}
