use fast_harness_core::panels::flatten;
use serde_json::{json, Value};
use slint::Model;

use crate::panels::{activate, flat, glyph, rows};

#[test]
fn panel_rows_map_to_the_slint_model() {
    let root = json!({"type": "column", "children": [
        {"type": "row", "children": [
            {"type": "select", "id": "s", "label": "Pick", "options": ["a", "b"], "value": "b"},
            {"type": "widget-x"}
        ]},
        {"type": "column", "children": [{"type": "progress", "value": 0.5}]},
        {"type": "checkbox", "id": "c", "label": "On", "checked": true},
        {"type": "table", "columns": ["k", "v"], "rows": [["a", 1]]}
    ]});
    let model = rows(&flatten(&root));
    let all: Vec<_> = (0..model.row_count())
        .filter_map(|i| model.row_data(i))
        .collect();
    let looks: Vec<i32> = all.iter().map(|r| r.look).collect();
    assert_eq!(looks, [1, 4, 3, 0, 5, 0]);
    let texts: Vec<&str> = all.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "Pick: b  >",
            "! unknown widget widget-x",
            "50%",
            "[x] On",
            "k\tv",
            "a\t1"
        ]
    );
    assert!(all[0].click && !all[1].click && all[3].click);
    assert_eq!(all[2].depth, 1);
}

#[test]
fn a_click_sends_the_event_of_its_widget() {
    let root = json!({"type": "column", "children": [
        {"type": "button", "id": "go", "text": "Go"},
        {"type": "checkbox", "id": "c", "checked": true},
        {"type": "select", "id": "s", "options": ["a", "b"], "value": "b"},
        {"type": "list", "id": "l", "items": ["one"]},
        {"type": "text", "text": "plain"}
    ]});
    let cells = flat(&flatten(&root));
    let sent: Vec<_> = cells.iter().map(|(_, c)| activate(c)).collect();
    assert_eq!(sent[0], Some(("click", Value::Null)));
    assert_eq!(sent[1], Some(("change", json!(false))));
    assert_eq!(
        sent[2],
        Some(("change", json!("a"))),
        "a select wraps to its first option"
    );
    assert_eq!(sent[3], Some(("click", json!("one"))));
    assert_eq!(sent[4], None);
}

#[test]
fn panel_icons_are_glyphs_of_the_icon_font() {
    let icons = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/icons.slint"))
        .expect("icons.slint");
    for name in ["tools", "pulse", "terminal", "settings-gear", "", "nope"] {
        let code = format!("\\u{{{:x}}}", glyph(name).chars().next().unwrap() as u32);
        assert!(
            icons.contains(&code),
            "{name}: {code} is not in icons.slint"
        );
    }
}
