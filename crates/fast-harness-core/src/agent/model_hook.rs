//! The `PreModelRequest` hook (PLAN.md D63): once per step, before the
//! step's first attempt (retries, failovers and the call after an overflow
//! compaction send the same request and do not run it again; the summary
//! request of a compaction does not run it). It sees sizes and the parts it
//! may change, never a credential; it may block the call, change the effort,
//! lower `max_tokens` or add one context note at the tail.

use serde_json::{json, Value};

use crate::agent::call::request_model;
use crate::agent::hooks::fire;
use crate::agent::turn::{Res, Turn};
use crate::agent::AgentParts;
use crate::events::{AgentEvent, TurnOutcome};
use crate::tools::hooks::{Answer, HookEvent, RequestChange};

/// `Err` when a `PreModelRequest` answer asks for a change the rules refuse
/// (checked against the request as the earlier hooks left it).
pub(super) fn allowed(event: HookEvent, answer: &Answer, input: &Value) -> Result<(), String> {
    let (HookEvent::PreModelRequest, Some(change)) = (event, &answer.request) else {
        return Ok(());
    };
    let cap = input["maxTokens"].as_u64().unwrap_or(0);
    change.check(u32::try_from(cap).unwrap_or(u32::MAX))
}

/// A later hook sees the request as the earlier ones changed it.
pub(super) fn show(change: &RequestChange, input: &mut Value) {
    if let Some(effort) = &change.effort {
        input["effort"] = json!(effort);
    }
    if let Some(max_tokens) = change.max_tokens {
        input["maxTokens"] = json!(max_tokens);
    }
}

impl Turn<'_> {
    /// Runs the `PreModelRequest` hooks of step `step`; `Some` when one
    /// blocked the call (the turn fails with the reason shown).
    pub(super) fn model_hooks(&mut self, step: u32) -> Res<Option<TurnOutcome>> {
        self.change = None;
        let event = HookEvent::PreModelRequest;
        if self.quiet || !self.agent.parts.extensions.has_event(event) {
            return Ok(None);
        }
        let data = self.request_data(step);
        let (agent, session) = (self.agent, &self.conv.id);
        let mut verdict = fire(agent, session, self.cancel, (event, None), data);
        self.log_hooks(&verdict.records)?;
        if let Some(reason) = verdict.block {
            let note = format!("model request blocked by a hook: {reason}");
            (self.sink)(AgentEvent::Error(note));
            return Ok(Some(TurnOutcome::Failed));
        }
        self.hook_context.append(&mut verdict.context);
        self.change = verdict.request;
        self.flush_hook_context()?;
        Ok(None)
    }

    /// What a `PreModelRequest` hook reads besides the event, session and
    /// folder: sizes and the parts it may change. No credential, no account.
    fn request_data(&self, step: u32) -> Value {
        let (model, effort, max_tokens) = self.request_parts();
        let messages = &self.conv.messages;
        let message_bytes = serde_json::to_vec(messages).map_or(0, |bytes| bytes.len());
        let tools = (self.conv.tools.as_deref()).unwrap_or(self.agent.tools.specs());
        let tool_bytes: usize = (tools.iter())
            .map(|t| t.name.len() + t.description.len() + t.schema.to_string().len())
            .sum();
        json!({"turn": self.turn, "step": step, "depth": self.conv.depth,
            "model": model, "effort": effort, "maxTokens": max_tokens,
            "messages": {"count": messages.len(), "bytes": message_bytes},
            "tools": {"count": tools.len(), "bytes": tool_bytes}})
    }

    /// Model, effort and `max_tokens` of the next call: the compaction's
    /// own while one runs, else the configuration with this step's hook
    /// change over it.
    pub(super) fn request_parts(&self) -> (&str, Option<&str>, u32) {
        let change = self.change.as_ref().filter(|_| !self.quiet);
        request_parts(&self.request, change, &self.agent.parts)
    }
}

/// [`Turn::request_parts`] from the fields (a disjoint borrow).
pub(super) fn request_parts<'a>(
    request: &'a Option<(String, Option<String>)>,
    change: Option<&'a RequestChange>,
    parts: &'a AgentParts,
) -> (&'a str, Option<&'a str>, u32) {
    let config = &parts.config;
    let (model, effort) = request_model(request, config);
    // A `maxOutput` in `providers/*.json` caps it (PLAN.md D41).
    let cap = parts
        .catalog
        .max_output_override(model)
        .map_or(config.max_tokens, |cap| {
            config
                .max_tokens
                .min(u32::try_from(cap).unwrap_or(u32::MAX))
        });
    match change {
        Some(change) => (
            model,
            change.effort.as_deref().or(effort),
            change.max_tokens.map_or(cap, |n| n.min(cap)),
        ),
        None => (model, effort, cap),
    }
}
