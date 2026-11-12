//! `session/prompt` and our `_fh/steer`, `_fh/follow_up`, `_fh/compact` and
//! `_fh/continue` (PLAN.md D48). An idle session starts a turn; a running
//! one takes a prompt or a follow-up at its next turn and a steer at its
//! next step boundary (PLAN.md D39). The response carries the user
//! message id as soon as the text is accepted; with `_meta: {"fh/wait":
//! true}` it waits for the turn and adds the stop reason and the usage.
//! <https://agentclientprotocol.com/protocol/v2/prompt-lifecycle.md>

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

use super::methods::{session_of, text_param};
use super::rpc::{response, send, Reply, RpcError, Server};
use super::turn::{Done, Finished, Sink};
use super::{acp, json as events, Expanded, Session, Start};
use crate::agent::{Input, COMPACT_COMMAND};

pub(super) fn call(server: &Arc<Server>, method: &str, params: &Value, id: Option<Value>) -> Reply {
    match run(server, method, params, id) {
        Ok(reply) => reply,
        Err(err) => Reply::Now(Err(err)),
    }
}

fn run(
    server: &Arc<Server>,
    method: &str,
    params: &Value,
    id: Option<Value>,
) -> Result<Reply, RpcError> {
    let session = session_of(server, params)?;
    let wait = params["_meta"]["fh/wait"].as_bool().unwrap_or(false);
    let id = id.filter(|_| wait);
    match method {
        "session/prompt" => submit(server, &session, prompt_text(&params["prompt"])?, false, id),
        "_fh/steer" => submit(
            server,
            &session,
            text_param(params, "text")?.into(),
            true,
            id,
        ),
        "_fh/follow_up" => submit(
            server,
            &session,
            text_param(params, "text")?.into(),
            false,
            id,
        ),
        "_fh/compact" => {
            let more = params["instructions"].as_str().unwrap_or("");
            let text = format!("{COMPACT_COMMAND} {more}").trim().to_string();
            start(server, &session, Start::Text(text), id)
        }
        _ => start(server, &session, Start::Resume, id),
    }
}

/// The text of an ACP prompt: `text`, `resource_link` (as a Markdown link)
/// and `resource` with text (the text after its URI); others are refused.
pub fn prompt_text(prompt: &Value) -> Result<String, RpcError> {
    let blocks = prompt.as_array().filter(|list| !list.is_empty());
    let blocks = blocks.ok_or_else(|| RpcError::params("prompt must be a non-empty array"))?;
    let mut parts = Vec::new();
    for block in blocks {
        let part = match block["type"].as_str() {
            Some("text") => block["text"].as_str().map(str::to_string),
            Some("resource_link") => (block["uri"].as_str())
                .map(|uri| format!("[{}]({uri})", block["name"].as_str().unwrap_or(uri))),
            Some("resource") => (block["resource"]["text"].as_str()).map(|text| {
                let uri = block["resource"]["uri"].as_str().unwrap_or("");
                format!("{uri}:\n```\n{text}\n```")
            }),
            _ => None,
        };
        parts.push(part.ok_or_else(|| RpcError::params("unsupported content block"))?);
    }
    Ok(parts.join("\n\n"))
}

fn queued_id() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("msg_q{}_{n}", crate::trace::now_ms())
}

/// A running turn takes the text from its inbox; an idle session runs it.
fn submit(
    server: &Arc<Server>,
    session: &Arc<Session>,
    text: String,
    steer: bool,
    id: Option<Value>,
) -> Result<Reply, RpcError> {
    if steer && session.send(Input::Steer(text.clone())) {
        return Ok(Reply::Now(Ok(json!({"queued": true}))));
    }
    if !steer {
        let qid = queued_id();
        if let Ok(mut queue) = session.queued.lock() {
            queue.push_back(qid.clone());
        }
        if session.send(Input::FollowUp(text.clone())) {
            return Ok(Reply::Now(Ok(json!({"messageId": qid, "queued": true}))));
        }
        if let Ok(mut queue) = session.queued.lock() {
            queue.retain(|q| *q != qid);
        }
    }
    match server
        .service
        .host()
        .expand(&text)
        .map_err(RpcError::params)?
    {
        Expanded::Send(text) => start(server, session, Start::Text(text), id),
        Expanded::Reply(answer) => local(server, session, &text, &answer),
    }
}

/// A command answered without the model (`/help`): live only, not logged.
fn local(server: &Server, session: &Session, text: &str, answer: &str) -> Result<Reply, RpcError> {
    let index = session.with_conv(|conv| conv.messages.len())?;
    let message = acp::user_id(index);
    server.notify(&session.id, acp::user_message(&message, text));
    server.notify(&session.id, acp::state("running", None, None));
    let reply = acp::agent_message(&format!("msg_l{index}"), answer);
    server.notify(&session.id, reply);
    server.notify(&session.id, acp::state("idle", Some("end_turn"), None));
    Ok(Reply::Now(Ok(json!({"messageId": message}))))
}

/// Starts a turn; `wait` is the request id answered when it ends.
fn start(
    server: &Arc<Server>,
    session: &Arc<Session>,
    first: Start,
    wait: Option<Value>,
) -> Result<Reply, RpcError> {
    let index = session.with_conv(|conv| conv.messages.len())?;
    let message = acp::user_id(index);
    if let Start::Text(text) = &first {
        server.notify(&session.id, acp::user_message(&message, text));
    }
    server.notify(&session.id, acp::state("running", None, None));
    let (sid, events_to) = (session.id.clone(), Arc::clone(server));
    let mut updates = acp::Updates::new(Arc::clone(&session.queued));
    let sink: Sink = Box::new(move |event| {
        if let Some(update) = updates.map(&event) {
            events_to.notify(&sid, update);
        }
    });
    let done = done(server, &session.id, &message, wait.clone());
    if let Err(err) = server.service.start(session, first, sink, done) {
        server.notify(&session.id, acp::state("idle", Some("error"), None));
        return Err(err.into());
    }
    Ok(match wait {
        Some(_) => Reply::Later,
        None => Reply::Now(Ok(json!({"messageId": message}))),
    })
}

/// The end of a turn: notes, unused inputs, the idle state, and the
/// waiting response.
fn done(server: &Arc<Server>, sid: &str, message: &str, wait: Option<Value>) -> Done {
    let (server, sid, message) = (Arc::clone(server), sid.to_string(), message.to_string());
    Box::new(move |end: &Finished| {
        for note in &end.notes {
            server.notify(&sid, acp::notice("info", note));
        }
        if !end.leftover.is_empty() {
            // A served prompt queues through the inbox: what it left is
            // reported here, not kept for a front end to start.
            if let Ok(session) = server.service.get(&sid) {
                session.drop_queue();
            }
            let mut notice = acp::notice("warning", "queued inputs were not used");
            notice["_meta"] = json!({"fh/leftover": end.leftover});
            server.notify(&sid, notice);
        }
        let stop = events::stop_reason(end.outcome);
        server.notify(&sid, acp::state("idle", Some(stop), end.error.as_deref()));
        if let Some(id) = wait {
            let result = json!({"messageId": message, "stopReason": stop,
                "_meta": {"fh/outcome": events::outcome_name(end.outcome),
                    "fh/usage": end.usage, "fh/durationMs": end.duration_ms}});
            send(&server.out, &response(&id, Ok(result)));
        }
    })
}
