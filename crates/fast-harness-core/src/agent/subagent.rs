//! The `subagent` tool: a nested agent with the built-in tools only (so one
//! level deep), its own session log naming the parent session, one turn.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::agent::{Agent, AgentParts, Conversation};
use crate::events::{AgentEvent, TurnOutcome};
use crate::llm::ToolSpec;
use crate::tools::{self, Tool, ToolCtx, ToolOutput};
use crate::types::Role;

pub const NAME: &str = "subagent";

pub struct SubagentTool {
    parts: AgentParts,
    app_home: PathBuf,
    parent: String,
}

impl SubagentTool {
    /// `parts` are the parent agent's; `parent_session` goes into the child header.
    pub fn new(parts: AgentParts, app_home: &Path, parent_session: &str) -> SubagentTool {
        SubagentTool {
            parts,
            app_home: app_home.to_path_buf(),
            parent: parent_session.to_string(),
        }
    }
}

impl Tool for SubagentTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: NAME.into(),
            description: "Run a self-contained task in a fresh agent with the same tools \
                (except subagent) and its own context. Give it everything it needs in \
                `prompt`; it returns its final answer."
                .into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "prompt": {"type": "string", "description": "The complete task."},
                    "description": {"type": "string", "description": "A short label, 3 to 5 words."}
                },
                "required": ["prompt"]
            }),
        }
    }

    fn run(&self, args: &Value, ctx: &ToolCtx<'_>) -> ToolOutput {
        let Some(prompt) = args["prompt"].as_str().filter(|p| !p.trim().is_empty()) else {
            return ToolOutput::err("`prompt` must be a non-empty string");
        };
        let mut conv =
            match Conversation::create(&self.app_home, &self.parts.cwd, Some(&self.parent)) {
                Ok(conv) => conv,
                Err(err) => {
                    return ToolOutput::err(format!("cannot start a subagent session: {err}"))
                }
            };
        let agent = Agent::new(self.parts.clone(), Arc::new(tools::builtin()));
        let mut errors: Vec<String> = Vec::new();
        let outcome = agent.run_turn(&mut conv, prompt, ctx.cancel, &mut |event| {
            if let AgentEvent::Error(message) = event {
                errors.push(message);
            }
        });
        let text = conv
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::Assistant)
            .map(|m| m.text())
            .unwrap_or_default();
        result(text, &conv.id, outcome, &errors)
    }
}

fn result(text: String, session: &str, outcome: TurnOutcome, errors: &[String]) -> ToolOutput {
    let mut content = text.trim_end().to_string();
    if outcome != TurnOutcome::Done {
        let detail = errors.last().map(|e| format!(": {e}")).unwrap_or_default();
        content.push_str(&format!("\n\nsubagent ended {outcome:?}{detail}"));
    }
    content.push_str(&format!("\n\n[subagent session {session}]"));
    ToolOutput {
        content: content.trim_start().to_string(),
        is_error: outcome != TurnOutcome::Done,
    }
}
