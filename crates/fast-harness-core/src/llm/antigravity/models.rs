//! The model catalog of an Antigravity account (`fetchAvailableModels`) as
//! entries named `antigravity/<id>`, with the effort levels each model takes
//! (PLAN.md D23, D24, "Antigravity provider").
//!
//! Kept and named as DSH's antigravity-switch does (`pickerIds`,
//! `normalizeCatalog`, live-verified): `gemini-`, `claude-` and `gpt-oss-` ids,
//! no image, imagen, `-disabled` or `-tiered` id and no disabled entry, and
//! only ids in `agentModelSorts` or `tieredModelIds` when those lists exist,
//! in Antigravity's order, then the rest sorted.

mod family;

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, PoisonError};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub use family::{
    efforts, family, family_of, fit_effort, id_level, Family, CLAUDE_BUDGETS, GEMINI_BUDGETS,
};

use super::hub::Hub;
use super::PREFIX;
use crate::http::Transport;
use crate::llm::anthropic::ModelInfo;
use crate::llm::{ErrorKind, LlmError};

/// Context window of a Gemini model the catalog gives none for (the public
/// Gemini API figure; unverified for Antigravity).
pub const GEMINI_CONTEXT: u64 = 1_048_576;
/// Context window of other models the catalog gives none for.
pub const OTHER_CONTEXT: u64 = 200_000;
/// Max output of a model the catalog gives none for.
pub const DEFAULT_MAX_OUTPUT: u64 = 65_536;

/// The catalog fields of one model (also the entries of ntigravity-models.json).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CatalogEntry {
    /// Bare id (no `antigravity/`).
    pub id: String,
    pub display_name: String,
    /// `maxTokens`.
    pub context_window: Option<u64>,
    /// `maxOutputTokens`.
    pub max_output: Option<u64>,
    pub supports_thinking: Option<bool>,
    pub supports_adaptive_thinking: Option<bool>,
    /// `thinkingBudget` (Claude 4.x).
    pub thinking_budget: Option<u64>,
    /// `thinkingLevel` as a word (catalog numbers 1 low, 2 medium, 3 high).
    pub thinking_level: Option<String>,
}

fn registry() -> &'static Mutex<HashMap<String, CatalogEntry>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, CatalogEntry>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Keep `entries` for [`lookup`] (a later read of the same id replaces it).
pub fn remember(entries: &[CatalogEntry]) {
    let mut map = registry().lock().unwrap_or_else(PoisonError::into_inner);
    for entry in entries {
        map.insert(entry.id.clone(), entry.clone());
    }
}

/// The remembered entry of `model` (bare or `antigravity/` id).
pub fn lookup(model: &str) -> Option<CatalogEntry> {
    let bare = model.strip_prefix(PREFIX).unwrap_or(model);
    let map = registry().lock().unwrap_or_else(PoisonError::into_inner);
    map.get(bare).cloned()
}

/// `fetchAvailableModels {project}` as model entries; the entries are also remembered.
pub fn list(
    http: &dyn Transport,
    hub: &Hub,
    access: &str,
    project: &str,
) -> Result<Vec<ModelInfo>, LlmError> {
    let body = json!({"project": project});
    let reply = hub
        .control_call(http, access, "fetchAvailableModels", &body, &[])
        .map_err(|err| LlmError::new(ErrorKind::Network, format!("fetchAvailableModels: {err}")))?;
    if !reply.ok() {
        let text = String::from_utf8_lossy(&reply.body);
        let quoted: String = text.chars().take(300).collect();
        let message = format!("fetchAvailableModels: HTTP {}: {quoted}", reply.status);
        let mut out = LlmError::new(kind_for_status(reply.status), message);
        out.status = Some(reply.status);
        return Err(out);
    }
    let json = reply
        .json()
        .ok_or_else(|| LlmError::new(ErrorKind::Protocol, "fetchAvailableModels: not JSON"))?;
    let entries = parse_catalog(&json);
    remember(&entries);
    Ok(entries.iter().map(to_info).collect())
}

fn kind_for_status(status: u16) -> ErrorKind {
    match status {
        401 | 403 => ErrorKind::Auth,
        429 => ErrorKind::RateLimit,
        500..=599 => ErrorKind::Server,
        400..=499 => ErrorKind::BadRequest,
        _ => ErrorKind::Protocol,
    }
}

