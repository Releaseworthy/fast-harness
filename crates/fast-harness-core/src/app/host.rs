//! The [`Runtime`] as the host of served sessions (PLAN.md D48).

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use super::{read_cache, Runtime, Slash};
use crate::agent::{Agent, Conversation};
use crate::config::Config;
use crate::service::{Expanded, Host};

/// The runtime behind a lock, its settings loaded for one project at a
/// time; `cli` (the `--model` and `--effort` flags) stays the top layer.
pub struct RuntimeHost {
    home: PathBuf,
    runtime: Mutex<(Runtime, Option<PathBuf>)>,
    cli: Vec<(String, String)>,
}

impl RuntimeHost {
    pub fn new(runtime: Runtime, cli: Vec<(String, String)>) -> RuntimeHost {
        RuntimeHost {
            home: runtime.home.clone(),
            runtime: Mutex::new((runtime, None)),
            cli,
        }
    }

    /// The runtime with the settings of `cwd`'s project.
    fn at(&self, cwd: &Path) -> Result<MutexGuard<'_, (Runtime, Option<PathBuf>)>, String> {
        let mut guard = self.runtime.lock().unwrap_or_else(|p| p.into_inner());
        if guard.1.as_deref() != Some(cwd) {
            guard.0.use_project(cwd)?;
            for (key, value) in &self.cli {
                guard.0.set_cli(key, value);
            }
            guard.1 = Some(cwd.to_path_buf());
        }
        Ok(guard)
    }
}

impl Host for RuntimeHost {
    fn home(&self) -> PathBuf {
        self.home.clone()
    }

    fn config(&self, cwd: &Path) -> Result<Config, String> {
        Ok(self.at(cwd)?.0.config.clone())
    }

    fn agent(
        &self,
        cwd: &Path,
        conv: &Conversation,
        config: &Config,
    ) -> Result<(Agent, Vec<String>), String> {
        self.at(cwd)?.0.agent_with(cwd, conv, config)
    }

    fn models(&self) -> Vec<String> {
        let cached = read_cache(&self.home).unwrap_or_default();
        cached.into_iter().map(|model| model.id).collect()
    }

    fn expand(&self, text: &str) -> Result<Expanded, String> {
        let guard = self.runtime.lock().unwrap_or_else(|p| p.into_inner());
        match guard.0.slash(text) {
            Slash::Send(text) => Ok(Expanded::Send(text)),
            Slash::Help(help) => Ok(Expanded::Reply(help)),
            Slash::Error(err) => Err(err),
        }
    }
}
