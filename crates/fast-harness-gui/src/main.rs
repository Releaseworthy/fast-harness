//! fast-harness-gui: the fast-harness desktop window (Slint: Skia on Direct3D 12, or the software renderer).
//!
//! `fast-harness-gui [working directory]`: sessions and tools use that directory, the
//! current one when it is not given. `fast-harness-gui --export-ui <dir>` writes
//! the built-in `ui/*.slint` files (PLAN.md D51).

#![windows_subsystem = "windows"]

slint::include_modules!();

include!("root_items.rs");

#[cfg(test)]
mod appearance_tests;
#[cfg(test)]
mod approval_focus_tests;
#[cfg(test)]
mod child_session_tests;
#[cfg(test)]
mod composer_tests;
#[cfg(test)]
mod drafts_tests;
#[cfg(test)]
mod dropdown_tests;
#[cfg(test)]
mod jobs_layout_tests;
#[cfg(test)]
mod layout_tests;
#[cfg(test)]
mod loading_tests;
#[cfg(test)]
mod open_tests;
#[cfg(test)]
mod panels_tests;
#[cfg(test)]
mod popup_view_tests;
#[cfg(test)]
mod prompt_settings_tests;
#[cfg(test)]
mod rails_view_tests;
#[cfg(test)]
mod scroll_tests;
#[cfg(test)]
mod shots_appearance;
#[cfg(test)]
mod shots_approval;
#[cfg(test)]
mod shots_jobs;
#[cfg(test)]
mod shots_png;
#[cfg(test)]
mod shots_popup;
#[cfg(test)]
mod shots_prompt;
#[cfg(test)]
mod shots_rails;
#[cfg(test)]
mod shots_tabbar;
#[cfg(test)]
mod shots_tests;
#[cfg(test)]
mod shots_tools;
#[cfg(test)]
mod shots_trace;
#[cfg(test)]
mod shots_turns;
#[cfg(test)]
mod side_tests;
mod step_rows;
#[cfg(test)]
mod tabbar_tests;
#[cfg(test)]
mod turn_nav_view_tests;
#[cfg(test)]
mod workspace_switch_tests;

/// This binary has its UI built in: Reload (keys.json `reload_ui`) only says
/// which binary reloads UI files.
fn ui_files_connect(ui: &MainWindow) {
    let weak = ui.as_weak();
    ui.on_reload_ui(move || {
        if let Some(ui) = weak.upgrade() {
            let note = "UI files load in fast-harness-gui-live (docs/ui-contract.md).";
            ui.set_error_text(note.into());
        }
    });
}

fn main() {
    if let Some(code) = ui_files::from_args() {
        std::process::exit(code);
    }
    select_backend();
    let Ok(ui) = MainWindow::new() else {
        return;
    };
    run(ui, None);
}
