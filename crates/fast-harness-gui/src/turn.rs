//! Turns on worker threads, one per running session (PLAN.md D39): the
//! conversation moves into the thread, events come back to the UI thread
//! tagged with the session id, and the conversation returns when the turn
//! ends. Only the shown session's events touch the transcript rows.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::thread;

use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::app::Slash;
use fast_harness_core::events::{AgentEvent, TurnOutcome};
use fast_harness_core::view::{Item, NoteKind};

use crate::app::{show_error, with_app, App};
use crate::{accounts, MainWindow};

/// What a turn thread does: a new user text, or resume the last turn.
enum Job {
    Text(String),
    Resume,
}

/// What the worker hands back when the turn is over.
enum Ended {
    Turn(Box<Conversation>, TurnOutcome, Vec<String>),
    /// The worker panicked: the conversation is reloaded from its log.
    Lost,
}

/// Enter: starts a turn with `text`, or steers the shown session's running
/// turn. False (input kept) when nothing was taken.
pub fn send(app: &mut App, ui: &MainWindow, text: &str) -> bool {
    let text = text.trim_end().to_string();
    if text.trim().is_empty() {
        return false;
    }
    if app.runs.shown.busy {
        app.runs.shown.steer(text);
        app.refresh_run(ui);
        return true;
    }
    start_text(app, ui, text)
}

/// Alt+Enter: queues a follow-up while the shown session runs, else sends.
pub fn queue(app: &mut App, ui: &MainWindow, text: &str) -> bool {
    let text = text.trim_end().to_string();
    if !app.runs.shown.busy || text.trim().is_empty() {
        return send(app, ui, &text);
    }
    app.runs.shown.queue(text);
    app.refresh_run(ui);
    true
}

/// The chip's x.
pub fn remove_queued(app: &mut App, ui: &MainWindow, index: i32) {
    if let Ok(index) = usize::try_from(index) {
        app.runs.shown.remove_queued(index);
        app.refresh_run(ui);
    }
}

pub fn pause(app: &mut App, ui: &MainWindow) {
    if app.runs.shown.pause() {
        app.refresh_status(ui);
    }
}

pub fn stop(app: &mut App, ui: &MainWindow) {
    if app.runs.shown.stop() {
        app.refresh_status(ui);
    }
}

/// Resume: continues the shown session's paused, cancelled or interrupted turn.
pub fn resume(app: &mut App, ui: &MainWindow) {
    let (Some(id), false) = (app.runs.id.clone(), app.runs.shown.busy) else {
        return;
    };
    let Some(mut conv) = app.runs.shown.conv.take() else {
        return;
    };
    if !conv.can_resume() {
        app.runs.shown.conv = Some(conv);
        app.runs.shown.unfinished = None;
        app.refresh_run(ui);
        return show_error(ui, "Nothing to resume: the last turn ended.");
    }
    launch(app, ui, &id, conv, Job::Resume);
}

/// Starts a turn with `text` in the shown session (created on the first message).
fn start_text(app: &mut App, ui: &MainWindow, text: String) -> bool {
    // `/trust` trusts the project's .fast-harness folder (PLAN.md D41).
    if text.trim() == "/trust" {
        let cwd = app.cwd.clone();
        match app.rt.trust_project(&cwd) {
            Ok(note) => ui.set_error_text(note.into()),
            Err(err) => show_error(ui, err),
        }
        return false;
    }
    // `/panel [id]` opens a panel or lists them (PLAN.md D46).
    if crate::panels::slash(app, ui, &text) {
        return false;
    }
    // `/reflect [apply]` runs off the UI thread (PLAN.md D50).
    if let Some(args) = fast_harness_core::app::reflect_args(&text) {
        crate::reflect::start(app, ui, args);
        return false;
    }
    // `/help` and the user's commands/*.md (PLAN.md D42).
    let text = match app.rt.slash(&text) {
        Slash::Send(text) => text,
        Slash::Help(help) => {
            ui.set_error_text(help.into());
            return false;
        }
        Slash::Error(err) => {
            show_error(ui, err);
            return false;
        }
    };
    // Checked first so a refused send leaves no empty session behind; the
    // model picks Claude's credential or the Google accounts.
    if let Err(err) = app.rt.route(&app.rt.config.model) {
        show_error(ui, err);
        return false;
    }
    let conv = match app.runs.shown.conv.take() {
        Some(conv) => conv,
        None => match app.rt.open_conversation(&app.cwd, None, false) {
            Ok(conv) => conv,
            Err(err) => {
                show_error(ui, err);
                return false;
            }
        },
    };
    let id = conv.id.clone();
    if app.runs.id.as_deref() != Some(id.as_str()) {
        app.runs.id = Some(id.clone());
        app.runs.shown.cwd = app.cwd.clone();
    }
    launch(app, ui, &id, conv, Job::Text(text))
}

