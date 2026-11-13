//! Wiring shared by every front end (CLI print mode, TUI, GUI): settings,
//! credentials, the providers, the transport and sessions. The model id picks
//! the provider (PLAN.md D24): `<provider id>/<id>` goes to that provider of
//! `providers/*.json`, any other id to Claude. `antigravity/<id>` (moved to
//! branch `antigravity`, D53) falls back to the default model with a note.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::accounts::pool::AccountPool;
use crate::agent::{prompt, Agent, AgentParts, Context, Conversation, Router};
use crate::auth::{AuthSource, StaticAuth};
use crate::config::layers::Layered;
use crate::config::Config;
use crate::http::Transport;
use crate::llm::anthropic::Anthropic;
use crate::llm::catalog::{self, Catalog};
use crate::llm::custom::ProviderFile;
use crate::llm::{Auth, Provider};
use crate::net::UreqTransport;
use crate::tools::Extensions;
use crate::{platform, session};

/// Everything a front end loads once at start.
pub struct Runtime {
    /// `~/.fast-harness` or `FAST_HARNESS_HOME`.
    pub home: PathBuf,
    pub config: Config,
    pub pool: Arc<AccountPool>,
    pub transport: Arc<UreqTransport>,
    /// Claude, for every model id no `providers/*.json` provider claims.
    pub provider: Arc<Anthropic>,
    /// The provider files of the loaded settings and the live model table
    /// (PLAN.md D67): this runtime's, never another's.
    pub catalog: Catalog,
    /// Set by the TUI and the GUI: `cacheTtl: auto` then caches the
    /// messages for 1 hour (5 minutes in print mode), PLAN.md D36.
    pub interactive: bool,
    /// The settings layers with the source of every key and the report of
    /// what loaded (PLAN.md D41).
    pub layered: Layered,
    /// The providers of `providers/*.json` with their adapters.
    custom: Vec<(Arc<dyn Provider>, ProviderFile)>,
    /// The tool, agent and command files (PLAN.md D42), indexed when the
    /// settings load.
    pub extensions: Arc<Extensions>,
    /// The TUI and GUI key bindings of `keys.json` (PLAN.md D45).
    pub keys: Arc<crate::keys::Keys>,
    /// The `panels/*.json` files (PLAN.md D46): indexed only, no process.
    pub panels: Vec<crate::panels::PanelFile>,
    /// Notes handed out once with the next agent (an untrusted project).
    notes: Mutex<Vec<String>>,
}

