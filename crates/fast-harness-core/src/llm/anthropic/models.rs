//! The built-in Claude model table and `GET /v1/models`.
//!
//! Table values come from pi-ai's catalog (`providers/data/anthropic.json`,
//! as bundled in DeepSeek Harness, read 2026-11-11): context window,
//! max output, reasoning and the thinking flags (`forceAdaptiveThinking`,
//! `supportsMidConvoEffort`, `thinkingLevelMap`, `allowedFallbackModels`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::live::{self, LiveCaps, LiveModels};
use super::{error, wire};
use crate::http::Transport;
use crate::llm::{Auth, ErrorKind, LlmError};

/// One model as the pickers and `fast-harness models` show it; also the entry format
/// of the `models.json` cache.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub display_name: String,
    pub context_window: u64,
    pub max_output: u64,
    pub reasoning: bool,
    /// Effort levels the model takes, lowest first, then `off` when
    /// thinking can be switched off.
    #[serde(default)]
    pub efforts: Vec<String>,
    /// `lifecycle: deprecated` in the API list.
    #[serde(default)]
    pub deprecated: bool,
    /// Capabilities from the API; `None` for entries known only from the table.
    #[serde(default)]
    pub live: Option<LiveCaps>,
}

/// How a model is asked to think.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Thinking {
    /// `thinking: {type: enabled, budget_tokens}` plus the interleaved-thinking beta.
    Budget,
    /// `thinking: {type: adaptive}` with `output_config.effort`.
    Adaptive,
    /// Adaptive thinking whose effort travels in a trailing system message
    /// (pi-ai `supportsMidConvoEffort`).
    Managed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ModelSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub context: u64,
    pub max_output: u64,
    pub thinking: Thinking,
    /// Effort names the model accepts beyond low, medium and high.
    pub extra_efforts: &'static [&'static str],
    /// pi-ai `thinkingLevelMap.off: null`: thinking cannot be switched off.
    pub no_off: bool,
    /// Server-side fallback models (pi-ai `allowedFallbackModels`).
    pub fallbacks: &'static [&'static str],
}

const XHIGH_MAX: &[&str] = &["xhigh", "max"];
const MAX: &[&str] = &["max"];

const fn spec(
    id: &'static str,
    name: &'static str,
    context: u64,
    max_output: u64,
    thinking: Thinking,
    extra_efforts: &'static [&'static str],
    no_off: bool,
) -> ModelSpec {
    ModelSpec {
        id,
        name,
        context,
        max_output,
        thinking,
        extra_efforts,
        no_off,
        fallbacks: &[],
    }
}

use Thinking::{Adaptive, Budget, Managed};

const M: u64 = 1_000_000;

#[rustfmt::skip]
pub(crate) const MODELS: &[ModelSpec] = &[
    spec("claude-opus-5-5", "Claude Opus 5.5", M, 128_000, Managed, XHIGH_MAX, true),
    spec("claude-sonnet-5-5", "Claude Sonnet 5.5", M, 128_000, Managed, XHIGH_MAX, true),
    spec("claude-haiku-5-5", "Claude Haiku 5.5", M, 128_000, Managed, XHIGH_MAX, true),
    spec("claude-fable-5-1", "Claude Fable 5.1", M, 128_000, Managed, XHIGH_MAX, true),
    spec("claude-opus-5", "Claude Opus 5", M, 128_000, Managed, XHIGH_MAX, true),
    ModelSpec {
        fallbacks: &["claude-opus-4-8", "claude-opus-5"],
        ..spec("claude-fable-5", "Claude Fable 5", M, 128_000, Adaptive, XHIGH_MAX, true)
    },
    spec("claude-sonnet-5", "Claude Sonnet 5", M, 128_000, Adaptive, XHIGH_MAX, false),
    spec("claude-opus-4-8", "Claude Opus 4.8", M, 128_000, Adaptive, XHIGH_MAX, false),
    spec("claude-opus-4-7", "Claude Opus 4.7", M, 128_000, Adaptive, XHIGH_MAX, false),
    spec("claude-opus-4-6", "Claude Opus 4.6", M, 128_000, Adaptive, MAX, false),
    spec("claude-sonnet-4-6", "Claude Sonnet 4.6", M, 128_000, Adaptive, MAX, false),
    spec("claude-opus-4-5", "Claude Opus 4.5", 200_000, 64_000, Budget, &[], false),
    spec("claude-opus-4-5-20251101", "Claude Opus 4.5", 200_000, 64_000, Budget, &[], false),
    spec("claude-sonnet-4-5", "Claude Sonnet 4.5", M, 64_000, Budget, &[], false),
    spec("claude-sonnet-4-5-20250929", "Claude Sonnet 4.5", M, 64_000, Budget, &[], false),
    spec("claude-haiku-4-5", "Claude Haiku 4.5", 200_000, 64_000, Budget, &[], false),
    spec("claude-haiku-4-5-20251001", "Claude Haiku 4.5", 200_000, 64_000, Budget, &[], false),
];

