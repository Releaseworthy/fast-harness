//! The tool boundary. A tool is a function of its JSON arguments and the
//! context; it never panics and reports failure through `ToolOutput::is_error`.
//! Built-in tools: read, write, edit, shell, grep, glob (`builtin`), behind
//! `find_tools` (`find`, PLAN.md D30). The subagent tool lives in `agent`
//! because it runs a nested agent loop.

use std::path::Path;
use std::sync::atomic::AtomicBool;

use serde_json::Value;

use crate::llm::ToolSpec;
pub use budget::ToolSession;
pub use command::CommandTool;
pub use ext::Extensions;

mod args;
pub mod budget;
mod command;
mod edit;
pub mod ext;
pub mod find;
mod glob;
mod grep;
mod hook_run;
pub mod hooks;
mod read;
mod shell;
pub mod skills;
mod walk;
mod write;

/// The end of `find_tools`' description when skills exist (D43).
const SKILLS_NOTE: &str = "`names` and `query` also match the skills named in the system \
prompt: a matched skill's instructions come back as text with its folder.";

pub struct ToolCtx<'a> {
    /// Relative paths in tool arguments resolve against this directory.
    pub cwd: &'a Path,
    /// Set from another thread to stop a long-running tool.
    pub cancel: &'a AtomicBool,
    /// Byte budget of this result (`Config::tool_budget`, PLAN.md D37).
    pub budget: usize,
    /// Id of the call; names its spill file and its read ranges.
    pub call_id: &'a str,
    /// State shared by the tools of one session; `None` outside a session.
    pub session: Option<&'a ToolSession>,
}

impl<'a> ToolCtx<'a> {
    /// A context outside a session with the default budget.
    pub fn new(cwd: &'a Path, cancel: &'a AtomicBool) -> ToolCtx<'a> {
        ToolCtx {
            cwd,
            cancel,
            budget: crate::config::DEFAULT_TOOL_OUTPUT_BYTES,
            call_id: "",
            session: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
    /// Size of the full output before the budget cut it.
    pub bytes: usize,
    /// The spill file holding the full output, when it was cut.
    pub spilled: Option<String>,
}

impl ToolOutput {
    pub fn new(content: impl Into<String>, is_error: bool) -> ToolOutput {
        let content = content.into();
        ToolOutput {
            bytes: content.len(),
            content,
            is_error,
            spilled: None,
        }
    }

    pub fn ok(content: impl Into<String>) -> ToolOutput {
        ToolOutput::new(content, false)
    }

    pub fn err(content: impl Into<String>) -> ToolOutput {
        ToolOutput::new(content, true)
    }
}

pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;

    /// `args` is the parsed arguments object. Invalid arguments produce an
    /// error output that tells the model what to fix.
    fn run(&self, args: &Value, ctx: &ToolCtx<'_>) -> ToolOutput;
}

#[derive(Default)]
pub struct Registry {
    tools: Vec<Box<dyn Tool>>,
    /// With `find`, `specs[0]` is `find_tools` and `specs[i + 1]` is `tools[i]`.
    specs: Vec<ToolSpec>,
    find: bool,
    /// Added to `find_tools`' description (the agent types, D42).
    note: String,
    /// Skills `find_tools` matches besides tools (D43).
    skills: Vec<skills::Skill>,
}

impl Registry {
    /// A registry whose tools are all loaded from the start (tests, tools).
    pub fn new() -> Registry {
        Registry::default()
    }

    /// A registry that starts with `find_tools` first and defers the other
    /// tools until a find loads them (PLAN.md D30).
    pub fn with_find() -> Registry {
        Registry::with_find_note(String::new())
    }

    /// [`Registry::with_find`] whose `find_tools` description ends with
    /// `note` (nothing when empty).
    pub fn with_find_note(note: String) -> Registry {
        Registry {
            tools: Vec::new(),
            specs: vec![find::spec_noted(&[], &note)],
            find: true,
            note,
            skills: Vec::new(),
        }
    }

    /// Lets `find_tools` match `skills` too; with any, its description says so.
    pub fn set_skills(&mut self, skills: Vec<skills::Skill>) {
        if !skills.is_empty() && self.find {
            if !self.note.is_empty() {
                self.note.push(' ');
            }
            self.note.push_str(SKILLS_NOTE);
            self.specs[0] = find::spec_noted(&self.specs[1..], &self.note);
        }
        self.skills = skills;
    }

