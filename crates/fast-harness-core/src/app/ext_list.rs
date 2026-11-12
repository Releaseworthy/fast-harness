//! `fast-harness ext list [--json]` (phase 29): every extension file a
//! session in a folder would see, with its kind, name, source, state and
//! `version`. Built from [`Settings`] only: no MCP server starts, no hook
//! or panel runs.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::Settings;
use crate::config::ext_file::{self, DISABLED};
use crate::config::layers::{LayerFile, Source, AGENTS_FILE};

/// Folders whose files are one extension each, named by their folder.
const FOLDER_KINDS: [&str; 5] = ["tools", "agents", "commands", "providers", "panels"];

fn source_word(source: Source) -> &'static str {
    match source {
        Source::User => "home",
        Source::Project => "project",
        _ => "built-in",
    }
}

/// `(kind, name)` of an extension path.
fn kind_name(path: &Path) -> (String, String) {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let parent = path.parent().unwrap_or(path);
    let folder = parent.file_name().unwrap_or_default().to_string_lossy();
    let file = path.file_name().unwrap_or_default().to_string_lossy();
    if file == "SKILL.md" {
        return ("skills".into(), folder.into_owned());
    }
    if file == AGENTS_FILE {
        return ("agents-md".into(), path.display().to_string());
    }
    if FOLDER_KINDS.contains(&folder.as_ref()) {
        return (folder.into_owned(), stem.into_owned());
    }
    (stem.clone().into_owned(), stem.into_owned())
}

fn entry(kind: &str, name: &str, path: &Path, source: &str, state: &str) -> Value {
    json!({"kind": kind, "name": name, "source": source, "path": path.display().to_string(),
        "state": state, "version": ext_file::version_of(path)})
}

/// The `id` of a provider or panel file (its name), else the file stem.
fn file_id(path: &Path, stem: String) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    value["id"].as_str().map_or(stem, str::to_string)
}

/// The files of `report.loaded`; a provider or panel replaced by a later
/// file with the same id is `shadowed` by that file.
fn loaded(files: &[LayerFile], out: &mut Vec<Value>) {
    let named: Vec<(String, String)> = (files.iter())
        .map(|file| {
            let (kind, stem) = kind_name(&file.path);
            let name = match kind.as_str() {
                "providers" | "panels" => file_id(&file.path, stem),
                _ => stem,
            };
            (kind, name)
        })
        .collect();
    for (at, file) in files.iter().enumerate() {
        let (kind, name) = &named[at];
        let source = source_word(file.source);
        let mut item = entry(kind, name, &file.path, source, "loaded");
        let later = (at + 1..files.len()).rev().find(|&j| named[j] == named[at]);
        if let Some(j) = later.filter(|_| matches!(kind.as_str(), "providers" | "panels")) {
            item["state"] = json!("shadowed");
            item["by"] = json!(files[j].path.display().to_string());
        }
        out.push(item);
    }
}

fn path_source(home: &Path, path: &Path) -> &'static str {
    match path.starts_with(home) {
        true => "home",
        false => "project",
    }
}

/// Every extension file of `settings`, as the JSON list of `ext list --json`.
pub fn ext_list(settings: &Settings) -> Value {
    let (index, report) = (&settings.index, &settings.layered.report);
    let mut out = Vec::new();
    let kinds = [
        ("tools", &index.tools),
        ("agents", &index.agents),
        ("commands", &index.commands),
        ("skills", &index.skills),
    ];
    for (kind, items) in kinds {
        for item in items {
            let state = if item.entry.disabled {
                "disabled"
            } else {
                "loaded"
            };
            let source = source_word(item.source);
            out.push(entry(kind, &item.entry.name, &item.path, source, state));
        }
    }
    for (kind, item, by) in &index.shadowed {
        let mut value = entry(
            kind,
            &item.entry.name,
            &item.path,
            source_word(item.source),
            "shadowed",
        );
        value["by"] = json!(by.display().to_string());
        out.push(value);
    }
    loaded(&report.loaded, &mut out);
    skipped(settings, &mut out);
    Value::Array(out)
}

/// The skipped and disabled files, and the untrusted `AGENTS.md` files.
fn skipped(settings: &Settings, out: &mut Vec<Value>) {
    let (report, home) = (&settings.layered.report, &settings.layered.layers.home);
    for skipped in &report.skipped {
        let (kind, name) = kind_name(&skipped.path);
        let source = path_source(home, &skipped.path);
        let state = if skipped.reason == DISABLED {
            "disabled"
        } else {
            "skipped"
        };
        let mut value = entry(&kind, &name, &skipped.path, source, state);
        value["reason"] = json!(skipped.reason);
        out.push(value);
    }
    let agents: &[PathBuf] = &settings.layered.layers.agents_skipped;
    for path in agents {
        let mut value = entry(
            "agents-md",
            &path.display().to_string(),
            path,
            "project",
            "skipped",
        );
        value["reason"] = json!("project not trusted (`fast-harness trust`)");
        out.push(value);
    }
}

/// The plain text form: one line per file.
pub fn ext_lines(list: &Value) -> String {
    let mut out = String::new();
    for item in list.as_array().into_iter().flatten() {
        let text = |key: &str| item[key].as_str().unwrap_or_default().to_string();
        let mut line = format!(
            "{} {} [{}] ({}, v{}) {}",
            text("kind"),
            text("name"),
            text("state"),
            text("source"),
            item["version"],
            text("path")
        );
        for key in ["by", "reason"] {
            if let Some(value) = item[key].as_str() {
                line.push_str(&format!("; {key}: {value}"));
            }
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}
