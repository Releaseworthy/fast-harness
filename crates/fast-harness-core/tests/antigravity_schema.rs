//! Antigravity tool schemas and function names.

use fast_harness_core::llm::antigravity::request::names::{sanitize, NameTable};
use fast_harness_core::llm::antigravity::request::schema::clean_schema;
use serde_json::json;

#[test]
fn empty_and_non_object_schemas_get_the_placeholder() {
    let expected = json!({
        "type": "object",
        "properties": {"_placeholder": {"type": "boolean", "description": "Placeholder. Always pass true."}},
        "required": ["_placeholder"]
    });
    assert_eq!(clean_schema(&json!({"type": "object"})), expected);
    assert_eq!(clean_schema(&json!(null)), expected);
    assert_eq!(
        clean_schema(&json!({"type": "object", "properties": {}})),
        expected
    );
}

#[test]
fn keywords_dropped_and_hints_kept() {
    let schema = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": "object",
        "title": "Args",
        "additionalProperties": false,
        "properties": {
            "path": {"type": "string", "minLength": 1, "description": "File"},
            "mode": {"const": "fast"},
            "kind": {"type": "string", "enum": ["a", "b"]},
            "title": {"type": "string"}
        },
        "required": ["path", "ghost"]
    });
    let out = clean_schema(&schema);
    assert!(out.get("$schema").is_none() && out.get("title").is_none());
    assert!(out.get("additionalProperties").is_none());
    assert_eq!(out["description"], "No extra properties allowed");
    let props = &out["properties"];
    assert_eq!(
        props["path"],
        json!({"type": "string", "description": "File (minLength: 1)"})
    );
    assert_eq!(props["mode"], json!({"enum": ["fast"]}));
    assert_eq!(props["kind"]["description"], "Allowed: a, b");
    assert_eq!(props["kind"]["enum"], json!(["a", "b"]));
    // A property named like a keyword stays.
    assert_eq!(props["title"], json!({"type": "string"}));
    assert_eq!(out["required"], json!(["path"]));
}

#[test]
fn refs_unions_and_type_arrays_flatten() {
    let schema = json!({
        "type": "object",
        "properties": {
            "node": {"$ref": "#/$defs/Node", "description": "Tree"},
            "either": {"anyOf": [{"type": "string"}, {"type": "object", "properties": {"x": {"type": "number"}}}]},
            "choice": {"oneOf": [{"const": "a"}, {"const": "b"}]},
            "maybe": {"type": ["string", "null"]},
            "list": {"type": "array"}
        },
        "required": ["maybe", "list"]
    });
    let out = clean_schema(&schema);
    let props = &out["properties"];
    assert_eq!(props["node"]["description"], "Tree (See: Node)");
    assert_eq!(props["node"]["type"], "object");
    assert_eq!(props["either"]["type"], "object");
    assert_eq!(props["either"]["description"], "Accepts: string | object");
    assert_eq!(props["either"]["properties"]["x"]["type"], "number");
    assert_eq!(
        props["choice"],
        json!({"type": "string", "enum": ["a", "b"]})
    );
    assert_eq!(
        props["maybe"],
        json!({"type": "string", "description": "nullable"})
    );
    assert_eq!(props["list"]["items"], json!({"type": "string"}));
    assert_eq!(out["required"], json!(["list"]));
}

#[test]
fn all_of_merges() {
    let schema = json!({
        "allOf": [
            {"type": "object", "properties": {"a": {"type": "string"}}, "required": ["a"]},
            {"properties": {"b": {"type": "integer", "maximum": 3}}, "required": ["b"]}
        ]
    });
    let out = clean_schema(&schema);
    assert!(out.get("allOf").is_none());
    assert_eq!(out["type"], "object");
    assert_eq!(out["properties"]["a"]["type"], "string");
    assert_eq!(
        out["properties"]["b"],
        json!({"type": "integer", "maximum": 3})
    );
    assert_eq!(out["required"], json!(["a", "b"]));
}

#[test]
fn names_are_sanitised() {
    assert_eq!(sanitize("read_file"), "read_file");
    assert_eq!(sanitize("mcp.server/tool name"), "mcp_server_tool_name");
    assert_eq!(sanitize("9lives"), "_9lives");
    assert_eq!(sanitize("-dash"), "_-dash");
    assert_eq!(sanitize(""), "_");
    assert_eq!(sanitize(&"x".repeat(80)).len(), 64);
}

#[test]
fn colliding_names_get_suffixes() {
    let mut table = NameTable::default();
    assert_eq!(table.sent("a.b"), "a_b");
    assert_eq!(table.sent("a/b"), "a_b_2");
    assert_eq!(table.sent("a_b"), "a_b_3");
    assert_eq!(table.sent("a.b"), "a_b");
    let long_a = format!("{}a", "y".repeat(70));
    let long_b = format!("{}b", "y".repeat(70));
    assert_eq!(table.sent(&long_a).len(), 64);
    let second = table.sent(&long_b);
    assert_eq!(second.len(), 64);
    assert!(second.ends_with("_2"));
    let names = table.into_tool_names();
    assert_eq!(names.original("a_b_2"), "a/b");
    assert_eq!(names.original("a_b_3"), "a_b");
    assert_eq!(names.original(&second), long_b);
}
