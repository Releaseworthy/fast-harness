//! Settings limits (PLAN.md D80): the UI scale in Settings > Appearance and
//! the subagent limits (`subagent.maxParallel`, `subagent.maxDepth`).
//!
//! UI scale: the window gets `WindowEvent::ScaleFactorChanged` (Slint 1.18.1
//! `api.rs` sets the window's scale factor from it) and then `Resized` to
//! its unchanged physical size in the new logical pixels, so the window
//! keeps its size and the content grows or shrinks. winit's own
//! `ScaleFactorChanged` (another monitor, another Windows scale) never
//! reaches Slint while `SLINT_SCALE_FACTOR` is set (`display`, D75), and
//! `window_events` keeps the physical size then too: the Windows scale stays
//! ignored. A drag previews; the value is saved when it is final.

use fast_harness_core::config::{save_section_setting, save_setting};
use slint::platform::WindowEvent;
use slint::ComponentHandle;

use crate::app::{show_error, with_app, App};
use crate::{LimitsUi, MainWindow};

/// The UI scale range in percent (`display` clamps `uiScale` to 0.5..=4).
pub const SCALE_PERCENT: (i32, i32) = (50, 400);
/// `subagent.maxParallel` in Settings.
pub const MAX_PARALLEL: (i32, i32) = (1, 16);
/// `subagent.maxDepth` in Settings (0 turns subagents off).
pub const MAX_DEPTH: (i32, i32) = (0, 5);

fn clamp(v: i32, range: (i32, i32)) -> i32 {
    v.clamp(range.0, range.1)
}

/// The scale factor of a percent, clamped to the range.
pub fn factor(percent: i32) -> f32 {
    clamp(percent, SCALE_PERCENT) as f32 / 100.0
}

/// The percent of a scale factor, clamped to the range.
pub fn percent(factor: f32) -> i32 {
    let p = if factor.is_finite() { (factor * 100.0).round() } else { 100.0 };
    clamp(p as i32, SCALE_PERCENT)
}

/// Renders `window` at `factor` at once, keeping its physical size.
pub fn apply_scale(window: &slint::Window, factor: f32) {
    if (window.scale_factor() - factor).abs() < f32::EPSILON {
        return;
    }
    let size = window.size();
    window.dispatch_event(WindowEvent::ScaleFactorChanged {
        scale_factor: factor,
    });
    window.dispatch_event(WindowEvent::Resized {
        size: size.to_logical(factor),
    });
    window.request_redraw();
}

/// Fills the controls from the window and the configuration.
pub fn show(app: &App, ui: &MainWindow) {
    let g = ui.global::<LimitsUi>();
    g.set_scale(percent(ui.window().scale_factor()));
    let sub = &app.rt.config.subagent;
    let parallel = i32::try_from(sub.max_parallel).unwrap_or(i32::MAX);
    g.set_max_parallel(clamp(parallel, MAX_PARALLEL));
    let depth = i32::try_from(sub.max_depth).unwrap_or(i32::MAX);
    g.set_max_depth(clamp(depth, MAX_DEPTH));
}

fn save_scale(app: &App, ui: &MainWindow, percent: i32) {
    let value = serde_json::json!(f64::from(clamp(percent, SCALE_PERCENT)) / 100.0);
    if let Err(err) = save_setting(&app.rt.home, "uiScale", value) {
        show_error(ui, format!("Cannot save the UI scale: {err}"));
    }
}

/// Sets one subagent limit for the next turn and saves it.
fn set_subagent(app: &mut App, ui: &MainWindow, key: &str, value: i32) {
    let sub = &mut app.rt.config.subagent;
    let value = match key {
        "maxParallel" => {
            let v = clamp(value, MAX_PARALLEL);
            sub.max_parallel = usize::try_from(v).unwrap_or(1);
            v
        }
        _ => {
            let v = clamp(value, MAX_DEPTH);
            sub.max_depth = u32::try_from(v).unwrap_or(0);
            v
        }
    };
    let saved = save_section_setting(&app.rt.home, "subagent", key, serde_json::json!(value));
    if let Err(err) = saved {
        show_error(ui, format!("Cannot save subagent.{key}: {err}"));
    }
}

pub fn connect(ui: &MainWindow) {
    let g = ui.global::<LimitsUi>();
    let weak = ui.as_weak();
    g.on_scale_preview(move |p| {
        if let Some(ui) = weak.upgrade() {
            apply_scale(ui.window(), factor(p));
        }
    });
    g.on_scale_saved(|p| {
        with_app(|app, ui| {
            apply_scale(ui.window(), factor(p));
            save_scale(app, ui, p);
        });
    });
    g.on_max_parallel_changed(|v| {
        with_app(|app, ui| set_subagent(app, ui, "maxParallel", v));
    });
    g.on_max_depth_changed(|v| {
        with_app(|app, ui| set_subagent(app, ui, "maxDepth", v));
    });
}

#[cfg(test)]
#[path = "limits_tests.rs"]
mod tests;
