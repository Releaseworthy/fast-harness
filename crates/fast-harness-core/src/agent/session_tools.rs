//! `sessions_list`, `session_start`, `session_send`, `session_read` and
//! `session_stop` (PLAN.md D49): the model drives other sessions through
//! their lock and inbox files; a new or idle session runs in a detached
//! worker ([`super::worker`]). Deferred like every tool (D30).

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::worker::{start_session, worker_exe, Launch, Launcher};
use crate::llm::ToolSpec;
use crate::session::coord::{self, Command};
use crate::session::lock::{self, Lock};
use crate::session::{self as sessions};
use crate::tools::{Effect, Tool, ToolCtx, ToolOutput};
use crate::types::Role;

pub const LIST: &str = "sessions_list";
pub const START: &str = "session_start";
pub const SEND: &str = "session_send";
pub const READ: &str = "session_read";
pub const STOP: &str = "session_stop";
/// Sessions of the working directory `sessions_list` shows.
const RECENT: usize = 10;

#[derive(Clone, Copy)]
enum Kind {
    List,
    Start,
    Send,
    Read,
    Stop,
}

struct SessionTool {
    kind: Kind,
    home: PathBuf,
    launcher: Launcher,
}

/// The five tools over the sessions under `home`; workers start through `launcher`.
pub fn all(home: &Path, launcher: Launcher) -> Vec<Box<dyn Tool>> {
    [Kind::List, Kind::Start, Kind::Send, Kind::Read, Kind::Stop]
        .into_iter()
        .map(|kind| {
            let (home, launcher) = (home.to_path_buf(), launcher.clone());
            Box::new(SessionTool {
                kind,
                home,
                launcher,
            }) as Box<dyn Tool>
        })
        .collect()
}

fn schema(properties: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": properties, "required": required})
}

fn id_prop() -> Value {
    json!({"type": "string", "description": "A session id (sessions_list)."})
}

impl Tool for SessionTool {
    /// Listing and reading only read lock and session files; start, send
    /// and stop act on other sessions.
    fn effect(&self) -> Effect {
        match self.kind {
            Kind::List | Kind::Read => Effect::Read,
            Kind::Start | Kind::Send | Kind::Stop => Effect::Run,
        }
    }

    fn spec(&self) -> ToolSpec {
        let (name, description, schema) = match self.kind {
            Kind::List => (LIST, "Running Fast Harness sessions (any directory) and the recent sessions of the working directory.", schema(json!({}), &[])),
            Kind::Start => (START, "Start a new session in a detached worker process running `prompt`; returns its id at once. Read its answer with session_read.", schema(json!({"prompt": {"type": "string"}, "cwd": {"type": "string", "description": "Working directory (default this one)."}, "model": {"type": "string"}}), &["prompt"])),
            Kind::Send => (SEND, "Send text to a session: a running turn reads it at its next step (`mode` steer, the default) or after it ends (`queue`); an idle session runs it as a new turn in a worker.", schema(json!({"id": id_prop(), "text": {"type": "string"}, "mode": {"type": "string", "enum": ["steer", "queue"]}}), &["id", "text"])),
            Kind::Read => (READ, "A session's state and its last answer.", schema(json!({"id": id_prop()}), &["id"])),
            Kind::Stop => (STOP, "Stop a session's running turn at its next step.", schema(json!({"id": id_prop()}), &["id"])),
        };
        ToolSpec {
            name: name.into(),
            description: description.into(),
            schema,
        }
    }

    fn run(&self, args: &Value, ctx: &ToolCtx<'_>) -> ToolOutput {
        let result = match self.kind {
            Kind::List => Ok(self.list(ctx.cwd)),
            Kind::Start => self.start(args, ctx.cwd),
            Kind::Send => self.send(args),
            Kind::Read => self.read(args),
            Kind::Stop => self.stop(args),
        };
        result.map_or_else(ToolOutput::err, ToolOutput::ok)
    }
}

fn text_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    match args[key].as_str().map(str::trim) {
        Some(text) if !text.is_empty() => Ok(text),
        _ => Err(format!("`{key}` must be a non-empty string")),
    }
}

