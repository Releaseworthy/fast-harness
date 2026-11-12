//! `~/.fast-harness/config.json`: model, effort, cache TTL, limits, theme, greyscale, wire capture.
//! Keys are camelCase; a missing file or key takes the default, unknown keys
//! are ignored. `FAST_HARNESS_MODEL`, `FAST_HARNESS_EFFORT`, `FAST_HARNESS_THEME`, `FAST_HARNESS_GREYSCALE`, `FAST_HARNESS_ICONS` and `FAST_HARNESS_WIRE` override
//! the file. [`save_setting`] writes one key back for the front ends.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};

use crate::theme;

pub const FILE_NAME: &str = "config.json";
/// Default byte budget of one tool result (`toolOutputBytes`).
pub const DEFAULT_TOOL_OUTPUT_BYTES: usize = 8 * 1024;
/// The smallest budget: room for the spill note and some text.
pub const MIN_TOOL_OUTPUT_BYTES: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub model: String,
    /// Reasoning effort; `None` (JSON `null`) uses the model's default.
    pub effort: Option<String>,
    pub max_tokens: u32,
    /// `cacheTtl` (PLAN.md D36): `"5m"`, `"1h"` or `"auto"`; `None` when the
    /// file has no valid value (then `cacheLong` decides, else `auto`).
    #[serde(deserialize_with = "ttl_word")]
    pub cache_ttl: Option<CacheTtl>,
    /// The older `cacheLong` key, still read: `true` means `cacheTtl: "1h"`.
    pub cache_long: bool,
    /// Set by the TUI and the GUI (never read from the file): under `auto`
    /// the message breakpoint gets 1 hour there, 5 minutes in print mode
    /// and subagents.
    #[serde(skip)]
    pub interactive: bool,
    /// Model calls allowed in one turn.
    pub max_steps: u32,
    /// Longest failover wait (milliseconds) the agent accepts before failing.
    pub max_wait_ms: u64,
    /// Retries of one step after retryable errors (overloaded, server, network).
    pub retries: u32,
    /// Colour theme id (built-in or a user theme file); an unknown id shows
    /// the default theme. The removed `greyscale-dark` and `greyscale-light`
    /// load as `dark` and `light` with greyscale 100.
    pub theme: String,
    /// Greyscale level of every theme, 0 (full colour) to 100 (pure greys).
    /// A number is rounded and clamped; anything else reads as 0.
    #[serde(deserialize_with = "greyscale_level")]
    pub greyscale: u8,
    /// Icon set of the TUI: `nerd` (Nerd Font icons, the default) or
    /// `plain` (Unicode marks); anything else reads as `nerd`.
    pub icons: String,
    /// Raw wire capture of every model call (redacted request and raw
    /// response stream) under `<session>.wire/`. Off by default.
    pub wire: bool,
    /// `toolOutputBytes` (PLAN.md D37): the byte budget of one tool result.
    pub tool_output_bytes: usize,
    /// `toolOutput`: budgets per tool name, e.g. `{"read": 16384}`.
    pub tool_output: BTreeMap<String, usize>,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            model: "claude-opus-5-5".into(),
            effort: Some("high".into()),
            max_tokens: 32_000,
            cache_ttl: None,
            cache_long: false,
            interactive: false,
            max_steps: 200,
            max_wait_ms: 3_600_000,
            retries: 8,
            theme: theme::DEFAULT_THEME.into(),
            greyscale: 0,
            icons: "nerd".into(),
            wire: false,
            tool_output_bytes: DEFAULT_TOOL_OUTPUT_BYTES,
            tool_output: BTreeMap::new(),
        }
    }
}

impl Config {
    /// Reads `<app_home>/config.json` and applies the environment overrides.
    /// A file that cannot be read or parsed is an error message.
    pub fn load(app_home: &Path) -> Result<Config, String> {
        Config::load_with(app_home, &|key| std::env::var(key).ok())
    }

    /// [`Config::load`] with the environment given as a lookup function.
    pub fn load_with(
        app_home: &Path,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Config, String> {
        let path = app_home.join(FILE_NAME);
        let mut config = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<Config>(&bytes)
                .map_err(|err| format!("{}: {err}", path.display()))?,
            Err(err) if err.kind() == io::ErrorKind::NotFound => Config::default(),
            Err(err) => return Err(format!("{}: {err}", path.display())),
        };
        config.apply_env(env);
        if let Some(base) = theme::legacy_greyscale(&config.theme) {
            config.theme = base.into();
            config.greyscale = theme::GREYSCALE_MAX;
        }
        Ok(config)
    }

