//! The settings of one served session (PLAN.md D48): any settings key can
//! be changed for that session only, and `model` and `effort` are offered
//! as ACP config options
//! (<https://agentclientprotocol.com/protocol/v2/session-config-options.md>).

use serde_json::{json, Value};

use crate::config::Config;
use crate::llm::catalog;

/// `config` with dotted `key` (the camelCase names of `config.json`, e.g.
/// `maxSteps` or `compaction.percent`) set to `value`. A string is parsed as
/// JSON when the key does not hold a string (`"40"` for a number). An
/// unknown key or a value of the wrong type is an error and changes nothing.
pub fn set_key(config: &Config, key: &str, value: &Value) -> Result<Config, String> {
    // Only the user's config.json weakens the approval gate (PLAN.md D64).
    if key == "approvals" || key == "allow" || key.starts_with("allow.") {
        return Err(format!("{key}: set it in ~/.fast-harness/config.json"));
    }
    if key == "effort" {
        let word = value.as_str().ok_or("effort: expected a string")?;
        let mut next = config.clone();
        next.set_effort(word);
        return Ok(next);
    }
    let mut tree = serde_json::to_value(config).map_err(|err| err.to_string())?;
    let slot = key
        .split('.')
        .try_fold(&mut tree, |node, part| node.get_mut(part))
        .ok_or_else(|| format!("unknown settings key `{key}`"))?;
    *slot = match (&*slot, value) {
        (Value::String(_) | Value::Null, _) => value.clone(),
        (_, Value::String(text)) => serde_json::from_str(text).unwrap_or_else(|_| value.clone()),
        _ => value.clone(),
    };
    let mut next: Config = serde_json::from_value(tree).map_err(|err| format!("{key}: {err}"))?;
    next.interactive = config.interactive;
    Ok(next)
}

fn select(values: Vec<String>, current: &str, label: fn(&str) -> String) -> Vec<Value> {
    let mut values = values;
    if !values.iter().any(|value| value == current) {
        values.insert(0, current.to_string());
    }
    let mut seen: Vec<String> = Vec::new();
    values.retain(|value| {
        let fresh = !seen.contains(value);
        seen.push(value.clone());
        fresh
    });
    (values.iter())
        .map(|value| json!({"value": value, "name": label(value)}))
        .collect()
}

fn model_label(model: &str) -> String {
    catalog::display_name(model).unwrap_or_else(|| model.to_string())
}

/// The `configOptions` of a session: `model` (from `models` and the
/// current one) and `effort` (`default` plus the model's levels).
pub fn options(config: &Config, models: Vec<String>) -> Value {
    let effort = config.effort.clone().unwrap_or_else(|| "default".into());
    let mut levels = vec!["default".to_string()];
    levels.extend(catalog::efforts(&config.model));
    json!([
        {"configId": "model", "name": "Model", "category": "model", "type": "select",
            "currentValue": config.model,
            "options": select(models, &config.model, model_label)},
        {"configId": "effort", "name": "Effort", "category": "thought_level", "type": "select",
            "currentValue": effort, "options": select(levels, &effort, str::to_string)},
    ])
}
