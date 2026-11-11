//! Antigravity model catalog, thinking config per family and effort, efforts.

use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use fast_harness_core::llm::antigravity::hub::Hub;
use fast_harness_core::llm::antigravity::models::{
    efforts, fit_effort, list, lookup, parse_catalog, picker_ids, remember, CatalogEntry,
};
use fast_harness_core::llm::antigravity::request::thinking::generation_config;
use fast_harness_core::llm::ErrorKind;
use serde_json::{json, Value};

fn reply() -> Value {
    json!({
        "models": {
            "gemini-3.1-pro-cat": {"displayName": "Gemini 3.1 Pro", "maxTokens": 1048576, "maxOutputTokens": 65535, "supportsThinking": true, "thinkingLevel": 3},
            "claude-opus-4-6-cat-thinking": {"displayName": "Claude Opus 4.6 (Thinking)", "maxTokens": "200000", "maxOutputTokens": 64000, "supportsThinking": true, "thinkingBudget": 1024},
            "claude-sonnet-5-5-cat": {"displayName": "Claude Sonnet 5.5", "supportsThinking": true, "supportsAdaptiveThinking": true, "thinkingLevel": 2},
            "gpt-oss-120b-cat": {"displayName": "GPT-OSS 120B"},
            "gemini-3-flash-cat": {"displayName": "Gemini 3 Flash"},
            "gemini-3.1-flash-image-cat": {"displayName": "Image"},
            "gemini-old-cat-disabled": {"displayName": "Old"},
            "gemini-gone-cat": {"displayName": "Gone", "disabled": true},
            "chat_20706": {"displayName": "Internal"},
            "gemini-hidden-cat": {"displayName": "Not in the sorts"}
        },
        "agentModelSorts": [{"groups": [
            {"modelIds": ["claude-sonnet-5-5-cat", "gemini-3.1-pro-cat", "gemini-3.1-flash-image-cat", "gemini-gone-cat"]},
            {"modelIds": ["claude-opus-4-6-cat-thinking", "gemini-3.1-pro-cat"]}
        ]}],
        "tieredModelIds": {"x": ["gpt-oss-120b-cat", "gemini-3-flash-cat"]}
    })
}

#[test]
fn catalog_keeps_and_orders_picker_models() {
    assert_eq!(
        picker_ids(&reply()),
        vec![
            "claude-sonnet-5-5-cat",
            "gemini-3.1-pro-cat",
            "claude-opus-4-6-cat-thinking",
            "gemini-3-flash-cat",
            "gpt-oss-120b-cat",
        ]
    );
    let without_sorts = json!({"models": {"gemini-b": {}, "gemini-a": {}, "other": {}}});
    assert_eq!(picker_ids(&without_sorts), vec!["gemini-a", "gemini-b"]);
    let entries = parse_catalog(&reply());
    let opus = &entries[2];
    assert_eq!(opus.context_window, Some(200_000));
    assert_eq!(opus.thinking_budget, Some(1024));
    assert_eq!(entries[0].thinking_level.as_deref(), Some("medium"));
}

#[test]
fn list_reads_names_and_remembers() {
    let http = ScriptedTransport::new(vec![
        ScriptedResponse::new(200, "version: 2.30.0\n", 4),
        ScriptedResponse::new(200, &reply().to_string(), 7),
    ]);
    let hub = Hub::new("https://manifest.example/m.yml", &["https://daily.example"]);
    let models = list(&http, &hub, "token", "proj-9").expect("models");
    let requests = http.requests();
    assert_eq!(
        requests[1].url,
        "https://daily.example/v1internal:fetchAvailableModels"
    );
    let sent: Value = serde_json::from_str(&requests[1].body).expect("json body");
    assert_eq!(sent, json!({"project": "proj-9"}));
    assert_eq!(models.len(), 5);
    let pro = &models[1];
    assert_eq!(pro.id, "antigravity/gemini-3.1-pro-cat");
    assert_eq!(pro.display_name, "Gemini 3.1 Pro");
    assert_eq!((pro.context_window, pro.max_output), (1_048_576, 65_535));
    assert!(pro.reasoning);
    assert_eq!(pro.efforts, vec!["low", "medium", "high"]);
    assert_eq!(models[0].efforts, vec!["low", "medium", "high", "off"]);
    let gpt = &models[4];
    assert!(!gpt.reasoning && gpt.efforts.is_empty());
    assert_eq!((gpt.context_window, gpt.max_output), (200_000, 65_536));
    assert!(lookup("antigravity/claude-opus-4-6-cat-thinking").is_some());
    // The Hub version is cached, so only the call goes out.
    let failing = ScriptedTransport::new(vec![ScriptedResponse::new(
        403,
        r#"{"error":{"message":"denied"}}"#,
        4,
    )]);
    let err = list(&failing, &hub, "token", "p").expect_err("403");
    assert_eq!((err.kind, err.status), (ErrorKind::Auth, Some(403)));
}

fn thinking_of(model: &str, max: u32, effort: Option<&str>) -> Value {
    generation_config(model, max, effort)
}

