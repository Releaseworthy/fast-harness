//! Context and cache numbers of session logs (`fast-harness stats`, PLAN.md
//! D35): steps, prompt tokens, cache hit rate, cache rewrites after an idle
//! gap, tool result bytes per tool, the largest and the clipped results,
//! `find_tools` steps, compactions and server input transformations. No
//! prices: they come with the model entries of a later phase.

mod format;
pub mod signals;

pub use format::format_text;
pub use signals::{NameCount, Signals};

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::llm::find::FIND_TOOLS;
use crate::tools::budget::SPILL_MARK;
use crate::trace::record::{AssistantMessage, ToolCall, ToolResult};
use crate::trace::{prefix_binding_mismatches, Record, TraceEvent};
use crate::types::{Block, Usage};

/// The cache lives 5 minutes after its last use, or 1 hour when written so.
pub const IDLE_LIMIT_MS: u64 = 5 * 60 * 1000;
pub const IDLE_LIMIT_1H_MS: u64 = 60 * 60 * 1000;
/// How many of the largest tool results are kept.
pub const LARGEST: usize = 5;
/// The mark the 32 KB clip of trace versions 1 to 5 left in a result.
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
    /// Cost in millionths of a US dollar of the steps whose model has a
    /// `price` in `providers/*.json` (PLAN.md D41), and how many steps that is.
    pub cost_micro_usd: u64,
    pub priced_steps: u64,
    /// Free signals for `reflect` (PLAN.md D50).
    pub signals: Signals,
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
    arguments: HashMap<String, String>,
    signals: signals::SignalPass,
    find_steps: HashSet<(u64, u64)>,
    tools: HashMap<String, ToolBytes>,
    results: Vec<ResultSize>,
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
        match event.record() {
            Record::TurnStart(_) => {
                self.stats.turns += 1;
                self.signals.turn();
            }
            Record::StepStart(_) => {
                self.stats.steps += 1;
                self.signals.step();
                let end = self.last_step_end;
                self.gap_ms = end.map(|last| event.time.saturating_sub(last));
            }
            Record::StepEnd(_) => self.last_step_end = Some(event.time),
            Record::AssistantMessage(answer) => self.answer(&answer),
            Record::ToolCall(call) => self.call(&call),
            Record::ToolResult(result) => self.result(&result),
            Record::Compaction(_) => self.stats.compactions += 1,
            Record::RequestTransformations(found) => {
                self.stats.transformations += 1;
                let mismatches = prefix_binding_mismatches(&found.transformations);
                self.stats.prefix_binding_mismatches += mismatches;
            }
            _ => {}
        }
    }

    fn answer(&mut self, answer: &AssistantMessage) {
        let usage = answer.usage;
        self.stats.usage.add(&usage);
        (self.signals).answer(usage.cache_read, usage.cache_write);
        let model = answer.message.model.as_deref().unwrap_or("");
        if let Some(price) = crate::llm::custom::price(model) {
            self.stats.cost_micro_usd += (price.cost(&usage) * 1_000_000.0).round() as u64;
            self.stats.priced_steps += 1;
        }
        let context = answer.context.unwrap_or(usage.context());
        self.stats.peak_context = self.stats.peak_context.max(context);
        if let Some(window) = answer.window.filter(|w| *w > 0) {
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

    fn call(&mut self, call: &ToolCall) {
        if call.name == FIND_TOOLS {
            let (turn, step) = (u64::from(call.turn), u64::from(call.step));
            self.find_steps.insert((turn, step));
        }
        let id = call.call_id.clone();
        self.arguments.insert(id.clone(), call.arguments.clone());
        self.names.insert(id, call.name.clone());
    }

    fn result(&mut self, result: &ToolResult) {
        for block in &result.message.content {
            let Block::ToolResult {
                call_id,
                content,
                is_error,
            } = block
            else {
                continue;
            };
            let name = self
                .names
                .get(call_id)
                .cloned()
                .unwrap_or_else(|| "?".into());
            let arguments = self.arguments.get(call_id).map_or("", String::as_str);
            (self.signals).result(&name, arguments, content, *is_error);
            let bytes = content.len() as u64;
            let entry = self.tools.entry(name.clone()).or_default();
            entry.results += 1;
            entry.bytes += bytes;
            // Version 6 names the spill file of a result cut to its budget.
            let spilled = result.spilled.is_some() || content.contains(SPILL_MARK);
            self.stats.clipped += u64::from(spilled || content.contains(CLIP_MARK));
            self.results.push(ResultSize {
                session: self.session.clone(),
                turn: u64::from(result.turn),
                step: u64::from(result.step),
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
        self.stats.signals = self.signals.finish();
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
        self.cost_micro_usd += other.cost_micro_usd;
        self.priced_steps += other.priced_steps;
        self.signals.merge(other.signals);
        tidy(self);
    }
}
