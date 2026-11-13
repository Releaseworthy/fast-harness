//! An MCP server over Streamable HTTP (PLAN.md D47): every message is its
//! own POST, the answer a JSON object or an SSE stream ending with the
//! response. Modern requests carry `MCP-Protocol-Version`, `Mcp-Method`,
//! `Mcp-Name` and the `Mcp-Param-*` headers of `x-mcp-header` parameters; a
//! `4xx` without a modern JSON-RPC error falls back to `initialize` and the
//! legacy `Mcp-Session-Id`
//! (<https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http>).
//! Header values come from `mcp.json` with `${VAR}` expanded here and are
//! never logged.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde_json::{json, Value};

use super::config::{expand, Endpoint, ServerSpec};
use super::rpc::{self, Era, Reply, LEGACY, MODERN};
use super::{Call, Conn};
use crate::http::{Flow, HttpHead, HttpRequest, Transport};

pub struct HttpConn {
    transport: Arc<dyn Transport>,
    url: String,
    headers: Vec<(String, String)>,
    era: Era,
    /// The legacy protocol version `initialize` agreed on.
    version: String,
    /// The legacy `Mcp-Session-Id`.
    session: Option<String>,
    next: u64,
    /// The session expired and a new one failed too (D63): `alive` is false.
    dead: bool,
}

struct Posted {
    head: HttpHead,
    reply: Option<Reply>,
}

impl HttpConn {
    /// Connects to the server of `spec` and finds its protocol era.
    pub fn open(
        spec: &ServerSpec,
        transport: Arc<dyn Transport>,
        env: &dyn Fn(&str) -> Option<String>,
        call: &Call<'_>,
    ) -> Result<HttpConn, String> {
        let Endpoint::Http { url, headers } = &spec.endpoint else {
            return Err("not an HTTP server".into());
        };
        let mut expanded = Vec::new();
        for (name, value) in headers {
            expanded.push((name.to_lowercase(), expand(value, env)?));
        }
        let mut conn = HttpConn {
            transport,
            url: url.clone(),
            headers: expanded,
            era: Era::Modern,
            version: LEGACY.into(),
            session: None,
            next: 1,
            dead: false,
        };
        let id = conn.id();
        let probe = rpc::request(id, "server/discover", json!({}), Era::Modern);
        let posted = conn.post(&probe, id, &[], call)?;
        conn.era = match posted.reply {
            Some(reply) => rpc::probe_era(Some(&reply)),
            None => Era::Legacy,
        };
        if conn.era == Era::Legacy {
            conn.initialize(call)?;
        }
        Ok(conn)
    }

    fn initialize(&mut self, call: &Call<'_>) -> Result<(), String> {
        let id = self.id();
        let posted = self.post(&rpc::initialize(id), id, &[], call)?;
        self.session = posted.head.header("mcp-session-id").map(str::to_string);
        let reply = posted
            .reply
            .ok_or_else(|| format!("initialize: HTTP {}", posted.head.status))?;
        let result = rpc::result(reply).map_err(|err| format!("initialize: {err}"))?;
        if let Some(version) = result.get("protocolVersion").and_then(Value::as_str) {
            self.version = version.to_string();
        }
        let done = rpc::notification("notifications/initialized", json!({}));
        self.post(&done, 0, &[], call)?;
        Ok(())
    }

    fn id(&mut self) -> u64 {
        self.next += 1;
        self.next - 1
    }

    fn request_headers(&self, msg: &Value, extra: &[(String, String)]) -> Vec<(String, String)> {
        let mut out = vec![
            ("content-type".to_string(), "application/json".to_string()),
            (
                "accept".into(),
                "application/json, text/event-stream".into(),
            ),
        ];
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        match self.era {
            Era::Modern => {
                out.push(("mcp-protocol-version".into(), MODERN.into()));
                out.push(("mcp-method".into(), method.into()));
                let params = msg.get("params");
                let name = (params.and_then(|p| p.get("name").or_else(|| p.get("uri"))))
                    .and_then(Value::as_str);
                if let Some(name) = name.filter(|_| method != "server/discover") {
                    out.push(("mcp-name".into(), header_value(name)));
                }
                out.extend(extra.iter().cloned());
            }
            Era::Legacy if method != "initialize" => {
                out.push(("mcp-protocol-version".into(), self.version.clone()));
                if let Some(session) = &self.session {
                    out.push(("mcp-session-id".into(), session.clone()));
                }
            }
            Era::Legacy => {}
        }
        out.extend(self.headers.iter().cloned());
        out
    }