/// `id  state  turn t step s  pid p  cwd` of a live lock.
pub fn lock_line(lock: &Lock) -> String {
    let (id, state, turn, step) = (&lock.id, &lock.state, lock.turn, lock.step);
    let (pid, program, cwd) = (lock.pid, &lock.program, &lock.cwd);
    format!("{id}  {state}  turn {turn} step {step}  pid {pid} ({program})  {cwd}")
}

impl SessionTool {
    fn list(&self, cwd: &Path) -> String {
        let running = lock::running(&self.home);
        let mut out = String::from("running:\n");
        if running.is_empty() {
            out.push_str("  (none)\n");
        }
        for found in &running {
            out.push_str(&format!("  {}\n", lock_line(&found.lock)));
        }
        out.push_str(&format!("recent in {}:\n", cwd.display()));
        for summary in sessions::list(&self.home, cwd)
            .unwrap_or_default()
            .iter()
            .take(RECENT)
        {
            let (id, turns, title) = (&summary.id, summary.turns, &summary.title);
            out.push_str(&format!("  {id}  {turns} turns  {title}\n"));
        }
        out
    }

    fn start(&self, args: &Value, cwd: &Path) -> Result<String, String> {
        let prompt = text_arg(args, "prompt")?;
        let dir = match args["cwd"].as_str().filter(|d| !d.trim().is_empty()) {
            Some(dir) => cwd.join(dir),
            None => cwd.to_path_buf(),
        };
        let model = args["model"].as_str();
        let (id, pid) = start_session(&self.home, &dir, prompt, model, &self.launcher)?;
        Ok(format!(
            "started session {id} (worker pid {pid}); session_read {id} reads its answer"
        ))
    }

    fn log(&self, args: &Value) -> Result<(String, PathBuf), String> {
        let id = text_arg(args, "id")?;
        let log = lock::find(&self.home, id).ok_or_else(|| format!("no session `{id}`"))?;
        Ok((id.to_string(), log))
    }

    fn send(&self, args: &Value) -> Result<String, String> {
        let (id, log) = self.log(args)?;
        let text = text_arg(args, "text")?.to_string();
        if lock::live_lock(&log).is_some() {
            let queue = args["mode"].as_str() == Some("queue");
            let command = if queue {
                Command::FollowUp(text)
            } else {
                Command::Steer(text)
            };
            let path = coord::inbox_path(&log);
            coord::append(&path, &command).map_err(|err| format!("{}: {err}", path.display()))?;
            return Ok(format!("sent to the running session {id}"));
        }
        let header = sessions::read_header(&log).ok_or_else(|| format!("{id}: no header"))?;
        let launch = Launch::worker(&worker_exe()?, &id, Path::new(&header.cwd), &text, None);
        let pid = (self.launcher)(&launch)?;
        Ok(format!(
            "{id} was idle: a worker (pid {pid}) runs it as a new turn"
        ))
    }

    fn read(&self, args: &Value) -> Result<String, String> {
        let (id, log) = self.log(args)?;
        let state = match lock::live_lock(&log) {
            Some(lock) => format!("{id} is running (turn {} step {})", lock.turn, lock.step),
            None => format!("{id} is idle"),
        };
        let messages = sessions::replay(&log).map_err(|err| format!("{id}: {err}"))?;
        let answer = (messages.iter().rev())
            .filter(|m| m.role == Role::Assistant)
            .map(|m| m.text())
            .find(|text| !text.trim().is_empty());
        Ok(match answer {
            Some(text) => format!("{state}; last answer:\n{text}"),
            None => format!("{state}; no answer yet"),
        })
    }

    fn stop(&self, args: &Value) -> Result<String, String> {
        let (id, log) = self.log(args)?;
        if lock::live_lock(&log).is_none() {
            return Ok(format!("{id} is not running"));
        }
        let path = coord::inbox_path(&log);
        coord::append(&path, &Command::Stop).map_err(|err| format!("{}: {err}", path.display()))?;
        Ok(format!("stop sent to {id}; its turn ends at its next step"))
    }
}
