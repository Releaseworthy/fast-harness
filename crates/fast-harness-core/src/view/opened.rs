//! A session opened on a worker thread (D60, gate G10): the parse of its log
//! and the transcript built from it, so a front end's UI thread never reads a
//! log. [`Opening`] supersedes an older open of one front end: a result that
//! is not the newest is dropped and the newest is started.

use std::path::Path;

use crate::agent::Conversation;
use crate::view::Transcript;

/// A session opened: its conversation and the view of its history.
pub struct Opened {
    pub conv: Conversation,
    pub view: Transcript,
}

/// Opens session `id` of `cwd` under `home` (worker thread): one parse of
/// its log, then the transcript from those events with the damaged-line note
/// (`Transcript::opened`, D60). A log that does not read is the error.
pub fn load(home: &Path, cwd: &Path, id: &str) -> Result<Opened, String> {
    let (conv, events) =
        Conversation::reopen(home, cwd, id).map_err(|err| format!("session {id}: {err}"))?;
    let view = Transcript::opened(&events, &conv.messages, conv.take_notes());
    drop(events);
    Ok(Opened { conv, view })
}

/// [`load`] for `id`, or a new session in `cwd` when it is `None` (worker
/// thread; a new session has no log, so no parse).
pub fn open(home: &Path, cwd: &Path, id: Option<&str>) -> Result<Opened, String> {
    match id {
        Some(id) => load(home, cwd, id),
        None => {
            let conv = Conversation::create(home, cwd, None)
                .map_err(|err| format!("new session: {err}"))?;
            Ok(Opened {
                conv,
                view: Transcript::new(),
            })
        }
    }
}

/// One open asked for: its generation and the session it opens (`None`: a
/// new one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    pub generation: u64,
    pub id: Option<String>,
}

/// What the UI thread does with a worker that ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Next {
    /// The newest open: its result is applied.
    Apply,
    /// A newer open was asked meanwhile: this result is dropped.
    Drop,
    /// The newest open waited for this worker: its worker starts now.
    Start(Ask),
}

/// The opens asked for by one front end, newest wins. One worker runs at a
/// time; an ask made while it runs is remembered and started when it ends.
#[derive(Default)]
pub struct Opening {
    /// The newest generation asked.
    newest: u64,
    /// The generation whose worker runs, while one does.
    running: Option<u64>,
    /// The newest ask, while the view still wants it.
    wanted: Option<Ask>,
}

impl Opening {
    /// Asks an open of session `id` (`None`: a new one): the ask a worker
    /// starts with, or `None` when a worker runs (the newest starts after).
    pub fn ask(&mut self, id: Option<&str>) -> Option<Ask> {
        self.newest += 1;
        let ask = Ask {
            generation: self.newest,
            id: id.map(String::from),
        };
        self.wanted = Some(ask.clone());
        if self.running.is_some() {
            return None;
        }
        self.running = Some(self.newest);
        Some(ask)
    }

    /// A worker of `generation` ended: apply it, drop it, or start the newest
    /// ask instead (`Next::Start` has taken the run slot).
    pub fn arrive(&mut self, generation: u64) -> Next {
        if self.running == Some(generation) {
            self.running = None;
        }
        if generation == self.newest {
            return Next::Apply;
        }
        match self.wanted.clone() {
            // The view left the open, or another worker already holds it.
            None => Next::Drop,
            Some(_) if self.running.is_some() => Next::Drop,
            Some(ask) => {
                self.running = Some(ask.generation);
                Next::Start(ask)
            }
        }
    }

    /// The view shows another session (a new one, another workspace): a
    /// worker still running is dropped when it comes.
    pub fn cancel(&mut self) {
        self.newest += 1;
        self.wanted = None;
    }

    /// The worker could not start: nothing loads, the view keeps its session.
    pub fn failed(&mut self) {
        self.running = None;
        self.wanted = None;
    }

    /// An open is in flight: a turn must not start meanwhile.
    pub fn loading(&self) -> bool {
        self.running.is_some()
    }
}

#[cfg(test)]
#[path = "opened_tests.rs"]
mod tests;
