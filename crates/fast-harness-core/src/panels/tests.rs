use serde_json::json;

use super::*;

fn kinds(rows: &[Row]) -> Vec<Vec<&str>> {
    rows.iter()
        .map(|r| r.cells.iter().map(|c| c.kind.as_str()).collect())
        .collect()
}

#[test]
fn a_tree_flattens_into_rows_with_depth_and_cells() {
    let root = json!({"type": "column", "id": "root", "children": [
        {"type": "text", "id": "t", "text": "Hello"},
        {"type": "row", "children": [
            {"type": "input", "id": "q", "label": "Find", "placeholder": "text", "value": "x"},
            {"type": "button", "id": "go", "text": "Go"}
        ]},
        {"type": "column", "children": [{"type": "checkbox", "id": "c", "label": "On", "checked": true}]},
        {"type": "select", "id": "s", "options": ["a", "b"], "value": "b"},
        {"type": "progress", "label": "Done", "value": 1.7},
        {"type": "list", "id": "l", "items": ["one", 2]},
        {"type": "table", "columns": ["k", "v"], "rows": [["a", 1]]}
    ]});
    let rows = flatten(&root);
    let want: Vec<Vec<&str>> = vec![
        vec!["text"],
        vec!["input", "button"],
        vec!["checkbox"],
        vec!["select"],
        vec!["progress"],
        vec!["item"],
        vec!["item"],
        vec!["cell", "cell"],
        vec!["cell", "cell"],
    ];
    assert_eq!(kinds(&rows), want);
    assert_eq!(rows[2].depth, 1, "a nested column indents");
    let input = &rows[1].cells[0];
    assert_eq!(
        (
            input.text.as_str(),
            input.hint.as_str(),
            input.value.as_str()
        ),
        ("Find", "text", "x")
    );
    assert!(rows[2].cells[0].checked);
    assert_eq!(rows[3].cells[0].options, ["a", "b"]);
    assert_eq!(rows[4].cells[0].progress, 1.0, "progress is clamped");
    assert_eq!(rows[6].cells[0].text, "2");
    assert!(rows[7].cells[0].header && !rows[8].cells[0].header);
    assert_eq!(rows[8].cells[1].text, "1");
}

#[test]
fn an_unknown_widget_is_a_note() {
    let root = json!({"type": "column", "children": [{"type": "chart", "id": "x"}, {"id": "y"}]});
    let rows = flatten(&root);
    assert_eq!(rows[0].cells[0].kind, "note");
    assert_eq!(rows[0].cells[0].text, "unknown widget chart");
    assert_eq!(rows[1].cells[0].text, "unknown widget (no type)");
}

#[test]
fn a_patch_replaces_the_node_with_its_id() {
    let mut root =
        json!({"type": "column", "children": [{"type": "text", "id": "t", "text": "a"}]});
    assert!(patch(
        &mut root,
        "t",
        json!({"type": "text", "id": "t", "text": "b"})
    ));
    assert_eq!(flatten(&root)[0].cells[0].text, "b");
    assert!(!patch(&mut root, "missing", json!({})));
}

#[test]
fn lines_parse_into_messages() {
    assert_eq!(parse_line("  "), None);
    let view = parse_line(r#"{"type":"view","root":{"type":"text"}}"#);
    assert_eq!(view, Some(Incoming::View(json!({"type": "text"}))));
    let sub = parse_line(r#"{"type":"subscribe","events":["turn","tool"]}"#);
    assert_eq!(
        sub,
        Some(Incoming::Subscribe(vec!["turn".into(), "tool".into()]))
    );
    assert!(matches!(parse_line("hello"), Some(Incoming::Bad(why)) if why.starts_with("not JSON")));
    assert!(matches!(
        parse_line(r#"{"type":"zap"}"#),
        Some(Incoming::Bad(_))
    ));
}

#[test]
fn init_and_events_are_json_lines() {
    let host = Host {
        cwd: std::path::Path::new("/work"),
        session: "s1",
        dark: true,
    };
    let init: serde_json::Value = serde_json::from_str(&init_line("p", host)).unwrap();
    assert_eq!(init["type"], "init");
    assert_eq!(init["theme"]["kind"], "dark");
    assert_eq!(init["session"], "s1");
    let click: serde_json::Value =
        serde_json::from_str(&event_line("click", "go", json!(null))).unwrap();
    assert_eq!(click, json!({"type": "click", "id": "go", "value": null}));
}

#[test]
fn session_events_map_to_kinds() {
    let start = AgentEvent::TurnStart { turn: 2 };
    assert_eq!(session_event(&start).map(|e| e.0), Some("turn"));
    let call = AgentEvent::ToolCall {
        id: "1".into(),
        name: "read".into(),
        arguments: "{}".into(),
    };
    assert_eq!(session_event(&call).unwrap().1["name"], "read");
    assert!(session_event(&AgentEvent::TextDelta("x".into())).is_none());
}
