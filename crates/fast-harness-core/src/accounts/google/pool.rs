//! The Google account pool (PLAN.md D23): an [`crate::auth::AuthSource`] over
//! the Google accounts of the shared [`AccountPool`], which stays the one
//! owner of `accounts.json`. Holds, quota reads and errors live here, in
//! memory; the last quota read of each account also in `google-usage.json`.
//!
//! Lock order: this pool's mutex, then the account pool's. A token refresh
//! runs with this mutex held, so one account never refreshes twice at once.

use super::pool_order::{google_order, GoogleCandidate};
use super::usage_file::{load_usage, save_usage, usage_path, QuotaRead};
use super::{load_client, GoogleApi, GoogleClient, GoogleError, GoogleQuota, FALLBACK_PROJECT_ID};
use crate::accounts::model::{now_unix_ms, Account, Secret};
use crate::accounts::pool::{AccountPool, PoolError};
use crate::auth::Lease;
use crate::http::Transport;
use crate::llm::Auth;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// An access token expiring sooner than this is refreshed before use.
pub const GOOGLE_REFRESH_MARGIN_MS: u64 = 5 * 60_000;
/// Hold after refused credentials or a verification request.
pub const GOOGLE_BROKEN_HOLD_MS: u64 = 60 * 60_000;

/// Why an account is held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoldReason {
    RateLimit,
    /// Google asks the user to verify the account (the URL is in the hold).
    Validation,
    /// The token or the refresh token was refused.
    Refused,
}

/// A running hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GoogleHold {
    /// Unix milliseconds when it ends.
    pub until: u64,
    pub reason: HoldReason,
    /// The verification page of a `Validation` hold.
    pub url: Option<String>,
}

/// One Google account as a front end shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct GoogleStatus {
    pub account: Account,
    pub active: bool,
    pub hold: Option<GoogleHold>,
    pub quota: Option<GoogleQuota>,
    /// Unix milliseconds of the quota read.
    pub read_at: Option<u64>,
    pub last_error: Option<String>,
}

/// Quota-read pause after a 429.
#[derive(Clone, Copy, Debug)]
pub(super) struct Backoff {
    pub until: u64,
    pub step: u64,
}

#[derive(Default)]
pub(super) struct State {
    pub holds: HashMap<String, GoogleHold>,
    pub reads: HashMap<String, QuotaRead>,
    /// Scheduled quota reads (`after_turn`), Unix milliseconds.
    pub due: HashMap<String, u64>,
    pub backoff: HashMap<String, Backoff>,
    pub errors: HashMap<String, String>,
    /// Accounts refreshed after an auth failure, until a call succeeds.
    pub forced: HashSet<String>,
    /// Refresh tokens Google refused, by account: never sent again.
    pub dead: HashMap<String, String>,
    pub last_used: Option<String>,
}

impl State {
    /// Holds `id` until `until`; a longer hold in place is kept.
    pub fn hold(&mut self, id: &str, until: u64, reason: HoldReason, url: Option<String>) {
        if self.holds.get(id).is_some_and(|h| h.until >= until) {
            return;
        }
        let hold = GoogleHold { until, reason, url };
        self.holds.insert(id.to_string(), hold);
    }

    pub fn held_until(&self, id: &str, now: u64) -> Option<u64> {
        self.holds.get(id).map(|h| h.until).filter(|u| *u > now)
    }
}

/// The Google pool. `Send + Sync`; every method takes `&self`.
pub struct GoogleAuth {
    pub(super) pool: Arc<AccountPool>,
    home: PathBuf,
    pub(super) api: GoogleApi,
    client: Mutex<Option<GoogleClient>>,
    pub(super) state: Mutex<State>,
    clock: AtomicU64,
}

impl std::fmt::Debug for GoogleAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoogleAuth").finish_non_exhaustive()
    }
}

