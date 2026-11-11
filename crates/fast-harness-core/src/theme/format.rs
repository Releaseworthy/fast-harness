//! The theme file format (version 1, `docs/themes.md`):
//! `{"version":1,"id","name","kind":"dark"|"light","extends"?,"colors":{role:"#rrggbb"}}`.
//! Built-in and user themes go through the same parser; `extends` starts
//! from another theme and changes only the roles listed.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Map, Value};

use super::{check, Palette, Rgb, Theme, ROLES};

/// The folder under the app home that holds user theme files.
pub const THEMES_DIR: &str = "themes";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Dark,
    Light,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThemeFile {
    version: u32,
    id: String,
    name: String,
    kind: Kind,
    #[serde(default)]
    extends: Option<String>,
    #[serde(default)]
    colors: Map<String, Value>,
}

/// One parsed theme file, its colours checked but not yet resolved.
#[derive(Clone, Debug)]
pub struct Doc {
    /// Where it came from, for messages: a path or `built-in <file>`.
    pub origin: String,
    pub file: Option<PathBuf>,
    pub id: String,
    pub name: String,
    pub dark: bool,
    pub extends: Option<String>,
    pub colors: Vec<(String, Rgb)>,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// Parses one theme document; every error names `origin` and the key.
pub fn parse(text: &str, origin: &str) -> Result<Doc, String> {
    let file: ThemeFile = serde_json::from_str(text).map_err(|err| format!("{origin}: {err}"))?;
    if file.version != 1 {
        return Err(format!(
            "{origin}: version: {} is not supported (expected 1)",
            file.version
        ));
    }
    if !valid_id(&file.id) {
        return Err(format!(
            "{origin}: id: \"{}\" must be lower-case letters, digits, '-' or '_'",
            file.id
        ));
    }
    if file.name.trim().is_empty() {
        return Err(format!("{origin}: name: must not be empty"));
    }
    let mut colors = Vec::new();
    for (key, value) in &file.colors {
        if !ROLES.contains(&key.as_str()) {
            let known = ROLES.join(", ");
            return Err(format!(
                "{origin}: colors.{key}: unknown role (roles: {known})"
            ));
        }
        let Some(c) = value.as_str().and_then(Rgb::parse) else {
            return Err(format!(
                "{origin}: colors.{key}: {value} is not a \"#rrggbb\" colour"
            ));
        };
        colors.push((key.clone(), c));
    }
    Ok(Doc {
        origin: origin.to_string(),
        file: None,
        id: file.id,
        name: file.name.trim().to_string(),
        dark: file.kind == Kind::Dark,
        extends: file.extends.map(|e| e.trim().to_string()),
        colors,
    })
}

/// The palette of `docs[index]`: its base (another doc or a theme of
/// `base`) with its own colours on top. `stack` holds the docs being
/// resolved, to report a cycle.
fn palette_of(
    docs: &[Doc],
    base: &[Theme],
    index: usize,
    stack: &mut Vec<usize>,
) -> Result<Palette, String> {
    let doc = &docs[index];
    if stack.contains(&index) {
        let mut ids: Vec<&str> = stack.iter().map(|&i| docs[i].id.as_str()).collect();
        ids.push(&doc.id);
        return Err(format!("extends cycle: {}", ids.join(" -> ")));
    }
    let start = match &doc.extends {
        None => None,
        Some(parent) => Some(parent_palette(docs, base, index, parent, stack)?),
    };
    let mut palette = start.unwrap_or_else(|| Palette::filled(Rgb(0, 0, 0)));
    for (role, c) in &doc.colors {
        palette.set(role, *c);
    }
    if start.is_none() {
        let set: Vec<&str> = doc.colors.iter().map(|(r, _)| r.as_str()).collect();
        let missing: Vec<&str> = ROLES.iter().copied().filter(|r| !set.contains(r)).collect();
        if !missing.is_empty() {
            return Err(format!(
                "colors: missing {} (a theme without \"extends\" sets every role)",
                missing.join(", ")
            ));
        }
    }
    Ok(palette)
}

fn parent_palette(
    docs: &[Doc],
    base: &[Theme],
    index: usize,
    parent: &str,
    stack: &mut Vec<usize>,
) -> Result<Palette, String> {
    if let Some(i) = docs.iter().position(|d| d.id == parent) {
        stack.push(index);
        let palette = palette_of(docs, base, i, stack);
        stack.pop();
        return palette.map_err(|err| match err.starts_with("extends cycle") {
            true => err,
            false => format!("extends \"{parent}\": {err}"),
        });
    }
    match base.iter().find(|t| t.id == parent) {
        Some(theme) => Ok(theme.palette),
        None => Err(format!("extends: no theme \"{parent}\"")),
    }
}

/// Resolves every doc against the other docs and `base`: the themes that
/// resolve, and one message per doc that does not.
pub fn resolve(docs: &[Doc], base: &[Theme]) -> (Vec<Theme>, Vec<String>) {
    let (mut themes, mut errors) = (Vec::new(), Vec::new());
    for (index, doc) in docs.iter().enumerate() {
        match palette_of(docs, base, index, &mut Vec::new()) {
            Ok(palette) => themes.push(Theme {
                id: doc.id.clone(),
                name: doc.name.clone(),
                dark: doc.dark,
                extends: doc.extends.clone(),
                palette,
                file: doc.file.clone(),
            }),
            Err(err) => errors.push(format!("{}: {err}", doc.origin)),
        }
    }
    (themes, errors)
}

/// The `*.json` files of `dir` in file name order; a missing folder is empty.
fn theme_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(format!("{}: {err}", dir.display())),
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .filter(|p| {
            p.extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("json"))
        })
        .collect();
    files.sort();
    Ok(files)
}

