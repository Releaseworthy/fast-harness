//! The plugin list generator (PLAN.md D52, docs/plugins.md), shared by
//! `build.rs` and the unit tests. `plugins.toml` uses a small TOML subset
//! parsed here by hand, so the build needs no TOML crate (D1):
//!
//! ```toml
//! # a comment
//! [[plugin]]
//! name = "my-plugin"          # the package name (also its dependency key)
//! git = "https://github.com/me/my-plugin"
//! rev = "0123abcd"            # needed with git: builds are pinned
//! # or: path = "plugins/my-plugin" (relative to the workspace root)
//! api = 1                     # optional: the plugin API it targets (1 or "1")
//! ```

/// The plugin API version this build accepts in `api`. A copy of
/// `fast_harness_core::ext::API_VERSION` (a build script cannot use the
/// core); `tests.rs` checks that both, and `scripts/apply-plugins.ts`, agree.
pub const API: u32 = 1;

/// One `[[plugin]]` entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plugin {
    pub name: String,
    pub git: Option<String>,
    pub path: Option<String>,
    pub rev: Option<String>,
    /// `api`; absent reads as 1.
    pub api: Option<u32>,
}

impl Plugin {
    /// The Rust name of the crate (`-` becomes `_`).
    pub fn ident(&self) -> String {
        self.name.replace('-', "_")
    }
}

/// The marker lines of `crates/fast-harness-plugins/Cargo.toml` between
/// which `scripts/apply-plugins.ts` writes one dependency per plugin.
pub const BEGIN: &str = "# BEGIN PLUGINS";
pub const END: &str = "# END PLUGINS";

/// Parses `plugins.toml`; an error names the line.
pub fn parse(text: &str) -> Result<Vec<Plugin>, String> {
    let mut plugins: Vec<Plugin> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let at = |why: String| format!("plugins.toml line {}: {why}", index + 1);
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line == "[[plugin]]" {
            plugins.push(Plugin::default());
            continue;
        }
        let (key, raw) = key_value(line).map_err(at)?;
        let Some(plugin) = plugins.last_mut() else {
            return Err(at("a key before the first [[plugin]]".into()));
        };
        if key == "api" {
            plugin.api = Some(api_value(raw).map_err(at)?);
            continue;
        }
        let slot = match key {
            "name" => &mut plugin.name,
            "git" => plugin.git.get_or_insert_with(String::new),
            "path" => plugin.path.get_or_insert_with(String::new),
            "rev" => plugin.rev.get_or_insert_with(String::new),
            other => {
                return Err(at(format!(
                    "unknown key `{other}` (name, git, path, rev, api)"
                )))
            }
        };
        *slot = plain(key, raw).map_err(at)?;
    }
    for plugin in &plugins {
        check(plugin, &plugins)?;
    }
    Ok(plugins)
}

/// A `#` ends the line unless it is inside a string.
fn strip_comment(line: &str) -> &str {
    let mut quoted = false;
    for (at, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '#' if !quoted => return &line[..at],
            _ => {}
        }
    }
    line
}

/// `key = value`, both trimmed.
fn key_value(line: &str) -> Result<(&str, &str), String> {
    let (key, value) = line
        .split_once('=')
        .ok_or_else(|| format!("expected `key = \"value\"`, got `{line}`"))?;
    Ok((key.trim(), value.trim()))
}

/// The inside of a basic string without escapes.
fn plain(key: &str, value: &str) -> Result<String, String> {
    let inner = (value.strip_prefix('"').and_then(|v| v.strip_suffix('"')))
        .filter(|inner| !inner.contains('"') && !inner.contains('\\'))
        .ok_or_else(|| format!("`{key}` needs a plain quoted string"))?;
    Ok(inner.to_string())
}

/// `api = 1` or `api = "1"`.
fn api_value(value: &str) -> Result<u32, String> {
    let inner = (value.strip_prefix('"').and_then(|v| v.strip_suffix('"'))).unwrap_or(value);
    (inner.parse::<u32>()).map_err(|_| format!("`api` needs a whole number, got `{value}`"))
}

fn check(plugin: &Plugin, all: &[Plugin]) -> Result<(), String> {
    let name = &plugin.name;
    let valid = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    if name.is_empty() || !name.chars().all(valid) {
        return Err(format!(
            "plugin name `{name}`: use letters, digits, - and _"
        ));
    }
    if all.iter().filter(|p| p.ident() == plugin.ident()).count() > 1 {
        return Err(format!("plugin `{name}` is listed twice"));
    }
    let api = plugin.api.unwrap_or(1);
    if api != API {
        return Err(format!(
            "plugin `{name}` targets plugin API {api}, this Fast Harness supports API {API} (fast_harness_core::ext::API_VERSION)"
        ));
    }
    match (&plugin.git, &plugin.path, &plugin.rev) {
        (Some(_), None, Some(rev)) if !rev.is_empty() => Ok(()),
        (Some(_), None, _) => Err(format!("plugin `{name}`: a git plugin needs `rev`")),
        (None, Some(path), _) if !path.is_empty() => Ok(()),
        _ => Err(format!(
            "plugin `{name}`: set exactly one of `git` or `path`"
        )),
    }
}

/// The Rust source of `register_all` and `PLUGINS` for `plugins`.
pub fn generate(plugins: &[Plugin]) -> String {
    let mut out = String::from(
        "// Generated by build.rs from plugins.toml (docs/plugins.md): do not edit.\n\n",
    );
    let names: Vec<String> = plugins.iter().map(|p| format!("{:?}", p.name)).collect();
    out.push_str(&format!(
        "/// The plugins of this build, in `plugins.toml` order.\npub const PLUGINS: &[&str] = &[{}];\n\n",
        names.join(", ")
    ));
    out.push_str("/// Calls `register` of every plugin, in `plugins.toml` order.\n");
    if plugins.is_empty() {
        out.push_str("pub fn register_all(_ext: &mut fast_harness_core::ext::v1::Ext) {}\n");
        return out;
    }
    out.push_str("pub fn register_all(ext: &mut fast_harness_core::ext::v1::Ext) {\n");
    for plugin in plugins {
        out.push_str(&format!("    ::{}::register(ext);\n", plugin.ident()));
    }
    out.push_str("}\n");
    out
}

/// The dependency keys between the markers of `cargo_toml` must be the
/// plugin names: else `scripts/apply-plugins.ts` was not run.
pub fn check_deps(cargo_toml: &str, plugins: &[Plugin]) -> Result<(), String> {
    let start = cargo_toml
        .find(BEGIN)
        .ok_or("Cargo.toml lacks the BEGIN PLUGINS marker")?;
    let end = cargo_toml
        .find(END)
        .ok_or("Cargo.toml lacks the END PLUGINS marker")?;
    let section = cargo_toml.get(start + BEGIN.len()..end).unwrap_or_default();
    let mut keys: Vec<String> = (section.lines())
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_once('=').map(|(key, _)| key.trim().to_string()))
        .collect();
    let mut names: Vec<String> = plugins.iter().map(|p| p.name.clone()).collect();
    keys.sort();
    names.sort();
    match keys == names {
        true => Ok(()),
        false => Err(format!(
            "plugins.toml lists [{}] but crates/fast-harness-plugins/Cargo.toml has [{}]: run `bun scripts/apply-plugins.ts`",
            names.join(", "),
            keys.join(", ")
        )),
    }
}
