//! The `subagent` tool (PLAN.md D40): a child agent with its own session log
//! naming the parent, an optional model, effort and preloaded tools, run in
//! the foreground (the tool returns its answer) or in the background (the
//! tool returns `{id}` and the answer reaches the parent's inbox later).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use super::child_run;
use super::children::{self, Children};
use crate::agent::{AgentParts, Router};
use crate::llm::ToolSpec;
use crate::tools::{Tool, ToolCtx, ToolOutput};

pub const NAME: &str = "subagent";

pub struct SubagentTool {
    pub(super) parts: AgentParts,
    pub(super) app_home: PathBuf,
    /// The session whose children this tool starts.
    pub(super) session: String,
    /// That session's nesting level (0 for a session a person started).
    pub(super) depth: u32,
    pub(super) router: Option<Router>,
    pub(super) children: Arc<Children>,
}

impl SubagentTool {
    /// `parts` are the parent agent's; `session` goes into each child header.
    pub fn new(
        parts: AgentParts,
        app_home: &Path,
        session: &str,
        depth: u32,
        router: Option<Router>,
    ) -> SubagentTool {
        SubagentTool {
            parts,
            app_home: app_home.to_path_buf(),
            session: session.to_string(),
            depth,
            router,
            children: children::of_session(session),
        }
    }
}

/// The arguments of one call.
pub(super) struct Spec {
    pub prompt: String,
    pub label: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub tools: Vec<String>,
    pub background: bool,
}

impl Tool for SubagentTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: NAME.into(),
            description: "Run a self-contained task in a fresh agent with its own context; \
                it returns its final answer. Give it everything it needs in `prompt`. \
                Optional: `model` (a model id, `antigravity/<id>` for Google models) and \
                `effort`; `tools` (tool names it may call at once, without find_tools); \
                `background: true` returns `{id}` at once and the answer arrives later as a \
                `[subagent <id> finished]` message (agent_send steers it, agent_stop stops \
                it, agent_output reads its answer)."
                .into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "prompt": {"type": "string", "description": "The complete task."},
                    "description": {"type": "string", "description": "A short label, 3 to 5 words."},
                    "agent": {"type": "string", "description": "An agent type; none exist yet."},
                    "model": {"type": "string", "description": "Model id; default: the parent's."},
                    "effort": {"type": "string", "description": "low, medium, high, max or off."},
                    "tools": {"type": "array", "items": {"type": "string"}, "description": "Tools loaded from the start."},
                    "background": {"type": "boolean", "description": "Return {id} at once."}
                },
                "required": ["prompt"]
            }),
        }
    }

    fn run(&self, args: &Value, ctx: &ToolCtx<'_>) -> ToolOutput {
        Spec::parse(args)
            .and_then(|spec| child_run::start(self, &spec, ctx))
            .unwrap_or_else(ToolOutput::err)
    }
}

impl Spec {
    fn parse(args: &Value) -> Result<Spec, String> {
        let prompt = args["prompt"].as_str().filter(|p| !p.trim().is_empty());
        let prompt = prompt.ok_or_else(|| "`prompt` must be a non-empty string".to_string())?;
        agent_type(&args["agent"])?;
        let text = |key: &str| {
            let value = args.get(key).and_then(Value::as_str).map(str::trim);
            value.filter(|s| !s.is_empty()).map(str::to_string)
        };
        let names = "`tools` must be a list of tool names";
        let tools = match args.get("tools") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(list)) => list
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| names.to_string())
                })
                .collect::<Result<_, _>>()?,
            Some(_) => return Err(names.into()),
        };
        Ok(Spec {
            prompt: prompt.to_string(),
            label: text("description").unwrap_or_default(),
            model: text("model"),
            effort: text("effort"),
            tools,
            background: args["background"].as_bool().unwrap_or(false),
        })
    }
}

/// The one place agent types (`agents/*.md`, phase 17A) will resolve; no
/// type exists yet, so any name is an error.
fn agent_type(value: &Value) -> Result<(), String> {
    let name = value.as_str().map(str::trim).unwrap_or_default();
    if name.is_empty() {
        return Ok(());
    }
    Err(format!(
        "unknown agent type `{name}`: agent types (agents/*.md) are not available yet; omit `agent`"
    ))
}
