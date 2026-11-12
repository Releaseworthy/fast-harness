//! Sessions as a service (PLAN.md D48): a table of open sessions, each with
//! its conversation, settings, inbox sender and cancel flag, and turns run
//! on one thread each, so several sessions work at once. `fast-harness
//! serve --stdio` drives it over JSON-RPC ([`rpc`]); the GUI and the
//! session files and tools of the next phase reuse the same calls.

pub mod acp;
pub mod config;
pub mod json;
mod methods;
mod prompt;
pub mod rpc;
mod turn;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;

use serde_json::Value;

use crate::agent::{Agent, Conversation, Input};
use crate::config::Config as Settings;
use crate::events::TurnOutcome;
use crate::session::{self, SessionSummary};
use crate::stats::{self, Stats};
use crate::trace;

pub use turn::{Finished, Start};

/// What a prompt text becomes: text for the model, or a local answer
/// (`/help`) that runs no model call.
pub enum Expanded {
    Send(String),
    Reply(String),
}

/// What the service needs from the front end: settings and agents. The CLI
/// passes the `app::Runtime` (`app::RuntimeHost`), tests a scripted one.
pub trait Host: Send + Sync {
    /// `~/.fast-harness` or `FAST_HARNESS_HOME`.
    fn home(&self) -> PathBuf;
    /// The settings a session in `cwd` starts with.
    fn config(&self, cwd: &Path) -> Result<Settings, String>;
    /// The agent of one turn of `conv`, with the session's `config`, and
    /// the notes to show (an untrusted project, an unreadable prompt file).
    fn agent(
        &self,
        cwd: &Path,
        conv: &Conversation,
        config: &Settings,
    ) -> Result<(Agent, Vec<String>), String>;
    /// Model ids offered as the `model` config option.
    fn models(&self) -> Vec<String> {
        Vec::new()
    }
    /// `/help` and the user's `commands/*.md` (PLAN.md D42).
    fn expand(&self, text: &str) -> Result<Expanded, String> {
        Ok(Expanded::Send(text.to_string()))
    }
}

/// Why a call failed; [`rpc`] maps each to a JSON-RPC error code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServiceError {
    /// No open session with this id (ACP `-32002`).
    Unknown(String),
    /// The session runs a turn and the call needs an idle one.
    Busy(String),
    /// Bad input: an unknown settings key, a relative path (`-32602`).
    Invalid(String),
    /// A file or the host failed.
    Failed(String),
}

/// One open session.
pub struct Session {
    pub id: String,
    pub cwd: PathBuf,
    state: Mutex<State>,
    /// Ids of prompts queued while a turn runs (see [`acp::Queued`]).
    pub queued: acp::Queued,
}

struct State {
    /// `None` while a turn runs: its thread holds the conversation.
    conv: Option<Conversation>,
    config: Settings,
    inbox: Option<Sender<Input>>,
    cancel: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    last: Option<TurnOutcome>,
}

