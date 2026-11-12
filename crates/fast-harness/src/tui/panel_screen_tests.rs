use fast_harness_core::panels::flatten;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{json, Value};

use super::panel_screen::{PanelScreen, Step};
use super::theme::Styles;

fn screen() -> PanelScreen {
    let root = json!({"type": "column", "children": [
        {"type": "text", "text": "Hello"},
        {"type": "row", "children": [
            {"type": "button", "id": "go", "text": "Go"},
            {"type": "input", "id": "q", "label": "Find", "placeholder": "word"}
        ]},
        {"type": "checkbox", "id": "c", "label": "On", "checked": true},
        {"type": "select", "id": "s", "options": ["a", "b"], "value": "a"},
        {"type": "sparkline", "id": "x"}
    ]});
    PanelScreen {
        id: "demo".into(),
        title: "Demo".into(),
        rows: flatten(&root),
        status: "process ended (exit code 1): boom".into(),
        ended: true,
        ..PanelScreen::default()
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn text(screen: &PanelScreen) -> Vec<String> {
    (screen.lines(&Styles::default()).iter())
        .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect()
}

#[test]
fn the_tree_draws_as_text_with_marks() {
    let lines = text(&screen());
    assert_eq!(lines[0].trim(), "Hello");
    assert!(
        lines[1].contains("› [ Go ]") && lines[1].contains("Find: [word_]"),
        "{}",
        lines[1]
    );
    assert!(lines[2].contains("[x] On"));
    assert!(lines[3].contains("< a >"));
    assert!(lines[4].contains("! unknown widget sparkline"));
    assert_eq!(lines[5], "! process ended (exit code 1): boom");
}

#[test]
fn tab_and_enter_reach_every_control() {
    let mut s = screen();
    let click = Step::Send("click", "go".into(), Value::Null);
    assert_eq!(s.key(key(KeyCode::Enter)), click);
    s.key(key(KeyCode::Tab));
    for c in "rust".chars() {
        s.key(key(KeyCode::Char(c)));
    }
    let submit = Step::Send("submit", "q".into(), json!("rust"));
    assert_eq!(s.key(key(KeyCode::Enter)), submit);
    s.key(key(KeyCode::Tab));
    let change = Step::Send("change", "c".into(), json!(false));
    assert_eq!(s.key(key(KeyCode::Enter)), change);
    s.key(key(KeyCode::Tab));
    let next = Step::Send("change", "s".into(), json!("b"));
    assert_eq!(s.key(key(KeyCode::Right)), next);
    s.key(key(KeyCode::Tab));
    assert_eq!(
        s.key(key(KeyCode::Enter)),
        click,
        "Tab wraps to the first control"
    );
    assert_eq!(s.key(key(KeyCode::Esc)), Step::Close);
}
