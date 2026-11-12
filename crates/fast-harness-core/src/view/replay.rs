//! Building a [`Transcript`] from a replayed conversation or a session log.

use super::{
    compaction_text, logged_outcome_text, new_tool, retry_text, switch_text, Item, NoteKind,
    Transcript,
};
use crate::llm::Timing;
use crate::trace::record::{AssistantMessage, Compaction, TurnEnd, UserMessage};
use crate::trace::{Record, TraceEvent};
use crate::types::{Block, Message, Role, StopReason, Usage};

impl Transcript {
    /// User text, assistant text and reasoning, and tool calls joined with
    /// their results by call id. Turns count user messages with text; steps
    /// count assistant messages within a turn.
    pub fn from_messages(messages: &[Message]) -> Transcript {
        let mut view = Transcript::new();
        for message in messages {
            match message.role {
                Role::Assistant => {
                    view.step += 1;
                    view.add_assistant(view.turn, view.step, message);
                }
                Role::User | Role::Tool => {
                    if view.add_user(message) {
                        view.turn += 1;
                        view.step = 0;
                    }
                    view.add_results(message, None);
                }
            }
        }
        view.revision = view.items.len() as u64;
        view
    }

    /// An opened session (`Runtime::open_with_events`): its events when the
    /// log had any, else its messages, then `notes` (a skipped damaged line,
    /// `Conversation::take_notes`, D60) as error notes, so they show at the
    /// open and once.
    pub fn opened(events: &[TraceEvent], messages: &[Message], notes: Vec<String>) -> Transcript {
        let mut view = match events.is_empty() {
            true => Transcript::from_messages(messages),
            false => Transcript::from_trace(events),
        };
        for text in notes {
            view.items.push(Item::Note {
                kind: NoteKind::Error,
                text,
            });
        }
        view.revision += 1;
        view
    }

    /// A saved session log: the same as [`Transcript::from_messages`] plus
    /// step footers, retry and account switch notes, and tool durations.
    pub fn from_trace(events: &[TraceEvent]) -> Transcript {
        let mut replay = Replay::default();
        for event in events {
            replay.event(event);
        }
        replay.finish();
        let mut view = replay.view;
        view.revision = view.items.len() as u64;
        view
    }

    /// Adds a user item when the message has text; true if it did.
    fn add_user(&mut self, message: &Message) -> bool {
        let text = message.text();
        if text.is_empty() {
            return false;
        }
        self.items.push(Item::User { text });
        true
    }

    fn add_assistant(&mut self, turn: u32, step: u32, message: &Message) {
        let mut thinking = String::new();
        for block in &message.content {
            if let Block::Thinking { text, .. } = block {
                thinking.push_str(text);
            }
        }
        let text = message.text();
        if !text.is_empty() || !thinking.is_empty() {
            self.items.push(Item::Assistant {
                turn,
                step,
                text,
                thinking,
            });
        }
        for (id, name, arguments) in message.tool_calls() {
            self.items.push(new_tool(id, name, arguments.to_string()));
        }
    }

    fn add_results(&mut self, message: &Message, duration_ms: Option<u64>) {
        for block in &message.content {
            if let Block::ToolResult {
                call_id,
                content,
                is_error,
            } = block
            {
                self.fill_result(call_id, content, *is_error, duration_ms);
            }
        }
    }
}

/// Replay state: the footer of the current step waits for `step/end`; a
/// failed attempt's error waits to see whether a retry or switch follows
/// (then the live view showed only that note); a compaction at the start of
/// a turn waits for the turn's user message (the live view shows the user's
/// text when it is sent).
#[derive(Default)]
struct Replay {
    view: Transcript,
    footer: Option<(u32, u32, Usage, Timing, StopReason, String)>,
    attempt_error: Option<String>,
    awaiting_user: bool,
    held_note: Option<String>,
}