    /// POSTs `msg` and reads the reply to `id` from a JSON or SSE body.
    fn post(
        &self,
        msg: &Value,
        id: u64,
        extra: &[(String, String)],
        call: &Call<'_>,
    ) -> Result<Posted, String> {
        let headers = self.request_headers(msg, extra);
        let pairs: Vec<(&str, &str)> = headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let body = msg.to_string();
        let req = HttpRequest {
            method: "POST",
            url: &self.url,
            headers: &pairs,
            body: body.as_bytes(),
            timeout_ms: u32::try_from(call.timeout.as_millis()).unwrap_or(u32::MAX),
            cancel: Some(call.cancel),
        };
        let mut bytes = Vec::new();
        let head = (self.transport)
            .send(&req, &mut |_, chunk| {
                bytes.extend_from_slice(chunk);
                match call.cancel.load(Ordering::Relaxed) {
                    true => Flow::Stop,
                    false => Flow::Continue,
                }
            })
            .map_err(|err| match call.cancel.load(Ordering::Relaxed) {
                true => "cancelled".to_string(),
                false => format!("{}: {:?}", self.url, err.kind),
            })?;
        let text = String::from_utf8_lossy(&bytes);
        let sse = head
            .header("content-type")
            .is_some_and(|t| t.contains("text/event-stream"));
        let reply = match sse {
            true => (rpc::sse_messages(&text).iter()).find_map(|msg| rpc::reply_to(msg, id)),
            false => serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|msg| rpc::reply_of(&msg)),
        };
        Ok(Posted { head, reply })
    }
}

impl Conn for HttpConn {
    fn request(&mut self, method: &str, params: Value, call: &Call<'_>) -> Result<Value, String> {
        let mut posted = self.send(method, params.clone(), call)?;
        // HTTP 404 to a request with a session: the server ended the session
        // (MCP 2025-11-25 Session Management, D63). One new `initialize`,
        // one retry; a second failure marks the connection dead.
        if self.expired(&posted) {
            self.session = None;
            if let Err(err) = self.initialize(call) {
                self.dead = true;
                return Err(format!(
                    "{method}: session expired; initialize again: {err}"
                ));
            }
            posted = self.send(method, params, call)?;
            if self.expired(&posted) {
                self.dead = true;
                return Err(format!("{method}: HTTP 404 again after a new session"));
            }
        }
        if call.cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        match posted.reply {
            Some(reply) => rpc::result(reply),
            None => Err(format!(
                "{method}: HTTP {} without a reply",
                posted.head.status
            )),
        }
    }

    fn era(&self) -> Era {
        self.era
    }

    fn alive(&mut self) -> bool {
        !self.dead
    }
}

impl HttpConn {
    fn send(&mut self, method: &str, params: Value, call: &Call<'_>) -> Result<Posted, String> {
        let id = self.id();
        let msg = rpc::request(id, method, params, self.era);
        self.post(&msg, id, &call.headers, call)
    }

    /// A legacy request with a session id answered with HTTP 404.
    fn expired(&self, posted: &Posted) -> bool {
        self.era == Era::Legacy && self.session.is_some() && posted.head.status == 404
    }
}

/// The `Mcp-Param-*` headers of the `x-mcp-header` parameters of `schema`
/// present in `args` (properties chains only).
pub fn param_headers(schema: &Value, args: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Some(props) = schema.get("properties").and_then(Value::as_object) else {
        return out;
    };
    for (key, prop) in props {
        let Some(value) = args.get(key) else {
            continue;
        };
        if let Some(name) = prop.get("x-mcp-header").and_then(Value::as_str) {
            let text = match value {
                Value::String(text) => text.clone(),
                Value::Bool(flag) => flag.to_string(),
                Value::Number(n) if n.is_i64() || n.is_u64() => n.to_string(),
                _ => continue,
            };
            out.push((
                format!("mcp-param-{}", name.to_lowercase()),
                header_value(&text),
            ));
        } else if prop.get("properties").is_some() {
            out.extend(param_headers(prop, value));
        }
    }
    out
}

/// `text` as is when it is plain ASCII without edge spaces, else the
/// `=?base64?...?=` form of its UTF-8 bytes.
pub fn header_value(text: &str) -> String {
    let plain = text.bytes().all(|b| (0x20..0x7f).contains(&b))
        && text.trim() == text
        && !(text.starts_with("=?base64?") && text.ends_with("?="));
    match plain {
        true => text.to_string(),
        false => format!("=?base64?{}?=", base64(text.as_bytes())),
    }
}

fn base64(data: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            match i <= chunk.len() {
                true => out.push(ABC[((n >> (18 - 6 * i)) & 63) as usize] as char),
                false => out.push('='),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_values_follow_the_encoding_rule() {
        assert_eq!(header_value("us-west1"), "us-west1");
        assert_eq!(
            header_value("Hello, 世界"),
            "=?base64?SGVsbG8sIOS4lueVjA==?="
        );
        assert_eq!(header_value(" padded "), "=?base64?IHBhZGRlZCA=?=");
        assert_eq!(
            header_value("=?base64?literal?="),
            "=?base64?PT9iYXNlNjQ/bGl0ZXJhbD89?="
        );
    }

    #[test]
    fn x_mcp_header_parameters_become_headers() {
        let schema = json!({"properties": {
            "region": {"type": "string", "x-mcp-header": "Region"},
            "query": {"type": "string"},
            "opts": {"properties": {"dry": {"type": "boolean", "x-mcp-header": "Dry"}}}
        }});
        let args = json!({"region": "us-west1", "query": "q", "opts": {"dry": true}});
        let headers = param_headers(&schema, &args);
        assert!(headers.contains(&("mcp-param-region".into(), "us-west1".into())));
        assert!(headers.contains(&("mcp-param-dry".into(), "true".into())));
        assert_eq!(headers.len(), 2);
    }
}
