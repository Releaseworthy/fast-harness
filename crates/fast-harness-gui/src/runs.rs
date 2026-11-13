//! The view state of each session (PLAN.md D39, D59): several sessions run
//! at once in `service::Service`, which keeps their conversations, turn
//! threads, steers and queues. A `Run` is what the window adds: the
//! session's transcript, where its running turn starts, and whether the
//! window saw that turn start and not yet end. The shown session's `Run`
//! drives the transcript rows; the others are parked while a turn runs or
//! texts wait for them, and their events update their own transcript only.

use std::collections::HashMap;
use std::mem;
use std::path::PathBuf;
use std::sync::Arc;

use fast_harness_core::events::AgentEvent;
use fast_harness_core::service::{Session, Status};
use fast_harness_core::session::Unfinished;
use fast_harness_core::view::Transcript;

/// Longest chip text, in characters.
const CHIP_CHARS: usize = 60;

/// One session's view.
pub struct Run {
    /// The folder the session's tools use.
    pub cwd: PathBuf,
    /// The service's session; `None` before the first message.
    pub session: Option<Arc<Session>>,
    pub view: Transcript,
    /// First item of the running turn: items before it do not change.
    pub turn_from: usize,
    /// A turn started here and its end has not reached the window yet.
    pub busy: bool,
}

impl Run {
    pub fn new(cwd: PathBuf, session: Option<Arc<Session>>, view: Transcript) -> Run {
        Run {
            cwd,
            session,
            view,
            turn_from: 0,
            busy: false,
        }
    }

    /// The session's steers, queue, stop and pause marks.
    pub fn status(&self) -> Status {
        (self.session.as_ref()).map_or_else(Status::default, |s| s.status())
    }

    /// Kept when not shown: a turn runs, texts or approval requests wait.
    fn live(&self) -> bool {
        let status = self.status();
        self.busy || !status.queued.is_empty() || !status.waiting.is_empty()
    }

    /// True when the Resume action applies (whether the history allows it
    /// is checked on the click: `Session::can_resume` repairs the history).
    pub fn can_resume(&self) -> bool {
        !self.busy && self.session.is_some() && self.status().unfinished.is_some()
    }

    /// The chips above the composer: steers not yet read (index -1), then
    /// the queue (its index), each on one line.
    pub fn chips(&self) -> Vec<(i32, String)> {
        let status = self.status();
        let steers = status.steering.iter().map(|t| (-1, chip_text(t)));
        let queued = (status.queued.iter().enumerate())
            .map(|(i, t)| (i32::try_from(i).unwrap_or(i32::MAX), chip_text(t)));
        steers.chain(queued).collect()
    }

    /// The sidebar word: waiting (an approval request, D64), running, or
    /// how the last turn stopped.
    pub fn mark(&self) -> &'static str {
        let status = self.status();
        if !status.waiting.is_empty() {
            return "waiting";
        }
        if self.busy {
            return "running";
        }
        status.unfinished.map(Unfinished::word).unwrap_or_default()
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

    /// One event of session `id`'s turn, applied to that session's own
    /// transcript only. `Some(true)` when it is the shown session (only
    /// then may the rows, the status and the panels change), `None` when
    /// the window holds no run of `id`.
    pub fn apply(&mut self, id: &str, event: &AgentEvent) -> Option<bool> {
        let shown = self.is_shown(id);
        self.get_mut(id)?.view.apply(event);
        Some(shown)
    }

    /// Shows session `id` (`None`: a new one): a parked run comes back as it
    /// is, else `load` builds it. The run shown before is parked while live
    /// and dropped otherwise: its session id comes back so the caller can
    /// close it. The same id keeps the shown run.
    pub fn show(
        &mut self,
        id: Option<String>,
        load: impl FnOnce() -> Result<Run, String>,
    ) -> Result<Option<String>, String> {
        if id.is_some() && id == self.id {
            return Ok(None);
        }
        let parked = id.as_ref().and_then(|id| self.parked.remove(id));
        let run = match parked {
            Some(run) => run,
            None => load()?,
        };
        let old = mem::replace(&mut self.shown, run);
        let Some(old_id) = mem::replace(&mut self.id, id) else {
            return Ok(None);
        };
        if old.live() {
            self.parked.insert(old_id, old);
            return Ok(None);
        }
        Ok(Some(old_id))
    }

    /// Drops parked run `id` once it has nothing left to do; true when it
    /// was dropped (the caller closes its session).
    pub fn settle(&mut self, id: &str) -> bool {
        let idle = self.parked.get(id).is_some_and(|run| !run.live());
        if idle {
            self.parked.remove(id);
        }
        idle
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
