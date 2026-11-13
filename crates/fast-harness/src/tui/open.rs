//! Opening a session's log on a worker thread (D60, gate G10): the parse of
//! the log and the transcript built from it never run on the terminal's
//! thread. One open runs at a time; an open asked meanwhile is remembered and
//! the newest starts when the running one ends, so an older result is dropped.
//! While a session loads the transcript says so and no prompt starts; an open
//! that fails shows its error and keeps the shown session.

use fast_harness_core::service::Host;
use fast_harness_core::view::opened::{self, Next, Opened};
use fast_harness_core::view::Transcript;

use super::app::{Msg, Tui};

/// The word the transcript shows while a session opens.
pub const LOADING: &str = "Loading the session…";

impl Tui {
    /// Opens session `id` (`None`: a new one) off the UI thread (D60).
    pub(super) fn open_session(&mut self, id: Option<&str>) {
        if self.ui.running {
            return;
        }
        match id {
            Some(id) => {
                self.ui.loading = true;
                if let Some(ask) = self.opens.ask(Some(id)) {
                    self.start_open(ask.generation, ask.id.unwrap_or_default());
                }
            }
            None => self.new_session(),
        }
    }

    /// A new session in `cwd`: nothing to parse, and an open still running
    /// for another session is dropped when it comes.
    fn new_session(&mut self) {
        self.opens.cancel();
        self.ui.loading = false;
        let read = self.rt().open_with_events(&self.cwd, None, false);
        let opened = match read {
            Ok((conv, events)) => Opened {
                view: Transcript::opened(&events, &conv.messages, conv.take_notes()),
                conv,
            },
            Err(err) => return self.ui.status = err,
        };
        self.show_session(opened);
    }

    /// Starts the worker of `generation`, which opens `id` in `cwd`.
    fn start_open(&mut self, generation: u64, id: String) {
        let (home, cwd) = (self.host.home(), self.cwd.clone());
        let job = move || {
            let result = opened::open(&home, &cwd, Some(&id)).map(Box::new);
            Msg::Opened(generation, result)
        };
        if !super::loads::spawn("fast-harness-open", self.tx.clone(), job) {
            self.opens.failed();
            self.ui.loading = false;
            self.ui.status = "cannot start the session read".into();
        }
    }

    /// The worker of `generation` ended (UI thread): its result applies when
    /// it is the newest, else the newest open starts instead.
    pub(super) fn opened(&mut self, generation: u64, result: Result<Box<Opened>, String>) {
        match self.opens.arrive(generation) {
            Next::Apply => {
                self.ui.loading = false;
                match result {
                    Ok(opened) => self.show_session(*opened),
                    Err(err) => self.ui.status = err,
                }
            }
            Next::Drop => {}
            Next::Start(ask) => self.start_open(ask.generation, ask.id.unwrap_or_default()),
        }
    }

    /// A session opened: it replaces the shown one, with its session in the
    /// service. A session the service refuses leaves the shown one in place.
    fn show_session(&mut self, opened: Opened) {
        let Opened { conv, view } = opened;
        let (session, unfinished) = (conv.id.clone(), conv.unfinished);
        let log_path = conv.log_path.clone();
        match self.service.open(conv, &self.cwd) {
            Ok(current) => {
                current.set_answering(true);
                self.ui.approvals = current.approvals();
                self.service.close(&self.session);
                (self.current, self.session) = (current, session);
                (self.transcript, self.log_path) = (view, log_path);
                self.shown(unfinished);
                self.ui.status = format!("session {}", self.session);
            }
            Err(err) => self.ui.status = format!("{err:?}"),
        }
    }
}
