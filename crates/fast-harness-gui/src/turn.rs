//! The window's turns run in `service::Service` (PLAN.md D39, D59): a turn
//! starts with `Service::start`, its events and its end come back to the UI
//! thread tagged with the session id, and steers, the queue, pause and Stop
//! go to the session. Each event changes its own session's transcript; the
//! rows, the status and the panels follow the shown session only.

use fast_harness_core::agent::Agent;
use fast_harness_core::app::Slash;
use fast_harness_core::events::{AgentEvent, TurnOutcome};
use fast_harness_core::service::{Done, Finished, Service, ServiceError, Sink, Start};
use fast_harness_core::view::{Item, NoteKind, State};

use crate::app::{show_error, with_app, App};
use crate::host::GuiHost;
use crate::runs::Runs;
use crate::{accounts, MainWindow};

/// Enter: starts a turn with `text`, or steers the shown session's running
/// turn. False (input kept) when nothing was taken.
pub fn send(app: &mut App, ui: &MainWindow, text: &str) -> bool {
    let text = text.trim_end().to_string();
    if text.trim().is_empty() {
        return false;
    }
    // A session still opening has no history to send with (D60, G10).
    if app.opens.loading() {
        crate::app::show_error(ui, crate::open_load::LOADING);
        return false;
    }
    if app.runs.shown.busy {
        // A turn that ended meanwhile: the session queues the text.
        if let Some(session) = &app.runs.shown.session {
            session.steer(text);
        }
        app.refresh_run(ui);
        return true;
    }
    start_text(app, ui, text)
}

/// Alt+Enter: queues a follow-up while the shown session runs, else sends.
pub fn queue(app: &mut App, ui: &MainWindow, text: &str) -> bool {
    let text = text.trim_end().to_string();
    let session = app.runs.shown.session.clone();
    let Some(session) = session.filter(|_| app.runs.shown.busy && !text.trim().is_empty()) else {
        return send(app, ui, &text);
    };
    session.queue(text);
    app.refresh_run(ui);
    true
}

/// The chip's x.
pub fn remove_queued(app: &mut App, ui: &MainWindow, index: i32) {
    if let (Ok(index), Some(session)) = (usize::try_from(index), &app.runs.shown.session) {
        session.remove_queued(index);
        app.refresh_run(ui);
    }
}

pub fn pause(app: &mut App, ui: &MainWindow) {
    if (app.runs.shown.session.as_ref()).is_some_and(|session| session.pause()) {
        app.refresh_status(ui);
    }
}

pub fn stop(app: &mut App, ui: &MainWindow) {
    let session = app.runs.shown.session.as_ref();
    if app.runs.shown.busy && session.is_some_and(|session| session.cancel()) {
        app.refresh_status(ui);
    }
}

/// Resume: continues the shown session's paused, cancelled or interrupted turn.
pub fn resume(app: &mut App, ui: &MainWindow) {
    // The shown session is about to be replaced (D60, G10).
    if app.opens.loading() {
        return show_error(ui, crate::open_load::LOADING);
    }
    let (Some(id), false) = (app.runs.id.clone(), app.runs.shown.busy) else {
        return;
    };
    let Some(session) = app.runs.shown.session.clone() else {
        return;
    };
    if !session.can_resume() {
        session.forget_unfinished();
        app.refresh_run(ui);
        return show_error(ui, "Nothing to resume: the last turn ended.");
    }
    launch(app, ui, &id, Start::Resume);
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
    // model picks its provider's credential.
    if let Err(err) = app.rt.route(&app.rt.config.model) {
        show_error(ui, err);
        return false;
    }
    match shown_session(app) {
        Ok(id) => launch(app, ui, &id, Start::Text(text)),
        Err(err) => {
            show_error(ui, err);
            false
        }
    }
}

/// The id of the shown session, opened in the service on its first message.
fn shown_session(app: &mut App) -> Result<String, String> {
    if let Some(session) = &app.runs.shown.session {
        return Ok(session.id.clone());
    }
    let conv = app.rt.open_conversation(&app.cwd, None, false)?;
    let session = app.service.open(conv, &app.cwd).map_err(error_text)?;
    // The window shows approval requests and answers them (D64).
    session.set_answering(true);
    app.runs.id = Some(session.id.clone());
    // The new session's composer now belongs to its id (D65).
    app.drafts.created(&session.id);
    app.runs.shown.cwd = app.cwd.clone();
    app.runs.shown.session = Some(session.clone());
    Ok(session.id.clone())
}

/// Runs `start` for session `id` (shown or parked) in the service.
fn launch(app: &mut App, ui: &MainWindow, id: &str, start: Start) -> bool {
    let built = match app.runs.get_mut(id) {
        Some(run) => match &run.session {
            Some(session) => (session.with_conv(|conv| app.rt.agent(&run.cwd, conv)))
                .map_err(error_text)
                .and_then(|built| built),
            None => return false,
        },
        None => return false,
    };
    let built = match built {
        Ok(built) => built,
        Err(err) => {
            show_error(ui, err);
            return false;
        }
    };
    let shown = app.runs.is_shown(id);
    if shown {
        ui.set_error_text("".into());
        ui.set_picker_note("".into());
    }
    let begun = start_run(&mut app.runs, (&app.service, &app.host), id, built, start, wires(id));
    if let Err(err) = &begun {
        show_error(ui, format!("Cannot start the turn: {err}"));
    }
    if shown {
        app.sync_rows();
        app.refresh_status(ui);
        app.refresh_run(ui);
        // The first turn opened the session in the service: its jobs (D68).
        crate::jobs_view::follow(app, ui);
    }
    // The sidebar marks the session running.
    app.refresh_sessions(ui);
    begun.is_ok()
}

