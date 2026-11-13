//! The Sessions screen's list and the Trace screen's log, read on worker
//! threads (D60, chunk 26B). The screen opens at once with what it has (a
//! muted "Loading" line when that is nothing); the result comes back as a
//! [`Msg`] and is dropped when its screen was closed or a newer read was
//! asked. Keys pressed meanwhile act on what is shown.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::thread;

use fast_harness_core::service::Host;
use fast_harness_core::session::{self, SessionSummary};
use fast_harness_core::trace::{self, detail, report};

use super::app::{Msg, Tui};
use super::ui::Screen;

/// A Trace screen read: timeline entries and step table lines, or the error.
pub type TraceRead = Result<(Vec<detail::Entry>, Vec<String>), String>;

/// The newest read asked per screen.
#[derive(Default)]
pub struct Loads {
    next: u64,
    sessions: Option<u64>,
    trace: Option<u64>,
}

impl Loads {
    fn ticket(&mut self) -> u64 {
        self.next += 1;
        self.next
    }
}

/// Lists `cwd`'s sessions (worker thread).
pub fn list(home: &Path, cwd: &Path) -> Result<Vec<SessionSummary>, String> {
    session::list(home, cwd).map_err(|err| format!("sessions: {err}"))
}

/// Reads the log at `path` for the Trace screen (worker thread).
pub fn read_trace(path: &Path) -> TraceRead {
    match trace::read(path) {
        Ok((_, events)) => {
            let rows = report::step_rows(&events);
            let steps = match rows.is_empty() {
                true => Vec::new(),
                false => (report::format_table(&rows).lines())
                    .map(String::from)
                    .collect(),
            };
            Ok((detail::entries(&events), steps))
        }
        Err(err) => Err(format!("cannot read {}: {err}", path.display())),
    }
}

pub(super) fn spawn(
    name: &str,
    tx: Sender<Msg>,
    job: impl FnOnce() -> Msg + Send + 'static,
) -> bool {
    let spawned = thread::Builder::new().name(name.into()).spawn(move || {
        let _ = tx.send(job());
    });
    spawned.is_ok()
}

impl Tui {
    /// Opens the Sessions screen's read; the list arrives in `Msg::Sessions`.
    pub(super) fn load_sessions(&mut self) {
        let ticket = self.loads.ticket();
        self.loads.sessions = Some(ticket);
        self.ui.sessions_loading = true;
        let (home, cwd): (PathBuf, PathBuf) = (self.host.home(), self.cwd.clone());
        let job = move || Msg::Sessions(ticket, list(&home, &cwd));
        if !spawn("fast-harness-sessions", self.tx.clone(), job) {
            self.ui.sessions_loading = false;
            self.ui.status = "sessions: cannot start the list thread".into();
        }
    }

    /// A list arrived: shown when it is the newest and its screen is open.
    pub(super) fn sessions_loaded(
        &mut self,
        ticket: u64,
        result: Result<Vec<SessionSummary>, String>,
    ) {
        if self.loads.sessions != Some(ticket) || self.ui.screen != Screen::Sessions {
            return;
        }
        self.loads.sessions = None;
        self.ui.sessions_loading = false;
        match result {
            Ok(list) => self.ui.sessions = list,
            Err(err) => self.ui.status = err,
        }
        let len = self.ui.sessions.len();
        self.ui.selected = self.ui.selected.min(len.saturating_sub(1));
    }

    /// Opens the Trace screen's read; it arrives in `Msg::Trace`.
    pub(super) fn load_trace(&mut self) {
        self.ui.trace_top = 0;
        let ticket = self.loads.ticket();
        self.loads.trace = Some(ticket);
        self.ui.trace.loading();
        let path = self.log_path.clone();
        let job = move || Msg::Trace(ticket, read_trace(&path));
        if !spawn("fast-harness-trace", self.tx.clone(), job) {
            let text = "cannot start the trace thread".to_string();
            self.ui.trace.set(Vec::new(), Vec::new(), Some(text));
        }
    }

    /// A trace read arrived: shown when it is the newest and its screen is open.
    pub(super) fn trace_loaded(&mut self, ticket: u64, result: TraceRead) {
        if self.loads.trace != Some(ticket) || self.ui.screen != Screen::Trace {
            return;
        }
        self.loads.trace = None;
        match result {
            Ok((entries, steps)) => self.ui.trace.set(entries, steps, None),
            Err(err) => self.ui.trace.set(Vec::new(), Vec::new(), Some(err)),
        }
    }
}
