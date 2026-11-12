//! The pool and `accounts.json` shared with other processes (PLAN.md D56):
//! every change goes through [`AccountStore::update`] (a delta on the file's
//! current content under its lock), a refresh spends the refresh token read
//! from disk inside that lock, and a file another process changed is read
//! again before a lease or a status.

use super::State;
use crate::accounts::model::{now_unix_ms, Secret};
use crate::accounts::oauth::{self, OAuthError};
use crate::accounts::store::{AccountStore, StoreError};
use crate::http::Transport;
use std::collections::HashMap;
use std::time::Duration;

/// The store lock wait before a refresh. Another process may hold the lock
/// across its own refresh, one HTTPS call of at most
/// `oauth::REQUEST_TIMEOUT_MS` (12 s), plus a read, DPAPI and a write.
pub const REFRESH_WAIT: Duration = Duration::from_secs(30);

/// How a refresh under the lock ended.
pub(super) enum Refreshed {
    /// An access token: a pair another process stored, or a new one.
    Token(String),
    /// Anthropic refused the refresh token read from disk.
    Dead,
    /// No new pair (a network error, the lock, or no own secret on disk).
    Failed,
}

/// An access token valid for longer than the refresh margin.
pub(super) fn usable(expires_at: u64) -> bool {
    expires_at > now_unix_ms().saturating_add(super::auth::REFRESH_MARGIN_MS)
}

/// Puts the refreshed pairs not yet written into `store`. A pair of an
/// account another process removed goes with the account.
fn apply(store: &mut AccountStore, unsaved: &HashMap<String, Secret>) {
    for (id, secret) in unsaved {
        let _ = store.set_secret(id, secret.clone());
    }
}

impl State {
    /// Runs `f` on the file's current content under the store lock and
    /// writes it, refreshed pairs still unsaved included. A success clears
    /// the save error; a failure with pairs unsaved sets it.
    pub(super) fn change<T>(
        &mut self,
        f: impl FnOnce(&mut AccountStore) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let unsaved = self.unsaved.clone();
        let done = self.store.update(self.wait, |store| {
            apply(store, &unsaved);
            f(store)
        });
        self.after_write(done.as_ref().err());
        done
    }

    fn after_write(&mut self, err: Option<&StoreError>) {
        match err {
            None => {
                self.unsaved.clear();
                self.save_error = None;
            }
            Some(err) if !self.unsaved.is_empty() => self.save_error = Some(err.to_string()),
            Some(_) => {}
        }
    }

    /// Before a lease or a status: writes the refreshed pairs still unsaved,
    /// else reads `accounts.json` again when another process changed it.
    pub(super) fn fresh(&mut self) {
        if !self.unsaved.is_empty() && self.change(|_| Ok(())).is_ok() {
            return;
        }
        // An unreadable file keeps the copy in memory; the next change
        // reports the error.
        if self.store.changed_on_disk() && self.store.reload().is_ok() {
            apply(&mut self.store, &self.unsaved);
        }
    }

    /// Refreshes own account `id`, whose pair in memory holds refresh token
    /// `held`, under the store lock: a pair on disk this process has not
    /// held, with a usable access token, is taken without a network call;
    /// else the refresh token on disk is spent and the new pair written
    /// before the lock goes. A failed write keeps the pair in memory and
    /// unsaved (written by the next pool call) and sets the save error.
    pub(super) fn refresh_own(&mut self, id: &str, held: &str, http: &dyn Transport) -> Refreshed {
        let (unsaved, mut fresh, mut dead) = (self.unsaved.clone(), None, false);
        let done = self.store.update(self.refresh_wait, |store| {
            apply(store, &unsaved);
            let Some(Secret::Own {
                refresh,
                access,
                expires_at,
            }) = store.secret(id)?
            else {
                return Ok(None);
            };
            if refresh != held && usable(expires_at) {
                return Ok(Some(access));
            }
            match oauth::refresh(http, &refresh) {
                Ok(tokens) => {
                    let secret = Secret::Own {
                        refresh: tokens.refresh,
                        access: tokens.access.clone(),
                        expires_at: tokens.expires_at,
                    };
                    store.set_secret(id, secret.clone())?;
                    fresh = Some(secret);
                    Ok(Some(tokens.access))
                }
                Err(OAuthError::Dead(_)) => {
                    dead = true;
                    Ok(None)
                }
                Err(_) => Ok(None),
            }
        });
        self.refreshed(id, done, fresh, dead)
    }

    /// The end of [`State::refresh_own`] once the lock is gone.
    fn refreshed(
        &mut self,
        id: &str,
        done: Result<Option<String>, StoreError>,
        fresh: Option<Secret>,
        dead: bool,
    ) -> Refreshed {
        match (done, fresh) {
            (Ok(token), _) => {
                self.after_write(None);
                match token {
                    Some(token) => Refreshed::Token(token),
                    None if dead => Refreshed::Dead,
                    None => Refreshed::Failed,
                }
            }
            // The old refresh token is spent: keep the new pair.
            (Err(err), Some(secret)) => {
                let access = match &secret {
                    Secret::Own { access, .. } => access.clone(),
                    Secret::ApiKey { .. } => return Refreshed::Failed,
                };
                let _ = self.store.set_secret(id, secret.clone());
                self.unsaved.insert(id.to_string(), secret);
                self.save_error = Some(err.to_string());
                Refreshed::Token(access)
            }
            (Err(_), None) if dead => Refreshed::Dead,
            (Err(_), None) => Refreshed::Failed,
        }
    }
}
