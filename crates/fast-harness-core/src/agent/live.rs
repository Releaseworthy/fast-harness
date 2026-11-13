//! The subagent sessions running in this process (PLAN.md D79): a front end
//! shows one as a full session, so its composer steers the live child, its
//! Stop cancels only that child and its pause reaches the child's inbox,
//! while the parent's turn owns the loop. A watcher gets every event of the
//! child as it happens. A child session runs at most one turn at a time:
//! [`enter`] refuses a second (a front end continuing the session while its
//! parent's turn still runs it).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use super::Input;
use crate::events::AgentEvent;

/// What a front end reaches of a child its parent runs.
struct Controls {
    cancel: Arc<AtomicBool>,
    steer: Sender<Input>,
    watchers: Vec<Sender<AgentEvent>>,
}

#[derive(Default)]
struct Entry {
    controls: Option<Controls>,
    /// A turn of the session runs now.
    entered: bool,
}

fn table() -> MutexGuard<'static, HashMap<String, Entry>> {
    static ALL: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    let all = ALL.get_or_init(Mutex::default);
    all.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The controls of child session `id` while its parent runs it; dropping
/// it ends every watch.
pub(crate) struct Run(String);

impl Drop for Run {
    fn drop(&mut self) {
        let mut all = table();
        if let Some(entry) = all.get_mut(&self.0) {
            entry.controls = None;
            if !entry.entered {
                all.remove(&self.0);
            }
        }
    }
}

/// Registers the controls of child session `id`, run by its parent.
pub(crate) fn run(id: &str, cancel: Arc<AtomicBool>, steer: Sender<Input>) -> Run {
    let controls = Controls {
        cancel,
        steer,
        watchers: Vec::new(),
    };
    table().entry(id.to_string()).or_default().controls = Some(controls);
    Run(id.to_string())
}

/// One turn of a subagent session; dropping it lets the next one in.
pub(crate) struct Entered(String);

impl Drop for Entered {
    fn drop(&mut self) {
        let mut all = table();
        if let Some(entry) = all.get_mut(&self.0) {
            entry.entered = false;
            if entry.controls.is_none() {
                all.remove(&self.0);
            }
        }
    }
}

/// Starts a turn of subagent session `id`; the note when one runs already.
pub(crate) fn enter(id: &str) -> Result<Entered, String> {
    let mut all = table();
    let entry = all.entry(id.to_string()).or_default();
    if entry.entered {
        return Err(busy_note(id));
    }
    entry.entered = true;
    Ok(Entered(id.to_string()))
}

/// The note of a turn refused because the session runs one already.
pub fn busy_note(id: &str) -> String {
    format!("session {id} is running a turn already (its parent runs it as a subagent): steer it or wait for its end")
}

/// Whether child session `id` runs now inside its parent's turn.
pub fn running(id: &str) -> bool {
    table()
        .get(id)
        .is_some_and(|entry| entry.controls.is_some())
}

fn with<T>(id: &str, f: impl FnOnce(&mut Controls) -> T) -> Option<T> {
    table().get_mut(id).and_then(|e| e.controls.as_mut()).map(f)
}

/// Sends `text` to the running child `id` as a steer; false when it does not run.
pub fn steer(id: &str, text: &str) -> bool {
    with(id, |c| c.steer.send(Input::Steer(text.to_string())).is_ok()).unwrap_or(false)
}

/// Pauses the running child `id` at its next step boundary.
pub fn pause(id: &str) -> bool {
    with(id, |c| c.steer.send(Input::Pause).is_ok()).unwrap_or(false)
}

/// Stops the running child `id` (only it: its parent goes on).
pub fn stop(id: &str) -> bool {
    with(id, |c| c.cancel.store(true, Ordering::SeqCst)).is_some()
}

/// Every later event of the running child `id`, until it ends (then the
/// channel closes); `None` when it does not run.
pub fn watch(id: &str) -> Option<Receiver<AgentEvent>> {
    let (tx, rx) = mpsc::channel();
    with(id, |c| c.watchers.push(tx)).map(|()| rx)
}

/// Hands `event` of child `id` to its watchers (gone ones are dropped).
pub(crate) fn publish(id: &str, event: &AgentEvent) {
    with(id, |c| {
        if !c.watchers.is_empty() {
            c.watchers.retain(|tx| tx.send(event.clone()).is_ok());
        }
    });
}

#[cfg(test)]
#[path = "live_tests.rs"]
mod tests;
