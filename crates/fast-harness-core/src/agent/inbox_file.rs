//! The inbox file and lock of a running top-level session (PLAN.md D49):
//! attached when the turns start, read once per step boundary into the
//! same queues as the front end's channel, removed when the turns end.

use std::io::ErrorKind;
use std::path::Path;

use super::{Conversation, Inbox};
use crate::session::coord::{self, Command, FileInbox};
use crate::session::lock::Held;

pub(crate) struct Watch {
    inbox: FileInbox,
    lock: Held,
}

impl Inbox {
    /// Takes the lock of `conv` and starts reading its inbox file, a
    /// subagent session's too (PLAN.md D79). Another process holding
    /// the lock is the error (`session::lock::busy_note`): the turn must not run.
    pub(crate) fn attach(&mut self, conv: &Conversation, cwd: &Path) -> Result<(), String> {
        self.stopped = false;
        if self.file.is_some() {
            return Ok(());
        }
        let cwd = cwd.to_string_lossy();
        let lock = match Held::take(&conv.log_path, &conv.id, &cwd, conv.turn) {
            Ok(lock) => lock,
            Err(err) if err.kind() == ErrorKind::AlreadyExists => return Err(err.to_string()),
            // No lock (a read-only folder): the turn runs without coordination.
            Err(_) => return Ok(()),
        };
        let inbox = FileInbox::new(coord::inbox_path(&conv.log_path));
        self.file = Some(Watch { inbox, lock });
        Ok(())
    }

    /// One step boundary: one size check of the inbox file, its new lines
    /// into the queues, and the lock's state and step.
    pub(crate) fn boundary(&mut self, turn: u32, step: u32) {
        let Some(watch) = &mut self.file else {
            return;
        };
        let commands = watch.inbox.poll();
        self.apply(commands);
        let state = match (self.stopped, self.paused) {
            (true, _) => "stopping",
            (_, true) => "pausing",
            _ => "running",
        };
        if let Some(watch) = &mut self.file {
            watch.lock.update(state, turn, step);
        }
    }

    fn apply(&mut self, commands: Vec<Command>) {
        for command in commands {
            match command {
                Command::Steer(text) => self.steers.push(text),
                Command::FollowUp(text) => self.follow_ups.push_back(text),
                Command::Pause => self.paused = true,
                Command::Stop => self.stopped = true,
            }
        }
    }

    /// The turns ended: a last read (unused texts stay in
    /// [`Inbox::leftover`]), then the inbox file and the lock go.
    pub(crate) fn detach(&mut self) {
        let Some(mut watch) = self.file.take() else {
            return;
        };
        let commands = watch.inbox.poll();
        self.apply(commands);
        watch.inbox.remove();
    }

    /// A stop line arrived through the inbox file.
    pub fn stopped(&self) -> bool {
        self.stopped
    }
}
