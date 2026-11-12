//! `keys.json` of the user folder, then of the trusted project folder:
//! `{"version": 1, "tui": {"<action>": "<key>" | ["<key>", ...] | null},
//! "gui": {...}}`. A file that is not JSON is skipped; a bad key or an
//! unknown action skips that entry only, with a problem line.

use std::io;

use serde_json::Value;

use super::actions::Front;
use super::map::Keymap;
use crate::config::layers::{LayerFile, Layers, Skipped};

pub const FILE_NAME: &str = "keys.json";

/// The effective keymaps, the files they came from and what was skipped.
#[derive(Clone, Debug)]
pub struct Keys {
    pub tui: Keymap,
    pub gui: Keymap,
    pub files: Vec<LayerFile>,
    /// One line per skipped entry or conflict (doctor and a note).
    pub problems: Vec<String>,
}

impl Default for Keys {
    fn default() -> Keys {
        Keys {
            tui: Keymap::defaults(Front::Tui),
            gui: Keymap::defaults(Front::Gui),
            files: Vec::new(),
            problems: Vec::new(),
        }
    }
}

impl Keys {
    pub fn front(&self, front: Front) -> &Keymap {
        match front {
            Front::Tui => &self.tui,
            Front::Gui => &self.gui,
        }
    }

    /// `fast-harness keys`: every action of both front ends with its keys,
    /// layer, scope and state, then the files and the problems.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for map in [&self.tui, &self.gui] {
            out.push_str(&format!("{}:\n", map.front.word()));
            for (name, keys, source) in map.rows() {
                let def = map.def(name);
                let (scope, when) = def.map_or(("", ""), |d| (d.scope, d.when.word()));
                let about = def.map_or("", |d| d.about);
                out.push_str(&format!(
                    "  {name:<24} {keys:<36} {:<8} {scope}, {when}: {about}\n",
                    source.word()
                ));
            }
        }
        for file in &self.files {
            out.push_str(&format!(
                "file ({}): {}\n",
                file.source.word(),
                file.path.display()
            ));
        }
        for problem in &self.problems {
            out.push_str(&format!("problem: {problem}\n"));
        }
        out
    }

    /// `fast-harness keys --json`.
    pub fn json(&self) -> Value {
        let section = |map: &super::Keymap| -> Vec<Value> {
            (map.rows().into_iter())
                .map(|(name, _, source)| {
                    let keys: Vec<String> = map.keys(name).iter().map(|k| k.to_string()).collect();
                    let def = map.def(name);
                    serde_json::json!({
                        "action": name,
                        "keys": keys,
                        "source": source.word(),
                        "scope": def.map(|d| d.scope),
                        "when": def.map(|d| d.when.word()),
                        "about": def.map(|d| d.about),
                    })
                })
                .collect()
        };
        let files: Vec<Value> = (self.files.iter())
            .map(|f| serde_json::json!({"path": f.path.display().to_string(), "source": f.source.word()}))
            .collect();
        serde_json::json!({
            "version": 1,
            "tui": section(&self.tui),
            "gui": section(&self.gui),
            "files": files,
            "problems": self.problems,
        })
    }

    /// The problems as one note; `None` when there are none.
    pub fn note(&self) -> Option<String> {
        (!self.problems.is_empty()).then(|| format!("keys.json: {}", self.problems.join("; ")))
    }
}

/// Reads `keys.json` of every layer folder of `layers`.
pub fn load(layers: &Layers) -> (Keys, Vec<Skipped>) {
    let mut keys = Keys::default();
    let mut skipped = Vec::new();
    for (dir, source) in layers.dirs() {
        let path = dir.join(FILE_NAME);
        let value = match read(&path) {
            Ok(None) => continue,
            Ok(Some(value)) => value,
            Err(reason) => {
                skipped.push(Skipped { path, reason });
                continue;
            }
        };
        let place = path.display().to_string();
        for (field, child) in value.as_object().into_iter().flatten() {
            let map = match field.as_str() {
                "tui" => &mut keys.tui,
                "gui" => &mut keys.gui,
                "version" | "enabled" | "$schema" => continue,
                other => {
                    keys.problems
                        .push(format!("{place}: unknown section `{other}`"));
                    continue;
                }
            };
            let problems = map.apply(child, source);
            keys.problems
                .extend(problems.into_iter().map(|p| format!("{place}: {p}")));
        }
        keys.files.push(LayerFile { path, source });
    }
    for map in [&keys.tui, &keys.gui] {
        for c in map.conflicts() {
            let front = map.front.word();
            let line = format!(
                "{front}: `{}` starts both {} and {}",
                c.key, c.first, c.second
            );
            keys.problems.push(line);
        }
    }
    (keys, skipped)
}

/// The file as a JSON object with `version` 1; `None` when missing.
fn read(path: &std::path::Path) -> Result<Option<Value>, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.to_string()),
    };
    let value: Value = serde_json::from_slice(&bytes).map_err(|err| err.to_string())?;
    if !value.is_object() {
        return Err("not a JSON object".into());
    }
    crate::config::ext_file::on(crate::config::ext_file::json_header(&value)?)?;
    Ok(Some(value))
}
