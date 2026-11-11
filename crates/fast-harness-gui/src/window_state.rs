//! Minimize, maximize or restore and the title-bar double click go through
//! one Rust path that reads the real window state; the `win-maximized`
//! property the restore glyph and the window edge follow is set from that
//! state, and a timer re-reads it so Win+Up or Aero snap keep it in sync.

use std::cell::Cell;
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Timer, TimerMode};

use crate::MainWindow;

/// Two clicks on the empty title bar within this time maximize or restore.
const DOUBLE_CLICK: Duration = Duration::from_millis(500);

/// How often the real window state is re-read.
const SYNC_EVERY: Duration = Duration::from_millis(200);

thread_local! {
    static SYNC: Timer = Timer::default();
}

/// True when a click at `now` follows the one at `last` within `DOUBLE_CLICK`.
fn is_double(last: Option<Instant>, now: Instant) -> bool {
    last.is_some_and(|t| now.saturating_duration_since(t) < DOUBLE_CLICK)
}

pub fn connect(ui: &MainWindow) {
    let weak = ui.as_weak();
    ui.on_window_minimize(move || {
        if let Some(ui) = weak.upgrade() {
            ui.window().set_minimized(true);
        }
    });
    let weak = ui.as_weak();
    ui.on_window_toggle_maximize(move || {
        if let Some(ui) = weak.upgrade() {
            toggle(&ui);
        }
    });
    let weak = ui.as_weak();
    let last = Cell::new(None::<Instant>);
    ui.on_title_bar_clicked(move || {
        let now = Instant::now();
        let double = is_double(last.get(), now);
        last.set(if double { None } else { Some(now) });
        if let (true, Some(ui)) = (double, weak.upgrade()) {
            toggle(&ui);
        }
    });
    start_sync(ui);
}

/// Maximizes or restores from the real state, not from a property.
fn toggle(ui: &MainWindow) {
    let window = ui.window();
    let maximize = !window.is_maximized();
    window.set_maximized(maximize);
    ui.set_win_maximized(maximize);
}

fn sync(ui: &MainWindow) {
    let maximized = ui.window().is_maximized();
    if ui.get_win_maximized() != maximized {
        ui.set_win_maximized(maximized);
    }
}

fn start_sync(ui: &MainWindow) {
    sync(ui);
    let weak = ui.as_weak();
    SYNC.with(|timer| {
        timer.start(TimerMode::Repeated, SYNC_EVERY, move || {
            if let Some(ui) = weak.upgrade() {
                sync(&ui);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_click_needs_two_clicks_within_the_interval() {
        let t = Instant::now();
        assert!(!is_double(None, t));
        assert!(is_double(Some(t), t + Duration::from_millis(300)));
        assert!(!is_double(Some(t), t + Duration::from_millis(700)));
    }
}
