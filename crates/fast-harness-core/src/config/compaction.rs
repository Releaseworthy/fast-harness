//! The `compaction` settings (PLAN.md D38) and their per-model overrides:
//! `"compaction": {...}` at the top of `config.json`, then
//! `"models": {"<provider>/*" or "<model id>": {"compaction": {...}}}`.
//! [`resolve`] is the one place the layers meet (phase 17A adds file layers).

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

pub const DEFAULT_TRIGGER: Trigger = Trigger::Percent(50);
/// Owner decision G2 (2026-11-12): 50% of the window, capped at 500,000 tokens.
pub const DEFAULT_MAX_TOKENS: u64 = 500_000;
pub const DEFAULT_RESERVE: u64 = 16_000;
pub const DEFAULT_EFFORT: &str = "low";

/// One layer of settings; `None` keeps the value of the layer below.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CompactionPatch {
    pub auto: Option<bool>,
    /// `"50%"`, `"150k"`, `"1.2m"` or a number of tokens.
    #[serde(deserialize_with = "text_or_number")]
    pub trigger: Option<String>,
    pub max_tokens: Option<u64>,
    pub reserve: Option<u64>,
    /// Another model for the summary request (no cache hit then).
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Extra text appended to the summary request.
    pub instructions: Option<String>,
}

/// One entry of `models`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct ModelEntry {
    pub compaction: CompactionPatch,
}

/// The compaction trigger: a percent of the window or a token count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Percent(u64),
    Tokens(u64),
}

/// The resolved settings of one request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Compaction {
    pub auto: bool,
    pub trigger: Trigger,
    /// 0: no cap.
    pub max_tokens: u64,
    pub reserve: u64,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub instructions: Option<String>,
}

impl Default for Compaction {
    fn default() -> Compaction {
        Compaction {
            auto: true,
            trigger: DEFAULT_TRIGGER,
            max_tokens: DEFAULT_MAX_TOKENS,
            reserve: DEFAULT_RESERVE,
            model: None,
            effort: Some(DEFAULT_EFFORT.into()),
            instructions: None,
        }
    }
}

impl Compaction {
    fn apply(&mut self, patch: &CompactionPatch) {
        if let Some(auto) = patch.auto {
            self.auto = auto;
        }
        if let Some(trigger) = patch.trigger.as_deref().and_then(parse_trigger) {
            self.trigger = trigger;
        }
        if let Some(max) = patch.max_tokens {
            self.max_tokens = max;
        }
        if let Some(reserve) = patch.reserve {
            self.reserve = reserve;
        }
        if patch.model.is_some() {
            self.model.clone_from(&patch.model);
        }
        if patch.effort.is_some() {
            self.effort.clone_from(&patch.effort);
        }
        if patch.instructions.is_some() {
            self.instructions.clone_from(&patch.instructions);
        }
    }

    /// The context size that starts a compaction: the first bound reached of
    /// `trigger`, `maxTokens` and `window - reserve - max_output`. A window
    /// of 0 (unknown) leaves only the token bounds; 0 means never.
    pub fn threshold(&self, window: u64, max_output: u64) -> u64 {
        let mut limit = match self.max_tokens {
            0 => u64::MAX,
            max => max,
        };
        match self.trigger {
            Trigger::Percent(percent) if window > 0 => limit = limit.min(window * percent / 100),
            Trigger::Tokens(tokens) => limit = limit.min(tokens),
            Trigger::Percent(_) => {}
        }
        if window > 0 {
            limit = limit.min(window.saturating_sub(self.reserve.saturating_add(max_output)));
        }
        match limit {
            u64::MAX => 0,
            limit => limit,
        }
    }
}

/// Defaults, then `base` (the top-level `compaction`), then every
/// `"<prefix>/*"` entry matching the provider id or the model id's prefix,
/// then the exact model entry.
pub fn resolve(
    base: &CompactionPatch,
    models: &BTreeMap<String, ModelEntry>,
    provider: &str,
    model: &str,
) -> Compaction {
    let mut out = Compaction::default();
    out.apply(base);
    for (key, entry) in models {
        let Some(prefix) = key.strip_suffix("/*") else {
            continue;
        };
        let by_model = model
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'));
        if prefix == provider || by_model {
            out.apply(&entry.compaction);
        }
    }
    if let Some(entry) = models.get(model) {
        out.apply(&entry.compaction);
    }
    out
}

/// `"50%"`, `"150k"`, `"1.5m"` or `"120000"`; `None` for anything else.
pub fn parse_trigger(text: &str) -> Option<Trigger> {
    let text = text.trim().to_ascii_lowercase();
    if let Some(percent) = text.strip_suffix('%') {
        let value: f64 = percent.trim().parse().ok()?;
        return (value > 0.0 && value <= 100.0).then_some(Trigger::Percent(value as u64));
    }
    let (number, scale) = match text.chars().last()? {
        'k' => (&text[..text.len() - 1], 1_000.0),
        'm' => (&text[..text.len() - 1], 1_000_000.0),
        _ => (text.as_str(), 1.0),
    };
    let value: f64 = number.trim().parse().ok()?;
    (value > 0.0).then_some(Trigger::Tokens((value * scale) as u64))
}

fn text_or_number<'de, D: Deserializer<'de>>(de: D) -> Result<Option<String>, D::Error> {
    Ok(match Value::deserialize(de)? {
        Value::String(text) => Some(text),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    })
}
