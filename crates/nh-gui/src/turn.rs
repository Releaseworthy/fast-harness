//! One turn on a worker thread: the conversation moves into the thread,
//! events come back to the UI thread through the Slint event loop, and the
//! conversation returns when the turn ends.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::Ordering;
use std::thread;

use nh_core::agent::Conversation;
use nh_core::events::AgentEvent;

use crate::app::{show_error, with_app, App};
use crate::{accounts, MainWindow};

/// What the worker hands back when the turn is over.
enum Ended {
    Turn(Box<Conversation>),
    /// The worker panicked: the conversation is reloaded from its log.
    Lost(String),
}

/// Starts a turn with `text`; false (input kept) when it cannot start.
pub fn send(app: &mut App, ui: &MainWindow, text: &str) -> bool {
    let text = text.trim_end().to_string();
    if app.busy || text.trim().is_empty() {
        return false;
    }
    // Checked first so a refused send leaves no empty session behind.
    if let Err(err) = app.rt.auth() {
        show_error(ui, err);
        return false;
    }
    let conv = match app.conv.take() {
        Some(conv) => conv,
        None => match app.rt.open_conversation(&app.cwd, None, false) {
            Ok(conv) => conv,
            Err(err) => {
                show_error(ui, err);
                return false;
            }
        },
    };
    let agent = match app.rt.agent(&app.cwd, &conv) {
        Ok(agent) => agent,
        Err(err) => {
            app.conv = Some(conv);
            show_error(ui, err);
            return false;
        }
    };
    let new_session = app.conv_id.as_deref() != Some(conv.id.as_str());
    app.conv_id = Some(conv.id.clone());
    start(app, ui, text, conv, agent);
    if new_session {
        app.refresh_sessions(ui);
    }
    true
}

fn start(
    app: &mut App,
    ui: &MainWindow,
    text: String,
    conv: Conversation,
    agent: nh_core::agent::Agent,
) {
    ui.set_error_text("".into());
    ui.set_picker_note("".into());
    app.turn_from = app.view.items.len();
    app.view.push_user(text.as_str());
    app.sync_rows();
    app.busy = true;
    app.stopping = false;
    ui.set_busy(true);
    app.refresh_status(ui);
    app.cancel.store(false, Ordering::SeqCst);
    let cancel = app.cancel.clone();
    let id = conv.id.clone();
    let spawned = thread::Builder::new()
        .name("nh-turn".into())
        .spawn(move || {
            let mut conv = conv;
            let run = catch_unwind(AssertUnwindSafe(|| {
                let mut sink = |event: AgentEvent| {
                    let _ = slint::invoke_from_event_loop(move || deliver(event));
                };
                agent.run_turn(&mut conv, &text, &cancel, &mut sink)
            }));
            let ended = match run {
                Ok(_) => Ended::Turn(Box::new(conv)),
                Err(_) => Ended::Lost(id),
            };
            let _ = slint::invoke_from_event_loop(move || finish(ended));
        });
    if let Err(err) = spawned {
        app.busy = false;
        ui.set_busy(false);
        show_error(ui, format!("Cannot start the turn: {err}"));
    }
}

pub fn stop(app: &mut App, ui: &MainWindow) {
    if app.busy {
        app.cancel.store(true, Ordering::SeqCst);
        app.stopping = true;
        app.refresh_status(ui);
    }
}

/// Applies one agent event on the UI thread.
fn deliver(event: AgentEvent) {
    with_app(|app, ui| {
        app.view.apply(&event);
        app.sync_rows();
        app.refresh_status(ui);
        if matches!(
            event,
            AgentEvent::UsageUpdate { .. } | AgentEvent::AccountSwitch { .. }
        ) {
            accounts::refresh(app, ui);
        }
    });
}

fn finish(ended: Ended) {
    with_app(|app, ui| {
        match ended {
            Ended::Turn(conv) => app.conv = Some(*conv),
            Ended::Lost(id) => {
                show_error(
                    ui,
                    "The turn stopped on an internal error; the session was reloaded.",
                );
                app.conv = app.rt.open_conversation(&app.cwd, Some(&id), false).ok();
            }
        }
        app.busy = false;
        app.stopping = false;
        ui.set_busy(false);
        app.sync_rows();
        app.refresh_status(ui);
        app.refresh_sessions(ui);
        accounts::refresh(app, ui);
        if ui.get_page() == "trace" {
            app.refresh_trace(ui);
        }
    });
}
