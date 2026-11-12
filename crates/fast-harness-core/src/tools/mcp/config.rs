//! `mcp.json` (PLAN.md D47) in the user folder and a trusted project
//! folder: `{"servers": {"<name>": {"command", "args", "env"} | {"url",
//! "headers"}, "enabled", "alwaysLoad", "timeoutMs"}}`. A later layer's
//! server of the same name replaces the earlier one. Values may name
//! environment variables as `${VAR}`; they are expanded only when a server
//! starts, so the settings, traces and listings keep the template.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::config::ext_file;
use crate::config::layers::{LayerFile, Layers, Skipped, Source};

pub const MCP_FILE: &str = "mcp.json";
/// Default limit of one request to a server.
pub const DEFAULT_TIMEOUT_MS: u64 = 120_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Endpoint {
    /// A local process speaking JSON-RPC lines on stdin and stdout.
    Stdio {
        command: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        /// The folder of the `mcp.json` that named it (relative commands).
        dir: PathBuf,
    },
    /// A Streamable HTTP endpoint.
    Http {
        url: String,
        headers: Vec<(String, String)>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerSpec {
    pub name: String,
    pub endpoint: Endpoint,
    pub enabled: bool,
    /// Its tools are preloaded in a top-level session (`tools.preload`).
    pub always_load: bool,
    pub timeout_ms: u64,
    pub source: Source,
}

impl ServerSpec {
    /// `stdio` or `http`.
    pub fn kind(&self) -> &'static str {
        match self.endpoint {
            Endpoint::Stdio { .. } => "stdio",
            Endpoint::Http { .. } => "http",
        }
    }

    /// FNV-1a of the command and arguments or the URL: a cache written for
    /// another program is not used. Templates only, never expanded values.
    pub fn fingerprint(&self) -> String {
        let text = match &self.endpoint {
            Endpoint::Stdio { command, args, .. } => {
                format!("stdio\n{command}\n{}", args.join("\n"))
            }
            Endpoint::Http { url, .. } => format!("http\n{url}"),
        };
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in text.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        format!("{hash:016x}")
    }

    /// What `doctor` and `mcp list` show: the command or URL, never the
    /// values of `env` or `headers` (names only).
    pub fn target(&self) -> String {
        match &self.endpoint {
            Endpoint::Stdio {
                command, args, env, ..
            } => {
                let mut text = command.clone();
                for arg in args {
                    text.push(' ');
                    text.push_str(arg);
                }
                names_note(&mut text, "env", env);
                text
            }
            Endpoint::Http { url, headers } => {
                let mut text = url.clone();
                names_note(&mut text, "headers", headers);
                text
            }
        }
    }
}

fn names_note(text: &mut String, what: &str, pairs: &[(String, String)]) {
    if !pairs.is_empty() {
        let names: Vec<&str> = pairs.iter().map(|(name, _)| name.as_str()).collect();
        text.push_str(&format!(" ({what}: {})", names.join(", ")));
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawServer {
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    url: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    enabled: Option<bool>,
    #[serde(default)]
    always_load: bool,
    timeout_ms: Option<u64>,
}

#[derive(Deserialize)]
struct RawFile {
    #[serde(default)]
    servers: BTreeMap<String, RawServer>,
}

/// The servers of every layer's `mcp.json`, sorted by name (a stable tool
/// order), the files that loaded and the files or servers skipped. No file:
/// one failed read per folder and nothing else.
pub fn load(layers: &Layers) -> (Vec<ServerSpec>, Vec<LayerFile>, Vec<Skipped>) {
    let mut servers: BTreeMap<String, ServerSpec> = BTreeMap::new();
    let (mut files, mut skipped) = (Vec::new(), Vec::new());
    for (dir, source) in layers.dirs() {
        let path = dir.join(MCP_FILE);
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let raw: RawFile = match read_file(&bytes) {
            Ok(raw) => raw,
            Err(reason) => {
                skipped.push(Skipped { path, reason });
                continue;
            }
        };
        for (name, server) in raw.servers {
            match spec(&name, server, &dir, source) {
                Ok(spec) => drop(servers.insert(name, spec)),
                Err(reason) => skipped.push(Skipped {
                    path: path.clone(),
                    reason: format!("server `{name}`: {reason}"),
                }),
            }
        }
        files.push(LayerFile { path, source });
    }
    (servers.into_values().collect(), files, skipped)
}

/// One `mcp.json` with the shared `version` and `enabled` rules.
fn read_file(bytes: &[u8]) -> Result<RawFile, String> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    ext_file::on(ext_file::json_header(&value)?)?;
    serde_json::from_value(value).map_err(|e| e.to_string())
}

fn spec(name: &str, raw: RawServer, dir: &Path, source: Source) -> Result<ServerSpec, String> {
    if ext_file::check_name(name).is_err() {
        return Err("a name uses only letters, digits, `_` and `-`".into());
    }
    let endpoint = match (raw.command, raw.url) {
        (Some(command), None) => Endpoint::Stdio {
            command,
            args: raw.args,
            env: raw.env.into_iter().collect(),
            dir: dir.to_path_buf(),
        },
        (None, Some(url)) if url.starts_with("http://") || url.starts_with("https://") => {
            Endpoint::Http {
                url,
                headers: raw.headers.into_iter().collect(),
            }
        }
        (None, Some(_)) => return Err("`url` must start with http:// or https://".into()),
        _ => return Err("give either `command` or `url`".into()),
    };
    Ok(ServerSpec {
        name: name.to_string(),
        endpoint,
        enabled: raw.enabled.unwrap_or(true),
        always_load: raw.always_load,
        timeout_ms: raw.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS).max(1),
        source,
    })
}

/// `text` with every `${VAR}` replaced by the variable; a missing variable
/// is an error naming it (never a value).
pub fn expand(text: &str, env: &dyn Fn(&str) -> Option<String>) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            return Err("an unclosed `${` in a value".into());
        };
        let var = &after[..end];
        let value = env(var).ok_or_else(|| format!("set the {var} environment variable"))?;
        out.push_str(&value);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// The process environment, as [`expand`] reads it.
pub fn process_env(var: &str) -> Option<String> {
    std::env::var(var).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_replaces_variables_and_names_missing_ones() {
        let env = |var: &str| (var == "TOKEN").then(|| "s3cret".to_string());
        assert_eq!(expand("Bearer ${TOKEN}", &env).unwrap(), "Bearer s3cret");
        assert_eq!(expand("plain", &env).unwrap(), "plain");
        let err = expand("${NOPE}", &env).unwrap_err();
        assert!(err.contains("NOPE") && !err.contains("s3cret"));
        assert!(expand("${OPEN", &env).is_err());
    }
}
