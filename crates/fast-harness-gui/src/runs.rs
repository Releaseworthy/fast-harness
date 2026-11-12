//! Running turns kept per session id (PLAN.md D39, GUI): several sessions
//! run at once. The shown session's `Run` drives the transcript rows; the
//! others are parked while a turn runs or texts wait for them, and their
//! events update their own transcript only.
//!
//! Follow-ups stay here (so a chip can remove them) and start when a turn
//! ends `Done`; steers go to the running turn's inbox at once.

use std::collections::HashMap;
use std::mem;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use fast_harness_core::agent::{Conversation, Inbox, Input};
use fast_harness_core::events::{AgentEvent, TurnOutcome};
use fast_harness_core::session::Unfinished;
use fast_harness_core::view::Transcript;

/// Longest chip text, in characters.
const CHIP_CHARS: usize = 60;

/// One session's live state.
pub struct Run {
    /// The folder the session's tools use.
    pub cwd: PathBuf,
    /// `None` before the first message and while a turn has it.
    pub conv: Option<Conversation>,
    pub view: Transcript,
    /// First item of the running turn: items before it do not change.
    pub turn_from: usize,
    pub busy: bool,
    pub stopping: bool,
    pub pausing: bool,
    pub cancel: Arc<AtomicBool>,
    /// The running turn's inbox.
    inbox: Option<Sender<Input>>,
    /// Steers sent to the turn and not yet read by it.
    pub steering: Vec<String>,
    /// Follow-ups that start when a turn ends `Done`, in order.
    pub queued: Vec<String>,
    /// How the last turn stopped when it can be resumed.
    pub unfinished: Option<Unfinished>,
}

impl Run {
    pub fn new(cwd: PathBuf, conv: Option<Conversation>, view: Transcript) -> Run {
        let unfinished = conv.as_ref().and_then(|c| c.unfinished);
        Run {
            cwd,
            conv,
            view,
            turn_from: 0,
            busy: false,
            stopping: false,
            pausing: false,
            cancel: Arc::new(AtomicBool::new(false)),
            inbox: None,
            steering: Vec::new(),
            queued: Vec::new(),
            unfinished,
        }
    }

    /// Kept when not shown: a turn runs or texts wait.
    fn live(&self) -> bool {
        self.busy || !self.queued.is_empty()
    }

    /// A turn starts: the worker gets the cancel flag and the inbox.
    pub fn begin(&mut self) -> (Arc<AtomicBool>, Inbox) {
        let (tx, inbox) = Inbox::channel();
        self.busy = true;
        self.stopping = false;
        self.pausing = false;
        self.unfinished = None;
        self.cancel = Arc::new(AtomicBool::new(false));
        self.inbox = Some(tx);
        (self.cancel.clone(), inbox)
    }

    /// Enter while busy: the text goes to the turn's inbox, or to the queue
    /// when the turn ended meanwhile. True when it was sent as a steer.
    pub fn steer(&mut self, text: String) -> bool {
        let sent = self.busy
            && (self.inbox.as_ref()).is_some_and(|tx| tx.send(Input::Steer(text.clone())).is_ok());
        if sent {
            self.steering.push(text);
        } else {
            self.queued.push(text);
        }
        sent
    }

    /// Alt+Enter: a follow-up for after the turn.
    pub fn queue(&mut self, text: String) {
        self.queued.push(text);
    }

    /// The chip's x: removes queued text `index`.
    pub fn remove_queued(&mut self, index: usize) -> Option<String> {
        (index < self.queued.len()).then(|| self.queued.remove(index))
    }

    /// Pause: the turn ends after this step's tools. False when no turn runs.
    pub fn pause(&mut self) -> bool {
        let sent =
            self.busy && (self.inbox.as_ref()).is_some_and(|tx| tx.send(Input::Pause).is_ok());
        self.pausing |= sent;
        sent
    }

    /// Stop: cancels the running turn. False when none runs.
    pub fn stop(&mut self) -> bool {
        if self.busy {
            self.cancel.store(true, Ordering::SeqCst);
            self.stopping = true;
        }
        self.busy
    }

    /// One event of this session's turn.
    pub fn apply(&mut self, event: &AgentEvent) {
        if let AgentEvent::UserInput { text, steer: true } = event {
            self.steering.retain(|sent| !text.contains(sent.as_str()));
        }
        self.view.apply(event);
    }

