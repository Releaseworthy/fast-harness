//! The methods of `fast-harness serve` besides the prompt ones (PLAN.md
//! D48): ACP v2 `initialize`, `session/new`, `session/resume`,
//! `session/list`, `session/close`, `session/cancel`,
//! `session/set_config_option`, and our `_fh/pause` and `_fh/stats`.
//! <https://agentclientprotocol.com/protocol/v2/session-setup.md>,
//! <https://agentclientprotocol.com/protocol/v2/session-list.md>,
//! <https://agentclientprotocol.com/protocol/v2/session-config-options.md>

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use super::rpc::{Reply, RpcError, Server, METHOD_NOT_FOUND};
use super::{acp, config, json as events, prompt, Session};
use crate::agent::Input;
use crate::session::{self, SessionSummary};

/// The ACP protocol version this server speaks.
pub const PROTOCOL_VERSION: u32 = 2;

/// Our methods, advertised in `initialize`.
pub const EXTENSIONS: [&str; 7] = [
    "_fh/approve",
    "_fh/steer",
    "_fh/follow_up",
    "_fh/pause",
    "_fh/continue",
    "_fh/compact",
    "_fh/stats",
];

pub(super) fn call(server: &Arc<Server>, method: &str, params: Value, id: Option<Value>) -> Reply {
    let result = match method {
        "initialize" => Ok(initialize()),
        "session/new" => new_session(server, &params),
        "session/resume" => resume(server, &params),
        "session/list" => list(server, &params),
        "session/close" => close(server, &params),
        "session/cancel" => cancel(server, &params),
        "session/set_config_option" => set_option(server, &params),
        "_fh/pause" => pause(server, &params),
        "_fh/stats" => stats(server, &params),
        "_fh/approve" => super::approve::method(server, &params),
        "session/prompt" | "_fh/steer" | "_fh/follow_up" | "_fh/compact" | "_fh/continue" => {
            return prompt::call(server, method, &params, id)
        }
        _ => Err(RpcError::new(
            METHOD_NOT_FOUND,
            format!("unknown method {method}"),
        )),
    };
    Reply::Now(result)
}

pub(super) fn text_param<'a>(params: &'a Value, key: &str) -> Result<&'a str, RpcError> {
    (params.get(key).and_then(Value::as_str))
        .ok_or_else(|| RpcError::params(format!("{key} is required")))
}

pub(super) fn session_of(server: &Server, params: &Value) -> Result<Arc<Session>, RpcError> {
    Ok(server.service.get(text_param(params, "sessionId")?)?)
}

fn cwd_param(params: &Value) -> Result<PathBuf, RpcError> {
    let cwd = PathBuf::from(text_param(params, "cwd")?);
    if !cwd.is_absolute() {
        return Err(RpcError::params("cwd must be an absolute path"));
    }
    Ok(cwd)
}

fn initialize() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {"session": {}, "_meta": {"fh/methods": EXTENSIONS, "fh/wait": true}},
        "info": {"name": "fast-harness", "title": "Fast Harness",
            "version": env!("CARGO_PKG_VERSION")}
    })
}

fn options(server: &Server, session: &Session) -> Value {
    config::options(
        &server.service.host().catalog(&session.cwd),
        &session.config(),
        server.service.host().models(),
    )
}

/// MCP servers come from `mcp.json` (PLAN.md D47); ACP asks to reject a
/// transport that was not advertised.
fn no_mcp(params: &Value) -> Result<(), RpcError> {
    match params.get("mcpServers").and_then(Value::as_array) {
        Some(list) if !list.is_empty() => Err(RpcError::params(
            "client MCP servers are not supported: add them to mcp.json",
        )),
        _ => Ok(()),
    }
}

fn new_session(server: &Server, params: &Value) -> Result<Value, RpcError> {
    let cwd = cwd_param(params)?;
    no_mcp(params)?;
    let session = server.service.create(&cwd)?;
    session.set_answering(true);
    Ok(json!({"sessionId": session.id, "configOptions": options(server, &session)}))
}

