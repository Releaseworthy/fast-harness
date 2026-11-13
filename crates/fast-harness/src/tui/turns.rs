//! The TUI's turns run in `service::Service` (PLAN.md D59): a turn starts
//! with `Service::start`, its events and its end come back over the UI
//! channel, steers, pause and Stop go to the session, and the session keeps
//! the queue the view shows. Only drawing and keys stay in the TUI.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::app::RuntimeHost;
use fast_harness_core::config::Config;
use fast_harness_core::events::TurnOutcome;
use fast_harness_core::service::{Done, Expanded, Finished, Host, Sink, Start};
use fast_harness_core::view::{NoteKind, State};

use super::app::{Msg, Tui};

/// The runtime as the host of the TUI's turns: a build error names the
/// accounts screen, and the build's notes reach the view before the turn's
/// events (as `Msg::Notes`).
pub struct TuiHost {
    runtime: Arc<RuntimeHost>,
    tx: Sender<Msg>,
}

impl TuiHost {
    pub fn new(runtime: Arc<RuntimeHost>, tx: Sender<Msg>) -> TuiHost {
        TuiHost { runtime, tx }
    }
}

impl Host for TuiHost {
    fn home(&self) -> PathBuf {
        self.runtime.home()
    }

    fn config(&self, cwd: &Path) -> Result<Config, String> {
        self.runtime.config(cwd)
    }

    fn agent(
        &self,
        cwd: &Path,
        conv: &Conversation,
        config: &Config,
    ) -> Result<(Agent, Vec<String>), String> {
        let built = self.runtime.agent(cwd, conv, config);
        let (agent, notes) = built.map_err(|err| format!("{err} (F2 opens the accounts)"))?;
        if !notes.is_empty() {
            let _ = self.tx.send(Msg::Notes(notes));
        }
        Ok((agent, Vec::new()))
    }

    fn expand(&self, text: &str) -> Result<Expanded, String> {
        self.runtime.expand(text)
    }

    fn catalog(&self, cwd: &Path) -> fast_harness_core::llm::Catalog {
        self.runtime.catalog(cwd)
    }
}

impl Tui {
    /// Starts a turn of the shown session with the settings of now.
    pub(super) fn start(&mut self, start: Start) {
        let config = self.rt().config.clone();
        self.current.set_config(config);
        let events = self.tx.clone();
        let sink: Sink = Box::new(move |event| {
            let _ = events.send(Msg::Agent(event));
        });
        let ended = self.tx.clone();
        let done: Done = Box::new(move |end: &Finished| {
            let _ = ended.send(Msg::TurnDone(Box::new(end.clone())));
        });
        match self.service.start(&self.current, start, sink, done) {
            Ok(_) => self.ui.running = true,
            Err(err) => self.note(NoteKind::Error, format!("cannot start the turn: {err:?}")),
        }
    }

    /// Enter while a turn runs: a steer, or a queued text when the turn
    /// ended meanwhile.
    pub(super) fn steer(&mut self, text: String) {
        if !self.current.steer(text) && !self.ui.running {
            self.next_queued();
        }
        self.show_queue();
    }

    pub(super) fn pause(&mut self) {
        if self.current.pause() {
            self.ui.status = "pausing after this step's tools...".into();
        }
    }

    /// `/resume`: continues the last turn when its history allows it.
    pub(super) fn resume_turn(&mut self) {
        // The open that arrives would close this session, cancelling the turn.
        if self.ui.loading {
            self.ui.status = "a session is loading".into();
            return;
        }
        if self.current.running() {
            self.ui.status = "a turn is running".into();
            return;
        }
        if !self.current.can_resume() {
            self.ui.status = "nothing to resume: the last turn ended".into();
            return;
        }
        self.ui.status = "resuming the last turn".into();
        self.start(Start::Resume);
    }

    /// The turn ended: the view shows the session's queue again, and a turn
    /// that ended `Done` starts the next queued text.
    pub(super) fn turn_done(&mut self, end: Finished) {
        self.ui.running = false;
        // A turn that panicked sent no `TurnEnd`.
        self.transcript.state = State::Idle;
        self.transcript.revision += 1;
        self.refresh_accounts();
        if end.outcome == TurnOutcome::Cancelled {
            self.ui.status = "turn cancelled: /resume continues it".into();
        } else if self.ui.status.starts_with("cancelling") {
            self.ui.status.clear();
        }
        self.show_queue();
        match end.outcome {
            TurnOutcome::Done => self.next_queued(),
            TurnOutcome::Paused => self.ui.status = "turn paused: /resume continues it".into(),
            _ => {}
        }
    }

    fn next_queued(&mut self) {
        if let Some(text) = self.current.take_next() {
            self.show_queue();
            self.send(text);
        }
    }

    /// Copies the session's steers and queue into the view.
    pub(super) fn show_queue(&mut self) {
        let status = self.current.status();
        (self.ui.steering, self.ui.queued) = (status.steering, status.queued);
    }

    /// A key changed the view's queue (`before` was its length): Alt+Enter
    /// added a text, Ctrl+Backspace removed the last one.
    pub(super) fn queue_keyed(&mut self, before: usize) {
        let now = self.ui.queued.len();
        if now > before {
            if let Some(text) = self.ui.queued.last() {
                self.current.queue(text.clone());
            }
        } else if now < before {
            let last = self.current.status().queued.len();
            self.current.remove_queued(last.saturating_sub(1));
        } else {
            return;
        }
        self.show_queue();
    }
}
