//! How the window renders (PLAN.md D75), set before the window exists: the
//! renderer and the UI scale, from `renderer` and `uiScale` in the user
//! layer (`<home>/config.json`).
//!
//! Scale: the window renders at `uiScale` (1.0 by default) whatever the
//! Windows display scale is. The process stays per-monitor DPI aware (winit),
//! so Windows never stretches a bitmap; Slint reads `SLINT_SCALE_FACTOR` when
//! it creates the window and then ignores winit's `ScaleFactorChanged`, and
//! `window_events` keeps the physical size when the window moves to a monitor
//! with another scale. A `SLINT_SCALE_FACTOR` already set wins.
//!
//! Renderer (D78): `auto` (default) is Slint's winit default, Skia: on
//! Windows Direct3D 12 through wgpu (AutoVsync, so FIFO), else Skia's own
//! CPU raster surface, and the software renderer only when the Skia renderer
//! itself cannot start. `skia` (alias `d3d`, `directx`) and `software` force
//! one; any other value reads as `auto`. The software renderer blits partial
//! frames with GDI and no vsync: scrolling and resizing tore (D75).
//! `SLINT_BACKEND` overrides the setting. Text is Slint's grayscale
//! anti-aliasing on both renderers (no subpixel text, owner 2026-11-14).

use std::path::Path;

/// The UI scale when the settings give none.
pub const DEFAULT_SCALE: f32 = 1.0;
const SCALE_RANGE: (f32, f32) = (0.5, 4.0);

/// What `select` applies.
#[derive(Debug, PartialEq)]
pub struct Plan {
    /// A winit renderer name, or `None` for Slint's default (Skia on
    /// Direct3D 12, then Skia on the CPU, then software).
    pub renderer: Option<&'static str>,
    /// The `SLINT_SCALE_FACTOR` to set, or `None` to leave the one set.
    pub scale: Option<f32>,
    /// False when `SLINT_BACKEND` picks the backend.
    pub select: bool,
}

/// The plan for the user settings `config` (the parsed `config.json`, or
/// null) and the environment lookup `env`.
pub fn plan(config: &serde_json::Value, env: &dyn Fn(&str) -> Option<String>) -> Plan {
    let renderer = match config.get("renderer").and_then(|v| v.as_str()) {
        Some("software") | Some("sw") => Some("software"),
        Some("skia") | Some("d3d") | Some("directx") => Some("skia"),
        _ => None,
    };
    let preset = env("SLINT_SCALE_FACTOR")
        .and_then(|v| v.trim().parse::<f32>().ok())
        .is_some_and(|f| f > 0.0);
    let wanted = config.get("uiScale").and_then(serde_json::Value::as_f64);
    let scale = wanted
        .map(|f| f as f32)
        .filter(|f| f.is_finite())
        .map_or(DEFAULT_SCALE, |f| f.clamp(SCALE_RANGE.0, SCALE_RANGE.1));
    Plan { renderer, scale: (!preset).then_some(scale), select: env("SLINT_BACKEND").is_none() }
}
/// The user layer settings, or null when they are missing or unreadable
/// (the runtime reports a broken file later).
fn user_settings(home: Option<&Path>) -> serde_json::Value {
    let bytes = home.and_then(|h| fast_harness_core::store::read(h.join("config.json")).ok());
    bytes
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(serde_json::Value::Null)
}

/// Applies the plan. Call first in `main`, before any other thread starts
/// (it sets an environment variable) and before the window is created.
pub fn select() {
    let home = fast_harness_core::platform::app_home();
    let plan = plan(&user_settings(home.as_deref()), &|k| std::env::var(k).ok());

    if let Some(scale) = plan.scale {
        std::env::set_var("SLINT_SCALE_FACTOR", scale.to_string());
    }
    if !plan.select {
        return;
    }
    let selector = slint::BackendSelector::new().backend_name("winit".into());
    let selector = match plan.renderer {
        Some(name) => selector.renderer_name(name.into()),
        None => selector,
    };
    let _ = selector.select();
}

#[cfg(test)]
#[path = "display_tests.rs"]
mod tests;
