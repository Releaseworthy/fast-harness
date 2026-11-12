//! A UI-independent model of the chat transcript. The TUI and the GUI both
//! render a [`Transcript`], built live from [`AgentEvent`]s with
//! [`Transcript::apply`] or from a saved conversation with
//! [`Transcript::from_messages`] and [`Transcript::from_trace`].

pub mod diff;
mod replay;
mod summary;
pub mod tool_detail;
mod tool_detail_fs;
mod tool_detail_run;

pub use summary::tool_summary;
pub use tool_detail::{
    tool_detail, Chip, Field, Line, LineKind, Tone, ToolDetail, ToolKind, MAX_LINES,
};

use crate::events::{AgentEvent, TurnOutcome};
use crate::llm::Timing;
use crate::types::{StopReason, Usage};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteKind {
    Retry,
    AccountSwitch,
    Error,
    Info,
}

/// One rendered entry of the transcript.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    User {
        text: String,
    },
    /// The text and reasoning of one step; both grow as deltas arrive.
    Assistant {
        turn: u32,
        step: u32,
        text: String,
        thinking: String,
    },
    /// `output` is `None` while the tool runs.
    Tool {
        id: String,
        name: String,
        arguments: String,
        summary: String,
        output: Option<String>,
        is_error: bool,
        duration_ms: Option<u64>,
    },
    Note {
        kind: NoteKind,
        text: String,
    },
    StepFooter {
        turn: u32,
        step: u32,
        model: String,
        account: String,
        usage: Usage,
        timing: Timing,
        stop: StopReason,
    },
}

/// What the agent is doing now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Idle,
    /// A request was sent and nothing came back yet.
    Waiting,
    Thinking,
    Responding,
    RunningTools {
        running: usize,
    },
    /// Unix milliseconds when the next attempt starts.
    Retrying {
        until_ms: u64,
    },
}

/// The transcript of one session. `revision` grows on every change, so a UI
/// redraws only when it moved.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Transcript {
    pub items: Vec<Item>,
    pub state: State,
    pub revision: u64,
    pub turn: u32,
    pub step: u32,
    pub model: String,
    pub account: String,
    pub totals: Usage,
}

pub(crate) fn retry_text(attempt: u32, delay_ms: u64, error: &str) -> String {
    format!("Attempt {attempt} failed, retrying in {delay_ms} ms: {error}")
}

pub(crate) fn switch_text(from: &str, to: &str, reason: &str) -> String {
    format!("Account {from} -> {to} ({reason})")
}

pub(crate) fn turn_end_text(outcome: &str) -> String {
    format!("Turn ended: {outcome}")
}

fn outcome_word(outcome: TurnOutcome) -> Option<&'static str> {
    match outcome {
        TurnOutcome::Done => None,
        TurnOutcome::Cancelled => Some("cancelled"),
        TurnOutcome::Failed => Some("failed"),
        TurnOutcome::StepLimit => Some("step limit"),
    }
}

impl Transcript {
    pub fn new() -> Transcript {
        Transcript::default()
    }

    pub fn push_user(&mut self, text: impl Into<String>) {
        self.items.push(Item::User { text: text.into() });
        self.bump();
    }

    /// Applies one live event, using the system clock for retry deadlines.
    pub fn apply(&mut self, event: &AgentEvent) {
        self.apply_at(event, crate::trace::now_ms());
    }

    /// [`Transcript::apply`] with `now_ms` (Unix milliseconds) as the clock.
    pub fn apply_at(&mut self, event: &AgentEvent, now_ms: u64) {
        self.bump();
        if !self.apply_stream(event) {
            self.apply_control(event, now_ms);
        }
    }

