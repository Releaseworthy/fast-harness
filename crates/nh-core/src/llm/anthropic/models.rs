//! The built-in Claude model table and `GET /v1/models`.
//!
//! Table values come from pi-ai's catalog (`providers/data/anthropic.json`,
//! as bundled in DeepSeek Harness, read 2026-11-11): context window,
//! max output, reasoning and the thinking flags (`forceAdaptiveThinking`,
//! `supportsMidConvoEffort`, `thinkingLevelMap`, `allowedFallbackModels`).

use serde_json::Value;

use super::{error, wire};
use crate::http::Transport;
use crate::llm::{Auth, ErrorKind, LlmError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: String,
    pub context_window: u64,
    pub max_output: u64,
    pub reasoning: bool,
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

impl ModelSpec {
    fn info(&self) -> ModelInfo {
        ModelInfo {
            id: self.id.to_string(),
            display_name: self.name.to_string(),
            context_window: self.context,
            max_output: self.max_output,
            reasoning: true,
        }
    }
}

/// The built-in table, newest first.
pub fn builtin_models() -> Vec<ModelInfo> {
    MODELS.iter().map(ModelSpec::info).collect()
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
) -> Result<Vec<ModelInfo>, LlmError> {
    let mut out = Vec::new();
    let mut after: Option<String> = None;
    // Up to 10 pages of 1000: far more models than exist.
    for _ in 0..10 {
        let page = fetch_page(http, base_url, auth, after.as_deref())?;
        let list = page["data"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        out.extend(list.iter().filter_map(merge));
        let more = page["has_more"].as_bool().unwrap_or(false);
        after = page["last_id"].as_str().map(str::to_string);
        if !more || after.is_none() {
            break;
        }
    }
    Ok(out)
}

/// One API entry merged with the table: positive API numbers win.
fn merge(entry: &Value) -> Option<ModelInfo> {
    let id = entry["id"].as_str()?;
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
}
