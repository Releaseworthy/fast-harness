//! Settings: the theme (saved), model and effort for the next turn (kept for\n//! this window only).

use nh_core::llm::anthropic::builtin_models;
use slint::{ModelRc, SharedString, VecModel};

use crate::app::App;
use crate::MainWindow;

/// The effort choices; `default` clears the setting.
pub const EFFORTS: [&str; 6] = ["default", "low", "medium", "high", "max", "off"];

/// Model ids and labels; the configured model first when it is not built in.
pub fn model_list(configured: &str) -> Vec<(String, String)> {
    let mut list: Vec<(String, String)> = builtin_models()
        .into_iter()
        .map(|m| (m.id.clone(), format!("{} ({})", m.display_name, m.id)))
        .collect();
    if !configured.is_empty() && !list.iter().any(|(id, _)| id == configured) {
        list.insert(0, (configured.to_string(), configured.to_string()));
    }
    list
}

pub fn effort_index(effort: Option<&str>) -> usize {
    let effort = effort.unwrap_or("default");
    EFFORTS.iter().position(|e| *e == effort).unwrap_or(0)
}

pub fn fill(app: &mut App, ui: &MainWindow) {
    crate::theme::fill(ui, &app.rt.config.theme);
    let list = model_list(&app.rt.config.model);
    let labels: Vec<SharedString> = list.iter().map(|(_, label)| label.into()).collect();
    let index = list
        .iter()
        .position(|(id, _)| *id == app.rt.config.model)
        .unwrap_or(0);
    app.models = list.into_iter().map(|(id, _)| id).collect();
    ui.set_models(ModelRc::new(VecModel::from(labels)));
    let efforts: Vec<SharedString> = EFFORTS.iter().map(|e| (*e).into()).collect();
    ui.set_efforts(ModelRc::new(VecModel::from(efforts)));
    ui.set_model_index(index as i32);
    ui.set_effort_index(effort_index(app.rt.config.effort.as_deref()) as i32);
}

pub fn apply(app: &mut App, ui: &MainWindow, model: i32, effort: i32) {
    let pick = |i: i32| usize::try_from(i).ok();
    if let Some(id) = pick(model).and_then(|i| app.models.get(i)) {
        app.rt.config.model.clone_from(id);
    }
    if let Some(effort) = pick(effort).and_then(|i| EFFORTS.get(i)) {
        app.rt.config.set_effort(effort);
    }
    app.refresh_status(ui);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_and_efforts() {
        let list = model_list("claude-opus-5-5");
        assert!(list.iter().any(|(id, _)| id == "claude-opus-5-5"));
        assert_eq!(model_list("custom-model")[0].0, "custom-model");
        assert_eq!(effort_index(None), 0);
        assert_eq!(effort_index(Some("max")), 4);
        assert_eq!(effort_index(Some("weird")), 0);
    }
}
