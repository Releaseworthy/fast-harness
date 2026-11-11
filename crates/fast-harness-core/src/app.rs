//! Wiring shared by every front end (CLI print mode, TUI, GUI): settings,
//! credentials, the provider, the transport and sessions.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::accounts::google::GoogleAuth;
use crate::accounts::pool::AccountPool;
use crate::agent::{prompt, Agent, AgentParts, Conversation};
use crate::auth::{AuthSource, StaticAuth};
use crate::config::Config;
use crate::http::Transport;
use crate::llm::anthropic::Anthropic;
use crate::llm::{Auth, Provider};
use crate::net::UreqTransport;
use crate::{platform, session};

/// Everything a front end loads once at start.
pub struct Runtime {
    /// `~/.fast-harness` or `FAST_HARNESS_HOME`.
    pub home: PathBuf,
    pub config: Config,
    pub pool: Arc<AccountPool>,
    /// The Google accounts of the same pool (PLAN.md D23); not routed yet.
    pub google: Arc<GoogleAuth>,
    pub transport: Arc<UreqTransport>,
    pub provider: Arc<Anthropic>,
}

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

    /// Agent parts for `cwd` with the current settings.
    pub fn parts(&self, cwd: &Path) -> Result<AgentParts, String> {
        let provider: Arc<dyn Provider> = self.provider.clone();
        let transport: Arc<dyn Transport> = self.transport.clone();
        Ok(AgentParts {
            provider,
            transport,
            auth: self.auth()?,
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

// Live model catalog: the cache and `GET /v1/models` (`app/models.rs`).
mod models;

pub use models::{read_cache, write_cache, ModelSource, CACHE_FILE, CACHE_VERSION};

impl Runtime {
    /// The model catalog with the credential `auth()` picks now. Cheap: take
    /// a new one each time a picker opens, then call `fetch` off the UI thread.
    pub fn models(&self) -> ModelSource {
        let transport: Arc<dyn Transport> = self.transport.clone();
        ModelSource::new(
            self.home.clone(),
            self.auth(),
            transport,
            self.provider.clone(),
        )
    }
}
