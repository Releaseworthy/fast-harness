//! Settings layers (PLAN.md D41). Later wins, key by key: built-in defaults,
//! `~/.fast-harness/config.json`, `<project>/.fast-harness/config.json` (only
//! in a trusted project, [`super::trust`]), the environment, the command
//! line. Every resolved key remembers the layer that set it.
//!
//! [`kind_files`] is the one lister of extension files: `<kind>/*.<ext>` of
//! the user folder, then of the trusted project folder (providers now; tools,
//! agents and commands use it too). No folder, no work.

use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{trust, Config, FILE_NAME};
use crate::theme;

/// The project's settings folder name.
pub const PROJECT_DIR: &str = ".fast-harness";

/// Where a resolved key came from, lowest layer first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    Default,
    User,
    Project,
    Env,
    Cli,
}

impl Source {
    pub fn word(self) -> &'static str {
        match self {
            Source::Default => "default",
            Source::User => "user",
            Source::Project => "project",
            Source::Env => "env",
            Source::Cli => "command line",
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.word())
    }
}

/// The folders the settings come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layers {
    /// `~/.fast-harness` or `FAST_HARNESS_HOME`.
    pub home: PathBuf,
    /// The nearest folder at or above the working directory that holds a
    /// `.fast-harness` folder other than `home`.
    pub project: Option<PathBuf>,
    pub trusted: bool,
    /// The `AGENTS.md` files of `cwd` left out because their project is not
    /// trusted (D62), outermost first; named in [`Layers::untrusted_note`].
    pub agents_skipped: Vec<PathBuf>,
}

impl Layers {
    /// The user layer only (no working directory known yet).
    pub fn user_only(home: &Path) -> Layers {
        Layers {
            home: home.to_path_buf(),
            project: None,
            trusted: false,
            agents_skipped: Vec::new(),
        }
    }

    /// The layers of `cwd`: its project (if any) and whether it is trusted.
    pub fn find(home: &Path, cwd: &Path) -> Layers {
        let project = project_root(home, cwd);
        let trusted = project
            .as_deref()
            .is_some_and(|root| trust::is_trusted(home, root));
        let agents_skipped = agents_dirs(cwd)
            .into_iter()
            .map(|dir| dir.join(AGENTS_FILE))
            .filter(|file| file.is_file())
            .filter(|file| !agents_trusted(home, file.parent().unwrap_or(cwd)))
            .collect();
        Layers {
            home: home.to_path_buf(),
            project,
            trusted,
            agents_skipped,
        }
    }

    /// The folders read, in layer order: the user folder, then the project's
    /// `.fast-harness` when trusted.
    pub fn dirs(&self) -> Vec<(PathBuf, Source)> {
        let mut out = vec![(self.home.clone(), Source::User)];
        if let (Some(root), true) = (&self.project, self.trusted) {
            out.push((root.join(PROJECT_DIR), Source::Project));
        }
        out
    }

    /// The one-time note of an untrusted project (D62): its `.fast-harness`
    /// folder and the `AGENTS.md` files left out; `None` otherwise.
    pub fn untrusted_note(&self) -> Option<String> {
        let folder = self.project.as_ref().filter(|_| !self.trusted);
        let first = self.agents_skipped.first();
        let root = match (folder, first) {
            (Some(root), _) => root.clone(),
            (None, Some(file)) => trust_root(&self.home, file.parent()?),
            (None, None) => return None,
        };
        let mut what = Vec::new();
        if folder.is_some() {
            what.push("nothing in its .fast-harness folder was loaded".to_string());
        }
        if !self.agents_skipped.is_empty() {
            let files: Vec<String> = (self.agents_skipped.iter())
                .map(|file| file.display().to_string())
                .collect();
            what.push(format!(
                "its AGENTS.md files were left out of the system prompt ({})",
                files.join(", ")
            ));
        }
        Some(format!(
            "project {} is not trusted: {}; type /trust or run `fast-harness trust` to load it (AGENTS.md joins the system prompt of the next session; a running session gets it as a context update)",
            root.display(),
            what.join(" and ")
        ))
    }
}

/// The file name of the project instructions.
pub const AGENTS_FILE: &str = "AGENTS.md";

/// The nearest folder at or above `cwd` holding a `.git` entry.
pub fn repo_root(cwd: &Path) -> Option<PathBuf> {
    cwd.ancestors()
        .find(|dir| std::fs::symlink_metadata(dir.join(".git")).is_ok())
        .map(Path::to_path_buf)
}

