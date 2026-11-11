//! The end of a Google sign-in: code exchange, userinfo, project and plan,
//! then a `kind: own, provider: google` account with [`Secret::Google`].

use super::{GoogleApi, GoogleClient, GoogleError, GoogleLoginFlow};
use crate::accounts::model::{now_unix_ms, Account, AccountKind, Provider, Secret};
use crate::accounts::store::AccountStore;
use crate::http::Transport;

/// A signed-in Google account, not stored yet. `Debug` hides the tokens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignIn {
    pub account: Account,
    pub secret: Secret,
}

/// The network half: exchanges `code`, reads who it is and its project and
/// plan. A failed project read leaves the project unknown, as the plugin does.
pub fn fetch_sign_in(
    api: &GoogleApi,
    http: &dyn Transport,
    client: &GoogleClient,
    flow: &GoogleLoginFlow,
    code: &str,
) -> Result<SignIn, GoogleError> {
    let tokens = api.exchange_code(http, client, flow, code)?;
    let user = api.userinfo(http, &tokens.access)?;
    let project = api
        .load_project(http, &tokens.access, None, true)
        .unwrap_or_default();
    let account = Account {
        id: Account::google_id(&user.id),
        kind: AccountKind::Own,
        provider: Provider::Google,
        email: user.email,
        name: user.name,
        plan: project.tier_name,
        added_at: now_unix_ms(),
    };
    let secret = Secret::Google {
        refresh: tokens.refresh,
        access: tokens.access,
        expires_at: tokens.expires_at,
        project_id: project.project_id,
    };
    Ok(SignIn { account, secret })
}

/// The store half: upserts the account (keeping the first `added_at`), makes
/// it the active Google account when there is none, and saves.
pub fn save_sign_in(store: &mut AccountStore, sign_in: SignIn) -> Result<Account, GoogleError> {
    let SignIn {
        mut account,
        secret,
    } = sign_in;
    if let Some(old) = store.get(&account.id) {
        account.added_at = old.added_at;
    }
    let id = account.id.clone();
    store.upsert(account, Some(secret))?;
    if store.active_for(Provider::Google).is_none() {
        store.set_active(&id)?;
    }
    store.save()?;
    store
        .get(&id)
        .cloned()
        .ok_or(GoogleError::Invalid("stored account"))
}

/// [`fetch_sign_in`] then [`save_sign_in`].
pub fn complete_sign_in(
    store: &mut AccountStore,
    api: &GoogleApi,
    http: &dyn Transport,
    client: &GoogleClient,
    flow: &GoogleLoginFlow,
    code: &str,
) -> Result<Account, GoogleError> {
    let sign_in = fetch_sign_in(api, http, client, flow, code)?;
    save_sign_in(store, sign_in)
}
