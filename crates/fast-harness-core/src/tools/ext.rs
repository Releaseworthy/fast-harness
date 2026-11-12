//! The extension files of a session (PLAN.md D42) as tools: command tools,
//! overrides of built-in tools (`description`, `outputBytes`, `disabled`)
//! and the agent types named in `find_tools`. Built once per settings load
//! from the index; every registry of the session is built from it, so the
//! tool list a session freezes is builtins plus the index.

use super::command::CommandTool;
use super::{builtin_tools, Registry, Tool, ToolCtx, ToolOutput};
use crate::config::index::{Entry, ExtIndex, Item};
use crate::llm::ToolSpec;

/// Most bytes of the agent type list in `find_tools`' description.
const AGENT_NOTE_BYTES: usize = 1000;

#[derive(Default)]
pub struct Extensions {
    pub index: ExtIndex,
    commands: Vec<CommandTool>,
}

impl Extensions {
    pub fn new(index: ExtIndex) -> Extensions {
        let commands = (index.tools.iter())
            .filter(|item| item.entry.command && !item.entry.disabled)
            .map(|item| CommandTool::new(&item.entry.name, &item.entry.description, &item.path))
            .collect();
        Extensions { index, commands }
    }

    /// The override file of tool `name` (a tool file without `command`).
    fn override_of(&self, name: &str) -> Option<&Entry> {
        let item =
            (self.index.tools.iter()).find(|item| !item.entry.command && item.entry.name == name);
        item.map(|item| &item.entry)
    }

    /// Adds `tool` unless a file disables it; a file's description replaces
    /// the tool's own.
    pub fn add(&self, registry: &mut Registry, tool: Box<dyn Tool>) {
        let name = tool.spec().name;
        match self.override_of(&name) {
            Some(entry) if entry.disabled => {}
            Some(entry) if !entry.description.trim().is_empty() => {
                let description = entry.description.clone();
                registry.add(Box::new(Described { tool, description }));
            }
            _ => registry.add(tool),
        }
    }

    /// The command tools whose name no tool of `registry` has yet.
    pub fn add_commands(&self, registry: &mut Registry) {
        for tool in &self.commands {
            let name = tool.spec().name;
            if !registry.specs().iter().any(|spec| spec.name == name) {
                registry.add(Box::new(tool.clone()));
            }
        }
    }

    /// `find_tools` (naming the agent types) and the built-in tools with
    /// their overrides, without command tools.
    pub fn builtin(&self) -> Registry {
        let mut registry = Registry::with_find_note(self.agent_note());
        for tool in builtin_tools() {
            self.add(&mut registry, tool);
        }
        registry
    }

    /// [`Extensions::builtin`] plus the command tools.
    pub fn registry(&self) -> Registry {
        let mut registry = self.builtin();
        self.add_commands(&mut registry);
        registry
    }

    /// `outputBytes` of every tool file (overrides and command tools).
    pub fn output_bytes(&self) -> Vec<(String, usize)> {
        (self.index.tools.iter())
            .filter_map(|item| Some((item.entry.name.clone(), item.entry.output_bytes?)))
            .collect()
    }

    pub fn agent(&self, name: &str) -> Option<&Item> {
        self.index.agent(name)
    }

    pub fn command(&self, name: &str) -> Option<&Item> {
        self.index.command(name)
    }

    /// `Agent types for subagent's agent: a, b and 3 more.`, at most about
    /// 1 KB; empty without agent files (the description then stays as it was).
    pub fn agent_note(&self) -> String {
        let names: Vec<&str> = (self.index.agents.iter())
            .map(|item| item.entry.name.as_str())
            .collect();
        agent_note(&names)
    }
}

pub fn agent_note(names: &[&str]) -> String {
    if names.is_empty() {
        return String::new();
    }
    let mut shown = String::new();
    let mut count = 0;
    for name in names {
        if shown.len() + name.len() + 2 > AGENT_NOTE_BYTES {
            break;
        }
        if count > 0 {
            shown.push_str(", ");
        }
        shown.push_str(name);
        count += 1;
    }
    let more = match names.len() - count {
        0 => String::new(),
        n => format!(" and {n} more"),
    };
    format!("Agent types for the subagent tool's `agent`: {shown}{more}.")
}

/// A built-in tool with the description of its override file.
struct Described {
    tool: Box<dyn Tool>,
    description: String,
}

impl Tool for Described {
    fn spec(&self) -> ToolSpec {
        let mut spec = self.tool.spec();
        spec.description.clone_from(&self.description);
        spec
    }

    fn run(&self, args: &serde_json::Value, ctx: &ToolCtx<'_>) -> ToolOutput {
        self.tool.run(args, ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_note_stays_small() {
        assert_eq!(agent_note(&[]), "");
        assert!(agent_note(&["a", "b"]).ends_with(": a, b."));
        let many: Vec<String> = (0..400).map(|i| format!("agent{i}")).collect();
        let names: Vec<&str> = many.iter().map(String::as_str).collect();
        let note = agent_note(&names);
        assert!(note.len() < 1100, "{}", note.len());
        assert!(note.contains("more."));
    }
}
