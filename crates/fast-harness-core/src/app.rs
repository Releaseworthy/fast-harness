//! Wiring shared by every front end (CLI print mode, TUI, GUI): settings,
//! credentials, the providers, the transport and sessions. The model id picks
//! the provider (PLAN.md D24): `antigravity/<id>` goes to Google Antigravity
//! with the Google accounts, any other id to Claude.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::accounts::google::GoogleAuth;
use crate::accounts::pool::AccountPool;
use crate::agent::{prompt, Agent, AgentParts, Conversation};
use crate::auth::{AuthSource, StaticAuth};
use crate::config::Config;
use crate::http::Transport;
use crate::llm::anthropic::Anthropic;
use crate::llm::antigravity::Antigravity;
use crate::llm::catalog::{provider_of, ANTIGRAVITY};
use crate::llm::{Auth, Provider};
use crate::net::UreqTransport;
use crate::{platform, session};

/// Everything a front end loads once at start.
pub struct Runtime {
    /// `~/.fast-harness` or `FAST_HARNESS_HOME`.
    pub home: PathBuf,
    pub config: Config,
    pub pool: Arc<AccountPool>,
    /// The Google accounts of the same pool (PLAN.md D23): the credential of
    /// `antigravity/<id>` models.
    pub google: Arc<GoogleAuth>,
    pub transport: Arc<UreqTransport>,
    /// Claude, for every model id without the `antigravity/` prefix.
    pub provider: Arc<Anthropic>,
    /// Google Antigravity, for `antigravity/<id>` models.
    pub antigravity: Arc<Antigravity>,
}

/// The provider and the credential source of one model.
pub type Route = (Arc<dyn Provider>, Arc<dyn AuthSource>);

/// Why an `antigravity/<id>` model cannot run: no Google account.
pub const NO_GOOGLE_ACCOUNT: &str = "no Google account for Antigravity models: add one in Accounts > Add Google account, or run `fast-harness accounts login --google`";

/// The account pool of `home`: `accounts.json` and `usage.json`.
pub fn open_pool(home: &Path) -> Result<AccountPool, String> {
    AccountPool::open(home.join("accounts.json"), home.join("usage.json"))
        .map_err(|err| format!("accounts: {err}"))
}

fn env_value(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

impl Runtime {
    /// Loads `~/.fast-harness` (or `FAST_HARNESS_HOME`): settings and the account pool.
    pub fn load() -> Result<Runtime, String> {
        let home =
            platform::app_home().ok_or("cannot find the home folder; set FAST_HARNESS_HOME")?;
        Runtime::load_at(home)
    }

    pub fn load_at(home: PathBuf) -> Result<Runtime, String> {
        let config = Config::load(&home)?;
        models::load_cache(&home);
        let pool = Arc::new(open_pool(&home)?);
        let google = Arc::new(GoogleAuth::new(Arc::clone(&pool), &home));
        Ok(Runtime {
            home,
            config,
            pool,
            google,
            transport: Arc::new(UreqTransport::new()),
            provider: Arc::new(Anthropic::default()),
            antigravity: Arc::new(Antigravity::default()),
        })
    }

    /// `ANTHROPIC_API_KEY`, else `FAST_HARNESS_OAUTH_TOKEN`, else the account pool when
    /// it holds at least one account.
    pub fn auth(&self) -> Result<Arc<dyn AuthSource>, String> {
        if let Some(key) = env_value("ANTHROPIC_API_KEY") {
            let auth = StaticAuth::new("env-api-key", "ANTHROPIC_API_KEY", Auth::ApiKey(key));
            return Ok(Arc::new(auth));
        }
        if let Some(token) = env_value("FAST_HARNESS_OAUTH_TOKEN") {
            let auth = StaticAuth::new("env-oauth", "FAST_HARNESS_OAUTH_TOKEN", Auth::OAuth(token));
            return Ok(Arc::new(auth));
        }
        if self.pool.status().is_empty() {
            return Err(
                "no account: sign in or add an API key (`fast-harness accounts`), or set ANTHROPIC_API_KEY"
                    .into(),
            );
        }
        Ok(self.pool.clone())
    }

    /// The Google accounts as a credential source; [`NO_GOOGLE_ACCOUNT`] when there is none.
    pub fn google_auth(&self) -> Result<Arc<dyn AuthSource>, String> {
        if self.google.status().is_empty() {
            return Err(NO_GOOGLE_ACCOUNT.into());
        }
        Ok(self.google.clone())
    }

    /// The provider and credential source of `model`.
    pub fn route(&self, model: &str) -> Result<Route, String> {
        if provider_of(model) == ANTIGRAVITY {
            return Ok((self.antigravity.clone(), self.google_auth()?));
        }
        Ok((self.provider.clone(), self.auth()?))
    }

    /// Agent parts for `cwd` with the current settings; the provider and the
    /// credential follow `config.model` ([`Runtime::route`]). Subagents get a
    /// copy of these parts.
    pub fn parts(&self, cwd: &Path) -> Result<AgentParts, String> {
        let (provider, auth) = self.route(&self.config.model)?;
        let transport: Arc<dyn Transport> = self.transport.clone();
        Ok(AgentParts {
            provider,
            transport,
            auth,
            config: self.config.clone(),
            system: prompt::system_prompt(cwd),
            cwd: cwd.to_path_buf(),
        })
    }

    /// An agent with the built-in tools plus the subagent tool for `conv`.
    pub fn agent(&self, cwd: &Path, conv: &Conversation) -> Result<Agent, String> {
        Ok(Agent::with_subagent(self.parts(cwd)?, &self.home, &conv.id))
    }

    /// `resume` when given, else the newest session of `cwd` when
    /// `continue_latest` (a new one when there is none), else a new session.
    pub fn open_conversation(
        &self,
        cwd: &Path,
        resume: Option<&str>,
        continue_latest: bool,
    ) -> Result<Conversation, String> {
        let id = match (resume, continue_latest) {
            (Some(id), _) => Some(id.to_string()),
            (None, true) => session::latest(&self.home, cwd)
                .map_err(|err| format!("sessions: {err}"))?
                .map(|summary| summary.id),
            (None, false) => None,
        };
        match id {
            Some(id) => Conversation::resume(&self.home, cwd, &id)
                .map_err(|err| format!("session {id}: {err}")),
            None => Conversation::create(&self.home, cwd, None)
                .map_err(|err| format!("new session: {err}")),
        }
    }
}

// Live model catalog: the caches and each provider's list (`app/models.rs`,
// `app/ag_models.rs`).
mod ag_models;
mod models;

pub use ag_models::{read_ag_cache, write_ag_cache, AG_CACHE_FILE, AG_CACHE_VERSION};
pub use models::{read_cache, write_cache, ModelSource, CACHE_FILE, CACHE_VERSION};

impl Runtime {
    /// The model catalog with the credentials `auth()` and `google_auth()`
    /// pick now. Cheap: take a new one each time a picker opens, then call
    /// `fetch` off the UI thread.
    pub fn models(&self) -> ModelSource {
        let transport: Arc<dyn Transport> = self.transport.clone();
        let source = ModelSource::new(
            self.home.clone(),
            self.auth(),
            transport,
            self.provider.clone(),
        );
        match self.google_auth() {
            Ok(google) => source.with_google(google, self.antigravity.hub.clone()),
            Err(_) => source,
        }
    }
}
