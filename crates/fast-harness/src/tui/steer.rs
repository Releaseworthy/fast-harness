//! Steering, queue, pause and resume in the TUI (PLAN.md D39): Enter while
//! a turn runs steers it, Alt+Enter queues a follow-up, Ctrl+G pauses after
//! the step's tools, Ctrl+Backspace drops the last queued text and
//! `/resume` continues a paused, cancelled or interrupted turn.
//!
//! Follow-ups stay in the TUI (so they can be removed) and start when a
//! turn ends `Done`; steers go to the running turn's inbox at once.

use fast_harness_core::agent::Input;
use fast_harness_core::events::TurnOutcome;

use super::app::Tui;
use super::ui::{Action, Ui};
use super::worker::Job;

/// The text that resumes the last turn instead of sending a prompt.
pub const RESUME_COMMAND: &str = "/resume";

/// Most queue rows shown above the input.
pub const QUEUE_ROWS: usize = 4;

impl Ui {
    /// Alt+Enter while a turn runs: the input becomes a queued follow-up.
    pub fn queue(&mut self) -> Action {
        if self.input.text().trim().is_empty() {
            return Action::None;
        }
        let text = self.input.submit();
        self.queued.push(text);
        Action::None
    }

    /// Ctrl+Backspace: removes the last queued text.
    pub fn drop_queued(&mut self) {
        if let Some(text) = self.queued.pop() {
            let short: String = text.chars().take(40).collect();
            self.status = format!("removed from the queue: {short}");
        }
    }

    /// The rows above the input: steers not yet read, then the queue.
    pub fn queue_rows(&self) -> Vec<String> {
        let steers = self
            .steering
            .iter()
            .map(|t| format!("steer (next step): {t}"));
        let queued =
            (self.queued.iter().enumerate()).map(|(i, t)| format!("queued {}: {t}", i + 1));
        let mut rows: Vec<String> = steers.chain(queued).map(|row| first_line(&row)).collect();
        if rows.len() > QUEUE_ROWS {
            let more = rows.len() - QUEUE_ROWS + 1;
            rows.truncate(QUEUE_ROWS - 1);
            rows.push(format!("... {more} more"));
        }
        rows
    }
}

fn first_line(text: &str) -> String {
    let mut lines = text.lines();
    let first = lines.next().unwrap_or_default().to_string();
    match lines.next() {
        Some(_) => format!("{first} ..."),
        None => first,
    }
}

impl Tui {
    /// Enter while a turn runs: the text goes to its inbox.
    pub(super) fn steer(&mut self, text: String) {
        let sent = match &self.inbox {
            Some(inbox) => inbox.send(Input::Steer(text.clone())).is_ok(),
            None => false,
        };
        if sent {
            self.ui.steering.push(text);
        } else {
            // The turn ended meanwhile: the text waits for the next one.
            self.ui.queued.push(text);
            if !self.ui.running {
                self.next_queued();
            }
        }
    }

    pub(super) fn pause(&mut self) {
        if let Some(inbox) = &self.inbox {
            if inbox.send(Input::Pause).is_ok() {
                self.ui.status = "pausing after this step's tools...".into();
            }
        }
    }

    /// `/resume`: continues the last turn when its history allows it.
    pub(super) fn resume_turn(&mut self) {
        let Some(conv) = self.conv_mut() else {
            self.ui.status = "a turn is running".into();
            return;
        };
        if !conv.can_resume() {
            self.ui.status = "nothing to resume: the last turn ended".into();
            return;
        }
        self.ui.status = "resuming the last turn".into();
        self.start(Job::Resume);
    }

    /// After a turn: texts the turn did not use go first in the queue, and
    /// a turn that ended `Done` starts the next queued one.
    pub(super) fn after_turn(&mut self, outcome: TurnOutcome, leftover: Vec<String>) {
        self.ui.steering.clear();
        if !leftover.is_empty() {
            let rest = std::mem::replace(&mut self.ui.queued, leftover);
            self.ui.queued.extend(rest);
        }
        match outcome {
            TurnOutcome::Done => self.next_queued(),
            TurnOutcome::Paused => {
                self.ui.status = "turn paused: /resume continues it".into();
            }
            _ => {}
        }
    }

    fn next_queued(&mut self) {
        if !self.ui.queued.is_empty() {
            let text = self.ui.queued.remove(0);
            self.send(text);
        }
    }
}
