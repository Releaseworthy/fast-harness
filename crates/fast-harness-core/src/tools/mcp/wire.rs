//! How MCP tools reach the model (PLAN.md D47). Up to [`CAP_TOOLS`] tools
//! and [`CAP_BYTES`] of definitions join the session's tool list as
//! deferred tools (found like any other, D30), whole servers in name order.
//! From the first server that does not fit on, the tools stay out of the
//! list: `find_tools` matches them from the index and loads
//! [`DESCRIBE`] and [`CALL`], which reach any indexed MCP tool by name. The
//! list therefore has the same size on every provider, and it is frozen
//! with the session (D36).

use serde_json::{json, Value};

use super::{IndexTool, McpServers};
use crate::llm::ToolSpec;
use crate::tools::{Tool, ToolCtx, ToolOutput};

pub const CAP_TOOLS: usize = 64;
pub const CAP_BYTES: usize = 64 * 1024;
pub const DESCRIBE: &str = "mcp_describe";
pub const CALL: &str = "mcp_call";

/// The tools that join the list and those reached through [`CALL`].
#[derive(Debug, Default)]
pub struct Plan {
    pub direct: Vec<IndexTool>,
    pub indirect: Vec<IndexTool>,
}

/// Bytes of one definition as sent: name, description and schema.
fn spec_bytes(tool: &IndexTool) -> usize {
    tool.spec.name.len() + tool.spec.description.len() + tool.spec.schema.to_string().len()
}

/// Splits `index` (servers in order) at the cap, whole servers at a time.
pub fn plan(index: Vec<IndexTool>) -> Plan {
    let mut groups: Vec<Vec<IndexTool>> = Vec::new();
    for tool in index {
        match groups.last_mut() {
            Some(group) if group[0].server == tool.server => group.push(tool),
            _ => groups.push(vec![tool]),
        }
    }
    let mut out = Plan::default();
    let (mut count, mut bytes, mut full) = (0, 0, false);
    for group in groups {
        let size: usize = group.iter().map(spec_bytes).sum();
        if !full && count + group.len() <= CAP_TOOLS && bytes + size <= CAP_BYTES {
            count += group.len();
            bytes += size;
            out.direct.extend(group);
        } else {
            full = true;
            out.indirect.extend(group);
        }
    }
    out
}

/// One MCP tool in the session's list.
pub struct McpTool {
    servers: McpServers,
    entry: IndexTool,
}

impl McpTool {
    pub fn new(servers: McpServers, entry: IndexTool) -> McpTool {
        McpTool { servers, entry }
    }
}

impl Tool for McpTool {
    fn spec(&self) -> ToolSpec {
        self.entry.spec.clone()
    }

    fn run(&self, args: &Value, ctx: &ToolCtx<'_>) -> ToolOutput {
        (self.servers).call(&self.entry.server, &self.entry.tool, args, ctx)
    }
}

/// `mcp_describe(name)`: an indexed tool's description and schema.
pub struct DescribeTool(pub McpServers);

/// `mcp_call(name, arguments)`: runs an indexed tool.
pub struct CallTool(pub McpServers);

fn name_arg(args: &Value) -> Result<&str, ToolOutput> {
    args.get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolOutput::err("pass `name`, an MCP tool name (mcp__<server>__<tool>)"))
}

fn unknown(name: &str) -> ToolOutput {
    ToolOutput::err(format!(
        "no MCP tool `{name}`: find_tools lists the MCP tools matching your words"
    ))
}

impl Tool for DescribeTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: DESCRIBE.into(),
            description: "Show the description and input schema of an MCP tool that find_tools \
                listed as reached through mcp_call."
                .into(),
            schema: json!({
                "type": "object",
                "properties": {"name": {"type": "string", "description": "The MCP tool name (mcp__<server>__<tool>)."}},
                "required": ["name"]
            }),
        }
    }

    fn run(&self, args: &Value, _ctx: &ToolCtx<'_>) -> ToolOutput {
        let name = match name_arg(args) {
            Ok(name) => name,
            Err(err) => return err,
        };
        match self.0.lookup(name) {
            Some(tool) => ToolOutput::ok(
                json!({
                    "name": tool.name,
                    "description": tool.full_description,
                    "inputSchema": tool.spec.schema,
                })
                .to_string(),
            ),
            None => unknown(name),
        }
    }
}

impl Tool for CallTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: CALL.into(),
            description: "Call an MCP tool that find_tools listed as reached through mcp_call, \
                with `arguments` matching the schema mcp_describe shows."
                .into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "The MCP tool name (mcp__<server>__<tool>)."},
                    "arguments": {"type": "object", "description": "The tool's arguments."}
                },
                "required": ["name"]
            }),
        }
    }

    fn run(&self, args: &Value, ctx: &ToolCtx<'_>) -> ToolOutput {
        let name = match name_arg(args) {
            Ok(name) => name,
            Err(err) => return err,
        };
        let Some(tool) = self.0.lookup(name) else {
            return unknown(name);
        };
        let arguments = match args.get("arguments") {
            Some(value @ Value::Object(_)) => value.clone(),
            None | Some(Value::Null) => json!({}),
            Some(_) => return ToolOutput::err("`arguments` must be a JSON object"),
        };
        self.0.call(&tool.server, &tool.tool, &arguments, ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(server: &str, i: usize, description: &str) -> IndexTool {
        IndexTool {
            name: format!("mcp__{server}__t{i}"),
            server: server.into(),
            tool: format!("t{i}"),
            spec: ToolSpec {
                name: format!("mcp__{server}__t{i}"),
                description: description.into(),
                schema: json!({}),
            },
            full_description: description.into(),
        }
    }

    #[test]
    fn whole_servers_join_until_the_cap() {
        let mut index: Vec<IndexTool> = (0..40).map(|i| tool("a", i, "x")).collect();
        index.extend((0..30).map(|i| tool("b", i, "x")));
        index.extend((0..2).map(|i| tool("c", i, "x")));
        let plan = plan(index);
        assert_eq!(plan.direct.len(), 40);
        assert_eq!(plan.indirect.len(), 32, "b passes 64 tools, c follows it");
        let big = "d".repeat(1000);
        let plan = super::plan((0..70).map(|i| tool("z", i, &big)).collect());
        assert!(plan.direct.is_empty() && plan.indirect.len() == 70);
    }
}
