//! `send`, `ps`, `stop`, `pause`, `wait`, `tail` and `start` (PLAN.md D49):
//! sessions coordinated through their lock and inbox files, no daemon.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use fast_harness_core::agent::session_tools::lock_line;
use fast_harness_core::agent::worker;
use fast_harness_core::app::RuntimeHost;
use fast_harness_core::events::{AgentEvent, TurnOutcome};
use fast_harness_core::service::{Mode, Sent, Service};
use fast_harness_core::session::coord::{self, Command};
use fast_harness_core::session::{self, lock};
use fast_harness_core::types::Role;

use crate::args_sessions::SessionAction;
use crate::auth_choice::runtime;
use crate::tail;

/// How often `wait`, `send --wait` and `tail` look at the lock.
pub const EVERY: Duration = Duration::from_millis(250);

pub fn run(action: SessionAction, cwd: Option<PathBuf>) -> Result<ExitCode, String> {
    let home = crate::home()?;
    match action {
        SessionAction::Send {
            id,
            text,
            queue,
            wait,
        } => send(&resolve(&home, &id, cwd)?, &text, queue, wait),
        SessionAction::Ps { json } => ps(&home, json),
        SessionAction::Stop { id } => command(&home, &id, cwd, Command::Stop),
        SessionAction::Pause { id } => command(&home, &id, cwd, Command::Pause),
        SessionAction::Wait { id } => {
            let log = log_of(&home, &resolve(&home, &id, cwd)?)?;
            lock::wait(&log, EVERY, None);
            Ok(ExitCode::SUCCESS)
        }
        SessionAction::Tail { id, json } => {
            let log = log_of(&home, &resolve(&home, &id, cwd)?)?;
            tail::follow(&log, json)
        }
        SessionAction::Start { prompt, model } => {
            let dir = crate::work_dir(cwd)?;
            let launcher = worker::default_launcher();
            let (id, _pid) =
                worker::start_session(&home, &dir, &prompt, model.as_deref(), &launcher)?;
            println!("{id}");
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// `latest` is the newest session of the directory; any other id is kept.
fn resolve(home: &Path, id: &str, cwd: Option<PathBuf>) -> Result<String, String> {
    if id != "latest" {
        return Ok(id.to_string());
    }
    let dir = crate::work_dir(cwd)?;
    let latest = session::latest(home, &dir).map_err(|err| format!("sessions: {err}"))?;
    latest
        .map(|summary| summary.id)
        .ok_or_else(|| format!("no session in {}", dir.display()))
}

fn log_of(home: &Path, id: &str) -> Result<PathBuf, String> {
    lock::find(home, id).ok_or_else(|| format!("no session {id}"))
}

fn ps(home: &Path, json: bool) -> Result<ExitCode, String> {
    let running = lock::running(home);
    if json {
        let locks: Vec<&lock::Lock> = running.iter().map(|found| &found.lock).collect();
        let text = serde_json::to_string(&locks).map_err(|err| err.to_string())?;
        println!("{text}");
    } else if running.is_empty() {
        println!("no running session");
    }
    for found in running.iter().filter(|_| !json) {
        println!("{}", lock_line(&found.lock));
    }
    Ok(ExitCode::SUCCESS)
}

/// `stop` or `pause`: one inbox line for the running turn.
fn command(
    home: &Path,
    id: &str,
    cwd: Option<PathBuf>,
    command: Command,
) -> Result<ExitCode, String> {
    let id = resolve(home, id, cwd)?;
    let log = log_of(home, &id)?;
    if lock::live_lock(&log).is_none() {
        eprintln!("fast-harness: session {id} is not running");
        return Ok(ExitCode::FAILURE);
    }
    let path = coord::inbox_path(&log);
    coord::append(&path, &command).map_err(|err| format!("{}: {err}", path.display()))?;
    Ok(ExitCode::SUCCESS)
}

fn send(id: &str, text: &str, queue: bool, wait: bool) -> Result<ExitCode, String> {
    let service = Service::new(Arc::new(RuntimeHost::new(runtime()?, Vec::new())));
    let mode = if queue { Mode::Queue } else { Mode::Steer };
    let sink = Box::new(|event: AgentEvent| match event {
        AgentEvent::TextDelta(text) => {
            print!("{text}");
            let _ = std::io::stdout().flush();
        }
        AgentEvent::Error(message) => eprintln!("[error] {message}"),
        _ => {}
    });
    match service
        .send_to(id, text, mode, sink)
        .map_err(|err| format!("{err:?}"))?
    {
        Sent::Queued { pid, program } => {
            eprintln!("[note] sent to {id}, run by {program} (pid {pid})");
            if wait {
                let log = log_of(&crate::home()?, id)?;
                lock::wait(&log, EVERY, None);
                println!("{}", last_answer(&log));
            }
            Ok(ExitCode::SUCCESS)
        }
        Sent::Ran(finished) => {
            println!();
            for note in &finished.notes {
                eprintln!("[note] {note}");
            }
            let done = finished.outcome == TurnOutcome::Done;
            Ok(if done {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
    }
}

/// The text of the session's last answer, or an empty string.
pub fn last_answer(log: &Path) -> String {
    let messages = session::replay(log).unwrap_or_default();
    (messages.iter().rev())
        .filter(|m| m.role == Role::Assistant)
        .map(|m| m.text())
        .find(|text| !text.trim().is_empty())
        .unwrap_or_default()
}
