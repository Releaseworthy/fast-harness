//! The GUI side of `keys.json` (PLAN.md D45): `ui/main.slint` hands every
//! key of the window (capture) and of the message box to `key-action`, which
//! answers with the action name; Slint runs it. Text editing keys stay with
//! the text inputs.

use fast_harness_core::keys::{Key, Keymap};
use slint::platform::Key as SlintKey;

use crate::app::with_app;
use crate::MainWindow;

/// Slint's special keys and their keymap names.
const NAMED: [(SlintKey, &str); 15] = [
    (SlintKey::Return, "enter"),
    (SlintKey::Escape, "esc"),
    (SlintKey::Tab, "tab"),
    (SlintKey::Backtab, "tab"),
    (SlintKey::Backspace, "backspace"),
    (SlintKey::Delete, "delete"),
    (SlintKey::Insert, "insert"),
    (SlintKey::UpArrow, "up"),
    (SlintKey::DownArrow, "down"),
    (SlintKey::LeftArrow, "left"),
    (SlintKey::RightArrow, "right"),
    (SlintKey::Home, "home"),
    (SlintKey::End, "end"),
    (SlintKey::PageUp, "pageup"),
    (SlintKey::PageDown, "pagedown"),
];

const FUNCTION: [SlintKey; 24] = [
    SlintKey::F1,
    SlintKey::F2,
    SlintKey::F3,
    SlintKey::F4,
    SlintKey::F5,
    SlintKey::F6,
    SlintKey::F7,
    SlintKey::F8,
    SlintKey::F9,
    SlintKey::F10,
    SlintKey::F11,
    SlintKey::F12,
    SlintKey::F13,
    SlintKey::F14,
    SlintKey::F15,
    SlintKey::F16,
    SlintKey::F17,
    SlintKey::F18,
    SlintKey::F19,
    SlintKey::F20,
    SlintKey::F21,
    SlintKey::F22,
    SlintKey::F23,
    SlintKey::F24,
];

/// Ctrl, Alt, Shift, Meta.
pub type Mods = [bool; 4];

/// The keymap key of a Slint key event (its `text` and modifiers).
pub fn key_of(text: &str, [ctrl, alt, shift, meta]: Mods) -> Option<Key> {
    let mut chars = text.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    let named = NAMED.iter().find(|(k, _)| char::from(*k) == c);
    let name = match named {
        Some((key, name)) => {
            // Backtab is Shift+Tab.
            let shift = shift || *key == SlintKey::Backtab;
            return Some(Key {
                ctrl,
                alt,
                shift,
                meta,
                name: (*name).into(),
            });
        }
        None => FUNCTION.iter().position(|k| char::from(*k) == c),
    };
    if let Some(index) = name {
        let name = format!("f{}", index + 1);
        return Some(Key {
            ctrl,
            alt,
            shift,
            meta,
            name,
        });
    }
    // A control character (Ctrl+letter on some platforms) is its letter.
    let c = match c as u32 {
        code @ 1..=26 if ctrl => char::from(b'a' + (code as u8) - 1),
        _ if c.is_control() => return None,
        _ => c,
    };
    Some(Key::from_char(c, ctrl, alt, shift, meta))
}

/// The action of a key in `scope` (`window` or `composer`). The window
/// takes only keys with Ctrl, Alt or Meta, or function keys, so typing and
/// list keys reach the focused control.
pub fn resolve(
    map: &Keymap,
    scope: &str,
    text: &str,
    mods: Mods,
    busy: bool,
) -> Option<&'static str> {
    let key = key_of(text, mods)?;
    let function =
        key.name.len() > 1 && key.name.starts_with('f') && key.name[1..].parse::<u8>().is_ok();
    if scope == "window" && !(key.ctrl || key.alt || key.meta || function) {
        return None;
    }
    map.lookup(scope, &key, busy)
}

pub fn connect(ui: &MainWindow) {
    ui.on_key_action(|scope, text, ctrl, alt, shift, meta, busy| {
        let mods = [ctrl, alt, shift, meta];
        let name = with_app(|app, _| resolve(&app.rt.keys.gui, &scope, &text, mods, busy));
        name.flatten().unwrap_or_default().into()
    });
}

/// The composer hint shows the keys bound to send and queue.
pub fn show(map: &Keymap, ui: &MainWindow) {
    ui.set_send_key(map.label("send").unwrap_or_else(|| "Send".into()).into());
    ui.set_queue_key(
        map.label("queue")
            .unwrap_or_else(|| "(unbound)".into())
            .into(),
    );
}

#[cfg(test)]
mod tests {
    use fast_harness_core::config::layers::Source;
    use fast_harness_core::keys::Front;

    use super::*;

    #[test]
    fn slint_keys_resolve_through_the_keymap() {
        let mut map = Keymap::defaults(Front::Gui);
        let enter = String::from(char::from(SlintKey::Return));
        let none = [false; 4];
        assert_eq!(resolve(&map, "composer", &enter, none, false), Some("send"));
        assert_eq!(resolve(&map, "composer", &enter, none, true), Some("steer"));
        let alt = [false, true, false, false];
        assert_eq!(resolve(&map, "composer", &enter, alt, true), Some("queue"));
        assert_eq!(
            resolve(&map, "composer", &enter, [false, false, true, false], false),
            None
        );
        let ctrl = [true, false, false, false];
        assert_eq!(
            resolve(&map, "window", "n", ctrl, false),
            Some("new_session")
        );
        assert_eq!(
            resolve(&map, "window", "\u{e}", ctrl, false),
            Some("new_session")
        );
        assert_eq!(resolve(&map, "window", "n", none, false), None);
        let f3 = String::from(char::from(SlintKey::F3));
        assert_eq!(
            resolve(&map, "window", &f3, none, false),
            Some("open_trace")
        );
        let section = serde_json::json!({"new_session": "ctrl+shift+n", "send": "ctrl+enter"});
        assert!(map.apply(&section, Source::User).is_empty());
        assert_eq!(resolve(&map, "window", "n", ctrl, false), None);
        assert_eq!(
            resolve(&map, "window", "N", ctrl, false),
            Some("new_session")
        );
        assert_eq!(resolve(&map, "composer", &enter, none, false), None);
        assert_eq!(resolve(&map, "composer", &enter, ctrl, false), Some("send"));
    }
}
