//! The model and effort choices shared by the composer and the Settings
//! page: one list in Rust, both views bind the same window properties.

use fast_harness_core::llm::anthropic::{efforts, fit_effort, ModelInfo};

/// Model ids and labels of `models` (the cached or fetched list), with
/// `configured` first (labelled by its id) when the list lacks it, and
/// " (deprecated)" after the label of a deprecated model.
pub fn model_list(models: &[ModelInfo], configured: &str) -> Vec<(String, String)> {
    let table: Vec<(String, String)> = models
        .iter()
        .map(|m| (m.id.clone(), m.display_name.clone()))
        .collect();
    let mut list = labelled(&table, configured);
    for (id, label) in &mut list {
        if models.iter().any(|m| &m.id == id && m.deprecated) {
            label.push_str(" (deprecated)");
        }
    }
    list
}

/// `(id, display name)` pairs to `(id, label)`: the display name, plus the
/// id in parentheses when another model shares the name.
pub fn labelled(table: &[(String, String)], configured: &str) -> Vec<(String, String)> {
    let mut list: Vec<(String, String)> = table
        .iter()
        .map(|(id, name)| {
            let shared = table.iter().filter(|(_, other)| other == name).count() > 1;
            let label = if shared || name.is_empty() {
                format!("{name} ({id})").trim().to_string()
            } else {
                name.clone()
            };
            (id.clone(), label)
        })
        .collect();
    if !configured.is_empty() && !list.iter().any(|(id, _)| id == configured) {
        list.insert(0, (configured.to_string(), configured.to_string()));
    }
    list
}

/// Index of `id` in `list`; -1 (nothing shown) when it is missing.
pub fn model_index(list: &[(String, String)], id: &str) -> i32 {
    position(list.iter().map(|(m, _)| m.as_str()), id)
}

/// The effort levels of `model`, as the dropdown lists them.
pub fn effort_list(model: &str) -> Vec<&'static str> {
    efforts(model)
}

/// Index of `effort` in `list`; -1 for `None` (the dropdown then shows its
/// `default` placeholder) or a level the list lacks.
pub fn effort_index(list: &[&str], effort: Option<&str>) -> i32 {
    match effort {
        Some(level) => position(list.iter().copied(), level),
        None => -1,
    }
}

fn position<'a>(mut items: impl Iterator<Item = &'a str>, wanted: &str) -> i32 {
    items
        .position(|item| item == wanted)
        .and_then(|i| i32::try_from(i).ok())
        .unwrap_or(-1)
}

/// The effort to keep when `model` is picked, and a note for the user when
/// it had to change (empty otherwise).
pub fn refit(model: &str, effort: Option<&str>) -> (Option<String>, String) {
    let fitted = fit_effort(model, effort);
    let note = match (effort, fitted.as_deref()) {
        (Some(old), Some(new)) if old != new => {
            format!("Effort set to {new}: {model} does not take {old}.")
        }
        _ => String::new(),
    };
    (fitted, note)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(id: &str, name: &str) -> (String, String) {
        (id.into(), name.into())
    }

    #[test]
    fn labels_add_the_id_only_to_shared_names() {
        let table = [
            pair("opus-a", "Opus"),
            pair("opus-b", "Opus"),
            pair("sonnet", "Sonnet"),
        ];
        let list = labelled(&table, "sonnet");
        let labels: Vec<&str> = list.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(labels, ["Opus (opus-a)", "Opus (opus-b)", "Sonnet"]);
        let custom = labelled(&table, "my-model");
        assert_eq!(custom[0], pair("my-model", "my-model"));
        assert_eq!(custom.len(), 4);
        assert_eq!(model_index(&custom, "sonnet"), 3);
        assert_eq!(model_index(&custom, "gone"), -1);
        assert_eq!(labelled(&table, "").len(), 3);
    }

    #[test]
    fn builtin_labels_are_unique() {
        let list = model_list(
            &fast_harness_core::llm::anthropic::builtin_models(),
            "claude-opus-5-5",
        );
        assert_eq!(list[0].1, "Claude Opus 5.5");
        let mut labels: Vec<&str> = list.iter().map(|(_, l)| l.as_str()).collect();
        assert!(labels.contains(&"Claude Opus 4.5 (claude-opus-4-5-20251101)"));
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), list.len());
    }

    #[test]
    fn effort_index_and_refit() {
        let list = effort_list("claude-opus-5-5");
        assert_eq!(effort_index(&list, None), -1);
        assert_eq!(effort_index(&list, Some("high")), 2);
        assert_eq!(effort_index(&list, Some("off")), -1);
        let (effort, note) = refit("claude-opus-4-5", Some("max"));
        assert_eq!(effort.as_deref(), Some("high"));
        assert!(note.contains("does not take max"), "{note}");
        assert_eq!(
            refit("claude-opus-4-5", Some("low")),
            (Some("low".into()), String::new())
        );
        assert_eq!(refit("claude-opus-4-5", None), (None, String::new()));
    }
}
