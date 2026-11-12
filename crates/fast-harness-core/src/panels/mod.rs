//! UI panels (PLAN.md D46, docs/panels.md): `panels/*.json` of the user
//! folder and a trusted project folder name a command that draws a view
//! tree over JSON lines. Indexing reads only the files; a panel's command
//! starts when the panel first opens and is killed with its process tree on
//! [`Panels::stop_all`] (or when its [`Process`] drops). No UI types here:
//! the GUI and the TUI draw the flat [`Row`]s.

mod file;
mod native;
mod process;
mod proto;
mod view;

#[cfg(test)]
mod tests;

pub use file::{load, PanelFile, Place};
pub use native::{NativePanel, NativeRef};
pub use process::{Msg, Process};
pub use proto::{event_line, init_line, parse_line, session_event, Incoming};
pub use view::{flatten, patch, Cell, Row};

use std::path::Path;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::events::AgentEvent;

/// Called from a panel's reader thread after it queued a message, so the
/// front end can wake its UI thread and call [`Panels::poll`].
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// What a panel's `init` message says about the host.
#[derive(Clone, Copy, Debug)]
pub struct Host<'a> {
    pub cwd: &'a Path,
    pub session: &'a str,
    pub dark: bool,
}

/// One panel: its file, its process once opened and its current view.
pub struct Panel {
    pub file: PanelFile,
    process: Option<Process>,
    root: Option<Value>,
    pub rows: Vec<Row>,
    /// The last `notify` text, a protocol problem, or why the process ended.
    pub status: String,
    /// The process ended (or never started); opening the panel restarts it.
    pub ended: bool,
    events: Vec<String>,
    /// A native panel (D52): its answers wait here for [`Panel::poll`],
    /// like a process's lines, and `wake` tells the front end.
    queued: Mutex<Vec<Incoming>>,
    wake: Option<Wake>,
}

impl Panel {
    pub fn new(file: PanelFile) -> Panel {
        Panel {
            file,
            process: None,
            root: None,
            rows: Vec::new(),
            status: String::new(),
            ended: false,
            events: Vec::new(),
            queued: Mutex::default(),
            wake: None,
        }
    }

    /// Whether the command was started (it never is before the first open).
    pub fn started(&self) -> bool {
        self.process.is_some() || self.wake.is_some()
    }

    /// Queues a native panel's answer and wakes the front end.
    fn queue(&self, messages: Vec<Incoming>) {
        if messages.is_empty() {
            return;
        }
        self.queued
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .extend(messages);
        if let Some(wake) = &self.wake {
            wake();
        }
    }

    /// Starts the command unless it runs; a start failure becomes the status.
    pub fn open(&mut self, host: Host<'_>, wake: Wake) {
        if let Some(native) = self.file.native.clone() {
            if self.wake.is_none() {
                self.wake = Some(wake);
                self.queue(native.0.open(host));
            }
            return;
        }
        if self.process.is_some() && !self.ended {
            return;
        }
        self.stop();
        self.ended = false;
        self.status = "starting...".into();
        let init = init_line(&self.file.id, host);
        match Process::start(&self.file, host.cwd, init, wake) {
            Ok(process) => self.process = Some(process),
            Err(err) => {
                self.status = format!("cannot start {}: {err}", self.file.command);
                self.ended = true;
            }
        }
    }

    /// Applies every queued message; true when the view or status changed.
    pub fn poll(&mut self) -> bool {
        let queued = std::mem::take(&mut *self.queued.lock().unwrap_or_else(|p| p.into_inner()));
        let mut changed = !queued.is_empty();
        for incoming in queued {
            self.apply(incoming);
        }
        while let Some(msg) = self.process.as_ref().and_then(Process::try_recv) {
            changed = true;
            match msg {
                Msg::Line(incoming) => self.apply(incoming),
                Msg::Ended(why) => {
                    self.ended = true;
                    self.status = why;
                }
            }
        }
        changed
    }

    fn apply(&mut self, incoming: Incoming) {
        match incoming {
            Incoming::View(root) => {
                if self.status == "starting..." {
                    self.status.clear();
                }
                self.rows = flatten(&root);
                self.root = Some(root);
            }
            Incoming::Patch { id, node } => {
                let found = self.root.as_mut().is_some_and(|r| patch(r, &id, node));
                match (found, &self.root) {
                    (true, Some(root)) => self.rows = flatten(root),
                    _ => self.status = format!("patch: no node with id {id}"),
                }
            }
            Incoming::Subscribe(events) => self.events = events,
            Incoming::Notify(text) => self.status = text,
            Incoming::Bad(why) => self.status = format!("panel protocol: {why}"),
        }
    }

    /// Sends a `click`, `submit` or `change` event to the running process.
    pub fn send(&self, kind: &str, id: &str, value: Value) {
        if let Some(native) = self.file.native.as_ref().filter(|_| self.wake.is_some()) {
            return self.queue(native.0.event(kind, id, &value));
        }
        if let Some(process) = self.process.as_ref().filter(|_| !self.ended) {
            process.send(event_line(kind, id, value));
        }
    }

    /// Sends `event` when the panel subscribed to its kind.
    pub fn session(&self, event: &AgentEvent) {
        let Some((kind, body)) = session_event(event) else {
            return;
        };
        if self.events.iter().any(|e| e == kind) {
            if let Some(native) = self.file.native.as_ref().filter(|_| self.wake.is_some()) {
                return self.queue(native.0.session(event));
            }
            let line = serde_json::json!({ "type": "session", "event": body });
            if let Some(process) = self.process.as_ref().filter(|_| !self.ended) {
                process.send(line.to_string());
            }
        }
    }

    /// Kills the process tree; the view stays.
    pub fn stop(&mut self) {
        if let Some(process) = self.process.take() {
            process.kill();
        }
    }
}

/// The id of a `/panel [id]` command ("" lists the panels); `None` for
/// other text.
pub fn slash_id(text: &str) -> Option<&str> {
    let rest = text.trim().strip_prefix("/panel")?;
    (rest.is_empty() || rest.starts_with(char::is_whitespace)).then(|| rest.trim())
}

/// The `/panel` text: every panel with its title, place and layer.
pub fn listing(panels: &Panels) -> String {
    if panels.list.is_empty() {
        return "No panels: add panels/<id>.json to the config folder (docs/panels.md).".into();
    }
    let mut out = String::from("Panels (/panel <id> opens one):");
    for panel in &panels.list {
        let file = &panel.file;
        out.push_str(&format!(
            "\n  {}: {} ({}, {})",
            file.id,
            file.title,
            file.place.word(),
            file.source
        ));
    }
    out
}

/// Every panel of the loaded files.
#[derive(Default)]
pub struct Panels {
    pub list: Vec<Panel>,
}

impl Panels {
    pub fn new(files: &[PanelFile]) -> Panels {
        Panels {
            list: files.iter().cloned().map(Panel::new).collect(),
        }
    }

    pub fn get(&self, id: &str) -> Option<&Panel> {
        self.list.iter().find(|p| p.file.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Panel> {
        self.list.iter_mut().find(|p| p.file.id == id)
    }

    /// [`Panel::poll`] of every panel; true when one changed.
    pub fn poll(&mut self) -> bool {
        // Every panel polls (no short circuit).
        let mut changed = false;
        for panel in &mut self.list {
            changed |= panel.poll();
        }
        changed
    }

    pub fn session(&self, event: &AgentEvent) {
        for panel in &self.list {
            panel.session(event);
        }
    }

    /// Kills every panel's process tree (app exit).
    pub fn stop_all(&mut self) {
        for panel in &mut self.list {
            panel.stop();
        }
    }
}
