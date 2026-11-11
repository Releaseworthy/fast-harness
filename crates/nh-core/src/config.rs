//! `~/.nh/config.json`: model, effort, cache TTL, limits.
//! Keys are camelCase; a missing file or key takes the default, unknown keys
//! are ignored. `NH_MODEL` and `NH_EFFORT` override the file.

use std::path::Path;

use serde::Deserialize;

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
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Config::default(),
            Err(err) => return Err(format!("{}: {err}", path.display())),
        };
        config.apply_env(env);
        Ok(config)
    }

    /// `NH_MODEL` replaces the model; `NH_EFFORT` the effort (`default` means
    /// the model's own default). Empty values are ignored.
    pub fn apply_env(&mut self, env: &dyn Fn(&str) -> Option<String>) {
        if let Some(model) = env("NH_MODEL").filter(|v| !v.trim().is_empty()) {
            self.model = model.trim().to_string();
        }
        if let Some(effort) = env("NH_EFFORT").filter(|v| !v.trim().is_empty()) {
            self.set_effort(&effort);
        }
    }

    /// Sets the effort from user text; `default` clears it.
    pub fn set_effort(&mut self, effort: &str) {
        let effort = effort.trim();
        self.effort = (effort != "default").then(|| effort.to_string());
    }
}
