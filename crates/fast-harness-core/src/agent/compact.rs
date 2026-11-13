//! Simple compaction (PLAN.md D38), the same on every provider: one extra
//! request with the frozen system, tools and messages plus a user message
//! asking for a summary, then the history becomes the summary, the tools
//! found so far and, mid-turn, `Continue the task.`. No old thinking stays.

use crate::agent::repair;
use crate::agent::turn::{outcome_word, traced, Res, Turn, OVERFLOW_NOTE};
use crate::agent::Conversation;
use crate::auth::Lease;
use crate::config::compaction::Compaction;
use crate::events::{AgentEvent, TurnOutcome};
use crate::llm::{find, Completion};
use crate::session::compacted;
use crate::types::{Block, Message, Role, Usage};

/// The manual command of the TUI, the GUI and `fast-harness compact`.
pub const COMMAND: &str = "/compact";

/// The user message of the summary request.
pub const SUMMARY_REQUEST: &str = "[compaction] Summarize this session so far. The summary replaces the whole conversation: you will continue from it alone, with no earlier messages, tool results or thinking. Write plain text under these headings:
## Task: what the user asked for, with their exact words where they matter.
## Decisions: what was decided and why.
## Files: every file read, created or changed, with its full path and what changed.
## Current state: what is done, what is in progress, what failed and the last error.
## Next action: the exact next step.
## Durable facts: facts worth keeping beyond this session (user preferences, project conventions, commands that work), or `none`.
Do not call any tool: answer with the summary text only.";

/// The error result of a tool call made by the summary answer.
pub const NO_TOOLS: &str =
    "no tool calls while summarizing: answer with the summary text only, without calling tools";

/// `Some(instructions)` when `text` is `/compact [instructions]`.
pub(super) fn command(text: &str) -> Option<Option<&str>> {
    let rest = text.trim().strip_prefix(COMMAND)?;
    if !(rest.is_empty() || rest.starts_with(char::is_whitespace)) {
        return None;
    }
    let rest = rest.trim();
    Some((!rest.is_empty()).then_some(rest))
}

struct Summary {
    text: String,
    usage: Usage,
    model: String,
}

