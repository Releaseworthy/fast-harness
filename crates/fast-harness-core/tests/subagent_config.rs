//! `subagent.maxParallel` and `subagent.maxDepth` (PLAN.md D40, D80):
//! defaults, parsing and the Settings write of one key of the section.

use fast_harness_core::config::{save_section_setting, Config, SubagentConfig};

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let dir =
            std::env::temp_dir().join(format!("fh-subagent-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn defaults_are_four_children_and_one_level() {
    let config = SubagentConfig::default();
    assert_eq!(config.max_parallel, 4);
    assert_eq!(config.max_depth, 1);
    assert_eq!(Config::default().subagent, config);
}

#[test]
fn the_section_parses_with_missing_keys_defaulted() {
    let config: SubagentConfig = serde_json::from_str(r#"{"maxDepth": 0}"#).unwrap();
    assert_eq!((config.max_depth, config.max_parallel), (0, 4));
    let config: SubagentConfig = serde_json::from_str(r#"{"maxParallel": 16}"#).unwrap();
    assert_eq!((config.max_depth, config.max_parallel), (1, 16));
    assert!(serde_json::from_str::<SubagentConfig>(r#"{"maxDepth": -1}"#).is_err());
}

#[test]
fn saving_one_key_keeps_the_rest_of_the_section_and_file() {
    let dir = TempDir::new("save");
    let file = dir.0.join("config.json");
    std::fs::write(
        &file,
        r#"{"theme": "dark", "subagent": {"model": "m", "maxDepth": 2}}"#,
    )
    .unwrap();
    save_section_setting(&dir.0, "subagent", "maxDepth", 3.into()).unwrap();
    save_section_setting(&dir.0, "subagent", "maxParallel", 8.into()).unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(saved["theme"], "dark");
    assert_eq!(saved["subagent"]["model"], "m");
    assert_eq!(saved["subagent"]["maxDepth"], 3);
    assert_eq!(saved["subagent"]["maxParallel"], 8);
    // A section that is not an object starts empty; a missing file is made.
    let fresh = TempDir::new("fresh");
    save_section_setting(&fresh.0, "subagent", "maxDepth", 0.into()).unwrap();
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fresh.0.join("config.json")).unwrap()).unwrap();
    assert_eq!(saved["subagent"]["maxDepth"], 0);
}