/// The folders whose `AGENTS.md` belongs to `cwd`, outermost first: from
/// [`repo_root`] down to `cwd`; only `cwd` outside a repository.
pub fn agents_dirs(cwd: &Path) -> Vec<&Path> {
    let mut dirs = Vec::new();
    for dir in cwd.ancestors() {
        dirs.push(dir);
        if std::fs::symlink_metadata(dir.join(".git")).is_ok() {
            dirs.reverse();
            return dirs;
        }
    }
    vec![cwd]
}

/// The folder `/trust` trusts for `dir` (D62): its project folder
/// ([`project_root`]), else its repository root, else `dir` itself.
pub fn trust_root(home: &Path, dir: &Path) -> PathBuf {
    project_root(home, dir)
        .or_else(|| repo_root(dir))
        .unwrap_or_else(|| dir.to_path_buf())
}

/// True when the `AGENTS.md` of `dir` may load (D62): the project of `dir`
/// ([`trust_root`]) is trusted. An untrusted project supplies nothing.
pub fn agents_trusted(home: &Path, dir: &Path) -> bool {
    trust::is_trusted(home, &trust_root(home, dir))
}

/// The nearest folder at or above `cwd` with a `.fast-harness` folder that
/// is not `home` itself.
pub fn project_root(home: &Path, cwd: &Path) -> Option<PathBuf> {
    let home = trust::canonical(home);
    let cwd = trust::canonical(cwd);
    cwd.ancestors()
        .find(|dir| {
            let settings = dir.join(PROJECT_DIR);
            settings.is_dir() && trust::canonical(&settings) != home
        })
        .map(Path::to_path_buf)
}

/// One extension file and its layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerFile {
    pub path: PathBuf,
    pub source: Source,
}

/// A file that did not load and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Skipped {
    pub path: PathBuf,
    pub reason: String,
}

/// What loaded and what was skipped (`fast-harness doctor`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub loaded: Vec<LayerFile>,
    pub skipped: Vec<Skipped>,
}

/// `<dir>/<kind>/*.<ext>` of every layer folder, user first, each folder
/// sorted by file name. A missing folder costs one failed directory read.
pub fn kind_files(layers: &Layers, kind: &str, ext: &str) -> Vec<LayerFile> {
    let mut out = Vec::new();
    for (dir, source) in layers.dirs() {
        let Ok(entries) = std::fs::read_dir(dir.join(kind)) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                let ours = path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case(ext));
                ours && path.is_file()
            })
            .collect();
        paths.sort();
        out.extend(paths.into_iter().map(|path| LayerFile { path, source }));
    }
    out
}

/// `<dir>/skills/<name>/SKILL.md` of every layer folder (Agent Skills,
/// PLAN.md D43), user first, each folder sorted by skill folder name.
pub fn skill_files(layers: &Layers) -> Vec<LayerFile> {
    let mut out = Vec::new();
    for (dir, source) in layers.dirs() {
        let Ok(entries) = std::fs::read_dir(dir.join("skills")) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("SKILL.md"))
            .filter(|path| path.is_file())
            .collect();
        paths.sort();
        out.extend(paths.into_iter().map(|path| LayerFile { path, source }));
    }
    out
}

/// The resolved settings with the source of every key.
#[derive(Clone, Debug)]
pub struct Layered {
    pub config: Config,
    /// Dotted key (`compaction.trigger`) to the layer that set it.
    pub sources: BTreeMap<String, Source>,
    pub layers: Layers,
    pub report: Report,
}

/// Environment variables and the key each one sets ([`Config::apply_env`]).
pub const ENV_KEYS: [(&str, &str); 6] = [
    ("FAST_HARNESS_MODEL", "model"),
    ("FAST_HARNESS_EFFORT", "effort"),
    ("FAST_HARNESS_THEME", "theme"),
    ("FAST_HARNESS_GREYSCALE", "greyscale"),
    ("FAST_HARNESS_ICONS", "icons"),
    ("FAST_HARNESS_WIRE", "wire"),
];

