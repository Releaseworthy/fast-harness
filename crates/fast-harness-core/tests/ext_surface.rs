//! Phase 29: `version` and `enabled` for every extension kind through the
//! shared rules (`config::ext_file`), and `ext list --json`.

use std::path::{Path, PathBuf};

use fast_harness_core::app::{ext_list, load_settings};
use fast_harness_core::config::trust;
use serde_json::Value;

struct Dirs {
    root: PathBuf,
    home: PathBuf,
    project: PathBuf,
}

impl Drop for Dirs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn dirs(name: &str) -> Dirs {
    let root = std::env::temp_dir().join(format!("fh-ext-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (home, project) = (root.join("home"), root.join("proj"));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(project.join(".fast-harness")).unwrap();
    Dirs {
        root,
        home,
        project,
    }
}

fn put(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// One file of every kind under `dir` with `extra` keys (JSON) or front
/// matter lines (Markdown).
fn every_kind(dir: &Path, tag: &str, json: &str, md: &str) {
    let tool = format!(r#"{{"name": "t-{tag}", "command": "x", "description": "d"{json}}}"#);
    put(&dir.join("tools").join(format!("t-{tag}.json")), &tool);
    for kind in ["agents", "commands"] {
        let body = format!("---\ndescription: d\n{md}---\nBody\n");
        put(&dir.join(kind).join(format!("a-{tag}.md")), &body);
    }
    let skill = format!("---\nname: s-{tag}\ndescription: d\n{md}---\nBody\n");
    put(
        &dir.join("skills").join(format!("s-{tag}")).join("SKILL.md"),
        &skill,
    );
    let provider = format!(
        r#"{{"id": "p-{tag}", "api": "openai-chat", "baseUrl": "http://127.0.0.1:9", "models": []{json}}}"#
    );
    put(
        &dir.join("providers").join(format!("p-{tag}.json")),
        &provider,
    );
    let panel = format!(r#"{{"id": "pa-{tag}", "command": "x"{json}}}"#);
    put(&dir.join("panels").join(format!("pa-{tag}.json")), &panel);
}

fn list(d: &Dirs, cwd: &Path) -> Vec<Value> {
    let settings = load_settings(&d.home, Some(cwd), &|_| None).unwrap();
    ext_list(&settings).as_array().unwrap().clone()
}

fn states(items: &[Value], name_part: &str) -> Vec<String> {
    (items.iter())
        .filter(|i| i["path"].as_str().unwrap().contains(name_part))
        .map(|i| {
            format!(
                "{}:{}",
                i["kind"].as_str().unwrap(),
                i["state"].as_str().unwrap()
            )
        })
        .collect()
}

#[test]
fn version_and_enabled_apply_to_every_kind() {
    let d = dirs("kinds");
    every_kind(&d.home, "absent", "", "");
    every_kind(&d.home, "one", r#", "version": 1"#, "version: 1\n");
    every_kind(&d.home, "newer", r#", "version": 2"#, "version: 2\n");
    every_kind(&d.home, "off", r#", "enabled": false"#, "enabled: false\n");
    put(&d.home.join("hooks.json"), r#"{"version": 9, "Stop": []}"#);
    put(
        &d.home.join("mcp.json"),
        r#"{"enabled": false, "servers": {"m": {"command": "x"}}}"#,
    );
    let items = list(&d, &d.project);
    for tag in ["absent", "one"] {
        let all = states(&items, &format!("-{tag}"));
        assert_eq!(all.len(), 6, "{all:?}");
        assert!(all.iter().all(|s| s.ends_with(":loaded")), "{all:?}");
    }
    let newer = states(&items, "-newer");
    assert_eq!(newer.len(), 6, "{newer:?}");
    assert!(newer.iter().all(|s| s.ends_with(":skipped")), "{newer:?}");
    let off = states(&items, "-off");
    assert_eq!(off.len(), 6, "{off:?}");
    assert!(off.iter().all(|s| s.ends_with(":disabled")), "{off:?}");
    assert_eq!(states(&items, "hooks.json"), ["hooks:skipped"]);
    assert_eq!(states(&items, "mcp.json"), ["mcp:disabled"]);
    let reason = items
        .iter()
        .find(|i| i["path"].as_str().unwrap().contains("t-newer"));
    assert!(reason.unwrap()["reason"]
        .as_str()
        .unwrap()
        .contains("newer than this build"));
}

#[test]
fn tool_disabled_alias_bad_names_and_layers() {
    let d = dirs("layers");
    put(&d.home.join("tools/shell.json"), r#"{"disabled": true}"#);
    put(&d.home.join("tools/bad.json"), r#"{"name": "a b"}"#);
    put(
        &d.home.join("panels/bad.json"),
        r#"{"id": "a b", "command": "x"}"#,
    );
    let project = d.project.join(".fast-harness");
    every_kind(&d.home, "same", "", "");
    every_kind(&project, "same", "", "");
    put(
        &project.join("hooks.json"),
        r#"{"SessionStart": [{"command": "marker"}]}"#,
    );
    let items = list(&d, &d.project);
    assert!(states(&items, "shell.json") == ["tools:disabled"]);
    assert!(states(&items, "tools\\bad.json")
        .iter()
        .chain(&states(&items, "tools/bad.json"))
        .any(|s| s == "tools:skipped"));
    assert!(
        states(&items, "proj")
            .iter()
            .all(|s| !s.ends_with(":loaded")),
        "untrusted: ignored"
    );
    trust::trust(&d.home, &d.project).unwrap();
    let items = list(&d, &d.project);
    let home_same: Vec<&Value> = (items.iter())
        .filter(|i| i["source"] == "home" && i["path"].as_str().unwrap().contains("-same"))
        .collect();
    assert_eq!(home_same.len(), 6);
    for item in home_same {
        assert_eq!(item["state"], "shadowed", "{item}");
        assert!(item["by"].as_str().unwrap().contains(".fast-harness"));
        assert_eq!(item["version"], 1);
    }
    let hook = items.iter().find(|i| i["kind"] == "hooks").unwrap();
    assert_eq!(
        (&hook["state"], &hook["source"]),
        (&Value::from("loaded"), &Value::from("project"))
    );
    assert!(!d.project.join("marker").exists(), "no hook ran");
}