#[test]
fn gemini_thinking_per_family_and_effort() {
    let pro3 = thinking_of("gemini-3.5-pro-x", 9000, Some("medium"));
    assert_eq!(
        pro3,
        json!({"maxOutputTokens": 9000, "thinkingConfig": {"includeThoughts": true, "thinkingLevel": "medium"}})
    );
    let off3 = thinking_of("gemini-3.5-flash-x", 9000, Some("off"));
    assert_eq!(off3["thinkingConfig"]["thinkingLevel"], "minimal");
    let max3 = thinking_of("gemini-3-pro-x", 9000, Some("max"));
    assert_eq!(max3["thinkingConfig"]["thinkingLevel"], "high");
    let default3 = thinking_of("gemini-3.5-pro-x", 9000, None);
    assert_eq!(default3["thinkingConfig"]["thinkingLevel"], "high");
    let suffix = thinking_of("gemini-3-pro-low", 9000, Some("high"));
    assert_eq!(suffix["thinkingConfig"]["thinkingLevel"], "low");
    let flash25 = thinking_of("gemini-2.5-flash-x", 9000, Some("low"));
    assert_eq!(
        flash25["thinkingConfig"],
        json!({"includeThoughts": true, "thinkingBudget": 8192})
    );
    let off25 = thinking_of("gemini-2.5-flash-x", 9000, Some("off"));
    assert_eq!(off25["thinkingConfig"], json!({"thinkingBudget": 0}));
    let pro25 = thinking_of("gemini-2.5-pro-x", 9000, Some("off"));
    assert_eq!(pro25["thinkingConfig"]["thinkingBudget"], 8192);
}

#[test]
fn claude_thinking_per_family_and_effort() {
    let adaptive = thinking_of("claude-opus-5-5-x", 32000, Some("low"));
    assert_eq!(
        adaptive["thinkingConfig"],
        json!({"includeThoughts": true, "thinkingLevel": "low"})
    );
    assert_eq!(adaptive["maxOutputTokens"], 32000);
    let adaptive_off = thinking_of("claude-opus-5-5-x", 32000, Some("off"));
    assert_eq!(adaptive_off, json!({"maxOutputTokens": 32000}));
    let budget = thinking_of("claude-opus-4-6-x-thinking", 8000, Some("medium"));
    assert_eq!(
        budget["thinkingConfig"],
        json!({"include_thoughts": true, "thinking_budget": 16384})
    );
    assert_eq!(budget["maxOutputTokens"], 64000);
    let default_budget = thinking_of("claude-opus-4-6-x-thinking", 100_000, None);
    assert_eq!(default_budget["thinkingConfig"]["thinking_budget"], 16000);
    assert_eq!(default_budget["maxOutputTokens"], 100_000);
    let plain = thinking_of("claude-sonnet-4-6", 8000, Some("high"));
    assert_eq!(plain, json!({"maxOutputTokens": 8000}));
    let gpt = thinking_of("gpt-oss-120b-x", 8000, Some("high"));
    assert_eq!(gpt, json!({"maxOutputTokens": 8000}));
}

#[test]
fn registry_caps_output_and_names_the_default() {
    remember(&[CatalogEntry {
        id: "gemini-3.7-cap-test".into(),
        display_name: "Cap".into(),
        max_output: Some(1000),
        thinking_level: Some("low".into()),
        ..CatalogEntry::default()
    }]);
    let config = thinking_of("gemini-3.7-cap-test", 9000, None);
    assert_eq!(config["maxOutputTokens"], 1000);
    assert_eq!(config["thinkingConfig"]["thinkingLevel"], "low");
}

#[test]
fn efforts_and_fit_effort() {
    let words = |model: &str| efforts(model);
    assert_eq!(
        words("antigravity/gemini-3.5-flash-y"),
        vec!["minimal", "low", "medium", "high"]
    );
    assert_eq!(words("gemini-3-pro-preview-y"), vec!["low", "high"]);
    assert_eq!(words("gemini-2.5-pro-y"), vec!["low", "medium", "high"]);
    assert_eq!(
        words("gemini-2.5-flash-y"),
        vec!["low", "medium", "high", "off"]
    );
    assert_eq!(
        words("antigravity/claude-opus-5-5-y"),
        vec!["low", "medium", "high", "off"]
    );
    assert!(words("gemini-3-pro-high").is_empty());
    assert!(words("gpt-oss-120b-y").is_empty());
    assert_eq!(
        fit_effort("gemini-3-pro-preview-y", Some("medium")).as_deref(),
        Some("high")
    );
    assert_eq!(
        fit_effort("gemini-3-pro-preview-y", Some("off")).as_deref(),
        Some("low")
    );
    assert_eq!(
        fit_effort("gemini-2.5-flash-y", Some("off")).as_deref(),
        Some("off")
    );
    assert_eq!(
        fit_effort("claude-opus-5-5-y", Some("xhigh")).as_deref(),
        Some("high")
    );
    assert_eq!(
        fit_effort("gemini-3.5-flash-y", Some("minimal")).as_deref(),
        Some("minimal")
    );
    assert_eq!(fit_effort("claude-opus-5-5-y", None), None);
    assert_eq!(fit_effort("gpt-oss-120b-y", Some("high")), None);
}