/// Starts `start` in run `id` with its built agent and `notes` (prompt
/// files skipped, shown before the message they apply to).
pub(crate) fn start_run(
    runs: &mut Runs,
    (service, host): (&Service, &GuiHost),
    id: &str,
    (agent, notes): (Agent, Vec<String>),
    start: Start,
    (sink, done): (Sink, Done),
) -> Result<(), String> {
    let run = runs.get_mut(id).ok_or("no such session")?;
    let session = run.session.clone().ok_or("no such session")?;
    run.turn_from = run.view.items.len();
    for text in notes {
        let kind = NoteKind::Error;
        run.view.items.push(Item::Note { kind, text });
    }
    if let Start::Text(text) = &start {
        run.view.push_user(text.as_str());
    }
    host.prepare(id, agent);
    match service.start(&session, start, sink, done) {
        Ok(_) => {
            run.busy = true;
            Ok(())
        }
        Err(err) => {
            host.forget(id);
            Err(error_text(err))
        }
    }
}

/// The sink and the end of session `id`'s turn: both go to the UI thread.
fn wires(id: &str) -> (Sink, Done) {
    let tag = id.to_string();
    let sink: Sink = Box::new(move |event| {
        let tag = tag.clone();
        let _ = slint::invoke_from_event_loop(move || deliver(&tag, event));
    });
    let tag = id.to_string();
    let done: Done = Box::new(move |end: &Finished| {
        let end = end.clone();
        let _ = slint::invoke_from_event_loop(move || finish(&tag, end));
    });
    (sink, done)
}

/// Applies one event of session `id` on the UI thread: its own transcript
/// changes; the rows, the status and the panels only when it is shown.
fn deliver(id: &str, event: AgentEvent) {
    with_app(|app, ui| {
        let Some(shown) = app.runs.apply(id, &event) else {
            return;
        };
        if shown {
            // Panels follow the shown session (PLAN.md D46, D59).
            app.panels.session(&event);
            app.sync_rows();
            app.refresh_status(ui);
            if matches!(event, AgentEvent::UserInput { steer: true, .. }) {
                app.refresh_run(ui);
            }
        }
        // The prompt and the sidebar's waiting mark follow the gate (D64).
        if matches!(
            event,
            AgentEvent::ApprovalRequest(_) | AgentEvent::ApprovalDone { .. }
        ) {
            if shown {
                app.refresh_run(ui);
            }
            crate::workspaces::render(app, ui);
        }
        if matches!(
            event,
            AgentEvent::UsageUpdate { .. } | AgentEvent::AccountSwitch { .. }
        ) {
            accounts::refresh(app, ui);
        }
    });
}

/// Run `id`'s turn ended: the view leaves "running" (a turn that panicked
/// sent no `TurnEnd`), and after `Done` the next queued text comes back.
pub(crate) fn end_run(runs: &mut Runs, id: &str, end: &Finished) -> Option<String> {
    let run = runs.get_mut(id)?;
    run.busy = false;
    run.view.state = State::Idle;
    let session = run.session.as_ref()?;
    match end.outcome {
        TurnOutcome::Done => session.take_next(),
        _ => None,
    }
}

fn finish(id: &str, end: Finished) {
    with_app(|app, ui| {
        if end.panicked {
            show_error(ui, crash_text());
        }
        let next = end_run(&mut app.runs, id, &end);
        after_turn(app, ui, id, next);
    });
}

/// The sentence a panicked turn shows; the panic hook wrote the details.
pub(crate) fn crash_text() -> String {
    let log = crate::crash::path().map_or_else(
        || crate::crash::FILE_NAME.to_string(),
        |path| path.display().to_string(),
    );
    format!(
        "The turn stopped on an internal error; the session was reloaded. \
        What happened is in {log}."
    )
}

/// Starts the next queued text or settles the run, then refreshes the views.
fn after_turn(app: &mut App, ui: &MainWindow, id: &str, next: Option<String>) {
    match next {
        Some(text) => {
            launch(app, ui, id, Start::Text(text));
        }
        None => {
            if app.runs.settle(id) {
                app.service.close(id);
            }
        }
    }
    let shown = app.runs.is_shown(id);
    if shown {
        app.sync_rows();
        app.refresh_status(ui);
        app.refresh_run(ui);
    }
    app.refresh_sessions(ui);
    accounts::refresh(app, ui);
    if shown && ui.get_page() == "trace" {
        app.refresh_trace(ui);
    }
}

/// A service error as one line.
pub fn error_text(err: ServiceError) -> String {
    match err {
        ServiceError::Unknown(text)
        | ServiceError::Busy(text)
        | ServiceError::Invalid(text)
        | ServiceError::Failed(text) => text,
    }
}
