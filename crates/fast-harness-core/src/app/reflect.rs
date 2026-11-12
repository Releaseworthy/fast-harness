//! `fast-harness reflect [ID]` and `/reflect` (PLAN.md D50) on a runtime:
//! the job is prepared on the caller's thread (log, stats, digest, route)
//! and runs on any thread (the one model call), so a front end keeps its
//! UI responsive. The proposed diff is kept next to the session log
//! (`<id>.reflect.diff`) until `/reflect apply` or `--apply` applies it.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use super::Runtime;
use crate::agent::memory::{self, Memory};
use crate::agent::{Agent, AgentParts, Conversation};
use crate::events::{AgentEvent, TurnOutcome};
use crate::reflect::{self, patch, Mode};
use crate::tools::Registry;
use crate::types::Role;
use crate::{session, stats, trace};

/// What a reflect job does with the proposal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Apply {
    /// Follow `selfImprove` (`ask`: nothing, `auto`: memory edits).
    Mode,
    /// Apply every file of the diff (`--apply`, `/reflect apply`).
    All,
}

/// A reflect run, ready to go to another thread.
pub type ReflectJob = Box<dyn FnOnce() -> Result<String, String> + Send>;

/// The pending diff of session `id`.
pub fn pending_path(home: &Path, cwd: &Path, id: &str) -> PathBuf {
    session::dir(home, cwd).join(format!("{id}.reflect.diff"))
}

struct Prepared {
    parts: AgentParts,
    home: PathBuf,
    cwd: PathBuf,
    id: String,
    prompt: String,
    mode: Mode,
    apply: Apply,
}

impl Runtime {
    /// The job of `/reflect [apply]` or `fast-harness reflect [ID]` for
    /// session `id` (the latest of `cwd` when `None`).
    pub fn reflect_job(
        &self,
        cwd: &Path,
        id: Option<&str>,
        args: &str,
    ) -> Result<ReflectJob, String> {
        let mode = Mode::of(&self.config);
        if mode == Mode::Off {
            return Err("reflect is off (\"selfImprove\": \"off\")".into());
        }
        let id = match id {
            Some(id) => id.to_string(),
            None => session::latest(&self.home, cwd)
                .map_err(|err| format!("sessions: {err}"))?
                .map(|s| s.id)
                .ok_or("no session to reflect on")?,
        };
        let (home, cwd_buf) = (self.home.clone(), cwd.to_path_buf());
        if args.trim() == "apply" {
            return Ok(Box::new(move || apply_pending(&home, &cwd_buf, &id)));
        }
        let apply = if args.trim() == "--apply" {
            Apply::All
        } else {
            Apply::Mode
        };
        let prompt = self.reflect_prompt(cwd, &id)?;
        let mut config = self.config.clone();
        config.model = reflect::pick_model(&self.config);
        config.effort = None;
        let parts = self.parts_with(cwd, reflect::SYSTEM.into(), &config)?;
        let job = Prepared {
            parts,
            home,
            cwd: cwd_buf,
            id,
            prompt,
            mode,
            apply,
        };
        Ok(Box::new(move || job.run()))
    }

    /// The stats and the digest of session `id`, and the memory index.
    fn reflect_prompt(&self, cwd: &Path, id: &str) -> Result<String, String> {
        let path = session::log_path(&self.home, cwd, id);
        let (_, events) = trace::read(&path).map_err(|err| format!("{}: {err}", path.display()))?;
        let numbers = stats::format_text(&stats::compute(id, &events));
        let index = Memory::read(&self.home, cwd).index;
        Ok(reflect::prompt(&numbers, &reflect::digest(&events), &index))
    }
}

impl Prepared {
    fn run(self) -> Result<String, String> {
        let model = self.parts.config.model.clone();
        let agent = Agent::new(self.parts, Arc::new(Registry::new()));
        let mut conv = Conversation::create(&self.home, &self.cwd, Some(&self.id))
            .map_err(|err| format!("reflect session: {err}"))?;
        let mut errors = Vec::new();
        let mut sink = |event: AgentEvent| {
            if let AgentEvent::Error(text) = event {
                errors.push(text);
            }
        };
        let outcome = agent.run_turn(&mut conv, &self.prompt, &AtomicBool::new(false), &mut sink);
        if outcome != TurnOutcome::Done {
            return Err(format!("reflect on {model} failed: {}", errors.join("; ")));
        }
        let answer = (conv.messages.iter().rev())
            .find(|m| m.role == Role::Assistant)
            .map(|m| m.text())
            .unwrap_or_default();
        finish(
            &self.home, &self.cwd, &self.id, &answer, self.mode, self.apply,
        )
    }
}

/// Keeps the diff of `answer` as pending, applies what `mode` and `apply`
/// allow, and returns the text to show.
pub fn finish(
    home: &Path,
    cwd: &Path,
    id: &str,
    answer: &str,
    mode: Mode,
    apply: Apply,
) -> Result<String, String> {
    let diff = reflect::extract_diff(answer);
    if diff.trim().is_empty() {
        return Ok(format!("reflect proposes no change.\n{}", answer.trim()));
    }
    let pending = pending_path(home, cwd, id);
    std::fs::write(&pending, &diff).map_err(|err| format!("{}: {err}", pending.display()))?;
    let memory_dir = memory::dir(home, cwd);
    let written = match (apply, mode) {
        (Apply::All, _) => patch::apply(&diff, cwd, &memory_dir, |_| true)?,
        (Apply::Mode, Mode::Auto) => {
            patch::apply(&diff, cwd, &memory_dir, patch::FilePatch::is_memory)?
        }
        _ => Vec::new(),
    };
    let mut text = format!("```diff\n{}```\n", diff);
    if written.is_empty() {
        text.push_str(
            "Nothing applied: /reflect apply (or fast-harness reflect --apply) applies it.",
        );
    } else {
        let names: Vec<String> = written.iter().map(|p| p.display().to_string()).collect();
        text.push_str(&format!("Applied: {}", names.join(", ")));
    }
    Ok(text)
}

fn apply_pending(home: &Path, cwd: &Path, id: &str) -> Result<String, String> {
    let pending = pending_path(home, cwd, id);
    let diff = std::fs::read_to_string(&pending)
        .map_err(|_| "no pending reflect diff: run /reflect first".to_string())?;
    let written = patch::apply(&diff, cwd, &memory::dir(home, cwd), |_| true)?;
    let _ = std::fs::remove_file(&pending);
    let names: Vec<String> = written.iter().map(|p| p.display().to_string()).collect();
    Ok(format!("Applied: {}", names.join(", ")))
}
