//! Settings > Subagent models (PLAN.md D82): which catalog models a
//! subagent may run on (`subagent.models`) and the model guide of the
//! `subagent` tool (`subagent.guide`), saved with `save_section_setting`.

use fast_harness_core::config::{save_section_setting, DEFAULT_SUBAGENT_GUIDE};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::app::{with_app, App};
use crate::{MainWindow, SpawnModel, SpawnUi};

/// Fills the check boxes and the guide editor from the configuration.
pub fn show(app: &App, ui: &MainWindow) {
    let g = ui.global::<SpawnUi>();
    g.set_models(ModelRc::new(VecModel::from(rows(app))));
    let guide = app.rt.config.subagent.guide.as_str();
    g.set_guide(guide.into());
    g.set_saved(guide.into());
    g.set_status("".into());
}

/// One row per catalog model, with its display name and on state.
fn rows(app: &App) -> Vec<SpawnModel> {
    let catalog = &app.rt.catalog;
    let sub = &app.rt.config.subagent;
    (catalog.model_ids().into_iter())
        .map(|id| {
            let name = catalog.display_name(&id).unwrap_or_else(|| id.clone());
            SpawnModel {
                label: format!("{name} ({id})").into(),
                on: sub.allows(&id),
                id: id.into(),
            }
        })
        .collect()
}

/// Turns model `index` on or off for the next call and saves the map; only
/// the models turned off are kept, so new models start enabled.
fn toggle(app: &mut App, ui: &MainWindow, index: i32, on: bool) {
    let ids = app.rt.catalog.model_ids();
    let Some(id) = usize::try_from(index).ok().and_then(|i| ids.get(i)) else {
        return;
    };
    let models = &mut app.rt.config.subagent.models;
    match on {
        true => models.remove(id),
        false => models.insert(id.clone(), false),
    };
    let value = serde_json::json!(models);
    let status = match save_section_setting(&app.rt.home, "subagent", "models", value) {
        Ok(()) => String::new(),
        Err(err) => format!("Cannot save subagent.models: {err}"),
    };
    show(app, ui);
    ui.global::<SpawnUi>().set_status(status.into());
}

/// Saves `text` as `subagent.guide` and shows the outcome.
fn save_guide(app: &mut App, ui: &MainWindow, text: &str) {
    app.rt.config.subagent.guide = text.to_string();
    let value = serde_json::json!(text);
    let status = match save_section_setting(&app.rt.home, "subagent", "guide", value) {
        Ok(()) => "Saved: new sessions use this guide.".to_string(),
        Err(err) => format!("Cannot save subagent.guide: {err}"),
    };
    show(app, ui);
    ui.global::<SpawnUi>().set_status(status.into());
}

pub fn connect(ui: &MainWindow) {
    let g = ui.global::<SpawnUi>();
    g.on_toggled(|index, on| {
        with_app(|app, ui| toggle(app, ui, index, on));
    });
    g.on_save(|| {
        with_app(|app, ui| {
            let text = ui.global::<SpawnUi>().get_guide();
            save_guide(app, ui, &text);
        });
    });
    g.on_reset(|| {
        with_app(|app, ui| save_guide(app, ui, DEFAULT_SUBAGENT_GUIDE));
    });
}

#[cfg(test)]
#[path = "spawn_settings_tests.rs"]
mod tests;
