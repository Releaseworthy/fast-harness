//! JSON-RPC messages of MCP (PLAN.md D47). Modern servers (2026-07-28,
//! stateless) get the protocol version, client info and capabilities in
//! every request's `_meta`; legacy servers (2025-11-25 and earlier) get the
//! `initialize` handshake once per process (stdio) or session (HTTP).
//! Specs: <https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning>,
//! <https://modelcontextprotocol.io/specification/2026-07-28/server/discover>.

use serde_json::{json, Map, Value};

use crate::tools::ToolOutput;

pub const MODERN: &str = "2026-07-28";
/// The version a legacy `initialize` asks for.
pub const LEGACY: &str = "2025-11-25";
/// `HeaderMismatch`, `MissingRequiredClientCapability`,
/// `UnsupportedProtocolVersion`: errors only a modern server sends.
const MODERN_ERRORS: [i64; 3] = [-32020, -32021, -32022];
const UNSUPPORTED_VERSION: i64 = -32022;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Era {
    Modern,
    Legacy,
}

impl Era {
    pub fn word(self) -> &'static str {
        match self {
            Era::Modern => "modern",
            Era::Legacy => "legacy",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    Result(Value),
    Error {
        code: i64,
        message: String,
        data: Value,
    },
}

pub fn client_info() -> Value {
    json!({"name": "fast-harness", "version": env!("CARGO_PKG_VERSION")})
}

/// A request; a modern one carries the per-request `_meta`.
pub fn request(id: u64, method: &str, params: Value, era: Era) -> Value {
    let mut params = match params {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    if era == Era::Modern {
        let meta = json!({
            "io.modelcontextprotocol/protocolVersion": MODERN,
            "io.modelcontextprotocol/clientInfo": client_info(),
            "io.modelcontextprotocol/clientCapabilities": {},
        });
        params.insert("_meta".into(), meta);
    }
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

/// The legacy handshake request.
pub fn initialize(id: u64) -> Value {
    let params =
        json!({"protocolVersion": LEGACY, "capabilities": {}, "clientInfo": client_info()});
    json!({"jsonrpc": "2.0", "id": id, "method": "initialize", "params": params})
}

pub fn notification(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "method": method, "params": params})
}

/// The answer to a server's own request (legacy servers may ask for roots
/// or sampling): this client offers none.
pub fn refuse(msg: &Value) -> Option<Value> {
    let id = msg.get("id")?;
    msg.get("method")?;
    let error = json!({"code": -32601, "message": "not supported by this client"});
    Some(json!({"jsonrpc": "2.0", "id": id, "error": error}))
}

/// The reply in `msg` when it answers request `id`.
pub fn reply_to(msg: &Value, id: u64) -> Option<Reply> {
    if msg.get("id").and_then(Value::as_u64) != Some(id) || msg.get("method").is_some() {
        return None;
    }
    reply_of(msg)
}

/// A JSON-RPC response as a [`Reply`]; `None` for other messages.
pub fn reply_of(msg: &Value) -> Option<Reply> {
    if let Some(result) = msg.get("result") {
        return Some(Reply::Result(result.clone()));
    }
    let error = msg.get("error")?;
    Some(Reply::Error {
        code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
        message: (error.get("message").and_then(Value::as_str))
            .unwrap_or("")
            .to_string(),
        data: error.get("data").cloned().unwrap_or(Value::Null),
    })
}

/// The era a `server/discover` probe shows: a result or a modern error
/// that still accepts [`MODERN`] is modern; any other error, a silence or
/// an unsupported-version error that lists only older versions is legacy.
pub fn probe_era(reply: Option<&Reply>) -> Era {
    match reply {
        Some(Reply::Result(_)) => Era::Modern,
        Some(Reply::Error { code, data, .. }) if *code == UNSUPPORTED_VERSION => {
            let supported = data.get("supported").and_then(Value::as_array);
            match supported.is_some_and(|list| list.iter().any(|v| v == MODERN)) {
                true => Era::Modern,
                false => Era::Legacy,
            }
        }
        Some(Reply::Error { code, .. }) if MODERN_ERRORS.contains(code) => Era::Modern,
        _ => Era::Legacy,
    }
}

/// A reply's result, or its error as text.
pub fn result(reply: Reply) -> Result<Value, String> {
    match reply {
        Reply::Result(value) => Ok(value),
        Reply::Error { code, message, .. } => Err(format!("server error {code}: {message}")),
    }
}

/// The JSON-RPC messages of an SSE body: the `data` lines of each event
/// joined; comments and other fields are skipped.
pub fn sse_messages(body: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let mut data = String::new();
    for line in body.lines().chain(std::iter::once("")) {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            if let Ok(value) = serde_json::from_str::<Value>(&data) {
                out.push(value);
            }
            data.clear();
        } else if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    out
}

/// A `tools/call` result as a tool output: the text blocks joined, other
/// blocks named in a note, `structuredContent` when there is no content.
pub fn tool_output(result: &Value) -> ToolOutput {
    if result.get("resultType").and_then(Value::as_str) == Some("input_required") {
        return ToolOutput::err(
            "the MCP server asked for user input, which this client does not support",
        );
    }
    let is_error = result.get("isError").and_then(Value::as_bool) == Some(true);
    let blocks = result.get("content").and_then(Value::as_array);
    let mut parts: Vec<String> = blocks.into_iter().flatten().map(block_text).collect();
    if parts.is_empty() {
        if let Some(structured) = result.get("structuredContent") {
            parts.push(structured.to_string());
        }
    }
    ToolOutput::new(parts.join("\n"), is_error)
}

fn block_text(block: &Value) -> String {
    let field = |name: &str| block.get(name).and_then(Value::as_str).unwrap_or("");
    match field("type") {
        "text" => field("text").to_string(),
        "image" | "audio" => format!(
            "[{} {} of {} base64 bytes, not shown]",
            field("type"),
            field("mimeType"),
            field("data").len()
        ),
        "resource" => {
            let resource = block.get("resource").cloned().unwrap_or(Value::Null);
            let uri = resource.get("uri").and_then(Value::as_str).unwrap_or("");
            match resource.get("text").and_then(Value::as_str) {
                Some(text) => format!("[resource {uri}]\n{text}"),
                None => format!("[resource {uri}: binary, not shown]"),
            }
        }
        "resource_link" => format!("[resource link {} {}]", field("uri"), field("name")),
        other => format!("[{other} content, not shown]"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_tells_the_eras_apart() {
        assert_eq!(probe_era(Some(&Reply::Result(json!({})))), Era::Modern);
        assert_eq!(probe_era(None), Era::Legacy);
        let unknown = Reply::Error {
            code: -32601,
            message: "Method not found".into(),
            data: Value::Null,
        };
        assert_eq!(probe_era(Some(&unknown)), Era::Legacy);
        let old_only = Reply::Error {
            code: -32022,
            message: String::new(),
            data: json!({"supported": ["2025-11-25"]}),
        };
        assert_eq!(probe_era(Some(&old_only)), Era::Legacy);
    }

    #[test]
    fn sse_and_content_blocks() {
        let body = ": keep\n\nevent: message\ndata: {\"a\":1}\n\ndata: {\"id\":2,\r\ndata: \"result\":{}}\r\n";
        let messages = sse_messages(body);
        assert_eq!(messages.len(), 2);
        assert_eq!(reply_to(&messages[1], 2), Some(Reply::Result(json!({}))));
        let result = json!({"content": [
            {"type": "text", "text": "hi"},
            {"type": "image", "mimeType": "image/png", "data": "AAAA"},
            {"type": "resource", "resource": {"uri": "file:///a", "text": "body"}}
        ], "isError": true});
        let output = tool_output(&result);
        assert!(output.is_error());
        assert!(output.text().starts_with("hi\n[image image/png of 4"));
        assert!(output.text().ends_with("[resource file:///a]\nbody"));
    }
}
