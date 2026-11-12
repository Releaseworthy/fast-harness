//! Steering, queue, pause and resume in the TUI (PLAN.md D39): Enter while
//! a turn runs steers it, Alt+Enter queues a follow-up, Ctrl+G pauses after
//! the step's tools, Ctrl+Backspace drops the last queued text and
//! `/resume` continues a paused, cancelled or interrupted turn.
//!
//! The session in the service owns the steers and the follow-ups (PLAN.md
//! D59, `tui/turns.rs`); `Ui::queued` and `Ui::steering` are the view's
//! copy, and a key that edits the copy is applied to the session at once.

use super::ui::{Action, Ui};

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
