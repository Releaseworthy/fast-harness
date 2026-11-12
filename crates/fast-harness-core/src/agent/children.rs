//! The subagents of one session (PLAN.md D40): the running count behind
//! `maxParallel`, the background children with their cancel flag, steer
//! channel and answer, the finished notices for the parent's inbox and the
//! events for the parent's front end. One registry per session id lives as
//! long as the process, so a front end that builds an agent per turn still
//! finds the children of an earlier turn.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use crate::agent::Input;
use crate::events::AgentEvent;

/// How often a waiting thread looks at its cancel flag.
pub(super) const TICK: Duration = Duration::from_millis(100);

/// A background child.
struct Kid {
    id: String,
    cancel: Arc<AtomicBool>,
    steer: Sender<Input>,
    /// The answer once it ended.
    output: Option<String>,
}

#[derive(Default)]
struct State {
    /// Children running now, foreground and background.
    running: usize,
    started: u32,
    kids: Vec<Kid>,
    notices: Vec<String>,
    events: Vec<AgentEvent>,
    /// The session's frozen system prompt, set at each turn start: a child
    /// takes it (a resumed session keeps its own, not a fresh render).
    system: Option<String>,
    /// The session's approval gate, set at each turn start (PLAN.md D64):
    /// a child asks through it.
    approver: Option<super::approval::Approver>,
}

#[derive(Default)]
pub struct Children {
    state: Mutex<State>,
    changed: Condvar,
}

/// The registry of session `id`.
pub fn of_session(id: &str) -> Arc<Children> {
    static ALL: OnceLock<Mutex<HashMap<String, Arc<Children>>>> = OnceLock::new();
    let all = ALL.get_or_init(Mutex::default);
    let mut all = all.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    all.entry(id.to_string()).or_default().clone()
}

/// A running child's place in the count; dropping it frees the place.
pub struct Slot(Arc<Children>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.lock().running -= 1;
        self.0.changed.notify_all();
    }
}

/// What the background tools see of one child.
pub(super) struct KidView {
    pub cancel: Arc<AtomicBool>,
    pub steer: Sender<Input>,
    pub output: Option<String>,
}

impl Children {
    /// Remembers the session's approval gate for its children.
    pub(super) fn set_approver(&self, approver: super::approval::Approver) {
        self.lock().approver = Some(approver);
    }

    pub(super) fn approver(&self) -> Option<super::approval::Approver> {
        self.lock().approver.clone()
    }

    /// Remembers the parent session's frozen system prompt.
    pub fn set_system(&self, system: &str) {
        self.lock().system = Some(system.to_string());
    }

    /// The parent session's frozen system prompt, once a turn started.
    pub fn system(&self) -> Option<String> {
        self.lock().system.clone()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A place among `max` running children. A full count is an error when
    /// `wait` is false; else this waits for a place until `cancel` is set.
    pub(super) fn acquire(
        self: &Arc<Self>,
        max: usize,
        wait: bool,
        cancel: &AtomicBool,
    ) -> Result<Slot, String> {
        let mut state = self.lock();
        while state.running >= max {
            if !wait {
                return Err(format!(
                    "{max} subagents are running already (subagent.maxParallel): wait for one \
                    with agent_output or stop one with agent_stop"
                ));
            }
            if cancel.load(Ordering::Relaxed) {
                return Err("cancelled".into());
            }
            state = self.wait(state, TICK);
        }
        state.running += 1;
        Ok(Slot(self.clone()))
    }

    fn wait<'a>(&self, state: MutexGuard<'a, State>, limit: Duration) -> MutexGuard<'a, State> {
        match self.changed.wait_timeout(state, limit) {
            Ok((state, _)) => state,
            Err(poisoned) => poisoned.into_inner().0,
        }
    }

    /// Registers a background child; returns its id (`bg1`, `bg2`, ...).
    pub(super) fn add_kid(&self, cancel: Arc<AtomicBool>, steer: Sender<Input>) -> String {
        let mut state = self.lock();
        state.started += 1;
        let id = format!("bg{}", state.started);
        state.kids.push(Kid {
            id: id.clone(),
            cancel,
            steer,
            output: None,
        });
        id
    }

    /// Stores the answer of background child `id` and queues the notice
    /// `[subagent <id> finished] <answer>`, cut to `budget` bytes.
    pub(super) fn finish_kid(&self, id: &str, output: String, budget: usize) {
        let notice = format!("[subagent {id} finished] {}", cut(&output, budget));
        let mut state = self.lock();
        if let Some(kid) = state.kids.iter_mut().find(|kid| kid.id == id) {
            kid.output = Some(output);
        }
        state.notices.push(notice);
        drop(state);
        self.changed.notify_all();
    }

    pub(super) fn kid(&self, id: &str) -> Option<KidView> {
        let state = self.lock();
        let kid = state.kids.iter().find(|kid| kid.id == id.trim())?;
        Some(KidView {
            cancel: kid.cancel.clone(),
            steer: kid.steer.clone(),
            output: kid.output.clone(),
        })
    }

    /// The answer of child `id`, waiting for it until `cancel` is set when `wait`.
    pub(super) fn output(&self, id: &str, wait: bool, cancel: &AtomicBool) -> Option<String> {
        let mut state = self.lock();
        loop {
            let kid = state.kids.iter().find(|kid| kid.id == id.trim())?;
            if kid.output.is_some() || !wait || cancel.load(Ordering::Relaxed) {
                return kid.output.clone();
            }
            state = self.wait(state, TICK);
        }
    }

    /// Background children that have not ended.
    pub fn running_background(&self) -> usize {
        let state = self.lock();
        state.kids.iter().filter(|kid| kid.output.is_none()).count()
    }

    /// The finished notices, joined by blank lines.
    pub(super) fn take_notices(&self) -> Option<String> {
        let notices = std::mem::take(&mut self.lock().notices);
        (!notices.is_empty()).then(|| notices.join("\n\n"))
    }

    pub(super) fn push_event(&self, event: AgentEvent) {
        self.lock().events.push(event);
        self.changed.notify_all();
    }

    pub(super) fn take_events(&self) -> Vec<AgentEvent> {
        std::mem::take(&mut self.lock().events)
    }

    /// Sets the cancel flag of every background child that runs.
    pub(super) fn cancel_all(&self) {
        let state = self.lock();
        for kid in state.kids.iter().filter(|kid| kid.output.is_none()) {
            kid.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Waits up to `limit` for a change (a child ended, an event arrived).
    pub(super) fn wait_change(&self, limit: Duration) {
        let state = self.lock();
        let _state = self.wait(state, limit);
    }

    /// Waits until no background child runs, at most until `deadline`.
    pub(super) fn wait_idle(&self, deadline: Instant) {
        while self.running_background() > 0 && Instant::now() < deadline {
            self.wait_change(TICK);
        }
    }
}

/// `text` within `budget` bytes, cut at a character boundary with a note.
fn cut(text: &str, budget: usize) -> String {
    if text.len() <= budget {
        return text.to_string();
    }
    let mut end = budget;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n[cut to {budget} bytes: agent_output gives the whole answer]",
        &text[..end]
    )
}
