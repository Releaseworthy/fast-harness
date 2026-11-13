//! The shown session's subagents (PLAN.md D74): a pill above the composer
//! while the session has any, its popover, and a subagent's session opened
//! read-only in the chat view. The list comes from the child logs whose
//! header names the shown session (`session::children`, read by a worker),
//! merged with the live `Subagent` and `SubagentProgress` events for the
//! state and the latest step. Those events never reach the GUI transcript:
//! [`take`] keeps them out (the TUI keeps its inline notes). A subagent
//! shown while it runs is not mirrored live: its log opens again on its
//! events (at most every 500 ms, the core's progress rate).

use std::collections::HashMap;
use std::time::Duration;

use fast_harness_core::events::AgentEvent;
use fast_harness_core::session::{self, Child, Unfinished};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::app::{with_app, App};
use crate::jobs_view::took_text;
use crate::{MainWindow, SubagentRow, SubagentsUi};

/// What this window's events said about one subagent session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Live {
    pub running: bool,
    /// The latest progress line, `label · step N · tool`.
    pub line: String,
}

/// A subagent session shown read-only, and where Back goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Viewing {
    pub child: String,
    pub parent: String,
    pub back: String,
    /// The subagent's own label, the Back of its own children.
    pub label: String,
}

/// The subagents state on the UI thread.
#[derive(Default)]
pub struct Subagents {
    /// Every subagent session the window's events named.
    pub live: HashMap<String, Live>,
    /// The session whose children are listed, and its children.
    parent: Option<String>,
    pub children: Vec<Child>,
    /// Tells a list of the current scan from an earlier one's.
    generation: u64,
    /// The subagent sessions opened from a list, outermost first.
    pub viewing: Vec<Viewing>,
}

/// One subagent event applied to `live`: `Some(child id)` when `event` is
/// one (the transcript never gets it), `None` for any other event.
pub fn take<'a>(live: &mut HashMap<String, Live>, event: &'a AgentEvent) -> Option<&'a str> {
    match event {
        AgentEvent::Subagent { session, running } => {
            live.entry(session.clone()).or_default().running = *running;
            Some(session)
        }
        AgentEvent::SubagentProgress { session, line } => {
            let entry = live.entry(session.clone()).or_default();
            entry.line.clone_from(line);
            Some(session)
        }
        _ => None,
    }
}

/// `step N · tool` of a progress line (its label cut off).
fn progress(line: &str) -> &str {
    line.find("step ").map_or(line, |at| &line[at..])
}

/// The label a progress line starts with, if any.
fn live_label(line: &str) -> Option<&str> {
    line.split_once(" · step ").map(|(label, _)| label)
}

/// The mark and the word of a child that is not running.
fn ended(child: &Child) -> (&'static str, &'static str) {
    match child.outcome.as_deref() {
        Some("done") => ("finished", "finished"),
        Some("cancelled") => ("stopped", "cancelled"),
        Some("paused") => ("stopped", "paused"),
        Some(_) => ("failed", "failed"),
        None if child.unfinished == Some(Unfinished::Paused) => ("stopped", "paused"),
        None => ("stopped", "interrupted"),
    }
}

/// The popover's rows: each child's log, its state and step from the live
/// events while it runs.
pub fn rows(children: &[Child], live: &HashMap<String, Live>) -> Vec<SubagentRow> {
    let row = |child: &Child| {
        let seen = live.get(&child.id);
        let running = seen.is_some_and(|l| l.running);
        let line = seen.map(|l| l.line.as_str()).unwrap_or_default();
        let label = live_label(line).unwrap_or(&child.label);
        let logged = match &child.tool {
            Some(tool) => format!("step {} · {tool}", child.step),
            None => format!("step {}", child.step),
        };
        let (mark, word) = if running { ("running", "running") } else { ended(child) };
        let took = Duration::from_millis(child.last_time.saturating_sub(child.created_at));
        SubagentRow {
            id: child.id.as_str().into(),
            label: label.into(),
            mark: mark.into(),
            word: word.into(),
            progress: if running && !line.is_empty() { progress(line).into() } else { logged.into() },
            when: if running { String::new() } else { took_text(Some(took)) }.into(),
            detail: Default::default(),
        }
    };
    children.iter().map(row).collect()
}

