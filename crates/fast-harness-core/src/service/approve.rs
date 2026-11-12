//! Approvals over `serve` (PLAN.md D64): each request goes to the client
//! as ACP `session/request_permission` (its response answers it), and our
//! `_fh/approve` answers by request id.
//! <https://agentclientprotocol.com/protocol/v2/tool-calls.md#requesting-permission>

use serde_json::{json, Value};

use super::methods::{session_of, text_param};
use super::rpc::{RpcError, Server};
use crate::agent::Answer;
use crate::events::ApprovalRequest;
use crate::tools::Effect;

/// The prefix of the ids of our permission requests.
const PREFIX: &str = "fh-approval/";

/// `read`, `write` or `run`.
pub fn effect_word(effect: Effect) -> &'static str {
    match effect {
        Effect::Read => "read",
        Effect::Write => "write",
        _ => "run",
    }
}

/// The ACP `session/request_permission` request of `request`.
pub(super) fn permission(session: &str, request: &ApprovalRequest) -> Value {
    let title = match &request.agent {
        Some(agent) => format!("Allow subagent {agent}: {}?", request.summary),
        None => format!("Allow {}?", request.summary),
    };
    let kind = match request.effect {
        Effect::Write => "edit",
        _ => "execute",
    };
    let option =
        |id: &str, name: &str, kind: &str| json!({"optionId": id, "name": name, "kind": kind});
    json!({"jsonrpc": "2.0", "id": format!("{PREFIX}{session}/{}", request.id),
        "method": "session/request_permission",
        "params": {"sessionId": session, "title": title,
            "subject": {"type": "tool_call", "toolCall": {"toolCallId": request.call_id,
                "name": request.tool, "kind": kind, "status": "pending",
                "rawInput": request.arguments}},
            "options": [
                option("allow-once", "Allow once", "allow_once"),
                option("allow-session", "Allow the same for this session", "allow_always"),
                option("reject-once", "Decline", "reject_once"),
            ],
            "_meta": {"fh/requestId": request.id, "fh/effect": effect_word(request.effect),
                "fh/agent": request.agent}}})
}

/// A client's response to one of our permission requests; false when `id`
/// is not ours. An outcome other than a known option declines (ACP: never
/// treat it as approval).
pub(super) fn response(server: &Server, id: &str, message: &Value) -> bool {
    let Some((session, request)) = id.strip_prefix(PREFIX).and_then(|r| r.split_once('/')) else {
        return false;
    };
    let outcome = &message["result"]["outcome"];
    let answer = match (outcome["outcome"].as_str(), outcome["optionId"].as_str()) {
        (Some("selected"), Some("allow-once")) => Answer::Once,
        (Some("selected"), Some("allow-session")) => Answer::Session,
        (Some("selected"), Some("reject-once")) => Answer::No(None),
        _ => Answer::No(Some("the prompt was dismissed".into())),
    };
    if let Ok(session) = server.service.get(session) {
        session.answer(request, answer);
    }
    true
}

/// `_fh/approve` `{sessionId, requestId, answer: once|session|no, reason?}`.
pub(super) fn method(server: &Server, params: &Value) -> Result<Value, RpcError> {
    let session = session_of(server, params)?;
    let id = text_param(params, "requestId")?;
    let reason = params["reason"].as_str().map(str::to_string);
    let answer = match text_param(params, "answer")? {
        "once" => Answer::Once,
        "session" => Answer::Session,
        "no" => Answer::No(reason),
        other => {
            return Err(RpcError::params(format!(
                "answer: once, session or no, not {other}"
            )))
        }
    };
    match session.answer(id, answer) {
        true => Ok(json!({})),
        false => Err(RpcError::params(format!("no approval request {id} waits"))),
    }
}
