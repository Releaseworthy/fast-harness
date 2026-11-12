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
use crate::config::index::{read_md, MdFile};
use crate::llm::ToolSpec;
use crate::tools::{Extensions, Tool, ToolCtx, ToolOutput};

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
    /// The agent type's `maxDepth` (it only lowers `subagent.maxDepth`).
    pub max_depth: Option<u32>,
}

impl Tool for SubagentTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: NAME.into(),
            description: "Run a self-contained task in a fresh agent with its own context; \
                it returns its final answer. Give it everything it needs in `prompt`. \
                Optional: `model` (a model id, `<provider id>/<id>` for providers/*.json models) and \
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
                    "agent": {"type": "string", "description": "An agent type (agents/*.md; find_tools names them): its prompt, model, effort and tools apply."},
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
        Spec::parse(args, &self.parts.extensions)
            .and_then(|spec| child_run::start(self, &spec, ctx))
            .unwrap_or_else(ToolOutput::err)
    }
}

impl Spec {
    fn parse(args: &Value, ext: &Extensions) -> Result<Spec, String> {
        let prompt = args["prompt"].as_str().filter(|p| !p.trim().is_empty());
        let prompt = prompt.ok_or_else(|| "`prompt` must be a non-empty string".to_string())?;
        let kind = agent_type(&args["agent"], ext)?.unwrap_or_default();
        let text = |key: &str| {
            let value = args.get(key).and_then(Value::as_str).map(str::trim);
            value.filter(|s| !s.is_empty()).map(str::to_string)
        };
        let mut tools = tool_names(args)?;
        if tools.is_empty() {
            tools.clone_from(&kind.tools);
        }
        // The type's body opens the child's first user message: the system
        // prompt stays the parent's, so the cached prefix is shared.
        let prompt = match kind.body.is_empty() {
            true => prompt.to_string(),
            false => format!("{}\n\n{prompt}", kind.body),
        };
        Ok(Spec {
            prompt,
            label: text("description").unwrap_or_else(|| kind.name.clone()),
            model: text("model").or(kind.model),
            effort: text("effort").or(kind.effort),
            tools,
            background: args["background"].as_bool().unwrap_or(false),
            max_depth: kind.max_depth,
        })
    }
}

fn tool_names(args: &Value) -> Result<Vec<String>, String> {
    let names = "`tools` must be a list of tool names";
    match args.get("tools") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(list)) => list
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| names.to_string())
            })
            .collect(),
        Some(_) => Err(names.into()),
    }
}

/// The agent type `value` names (`agents/*.md`, PLAN.md D42), read now;
/// `None` without one.
fn agent_type(value: &Value, ext: &Extensions) -> Result<Option<MdFile>, String> {
    let name = value.as_str().map(str::trim).unwrap_or_default();
    if name.is_empty() {
        return Ok(None);
    }
    match ext.agent(name) {
        Some(item) => read_md(&item.path).map(Some),
        None => {
            let names: Vec<&str> = (ext.index.agents.iter())
                .map(|item| item.entry.name.as_str())
                .collect();
            let known = match names.is_empty() {
                true => "none: add agents/<name>.md".to_string(),
                false => names.join(", "),
            };
            Err(format!(
                "unknown agent type `{name}` (agent types: {known})"
            ))
        }
    }
}
