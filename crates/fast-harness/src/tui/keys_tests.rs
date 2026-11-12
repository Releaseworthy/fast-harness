//! `keys.json` in the TUI (PLAN.md D45): an overridden key starts its
//! action, the old key no longer does, and the hint line shows the new key.

use fast_harness_core::config::layers::Source;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::ui::{Action, Screen, Ui};

fn press(ui: &mut Ui, code: KeyCode, mods: KeyModifiers) -> Action {
    ui.key(KeyEvent::new(code, mods))
}

#[test]
fn an_overridden_key_starts_its_action() {
    let mut ui = Ui::new("claude-sonnet-4-5".into(), None);
    let section =
        serde_json::json!({"pick_model": "f9", "toggle_trace": ["ctrl+shift+t"], "pause": null});
    assert!(ui.keys.apply(&section, Source::User).is_empty());
    assert_eq!(
        press(&mut ui, KeyCode::Char('p'), KeyModifiers::CONTROL),
        Action::None
    );
    assert!(ui.picker.is_none());
    assert_eq!(
        press(&mut ui, KeyCode::F(9), KeyModifiers::NONE),
        Action::FetchModels
    );
    assert!(ui.picker.is_some());
    press(&mut ui, KeyCode::Esc, KeyModifiers::NONE);
    assert!(super::keys::hint_text(&ui).contains("F9 model"));
    assert!(super::keys::hint_text(&ui).contains("Ctrl+Shift+T trace"));
    let shifted = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
    let open = press(&mut ui, KeyCode::Char('T'), shifted);
    assert_eq!(open, Action::Open(Screen::Trace));
    assert_eq!(
        press(&mut ui, KeyCode::F(3), KeyModifiers::NONE),
        Action::None
    );
    ui.screen = Screen::Chat;
    ui.running = true;
    assert_eq!(
        press(&mut ui, KeyCode::Char('g'), KeyModifiers::CONTROL),
        Action::None
    );
    assert!(!super::keys::hint_text(&ui).contains("pause"));
}

#[test]
fn a_rebound_screen_key_works() {
    let mut ui = Ui::new("claude-sonnet-4-5".into(), None);
    let section = serde_json::json!({"back": "backspace", "trace_full": "x"});
    assert!(ui.keys.apply(&section, Source::User).is_empty());
    ui.screen = Screen::Trace;
    press(&mut ui, KeyCode::Char('f'), KeyModifiers::NONE);
    assert!(!ui.trace.full);
    press(&mut ui, KeyCode::Char('x'), KeyModifiers::NONE);
    assert!(ui.trace.full);
    press(&mut ui, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(ui.screen, Screen::Trace);
    press(&mut ui, KeyCode::Backspace, KeyModifiers::NONE);
    assert_eq!(ui.screen, Screen::Chat);
}