    /// `FAST_HARNESS_MODEL` replaces the model; `FAST_HARNESS_EFFORT` the effort (`default` means
    /// the model's own default); `FAST_HARNESS_THEME` the theme; `FAST_HARNESS_GREYSCALE` the
    /// greyscale level (a number, clamped to 0..=100); `FAST_HARNESS_ICONS` the icon
    /// set; `FAST_HARNESS_WIRE` turns wire
    /// capture on (`1`, `true`, `on`, `yes`) or off (`0`, `false`, `off`,
    /// `no`). Empty and unrecognised values are ignored.
    pub fn apply_env(&mut self, env: &dyn Fn(&str) -> Option<String>) {
        let value = |key: &str| {
            env(key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        if let Some(model) = value("FAST_HARNESS_MODEL") {
            self.model = model;
        }
        if let Some(effort) = value("FAST_HARNESS_EFFORT") {
            self.set_effort(&effort);
        }
        if let Some(theme) = value("FAST_HARNESS_THEME") {
            self.theme = theme;
        }
        if let Some(level) = value("FAST_HARNESS_GREYSCALE").and_then(|v| theme::parse_level(&v)) {
            self.greyscale = level;
        }
        if let Some(icons) = value("FAST_HARNESS_ICONS") {
            self.icons = icons;
        }
        if let Some(wire) = value("FAST_HARNESS_WIRE").and_then(|v| switch(&v)) {
            self.wire = wire;
        }
    }

    /// Sets the effort from user text; `default` clears it.
    pub fn set_effort(&mut self, effort: &str) {
        let effort = effort.trim();
        self.effort = (effort != "default").then(|| effort.to_string());
    }

    /// The byte budget of one result of `tool` (PLAN.md D37): the
    /// `toolOutput` entry of the tool, else `toolOutputBytes`, never below
    /// [`MIN_TOOL_OUTPUT_BYTES`]. The one place every budget is looked up.
    pub fn tool_budget(&self, tool: &str) -> usize {
        let bytes = self
            .tool_output
            .get(tool)
            .copied()
            .unwrap_or(self.tool_output_bytes);
        bytes.max(MIN_TOOL_OUTPUT_BYTES)
    }

    /// The cache TTL in effect: `cacheTtl`, else `"1h"` for `cacheLong: true`, else `auto`.
    pub fn cache_setting(&self) -> CacheTtl {
        self.cache_ttl.unwrap_or(match self.cache_long {
            true => CacheTtl::Long,
            false => CacheTtl::Auto,
        })
    }

    /// `(system 1h, messages 1h)`: the tools and system breakpoints, then the
    /// last message. `auto` gives the system 1 hour always (every session of
    /// the workspace shares it) and the messages 1 hour only when `interactive`.
    pub fn cache_hours(&self) -> (bool, bool) {
        match self.cache_setting() {
            CacheTtl::Short => (false, false),
            CacheTtl::Long => (true, true),
            CacheTtl::Auto => (true, self.interactive),
        }
    }
}

/// The `cacheTtl` setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheTtl {
    Auto,
    /// `"5m"`.
    Short,
    /// `"1h"`.
    Long,
}

/// `"5m"`, `"1h"` or `"auto"`; anything else reads as absent.
fn ttl_word<'de, D: Deserializer<'de>>(de: D) -> Result<Option<CacheTtl>, D::Error> {
    let value = Value::deserialize(de)?;
    Ok(match value.as_str().map(str::trim) {
        Some("5m") => Some(CacheTtl::Short),
        Some("1h") => Some(CacheTtl::Long),
        Some("auto") => Some(CacheTtl::Auto),
        _ => None,
    })
}

/// The `greyscale` key: a JSON number rounded and clamped to 0..=100;
/// any other value reads as 0 instead of failing the whole file.
fn greyscale_level<'de, D: Deserializer<'de>>(de: D) -> Result<u8, D::Error> {
    let value = Value::deserialize(de)?;
    Ok(value.as_f64().and_then(theme::level_of).unwrap_or(0))
}

/// `true` or `false` from a switch word, `None` for anything else.
fn switch(word: &str) -> Option<bool> {
    match word.to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" => Some(true),
        "0" | "false" | "off" | "no" => Some(false),
        _ => None,
    }
}

/// Sets `key` to `value` in `<app_home>/config.json` and keeps every other
/// key (they come back sorted). A missing file or folder is created; a file
/// that is not a JSON object is an error and stays untouched. The write goes
/// to `config.json.tmp` first and is then renamed over the file.
pub fn save_setting(app_home: &Path, key: &str, value: Value) -> Result<(), String> {
    let path = app_home.join(FILE_NAME);
    let failed = |err: &dyn std::fmt::Display| format!("{}: {err}", path.display());
    let mut root = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice::<Value>(&bytes).map_err(|err| failed(&err))?,
        Err(err) if err.kind() == io::ErrorKind::NotFound => Value::Object(Map::new()),
        Err(err) => return Err(failed(&err)),
    };
    let Some(object) = root.as_object_mut() else {
        return Err(failed(&"not a JSON object"));
    };
    object.insert(key.to_string(), value);
    let mut bytes = serde_json::to_vec_pretty(&root).map_err(|err| failed(&err))?;
    bytes.push(b'\n');
    std::fs::create_dir_all(app_home).map_err(|err| failed(&err))?;
    let tmp = app_home.join(format!("{FILE_NAME}.tmp"));
    std::fs::write(&tmp, bytes).map_err(|err| failed(&err))?;
    std::fs::rename(&tmp, &path).map_err(|err| failed(&err))
}
