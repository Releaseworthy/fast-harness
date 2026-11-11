//! Union and type-array flattening, children and the final fix-ups of one
//! cleaned schema node.

use serde_json::{json, Value};

use super::{
    add_placeholder_if_empty, as_text, clean_node, option_score, union_enum, with_hint, Obj,
    CONSTRAINTS, DROPPED,
};

/// Flatten `anyOf`/`oneOf` to the best option with an `Accepts:` hint; an
/// all-const union becomes a string enum.
pub(super) fn flatten_union(mut result: Obj) -> Obj {
    for key in ["anyOf", "oneOf"] {
        let options = match result.get(key) {
            Some(Value::Array(options)) if !options.is_empty() => options.clone(),
            _ => continue,
        };
        let mut rest = result.clone();
        rest.remove(key);
        if let Some(values) = union_enum(&options) {
            rest.insert("type".into(), json!("string"));
            rest.insert("enum".into(), Value::Array(values));
            result = rest;
            continue;
        }
        result = pick_option(rest, &options);
    }
    result
}

/// The highest scoring option (first on a tie) merged over `rest`.
fn pick_option(mut rest: Obj, options: &[Value]) -> Obj {
    let mut best = 0;
    let mut best_score: i16 = -1;
    let mut types: Vec<String> = Vec::new();
    for (index, option) in options.iter().enumerate() {
        let (score, kind) = option_score(option);
        if !types.contains(&kind) {
            types.push(kind);
        }
        if i16::from(score) > best_score {
            best_score = i16::from(score);
            best = index;
        }
    }
    let mut selected = match options.get(best) {
        Some(Value::Object(map)) => map.clone(),
        _ => json!({"type": "string"})
            .as_object()
            .cloned()
            .unwrap_or_default(),
    };
    let parent = rest.remove("description");
    let parent = parent.as_ref().and_then(Value::as_str).unwrap_or("");
    let child = selected
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !parent.is_empty() {
        let text = if !child.is_empty() && child != parent {
            format!("{parent} ({child})")
        } else {
            parent.to_string()
        };
        selected.insert("description".into(), Value::String(text));
    }
    if types.len() > 1 {
        selected = with_hint(selected, &format!("Accepts: {}", types.join(" | ")));
    }
    rest.extend(selected);
    rest
}

/// One type out of a type array, with `Accepts:` and `nullable` hints.
pub(super) fn flatten_type_array(schema: Obj) -> Obj {
    let Some(Value::Array(kinds)) = schema.get("type") else {
        return schema;
    };
    let non_null: Vec<String> = kinds
        .iter()
        .filter(|kind| !kind.is_null() && kind.as_str() != Some("null"))
        .map(as_text)
        .collect();
    let nullable = kinds.iter().any(|kind| kind.as_str() == Some("null"));
    let mut result = schema.clone();
    let first = non_null.first().cloned().unwrap_or_else(|| "string".into());
    result.insert("type".into(), Value::String(first));
    if non_null.len() > 1 {
        result = with_hint(result, &format!("Accepts: {}", non_null.join(" | ")));
    }
    if nullable {
        result = with_hint(result, "nullable");
    }
    result
}

/// Unsupported keywords dropped and children cleaned; property names are
/// never mistaken for keywords.
pub(super) fn clean_children(schema: Obj) -> Obj {
    let mut result = Obj::new();
    for (key, value) in schema {
        if DROPPED.contains(&key.as_str()) || CONSTRAINTS.contains(&key.as_str()) {
            continue;
        }
        let cleaned = match (key.as_str(), &value) {
            ("properties", Value::Object(props)) => Value::Object(
                props
                    .iter()
                    .map(|(name, child)| (name.clone(), clean_node(child)))
                    .collect(),
            ),
            ("items", Value::Array(list)) => {
                clean_node(list.first().unwrap_or(&json!({"type": "string"})))
            }
            ("items", _) => clean_node(&value),
            ("enum", _) => value,
            (_, Value::Object(_)) => clean_node(&value),
            _ => value,
        };
        result.insert(key, cleaned);
    }
    result
}

/// Arrays get `items`, `required` keeps real non-nullable properties, and an
/// empty object gets the placeholder property.
pub(super) fn finish(result: &mut Obj) {
    if result.get("type").and_then(Value::as_str) == Some("array") && !result.contains_key("items")
    {
        result.insert("items".into(), json!({"type": "string"}));
    }
    keep_real_required(result);
    if result.get("type").and_then(Value::as_str) == Some("object") {
        add_placeholder_if_empty(result);
    }
}

fn keep_real_required(result: &mut Obj) {
    let (Some(Value::Array(required)), Some(Value::Object(props))) =
        (result.get("required"), result.get("properties"))
    else {
        return;
    };
    let nullable = |child: &Value| {
        child
            .get("description")
            .and_then(Value::as_str)
            .is_some_and(|text| {
                text.split(|c: char| !c.is_alphanumeric() && c != '_')
                    .any(|w| w == "nullable")
            })
    };
    let kept: Vec<Value> = required
        .iter()
        .filter(|name| {
            name.as_str()
                .and_then(|name| props.get(name))
                .is_some_and(|child| !nullable(child))
        })
        .cloned()
        .collect();
    if kept.is_empty() {
        result.remove("required");
    } else {
        result.insert("required".into(), Value::Array(kept));
    }
}
