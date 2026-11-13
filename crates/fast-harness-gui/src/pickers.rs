//! The model and effort choices shared by the composer and the Settings
//! page: the rows come from `app::pickers` (shared with the TUI, D59) in
//! the window's words; this file maps them to the dropdowns' indexes.

use fast_harness_core::app::pickers::{self, Row};
use fast_harness_core::app::words::Voice;
use fast_harness_core::llm::anthropic::ModelInfo;
use fast_harness_core::llm::Catalog;

pub use fast_harness_core::app::pickers::{is_header, refit};

/// Model ids and labels of `models` (the cached or fetched list) in the
/// window's words (`app::pickers::model_list`).
pub fn model_list(catalog: &Catalog, models: &[ModelInfo], configured: &str) -> Vec<Row> {
    pickers::model_list(catalog, models, configured, Voice::Window)
}

/// Index of `id` in `list`; -1 (nothing shown) when it is missing.
pub fn model_index(list: &[(String, String)], id: &str) -> i32 {
    if id.is_empty() {
        return -1;
    }
    position(list.iter().map(|(m, _)| m.as_str()), id)
}

/// The effort levels of `model`, as the dropdown lists them: the window
/// reads the remembered live list (`llm::efforts`), not a catalog.
pub fn effort_list(catalog: &Catalog, model: &str) -> Vec<String> {
    pickers::effort_list(catalog, &[], model)
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


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_follow_the_shared_rows() {
        let models = fast_harness_core::llm::anthropic::builtin_models();
        let list = model_list(&Catalog::default(), &models, "claude-new");
        assert_eq!(list[1], ("claude-new".into(), "claude-new".into()));
        assert_eq!(model_index(&list, "claude-new"), 1);
        assert_eq!(model_index(&list, "gone"), -1);
        assert_eq!(model_index(&list, ""), -1);
        let list = effort_list(&Catalog::default(), "claude-opus-5-5");
        assert_eq!(effort_index(&list, None), -1);
        assert_eq!(effort_index(&list, Some("high")), 2);
        assert_eq!(effort_index(&list, Some("off")), -1);
    }
}
