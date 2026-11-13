//! Settings: the Appearance section (`appearance.rs`), model and effort for the next turn (kept for
//! this window only). The composer and the Settings page bind the same
//! window properties, so a pick in one shows in the other.

use slint::{Model, ModelRc, SharedString, VecModel};

use crate::app::App;
use crate::pickers::{effort_index, effort_list, is_header, model_index, model_list, refit};
use crate::MainWindow;

/// `items` into `current` in place when it is a `VecModel` (an open list
/// then updates without closing), else a new model.
fn strings<'a>(
    current: ModelRc<SharedString>,
    items: impl Iterator<Item = &'a str>,
) -> Option<ModelRc<SharedString>> {
    replace(current, items.map(SharedString::from).collect())
}

fn replace<T: Clone + 'static>(current: ModelRc<T>, items: Vec<T>) -> Option<ModelRc<T>> {
    match current.as_any().downcast_ref::<VecModel<T>>() {
        Some(model) => {
            model.set_vec(items);
            None
        }
        None => Some(ModelRc::new(VecModel::from(items))),
    }
}

/// Fills the pickers from the cached list (the last fetch, else the table).
pub fn fill(app: &mut App, ui: &MainWindow) {
    crate::appearance::show(app, ui);
    crate::config_page::show(app, ui);
    let list = model_list(&app.rt.catalog, &app.rt.models().cached(), &app.rt.config.model);
    show_models(app, ui, list);
}

/// Loads the settings of the workspace `app.cwd` (its project layer when
/// trusted and its provider files, D67) and refills the pickers. A model
/// picked in this window stays when the new list still has it.
pub fn reload_project(app: &mut App, ui: &MainWindow) {
    let (model, effort) = (app.rt.config.model.clone(), app.rt.config.effort.clone());
    let cwd = app.cwd.clone();
    if let Err(err) = app.rt.use_project(&cwd) {
        ui.set_error_text(format!("Project settings: {err}").into());
    }
    let list = model_list(&app.rt.catalog, &app.rt.models().cached(), &app.rt.config.model);
    if list.iter().any(|(id, _)| *id == model) {
        let (effort, _) = refit(&app.rt.catalog, &model, effort.as_deref());
        app.rt.config.model = model;
        app.rt.config.effort = effort;
    }
    fill(app, ui);
}

/// Shows `(id, label)` rows (provider titles marked as headers) with the
/// current model selected, then its efforts.
pub fn show_models(app: &mut App, ui: &MainWindow, list: Vec<(String, String)>) {
    let headers: Vec<bool> = list.iter().map(is_header).collect();
    if let Some(model) = replace(ui.get_model_headers(), headers) {
        ui.set_model_headers(model);
    }
    let labels = list.iter().map(|(_, label)| label.as_str());
    if let Some(model) = strings(ui.get_models(), labels) {
        ui.set_models(model);
    }
    ui.set_model_index(model_index(&list, &app.rt.config.model));
    app.models = list.into_iter().map(|(id, _)| id).collect();
    fill_efforts(app, ui);
}

/// The effort list of the current model and the current effort in it.
fn fill_efforts(app: &App, ui: &MainWindow) {
    let list = effort_list(&app.rt.catalog, &app.rt.config.model);
    if let Some(model) = strings(ui.get_efforts(), list.iter().map(String::as_str)) {
        ui.set_efforts(model);
    }
    ui.set_effort_index(effort_index(&list, app.rt.config.effort.as_deref()));
}

/// A model picked in the composer or on the Settings page: the effort is
/// fitted to it and the user told when it changed. Applies to the next turn.
pub fn pick_model(app: &mut App, ui: &MainWindow, index: i32) {
    let picked = usize::try_from(index).ok().and_then(|i| app.models.get(i));
    // Title rows have an empty id and are never picked.
    let Some(id) = picked.filter(|id| !id.is_empty()).cloned() else {
        return;
    };
    let (effort, note) = refit(&app.rt.catalog, &id, app.rt.config.effort.as_deref());
    app.rt.config.model = id;
    app.rt.config.effort = effort;
    ui.set_model_index(index);
    fill_efforts(app, ui);
    ui.set_picker_note(note.into());
    app.refresh_status(ui);
}

pub fn pick_effort(app: &mut App, ui: &MainWindow, index: i32) {
    let list = effort_list(&app.rt.catalog, &app.rt.config.model);
    if let Some(level) = usize::try_from(index).ok().and_then(|i| list.get(i)) {
        app.rt.config.set_effort(level);
    }
    ui.set_effort_index(effort_index(&list, app.rt.config.effort.as_deref()));
    ui.set_picker_note("".into());
    app.refresh_status(ui);
}