impl Turn<'_> {
    fn settings(&self) -> Compaction {
        let parts = &self.agent.parts;
        parts.config.compaction_for(parts.provider.id())
    }

    /// Compacts when the last step's context is above the threshold; only
    /// where the tool results are complete (turn start, between steps).
    pub(super) fn auto_compact(&mut self, mid: bool) -> Res<Option<TurnOutcome>> {
        let settings = self.settings();
        if !settings.auto || self.compact_failed || self.conv.compacted {
            return Ok(None);
        }
        let config = &self.agent.parts.config;
        let window = self.agent.parts.catalog.context_window(&config.model);
        let limit = settings.threshold(window, u64::from(config.max_tokens));
        if limit == 0 || self.conv.last_context <= limit {
            return Ok(None);
        }
        match self.compact(&settings, None, true, mid)? {
            Ok(()) => Ok(None),
            Err(TurnOutcome::Cancelled) => Ok(Some(TurnOutcome::Cancelled)),
            Err(_) => {
                self.compact_failed = true;
                Ok(None)
            }
        }
    }

    /// `/compact [instructions]`: a turn that only compacts.
    pub(super) fn manual_compact(&mut self, instructions: Option<&str>) -> Res<TurnOutcome> {
        if self.conv.messages.is_empty() || self.conv.system.is_none() {
            (self.sink)(AgentEvent::Error("nothing to compact yet".into()));
            return Ok(TurnOutcome::Done);
        }
        let settings = self.settings();
        Ok(match self.compact(&settings, instructions, false, false)? {
            Ok(()) => TurnOutcome::Done,
            Err(outcome) => outcome,
        })
    }

    /// A context-size error: compacts once and sends the step again.
    pub(super) fn after_overflow(
        &mut self,
        step: u32,
    ) -> Res<Result<(Completion, Lease), TurnOutcome>> {
        let mut called = Err(TurnOutcome::ContextOverflow);
        if !self.overflowed {
            self.overflowed = true;
            let settings = self.settings();
            match self.compact(&settings, None, true, true)? {
                Ok(()) => called = self.call_model(step)?,
                Err(TurnOutcome::Cancelled) => return Ok(Err(TurnOutcome::Cancelled)),
                Err(_) => {}
            }
        }
        if matches!(called, Err(TurnOutcome::ContextOverflow)) {
            let text = self.last_error.take();
            let text = text.unwrap_or_else(|| OVERFLOW_NOTE.into());
            (self.sink)(AgentEvent::Error(text));
        }
        Ok(called)
    }

    /// The answer stopped because the window is full: compacts once and
    /// goes on with the next step.
    pub(super) fn window_full(&mut self) -> Res<Option<TurnOutcome>> {
        if !self.overflowed {
            self.overflowed = true;
            let settings = self.settings();
            match self.compact(&settings, None, true, true)? {
                Ok(()) => return Ok(None),
                Err(TurnOutcome::Cancelled) => return Ok(Some(TurnOutcome::Cancelled)),
                Err(_) => {}
            }
        }
        (self.sink)(AgentEvent::Error(OVERFLOW_NOTE.into()));
        Ok(Some(TurnOutcome::ContextOverflow))
    }

    /// Summarizes and replaces the history; `Err` when the summary request
    /// failed (shown as an error unless cancelled).
    fn compact(
        &mut self,
        settings: &Compaction,
        instructions: Option<&str>,
        auto: bool,
        mid: bool,
    ) -> Res<Result<(), TurnOutcome>> {
        // PreCompact hooks (PLAN.md D44): a block skips the compaction.
        let hooked = match self.pre_compact(settings, instructions, auto)? {
            Ok(hooked) => hooked,
            Err(outcome) => return Ok(Err(outcome)),
        };
        let instructions = hooked.as_deref();
        let summary = match self.summarize(settings, instructions)? {
            Ok(summary) => summary,
            Err(outcome) => {
                if outcome != TurnOutcome::Cancelled {
                    let why = self.last_error.take();
                    let why = why.unwrap_or_else(|| outcome_word(outcome).into());
                    (self.sink)(AgentEvent::Error(format!("compaction failed: {why}")));
                }
                return Ok(Err(outcome));
            }
        };
        self.replace_history(&summary, instructions, auto, mid)?;
        Ok(Ok(()))
    }

    fn summarize(
        &mut self,
        settings: &Compaction,
        instructions: Option<&str>,
    ) -> Res<Result<Summary, TurnOutcome>> {
        repair(&mut self.conv.messages);
        let base = self.conv.messages.len();
        let text = request_text(settings, instructions);
        self.conv.messages.push(Message::user(text));
        let config = &self.agent.parts.config;
        let model = settings
            .model
            .clone()
            .unwrap_or_else(|| config.model.clone());
        self.request = Some((model.clone(), settings.effort.clone()));
        self.quiet = true;
        let answer = self.ask(&model);
        self.quiet = false;
        self.request = None;
        self.conv.messages.truncate(base);
        Ok(answer?.map(|(text, usage)| Summary { text, usage, model }))
    }

    /// The summary request; once more, with error results, when the answer
    /// calls a tool.
    fn ask(&mut self, model: &str) -> Res<Result<(String, Usage), TurnOutcome>> {
        let mut usage = Usage::default();
        for round in 0..2 {
            let (mut done, lease) = match self.call_model(self.steps)? {
                Ok(called) => called,
                Err(outcome) => return Ok(Err(outcome)),
            };
            self.agent.parts.auth.observe(&lease, &done.head);
            usage.add(&done.usage);
            let text = done.message.text();
            let calls: Vec<String> = (done.message.tool_calls())
                .map(|(id, _, _)| id.to_string())
                .collect();
            if calls.is_empty() || round == 1 {
                if text.trim().is_empty() {
                    self.last_error = Some("the summary was empty".into());
                    return Ok(Err(TurnOutcome::Failed));
                }
                return Ok(Ok((text, usage)));
            }
            done.message.model = Some(model.to_string());
            self.conv.messages.push(done.message);
            self.conv.messages.push(refusals(&calls));
        }
        Ok(Err(TurnOutcome::Failed))
    }

    /// Appends the summary's durable facts to the workspace memory (PLAN.md
    /// D50); a top-level session only (a child has no context). A failed
    /// write is an error event, the compaction still stands.
    fn remember_facts(&mut self, summary: &str) {
        let Some(memory) = self.agent.context.as_ref().and_then(|c| c.memory.as_ref()) else {
            return;
        };
        let date = super::prompt::today();
        if let Err(err) = super::memory::append_facts(&memory.dir, &facts(summary), &date) {
            (self.sink)(AgentEvent::Error(format!("memory: {err}")));
        }
    }

    fn replace_history(
        &mut self,
        summary: &Summary,
        instructions: Option<&str>,
        auto: bool,
        mid: bool,
    ) -> Res<()> {
        let loaded = find::found(&self.conv.messages);
        let call_id = format!("compact_{}_{}", self.turn, self.steps);
        let kept = super::nested::instruction_lines(&self.conv.messages);
        let history = compacted::history(&summary.text, &loaded, &kept, &call_id, mid);
        let before = self.conv.last_context;
        let after = estimate(self.conv, &history);
        let data = crate::trace::record::Compaction {
            turn: Some(self.turn),
            step: Some(self.steps),
            summary: summary.text.clone(),
            facts: Some(facts(&summary.text)),
            loaded: Some(loaded.clone()),
            kept: Some(kept),
            call_id: Some(call_id),
            mid: Some(mid),
            before: Some(before),
            after: Some(after),
            usage: Some(summary.usage),
            auto: Some(auto),
            model: Some(Some(summary.model.clone())),
            instructions: Some(instructions.map(str::to_string)),
        };
        traced(self.conv.trace.compaction(data))?;
        self.remember_facts(&summary.text);
        self.conv.messages = history;
        self.conv.last_context = after;
        self.conv.compacted = true;
        self.conv.tool_session.clear_reads();
        self.usage.add(&summary.usage);
        (self.sink)(AgentEvent::Compaction {
            summary: summary.text.clone(),
            before,
            after,
            auto,
        });
        Ok(())
    }
}

