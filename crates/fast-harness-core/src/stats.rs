//! Context and cache numbers of session logs (`fast-harness stats`, PLAN.md
//! D35): steps, prompt tokens, cache hit rate, cache rewrites after an idle
//! gap, tool result bytes per tool, the largest and the clipped results,
//! `find_tools` steps, compactions and server input transformations. No
//! prices: they come with the model entries of a later phase.

mod format;

pub use format::format_text;

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use serde_json::Value;

use crate::llm::find::FIND_TOOLS;
use crate::trace::{kind, prefix_binding_mismatches, TraceEvent};
use crate::types::Usage;

/// The cache lives 5 minutes after its last use, or 1 hour when written so.
pub const IDLE_LIMIT_MS: u64 = 5 * 60 * 1000;
pub const IDLE_LIMIT_1H_MS: u64 = 60 * 60 * 1000;
/// How many of the largest tool results are kept.
pub const LARGEST: usize = 5;
/// The mark `tools::clip` leaves in a clipped result.
const CLIP_MARK: &str = "bytes clipped ...]";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolBytes {
    pub name: String,
    pub results: u64,
    pub bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultSize {
    pub session: String,
    pub turn: u64,
    pub step: u64,
    pub name: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub sessions: u64,
    pub turns: u64,
    pub steps: u64,
    pub usage: Usage,
    pub prompt_tokens: u64,
    /// `cache_read / prompt` in percent; `None` without any prompt.
    pub cache_hit_percent: Option<u64>,
    /// Steps that wrote the cache when the previous step had ended longer
    /// ago than the cache lifetime, and the tokens they wrote.
    pub idle_rewrites: u64,
    pub idle_rewrite_tokens: u64,
    /// The largest context of one step and the window of the last step.
    pub peak_context: u64,
    pub window: u64,
    /// Sorted by bytes, largest first.
    pub tools: Vec<ToolBytes>,
    pub largest: Vec<ResultSize>,
    pub clipped: u64,
    pub find_steps: u64,
    pub compactions: u64,
    pub transformations: u64,
    pub prefix_binding_mismatches: u64,
}

/// One pass over one session's events.
#[derive(Default)]
struct Pass {
    stats: Stats,
    session: String,
    /// When the previous step ended: the cache was last used before it.
    last_step_end: Option<u64>,
    gap_ms: Option<u64>,
    names: HashMap<String, String>,
    find_steps: HashSet<(u64, u64)>,
    tools: HashMap<String, ToolBytes>,
    results: Vec<ResultSize>,
}

fn int(data: &Value, key: &str) -> u64 {
    data[key].as_u64().unwrap_or(0)
}

/// The numbers of one session log's events; `session` names its results.
pub fn compute(session: &str, events: &[TraceEvent]) -> Stats {
    let mut pass = Pass {
        session: session.to_string(),
        ..Pass::default()
    };
    for event in events {
        pass.event(event);
    }
    pass.finish()
}

impl Pass {
    fn event(&mut self, event: &TraceEvent) {
        let data = &event.data;
        match event.kind.as_str() {
            kind::TURN_START => self.stats.turns += 1,
            kind::STEP_START => {
                self.stats.steps += 1;
                let end = self.last_step_end;
                self.gap_ms = end.map(|last| event.time.saturating_sub(last));
            }
            kind::STEP_END => self.last_step_end = Some(event.time),
            kind::ASSISTANT_MESSAGE => self.answer(data),
            kind::TOOL_CALL => self.call(data),
            kind::TOOL_RESULT => self.result(data),
            kind::COMPACTION => self.stats.compactions += 1,
            kind::REQUEST_TRANSFORMATIONS => {
                self.stats.transformations += 1;
                let mismatches = prefix_binding_mismatches(&data["transformations"]);
                self.stats.prefix_binding_mismatches += mismatches;
            }
            _ => {}
        }
    }

    fn answer(&mut self, data: &Value) {
        let usage: Usage = serde_json::from_value(data["usage"].clone()).unwrap_or_default();
        self.stats.usage.add(&usage);
        let context = data["context"].as_u64().unwrap_or(usage.context());
        self.stats.peak_context = self.stats.peak_context.max(context);
        if let Some(window) = data["window"].as_u64().filter(|w| *w > 0) {
            self.stats.window = window;
        }
        let limit = if usage.cache_write_1h > 0 {
            IDLE_LIMIT_1H_MS
        } else {
            IDLE_LIMIT_MS
        };
        if usage.cache_write > 0 && self.gap_ms.is_some_and(|gap| gap > limit) {
            self.stats.idle_rewrites += 1;
            self.stats.idle_rewrite_tokens += usage.cache_write;
        }
    }

    fn call(&mut self, data: &Value) {
        let name = data["name"].as_str().unwrap_or("?").to_string();
        if name == FIND_TOOLS {
            self.find_steps
                .insert((int(data, "turn"), int(data, "step")));
        }
        if let Some(id) = data["callId"].as_str() {
            self.names.insert(id.to_string(), name);
        }
    }

    fn result(&mut self, data: &Value) {
        let Some(blocks) = data["message"]["content"].as_array() else {
            return;
        };
        for block in blocks {
            let content = block["content"].as_str().unwrap_or("");
            let id = block["callId"].as_str().unwrap_or("");
            let name = self.names.get(id).cloned().unwrap_or_else(|| "?".into());
            let bytes = content.len() as u64;
            let entry = self.tools.entry(name.clone()).or_default();
            entry.results += 1;
            entry.bytes += bytes;
            self.stats.clipped += u64::from(content.contains(CLIP_MARK));
            self.results.push(ResultSize {
                session: self.session.clone(),
                turn: int(data, "turn"),
                step: int(data, "step"),
                name,
                bytes,
            });
        }
    }

    fn finish(mut self) -> Stats {
        self.stats.sessions = 1;
        self.stats.find_steps = self.find_steps.len() as u64;
        self.stats.tools = self
            .tools
            .into_iter()
            .map(|(name, tool)| ToolBytes { name, ..tool })
            .collect();
        self.stats.largest = self.results;
        tidy(&mut self.stats);
        self.stats
    }
}

/// Sorts the tool list, keeps the [`LARGEST`] results and sets the ratios.
fn tidy(stats: &mut Stats) {
    stats
        .tools
        .sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.name.cmp(&b.name)));
    stats
        .largest
        .sort_by_key(|result| std::cmp::Reverse(result.bytes));
    stats.largest.truncate(LARGEST);
    stats.prompt_tokens = stats.usage.prompt();
    stats.cache_hit_percent = stats.usage.cache_hit_percent();
}

impl Stats {
    /// Adds `other` (another session) to these numbers.
    pub fn merge(&mut self, other: Stats) {
        self.sessions += other.sessions;
        self.turns += other.turns;
        self.steps += other.steps;
        self.usage.add(&other.usage);
        self.idle_rewrites += other.idle_rewrites;
        self.idle_rewrite_tokens += other.idle_rewrite_tokens;
        self.peak_context = self.peak_context.max(other.peak_context);
        self.window = self.window.max(other.window);
        for tool in other.tools {
            match self.tools.iter_mut().find(|t| t.name == tool.name) {
                Some(mine) => {
                    mine.results += tool.results;
                    mine.bytes += tool.bytes;
                }
                None => self.tools.push(tool),
            }
        }
        self.largest.extend(other.largest);
        self.clipped += other.clipped;
        self.find_steps += other.find_steps;
        self.compactions += other.compactions;
        self.transformations += other.transformations;
        self.prefix_binding_mismatches += other.prefix_binding_mismatches;
        tidy(self);
    }
}
