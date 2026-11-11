//! The account pool: the saved accounts, their usage and holds, behind one
//! mutex. It is the agent's [`crate::auth::AuthSource`] (`pool/auth.rs`):
//! leases a credential, records live usage, and fails over on limits and auth
//! errors with claude-switch's rules.

mod auth;
mod poll;
mod snapshot;

use crate::accounts::model::{now_unix_ms, Account, AccountKind, Secret};
use crate::accounts::oauth::{self, LoginFlow, OAuthError};
use crate::accounts::order::{Hold, Holds};
use crate::accounts::store::{AccountStore, StoreError};
use crate::accounts::usage::{current_usage, Usage};
use crate::http::Transport;
use crate::platform::{self, PlatformError};
use snapshot::Snapshot;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

pub use poll::{READ_GAP_MS, USAGE_EVERY_MS};

/// One account as a UI shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct AccountStatus {
    pub account: Account,
    pub active: bool,
    /// The last usage read, windows whose reset passed read as empty.
    pub usage: Option<Usage>,
    /// Unix milliseconds when a running hold ends.
    pub held_until: Option<i64>,
    pub hold_reason: Option<String>,
    /// A credential is on hand (an own refresh token or a key).
    pub token_ok: bool,
}

/// Errors of the pool. None carries a secret.
#[derive(Debug)]
pub enum PoolError {
    Store(StoreError),
    OAuth(OAuthError),
    Platform(PlatformError),
    UnknownAccount(String),
    EmptyKey,
}

impl fmt::Display for PoolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PoolError::Store(err) => write!(f, "{err}"),
            PoolError::OAuth(err) => write!(f, "{err}"),
            PoolError::Platform(err) => write!(f, "{err}"),
            PoolError::UnknownAccount(id) => write!(f, "no account {id}"),
            PoolError::EmptyKey => f.write_str("the API key is empty"),
        }
    }
}

impl std::error::Error for PoolError {}

impl From<StoreError> for PoolError {
    fn from(err: StoreError) -> Self {
        PoolError::Store(err)
    }
}

impl From<OAuthError> for PoolError {
    fn from(err: OAuthError) -> Self {
        PoolError::OAuth(err)
    }
}

impl From<PlatformError> for PoolError {
    fn from(err: PlatformError) -> Self {
        PoolError::Platform(err)
    }
}

/// Everything behind the pool's mutex.
struct State {
    store: AccountStore,
    snapshot: Snapshot,
    holds: HashMap<String, Hold>,
    /// Own accounts refreshed after an auth failure, until a call succeeds.
    forced: HashSet<String>,
    /// Own accounts whose refresh token Anthropic refused: never sent again.
    dead: HashSet<String>,
    /// Usage-endpoint pauses after 429 answers.
    backoff: HashMap<String, poll::Backoff>,
}

/// The saved accounts. `Send + Sync`; every method takes `&self`.
pub struct AccountPool {
    state: Mutex<State>,
    read_gap_ms: AtomicU64,
}

impl fmt::Debug for AccountPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccountPool").finish_non_exhaustive()
    }
}

/// The current time in Unix milliseconds as the order functions take it.
fn now_ms() -> i64 {
    i64::try_from(now_unix_ms()).unwrap_or(i64::MAX)
}

