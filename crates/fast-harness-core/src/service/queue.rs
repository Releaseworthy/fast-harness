//! What a front end does around a session's turn (PLAN.md D39, D59):
//! steers, follow-ups the session keeps so a view can show and remove
//! them, pause, and what a paused, cancelled or cut turn leaves to resume.
//! The service owns this state; a view reads it with [`Session::status`].

use super::{Session, State};
use crate::agent::Input;
use crate::config::Config as Settings;
use crate::events::{ApprovalRequest, TurnOutcome};
use crate::session::Unfinished;

/// One look at a session, as a view shows it (state words, chips).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub running: bool,
    /// Stop was asked and the turn has not ended yet.
    pub stopping: bool,
    /// Pause was asked and the turn has not ended yet.
    pub pausing: bool,
    /// Steers sent to the running turn and not yet read by it.
    pub steering: Vec<String>,
    /// Follow-ups that start when a turn ends `Done`, in order.
    pub queued: Vec<String>,
    /// How the last turn stopped when it can be resumed.
    pub unfinished: Option<Unfinished>,
    /// Approval requests waiting for an answer (PLAN.md D64), of the turn
    /// or of a background subagent: the view marks the session waiting.
    pub waiting: Vec<ApprovalRequest>,
}

impl Session {
    pub fn status(&self) -> Status {
        let state = self.lock();
        Status {
            running: state.conv.is_none(),
            stopping: state.stopping,
            pausing: state.pausing,
            steering: state.steering.clone(),
            queued: state.follow_ups.clone(),
            unfinished: state.unfinished,
            waiting: self.gate.pending(),
        }
    }

    /// Enter while a turn runs: the text goes to the turn's inbox; when no
    /// turn runs (it ended meanwhile) it joins the queue. True when it was
    /// sent as a steer.
    pub fn steer(&self, text: String) -> bool {
        let mut state = self.lock();
        let sent =
            (state.inbox.as_ref()).is_some_and(|tx| tx.send(Input::Steer(text.clone())).is_ok());
        if sent {
            state.steering.push(text);
        } else {
            state.follow_ups.push(text);
        }
        sent
    }

    /// A follow-up for after the turn (Alt+Enter, a chip).
    pub fn queue(&self, text: String) {
        self.lock().follow_ups.push(text);
    }

    /// Removes queued text `index` (a chip's x, Ctrl+Backspace).
    pub fn remove_queued(&self, index: usize) -> Option<String> {
        let mut state = self.lock();
        (index < state.follow_ups.len()).then(|| state.follow_ups.remove(index))
    }

    /// Drops every queued text and returns them.
    pub fn drop_queue(&self) -> Vec<String> {
        std::mem::take(&mut self.lock().follow_ups)
    }

    /// The next follow-up of an idle session, taken off the queue; the
    /// front end starts it (after a turn that ended `Done`).
    pub fn take_next(&self) -> Option<String> {
        let mut state = self.lock();
        let idle = state.conv.is_some() && !state.follow_ups.is_empty();
        idle.then(|| state.follow_ups.remove(0))
    }

    /// Pause: the turn ends after this step's tools. False when none runs.
    pub fn pause(&self) -> bool {
        let mut state = self.lock();
        let sent = (state.inbox.as_ref()).is_some_and(|tx| tx.send(Input::Pause).is_ok());
        if sent {
            self.gate.pause();
        }
        state.pausing |= sent;
        sent
    }

    /// True when the session is idle and its last turn can go on
    /// (`Start::Resume`).
    pub fn can_resume(&self) -> bool {
        let mut state = self.lock();
        (state.conv.as_mut()).is_some_and(|conv| conv.can_resume())
    }

    /// Forgets how the last turn stopped (a Resume the history refused).
    pub fn forget_unfinished(&self) {
        self.lock().unfinished = None;
    }

    /// Replaces the settings the next turn starts with (a front end that
    /// changed the model or the effort).
    pub fn set_config(&self, config: Settings) {
        self.lock().config = config;
    }

    /// The turn read the steers in `text` (a `UserInput` event).
    pub(super) fn steer_read(&self, text: &str) {
        let mut state = self.lock();
        state.steering.retain(|sent| !text.contains(sent.as_str()));
    }
}

/// A turn starts: no stop or pause asked yet, nothing to resume.
pub(super) fn begun(state: &mut State) {
    (state.stopping, state.pausing) = (false, false);
    state.unfinished = None;
}

/// A turn ended: texts it did not use go first in the queue, and what can
/// be resumed follows its outcome.
pub(super) fn ended(state: &mut State, outcome: TurnOutcome, leftover: &[String]) {
    (state.stopping, state.pausing) = (false, false);
    state.steering.clear();
    let rest = std::mem::replace(&mut state.follow_ups, leftover.to_vec());
    state.follow_ups.extend(rest);
    state.unfinished = match outcome {
        TurnOutcome::Done => None,
        TurnOutcome::Paused => Some(Unfinished::Paused),
        TurnOutcome::Cancelled => Some(Unfinished::Cancelled),
        _ => (state.conv.as_mut())
            .is_some_and(|conv| conv.can_resume())
            .then_some(Unfinished::Interrupted),
    };
}