    /// The turn ended: texts it did not use go first in the queue, and the
    /// next queued text is returned when the turn ended `Done`.
    pub fn end(
        &mut self,
        conv: Option<Conversation>,
        outcome: TurnOutcome,
        leftover: Vec<String>,
    ) -> Option<String> {
        self.busy = false;
        self.stopping = false;
        self.pausing = false;
        self.inbox = None;
        self.steering.clear();
        let rest = mem::replace(&mut self.queued, leftover);
        self.queued.extend(rest);
        self.conv = conv;
        self.unfinished = match outcome {
            TurnOutcome::Done => None,
            TurnOutcome::Paused => Some(Unfinished::Paused),
            TurnOutcome::Cancelled => Some(Unfinished::Cancelled),
            _ => (self.conv.as_mut())
                .is_some_and(|c| c.can_resume())
                .then_some(Unfinished::Interrupted),
        };
        let next = outcome == TurnOutcome::Done && !self.queued.is_empty();
        next.then(|| self.queued.remove(0))
    }

    /// True when the Resume action applies.
    pub fn can_resume(&self) -> bool {
        !self.busy && self.conv.is_some() && self.unfinished.is_some()
    }

    /// The chips above the composer: steers not yet read (index -1), then
    /// the queue (its index), each on one line.
    pub fn chips(&self) -> Vec<(i32, String)> {
        let steers = self.steering.iter().map(|t| (-1, chip_text(t)));
        let queued = (self.queued.iter().enumerate())
            .map(|(i, t)| (i32::try_from(i).unwrap_or(i32::MAX), chip_text(t)));
        steers.chain(queued).collect()
    }

    /// The sidebar word: running, or how the last turn stopped.
    pub fn mark(&self) -> &'static str {
        if self.busy {
            return "running";
        }
        self.unfinished.map(Unfinished::word).unwrap_or_default()
    }
}

/// The first line, cut to `CHIP_CHARS` characters.
fn chip_text(text: &str) -> String {
    let first = text.trim().lines().next().unwrap_or_default();
    let mut out: String = first.chars().take(CHIP_CHARS).collect();
    if out.len() < text.trim().len() {
        out.push_str(" ...");
    }
    out
}

/// The shown run and the parked ones, by session id.
pub struct Runs {
    /// Id of the shown session; `None` before its first message.
    pub id: Option<String>,
    pub shown: Run,
    parked: HashMap<String, Run>,
}

impl Runs {
    pub fn new(cwd: PathBuf) -> Runs {
        Runs {
            id: None,
            shown: Run::new(cwd, None, Transcript::new()),
            parked: HashMap::new(),
        }
    }

    pub fn is_shown(&self, id: &str) -> bool {
        self.id.as_deref() == Some(id)
    }

    /// The run of session `id`, shown or parked.
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Run> {
        if self.is_shown(id) {
            return Some(&mut self.shown);
        }
        self.parked.get_mut(id)
    }

    /// Shows session `id` (`None`: a new one): a parked run comes back as it
    /// is, else `load` builds it. The run shown before is parked while live
    /// and dropped otherwise. The same id keeps the shown run.
    pub fn show(
        &mut self,
        id: Option<String>,
        load: impl FnOnce() -> Result<Run, String>,
    ) -> Result<(), String> {
        if id.is_some() && id == self.id {
            return Ok(());
        }
        let parked = id.as_ref().and_then(|id| self.parked.remove(id));
        let run = match parked {
            Some(run) => run,
            None => load()?,
        };
        let old = mem::replace(&mut self.shown, run);
        if let Some(old_id) = mem::replace(&mut self.id, id) {
            if old.live() {
                self.parked.insert(old_id, old);
            }
        }
        Ok(())
    }

    /// Drops parked run `id` once it has nothing left to do.
    pub fn settle(&mut self, id: &str) {
        if self.parked.get(id).is_some_and(|run| !run.live()) {
            self.parked.remove(id);
        }
    }

    /// The sidebar word of session `id`: its live run's, else `disk`'s.
    pub fn mark(&self, id: &str, disk: Option<Unfinished>) -> &'static str {
        let run = if self.is_shown(id) {
            Some(&self.shown)
        } else {
            self.parked.get(id)
        };
        match run {
            Some(run) => run.mark(),
            None => disk.map(Unfinished::word).unwrap_or_default(),
        }
    }

    /// Ids of the parked runs (tests).
    #[cfg(test)]
    pub fn parked_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.parked.keys().cloned().collect();
        ids.sort();
        ids
    }
}

#[cfg(test)]
#[path = "runs_tests.rs"]
mod tests;
