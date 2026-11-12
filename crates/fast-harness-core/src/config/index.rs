//! The extension index (PLAN.md D42): at session creation the `tools/*.json`,
//! `agents/*.md` and `commands/*.md` folders of every layer are listed and
//! only names and descriptions are read, cached in `<home>/index.json` by
//! path, size and modified time. Bodies (schemas, prompt bodies) are read on
//! use ([`read_md`], `tools::command`). No folder, no work: when no kind has
//! a file, `index.json` is not even opened.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::layers::{kind_files, skill_files, LayerFile, Layers, Skipped, Source};

/// The cache file in the user folder.
pub const INDEX_FILE: &str = "index.json";
/// 2: `Entry::effect` (PLAN.md D54).
const VERSION: u32 = 2;
pub const TOOLS: &str = "tools";
pub const AGENTS: &str = "agents";
pub const COMMANDS: &str = "commands";
/// `skills/<name>/SKILL.md` (Agent Skills, PLAN.md D43).
pub const SKILLS: &str = "skills";
/// Longest skill description kept (the Agent Skills limit).
pub const SKILL_DESCRIPTION_CHARS: usize = 1024;
const KINDS: [(&str, &str); 3] = [(TOOLS, "json"), (AGENTS, "md"), (COMMANDS, "md")];

/// What the index keeps of one file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    pub description: String,
    /// A tool file that names a `command` (a command tool); without one it
    /// overrides the built-in tool of the same name.
    #[serde(skip_serializing_if = "is_false")]
    pub command: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_bytes: Option<usize>,
    #[serde(skip_serializing_if = "is_false")]
    pub disabled: bool,
    /// A tool file's `effect`: `read`, `write` or `run` (PLAN.md D54).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
}

/// The words a tool file's `effect` may hold (`tools::Effect`).
const EFFECTS: [&str; 3] = ["read", "write", "run"];

fn is_false(value: &bool) -> bool {
    !value
}

/// One indexed file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub entry: Entry,
    pub path: PathBuf,
    pub source: Source,
}

/// The extension files of one set of layers. A later layer's file replaces
/// an earlier one of the same name.
#[derive(Clone, Debug, Default)]
pub struct ExtIndex {
    pub tools: Vec<Item>,
    pub agents: Vec<Item>,
    pub commands: Vec<Item>,
    /// Skills; `path` is the `SKILL.md` file.
    pub skills: Vec<Item>,
    pub skipped: Vec<Skipped>,
    /// Files read to fill the index (cache misses).
    pub reads: usize,
    /// Files taken from `index.json` without reading them.
    pub cached: usize,
}

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
struct Cache {
    version: u32,
    files: BTreeMap<String, Cached>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Cached {
    size: u64,
    modified: u64,
    #[serde(flatten)]
    entry: Entry,
}

impl ExtIndex {
    /// Lists the folders of `layers`; reads a file only when `index.json`
    /// has no entry with its size and modified time, and rewrites the cache
    /// only when it changed.
    pub fn build(layers: &Layers) -> ExtIndex {
        let mut lists: Vec<(&str, Vec<LayerFile>)> = KINDS
            .iter()
            .map(|(kind, ext)| (*kind, kind_files(layers, kind, ext)))
            .collect();
        lists.push((SKILLS, skill_files(layers)));
        let mut index = ExtIndex::default();
        if lists.iter().all(|(_, files)| files.is_empty()) {
            return index;
        }
        let path = layers.home.join(INDEX_FILE);
        let old = read_cache(&path);
        let mut new = Cache {
            version: VERSION,
            files: BTreeMap::new(),
        };
        for (kind, files) in &lists {
            for file in files {
                index.add(kind, file, &old, &mut new);
            }
        }
        if new != old {
            if let Ok(text) = serde_json::to_string_pretty(&new) {
                let _ = std::fs::write(&path, text);
            }
        }
        index
    }

    fn add(&mut self, kind: &str, file: &LayerFile, old: &Cache, new: &mut Cache) {
        let key = file.path.display().to_string();
        let Some((size, modified)) = stamp(&file.path) else {
            return;
        };
        let entry = match old.files.get(&key) {
            Some(hit) if hit.size == size && hit.modified == modified => {
                self.cached += 1;
                hit.entry.clone()
            }
            _ => {
                self.reads += 1;
                match parse(kind, &file.path) {
                    Ok(entry) => entry,
                    Err(reason) => {
                        let path = file.path.clone();
                        return self.skipped.push(Skipped { path, reason });
                    }
                }
            }
        };
        let cached = Cached {
            size,
            modified,
            entry: entry.clone(),
        };
        new.files.insert(key, cached);
        let list = match kind {
            TOOLS => &mut self.tools,
            AGENTS => &mut self.agents,
            SKILLS => &mut self.skills,
            _ => &mut self.commands,
        };
        list.retain(|item| item.entry.name != entry.name);
        let (path, source) = (file.path.clone(), file.source);
        list.push(Item {
            entry,
            path,
            source,
        });
    }

    /// The agent type or command named `name`.
    pub fn agent(&self, name: &str) -> Option<&Item> {
        self.agents.iter().find(|item| item.entry.name == name)
    }

    pub fn command(&self, name: &str) -> Option<&Item> {
        self.commands.iter().find(|item| item.entry.name == name)
    }
}

fn read_cache(path: &Path) -> Cache {
    let parsed = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Cache>(&bytes).ok());
    parsed
        .filter(|cache| cache.version == VERSION)
        .unwrap_or_default()
}

/// Size and modified time (nanoseconds since 1970) of `path`.
fn stamp(path: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((meta.len(), modified.as_nanos() as u64))
}

