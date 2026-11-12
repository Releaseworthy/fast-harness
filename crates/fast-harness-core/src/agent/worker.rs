//! Detached worker processes (PLAN.md D49): `fast-harness start` and the
//! `session_start` / `session_send` tools run a session's turn in a new
//! `fast-harness -p` process that outlives the caller.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use super::Conversation;
use crate::platform;

/// A worker to start: `exe args` in `cwd`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub exe: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
}

/// Starts a [`Launch`] and returns its pid; tests record instead.
pub type Launcher = Arc<dyn Fn(&Launch) -> Result<u32, String> + Send + Sync>;

impl Launch {
    /// Print mode running `prompt` as the next turn of session `id`.
    pub fn worker(exe: &Path, id: &str, cwd: &Path, prompt: &str, model: Option<&str>) -> Launch {
        let cwd_arg = cwd.display().to_string();
        let mut args: Vec<String> = vec!["-p".into(), prompt.into(), "--resume".into(), id.into()];
        args.extend(["--cwd".to_string(), cwd_arg]);
        if let Some(model) = model.filter(|m| !m.trim().is_empty()) {
            args.extend(["--model".to_string(), model.to_string()]);
        }
        Launch {
            exe: exe.to_path_buf(),
            args,
            cwd: cwd.to_path_buf(),
        }
    }

    pub fn command(&self) -> Command {
        let mut cmd = Command::new(&self.exe);
        cmd.args(&self.args).current_dir(&self.cwd);
        cmd
    }
}

/// The CLI program: the running one when it is `fast-harness`, else
/// `fast-harness` next to it (the GUI has no print mode).
pub fn worker_exe() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|err| format!("own program: {err}"))?;
    let name = format!("fast-harness{}", std::env::consts::EXE_SUFFIX);
    if exe.file_name().and_then(|n| n.to_str()) == Some(name.as_str()) {
        return Ok(exe);
    }
    let sibling = exe.with_file_name(&name);
    Ok(if sibling.is_file() { sibling } else { exe })
}

/// Starts `launch` detached (`platform::spawn_detached`); its pid.
pub fn spawn(launch: &Launch) -> Result<u32, String> {
    let mut cmd = launch.command();
    let child = platform::spawn_detached(&mut cmd)
        .map_err(|err| format!("start {}: {err}", launch.exe.display()))?;
    Ok(child.id())
}

/// The launcher of real workers: [`worker_exe`] through [`spawn`].
pub fn default_launcher() -> Launcher {
    Arc::new(spawn)
}

/// A new session of `cwd` whose first turn runs `prompt` in a worker
/// started by `launcher`; the session id and the worker's pid.
pub fn start_session(
    home: &Path,
    cwd: &Path,
    prompt: &str,
    model: Option<&str>,
    launcher: &Launcher,
) -> Result<(String, u32), String> {
    if !cwd.is_absolute() || !cwd.is_dir() {
        return Err(format!("{} is not a directory", cwd.display()));
    }
    let conv =
        Conversation::create(home, cwd, None).map_err(|err| format!("new session: {err}"))?;
    let id = conv.id.clone();
    drop(conv);
    let launch = Launch::worker(&worker_exe()?, &id, cwd, prompt, model);
    Ok((id, launcher(&launch)?))
}
