//! Command tools (PLAN.md D42): a `tools/*.json` file naming a program. The
//! arguments go in on stdin as JSON, stdout is the result, a non-zero exit
//! is an error with the tail of stderr. The process tree lives in one
//! `platform::ProcessGroup`: a timeout or a cancel kills all of it. The file
//! is read on first use (its schema when the session's tool list is
//! rendered), then kept for the life of the tool.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};

use super::shell::{drain, pump, supervise, Capture, Ending};
use super::{Tool, ToolCtx, ToolOutput};
use crate::llm::ToolSpec;
use crate::platform::ProcessGroup;

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
/// Bytes of stderr kept in an error.
const STDERR_TAIL: usize = 2000;

/// One command tool; clones share the file read.
#[derive(Clone)]
pub struct CommandTool(Arc<Inner>);

struct Inner {
    name: String,
    description: String,
    path: PathBuf,
    file: OnceLock<Result<CommandFile, String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommandFile {
    #[serde(default)]
    schema: Option<Value>,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    /// `json` (the default) writes the arguments; `none` writes nothing.
    #[serde(default)]
    stdin: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

impl CommandTool {
    /// The tool of file `path` with the name and description of the index.
    pub fn new(name: &str, description: &str, path: &Path) -> CommandTool {
        CommandTool(Arc::new(Inner {
            name: name.to_string(),
            description: description.to_string(),
            path: path.to_path_buf(),
            file: OnceLock::new(),
        }))
    }

    fn file(&self) -> &Result<CommandFile, String> {
        self.0.file.get_or_init(|| {
            let path = &self.0.path;
            let bytes = std::fs::read(path).map_err(|err| format!("{}: {err}", path.display()))?;
            serde_json::from_slice(&bytes).map_err(|err| format!("{}: {err}", path.display()))
        })
    }
}

impl Tool for CommandTool {
    fn spec(&self) -> ToolSpec {
        let schema = match self.file() {
            Ok(file) => file.schema.clone(),
            Err(_) => None,
        };
        ToolSpec {
            name: self.0.name.clone(),
            description: self.0.description.clone(),
            schema: schema
                .filter(Value::is_object)
                .unwrap_or_else(|| json!({"type": "object", "properties": {}})),
        }
    }

    fn run(&self, args: &Value, ctx: &ToolCtx<'_>) -> ToolOutput {
        let file = match self.file() {
            Ok(file) => file,
            Err(err) => return ToolOutput::err(format!("tool file: {err}")),
        };
        let dir = self.0.path.parent().unwrap_or(Path::new("."));
        execute(file, dir, args, ctx).unwrap_or_else(ToolOutput::err)
    }
}

/// `command` as given, or joined to the tool file's folder when it is a
/// relative path (`./run.exe`).
fn program(command: &str, dir: &Path) -> PathBuf {
    let path = Path::new(command);
    let has_dir = command.contains(['/', '\\']);
    match has_dir && path.is_relative() {
        true => dir.join(path),
        false => path.to_path_buf(),
    }
}

fn execute(
    file: &CommandFile,
    dir: &Path,
    args: &Value,
    ctx: &ToolCtx<'_>,
) -> Result<ToolOutput, String> {
    let mut cmd = Command::new(program(&file.command, dir));
    cmd.args(&file.args)
        .current_dir(ctx.cwd)
        .env("FAST_HARNESS_TOOL_DIR", dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let (group, mut child) = ProcessGroup::spawn(&mut cmd)
        .map_err(|err| format!("cannot start `{}`: {err}", file.command))?;
    if let Some(mut stdin) = child.stdin.take() {
        let input = match file.stdin.as_deref() {
            Some("none") => String::new(),
            _ => args.to_string(),
        };
        // A writer thread: a program that never reads stdin cannot block us.
        std::thread::spawn(move || {
            let _ = stdin.write_all(input.as_bytes());
        });
    }
    let (out_tx, out_rx) = mpsc::channel();
    let (err_tx, err_rx) = mpsc::channel();
    if let Some(stdout) = child.stdout.take() {
        pump(stdout, out_tx);
    }
    if let Some(stderr) = child.stderr.take() {
        pump(stderr, err_tx);
    }
    let timeout_ms = file.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS).max(1);
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut out = Capture::default();
    let ending = supervise(&mut child, &group, &out_rx, &mut out, deadline, ctx.cancel);
    drop(group);
    drain(&out_rx, &mut out);
    let mut err = Capture::default();
    drain(&err_rx, &mut err);
    Ok(render(out, err, ending?))
}

fn render(out: Capture, err: Capture, ending: Ending) -> ToolOutput {
    let stdout = String::from_utf8_lossy(&out.bytes).into_owned();
    let stderr = String::from_utf8_lossy(&err.bytes);
    let tail = tail(stderr.trim_end());
    let problem = match ending {
        Ending::Exited(status) if status.success() => {
            return match stdout.is_empty() {
                true => ToolOutput::ok("(no output)"),
                false => ToolOutput::ok(stdout),
            };
        }
        Ending::Exited(status) => match status.code() {
            Some(code) => format!("[exit code: {code}]"),
            None => "[exit code: none]".to_string(),
        },
        Ending::TimedOut(ms, _) => format!("[timed out after {ms} ms; process tree killed]"),
        Ending::Cancelled(_) => "[cancelled; process tree killed]".to_string(),
    };
    let mut text = problem;
    if !tail.is_empty() {
        text.push_str(&format!("\nstderr:\n{tail}"));
    }
    if !stdout.trim().is_empty() {
        text.push_str(&format!("\nstdout:\n{}", stdout.trim_end()));
    }
    ToolOutput::err(text)
}

/// The last [`STDERR_TAIL`] bytes, cut at a character boundary.
fn tail(text: &str) -> &str {
    let mut start = text.len().saturating_sub(STDERR_TAIL);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}
