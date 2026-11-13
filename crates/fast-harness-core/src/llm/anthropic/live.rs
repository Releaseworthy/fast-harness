//! Live model capabilities from `GET /v1/models` and the per-runtime table
//! ([`LiveModels`], PLAN.md D67) that `efforts`, `fit_effort` and the request builder read.
//!
//! Field names follow the official TypeScript SDK (`src/resources/models.ts`)
//! and <https://platform.claude.com/docs/en/api/models> (read 2026-11-11):
//! `capabilities.effort.{supported, low, medium, high, xhigh, max}` and
//! `capabilities.thinking.types.{adaptive, enabled, disabled}`, each with a
//! `supported` flag; a null or missing flag counts as false.

use std::collections::HashMap;
use std::sync::{PoisonError, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::models::ModelInfo;

/// Every effort name, lowest first, `off` last.
pub(crate) const LEVELS: [&str; 6] = ["low", "medium", "high", "xhigh", "max", "off"];

/// What the API says a model accepts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveCaps {
    /// `capabilities.effort.supported`: `output_config.effort` is accepted.
    pub effort: bool,
    /// `thinking.types.adaptive.supported`.
    pub adaptive: bool,
    /// `thinking.types.enabled.supported`: budget thinking.
    pub budget: bool,
    /// `thinking.types.disabled.supported`: thinking can be switched off.
    pub can_disable: bool,
}

fn supported(value: &Value) -> bool {
    value["supported"].as_bool().unwrap_or(false)
}

/// The capabilities and effort levels (lowest first, then `off`) of one API
/// entry; `None` when the entry has no `capabilities` object.
pub(crate) fn parse(entry: &Value) -> Option<(LiveCaps, Vec<String>)> {
    let caps = entry.get("capabilities").filter(|c| c.is_object())?;
    let effort = &caps["effort"];
    let types = &caps["thinking"]["types"];
    let live = LiveCaps {
        effort: supported(effort),
        adaptive: supported(&types["adaptive"]),
        budget: supported(&types["enabled"]),
        can_disable: supported(&types["disabled"]),
    };
    let mut levels: Vec<String> = if live.effort {
        LEVELS[..5]
            .iter()
            .filter(|level| supported(&effort[**level]))
            .map(|level| level.to_string())
            .collect()
    } else if live.budget {
        vec!["low".into(), "medium".into(), "high".into()]
    } else {
        Vec::new()
    };
    if live.can_disable {
        levels.push("off".into());
    }
    Some((live, levels))
}

/// The live model table of one runtime (PLAN.md D67): what the last model
/// list (or the cache) said each model accepts. A value, shared by the
/// runtime's catalog and its Claude provider through an `Arc`.
#[derive(Debug, Default)]
pub struct LiveModels {
    map: RwLock<HashMap<String, ModelInfo>>,
}

impl LiveModels {
    /// Records the live capabilities of `list` (a fetched or cached model
    /// list): entries with capabilities replace what was known, entries
    /// without them drop back to the built-in table.
    pub fn remember(&self, list: &[ModelInfo]) {
        let mut map = self.map.write().unwrap_or_else(PoisonError::into_inner);
        for info in list {
            if info.live.is_some() {
                map.insert(info.id.clone(), info.clone());
            } else {
                map.remove(&info.id);
            }
        }
    }

    /// The live entry of `model`, when a list or the cache reported its capabilities.
    pub fn lookup(&self, model: &str) -> Option<ModelInfo> {
        let map = self.map.read().unwrap_or_else(PoisonError::into_inner);
        map.get(model).cloned()
    }
}
/// `levels` as the fixed level names (unknown names dropped).
pub(crate) fn static_levels(levels: &[String]) -> Vec<&'static str> {
    levels
        .iter()
        .filter_map(|level| LEVELS.iter().copied().find(|name| name == level))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_effort_and_thinking_capabilities() {
        let entry = json!({"capabilities": {
            "effort": {"supported": true, "low": {"supported": true}, "medium": {"supported": true},
                "high": {"supported": true}, "xhigh": null, "max": {"supported": true}},
            "thinking": {"supported": true, "types": {"adaptive": {"supported": true},
                "enabled": {"supported": false}, "disabled": {"supported": true}}}}});
        let (caps, levels) = parse(&entry).unwrap();
        assert!(caps.effort && caps.adaptive && caps.can_disable && !caps.budget);
        assert_eq!(levels, ["low", "medium", "high", "max", "off"]);
        let budget = json!({"capabilities": {"effort": {"supported": false},
            "thinking": {"supported": true, "types": {"enabled": {"supported": true}}}}});
        assert_eq!(parse(&budget).unwrap().1, ["low", "medium", "high"]);
        let none = json!({"capabilities": {"thinking": {"supported": false}}});
        assert!(parse(&none).unwrap().1.is_empty());
        assert!(parse(&json!({"capabilities": null})).is_none());
        assert!(parse(&json!({"id": "x"})).is_none());
        let names = static_levels(&["max".into(), "bogus".into(), "off".into()]);
        assert_eq!(names, ["max", "off"]);
    }
}
