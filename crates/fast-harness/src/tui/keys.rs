//! The TUI side of `keys.json` (PLAN.md D45): a crossterm key event as a
//! [`Key`] for the keymap, and the hint line built from the bound keys.

use fast_harness_core::keys::Key;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::ui::{Screen, Ui};

/// The keymap key of a crossterm event; `None` for keys no action can use.
pub fn key_of(event: &KeyEvent) -> Option<Key> {
    let mods = event.modifiers;
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let alt = mods.contains(KeyModifiers::ALT);
    let shift = mods.contains(KeyModifiers::SHIFT);
    let meta = mods.intersects(KeyModifiers::SUPER | KeyModifiers::META);
    let name = match event.code {
        KeyCode::Char(c) => return Some(Key::from_char(c, ctrl, alt, shift, meta)),
        KeyCode::BackTab => {
            let name = "tab".into();
            let shift = true;
            return Some(Key {
                ctrl,
                alt,
                shift,
                meta,
                name,
            });
        }
        KeyCode::F(n) => format!("f{n}"),
        KeyCode::Enter => "enter".into(),
        KeyCode::Esc => "esc".into(),
        KeyCode::Tab => "tab".into(),
        KeyCode::Backspace => "backspace".into(),
        KeyCode::Delete => "delete".into(),
        KeyCode::Insert => "insert".into(),
        KeyCode::Up => "up".into(),
        KeyCode::Down => "down".into(),
        KeyCode::Left => "left".into(),
        KeyCode::Right => "right".into(),
        KeyCode::Home => "home".into(),
        KeyCode::End => "end".into(),
        KeyCode::PageUp => "pageup".into(),
        KeyCode::PageDown => "pagedown".into(),
        _ => return None,
    };
    Some(Key {
        ctrl,
        alt,
        shift,
        meta,
        name,
    })
}

/// `<key> <word>` of each bound action, joined with ` · `; an entry
/// `a|b` shows both actions' first keys as `A/B`.
fn hint(ui: &Ui, items: &[(&str, &str)]) -> Vec<String> {
    let mut out = Vec::new();
    for (actions, word) in items {
        let labels: Vec<String> = (actions.split('|'))
            .filter_map(|name| ui.keys.label(name))
            .collect();
        if !labels.is_empty() {
            out.push(format!("{} {word}", labels.join("/")));
        }
    }
    out
}

fn line(fixed: &[&str], ui: &Ui, items: &[(&str, &str)], tail: &[&str]) -> String {
    let mut parts: Vec<String> = fixed.iter().map(|s| s.to_string()).collect();
    parts.extend(hint(ui, items));
    parts.extend(tail.iter().map(|s| s.to_string()));
    parts.join(" · ")
}

const SCROLL: &str = "↑↓ PgUp PgDn Home End scroll";

const RUNNING: &[(&str, &str)] = &[
    ("steer", "steer"),
    ("queue", "queue"),
    ("pause", "pause"),
    ("remove_queued", "unqueue"),
    ("stop", "cancel"),
    ("pick_model", "model"),
    ("pick_effort", "effort"),
    ("scroll_page_up|scroll_page_down", "scroll"),
    ("toggle_thinking", "thinking"),
    ("toggle_trace", "trace"),
    ("cycle_theme", "theme"),
    ("greyscale_up", "grey"),
];

const IDLE: &[(&str, &str)] = &[
    ("send", "send"),
    ("newline", "newline"),
    ("pick_model", "model"),
    ("pick_effort", "effort"),
    ("toggle_thinking", "thinking"),
    ("new_session", "new"),
    ("open_sessions", "sessions"),
    ("open_accounts", "accounts"),
    ("toggle_trace", "trace"),
    ("cycle_theme", "theme"),
    ("greyscale_up", "grey"),
];

const ACCOUNT: &[(&str, &str)] = &[
    ("account_sign_in", "Claude sign-in"),
    ("account_add_key", "add key"),
    ("account_remove", "remove"),
    ("account_usage", "read usage"),
    ("back", "providers"),
];

/// The hint bar of the current screen with the keys as bound now.
pub fn hint_text(ui: &Ui) -> String {
    if !ui.status.is_empty() {
        return ui.status.clone();
    }
    let back = &[("back", "back")];
    match ui.screen {
        Screen::Chat if super::approval::shown(ui) => super::approval::keys_text(ui),
        Screen::Chat if ui.picker.is_some() => {
            "↑↓ PgUp PgDn choose · Enter pick · Esc close".into()
        }
        Screen::Chat if ui.running => line(&[], ui, RUNNING, &[]),
        Screen::Chat => line(&[], ui, IDLE, &[]),
        Screen::Accounts if ui.account_provider.is_none() => {
            let items = &[("account_usage", "read usage"), ("back", "back")];
            line(&["↑↓ choose", "Enter open provider"], ui, items, &[])
        }
        Screen::Accounts => line(&["Enter use"], ui, ACCOUNT, &[]),
        Screen::Trace if ui.trace.show_steps => {
            let items = [("trace_steps", "timeline")];
            join(line(&[], ui, &items, &[SCROLL]), hint(ui, back))
        }
        Screen::Trace => {
            let bodies = if ui.trace.full {
                "cut bodies"
            } else {
                "full bodies"
            };
            let items = [("trace_full", bodies), ("trace_steps", "steps table")];
            join(line(&[], ui, &items, &[SCROLL]), hint(ui, back))
        }
        Screen::Models => {
            let items = &[("cycle_effort", "cycle effort")];
            let fixed = &["↑↓ choose", "Enter use model"];
            join(line(fixed, ui, items, &[]), hint(ui, back))
        }
        Screen::Sessions => line(&["↑↓ choose", "Enter resume"], ui, back, &[]),
    }
}

fn join(mut text: String, more: Vec<String>) -> String {
    for part in more {
        text.push_str(" · ");
        text.push_str(&part);
    }
    text
}
