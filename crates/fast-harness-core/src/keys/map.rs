//! The effective bindings of one front end: defaults, then each
//! `keys.json` section on top (later layer wins per action).

use std::collections::BTreeMap;

use serde_json::Value;

use super::actions::{ActionDef, Front, When};
use super::Key;
use crate::config::layers::Source;

/// The keys of one action and the layer that set them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub keys: Vec<Key>,
    pub source: Source,
}

/// Two actions that one key would both start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    pub key: Key,
    pub first: &'static str,
    pub second: &'static str,
}

/// The bindings of one front end, by action name.
#[derive(Clone, Debug)]
pub struct Keymap {
    pub front: Front,
    bindings: BTreeMap<&'static str, Binding>,
}

/// Whether one key can reach actions of both scopes.
fn scopes_overlap(a: &str, b: &str) -> bool {
    let screen = |s: &str| matches!(s, "screens" | "accounts" | "trace" | "models");
    // The approval prompt takes its keys before the message box (D64).
    let typing = |s: &str| matches!(s, "chat" | "composer");
    a == b
        || matches!(a, "global" | "window")
        || matches!(b, "global" | "window")
        || (a == "screens" && screen(b))
        || (b == "screens" && screen(a))
        || (a == "approval" && typing(b))
        || (b == "approval" && typing(a))
}

/// Whether a lookup in `scope` reaches actions of `action_scope`.
fn in_scope(scope: &str, action_scope: &str) -> bool {
    let typing = matches!(scope, "chat" | "composer" | "approval");
    scope == action_scope || (action_scope == "screens" && !typing)
}

impl Keymap {
    /// Every action of `front` on its default keys.
    pub fn defaults(front: Front) -> Keymap {
        let bindings = (front.actions().iter())
            .map(|def| {
                let keys = def.keys.iter().filter_map(|k| k.parse().ok()).collect();
                let binding = Binding {
                    keys,
                    source: Source::Default,
                };
                (def.name, binding)
            })
            .collect();
        Keymap { front, bindings }
    }

    pub fn def(&self, name: &str) -> Option<&'static ActionDef> {
        self.front.actions().iter().find(|def| def.name == name)
    }

    pub fn binding(&self, name: &str) -> Option<&Binding> {
        self.bindings.get(name)
    }

    /// The keys of `name`; empty when unbound or unknown.
    pub fn keys(&self, name: &str) -> &[Key] {
        self.bindings.get(name).map_or(&[], |b| b.keys.as_slice())
    }

    /// The label of the first key of `name` (`Ctrl+P`); `None` when unbound.
    pub fn label(&self, name: &str) -> Option<String> {
        self.keys(name).first().map(Key::label)
    }

    /// Every label of `name` joined with `/` (`F2/Ctrl+A`).
    pub fn labels(&self, name: &str) -> String {
        let labels: Vec<String> = self.keys(name).iter().map(Key::label).collect();
        labels.join("/")
    }

    /// The action `key` starts in `scope` (with `global` or `window` first
    /// when `scope` is a page scope). An action for the current state wins
    /// over an `Always` action on the same key.
    pub fn lookup(&self, scope: &str, key: &Key, running: bool) -> Option<&'static str> {
        let found = |def: &&ActionDef| {
            in_scope(scope, def.scope)
                && def.when.applies(running)
                && self.keys(def.name).contains(key)
        };
        let actions = self.front.actions();
        let specific = actions
            .iter()
            .filter(|d| d.when != When::Always)
            .find(found);
        specific
            .or_else(|| actions.iter().find(found))
            .map(|def| def.name)
    }

    /// Applies one `keys.json` section; returns a problem per skipped entry.
    pub fn apply(&mut self, section: &Value, source: Source) -> Vec<String> {
        let front = self.front.word();
        let Some(object) = section.as_object() else {
            return vec![format!("`{front}` is not an object")];
        };
        let mut problems = Vec::new();
        for (name, value) in object {
            let Some(def) = self.def(name) else {
                problems.push(format!("{front}: unknown action `{name}`"));
                continue;
            };
            match parse_keys(value) {
                Ok(keys) => {
                    self.bindings.insert(def.name, Binding { keys, source });
                }
                Err(reason) => problems.push(format!("{front}.{name}: {reason}")),
            }
        }
        problems
    }

    /// Every pair of actions one key would both start in the same state.
    pub fn conflicts(&self) -> Vec<Conflict> {
        let defs = self.front.actions();
        let mut out = Vec::new();
        for (i, a) in defs.iter().enumerate() {
            for b in &defs[i + 1..] {
                if !scopes_overlap(a.scope, b.scope) || a.when != b.when {
                    continue;
                }
                for key in self.keys(a.name) {
                    if self.keys(b.name).contains(key) {
                        let (first, second) = (a.name, b.name);
                        out.push(Conflict {
                            key: key.clone(),
                            first,
                            second,
                        });
                    }
                }
            }
        }
        out
    }

    /// `(action, keys, source)` in action order.
    pub fn rows(&self) -> Vec<(&'static str, String, Source)> {
        (self.front.actions().iter())
            .map(|def| {
                let binding = self.bindings.get(def.name);
                let keys: Vec<String> = self.keys(def.name).iter().map(Key::to_string).collect();
                let keys = match keys.is_empty() {
                    true => "(unbound)".to_string(),
                    false => keys.join(", "),
                };
                (
                    def.name,
                    keys,
                    binding.map_or(Source::Default, |b| b.source),
                )
            })
            .collect()
    }
}

/// `"<key>"`, `["<key>", ...]` or `null` (unbound).
fn parse_keys(value: &Value) -> Result<Vec<Key>, String> {
    match value {
        Value::Null => Ok(Vec::new()),
        Value::String(text) => Ok(vec![text.parse()?]),
        Value::Array(items) => items
            .iter()
            .map(|item| match item.as_str() {
                Some(text) => text.parse(),
                None => Err("a key must be a string".to_string()),
            })
            .collect(),
        _ => Err("expected a key, a list of keys or null".into()),
    }
}
