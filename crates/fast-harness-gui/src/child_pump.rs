//! A running subagent shown in the chat view follows its live events
//! (PLAN.md D79): a pump thread reads `live::watch` of the child and hands
//! each event to the shown run as a turn's events go (`turn::deliver`); the
//! run is busy meanwhile, so Stop, Pause and the composer's steer reach the
//! child (`turn.rs` routes them to `live`). When the child ends the channel
//! closes: the run leaves busy and its log opens again, so the service
//! session holds the whole conversation for the next prompt.

use fast_harness_core::agent::live;
use fast_harness_core::view::State;

use crate::app::{with_app, App};
use crate::MainWindow;

/// Starts following child session `id` when it runs and no pump follows it.
/// `true` when a pump started.
pub fn follow(app: &mut App, id: &str) -> bool {
    if app.subagents.pumped.as_deref() == Some(id) || app.runs.get_mut(id).is_none() {
        return false;
    }
    let Some(events) = live::watch(id) else {
        return false;
    };
    if let Some(run) = app.runs.get_mut(id) {
        run.busy = true;
    }
    app.subagents.pumped = Some(id.to_string());
    let tag = id.to_string();
    let spawned = std::thread::Builder::new()
        .name("fast-harness-gui-child".into())
        .spawn(move || {
            for event in events {
                let tag = tag.clone();
                let _ = slint::invoke_from_event_loop(move || crate::turn::deliver(&tag, event));
            }
            let _ = slint::invoke_from_event_loop(move || {
                with_app(|app, ui| ended(app, ui, &tag));
            });
        });
    if spawned.is_err() {
        app.subagents.pumped = None;
    }
    spawned.is_ok()
}

/// The followed child ended: its run is idle and, when shown, opens again
/// from its log.
pub fn ended(app: &mut App, ui: &MainWindow, id: &str) {
    if app.subagents.pumped.as_deref() == Some(id) {
        app.subagents.pumped = None;
    }
    if let Some(run) = app.runs.get_mut(id) {
        run.busy = false;
        run.view.state = State::Idle;
    }
    if app.runs.id.as_deref() == Some(id) {
        crate::open_load::reload(app, ui, id);
    }
}