pub(crate) fn find(id: &str) -> Option<&'static ModelSpec> {
    MODELS.iter().find(|spec| spec.id == id)
}

/// The context window of `model`: the live list wins, then the table; 0 when unknown.
pub fn context_window(live: &LiveModels, model: &str) -> u64 {
    live.lookup(model)
        .map(|info| info.context_window)
        .filter(|window| *window > 0)
        .or_else(|| find(model).map(|spec| spec.context))
        .unwrap_or(0)
}

impl ModelSpec {
    fn info(&self) -> ModelInfo {
        ModelInfo {
            id: self.id.to_string(),
            display_name: self.name.to_string(),
            context_window: self.context,
            max_output: self.max_output,
            reasoning: true,
            efforts: table_efforts(self.id)
                .into_iter()
                .map(str::to_string)
                .collect(),
            deprecated: false,
            live: None,
        }
    }
}

/// The built-in table, newest first.
pub fn builtin_models() -> Vec<ModelInfo> {
    MODELS.iter().map(ModelSpec::info).collect()
}

/// The effort levels `model` takes, lowest first: `low`, `medium`, `high`,
/// then the model's own extra levels (`xhigh`, `max`), then `off` where
/// thinking can be switched off. The live capabilities of the last model
/// list (or the cache) win; else the table; a model in neither gets them all.
pub fn efforts(live: &LiveModels, model: &str) -> Vec<&'static str> {
    match live.lookup(model) {
        Some(info) => live::static_levels(&info.efforts),
        None => table_efforts(model),
    }
}

fn table_efforts(model: &str) -> Vec<&'static str> {
    let mut out = vec!["low", "medium", "high"];
    match find(model) {
        Some(spec) => {
            out.extend_from_slice(spec.extra_efforts);
            if !spec.no_off {
                out.push("off");
            }
        }
        None => out.extend_from_slice(&["xhigh", "max", "off"]),
    }
    out
}

/// `effort` when `model` takes it, `None` (the model default) as it is,
/// else `high`, or `None` when the model takes no effort at all.
pub fn fit_effort(live: &LiveModels, model: &str, effort: Option<&str>) -> Option<String> {
    let level = effort?;
    let levels = efforts(live, model);
    if levels.contains(&level) {
        Some(level.to_string())
    } else {
        levels.contains(&"high").then(|| "high".to_string())
    }
}

/// One page of `GET /v1/models`: a 200 body or the mapped error.
fn fetch_page(
    http: &dyn Transport,
    base_url: &str,
    auth: &Auth,
    after: Option<&str>,
) -> Result<Value, LlmError> {
    let mut url = format!("{base_url}/v1/models?limit=1000");
    if let Some(after) = after {
        url.push_str("&after_id=");
        url.push_str(after);
    }
    let (head, body) = wire::get(http, &url, auth)?;
    if !(200..300).contains(&head.status) {
        return Err(error::from_http(head, &body));
    }
    serde_json::from_slice(&body).map_err(|err| {
        let mut out = LlmError::new(ErrorKind::Protocol, format!("models list: {err}"));
        out.head = Some(head);
        out
    })
}

