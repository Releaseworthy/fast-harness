//! The window as the host of its sessions' turns (PLAN.md D59). The GUI's
//! runtime lives on the UI thread (every page reads it), so the agent of a
//! turn is built there, as before D59, and handed to the service's turn
//! thread through this host: `prepare` before `Service::start`, and the
//! turn thread takes it in `Host::agent`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::config::Config;
use fast_harness_core::service::Host;

pub struct GuiHost {
    home: PathBuf,
    /// Agents built for the next turn of each session, by session id.
    ready: Mutex<HashMap<String, Agent>>,
}

impl GuiHost {
    pub fn new(home: PathBuf) -> GuiHost {
        GuiHost {
            home,
            ready: Mutex::default(),
        }
    }

    /// The agent the next turn of session `id` runs with.
    pub fn prepare(&self, id: &str, agent: Agent) {
        let mut ready = self.ready.lock().unwrap_or_else(|p| p.into_inner());
        ready.insert(id.to_string(), agent);
    }

    /// Drops an agent no turn took (the start failed).
    pub fn forget(&self, id: &str) {
        let mut ready = self.ready.lock().unwrap_or_else(|p| p.into_inner());
        ready.remove(id);
    }
}

impl Host for GuiHost {
    fn home(&self) -> PathBuf {
        self.home.clone()
    }

    /// Unused by the window's turns: their agents carry the runtime's
    /// settings of the moment they were built.
    fn config(&self, _cwd: &Path) -> Result<Config, String> {
        Ok(Config::default())
    }

    fn agent(
        &self,
        _cwd: &Path,
        conv: &Conversation,
        _config: &Config,
    ) -> Result<(Agent, Vec<String>), String> {
        let mut ready = self.ready.lock().unwrap_or_else(|p| p.into_inner());
        let agent = ready.remove(&conv.id);
        agent
            .map(|agent| (agent, Vec::new()))
            .ok_or_else(|| format!("no agent was prepared for session {}", conv.id))
    }
}