fn resume(server: &Server, params: &Value) -> Result<Value, RpcError> {
    let (cwd, id) = (cwd_param(params)?, text_param(params, "sessionId")?);
    no_mcp(params)?;
    let replay = match params.get("replayFrom") {
        None | Some(Value::Null) => false,
        Some(cursor) if cursor["type"] == "start" => true,
        Some(_) => return Err(RpcError::params("replayFrom: only {\"type\": \"start\"}")),
    };
    let session = server.service.resume(&cwd, id)?;
    session.set_answering(true);
    if replay {
        for update in session.with_conv(|conv| acp::replay(&conv.messages))? {
            server.notify(id, update);
        }
    }
    let snapshot = match (session.running(), session.last_outcome()) {
        (true, _) => acp::state("running", None, None),
        (false, last) => acp::state("idle", last.map(events::stop_reason), None),
    };
    server.notify(id, snapshot);
    Ok(json!({"configOptions": options(server, &session)}))
}

/// `yyyy-mm-ddThh:mm:ssZ` of Unix milliseconds.
fn rfc3339(ms: u64) -> String {
    let s = session::utc_stamp(ms / 1000);
    let at = |range: std::ops::Range<usize>| s.get(range).unwrap_or("00");
    format!(
        "{}-{}-{}T{}:{}:{}Z",
        at(0..4),
        at(4..6),
        at(6..8),
        at(9..11),
        at(11..13),
        at(13..15)
    )
}

fn info(server: &Server, cwd: &Path, item: &SessionSummary) -> Value {
    let open = server.service.get(&item.id).is_ok();
    json!({"sessionId": item.id, "cwd": cwd, "title": item.title,
        "updatedAt": rfc3339(item.last_time),
        "_meta": {"fh/turns": item.turns, "fh/unfinished": item.unfinished.is_some(),
            "fh/open": open}})
}

fn list(server: &Server, params: &Value) -> Result<Value, RpcError> {
    if params.get("cursor").is_some_and(|c| !c.is_null()) {
        return Err(RpcError::params(
            "invalid cursor (this agent returns one page)",
        ));
    }
    let cwds = match params.get("cwd").filter(|c| !c.is_null()) {
        Some(_) => vec![cwd_param(params)?],
        None => session::known_cwds(&server.service.host().home())
            .map_err(|err| RpcError::new(super::rpc::INTERNAL_ERROR, err.to_string()))?
            .into_iter()
            .map(|known| known.cwd)
            .collect(),
    };
    let mut rows: Vec<(u64, Value)> = Vec::new();
    for cwd in cwds {
        for item in server.service.list(&cwd)? {
            rows.push((item.last_time, info(server, &cwd, &item)));
        }
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row.0));
    let sessions: Vec<Value> = rows.into_iter().map(|(_, row)| row).collect();
    Ok(json!({"sessions": sessions}))
}

fn close(server: &Server, params: &Value) -> Result<Value, RpcError> {
    server.service.close(text_param(params, "sessionId")?);
    Ok(json!({}))
}

/// A notification: an unknown or idle session is ignored.
fn cancel(server: &Server, params: &Value) -> Result<Value, RpcError> {
    if let Ok(session) = session_of(server, params) {
        session.cancel();
    }
    Ok(json!({}))
}

fn set_option(server: &Server, params: &Value) -> Result<Value, RpcError> {
    let session = session_of(server, params)?;
    let key = text_param(params, "configId")?;
    let value = params.get("value").cloned().unwrap_or(Value::Null);
    if params["type"] == "boolean" && !value.is_boolean() {
        return Err(RpcError::params("type boolean needs a boolean value"));
    }
    server.service.set_config(&session.id, key, &value)?;
    let list = options(server, &session);
    let update = json!({"sessionUpdate": "config_option_update", "configOptions": list});
    server.notify(&session.id, update);
    Ok(json!({"configOptions": list}))
}

fn pause(server: &Server, params: &Value) -> Result<Value, RpcError> {
    let session = session_of(server, params)?;
    Ok(json!({"paused": session.send(Input::Pause)}))
}

fn stats(server: &Server, params: &Value) -> Result<Value, RpcError> {
    let session = session_of(server, params)?;
    let numbers = server.service.stats(&session.id)?;
    serde_json::to_value(numbers)
        .map_err(|err| RpcError::new(super::rpc::INTERNAL_ERROR, err.to_string()))
}