impl GoogleAuth {
    /// The pool over `pool`'s Google accounts; `app_home` holds the OAuth
    /// client file and `google-usage.json`.
    pub fn new(pool: Arc<AccountPool>, app_home: &Path) -> GoogleAuth {
        let state = State {
            reads: load_usage(&usage_path(app_home)),
            ..State::default()
        };
        GoogleAuth {
            pool,
            home: app_home.to_path_buf(),
            api: GoogleApi::default(),
            client: Mutex::new(None),
            state: Mutex::new(state),
            clock: AtomicU64::new(0),
        }
    }

    /// Replaces the Google endpoints (tests point them at a scripted transport).
    pub fn with_api(mut self, api: GoogleApi) -> GoogleAuth {
        self.api = api;
        self
    }

    /// Uses `client` instead of [`load_client`]; `None` goes back to it.
    pub fn set_client(&self, client: Option<GoogleClient>) {
        *self.client.lock().unwrap_or_else(PoisonError::into_inner) = client;
    }

    /// Fixes the clock at `now_ms` (tests); `None` is the real clock.
    pub fn set_clock(&self, now_ms: Option<u64>) {
        self.clock.store(now_ms.unwrap_or(0), Ordering::Relaxed);
    }

    pub(super) fn now(&self) -> u64 {
        match self.clock.load(Ordering::Relaxed) {
            0 => now_unix_ms(),
            fixed => fixed,
        }
    }

