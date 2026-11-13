//! Step summaries (PLAN.md D73): one line per model step that says what the
//! step did in words ("Ran commands, read files, 1 failed"),
//! for views that show a step folded. A step is the run of items between a
//! user message or a step footer and the next step footer (inclusive).

use std::ops::Range;

use super::approvals::{gate_ended, WAITING};
use super::Item;

/// What one step did, counted once per step update.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StepSummary {
    /// The verbs in order of first use, DSH wording without numbers:
    /// "Thought, ran commands, read files".
    pub text: String,
    /// The same with counts: "Thought, ran 2 commands, read 1 file".
    pub counted: String,
    /// Calls that ended in an error.
    pub failed: usize,
    /// Calls the approval gate ended before they ran (D64).
    pub not_run: usize,
    /// A call is still running (or waits for an answer).
    pub running: bool,
    /// The running call: its tool name and argument summary.
    pub latest: String,
}

impl StepSummary {
    /// The whole line: the verbs, then the failures and the running call
    /// in words (never colour alone). Empty when the step only answered.
    pub fn line(&self) -> String {
        self.with(&self.text)
    }

    /// [`StepSummary::line`] with the counts, for the accessible label.
    pub fn counted_line(&self) -> String {
        self.with(&self.counted)
    }

    fn with(&self, verbs: &str) -> String {
        if verbs.is_empty() {
            return String::new();
        }
        let mut line = verbs.to_string();
        if self.failed > 0 {
            line.push_str(&format!(", {} failed", self.failed));
        }
        if self.not_run > 0 {
            line.push_str(&format!(", {} not run", self.not_run));
        }
        if self.running {
            line.push_str(&format!(", running {}", self.latest));
        }
        line
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verb {
    Thought,
    Ran,
    Read,
    Edited,
    Searched,
    Web,
    Fetched,
    Loaded,
    Created,
    Checked,
    Called,
}

/// The verb of a tool by its name; anything unknown (MCP, extensions,
/// session tools) is a plain call.
fn verb(name: &str) -> Verb {
    match name {
        "shell" | "command" => Verb::Ran,
        "read" => Verb::Read,
        "write" | "edit" => Verb::Edited,
        "grep" | "glob" | "find" => Verb::Searched,
        "web_search" => Verb::Web,
        "web_fetch" => Verb::Fetched,
        "find_tools" => Verb::Loaded,
        "subagent" => Verb::Created,
        n if n.starts_with("job_") => Verb::Checked,
        _ => Verb::Called,
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The shown phrase (DSH wording, no number) and the counted one (for the
/// accessible label and tooltip).
fn phrase(verb: Verb, n: usize) -> (&'static str, String) {
    let (shown, counted) = match verb {
        Verb::Thought => ("thought", "thought".to_string()),
        Verb::Ran => (
            "ran commands",
            format!("ran {}", plural(n, "command", "commands")),
        ),
        Verb::Read => ("read files", format!("read {}", plural(n, "file", "files"))),
        Verb::Edited => (
            "edited files",
            format!("edited {}", plural(n, "file", "files")),
        ),
        Verb::Searched => (
            "searched the code",
            format!("searched {}", plural(n, "time", "times")),
        ),
        Verb::Web => (
            "searched the web",
            format!("searched the web {}", plural(n, "time", "times")),
        ),
        Verb::Fetched => (
            "fetched pages",
            format!("fetched {}", plural(n, "page", "pages")),
        ),
        Verb::Loaded => (
            "loaded tools",
            format!("loaded {}", plural(n, "tool", "tools")),
        ),
        Verb::Created => (
            "coordinated subagents",
            format!("created {}", plural(n, "subagent", "subagents")),
        ),
        Verb::Checked => (
            "checked jobs",
            format!("checked {}", plural(n, "job", "jobs")),
        ),
        Verb::Called => (
            "called tools",
            format!("called {}", plural(n, "tool", "tools")),
        ),
    };
    (shown, counted)
}
fn count(counts: &mut Vec<(Verb, usize)>, verb: Verb) {
    match counts.iter_mut().find(|(v, _)| *v == verb) {
        Some((_, n)) => *n += 1,
        None => counts.push((verb, 1)),
    }
}

fn capital(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Summarises the items of one step, each with its approval word (D64).
pub fn summarize<'a>(items: impl IntoIterator<Item = (&'a Item, Option<&'a str>)>) -> StepSummary {
    let mut counts = Vec::new();
    let mut out = StepSummary::default();
    for (item, word) in items {
        match item {
            Item::Assistant { thinking, .. } if !thinking.trim().is_empty() => {
                count(&mut counts, Verb::Thought);
            }
            Item::Tool {
                name,
                summary,
                output,
                is_error,
                ..
            } => {
                count(&mut counts, verb(name));
                let ended = word.is_some_and(gate_ended);
                if ended {
                    out.not_run += 1;
                } else if *is_error {
                    out.failed += 1;
                } else if output.is_none() {
                    out.running = true;
                    out.latest = match word {
                        Some(WAITING) => format!("{name}, {WAITING}"),
                        _ => format!("{name} {summary}").trim().to_string(),
                    };
                }
            }
            _ => {}
        }
    }
    let (shown, counted): (Vec<_>, Vec<_>) = counts.iter().map(|(v, n)| phrase(*v, *n)).unzip();
    out.text = capital(&shown.join(", "));
    out.counted = capital(&counted.join(", "));
    out
}

/// The step around item `index`: from after the last user message or step
/// footer before it to its own step footer (inclusive) or the next user
/// message. A user message is a step of its own.
pub fn step_range(items: &[Item], index: usize) -> Range<usize> {
    let bound = |i: &Item| matches!(i, Item::User { .. } | Item::StepFooter { .. });
    let index = index.min(items.len());
    if matches!(items.get(index), Some(Item::User { .. })) {
        return index..index + 1;
    }
    let start = (items[..index].iter().rposition(bound)).map_or(0, |p| p + 1);
    let end = match items[index..].iter().position(bound) {
        Some(p) if matches!(items[index + p], Item::StepFooter { .. }) => index + p + 1,
        Some(p) => index + p,
        None => items.len(),
    };
    start..end
}

#[cfg(test)]
#[path = "steps_tests.rs"]
mod tests;
