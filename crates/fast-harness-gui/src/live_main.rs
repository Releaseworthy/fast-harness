//! fast-harness-gui-live: the fast-harness desktop window built at run time from
//! `~/.fast-harness/ui` (cargo feature `ui-files`, PLAN.md D51,
//! `docs/ui-contract.md`). Same arguments as `fast-harness-gui`.

#![windows_subsystem = "windows"]

// The live-preview `MainWindow` (build/live.rs), with the same API as the
// ahead-of-time one.
include!(concat!(env!("OUT_DIR"), "/live_ui.rs"));

include!("root_items.rs");

mod live;
mod live_check;
#[cfg(test)]
mod live_tests;

/// Settings shows the UI files section; Reload checks and loads the folder.
fn ui_files_connect(ui: &MainWindow) {
    live::connect(ui);
}

fn main() {
    if let Some(code) = ui_files::from_args() {
        std::process::exit(code);
    }
    select_backend();
    let home = fast_harness_core::platform::app_home();
    let note = live::prepare(home.as_deref());
    match crate::chat_list::new_window() {
        Ok(ui) => run(ui, note),
        Err(err) => {
            // The checked folder failed to build a window: the built-in UI.
            let root = live::stage_root("live");
            if live::stage(&root, None, true).is_ok() {
                if let Ok(ui) = crate::chat_list::new_window() {
                    run(ui, Some(format!("The built-in UI is in use: {err}")));
                }
            }
        }
    }
    live::cleanup();
}
