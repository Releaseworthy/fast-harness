// The items both GUI binaries share (PLAN.md D51): `fast-harness-gui` compiles
// the UI ahead of time, `fast-harness-gui-live` (feature `ui-files`) loads it
// from `~/.fast-harness/ui`. Each binary's root includes this file after its
// `MainWindow` and defines `ui_files_connect`.

mod accounts;
mod app;
mod appearance;
mod config_page;
mod format;
mod google;
#[cfg(test)]
mod google_fixtures;
mod google_rows;
mod keys;
mod mirror;
mod model_refresh;
mod panels;
mod pickers;
mod prompt_settings;
mod providers;
mod reflect;
mod rows;
mod runs;
mod settings;
mod theme;
mod tool_rows;
mod trace_rows;
mod turn;
mod window_state;
mod wire;
mod workspaces;
// Read at run time by fast-harness-gui-live only; the default binary tests it.
#[allow(dead_code)]
mod ui_contract;
mod ui_files;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use fast_harness_core::app::Runtime;

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

/// Wires the window and runs the event loop; `note` (the UI files report of
/// the live variant) goes to the banner when nothing else is there.
fn run(ui: MainWindow, note: Option<String>) {
    ui.set_version(env!("CARGO_PKG_VERSION").into());
    let stop = Arc::new(AtomicBool::new(false));
    start_runtime(&ui, &stop);
    ui_files_connect(&ui);
    if let Some(note) = note.filter(|_| ui.get_error_text().is_empty()) {
        ui.set_error_text(note.into());
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

/// Loads the settings and wires the app, or shows why it cannot.
fn start_runtime(ui: &MainWindow, stop: &Arc<AtomicBool>) {
    let ui = ui.clone_strong();
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
}