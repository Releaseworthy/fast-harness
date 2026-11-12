//! The tool index of MCP servers (PLAN.md D47): the last `tools/list` of a
//! server in `<home>/mcp-cache/<server>.json`, fresh for its `ttlMs`. The
//! index of a session is read from these files only: no server starts.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::config::ServerSpec;
use crate::llm::ToolSpec;

pub const CACHE_DIR: &str = "mcp-cache";
/// Freshness of a list without `ttlMs` (servers before 2026-07-28).
pub const DEFAULT_TTL_MS: u64 = 3_600_000;
/// Most bytes of a tool description sent to the model.
pub const DESCRIPTION_BYTES: usize = 1024;
/// Most characters of a tool name (the strictest provider rule, Gemini).
pub const NAME_CHARS: usize = 64;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheFile {
    pub version: u32,
    pub server: String,
    /// [`ServerSpec::fingerprint`] of the server that answered.
    pub fingerprint: String,
    /// Unix milliseconds of the `tools/list`.
    pub fetched_at: u64,
    #[serde(default)]
    pub ttl_ms: Option<u64>,
    #[serde(default)]
    pub cache_scope: Option<String>,
    /// `modern` (2026-07-28) or `legacy` (the `initialize` handshake).
    #[serde(default)]
    pub era: String,
    /// The tools as the server listed them.
    pub tools: Vec<Value>,
}

impl CacheFile {
    /// Still within its `ttlMs` at `now` (Unix milliseconds).
    pub fn fresh(&self, now: u64) -> bool {
        let ttl = self.ttl_ms.unwrap_or(DEFAULT_TTL_MS);
        now < self.fetched_at.saturating_add(ttl)
    }

    /// The raw tool object named `tool`.
    pub fn tool(&self, tool: &str) -> Option<&Value> {
        (self.tools.iter()).find(|item| item.get("name").and_then(Value::as_str) == Some(tool))
    }
}

pub fn path(home: &Path, server: &str) -> PathBuf {
    home.join(CACHE_DIR).join(format!("{server}.json"))
}

/// The cache of `spec`; `None` when missing, unreadable or written for
/// another command or URL.
pub fn read(home: &Path, spec: &ServerSpec) -> Option<CacheFile> {
    let bytes = std::fs::read(path(home, &spec.name)).ok()?;
    let file: CacheFile = serde_json::from_slice(&bytes).ok()?;
    (file.fingerprint == spec.fingerprint() && file.server == spec.name).then_some(file)
}

/// Writes `file` through a temporary file and a rename.
pub fn write(home: &Path, file: &CacheFile) -> Result<(), String> {
    let target = path(home, &file.server);
    let dir = home.join(CACHE_DIR);
    std::fs::create_dir_all(&dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    let text = serde_json::to_vec_pretty(file).map_err(|err| err.to_string())?;
    let tmp = target.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|err| format!("{}: {err}", tmp.display()))?;
    std::fs::rename(&tmp, &target).map_err(|err| format!("{}: {err}", target.display()))
}

/// One MCP tool of the index under its model-facing name.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexTool {
    /// `mcp__<server>__<tool>`, sanitised.
    pub name: String,
    pub server: String,
    /// The tool's own name on its server.
    pub tool: String,
    pub spec: ToolSpec,
    /// The description before the 1 KB cut (`mcp_describe`).
    pub full_description: String,
}

/// The tools of `file` in the server's order; names made unique.
pub fn index_of(file: &CacheFile) -> Vec<IndexTool> {
    let mut out: Vec<IndexTool> = Vec::new();
    for raw in &file.tools {
        let Some(tool) = raw.get("name").and_then(Value::as_str) else {
            continue;
        };
        let mut name = sanitize(&file.server, tool);
        if out.iter().any(|seen| seen.name == name) {
            name = with_hash(&name, &format!("{}\n{tool}", file.server));
        }
        let full = raw.get("description").and_then(Value::as_str).unwrap_or("");
        let schema = raw
            .get("inputSchema")
            .filter(|schema| schema.is_object())
            .cloned()
            .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
        out.push(IndexTool {
            spec: ToolSpec {
                name: name.clone(),
                description: cut_description(full),
                schema,
            },
            name,
            server: file.server.clone(),
            tool: tool.to_string(),
            full_description: full.to_string(),
        });
    }
    out
}

/// `mcp__<server>__<tool>` with every character outside `[A-Za-z0-9_-]`
/// made `_`; longer than [`NAME_CHARS`]: cut, then `_` and a hash.
pub fn sanitize(server: &str, tool: &str) -> String {
    let raw = format!("mcp__{server}__{tool}");
    let clean: String = raw
        .chars()
        .map(
            |c| match c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                true => c,
                false => '_',
            },
        )
        .collect();
    match clean.len() > NAME_CHARS {
        true => with_hash(&clean, &raw),
        false => clean,
    }
}

/// `name` cut to leave room for `_` and 8 hex digits of FNV-1a of `key`.
fn with_hash(name: &str, key: &str) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in key.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    let keep: String = name.chars().take(NAME_CHARS - 9).collect();
    format!("{keep}_{hash:08x}")
}

/// `text` cut to [`DESCRIPTION_BYTES`] on a character boundary.
pub fn cut_description(text: &str) -> String {
    if text.len() <= DESCRIPTION_BYTES {
        return text.to_string();
    }
    let mut end = DESCRIPTION_BYTES - 3;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_provider_rules() {
        assert_eq!(sanitize("gh", "list_issues"), "mcp__gh__list_issues");
        assert_eq!(sanitize("gh", "a.b/c d"), "mcp__gh__a_b_c_d");
        let long = sanitize("server", &"x".repeat(100));
        assert_eq!(long.len(), NAME_CHARS);
        assert_ne!(long, sanitize("server", &"x".repeat(101)));
        assert!(long.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    }

    #[test]
    fn descriptions_are_cut_to_one_kilobyte() {
        assert_eq!(cut_description("short"), "short");
        let cut = cut_description(&"é".repeat(2000));
        assert!(cut.len() <= DESCRIPTION_BYTES && cut.ends_with("..."));
    }

    #[test]
    fn duplicate_sanitised_names_get_a_hash() {
        let file = CacheFile {
            server: "s".into(),
            tools: vec![json!({"name": "a.b"}), json!({"name": "a/b"})],
            ..CacheFile::default()
        };
        let index = index_of(&file);
        assert_eq!(index[0].name, "mcp__s__a_b");
        assert_ne!(index[1].name, index[0].name);
        assert_eq!(index[1].tool, "a/b");
    }
}
