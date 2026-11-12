//! Free signals of a session log (PLAN.md D50): repeated reads of one file,
//! `not loaded yet` errors, failed edits, non-zero shell exits by command,
//! steps per turn and cache rewrites. `fast-harness reflect` reads them.

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use serde_json::Value;

/// How many names a list of counts keeps.
pub const TOP: usize = 10;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NameCount {
    pub name: String,
    pub count: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Signals {
    /// `read` calls of a path the session had read before, and the paths.
    pub repeated_reads: u64,
    pub repeated_files: Vec<NameCount>,
    /// Calls of a deferred tool before a `find_tools` loaded it.
    pub not_loaded: u64,
    /// `edit` and `write` results that are errors.
    pub failed_edits: u64,
    /// Shell results with a non-zero exit code, by program (first word).
    pub shell_failures: u64,
    pub failed_commands: Vec<NameCount>,
    /// The most steps of one turn.
    pub max_turn_steps: u64,
    /// Steps after a session's first whose cache write is at least its
    /// cache read: the cached prefix was written again.
    pub cache_rewrites: u64,
}

/// The per-session state behind [`Signals`].
#[derive(Default)]
pub(super) struct SignalPass {
    pub signals: Signals,
    reads: HashSet<String>,
    repeated: HashMap<String, u64>,
    commands: HashMap<String, u64>,
    turn_steps: u64,
    answers: u64,
}

/// The program of a shell command: its first word, without a path.
fn program(command: &str) -> String {
    let word = command.split_whitespace().next().unwrap_or("?");
    let word = word.trim_matches(['"', '\'']);
    let base = word.rsplit(['/', '\\']).next().unwrap_or(word);
    base.to_string()
}

/// The exit code a shell or command result ends with, when it has one.
pub fn exit_code(content: &str) -> Option<i64> {
    let at = content.rfind("[exit code: ")?;
    let rest = &content[at + "[exit code: ".len()..];
    rest.split(']').next()?.trim().parse().ok()
}

impl SignalPass {
    pub fn turn(&mut self) {
        self.turn_steps = 0;
    }

    pub fn step(&mut self) {
        self.turn_steps += 1;
        let max = &mut self.signals.max_turn_steps;
        *max = (*max).max(self.turn_steps);
    }

    pub fn answer(&mut self, cache_read: u64, cache_write: u64) {
        self.answers += 1;
        if self.answers > 1 && cache_write > 0 && cache_write >= cache_read {
            self.signals.cache_rewrites += 1;
        }
    }

    /// One result block of tool `name` called with `arguments` (JSON text).
    pub fn result(&mut self, name: &str, arguments: &str, content: &str, is_error: bool) {
        let args: Value = serde_json::from_str(arguments).unwrap_or(Value::Null);
        if is_error && content.contains("is not loaded yet") {
            self.signals.not_loaded += 1;
        }
        match name {
            "read" if !is_error => {
                let path = args["path"].as_str().unwrap_or("").replace('\\', "/");
                if !path.is_empty() && !self.reads.insert(path.clone()) {
                    self.signals.repeated_reads += 1;
                    *self.repeated.entry(path).or_default() += 1;
                }
            }
            "edit" | "write" if is_error => self.signals.failed_edits += 1,
            "shell" if exit_code(content).is_some_and(|code| code != 0) => {
                self.signals.shell_failures += 1;
                let command = args["command"].as_str().unwrap_or("?");
                *self.commands.entry(program(command)).or_default() += 1;
            }
            _ => {}
        }
    }

    pub fn finish(mut self) -> Signals {
        self.signals.repeated_files = top(self.repeated);
        self.signals.failed_commands = top(self.commands);
        self.signals
    }
}

fn top(counts: HashMap<String, u64>) -> Vec<NameCount> {
    let mut list: Vec<NameCount> = counts
        .into_iter()
        .map(|(name, count)| NameCount { name, count })
        .collect();
    sort(&mut list);
    list
}

fn sort(list: &mut Vec<NameCount>) {
    list.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));
    list.truncate(TOP);
}

fn merge_counts(mine: &mut Vec<NameCount>, other: Vec<NameCount>) {
    for item in other {
        match mine.iter_mut().find(|m| m.name == item.name) {
            Some(known) => known.count += item.count,
            None => mine.push(item),
        }
    }
    sort(mine);
}

impl Signals {
    pub fn merge(&mut self, other: Signals) {
        self.repeated_reads += other.repeated_reads;
        merge_counts(&mut self.repeated_files, other.repeated_files);
        self.not_loaded += other.not_loaded;
        self.failed_edits += other.failed_edits;
        self.shell_failures += other.shell_failures;
        merge_counts(&mut self.failed_commands, other.failed_commands);
        self.max_turn_steps = self.max_turn_steps.max(other.max_turn_steps);
        self.cache_rewrites += other.cache_rewrites;
    }
}
