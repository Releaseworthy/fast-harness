//! The Claude Code release an OAuth call presents itself as (PLAN.md D71).
//!
//! The API refuses a model to a client it thinks is too old ("Claude Code
//! 2.1.280 does not support this model; version 2.1.290 or newer is
//! required", HTTP 400, 2026-11-14). So the version is not a constant: it is
//! one value per process that only moves forward, read by every request.
//! At start it comes from `claude-code.json` in the home folder, then a
//! background fetch of the npm `latest` tag of `@anthropic-ai/claude-code`
//! raises it while the app runs (no restart), and an HTTP 400 that names a
//! newer required version raises it before the call is sent again.
//! `claudeCodeVersion` in the settings pins one version instead.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::http::{Flow, HttpRequest, Transport};

/// The version used before anything newer is known (npm `latest`,
/// registry.npmjs.org, read 2026-11-14).
pub const BUILTIN: &str = "2.1.323";
/// The npm document of the newest release.
pub const LATEST_URL: &str = "https://registry.npmjs.org/@anthropic-ai/claude-code/latest";
/// The cache in the home folder.
pub const CACHE_FILE: &str = "claude-code.json";
/// Largest registry answer read (the `latest` document is a few KB).
const MAX_BODY: usize = 1024 * 1024;
/// Limit for connect and each wait of the registry fetch.
const FETCH_TIMEOUT_MS: u32 = 20_000;

struct State {
    version: String,
    pinned: bool,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static UPDATING: AtomicBool = AtomicBool::new(false);

fn with_state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    let mut guard = STATE.lock().unwrap_or_else(|p| p.into_inner());
    let state = guard.get_or_insert_with(|| State {
        version: BUILTIN.into(),
        pinned: false,
    });
    f(state)
}

/// `major.minor.patch` of a plain release (no pre-release, no build part).
pub fn parse(version: &str) -> Option<(u32, u32, u32)> {
    let number = |part: &str| -> Option<u32> {
        let digits = (1..=9).contains(&part.len()) && part.bytes().all(|b| b.is_ascii_digit());
        digits.then(|| part.parse().ok()).flatten()
    };
    let parts: Vec<&str> = version.trim().split('.').collect();
    match parts[..] {
        [major, minor, patch] => Some((number(major)?, number(minor)?, number(patch)?)),
        _ => None,
    }
}

/// The version every OAuth request sends now.
pub fn version() -> String {
    with_state(|state| state.version.clone())
}

/// Whether `claudeCodeVersion` pins the version.
pub fn pinned() -> bool {
    with_state(|state| state.pinned)
}

/// Takes `version` when it is a plain release newer than the current one
/// and nothing is pinned; true when it changed.
pub fn offer(version: &str) -> bool {
    let Some(new) = parse(version) else {
        return false;
    };
    with_state(|state| {
        let newer = parse(&state.version).is_none_or(|old| new > old);
        let take = newer && !state.pinned;
        if take {
            state.version = version.trim().to_string();
        }
        take
    })
}

/// Applies the `claudeCodeVersion` setting: `auto` (or anything that is
/// not a plain release) follows the newest known release, a release such as
/// `2.1.323` pins it. Leaving a pin starts again from [`BUILTIN`].
pub fn configure(setting: &str) {
    with_state(|state| match parse(setting) {
        Some(_) => {
            state.version = setting.trim().to_string();
            state.pinned = true;
        }
        None if state.pinned => {
            state.version = BUILTIN.into();
            state.pinned = false;
        }
        None => {}
    });
}

/// The version an API error asks for: `... version 2.1.290 or newer is
/// required ...`.
pub fn required_by(message: &str) -> Option<String> {
    let end = message.find(" or newer is required")?;
    let head = &message[..end];
    let start = head.rfind("version ")? + "version ".len();
    let version = head[start..].trim();
    parse(version).map(|_| version.to_string())
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CacheFile {
    version: String,
    /// Unix milliseconds of the fetch.
    fetched_at: u64,
}

/// Offers the cached version of `home`, if any.
pub fn load_cache(home: &Path) {
    let Ok(bytes) = crate::store::read(home.join(CACHE_FILE)) else {
        return;
    };
    if let Ok(file) = serde_json::from_slice::<CacheFile>(&bytes) {
        offer(&file.version);
    }
}

fn write_cache(home: &Path, version: &str) -> Result<(), String> {
    let file = CacheFile {
        version: version.into(),
        fetched_at: now_ms(),
    };
    let text = serde_json::to_string_pretty(&file).map_err(|err| err.to_string())?;
    let path = home.join(CACHE_FILE);
    crate::store::write_atomic(&path, text.as_bytes())
        .map_err(|err| format!("{}: {err}", path.display()))
}

fn now_ms() -> u64 {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH);
    now.map_or(0, |d| d.as_millis() as u64)
}

/// The `version` of the npm `latest` document at `url`.
pub fn fetch_latest(http: &dyn Transport, url: &str) -> Result<String, String> {
    let agent = format!("fast-harness/{}", env!("CARGO_PKG_VERSION"));
    let headers = [
        ("accept", "application/json"),
        ("user-agent", agent.as_str()),
    ];
    let req = HttpRequest::new("GET", url, &headers, b"").with_timeout_ms(FETCH_TIMEOUT_MS);
    let mut body = Vec::new();
    let head = http
        .send(&req, &mut |_, chunk| {
            body.extend_from_slice(chunk);
            match body.len() > MAX_BODY {
                true => Flow::Stop,
                false => Flow::Continue,
            }
        })
        .map_err(|err| err.to_string())?;
    if !(200..300).contains(&head.status) {
        return Err(format!("{url}: HTTP {}", head.status));
    }
    let json: serde_json::Value =
        serde_json::from_slice(&body).map_err(|err| format!("{url}: {err}"))?;
    let version = json["version"].as_str().unwrap_or_default().trim();
    match parse(version) {
        Some(_) => Ok(version.to_string()),
        None => Err(format!("{url}: no release version in the answer")),
    }
}

/// Fetches the newest release, offers it and caches it in `home`; the
/// version in use afterwards. Nothing happens while a version is pinned.
pub fn update(home: &Path, http: &dyn Transport, url: &str) -> Result<String, String> {
    if pinned() {
        return Ok(version());
    }
    let latest = fetch_latest(http, url)?;
    offer(&latest);
    let now = version();
    write_cache(home, &now)?;
    Ok(now)
}

/// [`update`] from the registry on a background thread, once per process
/// (later calls do nothing while or after it runs). Requests sent after it
/// finishes use the new version. A failed fetch keeps the current version
/// silently: a TUI owns the terminal, and an HTTP 400 naming a newer
/// version still corrects it.
pub fn update_in_background(home: PathBuf, http: Arc<dyn Transport>) {
    if pinned() || UPDATING.swap(true, Ordering::SeqCst) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("claude-code-version".into())
        .spawn(move || {
            let _ = update(&home, http.as_ref(), LATEST_URL);
        });
    if spawned.is_err() {
        UPDATING.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
#[path = "claude_code_tests.rs"]
mod tests;
