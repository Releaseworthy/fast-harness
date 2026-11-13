//! Print mode: one turn, text deltas on stdout, one-line notes on stderr
//! (`print_out.rs`). The turn runs through `service::Service` like every
//! other front end (PLAN.md D59), so a panic on its thread ends it `Failed`
//! instead of ending the process.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::app::Slash;
use fast_harness_core::config::Config;
use fast_harness_core::events::{AgentEvent, TurnOutcome};
use fast_harness_core::service::{
    json as events, Finished, Host, Service, ServiceError, Session, Start,
};
use fast_harness_core::{platform, session};

use crate::args::PrintArgs;
use crate::auth_choice::runtime;
use crate::print_out::{json_sink, text_sink, Out};

static CANCEL: AtomicBool = AtomicBool::new(false);
/// The session whose turn runs, for Ctrl+C.
static ACTIVE: Mutex<Option<Arc<Session>>> = Mutex::new(None);

/// First Ctrl+C cancels the turn; a second one ends the process.
fn on_ctrl_c() {
    if CANCEL.swap(true, Ordering::SeqCst) {
        std::process::exit(130);
    }
    if let Some(session) = active().as_ref() {
        session.cancel();
    }
}

fn active() -> std::sync::MutexGuard<'static, Option<Arc<Session>>> {
    ACTIVE.lock().unwrap_or_else(|p| p.into_inner())
}

pub fn run(args: PrintArgs) -> Result<ExitCode, String> {
    let mut rt = runtime()?;
    let cwd = crate::work_dir(args.cwd.clone())?;
    // The project layer, then the flags as the top layer (PLAN.md D41).
    rt.use_project(&cwd)?;
    if let Some(model) = &args.model {
        rt.set_cli("model", model);
    }
    if let Some(effort) = &args.effort {
        rt.set_cli("effort", effort);
    }
    if args.continue_latest && args.resume.is_none() {
        let latest = session::latest(&rt.home, &cwd).map_err(|err| format!("sessions: {err}"))?;
        if latest.is_none() {
            eprintln!("[note] no session to continue; starting a new one");
        }
    }
    // `/help` and the user's commands/*.md (PLAN.md D42).
    let prompt = match rt.slash(&args.prompt) {
        Slash::Send(text) => text,
        Slash::Help(help) => {
            println!("{help}");
            return Ok(ExitCode::SUCCESS);
        }
        Slash::Error(err) => return Err(err),
    };
    let conv = rt.open_conversation(&cwd, args.resume.as_deref(), args.continue_latest)?;
    // Built here, as before the service ran it: a build error ends the
    // command, and the notes come before the turn's output.
    let (agent, notes) = rt.agent(&cwd, &conv)?;
    for note in notes {
        eprintln!("[note] {note}");
    }
    if let Err(err) = platform::on_interrupt(on_ctrl_c) {
        eprintln!("[note] Ctrl+C will not cancel cleanly: {err}");
    }
    let host = PrintHost::new(rt.home.clone(), rt.config.clone(), agent);
    let outcome = drive(host, conv, &cwd, &prompt, args.json, Out::std());
    if outcome == TurnOutcome::Truncated {
        eprintln!("[note] the answer was cut at the output token limit (maxTokens); `--continue` with \"continue\" goes on");
    }
    Ok(exit_code(outcome))
}

/// The service's host in print mode: the one agent built before the turn.
pub struct PrintHost {
    home: PathBuf,
    config: Config,
    /// The agent's model facts (PLAN.md D67).
    catalog: fast_harness_core::llm::Catalog,
    agent: Mutex<Option<Agent>>,
}

impl PrintHost {
    pub fn new(home: PathBuf, config: Config, agent: Agent) -> PrintHost {
        PrintHost {
            home,
            config,
            catalog: agent.parts.catalog.clone(),
            agent: Mutex::new(Some(agent)),
        }
    }
}

impl Host for PrintHost {
    fn home(&self) -> PathBuf {
        self.home.clone()
    }

    fn config(&self, _cwd: &Path) -> Result<Config, String> {
        Ok(self.config.clone())
    }

    fn agent(
        &self,
        _: &Path,
        _: &Conversation,
        _: &Config,
    ) -> Result<(Agent, Vec<String>), String> {
        let taken = self.agent.lock().unwrap_or_else(|p| p.into_inner()).take();
        taken
            .map(|agent| (agent, Vec::new()))
            .ok_or_else(|| "print mode runs one turn".to_string())
    }

    fn catalog(&self, _cwd: &Path) -> fast_harness_core::llm::Catalog {
        self.catalog.clone()
    }
}

/// Runs the turn of `prompt` on the service's turn thread and writes it to
/// `out`; with `json` a `session` line first and the `result` line last,
/// with the usage of every turn (schema in docs/service.md).
pub fn drive(
    host: PrintHost,
    conv: Conversation,
    cwd: &Path,
    prompt: &str,
    json: bool,
    out: Out,
) -> TurnOutcome {
    let id = conv.id.clone();
    if json {
        out.line(&events::session_line(&id, &cwd.to_string_lossy()));
    }
    let started = Instant::now();
    let finished = run_turn(host, conv, cwd, prompt, json, &out);
    let outcome = finished.as_ref().map_or(TurnOutcome::Failed, |f| f.outcome);
    if json {
        let usage = finished.map(|f| f.usage).unwrap_or_default();
        let ms = started.elapsed().as_millis() as u64;
        out.line(&events::result_line(&id, outcome, &usage, ms));
    }
    outcome
}

/// The turn through the service; `None` when it could not start (the
/// reason was written as an error).
fn run_turn(
    host: PrintHost,
    conv: Conversation,
    cwd: &Path,
    prompt: &str,
    json: bool,
    out: &Out,
) -> Option<Finished> {
    let service = Service::new(Arc::new(host));
    let sink = match json {
        true => json_sink(out.clone()),
        false => text_sink(out.clone(), conv.id.clone()),
    };
    let (tx, rx) = mpsc::channel();
    let done = Box::new(move |end: &Finished| {
        let _ = tx.send(end.clone());
    });
    let started = service.open(conv, cwd).and_then(|session| {
        Ok((
            service.start(&session, Start::Text(prompt.into()), sink, done)?,
            session,
        ))
    });
    let session = match started {
        Ok((_, session)) => session,
        Err(
            ServiceError::Unknown(err)
            | ServiceError::Busy(err)
            | ServiceError::Invalid(err)
            | ServiceError::Failed(err),
        ) => {
            match json {
                true => out.line(&events::event(&AgentEvent::Error(err))),
                false => out.note(&format!("[error] {err}")),
            }
            return None;
        }
    };
    *active() = Some(Arc::clone(&session));
    if CANCEL.load(Ordering::SeqCst) {
        session.cancel();
    }
    let finished = rx.recv().ok();
    *active() = None;
    session.wait();
    finished
}

pub fn exit_code(outcome: TurnOutcome) -> ExitCode {
    match outcome {
        TurnOutcome::Done => ExitCode::SUCCESS,
        TurnOutcome::Failed
        | TurnOutcome::StepLimit
        | TurnOutcome::Truncated
        | TurnOutcome::ContextOverflow
        | TurnOutcome::Paused => ExitCode::FAILURE,
        TurnOutcome::Cancelled => ExitCode::from(130),
        // An outcome added later is not a success.
        _ => ExitCode::FAILURE,
    }
}

#[cfg(test)]
#[path = "print_tests.rs"]
mod tests;
