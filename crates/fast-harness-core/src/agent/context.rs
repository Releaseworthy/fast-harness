//! Context updates (PLAN.md D36): what the system prompt of a new session
//! would be built from now (the date, the prompt template and its source,
//! the AGENTS.md files), and the note a running session gets instead of a
//! changed system prompt, which stays frozen for the session.
//!
//! The session keeps a stamp of these sources (`request/header` `context`):
//! the date, a hash of the template with its source, and a hash per AGENTS.md
//! file. When the sources differ from the stamp, the next user message starts
//! with one text block of `[context update]` lines, then the stamp moves on,
//! so a resume never repeats a note.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use super::prompt::{self, MAX_AGENTS_BYTES};
use super::prompt_store::{self, Source};
use crate::types::stable_hash;

/// The first words of every context update line.
pub const PREFIX: &str = "[context update]";

/// The prompt sources of a session's workspace, read now.
#[derive(Clone, Debug)]
pub struct Context {
    /// `yyyy-mm-dd` (UTC).
    pub date: String,
    /// The template filled for this date, cwd and model (no AGENTS.md section).
    pub prompt: String,
    pub source: Source,
    /// `(path, text)` of every AGENTS.md of the project, outermost first.
    pub agents: Vec<(PathBuf, String)>,
    /// Hash key of the template: its source and its text filled with `{date}` kept.
    prompt_key: String,
}

impl Context {
    /// The sources for `cwd` and `model` today, and the notes of the prompt
    /// files skipped (unreadable).
    pub fn read(home: &Path, cwd: &Path, model: &str) -> (Context, Vec<String>) {
        Context::read_at(home, cwd, model, &prompt::today())
    }

    /// [`Context::read`] as if today were `date` (`yyyy-mm-dd`).
    pub fn read_at(home: &Path, cwd: &Path, model: &str, date: &str) -> (Context, Vec<String>) {
        let template = prompt_store::template(home, cwd);
        let values = |date: &str| {
            let os = prompt::os_name().to_string();
            [
                cwd.display().to_string(),
                os,
                date.to_string(),
                model.into(),
            ]
        };
        let keyed = prompt::fill_with(&template.text, &values("{date}"));
        let context = Context {
            date: date.to_string(),
            prompt: prompt::fill_with(&template.text, &values(date)),
            prompt_key: format!("{}\n{keyed}", template.source),
            source: template.source,
            agents: prompt::agents_files(cwd),
        };
        (context, template.notes)
    }

    /// The system prompt of a new session: the prompt, then the AGENTS.md section.
    pub fn system(&self) -> String {
        prompt::compose(self.prompt.clone(), &self.agents)
    }

    /// `{date, prompt, agents: {path: hash}}`, kept in `request/header`.
    pub fn stamp(&self) -> Value {
        let agents: Map<String, Value> = self
            .agents
            .iter()
            .map(|(path, text)| (path.display().to_string(), json!(hash(text))))
            .collect();
        json!({"date": self.date, "prompt": hash(&self.prompt_key), "agents": agents})
    }

    /// The context update for a session stamped `old`; `None` when nothing
    /// changed. A changed prompt or AGENTS.md is quoted in full (the whole
    /// note is capped at twice the AGENTS.md limit); a removed file is named.
    pub fn update(&self, old: &Value) -> Option<String> {
        let mut lines = Vec::new();
        if old["date"].as_str() != Some(self.date.as_str()) {
            lines.push(format!("{PREFIX} Today is {}.", self.date));
        }
        if old["prompt"].as_str() != Some(hash(&self.prompt_key).as_str()) {
            lines.push(format!(
                "{PREFIX} The system prompt changed ({}); follow this text from now on:\n{}",
                self.source,
                self.prompt.trim_end()
            ));
        }
        self.agents_lines(&old["agents"], &mut lines);
        if lines.is_empty() {
            return None;
        }
        Some(prompt::cap(lines.join("\n\n"), 2 * MAX_AGENTS_BYTES))
    }

    fn agents_lines(&self, old: &Value, lines: &mut Vec<String>) {
        let empty = Map::new();
        let old = old.as_object().unwrap_or(&empty);
        for (path, text) in &self.agents {
            let key = path.display().to_string();
            if old.get(&key).and_then(Value::as_str) != Some(hash(text).as_str()) {
                let verb = if old.contains_key(&key) {
                    "changed"
                } else {
                    "was added"
                };
                lines.push(format!(
                    "{PREFIX} {key} {verb}; it now reads:\n{}",
                    text.trim_end()
                ));
            }
        }
        for key in old.keys() {
            if !self
                .agents
                .iter()
                .any(|(path, _)| path.display().to_string() == *key)
            {
                lines.push(format!(
                    "{PREFIX} {key} was removed; its instructions no longer apply."
                ));
            }
        }
    }
}

fn hash(text: &str) -> String {
    format!("{:016x}", stable_hash(text))
}