fn parse(kind: &str, path: &Path) -> Result<Entry, String> {
    let text = std::fs::read_to_string(path).map_err(|err| err.to_string())?;
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    match kind {
        TOOLS => parse_tool(&text, &stem),
        SKILLS => parse_skill(&text, path),
        _ => {
            let (meta, _) = front_matter(&text);
            let name = meta.get("name").map_or(&*stem, String::as_str);
            check_name(name)?;
            Ok(Entry {
                name: name.to_string(),
                description: meta.get("description").cloned().unwrap_or_default(),
                ..Entry::default()
            })
        }
    }
}

fn parse_tool(text: &str, stem: &str) -> Result<Entry, String> {
    let value: Value = serde_json::from_str(text).map_err(|err| err.to_string())?;
    let object = value.as_object().ok_or("not a JSON object")?;
    let text_of = |key: &str| object.get(key).and_then(Value::as_str).map(str::trim);
    let name = text_of("name").unwrap_or(stem);
    check_name(name)?;
    let command = text_of("command").is_some_and(|c| !c.is_empty());
    let description = text_of("description").unwrap_or_default();
    if command && description.is_empty() {
        return Err("a command tool needs a `description`".into());
    }
    let output_bytes = object.get("outputBytes").and_then(Value::as_u64);
    let effect = match object.get("effect") {
        None => None,
        Some(Value::String(word)) if EFFECTS.contains(&word.as_str()) => Some(word.clone()),
        Some(other) => return Err(format!("bad `effect` {other}: use read, write or run")),
    };
    Ok(Entry {
        name: name.to_string(),
        description: description.to_string(),
        command,
        output_bytes: output_bytes.map(|bytes| bytes as usize),
        disabled: object.get("disabled").and_then(Value::as_bool) == Some(true),
        effect,
    })
}

/// A `SKILL.md`: `name` (else the folder name) and a `description` (needed,
/// cut at [`SKILL_DESCRIPTION_CHARS`]); the body is read on use.
fn parse_skill(text: &str, path: &Path) -> Result<Entry, String> {
    let folder = path.parent().and_then(Path::file_name).unwrap_or_default();
    let folder = folder.to_string_lossy();
    let (meta, _) = front_matter(text);
    let name = meta
        .get("name")
        .map(String::as_str)
        .filter(|name| !name.is_empty())
        .unwrap_or(&folder);
    check_name(name)?;
    let description = meta.get("description").map_or("", |d| d.trim());
    if description.is_empty() {
        return Err("a skill needs a `description`".into());
    }
    Ok(Entry {
        name: name.to_string(),
        description: description.chars().take(SKILL_DESCRIPTION_CHARS).collect(),
        ..Entry::default()
    })
}

/// A tool, agent or command name: 1 to 64 letters, digits, `_` or `-`.
pub fn check_name(name: &str) -> Result<(), String> {
    let ok = (1..=64).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    match ok {
        true => Ok(()),
        false => Err(format!(
            "bad name `{name}`: use 1 to 64 letters, digits, `_` or `-`"
        )),
    }
}

/// `key: value` lines between a first line `---` and the next `---`, and
/// the body after them. Without that block: no keys, the whole text.
pub fn front_matter(text: &str) -> (BTreeMap<String, String>, &str) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let none = (BTreeMap::new(), text);
    let Some(rest) = text.strip_prefix("---") else {
        return none;
    };
    if !rest.starts_with('\n') && !rest.starts_with("\r\n") {
        return none;
    }
    let mut meta = BTreeMap::new();
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        offset += line.len();
        let line = line.trim();
        if line == "---" {
            return (meta, &rest[offset..]);
        }
        if let Some((key, value)) = line.split_once(':') {
            meta.insert(key.trim().to_string(), unquote(value.trim()).to_string());
        }
    }
    none
}

fn unquote(value: &str) -> &str {
    for quote in ['"', '\''] {
        if value.len() >= 2 && value.starts_with(quote) && value.ends_with(quote) {
            return &value[1..value.len() - 1];
        }
    }
    value
}

/// An `agents/*.md` or `commands/*.md` file read on use.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MdFile {
    pub name: String,
    pub description: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub tools: Vec<String>,
    pub max_depth: Option<u32>,
    pub body: String,
}

pub fn read_md(path: &Path) -> Result<MdFile, String> {
    let text = std::fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
    let (meta, body) = front_matter(&text);
    let get = |key: &str| meta.get(key).filter(|v| !v.is_empty()).cloned();
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let max_depth = match get("maxDepth") {
        Some(depth) => Some(
            depth
                .parse()
                .map_err(|_| format!("bad maxDepth `{depth}`"))?,
        ),
        None => None,
    };
    Ok(MdFile {
        name: get("name").unwrap_or_else(|| stem.into_owned()),
        description: get("description").unwrap_or_default(),
        model: get("model"),
        effort: get("effort"),
        tools: get("tools").map(|t| list(&t)).unwrap_or_default(),
        max_depth,
        body: body.trim().to_string(),
    })
}

/// `[a, b]` or `a, b` as names.
fn list(value: &str) -> Vec<String> {
    let inner = value.trim().trim_start_matches('[').trim_end_matches(']');
    inner
        .split(',')
        .map(|name| unquote(name.trim()).trim().to_string())
        .filter(|name| !name.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_matter_and_lists() {
        let (meta, body) = front_matter("---\nname: x\ndescription: \"A b\"\n---\nBody\n");
        assert_eq!(meta["name"], "x");
        assert_eq!(meta["description"], "A b");
        assert_eq!(body, "Body\n");
        assert_eq!(front_matter("no block").1, "no block");
        assert_eq!(list("[read, 'grep']"), vec!["read", "grep"]);
        assert!(check_name("a b").is_err());
    }
}
