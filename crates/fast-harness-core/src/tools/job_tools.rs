//! `job_output`, `job_kill` and `job_list` (PLAN.md D68): the deferred
//! tools that read, stop and list the background jobs `shell` starts with
//! `background: true`, and that start itself.

use std::time::Duration;

use serde_json::{json, Value};

use super::args::object_schema;
use super::jobs::{JobInfo, JobState, Reading};
use super::{Effect, Tool, ToolCtx, ToolOutput};
use crate::llm::ToolSpec;

pub const JOB_OUTPUT: &str = "job_output";
pub const JOB_KILL: &str = "job_kill";
pub const JOB_LIST: &str = "job_list";
/// How long a start waits for first output or the end before it returns.
const START_GRACE: Duration = Duration::from_millis(1000);
/// The longest `wait_ms` of `job_output`.
const MAX_WAIT_MS: u64 = 120_000;
/// Characters of a command a list row shows.
const LIST_COMMAND: usize = 100;

/// Starts `command` as a job of the session (`shell` with `background`).
pub(super) fn start(command: &str, ctx: &ToolCtx<'_>) -> ToolOutput {
    let Some(session) = ctx.session else {
        return ToolOutput::err("background jobs need a session; run it without `background`");
    };
    let jobs = session.jobs();
    let id = match jobs.start(command, ctx.cwd, session.out_dir.as_deref()) {
        Ok(id) => id,
        Err(message) => return ToolOutput::err(message),
    };
    match jobs.read(&id, START_GRACE, true, Some(ctx.cancel)) {
        Ok(reading) => ToolOutput::ok(format!(
            "started {id} in the background\n{}",
            render(&reading)
        )),
        Err(message) => ToolOutput::err(message),
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Output,
    Kill,
    List,
}

pub(super) struct JobTool(Kind);

/// The three job tools in their fixed order.
pub(super) fn all() -> Vec<Box<dyn Tool>> {
    [Kind::Output, Kind::Kill, Kind::List]
        .into_iter()
        .map(|kind| Box::new(JobTool(kind)) as Box<dyn Tool>)
        .collect()
}

fn id_prop() -> Value {
    json!({"type": "string", "description": "The job id `shell` returned (`job-1`)."})
}

impl Tool for JobTool {
    /// Reading and listing run nothing; a kill ends processes.
    fn effect(&self) -> Effect {
        match self.0 {
            Kind::Output | Kind::List => Effect::Read,
            Kind::Kill => Effect::Run,
        }
    }

    fn spec(&self) -> ToolSpec {
        let (name, description, schema) = match self.0 {
            Kind::Output => (
                JOB_OUTPUT,
                "Read the new output of a background job (`shell` with `background: true`) \
                 since the last read, with its state and exit code. `wait_ms` waits that long \
                 for new output, or with `until: \"end\"` for the job to end.",
                object_schema(
                    json!({
                        "id": id_prop(),
                        "wait_ms": {"type": "integer", "minimum": 0, "maximum": MAX_WAIT_MS, "description": "Milliseconds to wait. Default 0: return at once."},
                        "until": {"type": "string", "enum": ["output", "end"], "description": "What ends the wait: new output (default) or the end of the job."}
                    }),
                    &["id"],
                ),
            ),
            Kind::Kill => (
                JOB_KILL,
                "Stop a background job and every process it started.",
                object_schema(json!({"id": id_prop()}), &["id"]),
            ),
            Kind::List => (
                JOB_LIST,
                "List the background jobs of this session: id, state, exit code, command.",
                object_schema(json!({}), &[]),
            ),
        };
        ToolSpec {
            name: name.into(),
            description: description.into(),
            schema,
        }
    }

    fn run(&self, args: &Value, ctx: &ToolCtx<'_>) -> ToolOutput {
        let Some(session) = ctx.session else {
            return ToolOutput::err("background jobs need a session");
        };
        let jobs = session.jobs();
        let id = args["id"].as_str().unwrap_or_default();
        let result = match self.0 {
            Kind::Output => wait_args(args)
                .and_then(|(wait, end)| jobs.read(id, wait, end, Some(ctx.cancel)))
                .map(|reading| render(&reading)),
            Kind::Kill => jobs.kill(id).map(|info| killed(&info)),
            Kind::List => Ok(list(&jobs.list())),
        };
        match result {
            Ok(text) => ToolOutput::ok(text),
            Err(message) => ToolOutput::err(message),
        }
    }
}

fn wait_args(args: &Value) -> Result<(Duration, bool), String> {
    let wait = match args.get("wait_ms") {
        None | Some(Value::Null) => 0,
        Some(value) => value
            .as_u64()
            .ok_or("`wait_ms` must be a whole number of milliseconds, 0 or more")?,
    };
    let end = match args.get("until").and_then(Value::as_str) {
        None | Some("output") => false,
        Some("end") => true,
        Some(other) => return Err(format!("`until` is `output` or `end`, not `{other}`")),
    };
    Ok((Duration::from_millis(wait.min(MAX_WAIT_MS)), end))
}

/// The new output, then the job's state (and exit code once it ended).
pub fn render(reading: &Reading) -> String {
    let mut text = String::new();
    if reading.skipped > 0 {
        let log = match &reading.info.log {
            Some(path) => format!("; the log keeps the first 16 MiB: {}", path.display()),
            None => String::new(),
        };
        text.push_str(&format!(
            "[{} bytes left the buffer before this read{log}]\n",
            reading.skipped
        ));
    }
    match reading.text.is_empty() {
        true => text.push_str("(no new output)\n"),
        false => text.push_str(&reading.text),
    }
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text + &status(&reading.info)
}

fn status(info: &JobInfo) -> String {
    let code = match info.exit_code {
        Some(code) => format!("[exit code: {code}]"),
        None => "[exit code: none]".to_string(),
    };
    match info.state {
        JobState::Running => format!(
            "[{} is running: job_output reads more, job_kill stops it]",
            info.id
        ),
        JobState::Killed => format!("[{} was killed with its process tree]\n{code}", info.id),
        _ => format!("[{} ended]\n{code}", info.id),
    }
}

fn killed(info: &JobInfo) -> String {
    match info.state {
        JobState::Running => format!("[{} did not end within 5 s of the kill]", info.id),
        _ => status(info),
    }
}

fn list(jobs: &[JobInfo]) -> String {
    if jobs.is_empty() {
        return "No background jobs in this session.".to_string();
    }
    let rows: Vec<String> = jobs.iter().map(row).collect();
    rows.join("\n")
}

fn row(info: &JobInfo) -> String {
    let state = match (info.state, info.exit_code) {
        (JobState::Running, _) => "running".to_string(),
        (JobState::Killed, _) => "killed".to_string(),
        (_, Some(code)) => format!("exited {code}"),
        (_, None) => "exited".to_string(),
    };
    let flat = info.command.replace(['\r', '\n'], " ");
    let mut command: String = flat.chars().take(LIST_COMMAND).collect();
    if flat.chars().count() > LIST_COMMAND {
        command.push_str("...");
    }
    format!("{}  {state}  {} bytes  {command}", info.id, info.bytes)
}
