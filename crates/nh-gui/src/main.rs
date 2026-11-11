//! nh-gui: the native-harness desktop window (Slint, software renderer).
//!
//! `nh-gui [working directory]`: sessions and tools use that directory, the
//! current one when it is not given.

#![windows_subsystem = "windows"]

mod accounts;
mod app;
mod format;
mod mirror;
mod model_refresh;
mod pickers;
mod rows;
mod settings;
#[cfg(test)]
mod shots_png;
#[cfg(test)]
mod shots_tests;
#[cfg(test)]
mod shots_trace;
mod theme;
mod trace_rows;
mod turn;
mod window_state;
mod wire;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use nh_core::app::Runtime;

slint::include_modules!();

/// The software renderer on winit unless `SLINT_BACKEND` picks another one.
fn select_backend() {
    if std::env::var_os("SLINT_BACKEND").is_some() {
        return;
    }
    let _ = slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .select();
}

fn working_dir() -> PathBuf {
    let arg = std::env::args_os().nth(1).map(PathBuf::from);
    arg.filter(|p| p.is_dir())
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn main() {
    select_backend();
    let Ok(ui) = MainWindow::new() else {
        return;
    };
    ui.set_version(env!("CARGO_PKG_VERSION").into());
    let stop = Arc::new(AtomicBool::new(false));
    match Runtime::load() {
        Ok(rt) => {
            theme::apply(&ui, nh_core::theme::by_id(&rt.config.theme));
            let (pool, http) = (rt.pool.clone(), rt.transport.clone());
            app::install(app::App::new(&ui, rt, working_dir()));
            wire::connect(&ui);
            wire::first_paint();
            accounts::start_poller(pool, http, stop.clone());
        }
        Err(err) => {
            theme::apply(&ui, nh_core::theme::by_id(nh_core::theme::DEFAULT_THEME));
            ui.set_error_text(format!("Cannot load the settings: {err}").into());
            wire::connect_minimal(&ui);
        }
    }
    if ui.show().is_ok() {
        // The window exists once the event loop runs: then give it the big
        // (taskbar, Alt-Tab) icon; winit only sets the small one.
        slint::Timer::single_shot(std::time::Duration::from_millis(50), || {
            let _ = nh_core::platform::set_big_window_icon();
        });
        let _ = slint::run_event_loop();
        let _ = ui.hide();
    }
    stop.store(true, Ordering::SeqCst);
}