/// Redraws the pill and the popover; closes it when no row is left.
pub fn render(app: &App, ui: &MainWindow) {
    let s = &app.subagents;
    let rows = rows(&s.children, &s.live);
    let subs = ui.global::<SubagentsUi>();
    let running = rows.iter().filter(|r| r.mark == "running").count();
    subs.set_running(running as i32);
    if rows.is_empty() {
        subs.set_open(false);
    }
    subs.set_rows(ModelRc::new(VecModel::from(rows)));
    let top = s.viewing.last();
    let shown = app.runs.id.as_deref();
    subs.set_in_child(top.is_some_and(|v| Some(v.child.as_str()) == shown));
    subs.set_back_label(top.map(|v| v.back.as_str()).unwrap_or_default().into());
}

/// After a switch: lists the shown session's children (a worker reads
/// them), and leaves the subagent views the window no longer shows.
pub fn follow(app: &mut App, ui: &MainWindow) {
    let shown = app.runs.id.clone();
    let s = &mut app.subagents;
    let kept = s.viewing.iter().position(|v| Some(&v.child) == shown.as_ref());
    s.viewing.truncate(kept.map_or(0, |at| at + 1));
    if s.parent != shown {
        s.parent = shown;
        s.children.clear();
        ui.global::<SubagentsUi>().set_open(false);
        scan(app);
    }
    render(app, ui);
}

/// Asks a worker for the shown session's children.
fn scan(app: &mut App) {
    let s = &mut app.subagents;
    s.generation += 1;
    let Some(parent) = s.parent.clone() else {
        return;
    };
    let (home, cwd, generation) = (app.rt.home.clone(), app.cwd.clone(), s.generation);
    let spawned = std::thread::Builder::new()
        .name("fast-harness-gui-subagents".into())
        .spawn(move || {
            let found = session::children(&home, &cwd, &parent).unwrap_or_default();
            let _ = slint::invoke_from_event_loop(move || {
                with_app(|app, ui| scanned(app, ui, generation, found));
            });
        });
    drop(spawned);
}

/// A list from the worker: shown unless an earlier scan sent it.
pub fn scanned(app: &mut App, ui: &MainWindow, generation: u64, found: Vec<Child>) {
    if app.subagents.generation == generation {
        app.subagents.children = found;
        render(app, ui);
    }
}

/// One event of any session's turn: a subagent event updates the list (a
/// start or an end reads the children again) and a shown running subagent
/// opens again; it never reaches the transcript. `true` when consumed.
pub fn event(app: &mut App, ui: &MainWindow, event: &AgentEvent) -> bool {
    let Some(id) = take(&mut app.subagents.live, event) else {
        return false;
    };
    if matches!(event, AgentEvent::Subagent { .. }) {
        scan(app);
    }
    let viewed = app.subagents.viewing.last().is_some_and(|v| v.child == id);
    if viewed && app.runs.id.as_deref() == Some(id) {
        crate::open_load::reload(app, ui, id);
    }
    render(app, ui);
    true
}

/// A row's click: subagent `id` of the shown session, read-only.
pub fn open_child(app: &mut App, ui: &MainWindow, id: &str) {
    ui.global::<SubagentsUi>().set_open(false);
    let Some(parent) = app.runs.id.clone() else {
        return;
    };
    let s = &mut app.subagents;
    let back = match s.viewing.last() {
        Some(top) if top.child == parent => top.label.clone(),
        _ => "parent session".to_string(),
    };
    let rows = rows(&s.children, &s.live);
    let row = rows.iter().find(|r| r.id == id);
    let label = row.map_or("subagent".to_string(), |r| r.label.to_string());
    let child = id.to_string();
    s.viewing.push(Viewing { child, parent, back, label });
    app.open_session(ui, id);
}
/// Back: the session the shown subagent was opened from.
pub fn back(app: &mut App, ui: &MainWindow) {
    let shown = app.runs.id.as_deref();
    let Some(top) = app.subagents.viewing.last().filter(|v| Some(v.child.as_str()) == shown)
    else {
        return;
    };
    let parent = top.parent.clone();
    app.open_session(ui, &parent);
}

/// Wires the popover's callbacks.
pub fn connect(ui: &MainWindow) {
    let subs = ui.global::<SubagentsUi>();
    subs.on_open_child(|id| { with_app(|app, ui| open_child(app, ui, &id)); });
    subs.on_back(|| { with_app(back); });
}

#[cfg(test)]
#[path = "subagents_view_tests.rs"]
mod tests;
