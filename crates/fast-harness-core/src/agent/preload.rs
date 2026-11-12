//! Preloaded tools (PLAN.md D40): a subagent's `tools` are found before its
//! first step by a `find_tools` call and result written into the new
//! session, the same synthetic pair a compaction writes (D38). The tool list
//! stays the parent's (Claude: the same array with `defer_loading`, the
//! result's `tool_reference` blocks load the tools; Antigravity: the found
//! tools are declared from the first request), and the found set comes from
//! the history as always, so a resume needs nothing more.

use serde_json::json;

use crate::agent::turn::{traced, Res, Turn};
use crate::llm::find::{loaded_line, FIND_TOOLS};
use crate::llm::Timing;
use crate::types::{Block, Message, Role, StopReason, Usage};

/// Second line of the synthetic find result.
pub const PRELOADED: &str = "(tools loaded from the start)";

impl Turn<'_> {
    /// Adds the find call and result of the agent's preloaded tools after
    /// the first user message of a new session; nothing without them.
    pub(super) fn preload(&mut self) -> Res<()> {
        let names: Vec<&str> = self.agent.preload.iter().map(String::as_str).collect();
        if names.is_empty() || !self.agent.tools.finds() || self.conv.messages.len() != 1 {
            return Ok(());
        }
        let id = format!("preload_{}", self.turn);
        let arguments = json!({ "names": names }).to_string();
        let call = Message {
            role: Role::Assistant,
            content: vec![Block::ToolCall {
                id: id.clone(),
                name: FIND_TOOLS.into(),
                arguments: arguments.clone(),
                signature: None,
            }],
            model: None,
        };
        let content = format!("{}\n{PRELOADED}", loaded_line(&names));
        let size = content.len();
        let result = Message {
            role: Role::Tool,
            content: vec![Block::ToolResult {
                call_id: id.clone(),
                content,
                is_error: false,
            }],
            model: None,
        };
        let (turn, usage, timing) = (self.turn, Usage::default(), Timing::default());
        let trace = &mut self.conv.trace;
        let stop = StopReason::ToolUse;
        traced(trace.assistant_message(turn, 0, &call, &usage, stop, &timing, "", 0))?;
        traced(trace.tool_call(turn, 0, &id, FIND_TOOLS, &arguments))?;
        traced(trace.tool_result_sized(turn, 0, &result, 0, (size, None)))?;
        self.conv.messages.push(call);
        self.conv.messages.push(result);
        Ok(())
    }
}
