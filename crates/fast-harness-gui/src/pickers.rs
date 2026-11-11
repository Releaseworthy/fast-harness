//! The model and effort choices shared by the composer and the Settings
//! page: one list in Rust, both views bind the same window properties.

use fast_harness_core::llm::anthropic::ModelInfo;
use fast_harness_core::llm::catalog::{group_title, provider_of, PROVIDERS};
use fast_harness_core::llm::{efforts, fit_effort};

/// Model ids and labels of `models` (the cached or fetched list) grouped by
/// provider (PLAN.md D24): a title row (empty id, "Claude" or "Google
/// Antigravity") before each non-empty group, `configured` first in its
/// group (labelled by its id) when the list lacks it, and " (deprecated)"
/// after the label of a deprecated model.
pub fn model_list(models: &[ModelInfo], configured: &str) -> Vec<(String, String)> {
    let mut list = Vec::new();
    for provider in PROVIDERS {
        let table: Vec<(String, String)> = models
            .iter()
            .filter(|m| provider_of(&m.id) == provider)
            .map(|m| (m.id.clone(), m.display_name.clone()))
            .collect();
        let mine = if provider_of(configured) == provider {
            configured
        } else {
            ""
        };
        let mut rows = labelled(&table, mine);
        if rows.is_empty() {
            continue;
        }
        for (id, label) in &mut rows {
            if models.iter().any(|m| &m.id == id && m.deprecated) {
                label.push_str(" (deprecated)");
            }
        }
        list.push((String::new(), group_title(provider).to_string()));
        list.extend(rows);
    }
    list
}

/// True for the provider title rows of [`model_list`].
pub fn is_header(row: &(String, String)) -> bool {
    row.0.is_empty()
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
    if id.is_empty() {
        return -1;
    }
    position(list.iter().map(|(m, _)| m.as_str()), id)
}

/// The effort levels of `model`, as the dropdown lists them.
pub fn effort_list(model: &str) -> Vec<String> {
    efforts(model)
}

/// Index of `effort` in `list`; -1 for `None` (the dropdown then shows its
/// `default` placeholder) or a level the list lacks.
pub fn effort_index(list: &[String], effort: Option<&str>) -> i32 {
    match effort {
        Some(level) => position(list.iter().map(String::as_str), level),
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
        assert_eq!(list[0], (String::new(), "Claude".to_string()));
        assert_eq!(list[1].1, "Claude Opus 5.5");
        assert!(list.iter().skip(1).all(|row| !is_header(row)));
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

    fn info(id: &str, name: &str) -> ModelInfo {
        let mut model = fast_harness_core::llm::anthropic::builtin_models().remove(0);
        model.id = id.into();
        model.display_name = name.into();
        model
    }

    #[test]
    fn groups_by_provider_with_title_rows() {
        let models = [
            info("antigravity/gemini-g", "Gemini G"),
            info("claude-c", "Claude C"),
            info("antigravity/claude-c", "Claude C"),
        ];
        let list = model_list(&models, "antigravity/gemini-new");
        let labels: Vec<&str> = list.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Claude",
                "Claude C",
                "Google Antigravity",
                "antigravity/gemini-new",
                "Gemini G",
                "Claude C"
            ]
        );
        assert!(is_header(&list[0]) && is_header(&list[2]));
        assert_eq!(model_index(&list, "antigravity/claude-c"), 5);
        assert_eq!(model_index(&list, ""), -1);
        let only_claude = model_list(&models[1..2], "claude-c");
        assert_eq!(only_claude.len(), 2);
        let effort = effort_list("antigravity/gemini-3-pro-group");
        assert_eq!(effort, ["low", "high"]);
    }
}
