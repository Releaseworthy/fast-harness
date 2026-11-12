//! Settings layers, project trust, provider files, doctor and prices (D41).

use std::path::{Path, PathBuf};

use fast_harness_core::app::{doctor, load_settings};
use fast_harness_core::config::layers::Source;
use fast_harness_core::config::trust;
use fast_harness_core::llm::{catalog, custom};
use fast_harness_core::stats;
use fast_harness_core::trace::TraceEvent;
use serde_json::json;

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
    let root = std::env::temp_dir().join(format!("fh-layers-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (home, project) = (root.join("home"), root.join("proj"));
    std::fs::create_dir_all(home.join("providers")).unwrap();
    std::fs::create_dir_all(project.join(".fast-harness").join("providers")).unwrap();
    Dirs {
        root,
        home,
        project,
    }
}

fn write(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
}

fn no_env(_: &str) -> Option<String> {
    None
}

fn project_file(d: &Dirs, name: &str) -> PathBuf {
    d.project.join(".fast-harness").join(name)
}

#[test]
fn later_layers_win_and_keys_keep_their_source() {
    let d = dirs("order");
    let user = r#"{"maxSteps": 10, "theme": "light", "compaction": {"trigger": "40%"}}"#;
    write(&d.home.join("config.json"), user);
    write(
        &project_file(&d, "config.json"),
        r#"{"maxSteps": 20, "compaction": {"reserve": 1}}"#,
    );
    trust::trust(&d.home, &d.project).unwrap();
    let env = |key: &str| (key == "FAST_HARNESS_THEME").then(|| "dark".to_string());
    let sub = d.project.join("src");
    std::fs::create_dir_all(&sub).unwrap();
    let mut layered = load_settings(&d.home, Some(&sub), &env).unwrap().layered;
    let c = &layered.config;
    assert_eq!(
        (c.max_steps, c.theme.as_str(), c.compaction.reserve),
        (20, "dark", Some(1))
    );
    let source = |key: &str| layered.get(key).unwrap().1;
    assert_eq!(source("maxSteps"), Source::Project);
    assert_eq!(source("theme"), Source::Env);
    assert_eq!(source("compaction.trigger"), Source::User);
    assert_eq!(source("compaction"), Source::Project);
    assert_eq!(source("retries"), Source::Default);
    assert!(layered.get("noSuchKey").is_none());
    layered.set_cli("model", "cli-model");
    assert_eq!(
        layered.get("model").unwrap(),
        (json!("cli-model"), Source::Cli)
    );
}

#[test]
fn an_untrusted_project_loads_nothing_and_a_trusted_one_does() {
    let d = dirs("trust");
    write(&project_file(&d, "config.json"), r#"{"maxSteps": 7}"#);
    let provider = r#"{"id": "projprov", "api": "openai-chat", "baseUrl": "http://127.0.0.1:9/v1",
        "models": [{"id": "m"}]}"#;
    write(&project_file(&d, "providers/p.json"), provider);
    let settings = load_settings(&d.home, Some(&d.project), &no_env).unwrap();
    assert_eq!(settings.layered.config.max_steps, 200);
    assert!(settings.providers.is_empty());
    let note = settings.layered.layers.untrusted_note().unwrap();
    assert!(note.contains("not trusted"), "{note}");
    assert!(settings.layered.report.skipped[0]
        .reason
        .contains("not trusted"));
    trust::trust(&d.home, &d.project).unwrap();
    let settings = load_settings(&d.home, Some(&d.project), &no_env).unwrap();
    assert_eq!(settings.layered.config.max_steps, 7);
    assert_eq!(settings.providers[0].id, "projprov");
    assert!(settings.layered.layers.untrusted_note().is_none());
    let text = std::fs::read_to_string(d.home.join("trusted.json")).unwrap();
    assert!(text.contains("\"version\": 1"), "{text}");
}

#[test]
fn doctor_names_bad_files_and_their_reasons() {
    let d = dirs("doctor");
    write(&d.home.join("providers/bad.json"), "{not json");
    write(
        &d.home.join("providers/grpc.json"),
        r#"{"id": "g", "api": "grpc", "baseUrl": "x"}"#,
    );
    write(
        &d.home.join("providers/ok.json"),
        r#"{"id": "okprov", "api": "openai-chat", "baseUrl": "http://h"}"#,
    );
    write(&project_file(&d, "config.json"), r#"{"maxSteps": "many"}"#);
    trust::trust(&d.home, &d.project).unwrap();
    let settings = load_settings(&d.home, Some(&d.project), &no_env).unwrap();
    let text = doctor(&settings);
    assert!(text.contains("bad.json: key must be a string"), "{text}");
    assert!(text.contains("unknown api `grpc`"), "{text}");
    assert!(text.contains("okprov (openai-chat, 0 models)"), "{text}");
    assert!(text.contains("config.json: invalid type"), "{text}");
    assert!(text.contains("(trusted)"), "{text}");
    write(&d.home.join("config.json"), "[1]");
    let err = load_settings(&d.home, None, &no_env).err().unwrap();
    assert!(err.contains("not a JSON object"), "{err}");
}

#[test]
fn a_model_entry_overrides_a_built_in_models_numbers() {
    let d = dirs("override");
    let file = r#"{"id": "anthropic", "models": [{"id": "claude-haiku-4-5",
        "contextWindow": 123456, "maxOutput": 1000, "compaction": {"trigger": "30%"}}]}"#;
    write(&d.home.join("providers/anthropic.json"), file);
    let settings = load_settings(&d.home, None, &no_env).unwrap();
    custom::remember(&settings.providers);
    assert_eq!(catalog::context_window("claude-haiku-4-5"), 123456);
    assert_eq!(catalog::max_output_override("claude-haiku-4-5"), Some(1000));
    assert_eq!(catalog::provider_of("claude-haiku-4-5"), catalog::ANTHROPIC);
    let entry = &settings.layered.config.models["claude-haiku-4-5"];
    assert_eq!(entry.compaction.trigger.as_deref(), Some("30%"));
}

