//! What a front end sends to a running turn (PLAN.md D39): steer it, queue
//! a follow-up turn, or pause it after the current step's tools.

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, Sender};

use crate::agent::turn::{traced, Res, Turn};
use crate::events::AgentEvent;
use crate::types::{Block, Message, Role};

/// `kind` of a traced `user/message` that carries a steer.
pub const STEER: &str = "steer";
/// `kind` of a traced `user/message` that carries a context update on resume.
pub const CONTEXT: &str = "context";

/// One input to a running turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    /// Read at the next step boundary: a text block after that step's tool
    /// results (or the next step's user text when the model ended the turn).
    Steer(String),
    /// Starts a new turn when the current one ends `Done`, in queue order.
    FollowUp(String),
    /// The current step finishes its tools; the model is not called again
    /// and the turn ends `Paused`.
    Pause,
}

/// The receiving end the agent loop reads without blocking; the sender is
/// the front end's. An inbox without a channel never has input.
#[derive(Default)]
pub struct Inbox {
    rx: Option<Receiver<Input>>,
    steers: Vec<String>,
    follow_ups: VecDeque<String>,
    paused: bool,
}

impl Inbox {
    /// A connected pair: the front end keeps the sender.
    pub fn channel() -> (Sender<Input>, Inbox) {
        let (tx, rx) = mpsc::channel();
        let inbox = Inbox {
            rx: Some(rx),
            ..Inbox::default()
        };
        (tx, inbox)
    }

    /// An inbox that never has input (callers without steering).
    pub fn empty() -> Inbox {
        Inbox::default()
    }

    /// Moves every input that arrived into the queues, without waiting.
    fn poll(&mut self) {
        let Some(rx) = &self.rx else {
            return;
        };
        while let Ok(input) = rx.try_recv() {
            match input {
                Input::Steer(text) => self.steers.push(text),
                Input::FollowUp(text) => self.follow_ups.push_back(text),
                Input::Pause => self.paused = true,
            }
        }
    }

    /// The steers that arrived, joined by blank lines, if any.
    pub(crate) fn take_steers(&mut self) -> Option<String> {
        self.poll();
        if self.steers.is_empty() {
            return None;
        }
        Some(std::mem::take(&mut self.steers).join("\n\n"))
    }

    /// True once when a pause arrived.
    pub(crate) fn take_pause(&mut self) -> bool {
        self.poll();
        std::mem::take(&mut self.paused)
    }

    /// The text of the next turn after one that ended `Done`: steers that
    /// came too late for it, else the oldest follow-up. `None` after a pause.
    pub(crate) fn next_turn(&mut self) -> Option<String> {
        if self.take_pause() {
            return None;
        }
        self.take_steers().or_else(|| self.follow_ups.pop_front())
    }

    /// Texts no turn used (late steers first, then follow-ups in order); the
    /// front end shows them as queued again.
    pub fn leftover(&mut self) -> Vec<String> {
        self.poll();
        self.paused = false;
        let mut out = std::mem::take(&mut self.steers);
        out.extend(self.follow_ups.drain(..));
        out
    }
}

impl Turn<'_> {
    /// Adds the steers that arrived to the history: a text block after the
    /// last tool results or user text, else a new user message (Claude
    /// accepts text after the `tool_result` blocks of one user message).
    /// Background subagents' answers (PLAN.md D40) join as steers, after
    /// the front end's; their events reach the sink here too.
    pub(super) fn apply_steers(&mut self) -> Res<bool> {
        self.agent.forward_events(&mut *self.sink);
        let texts: Vec<String> = [self.inbox.take_steers(), self.agent.take_notices()]
            .into_iter()
            .flatten()
            .collect();
        if texts.is_empty() {
            return Ok(false);
        }
        let text = texts.join("\n\n");
        self.add_user_text(&text, STEER)?;
        (self.sink)(AgentEvent::UserInput { text, steer: true });
        Ok(true)
    }

    /// Traces `text` as a `user/message` of `kind` and adds it to the last
    /// message when that holds tool results or user text (`session::replay`
    /// merges it the same way).
    pub(super) fn add_user_text(&mut self, text: &str, kind: &str) -> Res<()> {
        let message = Message::user(text);
        traced(self.conv.trace.user_text(self.turn, &message, kind))?;
        let messages = &mut self.conv.messages;
        match messages.last_mut() {
            Some(last) if last.role != Role::Assistant => {
                last.content.push(Block::Text { text: text.into() });
            }
            _ => messages.push(message),
        }
        Ok(())
    }
}
