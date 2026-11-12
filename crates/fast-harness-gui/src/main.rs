//! fast-harness-gui: the fast-harness desktop window (Slint, software renderer).
//!
//! `fast-harness-gui [working directory]`: sessions and tools use that directory, the
//! current one when it is not given.

#![windows_subsystem = "windows"]

mod accounts;
mod app;
mod appearance;
#[cfg(test)]
mod appearance_tests;
mod config_page;
mod format;
mod google;
#[cfg(test)]
mod google_fixtures;
mod google_rows;
mod keys;
#[cfg(test)]
mod layout_tests;
mod mirror;
mod model_refresh;
mod panels;
#[cfg(test)]
mod panels_tests;
mod pickers;
mod prompt_settings;
#[cfg(test)]
mod prompt_settings_tests;
mod providers;
mod reflect;
mod rows;
mod runs;
#[cfg(test)]
mod scroll_tests;
mod settings;
#[cfg(test)]
mod shots_appearance;
#[cfg(test)]
mod shots_png;
#[cfg(test)]
mod shots_prompt;
#[cfg(test)]
mod shots_tests;
#[cfg(test)]
mod shots_tools;
#[cfg(test)]
mod shots_trace;
mod theme;
mod tool_rows;
mod trace_rows;
mod turn;
mod window_state;
mod wire;
mod workspaces;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use fast_harness_core::app::Runtime;

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

/// The folder given on the command line, if it exists.
fn working_dir() -> Option<PathBuf> {
    let arg = std::env::args_os().nth(1).map(PathBuf::from);
    arg.filter(|p| p.is_dir())
}

fn main() {
    select_backend();
    let Ok(ui) = MainWindow::new() else {
        return;
    };
    ui.set_version(env!("CARGO_PKG_VERSION").into());
    let stop = Arc::new(AtomicBool::new(false));
    match Runtime::load() {
        Ok(mut rt) => {
            rt.interactive = true;
            let cwd = working_dir();
            // The project layer of the start folder (PLAN.md D41); its
            // untrusted note comes with the first agent.
            let project = cwd.as_deref().map_or(Ok(()), |cwd| rt.use_project(cwd));
            let notes = theme::load(&rt.home);
            theme::apply_config(&ui, &rt.config);
            if let Some(note) = notes.first() {
                ui.set_error_text(format!("Themes: {note}").into());
            }
            if let Err(err) = project {
                ui.set_error_text(format!("Project settings: {err}").into());
            }
            let (pool, google, http) = (rt.pool.clone(), rt.google.clone(), rt.transport.clone());
            app::install(app::App::new(&ui, rt, cwd));
            wire::connect(&ui);
            wire::first_paint();
            accounts::start_poller(pool, google, http, stop.clone());
        }
        Err(err) => {
            theme::apply(
                &ui,
                fast_harness_core::theme::by_id(fast_harness_core::theme::DEFAULT_THEME),
                0,
            );
            ui.set_error_text(format!("Cannot load the settings: {err}").into());
            wire::connect_minimal(&ui);
        }
    }
    if ui.show().is_ok() {
        // The window exists once the event loop runs: then give it the big
        // (taskbar, Alt-Tab) icon; winit only sets the small one.
        slint::Timer::single_shot(std::time::Duration::from_millis(50), || {
            let _ = fast_harness_core::platform::set_big_window_icon();
        });
        let _ = slint::run_event_loop();
        // Every panel's process tree ends with the window (PLAN.md D46).
        app::with_app(|app, _| app.panels.stop_all());
        let _ = ui.hide();
    }
    stop.store(true, Ordering::SeqCst);
}