#[test]
fn stats_prices_steps_of_models_with_a_price() {
    let file = r#"{"id": "priced", "api": "openai-chat", "baseUrl": "http://h",
        "models": [{"id": "m1", "price": {"input": 2, "output": 10, "cacheRead": 0.5}}]}"#;
    custom::remember(&[serde_json::from_str(file).unwrap()]);
    let answer = |model: &str, input: u64| TraceEvent {
        kind: "assistant/message".into(),
        seq: 0,
        time: 0,
        data: json!({"turn": 1, "step": 1, "message": {"role": "assistant", "content": [], "model": model},
            "usage": {"inputTokens": input, "outputTokens": 100_000, "cacheReadTokens": 1_000_000}}),
    };
    let events = [
        TraceEvent {
            kind: "step/start".into(),
            seq: 0,
            time: 0,
            data: json!({}),
        },
        answer("priced/m1", 500_000),
        answer("claude-opus-5-5", 1),
    ];
    let s = stats::compute("s", &events);
    // 0.5M x 2 + 0.1M x 10 + 1M x 0.5 = 2.5 dollars.
    assert_eq!((s.cost_micro_usd, s.priced_steps), (2_500_000, 1));
    assert!(stats::format_text(&s).contains("cost $2.5000 (1 of 1 steps priced)"));
    let price: custom::Price = serde_json::from_str(r#"{"input": 1, "output": 0}"#).unwrap();
    let usage = fast_harness_core::types::Usage {
        cache_write: 3_000_000,
        cache_write_1h: 1_000_000,
        ..Default::default()
    };
    // 2M 5 minute writes at the input price, 1M 1 hour writes at twice it.
    assert!((price.cost(&usage) - 4.0).abs() < 1e-9);
}
