//! The sidebar's session lists, read on worker threads (D60, chunk 26B).
//! At most one listing runs per workspace; a request made while one runs
//! is remembered and the newest starts when it ends. A result for a
//! workspace that was collapsed or removed, or older than the newest
//! request, is dropped. The rows shown stay until a newer list arrives.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::thread;

use fast_harness_core::session::{self, SessionSummary};

/// What to do with a listing that ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Arrival {
    /// The newest for an expanded workspace: store and show it.
    Apply,
    /// The workspace was collapsed or removed: nothing changes.
    Drop,
    /// A newer request came meanwhile: start this ticket instead.
    Again(u64),
}

#[derive(Default)]
pub struct Listings {
    /// The last list received per workspace slug.
    shown: HashMap<String, Vec<SessionSummary>>,
    /// The newest request per slug.
    asked: HashMap<String, u64>,
    /// The ticket whose worker runs, per slug.
    running: HashMap<String, u64>,
    next: u64,
}

impl Listings {
    /// The list shown for `key`, if one arrived.
    pub fn sessions(&self, key: &str) -> Option<&[SessionSummary]> {
        self.shown.get(key).map(Vec::as_slice)
    }

    /// A new request for `key`: the ticket a worker starts with, or `None`
    /// when one runs (the newest request starts when it ends).
    pub fn ask(&mut self, key: &str) -> Option<u64> {
        self.next += 1;
        self.asked.insert(key.to_string(), self.next);
        if self.running.contains_key(key) {
            return None;
        }
        self.running.insert(key.to_string(), self.next);
        Some(self.next)
    }

    /// The worker of `ticket` for `key` ended; `open`: the workspace is
    /// still listed and expanded.
    pub fn arrive(&mut self, key: &str, ticket: u64, open: bool) -> Arrival {
        if self.running.get(key) == Some(&ticket) {
            self.running.remove(key);
        }
        if !open {
            self.forget(key);
            return Arrival::Drop;
        }
        match self.asked.get(key).copied() {
            Some(newest) if newest == ticket => Arrival::Apply,
            Some(newest) => {
                self.running.insert(key.to_string(), newest);
                Arrival::Again(newest)
            }
            None => Arrival::Drop,
        }
    }

    pub fn store(&mut self, key: &str, list: Vec<SessionSummary>) {
        self.shown.insert(key.to_string(), list);
    }

    /// A collapsed or removed workspace: its rows and requests go; a
    /// running worker stays counted, so a re-expand waits for it.
    pub fn forget(&mut self, key: &str) {
        self.shown.remove(key);
        self.asked.remove(key);
    }

    /// The worker of `key` could not start.
    pub fn failed(&mut self, key: &str) {
        self.running.remove(key);
        self.asked.remove(key);
    }
}

/// One listing's result, handed to the UI thread.
pub type Listed = Result<Vec<SessionSummary>, String>;

/// Lists workspace `path` on a thread; `done` gets the result there and
/// hands it to the UI thread.
pub fn spawn(
    home: PathBuf,
    path: PathBuf,
    done: impl FnOnce(Listed) + Send + 'static,
) -> io::Result<()> {
    thread::Builder::new()
        .name("fast-harness-gui-list".into())
        .spawn(move || done(session::list(&home, &path).map_err(|err| err.to_string())))
        .map(|_| ())
}

#[cfg(test)]
#[path = "listings_tests.rs"]
mod tests;