    pub fn skills(&self) -> &[skills::Skill] {
        &self.skills
    }

    pub fn add(&mut self, tool: Box<dyn Tool>) {
        self.specs.push(tool.spec());
        self.tools.push(tool);
        if self.find {
            self.specs[0] = find::spec_noted(&self.specs[1..], &self.note);
        }
    }

    /// True when the other tools must be found before they are called.
    pub fn finds(&self) -> bool {
        self.find
    }

    /// Specs in registration order (the order sent to the model), with
    /// `find_tools` first when the registry finds.
    pub fn specs(&self) -> &[ToolSpec] {
        &self.specs
    }

    /// Runs tool `name` with the raw JSON arguments text from the model.
    /// Unknown tools and unparsable arguments become error outputs.
    pub fn run(&self, name: &str, raw_args: &str, ctx: &ToolCtx<'_>) -> ToolOutput {
        self.run_with(name, raw_args, ctx, &mut |_, output| output)
    }

    /// [`Registry::run`] with `filter` applied to the tool's own output
    /// (with the parsed arguments) before the budget (PostToolUse hooks,
    /// PLAN.md D44).
    pub fn run_with(
        &self,
        name: &str,
        raw_args: &str,
        ctx: &ToolCtx<'_>,
        filter: &mut dyn FnMut(&Value, ToolOutput) -> ToolOutput,
    ) -> ToolOutput {
        let Some(index) = self.specs.iter().position(|spec| spec.name == name) else {
            return ToolOutput::err(format!("unknown tool `{name}`"));
        };
        let source = if raw_args.trim().is_empty() {
            "{}"
        } else {
            raw_args
        };
        let args = match serde_json::from_str::<Value>(source) {
            Ok(args) if args.is_object() => args,
            Ok(_) => return ToolOutput::err("tool arguments must be a JSON object"),
            Err(err) => {
                return ToolOutput::err(format!("tool arguments are not valid JSON: {err}"))
            }
        };
        let output = match (self.find, index) {
            // Not budgeted: the tool lines are short and each skill body is
            // already cut at `skills::SKILL_BYTES` (at most MAX_SKILLS).
            (true, 0) if !self.skills.is_empty() => {
                let output = find::run_with(&args, &self.specs[1..], &self.skills);
                return filter(&args, output);
            }
            (true, 0) => find::run(&args, &self.specs[1..]),
            (true, index) => self.tools[index - 1].run(&args, ctx),
            (false, index) => self.tools[index].run(&args, ctx),
        };
        budget::fit(name, filter(&args, output), ctx)
    }
}

/// Registry with `find_tools` and the built-in tools: read, write, edit,
/// shell, grep, glob.
pub fn builtin() -> Registry {
    let mut registry = Registry::with_find();
    for tool in builtin_tools() {
        registry.add(tool);
    }
    registry
}

/// The built-in tools in their fixed order.
fn builtin_tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(read::ReadTool),
        Box::new(write::WriteTool),
        Box::new(edit::EditTool),
        Box::new(shell::ShellTool),
        Box::new(grep::GrepTool),
        Box::new(glob::GlobTool),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;

    impl Tool for Echo {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: "echo".into(),
                description: "Echo".into(),
                schema: serde_json::json!({}),
            }
        }

        fn run(&self, args: &Value, _ctx: &ToolCtx<'_>) -> ToolOutput {
            ToolOutput::ok(args["text"].as_str().unwrap_or("").to_string())
        }
    }

    fn run(raw: &str, name: &str) -> ToolOutput {
        let mut registry = Registry::new();
        registry.add(Box::new(Echo));
        let cancel = AtomicBool::new(false);
        registry.run(name, raw, &ToolCtx::new(Path::new("."), &cancel))
    }

    #[test]
    fn registry_runs_and_reports_bad_input() {
        assert_eq!(run(r#"{"text":"hi"}"#, "echo"), ToolOutput::ok("hi"));
        assert_eq!(run("", "echo"), ToolOutput::ok(""));
        assert!(run("[1]", "echo").is_error);
        assert!(run("{oops", "echo").content.contains("not valid JSON"));
        assert!(run("{}", "nope").content.contains("unknown tool"));
    }
}
