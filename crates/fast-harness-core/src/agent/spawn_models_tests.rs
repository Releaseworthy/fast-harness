use super::*;
use crate::config::{Config, DEFAULT_SUBAGENT_GUIDE};

fn off(ids: &[&str]) -> SubagentConfig {
    let mut settings = SubagentConfig::default();
    for id in ids {
        settings.models.insert((*id).to_string(), false);
    }
    settings
}

#[test]
fn every_known_model_is_enabled_by_default() {
    let catalog = Catalog::default();
    let all = enabled(&catalog, &SubagentConfig::default());
    assert_eq!(all, catalog.model_ids());
    assert!(all.iter().any(|id| id == "claude-haiku-5-5"));
    assert!(
        !all.iter().any(|id| id.ends_with("-20251101")),
        "no dated snapshots"
    );
}

#[test]
fn a_disabled_model_leaves_the_enum_and_is_refused() {
    let catalog = Catalog::default();
    let settings = off(&["claude-opus-5-5"]);
    let list = enabled(&catalog, &settings);
    let schema = model_schema(&list);
    let names: Vec<&str> = schema["enum"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(!names.contains(&"claude-opus-5-5"));
    assert!(names.contains(&"claude-sonnet-5-5"));
    let err = check(&settings, &catalog, Some("claude-opus-5-5"), "x").unwrap_err();
    assert!(err.contains("`claude-opus-5-5` is disabled"), "{err}");
    assert!(err.contains("claude-sonnet-5-5"), "{err}");
    let err = check(&settings, &catalog, None, "claude-opus-5-5").unwrap_err();
    assert!(err.contains("pass `model`"), "{err}");
    assert!(check(
        &settings,
        &catalog,
        Some("claude-haiku-5-5"),
        "claude-opus-5-5"
    )
    .is_ok());
}

#[test]
fn the_guide_drops_lines_of_disabled_families() {
    let ids = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let all = guide(
        DEFAULT_SUBAGENT_GUIDE,
        &ids(&["claude-haiku-5-5", "claude-sonnet-5-5", "claude-opus-5-5"]),
    );
    assert_eq!(all, DEFAULT_SUBAGENT_GUIDE);
    let some = guide(DEFAULT_SUBAGENT_GUIDE, &ids(&["claude-sonnet-5-5"]));
    assert!(!some.contains("Haiku") && !some.contains("Opus"), "{some}");
    assert!(
        some.contains("Sonnet") && some.contains("Pick the"),
        "{some}"
    );
    assert_eq!(guide(DEFAULT_SUBAGENT_GUIDE, &ids(&["local/m"])), "");
    assert_eq!(guide("Prefer small models.", &[]), "Prefer small models.");
}

#[test]
fn the_settings_round_trip_with_defaults() {
    let config: Config = serde_json::from_str("{}").unwrap();
    assert!(config.subagent.models.is_empty());
    assert_eq!(config.subagent.guide, DEFAULT_SUBAGENT_GUIDE);
    let text = r#"{"subagent": {"models": {"claude-opus-5-5": false}, "guide": "G"}}"#;
    let config: Config = serde_json::from_str(text).unwrap();
    assert!(!config.subagent.allows("claude-opus-5-5"));
    assert!(config.subagent.allows("claude-haiku-5-5"));
    assert_eq!(config.subagent.guide, "G");
    assert_eq!(config.subagent.max_depth, 1, "other keys keep defaults");
    let back = serde_json::to_value(&config.subagent).unwrap();
    assert_eq!(back["models"]["claude-opus-5-5"], false);
    assert_eq!(back["guide"], "G");
}
