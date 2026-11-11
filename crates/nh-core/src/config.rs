//! `~/.nh/config.json`: model, effort, cache TTL, limits, theme, wire capture.
//! Keys are camelCase; a missing file or key takes the default, unknown keys
//! are ignored. `NH_MODEL`, `NH_EFFORT`, `NH_THEME` and `NH_WIRE` override
//! the file. [`save_setting`] writes one key back for the front ends.

use std::io;
use std::path::Path;

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::theme;

pub const FILE_NAME: &str = "config.json";

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub model: String,
    /// Reasoning effort; `None` (JSON `null`) uses the model's default.
    pub effort: Option<String>,
    pub max_tokens: u32,
    /// Ask for the 1 hour prompt cache instead of the 5 minute one.
    pub cache_long: bool,
    /// Model calls allowed in one turn.
    pub max_steps: u32,
    /// Longest failover wait (milliseconds) the agent accepts before failing.
    pub max_wait_ms: u64,
    /// Retries of one step after retryable errors (overloaded, server, network).
    pub retries: u32,
    /// Colour theme id (`theme::THEMES`); an unknown id shows the default theme.
    pub theme: String,
    /// Raw wire capture of every model call (redacted request and raw
    /// response stream) under `<session>.wire/`. Off by default.
    pub wire: bool,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            model: "claude-opus-5-5".into(),
            effort: Some("high".into()),
            max_tokens: 32_000,
            cache_long: false,
            max_steps: 200,
            max_wait_ms: 3_600_000,
            retries: 8,
            theme: theme::DEFAULT_THEME.into(),
            wire: false,
        }
    }
}

impl Config {
    /// Reads `<nh_home>/config.json` and applies the environment overrides.
    /// A file that cannot be read or parsed is an error message.
    pub fn load(nh_home: &Path) -> Result<Config, String> {
        Config::load_with(nh_home, &|key| std::env::var(key).ok())
    }

    /// [`Config::load`] with the environment given as a lookup function.
    pub fn load_with(
        nh_home: &Path,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Config, String> {
        let path = nh_home.join(FILE_NAME);
        let mut config = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<Config>(&bytes)
                .map_err(|err| format!("{}: {err}", path.display()))?,
            Err(err) if err.kind() == io::ErrorKind::NotFound => Config::default(),
            Err(err) => return Err(format!("{}: {err}", path.display())),
        };
        config.apply_env(env);
        Ok(config)
    }

    /// `NH_MODEL` replaces the model; `NH_EFFORT` the effort (`default` means
    /// the model's own default); `NH_THEME` the theme; `NH_WIRE` turns wire
    /// capture on (`1`, `true`, `on`, `yes`) or off (`0`, `false`, `off`,
    /// `no`). Empty and unrecognised values are ignored.
    pub fn apply_env(&mut self, env: &dyn Fn(&str) -> Option<String>) {
        let value = |key: &str| {
            env(key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        if let Some(model) = value("NH_MODEL") {
            self.model = model;
        }
        if let Some(effort) = value("NH_EFFORT") {
            self.set_effort(&effort);
        }
        if let Some(theme) = value("NH_THEME") {
            self.theme = theme;
        }
        if let Some(wire) = value("NH_WIRE").and_then(|v| switch(&v)) {
            self.wire = wire;
        }
    }

    /// Sets the effort from user text; `default` clears it.
    pub fn set_effort(&mut self, effort: &str) {
        let effort = effort.trim();
        self.effort = (effort != "default").then(|| effort.to_string());
    }
}

/// `true` or `false` from a switch word, `None` for anything else.
fn switch(word: &str) -> Option<bool> {
    match word.to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" => Some(true),
        "0" | "false" | "off" | "no" => Some(false),
        _ => None,
    }
}

/// Sets `key` to `value` in `<nh_home>/config.json` and keeps every other
/// key (they come back sorted). A missing file or folder is created; a file
/// that is not a JSON object is an error and stays untouched. The write goes
/// to `config.json.tmp` first and is then renamed over the file.
pub fn save_setting(nh_home: &Path, key: &str, value: Value) -> Result<(), String> {
    let path = nh_home.join(FILE_NAME);
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
    std::fs::create_dir_all(nh_home).map_err(|err| failed(&err))?;
    let tmp = nh_home.join(format!("{FILE_NAME}.tmp"));
    std::fs::write(&tmp, bytes).map_err(|err| failed(&err))?;
    std::fs::rename(&tmp, &path).map_err(|err| failed(&err))
}
