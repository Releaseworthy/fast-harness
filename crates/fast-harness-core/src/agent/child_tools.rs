//! `agent_send`, `agent_stop` and `agent_output` (PLAN.md D40): the
//! deferred tools that steer, stop and read a background subagent.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde_json::{json, Value};

use super::children::Children;
use crate::agent::Input;
use crate::llm::ToolSpec;
use crate::tools::{Effect, Tool, ToolCtx, ToolOutput};

pub const SEND: &str = "agent_send";
pub const STOP: &str = "agent_stop";
pub const OUTPUT: &str = "agent_output";
pub(super) const NAMES: [&str; 3] = [SEND, STOP, OUTPUT];

#[derive(Clone, Copy)]
enum Kind {
    Send,
    Stop,
    Output,
}

struct ChildTool {
    kind: Kind,
    children: Arc<Children>,
}

/// The three tools for the children of one session.
pub(super) fn all(children: &Arc<Children>) -> Vec<Box<dyn Tool>> {
    [Kind::Send, Kind::Stop, Kind::Output]
        .into_iter()
        .map(|kind| {
            let children = children.clone();
            Box::new(ChildTool { kind, children }) as Box<dyn Tool>
        })
        .collect()
}

fn id_schema(extra: Value) -> Value {
    let mut properties =
        json!({"id": {"type": "string", "description": "The id subagent returned."}});
    if let (Some(map), Value::Object(more)) = (properties.as_object_mut(), extra) {
        map.extend(more);
    }
    json!({"type": "object", "properties": properties, "required": ["id"]})
}

impl Tool for ChildTool {
    /// \gent_output\ only reads (or waits); send and stop act on a child.
    fn effect(&self) -> Effect {
        match self.kind {
            Kind::Output => Effect::Read,
            Kind::Send | Kind::Stop => Effect::Run,
        }
    }

    fn spec(&self) -> ToolSpec {
        let (name, description, schema) = match self.kind {
            Kind::Send => (
                SEND,
                "Send a steer text to a running background subagent; it reads it at its next step.",
                id_schema(json!({"text": {"type": "string"}})),
            ),
            Kind::Stop => (STOP, "Stop a background subagent.", id_schema(json!({}))),
            Kind::Output => (
                OUTPUT,
                "The answer of a background subagent; `wait: true` waits until it ends.",
                id_schema(json!({"wait": {"type": "boolean"}})),
            ),
        };
        ToolSpec {
            name: name.into(),
            description: description.into(),
            schema,
        }
    }

    fn run(&self, args: &Value, ctx: &ToolCtx<'_>) -> ToolOutput {
        let id = args["id"].as_str().unwrap_or_default().trim();
        let Some(kid) = self.children.kid(id) else {
            return ToolOutput::err(format!("no background subagent `{id}`"));
        };
        match self.kind {
            Kind::Send => {
                let text = args["text"].as_str().unwrap_or_default();
                if text.trim().is_empty() {
                    return ToolOutput::err("`text` must be a non-empty string");
                }
                if kid.output.is_some() || kid.steer.send(Input::Steer(text.into())).is_err() {
                    return ToolOutput::err(format!("subagent {id} has ended"));
                }
                ToolOutput::ok(format!("sent to {id}"))
            }
            Kind::Stop if kid.output.is_some() => ToolOutput::ok(format!("{id} had ended")),
            Kind::Stop => {
                kid.cancel.store(true, Ordering::Relaxed);
                ToolOutput::ok(format!("stopping {id}; its answer still arrives"))
            }
            Kind::Output => {
                let wait = args["wait"].as_bool().unwrap_or(false);
                match self.children.output(id, wait, ctx.cancel) {
                    Some(output) => ToolOutput::ok(output),
                    None => ToolOutput::ok(format!("{id} is still running")),
                }
            }
        }
    }
}
