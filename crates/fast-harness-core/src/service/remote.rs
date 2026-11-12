//! Text for a session that may run in another process (PLAN.md D49): with
//! a live lock it becomes one line of the session's inbox file, read by the
//! running turn at its next step; else this process runs the turn itself.

use std::path::Path;
use std::sync::mpsc;

use super::{Finished, Service, ServiceError, Sink, Start};
use crate::session::coord::{self, Command};
use crate::session::{self, lock};

/// How a running turn takes the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// At its next step boundary.
    Steer,
    /// As a new turn after it ends `Done`.
    Queue,
}

/// What [`Service::send_to`] did.
#[derive(Debug)]
pub enum Sent {
    /// Appended to the inbox of the session that process `pid` runs.
    Queued { pid: u32, program: String },
    /// Nobody ran the session: this process ran the turn.
    Ran(Finished),
}

impl Service {
    /// Sends `text` to session `id` (any directory under the home). The
    /// turn this process runs reports its events to `sink`.
    pub fn send_to(
        &self,
        id: &str,
        text: &str,
        mode: Mode,
        sink: Sink,
    ) -> Result<Sent, ServiceError> {
        let home = self.host.home();
        let log = lock::find(&home, id)
            .ok_or_else(|| ServiceError::Unknown(format!("no session {id}")))?;
        if let Some(lock) = lock::live_lock(&log) {
            let command = match mode {
                Mode::Steer => Command::Steer(text.to_string()),
                Mode::Queue => Command::FollowUp(text.to_string()),
            };
            let path = coord::inbox_path(&log);
            coord::append(&path, &command)
                .map_err(|err| ServiceError::Failed(format!("{}: {err}", path.display())))?;
            let (pid, program) = (lock.pid, lock.program);
            return Ok(Sent::Queued { pid, program });
        }
        let header = session::read_header(&log)
            .ok_or_else(|| ServiceError::Failed(format!("{id}: the log has no header")))?;
        let session = self.resume(Path::new(&header.cwd), id)?;
        let (tx, rx) = mpsc::channel();
        let done = Box::new(move |finished: &Finished| {
            let _ = tx.send(finished.clone());
        });
        self.start(&session, Start::Text(text.to_string()), sink, done)?;
        let finished = rx
            .recv()
            .map_err(|_| ServiceError::Failed("the turn ended without a result".into()))?;
        session.wait();
        Ok(Sent::Ran(finished))
    }
}