/// The picker ids of one reply, in Antigravity's order.
pub fn picker_ids(reply: &Value) -> Vec<String> {
    let empty = serde_json::Map::new();
    let models = reply["models"].as_object().unwrap_or(&empty);
    let sorted: Vec<&str> = array(&reply["agentModelSorts"])
        .flat_map(|sort| array(&sort["groups"]))
        .flat_map(|group| array(&group["modelIds"]))
        .filter_map(Value::as_str)
        .collect();
    let tiered: Vec<&str> = reply["tieredModelIds"]
        .as_object()
        .map(|map| {
            map.values()
                .flat_map(array)
                .filter_map(Value::as_str)
                .collect()
        })
        .unwrap_or_default();
    let keep = |id: &str| {
        let Some(info) = models.get(id).filter(|info| info.is_object()) else {
            return false;
        };
        let known = ["gemini-", "claude-", "gpt-oss-"]
            .iter()
            .any(|p| id.starts_with(p));
        let excluded = id.contains("image") || id.ends_with("-disabled") || id.ends_with("-tiered");
        let listed = sorted.is_empty() || sorted.contains(&id) || tiered.contains(&id);
        known && !excluded && info["disabled"] != Value::Bool(true) && listed
    };
    let mut out: Vec<String> = Vec::new();
    for id in sorted.iter().copied().filter(|id| keep(id)) {
        if !out.iter().any(|have| have == id) {
            out.push(id.to_string());
        }
    }
    let mut rest: Vec<String> = models
        .keys()
        .filter(|id| keep(id) && !out.contains(id))
        .cloned()
        .collect();
    rest.sort();
    out.extend(rest);
    out
}

fn array(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_array().into_iter().flatten()
}

/// The picker models of one reply with their catalog fields.
pub fn parse_catalog(reply: &Value) -> Vec<CatalogEntry> {
    picker_ids(reply)
        .into_iter()
        .map(|id| entry_of(&id, &reply["models"][&id]))
        .collect()
}

fn entry_of(id: &str, info: &Value) -> CatalogEntry {
    let level = match &info["thinkingLevel"] {
        Value::String(word) if family::is_level(&word.to_lowercase()) => Some(word.to_lowercase()),
        other => match number(other) {
            Some(1) => Some("low".to_string()),
            Some(2) => Some("medium".to_string()),
            Some(3) => Some("high".to_string()),
            _ => None,
        },
    };
    let name = info["displayName"].as_str().map(str::trim);
    CatalogEntry {
        id: id.to_string(),
        display_name: name.filter(|n| !n.is_empty()).unwrap_or(id).to_string(),
        context_window: number(&info["maxTokens"]).filter(|n| *n > 0),
        max_output: number(&info["maxOutputTokens"]).filter(|n| *n > 0),
        supports_thinking: info["supportsThinking"].as_bool(),
        supports_adaptive_thinking: info["supportsAdaptiveThinking"].as_bool(),
        thinking_budget: number(&info["thinkingBudget"]).filter(|n| *n > 0),
        thinking_level: level,
    }
}

/// A non-negative integer given as a JSON number or a numeric string.
fn number(value: &Value) -> Option<u64> {
    match value {
        Value::Number(n) => n
            .as_u64()
            .or_else(|| n.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64)),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// The picker entry `antigravity/<id>` of a catalog entry.
pub fn to_info(entry: &CatalogEntry) -> ModelInfo {
    let efforts = family::efforts_of(&entry.id, Some(entry));
    let fixed_level = id_level(&entry.id).is_some();
    let context = if entry.id.starts_with("gemini-") {
        GEMINI_CONTEXT
    } else {
        OTHER_CONTEXT
    };
    ModelInfo {
        id: format!("{PREFIX}{}", entry.id),
        display_name: entry.display_name.clone(),
        context_window: entry.context_window.unwrap_or(context),
        max_output: entry.max_output.unwrap_or(DEFAULT_MAX_OUTPUT),
        reasoning: !efforts.is_empty() || fixed_level || entry.supports_thinking == Some(true),
        efforts,
        deprecated: false,
        live: None,
    }
}
