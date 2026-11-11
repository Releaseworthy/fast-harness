//! Tool parameter schemas cleaned for Antigravity, ported from DSH's
//! antigravity-switch `cleanNode` and `cleanSchema` (live-verified): `$ref` to a
//! hint, `const` to `enum`, enum and constraint hints into the description,
//! `allOf` merged, `anyOf`/`oneOf` and type arrays flattened, unsupported
//! keywords dropped, `required` kept to real, non-nullable properties, and an
//! empty object given the `_placeholder` property Claude's VALIDATED mode needs.

use serde_json::{json, Map, Value};

/// Property added to an object schema without properties.
pub const PLACEHOLDER_NAME: &str = "_placeholder";
const PLACEHOLDER_DESCRIPTION: &str = "Placeholder. Always pass true.";

/// Constraints moved into the description.
const CONSTRAINTS: [&str; 10] = [
    "minLength",
    "maxLength",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "pattern",
    "minItems",
    "maxItems",
    "format",
    "default",
    "examples",
];

/// Keywords dropped after their hints are taken.
const DROPPED: [&str; 11] = [
    "$schema",
    "$defs",
    "definitions",
    "const",
    "$ref",
    "additionalProperties",
    "propertyNames",
    "title",
    "$id",
    "$comment",
    "allOf",
];

type Obj = Map<String, Value>;

/// A tool parameter schema Antigravity accepts: cleaned, always an object
/// with at least one property.
pub fn clean_schema(schema: &Value) -> Value {
    let input = match schema {
        Value::Object(_) => schema.clone(),
        _ => json!({"type": "object"}),
    };
    let mut result = match clean_node(&input) {
        Value::Object(map) => map,
        _ => Obj::new(),
    };
    result.insert("type".into(), json!("object"));
    result.remove("items");
    add_placeholder_if_empty(&mut result);
    Value::Object(result)
}

/// Clean one schema node and its children.
pub fn clean_node(input: &Value) -> Value {
    let Value::Object(source) = input else {
        return input.clone();
    };
    if let Some(reference) = source.get("$ref").and_then(Value::as_str) {
        return from_ref(source, reference);
    }
    let mut schema = with_hints(source.clone());
    if let Some(Value::Array(members)) = schema.get("allOf") {
        let cleaned: Vec<Value> = members.iter().map(clean_node).collect();
        schema.insert("allOf".into(), Value::Array(cleaned));
    }
    schema = merge_all_of(schema);
    for key in ["anyOf", "oneOf"] {
        if let Some(Value::Array(options)) = schema.get(key) {
            let cleaned: Vec<Value> = options.iter().map(clean_node).collect();
            schema.insert(key.into(), Value::Array(cleaned));
        }
    }
    schema = flatten_union(schema);
    schema = flatten_type_array(schema);
    let mut result = clean_children(schema);
    finish(&mut result);
    Value::Object(result)
}

fn from_ref(source: &Obj, reference: &str) -> Value {
    let name = reference.rsplit('/').next().unwrap_or(reference);
    let mut out = Obj::new();
    out.insert("type".into(), json!("object"));
    if let Some(description) = source.get("description") {
        out.insert("description".into(), description.clone());
    }
    Value::Object(with_hint(out, &format!("See: {name}")))
}

/// `const` as `enum`, then enum, `additionalProperties: false` and constraint hints.
fn with_hints(mut schema: Obj) -> Obj {
    if let Some(constant) = schema.get("const").cloned() {
        if !schema.contains_key("enum") {
            schema.insert("enum".into(), json!([constant]));
        }
    }
    if let Some(Value::Array(values)) = schema.get("enum") {
        if values.len() > 1 && values.len() <= 10 {
            let list: Vec<String> = values.iter().map(as_text).collect();
            schema = with_hint(schema, &format!("Allowed: {}", list.join(", ")));
        }
    }
    if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
        schema = with_hint(schema, "No extra properties allowed");
    }
    for constraint in CONSTRAINTS {
        match schema.get(constraint) {
            Some(value) if !value.is_object() && !value.is_array() && !value.is_null() => {
                let hint = format!("{constraint}: {}", as_text(value));
                schema = with_hint(schema, &hint);
            }
            _ => {}
        }
    }
    schema
}

