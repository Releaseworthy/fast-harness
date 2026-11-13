//! Minimize, maximize or restore and the title-bar double click go through
//! one Rust path that reads the real window state; the `win-maximized`
//! property the restore glyph and the window edge follow is set from that
//! state, and every resize re-reads it (`window_events`) so Win+Up, Aero
//! snap or a restore from the taskbar keep it in sync. A 200 ms polling timer
//! did that before and woke the idle window about 150 times per 30 s.

use std::cell::Cell;
use std::time::{Duration, Instant};

use slint::ComponentHandle;

use crate::MainWindow;

/// Two clicks on the empty title bar within this time maximize or restore.
const DOUBLE_CLICK: Duration = Duration::from_millis(500);

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
    sync(ui);
}

/// Maximizes or restores from the real state, not from a property.
fn toggle(ui: &MainWindow) {
    let maximize = !is_maximized(ui);
    ui.window().set_maximized(maximize);
    ui.set_win_maximized(maximize);
}

/// The live state of the native window (winit asks the OS). Slint's own
/// `Window::is_maximized` lags until Slint has handled the resize, and the
/// resize hook runs before that: it showed the glyph of the state before.
fn is_maximized(ui: &MainWindow) -> bool {
    use slint::winit_030::WinitWindowAccessor;
    let window = ui.window();
    (window.with_winit_window(|native| native.is_maximized()))
        .unwrap_or_else(|| window.is_maximized())
}

/// Sets `win-maximized` from the real window state when it differs.
pub fn sync(ui: &MainWindow) {
    let maximized = is_maximized(ui);
    if ui.get_win_maximized() != maximized {
        ui.set_win_maximized(maximized);
    }
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
