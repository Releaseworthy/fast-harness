//! Named key bindings of the TUI and the GUI (PLAN.md D45): every action a
//! key starts has a name, a default key per front end, and `keys.json` in
//! the user and trusted project folders rebinds or unbinds it without a
//! rebuild. No UI types here: each front end turns its own key event into a
//! [`Key`] and asks its [`Keymap`] for the action.
//!
//! Key strings: modifiers `ctrl`, `alt`, `shift`, `meta` joined with `+`
//! before one key name: a printable character (`k`, `,`, `/`), a named key
//! (`enter`, `esc`, `tab`, `backspace`, `delete`, `insert`, `space`, `up`,
//! `down`, `left`, `right`, `home`, `end`, `pageup`, `pagedown`) or `f1` to
//! `f24`. `ctrl+shift+k`, `alt+enter`, `f2`.

use std::fmt;

mod actions;
mod load;
mod map;
#[cfg(test)]
mod tests;

pub use actions::{ActionDef, Front, When, GUI_ACTIONS, TUI_ACTIONS};
pub use load::{load, Keys, FILE_NAME};
pub use map::{Binding, Conflict, Keymap};

/// The named keys (canonical spelling).
const NAMED: [&str; 15] = [
    "enter",
    "esc",
    "tab",
    "backspace",
    "delete",
    "insert",
    "space",
    "up",
    "down",
    "left",
    "right",
    "home",
    "end",
    "pageup",
    "pagedown",
];

/// One key with its modifiers; `name` is canonical (lower case).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
    pub name: String,
}

impl Key {
    /// A key without modifiers; `name` must already be canonical.
    pub fn plain(name: &str) -> Key {
        Key {
            ctrl: false,
            alt: false,
            shift: false,
            meta: false,
            name: name.to_string(),
        }
    }

    /// The key a printable character makes: an upper case letter becomes
    /// `shift+<letter>`; shift on other characters is dropped (it is part
    /// of the character).
    pub fn from_char(c: char, ctrl: bool, alt: bool, shift: bool, meta: bool) -> Key {
        let name = match c {
            ' ' => "space".to_string(),
            c => c.to_lowercase().collect(),
        };
        let shift = match c {
            c if c.is_uppercase() => true,
            c if c.is_alphabetic() || c == ' ' => shift,
            _ => false,
        };
        Key {
            ctrl,
            alt,
            shift,
            meta,
            name,
        }
    }

    /// No Ctrl, Alt or Meta: on a text field this key types.
    pub fn is_text(&self) -> bool {
        !self.ctrl && !self.alt && !self.meta && self.name.chars().count() == 1
    }

    /// `Ctrl+Shift+K`, `Alt+Enter`, `PgUp`, `l` for hints and the GUI.
    pub fn label(&self) -> String {
        let mut out = String::new();
        let mods = [
            (self.ctrl, "Ctrl+"),
            (self.alt, "Alt+"),
            (self.shift, "Shift+"),
            (self.meta, "Meta+"),
        ];
        for (on, word) in mods {
            if on {
                out.push_str(word);
            }
        }
        let modified = !out.is_empty();
        out.push_str(&name_label(&self.name, modified));
        out
    }
}

fn name_label(name: &str, modified: bool) -> String {
    match name {
        "pageup" => "PgUp".into(),
        "pagedown" => "PgDn".into(),
        "esc" => "Esc".into(),
        _ if name.chars().count() == 1 && !modified => name.into(),
        _ if name.chars().count() == 1 => name.to_uppercase(),
        _ => {
            let mut chars = name.chars();
            let first = chars.next().map(|c| c.to_uppercase().to_string());
            first.unwrap_or_default() + chars.as_str()
        }
    }
}

impl fmt::Display for Key {
    /// The canonical string: `ctrl+alt+shift+meta+<name>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mods = [
            (self.ctrl, "ctrl+"),
            (self.alt, "alt+"),
            (self.shift, "shift+"),
            (self.meta, "meta+"),
        ];
        for (on, word) in mods {
            if on {
                f.write_str(word)?;
            }
        }
        f.write_str(&self.name)
    }
}

impl std::str::FromStr for Key {
    type Err = String;

    /// Parses `ctrl+shift+k`; case and spaces around parts do not matter.
    fn from_str(text: &str) -> Result<Key, String> {
        let text = text.trim();
        // A trailing `+` is the plus key itself (`ctrl++`).
        let (mods, last) = match text.strip_suffix("++") {
            Some(rest) => (rest, "+"),
            None if text == "+" => ("", "+"),
            None => match text.rsplit_once('+') {
                Some((mods, last)) => (mods, last),
                None => ("", text),
            },
        };
        let mut key = Key::plain(&key_name(last)?);
        for part in mods.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            let slot = match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => &mut key.ctrl,
                "alt" | "option" => &mut key.alt,
                "shift" => &mut key.shift,
                "meta" | "cmd" | "super" | "win" => &mut key.meta,
                other => return Err(format!("unknown modifier `{other}` in `{text}`")),
            };
            if *slot {
                return Err(format!("`{part}` twice in `{text}`"));
            }
            *slot = true;
        }
        Ok(key)
    }
}

/// The canonical name of one key part, or why it is not a key.
fn key_name(part: &str) -> Result<String, String> {
    let part = part.trim();
    let lower = part.to_lowercase();
    let alias = match lower.as_str() {
        "return" => "enter",
        "escape" => "esc",
        "del" => "delete",
        "ins" => "insert",
        "pgup" => "pageup",
        "pgdn" | "pgdown" => "pagedown",
        "comma" => ",",
        "plus" => "+",
        "minus" => "-",
        other => other,
    };
    if NAMED.contains(&alias) || alias.chars().count() == 1 && !alias.trim().is_empty() {
        return Ok(alias.to_string());
    }
    let function = alias.strip_prefix('f').and_then(|n| n.parse::<u8>().ok());
    match function {
        Some(1..=24) => Ok(alias.to_string()),
        _ if part.is_empty() => Err("empty key".into()),
        _ => Err(format!("unknown key `{part}`")),
    }
}
