//! A [`ChatRequest`] as Antigravity's envelope
//! `{project, model, request: {contents, systemInstruction, tools, toolConfig,
//! generationConfig, sessionId}, requestType, userAgent, requestId}`.
//!
//! PLACEHOLDER: only user text goes out. The full port (history with thinking,
//! tool calls and results, signatures, cleaned tool schemas, sanitised names,
//! thinking config) replaces `build` and keeps its signature.

use serde_json::{json, Value};

use super::ToolNames;
use crate::llm::{ChatRequest, LlmError};
use crate::types::Role;

/// One request body and the tool names it used.
#[derive(Clone, Debug, PartialEq)]
pub struct Envelope {
    pub body: Value,
    pub names: ToolNames,
}

/// The envelope of `req` for the bare Antigravity model id `model` (no
/// `antigravity/` prefix) in `project`; `session_id` is a stable id for the
/// conversation (`request.sessionId`).
pub fn build(
    req: &ChatRequest<'_>,
    model: &str,
    project: &str,
    session_id: &str,
) -> Result<Envelope, LlmError> {
    let contents: Vec<Value> = req
        .messages
        .iter()
        .filter(|message| message.role == Role::User)
        .map(|message| json!({"role": "user", "parts": [{"text": message.text()}]}))
        .collect();
    let body = json!({
        "project": project,
        "model": model,
        "request": {"contents": contents, "sessionId": session_id},
        "requestType": "agent",
        "userAgent": "antigravity",
    });
    Ok(Envelope {
        body,
        names: ToolNames::default(),
    })
}
