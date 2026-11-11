//! Settings: the theme (saved), model and effort for the next turn (kept for
//! this window only). The composer and the Settings page bind the same
//! window properties, so a pick in one shows in the other.

use slint::{ModelRc, SharedString, VecModel};

use crate::app::App;
use crate::pickers::{effort_index, effort_list, model_index, model_list, refit};
use crate::MainWindow;

fn strings<'a>(items: impl Iterator<Item = &'a str>) -> ModelRc<SharedString> {
    let items: Vec<SharedString> = items.map(SharedString::from).collect();
    ModelRc::new(VecModel::from(items))
}

pub fn fill(app: &mut App, ui: &MainWindow) {
    crate::theme::fill(ui, &app.rt.config.theme);
    let list = model_list(&app.rt.config.model);
    ui.set_models(strings(list.iter().map(|(_, label)| label.as_str())));
    ui.set_model_index(model_index(&list, &app.rt.config.model));
    app.models = list.into_iter().map(|(id, _)| id).collect();
    fill_efforts(app, ui);
}

/// The effort list of the current model and the current effort in it.
fn fill_efforts(app: &App, ui: &MainWindow) {
    let list = effort_list(&app.rt.config.model);
    ui.set_efforts(strings(list.iter().copied()));
    ui.set_effort_index(effort_index(&list, app.rt.config.effort.as_deref()));
}

/// A model picked in the composer or on the Settings page: the effort is
/// fitted to it and the user told when it changed. Applies to the next turn.
pub fn pick_model(app: &mut App, ui: &MainWindow, index: i32) {
    let picked = usize::try_from(index).ok().and_then(|i| app.models.get(i));
    let Some(id) = picked.cloned() else {
        return;
    };
    let (effort, note) = refit(&id, app.rt.config.effort.as_deref());
    app.rt.config.model = id;
    app.rt.config.effort = effort;
    ui.set_model_index(index);
    fill_efforts(app, ui);
    ui.set_picker_note(note.into());
    app.refresh_status(ui);
}

pub fn pick_effort(app: &mut App, ui: &MainWindow, index: i32) {
    let list = effort_list(&app.rt.config.model);
    if let Some(level) = usize::try_from(index).ok().and_then(|i| list.get(i)) {
        app.rt.config.set_effort(level);
    }
    ui.set_effort_index(effort_index(&list, app.rt.config.effort.as_deref()));
    ui.set_picker_note("".into());
    app.refresh_status(ui);
}
