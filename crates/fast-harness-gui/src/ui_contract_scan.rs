//! Regenerates the UI contract from the sources: the `.slint` declarations of
//! `MainWindow` and the exported globals, kept when the Rust side calls their
//! `set_`, `get_`, `on_` or `invoke_` accessor (a global only when the Rust
//! side names it in `global::<Name>`). `UPDATE_UI_CONTRACT=1` rewrites the
//! list in `docs/ui-contract.md`.

use std::collections::BTreeSet;
use std::path::Path;

use super::{parse, Entry, Kind, DOC};
use crate::ui_files::FILES;

const START: &str = "<!-- contract:start -->";
const END: &str = "<!-- contract:end -->";

/// The members a block declares at its first level.
fn members(owner: &str, body: &str, global: bool) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    for line in body.lines() {
        if depth == 1 {
            if let Some((name, kind, ty)) = member(line.trim(), global) {
                let owner = owner.to_string();
                out.push(Entry { owner, name, kind, ty });
            }
        }
        depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;
        if depth <= 0 && line.contains('}') {
            break;
        }
    }
    out
}

fn member(line: &str, global: bool) -> Option<(String, Kind, String)> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let first = *words.first()?;
    let public = matches!(first, "in" | "out" | "in-out") || (global && first == "property");
    if public && line.contains("property <") {
        let (_, after) = line.split_once("property <")?;
        let (ty, rest) = after.split_once('>')?;
        return Some((name_of(rest), Kind::Property, ty.to_string()));
    }
    let rest = line.strip_prefix("pure ").unwrap_or(line);
    if let Some(rest) = rest.strip_prefix("callback ") {
        return Some((name_of(rest), Kind::Callback, String::new()));
    }
    let rest = line.strip_prefix("public ")?;
    let rest = rest.strip_prefix("pure ").unwrap_or(rest);
    let rest = rest.strip_prefix("function ")?;
    Some((name_of(rest), Kind::Function, String::new()))
}

fn name_of(rest: &str) -> String {
    let rest = rest.trim_start();
    let end = rest
        .find(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'))
        .unwrap_or(rest.len());
    rest[..end].to_string()
}

/// `MainWindow` and every exported global of the built-in UI.
fn declared() -> Vec<Entry> {
    let mut out = Vec::new();
    for (_, text) in FILES {
        for (at, _) in text.match_indices("export global ") {
            let owner = name_of(&text[at + "export global ".len()..]);
            out.extend(members(&owner, &text[at..], true));
        }
        if let Some(at) = text.find("export component MainWindow ") {
            out.extend(members("MainWindow", &text[at..], false));
        }
    }
    out
}

/// The accessor names and the global names the Rust sources use.
fn used() -> (BTreeSet<String>, BTreeSet<String>) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let (mut names, mut globals) = (BTreeSet::new(), BTreeSet::new());
    for entry in std::fs::read_dir(src).unwrap().flatten() {
        let path = entry.path();
        let file = path.file_name().unwrap().to_string_lossy().to_string();
        if !file.ends_with(".rs") || file.starts_with("ui_contract") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for prefix in [".set_", ".get_", ".on_", ".invoke_"] {
            for (at, _) in text.match_indices(prefix) {
                let name = name_of(&text[at + prefix.len()..]);
                if text[at + prefix.len() + name.len()..].starts_with('(') {
                    names.insert(name.replace('_', "-"));
                }
            }
        }
        for (at, _) in text.match_indices("global::<") {
            globals.insert(name_of(&text[at + "global::<".len()..]));
        }
    }
    (names, globals)
}

/// The contract the sources define, sorted by owner and name.
fn generate() -> Vec<Entry> {
    let (names, globals) = used();
    let mut out: Vec<Entry> = declared()
        .into_iter()
        .filter(|e| e.owner == "MainWindow" || globals.contains(&e.owner))
        .filter(|e| names.contains(&e.name))
        .collect();
    out.sort_by(|a, b| (&a.owner, &a.name).cmp(&(&b.owner, &b.name)));
    out.dedup();
    out
}

#[test]
fn the_contract_document_matches_the_sources() {
    let expected = generate();
    assert!(expected.len() > 100, "the scan found {}", expected.len());
    let lines: Vec<String> = expected.iter().map(Entry::line).collect();
    if std::env::var_os("UPDATE_UI_CONTRACT").is_some() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/ui-contract.md");
        let (head, rest) = DOC.split_once(START).unwrap();
        let (_, tail) = rest.split_once(END).unwrap();
        let list = lines.join("\n");
        std::fs::write(path, format!("{head}{START}\n{list}\n{END}{tail}")).unwrap();
        return;
    }
    let documented: Vec<String> = parse(DOC).iter().map(Entry::line).collect();
    assert_eq!(
        documented, lines,
        "docs/ui-contract.md is stale: run the test with UPDATE_UI_CONTRACT=1"
    );
}

#[test]
fn every_entry_is_declared_in_the_built_in_ui() {
    let declared = declared();
    for entry in parse(DOC) {
        assert!(declared.contains(&entry), "{} is not declared", entry.line());
    }
    assert!(parse(DOC).iter().any(|e| e.name == "reload-ui"));
}