/// The provider and the credential source of one model.
pub type Route = (Arc<dyn Provider>, Arc<dyn AuthSource>);

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
        let loaded = settings::load_settings(&home, None, &env_value)?;
        let catalog = Catalog::default();
        models::load_cache(&home, catalog.live());
        let pool = Arc::new(open_pool(&home)?);
        let mut runtime = Runtime {
            home,
            config: loaded.layered.config.clone(),
            layered: loaded.layered.clone(),
            pool,
            transport: Arc::new(UreqTransport::new()),
            provider: Arc::new(Anthropic::default().with_live(Arc::clone(catalog.live()))),
            catalog,
            interactive: false,
            custom: Vec::new(),
            extensions: Arc::default(),
            keys: Arc::default(),
            panels: Vec::new(),
            notes: Mutex::default(),
        };
        runtime.apply(loaded);
        Ok(runtime)
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

    /// The provider and credential source of `model`.
    pub fn route(&self, model: &str) -> Result<Route, String> {
        if let Some(route) = self.custom_route(model) {
            return route;
        }
        Ok((self.provider.clone(), self.auth()?))
    }

    /// The system prompt for `cwd` and the current model (PLAN.md D31), read
    /// from the prompt files now.
    pub fn prompt(&self, cwd: &Path) -> prompt::Rendered {
        prompt::render(&self.home, cwd, &self.config.model)
    }

    /// Agent parts for `cwd` with the current settings; the provider and the
    /// credential follow `config.model` ([`Runtime::route`]). Subagents get a
    /// copy of these parts.
    pub fn parts(&self, cwd: &Path) -> Result<AgentParts, String> {
        self.parts_with(cwd, self.prompt(cwd).text, &self.config)
    }

    /// [`Runtime::parts`] with `system` as the system prompt and `config`
    /// as the settings.
    fn parts_with(
        &self,
        cwd: &Path,
        system: String,
        config: &Config,
    ) -> Result<AgentParts, String> {
        let mut config = self.usable(config);
        let (provider, auth) = self.route(&config.model)?;
        let transport: Arc<dyn Transport> = self.transport.clone();
        config.interactive = self.interactive;
        Ok(AgentParts {
            provider,
            transport,
            auth,
            config,
            system,
            cwd: cwd.to_path_buf(),
            extensions: Arc::clone(&self.extensions),
            catalog: self.catalog.clone(),
        })
    }

    /// An agent with the built-in tools plus the subagent tool for `conv`,
    /// and the notes of the prompt files skipped (an unreadable file). Front
    /// ends build one per turn. The system prompt is rendered from the prompt
    /// sources only for a new session; a session keeps the one it froze, and
    /// a later change of the sources reaches it as a context update (D36).
    pub fn agent(&self, cwd: &Path, conv: &Conversation) -> Result<(Agent, Vec<String>), String> {
        self.agent_with(cwd, conv, &self.config)
    }

    /// [`Runtime::agent`] with `config` as the settings: a served session
    /// keeps its own (PLAN.md D48).
    pub fn agent_with(
        &self,
        cwd: &Path,
        conv: &Conversation,
        config: &Config,
    ) -> Result<(Agent, Vec<String>), String> {
        let config = &self.usable(config);
        let (mut context, read_notes) = Context::read(&self.home, cwd, &config.model);
        context.skills = self.extensions.skills_line();
        context.agents_bytes = config.agents_bytes.max(1024);
        if !config.memory {
            context.memory = None;
        }
        let mut notes = self.take_notes();
        notes.extend(conv.take_notes());
        notes.extend(read_notes);
        let system = conv.system.clone().unwrap_or_else(|| context.system());
        let parts = self.parts_with(cwd, system, config)?;
        let router = Some(self.router());
        let mut agent =
            Agent::with_subagents(parts, &self.home, &conv.id, 0, router).with_context(context);
        agent.preload = self.preload(&agent, conv, config, &mut notes);
        Ok((agent, notes))
    }

    /// `tools.preload` (PLAN.md D42) for a top-level session: the names its
    /// tool list has; on a new session an unknown name is a note.
    fn preload(
        &self,
        agent: &Agent,
        conv: &Conversation,
        config: &Config,
        notes: &mut Vec<String>,
    ) -> Vec<String> {
        let specs = agent.tools.specs();
        let (mut known, unknown): (Vec<String>, Vec<String>) = (config.tools.preload.iter())
            .cloned()
            .partition(|name| specs.iter().skip(1).any(|spec| &spec.name == name));
        // The tools of `alwaysLoad` MCP servers (D47).
        for name in self.extensions.mcp_preload() {
            let listed = specs.iter().skip(1).any(|spec| spec.name == name);
            if listed && !known.contains(&name) {
                known.push(name);
            }
        }
        if !unknown.is_empty() && conv.messages.is_empty() {
            notes.push(format!(
                "tools.preload: no tool named {}",
                unknown.join(", ")
            ));
        }
        known
    }

    /// [`Runtime::route`] for subagents with another model (PLAN.md D40);
    /// the credential sources are picked now.
    pub fn router(&self) -> Router {
        let claude: Result<Route, String> = self.auth().map(|auth| {
            let provider: Arc<dyn Provider> = self.provider.clone();
            (provider, auth)
        });
        let custom: Vec<(String, Result<Route, String>)> = (self.custom.iter())
            .map(|(_, file)| {
                let model = format!("{}/", file.id);
                (
                    model.clone(),
                    self.custom_route(&model).unwrap_or_else(|| Err(model)),
                )
            })
            .collect();
        Arc::new(move |model: &str| {
            if let Some((_, route)) = custom.iter().find(|(p, _)| model.starts_with(p.as_str())) {
                return route.clone();
            }
            claude.clone()
        })
    }
    /// `resume` when given, else the newest session of `cwd` when
    /// `continue_latest` (a new one when there is none), else a new session.
    pub fn open_conversation(
        &self,
        cwd: &Path,
        resume: Option<&str>,
        continue_latest: bool,
    ) -> Result<Conversation, String> {
        Ok(self.open_with_events(cwd, resume, continue_latest)?.0)
    }

    /// [`Runtime::open_conversation`] with the events of a reopened log, from
    /// the same parse (D60); a new session has none. A view builds its
    /// transcript from them without reading the log again.
    pub fn open_with_events(
        &self,
        cwd: &Path,
        resume: Option<&str>,
        continue_latest: bool,
    ) -> Result<(Conversation, Vec<crate::trace::TraceEvent>), String> {
        let id = match (resume, continue_latest) {
            (Some(id), _) => Some(id.to_string()),
            (None, true) => session::latest(&self.home, cwd)
                .map_err(|err| format!("sessions: {err}"))?
                .map(|summary| summary.id),
            (None, false) => None,
        };
        match id {
            Some(id) => Conversation::reopen(&self.home, cwd, &id)
                .map_err(|err| format!("session {id}: {err}")),
            None => Conversation::create(&self.home, cwd, None)
                .map(|conv| (conv, Vec::new()))
                .map_err(|err| format!("new session: {err}")),
        }
    }
}

// Live model catalog: the caches and each provider's list (`app/models.rs`,
// one module).
pub mod drafts;
mod ext_list;
mod host;
mod models;
pub mod pickers;
mod plugins;
pub mod reflect;
mod settings;
mod slash;
pub mod words;

pub use ext_list::{ext_lines, ext_list};
pub use settings::{doctor, load_settings, mcp_lines, provider_for, Settings};
pub use slash::{expand_body, help, reflect_args, slash, undo_args, Slash};

pub use host::RuntimeHost;

pub use models::{custom_models, read_cache, write_cache, ModelSource, CACHE_FILE, CACHE_VERSION};

impl Runtime {
    /// The model catalog with the credential `auth()`
    /// pick now. Cheap: take a new one each time a picker opens, then call
    /// `fetch` off the UI thread.
    pub fn models(&self) -> ModelSource {
        let transport: Arc<dyn Transport> = self.transport.clone();
        ModelSource::new(
            self.home.clone(),
            self.auth(),
            transport,
            self.provider.clone(),
            self.catalog.clone(),
        )
    }
}

impl Runtime {
    /// `config` with a retired `antigravity/<id>` model (D53) replaced by the
    /// default model; the note says so once.
    pub fn usable(&self, config: &Config) -> Config {
        let mut config = config.clone();
        let fallback = Config::default().model;
        if let Some(note) = catalog::retired_note(&config.model, &fallback) {
            self.note(note);
            config.model = fallback;
        }
        config
    }

    /// Replaces a retired `config.model` (see [`Runtime::usable`]).
    pub(crate) fn retire_model(&mut self) {
        self.config = self.usable(&self.config);
    }

    /// Queues `note` for the next agent unless it is queued already.
    fn note(&self, note: String) {
        let mut notes = self.notes.lock().unwrap_or_else(|p| p.into_inner());
        if !notes.contains(&note) {
            notes.push(note);
        }
    }
}