/// Reads and parses the user theme files of `dir`; a file whose id is a
/// built-in or an earlier file's id is skipped with a note.
fn user_docs(dir: &Path, builtins: &[Theme], notes: &mut Vec<String>) -> Vec<Doc> {
    let files = theme_files(dir).unwrap_or_else(|err| {
        notes.push(err);
        Vec::new()
    });
    let mut docs: Vec<Doc> = Vec::new();
    for path in files {
        let origin = path.display().to_string();
        let parsed = std::fs::read_to_string(&path)
            .map_err(|err| format!("{origin}: {err}"))
            .and_then(|text| parse(&text, &origin));
        let mut doc = match parsed {
            Ok(doc) => doc,
            Err(err) => {
                notes.push(err);
                continue;
            }
        };
        if builtins.iter().any(|t| t.id == doc.id) {
            notes.push(format!(
                "{origin}: skipped: \"{}\" is a built-in theme id",
                doc.id
            ));
        } else if let Some(first) = docs.iter().find(|d| d.id == doc.id) {
            notes.push(format!(
                "{origin}: skipped: id \"{}\" is already used by {}",
                doc.id, first.origin
            ));
        } else {
            doc.file = Some(path);
            docs.push(doc);
        }
    }
    docs
}

/// The user themes of `<app_home>/themes/*.json` and the notes to show:
/// files skipped or failing to load, and themes that load but fail the
/// contrast check (with the failing pairs). One bad file never blocks the
/// others.
pub fn load_user(app_home: &Path, builtins: &[Theme]) -> (Vec<Theme>, Vec<String>) {
    let mut notes = Vec::new();
    let docs = user_docs(&app_home.join(THEMES_DIR), builtins, &mut notes);
    let (themes, errors) = resolve(&docs, builtins);
    notes.extend(errors);
    for theme in &themes {
        let failures = check(&theme.palette);
        if !failures.is_empty() {
            let pairs: Vec<String> = failures.iter().map(ToString::to_string).collect();
            let origin = theme.file.as_deref().map(Path::display);
            let origin = origin.map(|d| d.to_string()).unwrap_or_default();
            notes.push(format!(
                "{origin}: theme \"{}\" fails the contrast check: {}",
                theme.id,
                pairs.join("; ")
            ));
        }
    }
    (themes, notes)
}
