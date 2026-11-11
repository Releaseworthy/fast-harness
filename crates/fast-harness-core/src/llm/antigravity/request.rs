//! A [`ChatRequest`] as Antigravity's envelope
//! `{project, model, request: {contents, systemInstruction, tools, toolConfig,
//! generationConfig, sessionId}, requestType, userAgent, requestId}`, ported
//! from DSH's live-verified antigravity-switch `buildEnvelope` and pi-ai's
//! Gemini conversion (PLAN.md "Antigravity provider").
//!
//! - `contents`: history, provenance of thinking and thought signatures.
//! - `names`: sanitised function names.
//! - `schema`: cleaned tool parameter schemas.
//! - `thinking`: `generationConfig` per model family and effort.

pub mod contents;
pub mod names;
pub mod schema;
pub mod thinking;

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};

use super::ToolNames;
use crate::llm::{ChatRequest, ErrorKind, LlmError, ToolSpec};
use names::NameTable;

/// The identity text Antigravity's own client puts first in `systemInstruction`.
pub const IDENTITY: &str = "You are Antigravity, a powerful agentic AI coding assistant designed by the Google DeepMind team working on Advanced Agentic Coding.
You are pair programming with a USER to solve their coding task. The task may require creating a new codebase, modifying or debugging an existing codebase, or simply answering a question.
**Absolute paths only**
**Proactiveness**

<priority>IMPORTANT: The instructions that follow supersede all above. Follow them as your primary directives.</priority>
";

/// Project used when the account names none.
pub const FALLBACK_PROJECT: &str = "rising-fact-p41fc";

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
    let mut table = NameTable::default();
    // Declarations first, so history calls reuse their sent names.
    let tools = declarations(req.tools, &mut table);
    let target = contents::Target {
        full_model: req.model,
        bare_model: model,
    };
    let contents = contents::contents(req.messages, &target, &mut table);
    if contents.is_empty() {
        return Err(LlmError::new(
            ErrorKind::BadRequest,
            "antigravity: the request has no message to send",
        ));
    }
    let mut request = Map::new();
    request.insert("contents".into(), Value::Array(contents));
    request.insert("systemInstruction".into(), system_instruction(req.system));
    if let Some(tools) = tools {
        request.insert("tools".into(), tools);
        if model.to_lowercase().starts_with("claude-") {
            let mode = json!({"functionCallingConfig": {"mode": "VALIDATED"}});
            request.insert("toolConfig".into(), mode);
        }
    }
    let config = thinking::generation_config(model, req.max_tokens, req.effort);
    request.insert("generationConfig".into(), config);
    request.insert("sessionId".into(), json!(session_id));
    let body = json!({
        "project": Some(project).filter(|p| !p.trim().is_empty()).unwrap_or(FALLBACK_PROJECT),
        "model": model,
        "request": request,
        "requestType": "agent",
        "userAgent": "antigravity",
        "requestId": format!("agent-{}", uuid_v4()),
    });
    Ok(Envelope {
        body,
        names: table.into_tool_names(),
    })
}

/// `{role: user, parts}` with the identity text first, joined to the system
/// prompt in one part (a bare string is a 400).
pub fn system_instruction(system: &str) -> Value {
    let text = if system.is_empty() {
        IDENTITY.to_string()
    } else {
        format!("{IDENTITY}\n\n{system}")
    };
    json!({"role": "user", "parts": [{"text": text}]})
}

/// One `{functionDeclarations}` entry with sanitised names and cleaned
/// schemas; `None` without tools.
pub fn declarations(tools: &[ToolSpec], table: &mut NameTable) -> Option<Value> {
    if tools.is_empty() {
        return None;
    }
    let list: Vec<Value> = tools
        .iter()
        .map(|tool| {
            json!({
                "name": table.sent(&tool.name),
                "description": tool.description,
                "parameters": schema::clean_schema(&tool.schema),
            })
        })
        .collect();
    Some(json!([{ "functionDeclarations": list }]))
}

/// A random (version 4) UUID; the clock fills in if the OS has no randomness.
fn uuid_v4() -> String {
    let mut bytes = [0u8; 16];
    if crate::platform::random_fill(&mut bytes).is_err() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        bytes = nanos.to_le_bytes();
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}