/// Runs `job` for session `id` (shown or parked) on a thread.
fn launch(app: &mut App, ui: &MainWindow, id: &str, conv: Conversation, job: Job) -> bool {
    let Some(run) = app.runs.get_mut(id) else {
        return false;
    };
    let (agent, notes) = match app.rt.agent(&run.cwd, &conv) {
        Ok(made) => made,
        Err(err) => {
            run.conv = Some(conv);
            show_error(ui, err);
            return false;
        }
    };
    run.turn_from = run.view.items.len();
    // Prompt files skipped (unreadable), before the message they apply to.
    for text in notes {
        let kind = NoteKind::Error;
        run.view.items.push(Item::Note { kind, text });
    }
    if let Job::Text(text) = &job {
        run.view.push_user(text.as_str());
    }
    let (cancel, inbox) = run.begin();
    let cwd = run.cwd.clone();
    let shown = app.runs.is_shown(id);
    if shown {
        ui.set_error_text("".into());
        ui.set_picker_note("".into());
    }
    let spawned = spawn(id.to_string(), agent, conv, job, (cancel, inbox));
    if let Err(err) = &spawned {
        if let Some(run) = app.runs.get_mut(id) {
            run.end(None, TurnOutcome::Failed, Vec::new());
            run.conv = app.rt.open_conversation(&cwd, Some(id), false).ok();
        }
        show_error(ui, format!("Cannot start the turn: {err}"));
    }
    if shown {
        app.sync_rows();
        app.refresh_status(ui);
        app.refresh_run(ui);
    }
    // The sidebar marks the session running.
    app.refresh_sessions(ui);
    spawned.is_ok()
}

type Wires = (
    std::sync::Arc<std::sync::atomic::AtomicBool>,
    fast_harness_core::agent::Inbox,
);

fn spawn(
    id: String,
    agent: Agent,
    mut conv: Conversation,
    job: Job,
    (cancel, mut inbox): Wires,
) -> std::io::Result<()> {
    thread::Builder::new()
        .name("fast-harness-turn".into())
        .spawn(move || {
            let tag = id.clone();
            let run = catch_unwind(AssertUnwindSafe(|| {
                let mut sink = |event: AgentEvent| {
                    let tag = tag.clone();
                    let _ = slint::invoke_from_event_loop(move || deliver(&tag, event));
                };
                match job {
                    Job::Text(text) => {
                        agent.run_turn_with(&mut conv, &text, &cancel, &mut inbox, &mut sink)
                    }
                    Job::Resume => agent.resume_turn(&mut conv, &cancel, &mut inbox, &mut sink),
                }
            }));
            let ended = match run {
                Ok(outcome) => Ended::Turn(Box::new(conv), outcome, inbox.leftover()),
                Err(_) => Ended::Lost,
            };
            let _ = slint::invoke_from_event_loop(move || finish(&id, ended));
        })
        .map(|_| ())
}

/// Applies one agent event of session `id` on the UI thread; the rows and
/// the status change only for the shown session.
fn deliver(id: &str, event: AgentEvent) {
    with_app(|app, ui| {
        let Some(run) = app.runs.get_mut(id) else {
            return;
        };
        run.apply(&event);
        // Panels subscribed to turn, tool or usage events (PLAN.md D46).
        app.panels.session(&event);
        if app.runs.is_shown(id) {
            app.sync_rows();
            app.refresh_status(ui);
            if matches!(event, AgentEvent::UserInput { steer: true, .. }) {
                app.refresh_run(ui);
            }
        }
        if matches!(
            event,
            AgentEvent::UsageUpdate { .. } | AgentEvent::AccountSwitch { .. }
        ) {
            accounts::refresh(app, ui);
        }
    });
}

fn finish(id: &str, ended: Ended) {
    with_app(|app, ui| {
        let Some(run) = app.runs.get_mut(id) else {
            return;
        };
        let next = match ended {
            Ended::Turn(conv, outcome, leftover) => run.end(Some(*conv), outcome, leftover),
            Ended::Lost => {
                show_error(
                    ui,
                    "The turn stopped on an internal error; the session was reloaded.",
                );
                let conv = app.rt.open_conversation(&run.cwd, Some(id), false).ok();
                run.end(conv, TurnOutcome::Failed, Vec::new())
            }
        };
        after_turn(app, ui, id, next);
    });
}

/// Starts the next queued text or settles the run, then refreshes the views.
fn after_turn(app: &mut App, ui: &MainWindow, id: &str, next: Option<String>) {
    let conv = (app.runs.get_mut(id)).and_then(|run| run.conv.take());
    match (next, conv) {
        (Some(text), Some(conv)) => {
            launch(app, ui, id, conv, Job::Text(text));
        }
        (next, conv) => {
            if let Some(run) = app.runs.get_mut(id) {
                run.conv = conv;
                run.queued.splice(0..0, next);
            }
            app.runs.settle(id);
        }
    }
    let shown = app.runs.is_shown(id);
    if shown {
        app.sync_rows();
        app.refresh_status(ui);
        app.refresh_run(ui);
    }
    app.refresh_sessions(ui);
    // Schedules the Google quota read of the account the turn used (D23).
    app.rt.google.after_turn(fast_harness_core::trace::now_ms());
    accounts::refresh(app, ui);
    if shown && ui.get_page() == "trace" {
        app.refresh_trace(ui);
    }
}