/// Append `(hint)` to a schema's description.
fn with_hint(mut schema: Obj, hint: &str) -> Obj {
    let existing = schema
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("");
    let text = if existing.is_empty() {
        hint.to_string()
    } else {
        format!("{existing} ({hint})")
    };
    schema.insert("description".into(), Value::String(text));
    schema
}

/// A JSON value as JavaScript's `String(value)` prints it.
fn as_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Merge `allOf` members into the schema.
fn merge_all_of(mut schema: Obj) -> Obj {
    let Some(Value::Array(members)) = schema.remove("allOf") else {
        return schema;
    };
    for member in members {
        let Value::Object(member) = member else {
            continue;
        };
        if let Some(Value::Object(props)) = member.get("properties") {
            let target = schema.entry("properties").or_insert_with(|| json!({}));
            if let Value::Object(target) = target {
                target.extend(props.clone());
            }
        }
        if let Some(Value::Array(required)) = member.get("required") {
            let mut list = match schema.get("required") {
                Some(Value::Array(have)) => have.clone(),
                _ => Vec::new(),
            };
            for name in required {
                if !list.contains(name) {
                    list.push(name.clone());
                }
            }
            schema.insert("required".into(), Value::Array(list));
        }
        for (key, value) in member {
            if key != "properties" && key != "required" && !schema.contains_key(&key) {
                schema.insert(key, value);
            }
        }
    }
    schema
}

/// Score of a union option: object 3, array 2, other type 1, null 0.
fn option_score(option: &Value) -> (u8, String) {
    let Value::Object(map) = option else {
        return (0, "unknown".into());
    };
    let kind = map.get("type").and_then(Value::as_str);
    if kind == Some("object") || map.contains_key("properties") {
        return (3, "object".into());
    }
    if kind == Some("array") || map.contains_key("items") {
        return (2, "array".into());
    }
    // JavaScript truthiness: an empty string, null or false type is no type.
    match map.get("type") {
        Some(Value::String(name)) if name == "null" => (0, "null".into()),
        Some(Value::String(name)) if name.is_empty() => (0, "null".into()),
        Some(Value::Null) | Some(Value::Bool(false)) | None => (0, "null".into()),
        Some(other) => (1, as_text(other)),
    }
}

/// Enum values when every union option is a const or enum, else `None`.
fn union_enum(options: &[Value]) -> Option<Vec<Value>> {
    let mut values = Vec::new();
    for option in options {
        let Value::Object(map) = option else {
            return None;
        };
        if let Some(constant) = map.get("const") {
            values.push(Value::String(as_text(constant)));
        } else if let Some(Value::Array(list)) = map
            .get("enum")
            .filter(|v| v.as_array().is_some_and(|a| !a.is_empty()))
        {
            values.extend(list.iter().map(|v| Value::String(as_text(v))));
        } else if ["properties", "items", "anyOf", "oneOf", "allOf", "type"]
            .iter()
            .any(|key| map.contains_key(*key))
        {
            return None;
        }
    }
    (!values.is_empty()).then_some(values)
}

mod flatten;
use flatten::{clean_children, finish, flatten_type_array, flatten_union};

fn add_placeholder_if_empty(result: &mut Obj) {
    let empty = match result.get("properties") {
        Some(Value::Object(props)) => props.is_empty(),
        _ => true,
    };
    if empty {
        let placeholder = json!({"type": "boolean", "description": PLACEHOLDER_DESCRIPTION});
        result.insert(
            "properties".into(),
            json!({ PLACEHOLDER_NAME: placeholder }),
        );
        result.insert("required".into(), json!([PLACEHOLDER_NAME]));
    }
}
