//! `fast-harness config`, `trust`, `doctor` (PLAN.md D41) and `keys` (D45).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use std::sync::Arc;

use fast_harness_core::app::{doctor as doctor_text, load_settings, mcp_lines};
use fast_harness_core::config::layers::{project_root, PROJECT_DIR};
use fast_harness_core::config::{save_setting, trust as trusted, FILE_NAME};
use fast_harness_core::http::Transport;
use fast_harness_core::net::UreqTransport;
use fast_harness_core::tools::mcp::McpServers;
use serde_json::{Map, Value};

use crate::args::ConfigAction;

fn env_value(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

pub fn config(action: ConfigAction, cwd: Option<PathBuf>) -> Result<ExitCode, String> {
    let (home, cwd) = (crate::home()?, crate::work_dir(cwd)?);
    match action {
        ConfigAction::Path => {
            println!("user    {}", home.join(FILE_NAME).display());
            if let Some(root) = project_root(&home, &cwd) {
                let state = match trusted::is_trusted(&home, &root) {
                    true => "trusted",
                    false => "not trusted",
                };
                let file = root.join(PROJECT_DIR).join(FILE_NAME);
                println!("project {} ({state})", file.display());
            }
        }
        ConfigAction::Get(key) => {
            let settings = load_settings(&home, Some(&cwd), &env_value)?;
            let (value, source) =
                (settings.layered.get(&key)).ok_or_else(|| format!("unknown setting `{key}`"))?;
            println!("{value}  ({source})");
        }
        ConfigAction::Set {
            key,
            value,
            project,
        } => set(&home, &cwd, &key, &value, project)?,
    }
    Ok(ExitCode::SUCCESS)
}

/// Writes dotted `key` into the user (or project) `config.json`; `value`
/// is read as JSON, else taken as a string.
fn set(home: &Path, cwd: &Path, key: &str, value: &str, project: bool) -> Result<(), String> {
    let dir = match project {
        true => project_root(home, cwd)
            .unwrap_or_else(|| cwd.to_path_buf())
            .join(PROJECT_DIR),
        false => home.to_path_buf(),
    };
    let value: Value =
        serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.to_string()));
    let mut parts = key.split('.');
    let top = parts.next().filter(|t| !t.is_empty()).ok_or("empty key")?;
    let rest: Vec<&str> = parts.collect();
    let current = std::fs::read(dir.join(FILE_NAME))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|root| root.get(top).cloned());
    let top_value = nest(current, &rest, value);
    save_setting(&dir, top, top_value)?;
    if load_settings(home, Some(cwd), &env_value).is_err() && !project {
        eprintln!("[note] config.json no longer loads; check the value of `{key}`");
    }
    println!("{key} set in {}", dir.join(FILE_NAME).display());
    if project && !project_root(home, cwd).is_some_and(|root| trusted::is_trusted(home, &root)) {
        eprintln!("[note] the project is not trusted: run `fast-harness trust` to load it");
    }
    Ok(())
}

/// `value` placed at `path` inside `current` (an object made when missing).
fn nest(current: Option<Value>, path: &[&str], value: Value) -> Value {
    let Some((first, rest)) = path.split_first() else {
        return value;
    };
    let mut object = match current {
        Some(Value::Object(object)) => object,
        _ => Map::new(),
    };
    let inner = nest(object.remove(*first), rest, value);
    object.insert(first.to_string(), inner);
    Value::Object(object)
}

pub fn trust(path: Option<PathBuf>) -> Result<ExitCode, String> {
    let home = crate::home()?;
    let dir = crate::work_dir(path)?;
    // The same folder `/trust` trusts (D62).
    let root = fast_harness_core::config::layers::trust_root(&home, &dir);
    let path = trusted::trust(&home, &root)?;
    println!("trusted {}", path.display());
    Ok(ExitCode::SUCCESS)
}

/// `fast-harness mcp list`: the servers with their cache; `mcp refresh
/// [SERVER]`: starts each (or one) enabled server, saves its `tools/list`
/// and stops it (PLAN.md D47).
pub fn mcp(
    refresh: bool,
    server: Option<String>,
    cwd: Option<PathBuf>,
) -> Result<ExitCode, String> {
    let (home, cwd) = (crate::home()?, crate::work_dir(cwd)?);
    let settings = load_settings(&home, Some(&cwd), &env_value)?;
    if !refresh {
        print!("{}", mcp_lines(&home, &settings.mcp));
        return Ok(ExitCode::SUCCESS);
    }
    let names: Vec<String> = (settings.mcp.iter())
        .filter(|spec| spec.enabled || server.as_deref() == Some(spec.name.as_str()))
        .filter(|spec| server.as_deref().is_none_or(|name| name == spec.name))
        .map(|spec| spec.name.clone())
        .collect();
    if let (Some(name), true) = (&server, names.is_empty()) {
        return Err(format!("no MCP server `{name}` in mcp.json"));
    }
    let transport: Arc<dyn Transport> = Arc::new(UreqTransport::new());
    let servers = McpServers::new(&home, settings.mcp, Some(transport));
    let mut failed = false;
    for name in names {
        match servers.refresh(&name) {
            Ok(count) => println!("{name}: {count} tools"),
            Err(err) => {
                failed = true;
                eprintln!("{name}: {err}");
            }
        }
        servers.stop_all();
    }
    Ok(if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

pub fn doctor(cwd: Option<PathBuf>) -> Result<ExitCode, String> {
    let (home, cwd) = (crate::home()?, crate::work_dir(cwd)?);
    let settings = load_settings(&home, Some(&cwd), &env_value)?;
    print!("{}", doctor_text(&settings));
    Ok(ExitCode::SUCCESS)
}

/// `fast-harness keys [--json]`: the effective key bindings with sources.
pub fn keys(json: bool, cwd: Option<PathBuf>) -> Result<ExitCode, String> {
    let (home, cwd) = (crate::home()?, crate::work_dir(cwd)?);
    let settings = load_settings(&home, Some(&cwd), &env_value)?;
    match json {
        true => {
            let text = serde_json::to_string_pretty(&settings.keys.json());
            println!("{}", text.map_err(|err| err.to_string())?);
        }
        false => print!("{}", settings.keys.text()),
    }
    Ok(ExitCode::SUCCESS)
}
