//! JSON-RPC 2.0 over newline-delimited JSON (PLAN.md D48): the transport of
//! `fast-harness serve --stdio`. Requests are read one line at a time on
//! the caller's thread; a turn runs on its own thread and its updates and
//! late responses share the writer, one whole line at a time, so answers
//! come in any order, matched by `id`.
//! <https://agentclientprotocol.com/protocol/v2/overview>,
//! <https://www.jsonrpc.org/specification>

use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use super::{methods, Service, ServiceError};

/// Where every message goes; one lock per line.
pub type Out = Arc<Mutex<Box<dyn Write + Send>>>;

/// `writer` as an [`Out`].
pub fn out(writer: impl Write + Send + 'static) -> Out {
    Arc::new(Mutex::new(Box::new(writer)))
}

/// Writes `value` as one line and flushes; a closed output is ignored
/// (the client went away, the reader sees the end of input next).
pub fn send(out: &Out, value: &Value) {
    let mut line = value.to_string();
    line.push('\n');
    let mut writer = out.lock().unwrap_or_else(|p| p.into_inner());
    let _ = writer
        .write_all(line.as_bytes())
        .and_then(|_| writer.flush());
}

/// A JSON-RPC error object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;
/// ACP: the session is not active on this connection.
/// <https://agentclientprotocol.com/protocol/v2/error.md#unknown-sessions>
pub const UNKNOWN_SESSION: i64 = -32002;
/// Ours: the call needs an idle session.
pub const SESSION_BUSY: i64 = -32010;

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> RpcError {
        RpcError {
            code,
            message: message.into(),
        }
    }

    pub fn params(message: impl Into<String>) -> RpcError {
        RpcError::new(INVALID_PARAMS, message)
    }
}

impl From<ServiceError> for RpcError {
    fn from(err: ServiceError) -> RpcError {
        match err {
            ServiceError::Unknown(m) => RpcError::new(UNKNOWN_SESSION, m),
            ServiceError::Busy(m) => RpcError::new(SESSION_BUSY, m),
            ServiceError::Invalid(m) => RpcError::new(INVALID_PARAMS, m),
            ServiceError::Failed(m) => RpcError::new(INTERNAL_ERROR, m),
        }
    }
}

/// A response to `id`.
pub fn response(id: &Value, result: Result<Value, RpcError>) -> Value {
    match result {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err(err) => json!({"jsonrpc": "2.0", "id": id,
            "error": {"code": err.code, "message": err.message}}),
    }
}

/// What a method gives back: an answer now, or one the turn thread sends.
pub enum Reply {
    Now(Result<Value, RpcError>),
    Later,
}

/// The connection: the service and the writer.
pub struct Server {
    pub service: Arc<Service>,
    pub out: Out,
}

impl Server {
    pub fn notify(&self, session: &str, update: Value) {
        send(&self.out, &super::acp::notification(session, update));
    }

    /// Handles one input line.
    pub fn line(self: &Arc<Self>, line: &str) {
        let message: Value = match serde_json::from_str(line) {
            Ok(message) => message,
            Err(err) => {
                let err = RpcError::new(PARSE_ERROR, format!("parse error: {err}"));
                return send(&self.out, &response(&Value::Null, Err(err)));
            }
        };
        let id = message.get("id").cloned();
        let method = message.get("method").and_then(Value::as_str);
        let valid_id = matches!(id, None | Some(Value::String(_) | Value::Number(_)));
        let Some(method) = method.filter(|_| message["jsonrpc"] == "2.0" && valid_id) else {
            // A response to a request of ours: a permission answer (D64),
            // anything else is ignored.
            if message.get("result").is_some() || message.get("error").is_some() {
                if let Some(Value::String(id)) = &id {
                    super::approve::response(self, id, &message);
                }
                return;
            }
            let err = RpcError::new(INVALID_REQUEST, "not a JSON-RPC 2.0 request");
            let id = id.filter(|_| valid_id).unwrap_or(Value::Null);
            return send(&self.out, &response(&id, Err(err)));
        };
        let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
        let reply = methods::call(self, method, params, id.clone());
        if let (Some(id), Reply::Now(result)) = (id, reply) {
            send(&self.out, &response(&id, result));
        }
    }
}

/// Serves `input` until it ends, then cancels every running turn and
/// waits for it, so the last updates are written before this returns.
pub fn serve(service: Arc<Service>, input: impl BufRead, out: Out) {
    let server = Arc::new(Server { service, out });
    for line in input.lines() {
        let Ok(line) = line else {
            break;
        };
        if !line.trim().is_empty() {
            server.line(&line);
        }
    }
    for session in server.service.open_sessions() {
        server.service.close(&session.id);
    }
}