impl AccountPool {
    /// Opens the store at `store_path` and the usage snapshot at `usage_path` (`<app_home>/usage.json`).
    pub fn open(
        store_path: impl Into<PathBuf>,
        usage_path: impl Into<PathBuf>,
    ) -> Result<AccountPool, PoolError> {
        let state = State {
            store: AccountStore::load(store_path)?,
            snapshot: Snapshot::load(usage_path.into(), now_unix_ms()),
            holds: HashMap::new(),
            forced: HashSet::new(),
            dead: HashSet::new(),
            backoff: HashMap::new(),
        };
        Ok(AccountPool {
            state: Mutex::new(state),
            read_gap_ms: AtomicU64::new(READ_GAP_MS),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while the lock was held leaves consistent data: every
        // change is a single assignment or a store call.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Every account with its usage, hold and credential state.
    pub fn status(&self) -> Vec<AccountStatus> {
        let state = self.lock();
        let now = now_ms();
        let active = state.store.active().map(|a| a.id.clone());
        let list = state.store.list().into_iter().cloned().collect::<Vec<_>>();
        list.into_iter()
            .map(|account| {
                let hold = state.holds.get(&account.id).filter(|h| h.until > now);
                let token_ok = state.token_on_hand(&account);
                AccountStatus {
                    active: active.as_deref() == Some(account.id.as_str()),
                    usage: state
                        .snapshot
                        .usage(&account.id)
                        .map(|u| current_usage(u, now)),
                    held_until: hold.map(|h| h.until),
                    hold_reason: hold.map(|h| h.reason.clone()),
                    token_ok,
                    account,
                }
            })
            .collect()
    }

    /// Makes `id` the active account.
    pub fn set_active(&self, id: &str) -> Result<(), PoolError> {
        let mut state = self.lock();
        state.store.set_active(id)?;
        state.store.save()?;
        Ok(())
    }

    /// Removes an account and its usage.
    pub fn remove(&self, id: &str) -> Result<Account, PoolError> {
        let mut state = self.lock();
        let account = state
            .store
            .remove(id)
            .ok_or_else(|| PoolError::UnknownAccount(id.to_string()))?;
        state.store.save()?;
        state.snapshot.remove(id);
        state.holds.remove(id);
        let _ = state.snapshot.save(now_unix_ms(), true);
        Ok(account)
    }

    /// Adds (or replaces) an API key account; it becomes active when none is.
    pub fn add_api_key(&self, key: &str, label: &str) -> Result<Account, PoolError> {
        let key = key.trim();
        if key.is_empty() {
            return Err(PoolError::EmptyKey);
        }
        let label = label.trim();
        let mut state = self.lock();
        let id = Account::api_key_id(key);
        let account = Account {
            added_at: state.store.get(&id).map_or(now_unix_ms(), |a| a.added_at),
            id,
            kind: AccountKind::ApiKey,
            email: None,
            name: (!label.is_empty()).then(|| label.to_string()),
            plan: None,
        };
        let secret = Secret::ApiKey {
            key: key.to_string(),
        };
        state.store.upsert(account.clone(), Some(secret))?;
        state.activate_if_none(&account.id)?;
        state.store.save()?;
        Ok(account)
    }

    /// Signs an own account in: opens the browser when asked, waits for the
    /// callback, exchanges the code, reads the profile and saves the account.
    pub fn login(
        &self,
        http: &dyn Transport,
        open_browser: bool,
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> Result<Account, PoolError> {
        let flow = oauth::begin_login()?;
        if open_browser {
            platform::open_url(flow.url())?;
        }
        self.finish_login(http, &flow, timeout, cancel)
    }

    /// The part of [`AccountPool::login`] after the URL is shown: a caller that
    /// prints the URL itself starts the flow with `oauth::begin_login`.
    pub fn finish_login(
        &self,
        http: &dyn Transport,
        flow: &LoginFlow,
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> Result<Account, PoolError> {
        let code = flow.wait(timeout, cancel)?;
        let tokens = oauth::exchange_code(http, flow, &code)?;
        let profile = oauth::profile(http, &tokens.access)?;
        let mut state = self.lock();
        let id = Account::oauth_id(&profile.uuid);
        let account = Account {
            added_at: state.store.get(&id).map_or(now_unix_ms(), |a| a.added_at),
            id,
            kind: AccountKind::Own,
            email: profile.email,
            name: profile.name,
            plan: profile.plan,
        };
        let secret = Secret::Own {
            refresh: tokens.refresh,
            access: tokens.access,
            expires_at: tokens.expires_at,
        };
        state.store.upsert(account.clone(), Some(secret))?;
        state.activate_if_none(&account.id)?;
        state.store.save()?;
        state.holds.remove(&account.id);
        state.forced.remove(&account.id);
        state.dead.remove(&account.id);
        Ok(account)
    }

    /// Sets the pause between two usage-endpoint reads of [`AccountPool::poll_usage`].
    pub fn set_read_gap(&self, gap: Duration) {
        let ms = u64::try_from(gap.as_millis()).unwrap_or(u64::MAX);
        self.read_gap_ms
            .store(ms, std::sync::atomic::Ordering::Relaxed);
    }
}

impl State {
    fn activate_if_none(&mut self, id: &str) -> Result<(), StoreError> {
        if self.store.active().is_none() {
            self.store.set_active(id)?;
        }
        Ok(())
    }

    /// The holds in the form `order::candidates` takes.
    fn holds(&self) -> Holds {
        let mut holds = Holds::new();
        for (id, hold) in &self.holds {
            holds.hold(id, hold.until, &hold.reason);
        }
        holds
    }

    /// Holds `id` until `until`; a longer hold in place is kept.
    fn hold(&mut self, id: &str, until: i64, reason: &str) {
        if self.holds.get(id).is_some_and(|h| h.until >= until) {
            return;
        }
        let hold = Hold {
            until,
            reason: reason.to_string(),
        };
        self.holds.insert(id.to_string(), hold);
    }

    /// Whether a credential is on hand, without any network call.
    fn token_on_hand(&self, account: &Account) -> bool {
        match account.kind {
            AccountKind::Own => {
                matches!(self.store.secret(&account.id), Ok(Some(Secret::Own { .. })))
            }
            AccountKind::ApiKey => {
                matches!(
                    self.store.secret(&account.id),
                    Ok(Some(Secret::ApiKey { .. }))
                )
            }
        }
    }
}