/// Resolves the layers. A bad user `config.json` is an error (as before
/// D41); a bad project file is skipped with its reason in the report.
pub fn load(layers: Layers, env: &dyn Fn(&str) -> Option<String>) -> Result<Layered, String> {
    let mut merged = serde_json::to_value(Config::default()).map_err(|err| err.to_string())?;
    let mut sources = BTreeMap::new();
    leaves(&merged, "", Source::Default, &mut sources);
    let mut report = Report::default();
    for (dir, source) in layers.dirs() {
        let path = dir.join(FILE_NAME);
        match read_layer(&path) {
            Ok(None) => {}
            Ok(Some(mut object)) => {
                if source == Source::Project {
                    project_rule(&mut object);
                }
                leaves(&object, "", source, &mut sources);
                merge(&mut merged, object);
                report.loaded.push(LayerFile { path, source });
            }
            Err(reason) if source == Source::User => {
                return Err(format!("{}: {reason}", path.display()))
            }
            Err(reason) => report.skipped.push(Skipped { path, reason }),
        }
    }
    if let Some(reason) = trust::problem(&layers.home) {
        let path = layers.home.join(trust::FILE_NAME);
        report.skipped.push(Skipped { path, reason });
    }
    if let Some(root) = layers.project.as_ref().filter(|_| !layers.trusted) {
        let path = root.join(PROJECT_DIR);
        let reason = "project not trusted (`fast-harness trust`)".into();
        report.skipped.push(Skipped { path, reason });
    }
    let mut config: Config = serde_json::from_value(merged).map_err(|err| err.to_string())?;
    config.apply_env(env);
    for (var, key) in ENV_KEYS {
        if env(var).is_some_and(|value| !value.trim().is_empty()) {
            sources.insert(key.to_string(), Source::Env);
        }
    }
    if let Some(base) = theme::legacy_greyscale(&config.theme) {
        config.theme = base.into();
        config.greyscale = theme::GREYSCALE_MAX;
    }
    Ok(Layered {
        config,
        sources,
        layers,
        report,
    })
}

/// One `config.json`: `None` when missing; an error when it is not a JSON
/// object or a key has a value of the wrong type.
fn read_layer(path: &Path) -> Result<Option<Value>, String> {
    let bytes = match crate::store::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.to_string()),
    };
    let mut value: Value = serde_json::from_slice(&bytes).map_err(|err| err.to_string())?;
    let Some(object) = value.as_object_mut() else {
        return Err("not a JSON object".into());
    };
    // `version` (phase 29): absent reads as 1, a newer one is refused.
    super::ext_file::check_version(object.remove("version").as_ref())?;
    serde_json::from_value::<Config>(value.clone()).map_err(|err| err.to_string())?;
    Ok(Some(value))
}

/// A trusted project may set `approvals` to `ask` but never to anything
/// else, and its `allow` is dropped (PLAN.md D64): only the user layer
/// weakens the gate.
fn project_rule(object: &mut Value) {
    let Some(map) = object.as_object_mut() else {
        return;
    };
    map.remove("allow");
    if map.get("approvals").is_some_and(|value| value != "ask") {
        map.remove("approvals");
    }
}

/// Records `source` for every leaf key of `value` under `prefix`.
fn leaves(value: &Value, prefix: &str, source: Source, out: &mut BTreeMap<String, Source>) {
    match value.as_object() {
        Some(object) if !object.is_empty() => {
            for (key, child) in object {
                let path = match prefix {
                    "" => key.clone(),
                    _ => format!("{prefix}.{key}"),
                };
                leaves(child, &path, source, out);
            }
        }
        _ if !prefix.is_empty() => {
            out.insert(prefix.to_string(), source);
        }
        _ => {}
    }
}

/// `top` over `base`: objects merge key by key, anything else replaces.
fn merge(base: &mut Value, top: Value) {
    match (base, top) {
        (Value::Object(base), Value::Object(top)) => {
            for (key, value) in top {
                match base.get_mut(&key) {
                    Some(slot) if slot.is_object() && value.is_object() => merge(slot, value),
                    _ => {
                        base.insert(key, value);
                    }
                }
            }
        }
        (base, top) => *base = top,
    }
}

impl Layered {
    /// The value of dotted `key` and its source; `None` for an unknown key.
    /// An object key takes the highest source among its leaves.
    pub fn get(&self, key: &str) -> Option<(Value, Source)> {
        let mut value = serde_json::to_value(&self.config).ok()?;
        for part in key.split('.') {
            value = value.get(part)?.clone();
        }
        let below = format!("{key}.");
        let source = self.sources.get(key).copied().or_else(|| {
            let children = self.sources.iter().filter(|(k, _)| k.starts_with(&below));
            children.map(|(_, source)| *source).max()
        });
        Some((value, source.unwrap_or(Source::Default)))
    }

    /// A command line flag: `model` or `effort`; other keys are ignored.
    pub fn set_cli(&mut self, key: &str, value: &str) {
        match key {
            "model" => self.config.model = value.to_string(),
            "effort" => self.config.set_effort(value),
            _ => return,
        }
        self.sources.insert(key.to_string(), Source::Cli);
    }
}
