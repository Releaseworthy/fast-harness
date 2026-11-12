//! A UI-independent model of one tool card: what the call targets, short
//! chips, labelled fields, numbered lines (a diff, file lines, matches) and
//! plain text. The GUI draws it; it never touches the file system.

use serde_json::{Map, Value};

use super::tool_summary;
use super::{tool_detail_fs as fs, tool_detail_run as run};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    Read,
    Write,
    Edit,
    Grep,
    Glob,
    Shell,
    Subagent,
    Find,
    Other,
}

impl ToolKind {
    /// The kind of the tool called `name`.
    pub fn of(name: &str) -> ToolKind {
        match name {
            "read" => ToolKind::Read,
            "write" => ToolKind::Write,
            "edit" => ToolKind::Edit,
            "grep" => ToolKind::Grep,
            "glob" => ToolKind::Glob,
            "shell" => ToolKind::Shell,
            "subagent" => ToolKind::Subagent,
            crate::llm::find::FIND_TOOLS => ToolKind::Find,
            _ => ToolKind::Other,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            ToolKind::Read => "read",
            ToolKind::Write => "write",
            ToolKind::Edit => "edit",
            ToolKind::Grep => "grep",
            ToolKind::Glob => "glob",
            ToolKind::Shell => "shell",
            ToolKind::Subagent => "subagent",
            ToolKind::Find => "find",
            ToolKind::Other => "other",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Success,
    Error,
    Info,
    Warning,
}

impl Tone {
    pub fn word(self) -> &'static str {
        match self {
            Tone::Plain => "plain",
            Tone::Success => "success",
            Tone::Error => "error",
            Tone::Info => "info",
            Tone::Warning => "warning",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chip {
    pub text: String,
    pub tone: Tone,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub label: String,
    pub value: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
    Header,
    Gap,
}

impl LineKind {
    pub fn word(self) -> &'static str {
        match self {
            LineKind::Context => "context",
            LineKind::Added => "added",
            LineKind::Removed => "removed",
            LineKind::Header => "header",
            LineKind::Gap => "gap",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub kind: LineKind,
    pub old: Option<u64>,
    pub new: Option<u64>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolDetail {
    pub kind: ToolKind,
    /// The full path, pattern or first command line; never cut.
    pub target: String,
    pub chips: Vec<Chip>,
    pub fields: Vec<Field>,
    /// At most [`MAX_LINES`].
    pub lines: Vec<Line>,
    /// How many lines there were before the cut.
    pub lines_total: usize,
    /// Output shown as text when there are no lines, or the error.
    pub text: String,
}

/// The most lines a [`ToolDetail`] keeps.
pub const MAX_LINES: usize = 200;

/// One call as the parsers see it.
pub(crate) struct Call<'a> {
    pub args: &'a Map<String, Value>,
    pub output: Option<&'a str>,
    pub is_error: bool,
}

impl Call<'_> {
    /// A string argument.
    pub fn str(&self, key: &str) -> Option<&str> {
        self.args.get(key).and_then(Value::as_str)
    }

    /// An argument as text: strings as is, the rest as compact JSON.
    pub fn text(&self, key: &str) -> Option<String> {
        self.args.get(key).filter(|v| !v.is_null()).map(value_text)
    }

    /// The output to parse: none while the tool runs or after an error.
    pub fn result(&self) -> Option<&str> {
        self.output.filter(|_| !self.is_error)
    }
}

/// The card of a tool call. `arguments` may still be streaming (not yet a
/// JSON object); `output` is None while the tool runs.
pub fn tool_detail(
    name: &str,
    arguments: &str,
    output: Option<&str>,
    is_error: bool,
    duration_ms: Option<u64>,
) -> ToolDetail {
    let kind = ToolKind::of(name);
    let mut detail = ToolDetail {
        kind,
        target: String::new(),
        chips: Vec::new(),
        fields: Vec::new(),
        lines: Vec::new(),
        lines_total: 0,
        text: String::new(),
    };
    let Ok(Value::Object(args)) = serde_json::from_str::<Value>(arguments) else {
        detail.target = tool_summary(name, arguments);
        detail.text = output.unwrap_or_default().to_string();
        return detail;
    };
    let call = Call {
        args: &args,
        output,
        is_error,
    };
    fill(&mut detail, &call, name, arguments);
    if let Some(path) = output.and_then(crate::tools::budget::spill_path) {
        detail.field("full output", path);
    }
    detail.lines_total = detail.lines_total.max(detail.lines.len());
    detail.lines.truncate(MAX_LINES);
    if let Some(ms) = duration_ms.filter(|&ms| ms >= 1) {
        detail.chip(duration(ms), Tone::Plain);
    }
    detail
}

fn fill(detail: &mut ToolDetail, call: &Call<'_>, name: &str, arguments: &str) {
    match detail.kind {
        ToolKind::Read => fs::read(detail, call),
        ToolKind::Write => fs::write(detail, call),
        ToolKind::Edit => fs::edit(detail, call),
        ToolKind::Grep => fs::grep(detail, call),
        ToolKind::Glob => fs::glob(detail, call),
        ToolKind::Shell => run::shell(detail, call),
        ToolKind::Subagent => run::subagent(detail, call),
        ToolKind::Find => run::find(detail, call, arguments),
        ToolKind::Other => run::other(detail, call, name, arguments),
    }
    // Shell and subagent read their own failed output; other errors are text.
    let own = matches!(detail.kind, ToolKind::Shell | ToolKind::Subagent);
    if call.is_error && !own {
        detail.text = call.output.unwrap_or_default().to_string();
    }
}

impl ToolDetail {
    pub(crate) fn chip(&mut self, text: impl Into<String>, tone: Tone) {
        self.chips.push(Chip {
            text: text.into(),
            tone,
        });
    }

    pub(crate) fn field(&mut self, label: &str, value: impl Into<String>) {
        self.fields.push(Field {
            label: label.to_string(),
            value: value.into(),
        });
    }

    pub(crate) fn line(&mut self, kind: LineKind, old: Option<u64>, new: Option<u64>, text: &str) {
        self.lines.push(Line {
            kind,
            old,
            new,
            text: text.to_string(),
        });
    }
}

pub(crate) fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// "1 line", "3 lines".
pub(crate) fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// "512 B", "1.5 KB", "2.0 MB".
pub fn size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1_048_576 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    }
}

/// "850 ms", "2.5 s", "3 min 5 s".
pub fn duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{:.1} s", ms as f64 / 1000.0)
    } else {
        format!("{} min {} s", ms / 60_000, ms % 60_000 / 1000)
    }
}
