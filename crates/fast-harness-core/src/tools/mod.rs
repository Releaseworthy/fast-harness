//! The tool boundary. A tool is a function of its JSON arguments and the
//! context; it never panics and reports failure through `ToolOutput::is_error`.
//! Built-in tools: read, write, edit, shell, grep, glob (`builtin`), behind
//! `find_tools` (`find`, PLAN.md D30). The subagent tool lives in `agent`
//! because it runs a nested agent loop.

use std::path::Path;
use std::sync::atomic::AtomicBool;

use serde_json::Value;

use crate::llm::ToolSpec;

mod args;
mod edit;
pub mod find;
mod glob;
mod grep;
mod read;
mod shell;
mod walk;
mod write;

pub struct ToolCtx<'a> {
    /// Relative paths in tool arguments resolve against this directory.
    pub cwd: &'a Path,
    /// Set from another thread to stop a long-running tool.
    pub cancel: &'a AtomicBool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
}

impl ToolOutput {
    pub fn ok(content: impl Into<String>) -> ToolOutput {
        ToolOutput {
            content: content.into(),
            is_error: false,
        }
    }

    pub fn err(content: impl Into<String>) -> ToolOutput {
        ToolOutput {
            content: content.into(),
            is_error: true,
        }
    }
}

pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;

    /// `args` is the parsed arguments object. Invalid arguments produce an
    /// error output that tells the model what to fix.
    fn run(&self, args: &Value, ctx: &ToolCtx<'_>) -> ToolOutput;
}

/// Largest tool output handed to the model; longer output is cut by [`clip`].
pub const MAX_OUTPUT_BYTES: usize = 32 * 1024;

/// Keeps the head and the tail of an oversized output with a marker between
/// them, cutting only on character boundaries.
pub fn clip(content: String, max_bytes: usize) -> String {
    if content.len() <= max_bytes {
        return content;
    }
    let half = max_bytes / 2;
    let mut head_end = half;
    while !content.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let mut tail_start = content.len() - half;
    while !content.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    let dropped = tail_start - head_end;
    format!(
        "{}\n[... {dropped} bytes clipped ...]\n{}",
        &content[..head_end],
        &content[tail_start..]
    )
}

#[derive(Default)]
pub struct Registry {
    tools: Vec<Box<dyn Tool>>,
    /// With `find`, `specs[0]` is `find_tools` and `specs[i + 1]` is `tools[i]`.
    specs: Vec<ToolSpec>,
    find: bool,
}

impl Registry {
    /// A registry whose tools are all loaded from the start (tests, tools).
    pub fn new() -> Registry {
        Registry::default()
    }

    /// A registry that starts with `find_tools` first and defers the other
    /// tools until a find loads them (PLAN.md D30).
    pub fn with_find() -> Registry {
        Registry {
            tools: Vec::new(),
            specs: vec![find::spec(&[])],
            find: true,
        }
    }

    pub fn add(&mut self, tool: Box<dyn Tool>) {
        self.specs.push(tool.spec());
        self.tools.push(tool);
        if self.find {
            self.specs[0] = find::spec(&self.specs[1..]);
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
        let mut output = match (self.find, index) {
            (true, 0) => find::run(&args, &self.specs[1..]),
            (true, index) => self.tools[index - 1].run(&args, ctx),
            (false, index) => self.tools[index].run(&args, ctx),
        };
        output.content = clip(output.content, MAX_OUTPUT_BYTES);
        output
    }
}

/// Registry with `find_tools` and the built-in tools: read, write, edit,
/// shell, grep, glob.
pub fn builtin() -> Registry {
    let mut registry = Registry::with_find();
    registry.add(Box::new(read::ReadTool));
    registry.add(Box::new(write::WriteTool));
    registry.add(Box::new(edit::EditTool));
    registry.add(Box::new(shell::ShellTool));
    registry.add(Box::new(grep::GrepTool));
    registry.add(Box::new(glob::GlobTool));
    registry
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
        registry.run(
            name,
            raw,
            &ToolCtx {
                cwd: Path::new("."),
                cancel: &cancel,
            },
        )
    }

    #[test]
    fn registry_runs_and_reports_bad_input() {
        assert_eq!(run(r#"{"text":"hi"}"#, "echo"), ToolOutput::ok("hi"));
        assert_eq!(run("", "echo"), ToolOutput::ok(""));
        assert!(run("[1]", "echo").is_error);
        assert!(run("{oops", "echo").content.contains("not valid JSON"));
        assert!(run("{}", "nope").content.contains("unknown tool"));
    }

    #[test]
    fn clip_keeps_head_and_tail_on_char_boundaries() {
        let text = "\u{1EBF}".repeat(100);
        let clipped = clip(text.clone(), 50);
        assert!(clipped.starts_with('\u{1EBF}') && clipped.ends_with('\u{1EBF}'));
        assert!(clipped.contains("bytes clipped"));
        assert_eq!(clip(text.clone(), 1000), text);
    }
}