impl Replay {
    fn event(&mut self, event: &TraceEvent) {
        match event.record() {
            Record::AssistantAttempt(failure) => self.attempt_error = Some(failure.error),
            Record::LlmRetry(retry) => {
                self.attempt_error = None;
                let note = retry_text(retry.attempt, retry.delay_ms, &retry.error);
                self.view.note(NoteKind::Retry, note);
            }
            Record::AccountSwitch(switch) => {
                self.attempt_error = None;
                let note = switch_text(&switch.from, &switch.to, &switch.reason);
                self.view.note(NoteKind::AccountSwitch, note);
                self.view.account = switch.to;
            }
            _ => {
                if let Some(error) = self.attempt_error.take() {
                    self.view.note(NoteKind::Error, error);
                }
                self.event_after_attempt(event);
            }
        }
    }

    fn event_after_attempt(&mut self, event: &TraceEvent) {
        match event.record() {
            Record::TurnStart(start) => {
                self.flush();
                self.view.turn_usage = Usage::default();
                self.awaiting_user = !start.resume;
            }
            Record::UserMessage(user) => self.user_message(&user),
            Record::StepStart(start) => {
                self.flush();
                self.view.turn = start.turn;
                self.view.step = start.step;
            }
            Record::RequestContext(request) => {
                self.view.model = request.model;
                self.view.account = request.account;
            }
            Record::AssistantMessage(answer) => self.assistant(&answer),
            Record::ToolCall(call) => {
                if self.view.tool_mut(&call.call_id).is_none() {
                    let tool = new_tool(&call.call_id, &call.name, call.arguments);
                    self.view.items.push(tool);
                }
            }
            Record::ToolResult(result) => {
                self.view
                    .add_results(&result.message, Some(result.duration_ms));
            }
            Record::Compaction(compaction) => self.compaction(&compaction),
            Record::StepEnd(_) => self.flush(),
            Record::TurnEnd(end) => self.turn_end(&end),
            _ => {}
        }
    }

    fn user_message(&mut self, user: &UserMessage) {
        self.flush();
        self.view.turn = user.turn;
        self.view.add_user(&user.message);
        self.view.add_results(&user.message, None);
        self.awaiting_user = false;
        if let Some(note) = self.held_note.take() {
            self.view.note(NoteKind::Info, note);
        }
    }

    fn compaction(&mut self, compaction: &Compaction) {
        self.flush();
        let auto = compaction.auto.unwrap_or(true);
        let before = compaction.before.unwrap_or(0);
        let after = compaction.after.unwrap_or(0);
        let text = compaction_text(auto, before, after);
        match self.awaiting_user {
            true => self.held_note = Some(text),
            false => self.view.note(NoteKind::Info, text),
        }
        self.view.context = after;
    }

    fn assistant(&mut self, answer: &AssistantMessage) {
        self.flush();
        let (turn, step) = (answer.turn, answer.step);
        self.view.add_assistant(turn, step, &answer.message);
        self.view.turn_usage.add(&answer.usage);
        // Trace version 4 keys; older logs leave the window unknown.
        self.view.context = answer.context.unwrap_or(answer.usage.context());
        self.view.window = answer.window.unwrap_or(0);
        let stop = answer.stop_reason();
        let account = match &answer.account {
            Some(account) => account.clone(),
            None => self.view.account.clone(),
        };
        self.footer = Some((turn, step, answer.usage, answer.timing, stop, account));
    }

    fn turn_end(&mut self, end: &TurnEnd) {
        self.flush();
        // A `/compact` turn has no user message.
        if let Some(note) = self.held_note.take() {
            self.view.note(NoteKind::Info, note);
        }
        self.awaiting_user = false;
        if let Some(text) = logged_outcome_text(&end.outcome) {
            self.view.note(NoteKind::Info, text);
        }
    }

    /// The end of the log: a failed attempt nothing followed, the footer.
    fn finish(&mut self) {
        if let Some(error) = self.attempt_error.take() {
            self.view.note(NoteKind::Error, error);
        }
        self.flush();
    }

    /// Writes the pending footer, if any.
    fn flush(&mut self) {
        if let Some((turn, step, usage, timing, stop, account)) = self.footer.take() {
            let live_account = std::mem::replace(&mut self.view.account, account);
            self.view.footer(turn, step, usage, timing, stop);
            self.view.account = live_account;
        }
    }
}
