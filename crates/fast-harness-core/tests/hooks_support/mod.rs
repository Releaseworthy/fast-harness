//! Shared helpers of the hooks tests (PLAN.md D44): hook entries running
//! small cmd (Windows) or sh scripts that save their stdin and answer with
//! the text of a file.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::config::layers::Layers;
use fast_harness_core::config::Config;
use fast_harness_core::events::{AgentEvent, TurnOutcome};
use fast_harness_core::http::ScriptedTransport;
use fast_harness_core::tools::hooks::Hooks;
use fast_harness_core::tools::Extensions;
use serde_json::{json, Value};

use crate::agent_support::{echo_registry, events, key_auth, parts, Env};

/// A hook entry running `script` (cmd parts on Windows, one sh line elsewhere).
pub fn entry(matcher: Option<&str>, script: Vec<String>, timeout_ms: u64) -> Value {
    let (command, mut args) = match cfg!(windows) {
        true => ("cmd", vec!["/c".to_string()]),
        false => ("sh", vec!["-c".to_string()]),
    };
    match cfg!(windows) {
        true => args.extend(script),
        false => args.push(script.join(" ")),
    }
    let mut value = json!({"command": command, "args": args, "timeoutMs": timeout_ms});
    if let Some(pattern) = matcher {
        value["match"] = json!(pattern);
    }
    value
}

/// A script saving its stdin to `<dir>/<name>.in` and printing `answer`
/// (nothing when `None`); returns the saved file and the script.
pub fn capture(dir: &Path, name: &str, answer: Option<&str>) -> (PathBuf, Vec<String>) {
    let seen = dir.join(format!("{name}.in"));
    let out = dir.join(format!("{name}.out"));
    if let Some(answer) = answer {
        std::fs::write(&out, answer).unwrap();
    }
    let (seen_s, out_s) = (seen.display().to_string(), out.display().to_string());
    let script = match (cfg!(windows), answer.is_some()) {
        (true, true) => vec![
            "more".into(),
            ">".into(),
            seen_s,
            "&".into(),
            "type".into(),
            out_s,
        ],
        (true, false) => vec!["more".into(), ">".into(), seen_s],
        (false, true) => vec![format!("cat > '{seen_s}'; cat '{out_s}'")],
        (false, false) => vec![format!("cat > '{seen_s}'")],
    };
    (seen, script)
}

/// A script that only prints `answer`.
pub fn answer(dir: &Path, name: &str, answer: &str) -> Vec<String> {
    let out = dir.join(format!("{name}.out"));
    std::fs::write(&out, answer).unwrap();
    match cfg!(windows) {
        true => vec!["type".into(), out.display().to_string()],
        false => vec![format!("cat '{}'", out.display())],
    }
}

pub fn write_hooks(home: &Path, hooks: Value) {
    std::fs::write(home.join("hooks.json"), hooks.to_string()).unwrap();
}

pub fn read_json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).unwrap();
    serde_json::from_str(text.trim()).unwrap_or_else(|err| panic!("{err}: {text}"))
}

/// An agent with the echo tool and the user layer's hooks.
pub fn agent(env: &Env, transport: &Arc<ScriptedTransport>, config: Config) -> Agent {
    let mut parts = parts(transport, key_auth(), &env.cwd);
    parts.config = config;
    let (hooks, skipped) = Hooks::load(&Layers::user_only(&env.home));
    assert!(skipped.is_empty(), "{skipped:?}");
    parts.extensions = Arc::new(Extensions::default().with_hooks(hooks));
    Agent::new(parts, echo_registry())
}

pub fn turn(agent: &Agent, conv: &mut Conversation, text: &str) -> (TurnOutcome, Vec<AgentEvent>) {
    let cancel = AtomicBool::new(false);
    let mut seen = Vec::new();
    let outcome = agent.run_turn(conv, text, &cancel, &mut |event| seen.push(event));
    (outcome, seen)
}

/// Model call bodies without `cache_control` and without the trailing
/// Claude effort entry (`role: system`).
pub fn bodies(transport: &ScriptedTransport) -> Vec<Value> {
    (transport.requests().iter())
        .filter(|r| r.url.contains("/messages"))
        .map(|r| strip(&serde_json::from_str(&r.body).unwrap()))
        .collect()
}

fn strip(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            (map.iter())
                .filter(|(key, _)| key.as_str() != "cache_control")
                .map(|(key, item)| (key.clone(), strip(item)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            (items.iter())
                .filter(|item| item["role"] != "system")
                .map(strip)
                .collect(),
        ),
        other => other.clone(),
    }
}

/// The data of the trace events of `kind`.
pub fn traced(conv: &Conversation, kind: &str) -> Vec<Value> {
    (events(&conv.log_path).into_iter())
        .filter(|e| e.kind == kind)
        .map(|e| e.data)
        .collect()
}