/// Lists the models this credential can use (API order, newest first),
/// filling sizes the API leaves out from the built-in table.
pub(crate) fn list(
    http: &dyn Transport,
    base_url: &str,
    auth: &Auth,
    live: &LiveModels,
) -> Result<Vec<ModelInfo>, LlmError> {
    let mut out = Vec::new();
    let mut after: Option<String> = None;
    // Up to 10 pages of 1000: far more models than exist.
    for _ in 0..10 {
        let page = fetch_page(http, base_url, auth, after.as_deref())?;
        let list = page["data"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        out.extend(list.iter().filter_map(merge));
        live.remember(&out);
        let more = page["has_more"].as_bool().unwrap_or(false);
        after = page["last_id"].as_str().map(str::to_string);
        if !more || after.is_none() {
            break;
        }
    }
    Ok(out)
}

/// One API entry merged with the table: positive API numbers and the API
/// capabilities win. Retired models are left out.
fn merge(entry: &Value) -> Option<ModelInfo> {
    let id = entry["id"].as_str()?;
    let lifecycle = entry["lifecycle"].as_str();
    if lifecycle == Some("retired") {
        return None;
    }
    let (live, efforts) = match live::parse(entry) {
        Some((caps, levels)) => (Some(caps), levels),
        None => (
            None,
            table_efforts(id).into_iter().map(str::to_string).collect(),
        ),
    };
    let known = find(id).map(ModelSpec::info);
    let number = |key: &str| entry[key].as_u64().filter(|n| *n > 0);
    let display_name = entry["display_name"]
        .as_str()
        .map(str::to_string)
        .or_else(|| known.as_ref().map(|k| k.display_name.clone()))
        .unwrap_or_else(|| id.to_string());
    let thinking = entry["capabilities"]["thinking"]["supported"].as_bool();
    Some(ModelInfo {
        id: id.to_string(),
        display_name,
        context_window: number("max_input_tokens")
            .or(known.as_ref().map(|k| k.context_window))
            .unwrap_or(0),
        max_output: number("max_tokens")
            .or(known.as_ref().map(|k| k.max_output))
            .unwrap_or(0),
        reasoning: thinking.or(known.map(|k| k.reasoning)).unwrap_or(false),
        efforts,
        deprecated: lifecycle == Some("deprecated"),
        live,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_prefers_api_numbers_and_falls_back_to_table() {
        let api = json!({"id": "claude-opus-4-7", "display_name": "Opus", "max_input_tokens": 0});
        let info = merge(&api).unwrap();
        assert_eq!((info.context_window, info.max_output), (M, 128_000));
        assert_eq!(info.display_name, "Opus");
        let new = json!({"id": "claude-x", "max_input_tokens": 5, "max_tokens": 6,
            "capabilities": {"thinking": {"supported": true}}});
        let info = merge(&new).unwrap();
        assert_eq!(
            (info.context_window, info.max_output, info.reasoning),
            (5, 6, true)
        );
        assert!(merge(&json!({"display_name": "no id"})).is_none());
        assert_eq!(builtin_models()[0].id, "claude-opus-5-5");
    }

    #[test]
    fn efforts_per_model() {
        let all = ["low", "medium", "high", "xhigh", "max"];
        assert_eq!(efforts(&LiveModels::default(), "claude-opus-5-5"), all);
        assert_eq!(
            efforts(&LiveModels::default(), "claude-opus-4-7"),
            [&all[..], &["off"]].concat()
        );
        assert_eq!(
            efforts(&LiveModels::default(), "claude-opus-4-6"),
            ["low", "medium", "high", "max", "off"]
        );
        assert_eq!(
            efforts(&LiveModels::default(), "claude-haiku-4-5"),
            ["low", "medium", "high", "off"]
        );
        assert_eq!(
            efforts(&LiveModels::default(), "claude-new"),
            [&all[..], &["off"]].concat()
        );
        assert_eq!(
            fit_effort(&LiveModels::default(), "claude-opus-5-5", Some("xhigh")).as_deref(),
            Some("xhigh")
        );
        assert_eq!(
            fit_effort(&LiveModels::default(), "claude-opus-5-5", Some("off")).as_deref(),
            Some("high")
        );
        assert_eq!(
            fit_effort(&LiveModels::default(), "claude-haiku-4-5", Some("max")).as_deref(),
            Some("high")
        );
        assert_eq!(
            fit_effort(&LiveModels::default(), "claude-haiku-4-5", None),
            None
        );
    }

    #[test]
    fn live_capabilities_win_over_the_table_defaults() {
        let entry = json!({"id": "claude-live-test-1", "lifecycle": "deprecated", "capabilities": {
            "effort": {"supported": true, "low": {"supported": true}, "high": {"supported": true}},
            "thinking": {"types": {"adaptive": {"supported": true}, "disabled": {"supported": true}}}}});
        let info = merge(&entry).unwrap();
        assert!(info.deprecated && info.live.is_some());
        assert_eq!(info.efforts, ["low", "high", "off"]);
        let live = LiveModels::default();
        assert_eq!(
            efforts(&live, "claude-live-test-1"),
            ["low", "medium", "high", "xhigh", "max", "off"]
        );
        live.remember(std::slice::from_ref(&info));
        assert_eq!(efforts(&live, "claude-live-test-1"), ["low", "high", "off"]);
        let fit = fit_effort(&live, "claude-live-test-1", Some("max"));
        assert_eq!(fit.as_deref(), Some("high"));
        let bare =
            json!({"id": "claude-live-test-2", "capabilities": {"thinking": {"supported": false}}});
        live.remember(&[merge(&bare).unwrap()]);
        assert!(efforts(&live, "claude-live-test-2").is_empty());
        assert_eq!(fit_effort(&live, "claude-live-test-2", Some("high")), None);
        assert!(merge(&json!({"id": "claude-old", "lifecycle": "retired"})).is_none());
        let table = merge(&json!({"id": "claude-haiku-4-5"})).unwrap();
        assert_eq!(table.efforts, ["low", "medium", "high", "off"]);
        assert!(table.live.is_none());
    }
}
