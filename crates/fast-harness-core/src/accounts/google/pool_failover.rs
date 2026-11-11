//! [`GoogleAuth`] as an [`AuthSource`]: leases and failover among Google
//! accounts only (PLAN.md D23). A provider never switches by itself.

use super::poll::TURN_READ_GAP_MS;
use super::pool::{GoogleAuth, HoldReason, State, GOOGLE_BROKEN_HOLD_MS};
use super::pool_order::{hold_until, LIMIT_HOLD_MS};
use crate::auth::{AuthError, AuthSource, Failover, Lease};
use crate::http::{HttpHead, Transport};
use crate::llm::{Auth, ErrorKind, LlmError};

/// The words of a 403 asking the user to verify the account.
const VALIDATION: &str = "validation required";

/// Whether an auth error is Google's account verification request.
pub fn is_validation(message: &str) -> bool {
    message.to_lowercase().contains(VALIDATION)
}

/// The first `http://` or `https://` URL in `message`, without trailing punctuation.
pub fn validation_url(message: &str) -> Option<String> {
    let start = message
        .find("https://")
        .or_else(|| message.find("http://"))?;
    let rest = &message[start..];
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '<' || c == '>')
        .unwrap_or(rest.len());
    let url = rest[..end].trim_end_matches(['.', ',', ';', ')', ']', '}']);
    (url.len() > "https://".len()).then(|| url.to_string())
}

impl GoogleAuth {
    /// Installs the best other account with a usable token (`Retry`); with
    /// none, waits until the soonest hold ends.
    fn fail_over(
        &self,
        state: &mut State,
        current: &str,
        model: &str,
        http: &dyn Transport,
    ) -> Failover {
        for id in self.order(state, current, model) {
            match self.token(state, &id, http, false) {
                Ok(_) => {
                    self.make_active(&id);
                    return Failover::Retry;
                }
                Err(err) => {
                    state.errors.insert(id, err);
                }
            }
        }
        let now = self.now();
        let soonest = state
            .holds
            .values()
            .map(|h| h.until)
            .filter(|u| *u > now)
            .min();
        Failover::Wait(soonest.map_or(LIMIT_HOLD_MS, |until| (until - now).max(1)))
    }

    fn on_limit(
        &self,
        lease: &Lease,
        err: &LlmError,
        model: &str,
        http: &dyn Transport,
    ) -> Failover {
        let id = lease.account.as_str();
        let now = self.now();
        let may_read = self.lock().may_read(id, now, TURN_READ_GAP_MS);
        if let (true, Auth::Google { access, project }) = (may_read, &lease.auth) {
            // Read without the lock: other threads keep leasing meanwhile.
            let result = self.api.retrieve_quota(http, access, project);
            let mut state = self.lock();
            self.record_read(&mut state, id, result, now);
        }
        let mut state = self.lock();
        let quota = state.reads.get(id).map(|r| &r.quota);
        let until = hold_until(quota, model, err.retry_after_ms(), now);
        state.hold(id, until, HoldReason::RateLimit, None);
        state.errors.insert(id.to_string(), err.message.clone());
        self.fail_over(&mut state, id, model, http)
    }

    fn on_validation(
        &self,
        lease: &Lease,
        err: &LlmError,
        model: &str,
        http: &dyn Transport,
    ) -> Failover {
        let mut state = self.lock();
        let id = lease.account.as_str();
        let until = self.now().saturating_add(GOOGLE_BROKEN_HOLD_MS);
        let url = validation_url(&err.message);
        state.hold(id, until, HoldReason::Validation, url);
        state.errors.insert(id.to_string(), err.message.clone());
        self.fail_over(&mut state, id, model, http)
    }

    fn on_auth(
        &self,
        lease: &Lease,
        err: &LlmError,
        model: &str,
        http: &dyn Transport,
    ) -> Failover {
        let mut state = self.lock();
        let id = lease.account.as_str();
        // One forced refresh per failure streak; a second refusal holds the account.
        if state.forced.insert(id.to_string()) && self.token(&mut state, id, http, true).is_ok() {
            return Failover::Retry;
        }
        let until = self.now().saturating_add(GOOGLE_BROKEN_HOLD_MS);
        state.hold(id, until, HoldReason::Refused, None);
        state.errors.insert(id.to_string(), err.message.clone());
        self.fail_over(&mut state, id, model, http)
    }
}

impl AuthSource for GoogleAuth {
    /// The active Google account unless held, else the best other one (which
    /// becomes active), else the held active one (after a `Wait`).
    fn lease(&self, http: &dyn Transport) -> Result<Lease, AuthError> {
        let mut state = self.lock();
        if self.pool.google_accounts().is_empty() {
            return Err(AuthError {
                message: "no Google account: run `fast-harness accounts login --google`".into(),
            });
        }
        let now = self.now();
        let active = self.pool.active_google();
        let current = active.clone().unwrap_or_default();
        let held = state.held_until(&current, now).is_some();
        let mut order: Vec<String> = active.iter().filter(|_| !held).cloned().collect();
        order.extend(self.order(&state, &current, ""));
        order.extend(active.iter().filter(|_| held).cloned());
        let mut last = None;
        for id in order {
            match self.token(&mut state, &id, http, false) {
                Ok((access, project)) => {
                    self.make_active(&id);
                    state.last_used = Some(id.clone());
                    return Ok(self.lease_of(&id, access, project));
                }
                Err(err) => {
                    state.errors.insert(id, err.clone());
                    last = Some(err);
                }
            }
        }
        let why = last.unwrap_or_else(|| "every account is held".into());
        Err(AuthError {
            message: format!("no Google account is usable: {why}"),
        })
    }

    /// Google sends no usage headers; a success ends a refresh streak.
    fn observe(&self, lease: &Lease, head: &HttpHead) {
        if (200..300).contains(&head.status) {
            self.lock().forced.remove(&lease.account);
        }
    }

    fn on_failure(
        &self,
        lease: &Lease,
        err: &LlmError,
        model: &str,
        http: &dyn Transport,
    ) -> Failover {
        if !matches!(lease.auth, Auth::Google { .. }) {
            return Failover::Fail;
        }
        match err.kind {
            ErrorKind::RateLimit => self.on_limit(lease, err, model, http),
            ErrorKind::Auth if is_validation(&err.message) => {
                self.on_validation(lease, err, model, http)
            }
            ErrorKind::Auth => self.on_auth(lease, err, model, http),
            _ => Failover::Fail,
        }
    }
}
