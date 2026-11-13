//! The [`Runtime`] as the host of served sessions (PLAN.md D48) and of the
//! TUI's turns (PLAN.md D59).

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use super::{read_cache, Runtime, Slash};
use crate::agent::{Agent, Conversation};
use crate::config::Config;
use crate::service::{Expanded, Host};

/// The runtime behind a lock, its settings loaded for one project at a
/// time; `cli` (the `--model` and `--effort` flags) stays the top layer.
/// Lock order: `runtime`, then `project`.
pub struct RuntimeHost {
    home: PathBuf,
    runtime: Mutex<Runtime>,
    /// The project whose settings `runtime` holds.
    project: Mutex<Option<PathBuf>>,
    cli: Vec<(String, String)>,
}

impl RuntimeHost {
    pub fn new(runtime: Runtime, cli: Vec<(String, String)>) -> RuntimeHost {
        RuntimeHost {
            home: runtime.home.clone(),
            runtime: Mutex::new(runtime),
            project: Mutex::new(None),
            cli,
        }
    }

    /// A front end of one project (the TUI): `runtime` holds the settings
    /// of `cwd` already (`Runtime::use_project`) and keeps them.
    pub fn at_project(runtime: Runtime, cwd: &Path) -> RuntimeHost {
        let host = RuntimeHost::new(runtime, Vec::new());
        *host.project.lock().unwrap_or_else(|p| p.into_inner()) = Some(cwd.to_path_buf());
        host
    }

    /// The runtime, for a front end that shares it with its turns.
    pub fn runtime(&self) -> MutexGuard<'_, Runtime> {
        self.runtime.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The runtime with the settings of `cwd`'s project.
    fn at(&self, cwd: &Path) -> Result<MutexGuard<'_, Runtime>, String> {
        let mut runtime = self.runtime();
        let mut project = self.project.lock().unwrap_or_else(|p| p.into_inner());
        if project.as_deref() != Some(cwd) {
            runtime.use_project(cwd)?;
            for (key, value) in &self.cli {
                runtime.set_cli(key, value);
            }
            *project = Some(cwd.to_path_buf());
        }
        drop(project);
        Ok(runtime)
    }
}

impl Host for RuntimeHost {
    fn home(&self) -> PathBuf {
        self.home.clone()
    }

    fn config(&self, cwd: &Path) -> Result<Config, String> {
        Ok(self.at(cwd)?.config.clone())
    }

    fn agent(
        &self,
        cwd: &Path,
        conv: &Conversation,
        config: &Config,
    ) -> Result<(Agent, Vec<String>), String> {
        self.at(cwd)?.agent_with(cwd, conv, config)
    }

    fn catalog(&self, cwd: &Path) -> crate::llm::Catalog {
        self.at(cwd)
            .map(|rt| rt.catalog.clone())
            .unwrap_or_default()
    }

    fn models(&self) -> Vec<String> {
        let cached = read_cache(&self.home).unwrap_or_default();
        cached.into_iter().map(|model| model.id).collect()
    }

    fn expand(&self, text: &str) -> Result<Expanded, String> {
        match self.runtime().slash(text) {
            Slash::Send(text) => Ok(Expanded::Send(text)),
            Slash::Help(help) => Ok(Expanded::Reply(help)),
            Slash::Error(err) => Err(err),
        }
    }
}
