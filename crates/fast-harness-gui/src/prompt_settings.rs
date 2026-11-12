//! Settings > System prompt (PLAN.md D31): the global template or the
//! selected workspace's override (`app.cwd`), saved through
//! `fast_harness_core::agent::prompt_store`, with a preview of the rendered
//! prompt. The section's state is the `PromptUi` global.

use std::path::Path;

use fast_harness_core::agent::prompt::{self, DEFAULT_TEMPLATE};
use fast_harness_core::agent::prompt_store::{self as store, Scope};
use slint::ComponentHandle;

use crate::app::{with_app, App};
use crate::{MainWindow, PromptUi};

/// The placeholder line under the editor.
pub const PLACEHOLDERS: &str = "Placeholders: {cwd} working folder, {os} operating system, \
{date} today (UTC), {model} model id. Any other text is sent as written.";

/// What the section shows for one scope, without a window.
#[derive(Debug, PartialEq)]
pub struct View {
    /// The text the editor starts with (the saved text).
    pub text: String,
    pub override_on: bool,
    /// Which file applies to the workspace and where Save writes.
    pub source: String,
    /// A file that could not be read, else "".
    pub status: String,
}

/// The saved text of `scope`, else `fallback`; an unreadable file adds to `status`.
fn saved_or(
    home: &Path,
    cwd: &Path,
    scope: Scope,
    fallback: String,
    status: &mut Vec<String>,
) -> (String, bool) {
    match store::load(home, cwd, scope) {
        Ok(Some(text)) => (text, true),
        Ok(None) => (fallback, false),
        Err(err) => {
            let file = store::path(home, cwd, scope);
            status.push(format!("Cannot read {}: {err}", file.display()));
            (fallback, false)
        }
    }
}

/// The view of `scope` for the workspace `cwd`.
pub fn view(home: &Path, cwd: &Path, scope: Scope) -> View {
    let mut status = Vec::new();
    let (global, _) = saved_or(
        home,
        cwd,
        Scope::Global,
        DEFAULT_TEMPLATE.into(),
        &mut status,
    );
    let file = store::path(home, cwd, scope);
    let (text, override_on, writes) = match scope {
        Scope::Global => (global, false, "Save writes"),
        Scope::Workspace => {
            let (text, own) = saved_or(home, cwd, scope, global, &mut status);
            let writes = if own {
                "Save writes"
            } else {
                "Override writes"
            };
            (text, own, writes)
        }
    };
    let used = store::template(home, cwd).source;
    View {
        text,
        override_on,
        source: format!(
            "Used in this workspace: {used}. {writes} {}.",
            file.display()
        ),
        status: status.join(" "),
    }
}

fn scope_of(index: i32) -> Scope {
    if index == 1 {
        Scope::Workspace
    } else {
        Scope::Global
    }
}

fn workspace_name(cwd: &Path) -> String {
    cwd.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| cwd.display().to_string())
}

/// Reloads the section from disk; `status` replaces the line under the buttons.
pub fn show(app: &App, ui: &MainWindow, status: &str) {
    let state = ui.global::<PromptUi>();
    let scope = scope_of(state.get_scope());
    let view = view(&app.rt.home, &app.cwd, scope);
    state.set_workspace(workspace_name(&app.cwd).into());
    state.set_override_on(view.override_on);
    state.set_text(view.text.as_str().into());
    state.set_saved(view.text.as_str().into());
    state.set_source(view.source.into());
    let status = [status, view.status.as_str()].join(" ");
    state.set_status(status.trim().into());
    state.set_placeholders(PLACEHOLDERS.into());
    show_preview(app, ui);
}

/// Fills the preview from the editor's text (saved or not) when it is shown.
fn show_preview(app: &App, ui: &MainWindow) {
    let state = ui.global::<PromptUi>();
    if !state.get_show_preview() {
        return;
    }
    let (home, model) = (&app.rt.home, &app.rt.config.model);
    let text = prompt::preview(home, &state.get_text(), &app.cwd, model);
    state.set_preview(text.into());
}

/// The outcome of a file action in words.
fn outcome(done: Result<(), String>, ok: &str) -> String {
    match done {
        Ok(()) => ok.to_string(),
        Err(err) => err,
    }
}

fn save(app: &App, ui: &MainWindow) {
    let state = ui.global::<PromptUi>();
    let scope = scope_of(state.get_scope());
    let done = store::save(&app.rt.home, &app.cwd, scope, &state.get_text());
    show(
        app,
        ui,
        &outcome(
            done,
            "Saved. Applies to new sessions; running sessions get a note.",
        ),
    );
}

fn reset(app: &App, ui: &MainWindow) {
    let scope = scope_of(ui.global::<PromptUi>().get_scope());
    let done = store::reset(&app.rt.home, &app.cwd, scope);
    let ok = match scope {
        Scope::Global => "Reset: the built-in default below applies.",
        Scope::Workspace => "Reset: this workspace uses the global prompt.",
    };
    show(app, ui, &outcome(done, ok));
}

/// On: the workspace gets its own file, starting from the global text; off deletes it.
fn toggle_override(app: &App, ui: &MainWindow, on: bool) {
    let (home, cwd) = (&app.rt.home, &app.cwd);
    let done = if on {
        let start = view(home, cwd, Scope::Global).text;
        store::save(home, cwd, Scope::Workspace, &start)
    } else {
        store::reset(home, cwd, Scope::Workspace)
    };
    let ok = if on {
        "Override on: edit this workspace's prompt, starting from the global one."
    } else {
        "Override off: this workspace uses the global prompt."
    };
    show(app, ui, &outcome(done, ok));
}

fn pick_scope(app: &App, ui: &MainWindow, index: i32) {
    let state = ui.global::<PromptUi>();
    let dropped = if state.get_dirty() {
        "Unsaved changes were dropped."
    } else {
        ""
    };
    state.set_scope(index);
    show(app, ui, dropped);
}

pub fn connect(ui: &MainWindow) {
    let state = ui.global::<PromptUi>();
    state.on_scope_picked(|index| {
        with_app(|app, ui| pick_scope(app, ui, index));
    });
    state.on_override_toggled(|on| {
        with_app(|app, ui| toggle_override(app, ui, on));
    });
    state.on_edited(|| {
        with_app(|app, ui| show_preview(app, ui));
    });
    state.on_save(|| {
        with_app(|app, ui| save(app, ui));
    });
    state.on_reset(|| {
        with_app(|app, ui| reset(app, ui));
    });
    state.on_preview_toggled(|on| {
        with_app(|app, ui| {
            ui.global::<PromptUi>().set_show_preview(on);
            show_preview(app, ui);
        });
    });
}