pub(super) fn request_text(settings: &Compaction, instructions: Option<&str>) -> String {
    let mut text = SUMMARY_REQUEST.to_string();
    let extra = [settings.instructions.as_deref(), instructions];
    for line in extra.into_iter().flatten() {
        text.push_str("\n\n");
        text.push_str(line.trim());
    }
    text
}

/// One error result per call of the summary answer.
fn refusals(calls: &[String]) -> Message {
    let content = calls
        .iter()
        .map(|id| Block::ToolResult {
            call_id: id.clone(),
            content: NO_TOOLS.into(),
            is_error: true,
        })
        .collect();
    Message {
        role: Role::Tool,
        content,
        model: None,
    }
}

/// The `Durable facts` section of a summary, empty when it has none
/// (kept apart in the event for the memory note of phase 20).
pub fn facts(summary: &str) -> String {
    let lower = summary.to_ascii_lowercase();
    let Some(start) = lower.find("# durable facts") else {
        return String::new();
    };
    let rest = &summary[start..];
    let body = rest.split_once('\n').map_or("", |(_, body)| body);
    let heading = rest.lines().next().unwrap_or_default();
    let inline = heading.split_once(':').map_or("", |(_, text)| text.trim());
    let end = body.find("\n#").unwrap_or(body.len());
    let text = format!("{inline}\n{}", &body[..end]);
    text.trim().to_string()
}

/// Tokens of the new prompt, estimated at 4 bytes a token until the next
/// step measures it.
fn estimate(conv: &Conversation, history: &[Message]) -> u64 {
    let system = conv.system.as_ref().map_or(0, String::len);
    let tools: usize = conv
        .tools
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|spec| spec.name.len() + spec.description.len() + spec.schema.to_string().len())
        .sum();
    let messages: usize = history
        .iter()
        .map(|message| serde_json::to_string(message).map_or(0, |text| text.len()))
        .sum();
    ((system + tools + messages) / 4) as u64
}
