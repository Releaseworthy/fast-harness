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
pub use jobs::{JobInfo, JobState, Jobs};
pub use output::{Part, ToolOutput};

mod args;
pub mod ask;
mod atomic;
pub mod budget;
pub(crate) mod command;
mod edit;
pub mod ext;
pub mod find;
mod glob;
mod grep;
pub mod hook_request;
mod hook_run;
pub mod hooks;
mod job_buffer;
pub mod job_tools;
pub mod jobs;
pub mod mcp;
mod output;
mod read;
mod read_stream;
mod shell;
mod shell_live;
pub mod skills;
pub mod undo;
mod undo_restore;
mod walk;
mod write;

pub use undo::{UndoInfo, UndoLog};
pub use undo_restore::UndoOutcome;

/// The end of `find_tools`' description when skills exist (D43).
const SKILLS_NOTE: &str = "`names` and `query` also match the skills named in the system \
prompt: a matched skill's instructions come back as text with its folder.";

#[non_exhaustive]
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
    /// Where [`ToolCtx::progress`] goes; `None` outside a session.
    progress: Option<&'a Progress<'a>>,
}

/// The receiver of a call's progress text (PLAN.md D58): callable from any
/// thread, it must return at once and never panic.
pub type Progress<'a> = dyn Fn(&str) + Sync + 'a;

impl<'a> ToolCtx<'a> {
    /// A context outside a session with the default budget.
    pub fn new(cwd: &'a Path, cancel: &'a AtomicBool) -> ToolCtx<'a> {
        ToolCtx {
            cwd,
            cancel,
            budget: crate::config::DEFAULT_TOOL_OUTPUT_BYTES,
            call_id: "",
            session: None,
            progress: None,
        }
    }

    /// Sends `progress` the text [`ToolCtx::progress`] reports.
    pub fn with_progress(mut self, progress: &'a Progress<'a>) -> ToolCtx<'a> {
        self.progress = Some(progress);
        self
    }

    /// Does [`ToolCtx::progress`] go anywhere? (Lets a tool skip the work.)
    pub fn reports_progress(&self) -> bool {
        self.progress.is_some()
    }

    /// Reports `text` (new output since the last call) while the tool runs;
    /// front ends get it as `AgentEvent::ToolProgress`. Live only: never in
    /// the trace, the result or a request. Callable from the tool's thread
    /// and from threads it starts; does nothing outside a session and never
    /// blocks on a slow front end.
    pub fn progress(&self, text: &str) {
        if let (Some(progress), false) = (self.progress, text.is_empty()) {
            progress(text);
        }
    }

    pub fn with_budget(mut self, budget: usize) -> ToolCtx<'a> {
        self.budget = budget;
        self
    }

    pub fn with_call_id(mut self, call_id: &'a str) -> ToolCtx<'a> {
        self.call_id = call_id;
        self
    }

    pub fn with_session(mut self, session: &'a ToolSession) -> ToolCtx<'a> {
        self.session = Some(session);
        self
    }
}

/// What a tool call may change; decides how the calls of one step run
/// (PLAN.md D54): parallel calls run together, the others one at a time in
/// call order, after everything before them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Effect {
    /// Changes nothing outside the conversation (reads files, searches).
    Read,
    /// Changes files.
    Write,
    /// May do anything (runs programs); the default.
    Run,
}

impl Effect {
    /// `read`, `write` or `run` (the `effect` key of a tool file).
    pub fn from_word(word: &str) -> Option<Effect> {
        match word {
            "read" => Some(Effect::Read),
            "write" => Some(Effect::Write),
            "run" => Some(Effect::Run),
            _ => None,
        }
    }
}

pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;

    /// `args` is the parsed arguments object. Invalid arguments produce an
    /// error output that tells the model what to fix.
    fn run(&self, args: &Value, ctx: &ToolCtx<'_>) -> ToolOutput;

    /// What a call may change (PLAN.md D54); `Run` unless the tool says so.
    fn effect(&self) -> Effect {
        Effect::Run
    }

    /// May a call run alongside the other calls of its step? True for
    /// `Effect::Read`; a tool that is safe to run at once anyway (the
    /// subagent tool) says true itself.
    fn parallel(&self) -> bool {
        self.effect() == Effect::Read
    }
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
    /// MCP tools past the cap, matched by `find_tools` and reached through
    /// `mcp_describe` and `mcp_call` (D47).
    mcp: Vec<ToolSpec>,
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
            mcp: Vec::new(),
        }
    }

    /// Lets `find_tools` match the MCP tools past the cap (D47); with any,
    /// its description says how they are reached.
    pub fn set_mcp(&mut self, mcp: Vec<ToolSpec>) {
        if !mcp.is_empty() && self.find {
            if !self.note.is_empty() {
                self.note.push(' ');
            }
            self.note.push_str(&format!(
                "`query` also matches {} MCP tools outside this list: a find names them and \
                loads mcp_describe (a tool's schema) and mcp_call (runs it).",
                mcp.len()
            ));
            self.specs[0] = find::spec_noted(&self.specs[1..], &self.note);
        }
        self.mcp = mcp;
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

    /// The tool named `name`; `None` for `find_tools` and unknown names.
    fn tool(&self, name: &str) -> Option<&dyn Tool> {
        let index = self.specs.iter().position(|spec| spec.name == name)?;
        let index = match self.find {
            true => index.checked_sub(1)?,
            false => index,
        };
        self.tools.get(index).map(|tool| &**tool)
    }

    /// The effect of tool `name`: `Read` for `find_tools`, `None` when unknown.
    pub fn effect(&self, name: &str) -> Option<Effect> {
        match self.tool(name) {
            Some(tool) => Some(tool.effect()),
            None if self.find && self.specs.first().is_some_and(|s| s.name == name) => {
                Some(Effect::Read)
            }
            None => None,
        }
    }

    /// May a call of `name` run alongside the other calls of its step
    /// (PLAN.md D54)? True for `find_tools` and for an unknown name (it only
    /// returns an error).
    pub fn parallel(&self, name: &str) -> bool {
        self.tool(name).is_none_or(|tool| tool.parallel())
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
                let output = find::run_full(&args, &self.specs[1..], &self.skills, &self.mcp);
                return filter(&args, output);
            }
            (true, 0) => find::run_full(&args, &self.specs[1..], &[], &self.mcp),
            (true, index) => self.tools[index - 1].run(&args, ctx),
            (false, index) => self.tools[index].run(&args, ctx),
        };
        budget::fit(name, filter(&args, output), ctx)
    }
}

/// Registry with `find_tools` and the built-in tools: read, write, edit,
/// shell, grep, glob, job_output, job_kill, job_list, ask_user_question.
pub fn builtin() -> Registry {
    let mut registry = Registry::with_find();
    for tool in builtin_tools() {
        registry.add(tool);
    }
    registry
}

/// The built-in tools in their fixed order.
fn builtin_tools() -> Vec<Box<dyn Tool>> {
    let mut tools: Vec<Box<dyn Tool>> = vec![
        Box::new(read::ReadTool),
        Box::new(write::WriteTool),
        Box::new(edit::EditTool),
        Box::new(shell::ShellTool),
        Box::new(grep::GrepTool),
        Box::new(glob::GlobTool),
    ];
    tools.extend(job_tools::all());
    tools.push(Box::new(ask::AskTool));
    tools
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
        assert!(run("[1]", "echo").is_error());
        assert!(run("{oops", "echo").text().contains("not valid JSON"));
        assert!(run("{}", "nope").text().contains("unknown tool"));
    }
}