    fn bump(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    /// Text, reasoning and tool events; false for any other event.
    fn apply_stream(&mut self, event: &AgentEvent) -> bool {
        match event {
            AgentEvent::TextDelta(chunk) => self.delta(chunk, false),
            AgentEvent::ThinkingDelta(chunk) => self.delta(chunk, true),
            AgentEvent::ToolCallStart { id, name } => {
                self.items.push(new_tool(id, name, String::new()));
                self.state = State::Responding;
            }
            AgentEvent::ToolArgsDelta { id, chunk } => {
                if let Some(Item::Tool { arguments, .. }) = self.tool_mut(id) {
                    arguments.push_str(chunk);
                }
            }
            AgentEvent::ToolCall {
                id,
                name,
                arguments,
            } => self.tool_call(id, name, arguments),
            AgentEvent::ToolResult {
                id,
                content,
                is_error,
                duration_ms,
            } => {
                self.fill_result(id, content, *is_error, Some(*duration_ms));
                let running = self.running_tools();
                self.state = if running == 0 {
                    State::Waiting
                } else {
                    State::RunningTools { running }
                };
            }
            _ => return false,
        }
        true
    }

    fn apply_control(&mut self, event: &AgentEvent, now_ms: u64) {
        match event {
            AgentEvent::TurnStart { turn } => {
                self.turn = *turn;
                self.step = 0;
            }
            AgentEvent::StepStart { turn, step } => {
                self.turn = *turn;
                self.step = *step;
            }
            AgentEvent::Request { model, account, .. } => {
                self.model.clone_from(model);
                self.account.clone_from(account);
                self.state = State::Waiting;
            }
            AgentEvent::Retry {
                attempt,
                delay_ms,
                error,
            } => {
                self.note(NoteKind::Retry, retry_text(*attempt, *delay_ms, error));
                let until_ms = now_ms.saturating_add(*delay_ms);
                self.state = State::Retrying { until_ms };
            }
            AgentEvent::AccountSwitch { from, to, reason } => {
                self.note(NoteKind::AccountSwitch, switch_text(from, to, reason));
                self.account.clone_from(to);
            }
            AgentEvent::Subagent { session, running } => {
                let verb = if *running { "started" } else { "finished" };
                self.note(NoteKind::Info, format!("Subagent {session} {verb}"));
            }
            AgentEvent::StepEnd {
                turn,
                step,
                usage,
                timing,
                stop,
            } => self.footer(*turn, *step, *usage, *timing, *stop),
            AgentEvent::TurnEnd { outcome, .. } => {
                self.state = State::Idle;
                if let Some(word) = outcome_word(*outcome) {
                    self.note(NoteKind::Info, turn_end_text(word));
                }
            }
            AgentEvent::Error(text) => self.note(NoteKind::Error, text.clone()),
            _ => {}
        }
    }

    fn delta(&mut self, chunk: &str, is_thinking: bool) {
        let (turn, step) = (self.turn, self.step);
        let current = matches!(self.items.last(),
            Some(Item::Assistant { turn: t, step: s, .. }) if *t == turn && *s == step);
        if !current {
            self.items.push(Item::Assistant {
                turn,
                step,
                text: String::new(),
                thinking: String::new(),
            });
        }
        if let Some(Item::Assistant { text, thinking, .. }) = self.items.last_mut() {
            let target = if is_thinking { thinking } else { text };
            target.push_str(chunk);
        }
        self.state = if is_thinking {
            State::Thinking
        } else {
            State::Responding
        };
    }

    fn tool_call(&mut self, id: &str, name: &str, arguments: &str) {
        match self.tool_mut(id) {
            Some(Item::Tool {
                name: n,
                arguments: a,
                summary,
                ..
            }) => {
                *n = name.to_string();
                *a = arguments.to_string();
                *summary = tool_summary(name, arguments);
            }
            _ => self.items.push(new_tool(id, name, arguments.to_string())),
        }
        let running = self.running_tools();
        self.state = State::RunningTools { running };
    }

    /// Tools without output since the last user message.
    fn running_tools(&self) -> usize {
        self.items
            .iter()
            .rev()
            .take_while(|item| !matches!(item, Item::User { .. }))
            .filter(|item| matches!(item, Item::Tool { output: None, .. }))
            .count()
    }

    pub(crate) fn tool_mut(&mut self, id: &str) -> Option<&mut Item> {
        self.items
            .iter_mut()
            .rev()
            .find(|item| matches!(item, Item::Tool { id: x, .. } if x == id))
    }

    pub(crate) fn fill_result(&mut self, id: &str, content: &str, error: bool, ms: Option<u64>) {
        if let Some(Item::Tool {
            output,
            is_error,
            duration_ms,
            ..
        }) = self.tool_mut(id)
        {
            *output = Some(content.to_string());
            *is_error = error;
            *duration_ms = ms;
        }
    }

    pub(crate) fn note(&mut self, kind: NoteKind, text: String) {
        self.items.push(Item::Note { kind, text });
    }

    pub(crate) fn footer(
        &mut self,
        turn: u32,
        step: u32,
        usage: Usage,
        timing: Timing,
        stop: StopReason,
    ) {
        self.totals.add(&usage);
        self.items.push(Item::StepFooter {
            turn,
            step,
            model: self.model.clone(),
            account: self.account.clone(),
            usage,
            timing,
            stop,
        });
    }
}

pub(crate) fn new_tool(id: &str, name: &str, arguments: String) -> Item {
    Item::Tool {
        id: id.to_string(),
        name: name.to_string(),
        summary: if arguments.is_empty() {
            String::new()
        } else {
            tool_summary(name, &arguments)
        },
        arguments,
        output: None,
        is_error: false,
        duration_ms: None,
    }
}