    pub(super) fn lock(&self) -> MutexGuard<'_, State> {
        // Every change under the lock is a single insert or remove.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn client(&self) -> Result<GoogleClient, GoogleError> {
        let fixed = self.client.lock().unwrap_or_else(PoisonError::into_inner);
        match fixed.as_ref() {
            Some(client) => Ok(client.clone()),
            None => load_client(&self.home),
        }
    }

    pub(super) fn save_reads(&self, state: &State) {
        // Display data only: a failed write keeps the reads in memory.
        let _ = save_usage(&usage_path(&self.home), &state.reads);
    }

    /// The access token and project of `id`; the token is refreshed when it
    /// expires within [`GOOGLE_REFRESH_MARGIN_MS`] or when `force` is set.
    pub(super) fn token(
        &self,
        state: &mut State,
        id: &str,
        http: &dyn Transport,
        force: bool,
    ) -> Result<(String, String), String> {
        let Some(Secret::Google {
            refresh,
            access,
            expires_at,
            project_id,
        }) = self.pool.google_secret(id)
        else {
            return Err("no Google credential: sign in again".into());
        };
        if state.dead.get(id) == Some(&refresh) {
            return Err("Google refused the refresh token: sign in again".into());
        }
        let now = self.now();
        let fresh = expires_at > now.saturating_add(GOOGLE_REFRESH_MARGIN_MS);
        let access = if fresh && !force {
            access
        } else {
            match self.refreshed(state, id, http, &refresh, project_id.clone()) {
                Ok(access) => access,
                Err(_) if !force && expires_at > now => access,
                Err(err) => return Err(err),
            }
        };
        let project = match project_id {
            Some(project) => project,
            None => self.project(id, http, &access),
        };
        Ok((access, project))
    }

    fn refreshed(
        &self,
        state: &mut State,
        id: &str,
        http: &dyn Transport,
        refresh: &str,
        project_id: Option<String>,
    ) -> Result<String, String> {
        let client = self.client().map_err(|err| err.to_string())?;
        match self.api.refresh(http, &client, refresh) {
            Ok(tokens) => {
                let secret = Secret::Google {
                    refresh: tokens.refresh,
                    access: tokens.access.clone(),
                    expires_at: tokens.expires_at,
                    project_id,
                };
                // Google keeps the refresh token valid: a failed write only
                // costs a refresh next time.
                let _ = self.pool.set_google_secret(id, secret);
                Ok(tokens.access)
            }
            Err(err @ GoogleError::Dead(_)) => {
                state.dead.insert(id.to_string(), refresh.to_string());
                let until = self.now().saturating_add(GOOGLE_BROKEN_HOLD_MS);
                state.hold(id, until, HoldReason::Refused, None);
                Err(err.to_string())
            }
            Err(err) => Err(err.to_string()),
        }
    }

    /// The project of an account without one: `loadCodeAssist`, saved; else the fallback.
    fn project(&self, id: &str, http: &dyn Transport, access: &str) -> String {
        let found = self.api.load_project(http, access, None, false).ok();
        let Some(project) = found.and_then(|info| info.project_id) else {
            return FALLBACK_PROJECT_ID.to_string();
        };
        if let Some(Secret::Google {
            refresh,
            access,
            expires_at,
            ..
        }) = self.pool.google_secret(id)
        {
            let project_id = Some(project.clone());
            let secret = Secret::Google {
                refresh,
                access,
                expires_at,
                project_id,
            };
            let _ = self.pool.set_google_secret(id, secret);
        }
        project
    }

    pub(super) fn candidates(&self, state: &State, now: u64) -> Vec<GoogleCandidate> {
        let accounts = self.pool.google_accounts();
        accounts
            .into_iter()
            .filter(|a| !state.dead.contains_key(&a.id))
            .map(|a| GoogleCandidate {
                quota: state.reads.get(&a.id).map(|r| r.quota.clone()),
                held_until: state.held_until(&a.id, now),
                id: a.id,
            })
            .collect()
    }

    pub(super) fn order(&self, state: &State, current: &str, model: &str) -> Vec<String> {
        let now = self.now();
        google_order(&self.candidates(state, now), current, model, now)
    }

    pub(super) fn make_active(&self, id: &str) {
        if self.pool.active_google().as_deref() != Some(id) {
            let _ = self.pool.set_active(id);
        }
    }

    pub(super) fn lease_of(&self, id: &str, access: String, project: String) -> Lease {
        let account = self.pool.google_accounts().into_iter().find(|a| a.id == id);
        let label = account
            .and_then(|a| a.email.or(a.name))
            .unwrap_or_else(|| id.to_string());
        Lease {
            account: id.to_string(),
            label,
            auth: Auth::Google { access, project },
        }
    }

    /// Every Google account with its hold, last quota read and last error.
    pub fn status(&self) -> Vec<GoogleStatus> {
        let state = self.lock();
        let now = self.now();
        let active = self.pool.active_google();
        let accounts = self.pool.google_accounts();
        accounts
            .into_iter()
            .map(|account| {
                let id = account.id.as_str();
                let read = state.reads.get(id);
                GoogleStatus {
                    active: active.as_deref() == Some(id),
                    hold: state.holds.get(id).filter(|h| h.until > now).cloned(),
                    quota: read.map(|r| r.quota.clone()),
                    read_at: read.map(|r| r.read_at),
                    last_error: state.errors.get(id).cloned(),
                    account,
                }
            })
            .collect()
    }

    /// Makes `id` the active Google account and lifts its hold (the user chose it).
    pub fn use_account(&self, id: &str) -> Result<(), PoolError> {
        if !self.pool.google_accounts().iter().any(|a| a.id == id) {
            return Err(PoolError::UnknownAccount(id.to_string()));
        }
        self.pool.set_active(id)?;
        let mut state = self.lock();
        state.holds.remove(id);
        state.forced.remove(id);
        Ok(())
    }

    /// Removes Google account `id` from the store and forgets its state.
    pub fn remove(&self, id: &str) -> Result<Account, PoolError> {
        let mut state = self.lock();
        let account = self.pool.remove(id)?;
        state.holds.remove(id);
        state.due.remove(id);
        state.backoff.remove(id);
        state.errors.remove(id);
        state.forced.remove(id);
        state.dead.remove(id);
        if state.reads.remove(id).is_some() {
            self.save_reads(&state);
        }
        Ok(account)
    }
}