impl Session {
    fn new(conv: Conversation, cwd: &Path, config: Settings) -> Session {
        Session {
            id: conv.id.clone(),
            cwd: cwd.to_path_buf(),
            state: Mutex::new(State {
                conv: Some(conv),
                config,
                inbox: None,
                cancel: Arc::default(),
                worker: None,
                last: None,
            }),
            queued: Arc::default(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// A turn runs now.
    pub fn running(&self) -> bool {
        self.lock().conv.is_none()
    }

    /// The outcome of the last turn this process ran.
    pub fn last_outcome(&self) -> Option<TurnOutcome> {
        self.lock().last
    }

    pub fn config(&self) -> Settings {
        self.lock().config.clone()
    }

    /// Hands `input` to the running turn; false when no turn runs.
    pub fn send(&self, input: Input) -> bool {
        let state = self.lock();
        match &state.inbox {
            Some(tx) => tx.send(input).is_ok(),
            None => false,
        }
    }

    /// Cancels the running turn; false when none runs.
    pub fn cancel(&self) -> bool {
        let state = self.lock();
        if state.conv.is_some() {
            return false;
        }
        state
            .cancel
            .store(true, std::sync::atomic::Ordering::SeqCst);
        true
    }

    /// Waits for the turn thread, if any, to end.
    pub fn wait(&self) {
        let worker = self.lock().worker.take();
        if let Some(worker) = worker {
            let _ = worker.join();
        }
    }

    /// `f` on the conversation of an idle session.
    pub fn with_conv<T>(&self, f: impl FnOnce(&Conversation) -> T) -> Result<T, ServiceError> {
        let state = self.lock();
        let conv = state.conv.as_ref().ok_or_else(|| busy(&self.id))?;
        Ok(f(conv))
    }
}

fn busy(id: &str) -> ServiceError {
    ServiceError::Busy(format!("session {id} is running a turn"))
}

/// The open sessions and the host that builds their agents.
pub struct Service {
    host: Arc<dyn Host>,
    sessions: Mutex<HashMap<String, Arc<Session>>>,
}

impl Service {
    pub fn new(host: Arc<dyn Host>) -> Service {
        Service {
            host,
            sessions: Mutex::default(),
        }
    }

    pub fn host(&self) -> &Arc<dyn Host> {
        &self.host
    }

    fn table(&self) -> MutexGuard<'_, HashMap<String, Arc<Session>>> {
        self.sessions.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn open(&self, conv: Conversation, cwd: &Path) -> Result<Arc<Session>, ServiceError> {
        let config = self.host.config(cwd).map_err(ServiceError::Failed)?;
        let session = Arc::new(Session::new(conv, cwd, config));
        self.table()
            .insert(session.id.clone(), Arc::clone(&session));
        Ok(session)
    }

    /// A new session in `cwd` (an existing directory).
    pub fn create(&self, cwd: &Path) -> Result<Arc<Session>, ServiceError> {
        check_dir(cwd)?;
        let conv = Conversation::create(&self.host.home(), cwd, None)
            .map_err(|err| ServiceError::Failed(format!("new session: {err}")))?;
        self.open(conv, cwd)
    }

    /// Session `id` of `cwd`: the open one, else reopened from its log.
    pub fn resume(&self, cwd: &Path, id: &str) -> Result<Arc<Session>, ServiceError> {
        check_dir(cwd)?;
        if let Some(open) = self.table().get(id) {
            return Ok(Arc::clone(open));
        }
        let conv = Conversation::resume(&self.host.home(), cwd, id)
            .map_err(|err| ServiceError::Unknown(format!("session {id}: {err}")))?;
        self.open(conv, cwd)
    }

    pub fn get(&self, id: &str) -> Result<Arc<Session>, ServiceError> {
        let table = self.table();
        let session = table.get(id).ok_or_else(|| unknown(id))?;
        Ok(Arc::clone(session))
    }

    /// The open sessions.
    pub fn open_sessions(&self) -> Vec<Arc<Session>> {
        self.table().values().cloned().collect()
    }

    /// Cancels the session's turn, waits for it, and forgets the session
    /// (its log stays). `None` when it was not open.
    pub fn close(&self, id: &str) -> Option<Arc<Session>> {
        let session = self.table().remove(id)?;
        session.cancel();
        session.wait();
        Some(session)
    }

    /// The saved sessions of `cwd`, newest first.
    pub fn list(&self, cwd: &Path) -> Result<Vec<SessionSummary>, ServiceError> {
        session::list(&self.host.home(), cwd).map_err(|err| ServiceError::Failed(err.to_string()))
    }

    /// Sets settings `key` of session `id` only; returns the new settings.
    /// A change reaches the next turn (a running one keeps its agent).
    pub fn set_config(&self, id: &str, key: &str, value: &Value) -> Result<Settings, ServiceError> {
        let session = self.get(id)?;
        let mut state = session.lock();
        let next = config::set_key(&state.config, key, value).map_err(ServiceError::Invalid)?;
        state.config = next.clone();
        Ok(next)
    }

    /// The context and cache numbers of session `id` (PLAN.md D35).
    pub fn stats(&self, id: &str) -> Result<Stats, ServiceError> {
        let session = self.get(id)?;
        let path = session::log_path(&self.host.home(), &session.cwd, id);
        let (_, events) =
            trace::read(&path).map_err(|err| ServiceError::Failed(format!("{id}: {err}")))?;
        Ok(stats::compute(id, &events))
    }
}

fn unknown(id: &str) -> ServiceError {
    ServiceError::Unknown(format!("no open session {id}"))
}

fn check_dir(cwd: &Path) -> Result<(), ServiceError> {
    if !cwd.is_absolute() || !cwd.is_dir() {
        let shown = cwd.display();
        return Err(ServiceError::Invalid(format!(
            "cwd must be an absolute directory: {shown}"
        )));
    }
    Ok(())
}
