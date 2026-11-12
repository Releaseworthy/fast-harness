//! Running one hook (PLAN.md D44): the event JSON on stdin, the answer on
//! stdout. The process tree lives in one `platform::ProcessGroup`, killed
//! on timeout or cancel. A timeout, a non-zero exit, a start failure or a
//! bad answer is an error the caller reports and ignores (fail open).

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::command::program;
use super::hooks::{Answer, HookEntry};
use super::shell::{drain, pump, supervise, Capture, Ending};
use crate::platform::ProcessGroup;

/// Bytes of stderr kept in an error.
const STDERR_TAIL: usize = 500;

/// One hook run: its duration and its answer or the reason it was ignored.
#[derive(Clone, Debug)]
pub struct HookRun {
    pub ms: u64,
    pub answer: Result<Answer, String>,
}

/// Runs `entry` in `cwd` with `input` on stdin.
pub fn run(entry: &HookEntry, input: &Value, cwd: &Path, cancel: &AtomicBool) -> HookRun {
    let started = Instant::now();
    let answer = execute(entry, input, cwd, cancel).and_then(|out| Answer::parse(&out));
    HookRun {
        ms: started.elapsed().as_millis() as u64,
        answer,
    }
}

fn execute(
    entry: &HookEntry,
    input: &Value,
    cwd: &Path,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let mut cmd = Command::new(program(&entry.command, &entry.dir));
    cmd.args(&entry.args)
        .current_dir(cwd)
        .env("FAST_HARNESS_HOOKS_DIR", &entry.dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let (group, mut child) =
        ProcessGroup::spawn(&mut cmd).map_err(|err| format!("cannot start: {err}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let text = input.to_string();
        // A writer thread: a hook that never reads stdin cannot block us.
        std::thread::spawn(move || {
            let _ = stdin.write_all(text.as_bytes());
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
    let deadline = Instant::now() + Duration::from_millis(entry.timeout_ms());
    let mut out = Capture::default();
    let ending = supervise(&mut child, &group, &out_rx, &mut out, deadline, cancel);
    group.close();
    drain(&out_rx, &mut out);
    let mut err = Capture::default();
    drain(&err_rx, &mut err);
    let stdout = String::from_utf8_lossy(&out.bytes).into_owned();
    let stderr = String::from_utf8_lossy(&err.bytes);
    let problem = match ending? {
        Ending::Exited(status) if status.success() => return Ok(stdout),
        Ending::Exited(status) => match status.code() {
            Some(code) => format!("exit code {code}"),
            None => "exit code none".into(),
        },
        Ending::TimedOut(ms, _) => format!("timed out after {ms} ms; process tree killed"),
        Ending::Cancelled(_) => "cancelled; process tree killed".into(),
    };
    Err(match tail(stderr.trim()) {
        "" => problem,
        tail => format!("{problem}: {tail}"),
    })
}

/// The last [`STDERR_TAIL`] bytes, cut at a character boundary.
fn tail(text: &str) -> &str {
    let mut start = text.len().saturating_sub(STDERR_TAIL);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}
