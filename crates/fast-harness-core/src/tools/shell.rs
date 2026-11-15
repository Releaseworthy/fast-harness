//! `shell`: run a command in PowerShell (Windows) or `sh` (elsewhere), its
//! whole process tree in one `platform::ProcessGroup`.

use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::args::{object_schema, opt_positive, req_str};
use super::clean::clean_output;
use super::shell_live::Live;
use super::{Tool, ToolCtx, ToolOutput};
use crate::llm::ToolSpec;
use crate::platform::ProcessGroup;

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MAX_TIMEOUT_MS: u64 = 600_000;
/// How often the exit, the deadline and the cancel flag are checked.
const POLL: Duration = Duration::from_millis(50);
/// How long output still arriving after the end is collected.
const DRAIN_GRACE: Duration = Duration::from_millis(1500);
/// Output kept in memory; the registry clips far below this anyway.
const MAX_CAPTURE: usize = 8 * 1024 * 1024;

/// Makes PowerShell write UTF-8 without colour codes (`$PSStyle` exists
/// from 7.2 on; Windows PowerShell 5 has none), stay quiet about progress
/// and format 400 columns wide: the hidden console is 120 wide, so
/// `Format-Table` cut or wrapped columns. A host without a resizable buffer
/// keeps its width (the `try`).
const PWSH_PRELUDE: &str = "[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)\n\
$OutputEncoding = [Console]::OutputEncoding\n\
if ($PSStyle) { $PSStyle.OutputRendering = 'PlainText' }\n\
$ProgressPreference = 'SilentlyContinue'\n\
try { $Host.UI.RawUI.BufferSize = [Management.Automation.Host.Size]::new(400, [Math]::Max(1, $Host.UI.RawUI.BufferSize.Height)) } catch {}\n\
$PSDefaultParameterValues['Out-String:Width'] = 400\n\
$PSDefaultParameterValues['Out-File:Width'] = 400\n\
$Error.Clear()\n";

/// After the command (stdin is closed and the host non-interactive, so a
/// prompt fails at once): exit 1 when the last statement failed (what
/// `-Command` reports anyway) or when any prompt could not be shown
/// (`Read-Host`, `Get-Credential`, `-Confirm`, a missing mandatory
/// parameter), even if later statements ran: the command did not get the
/// input it asked for. Other errors keep pwsh's own rule (a `Write-Error`
/// or a caught exception mid-script still exits 0); an `exit N` never
/// reaches this.
const PWSH_EPILOGUE: &str = "\n$__fhOk = $?\n\
$__fhPrompt = @($Error | Where-Object { $_.Exception -is [System.Management.Automation.Host.HostException] -or \"$($_.Exception.Message)\" -match 'NonInteractive' -or \"$($_.FullyQualifiedErrorId)\" -like 'MissingMandatoryParameter*' }).Count\n\
if ($__fhPrompt -or -not $__fhOk) { exit 1 }\n";

pub(crate) struct ShellTool;

impl Tool for ShellTool {
    fn spec(&self) -> ToolSpec {
        let shell = if cfg!(windows) {
            "PowerShell (pwsh -NoProfile -NonInteractive -Command)"
        } else {
            "sh -c"
        };
        ToolSpec {
            name: "shell".into(),
            description: format!(
                "Run a shell command with {shell} in the working directory. stdout and stderr \
                 are merged; the output ends with the exit code. On timeout or cancel the \
                 whole process tree is killed. Processes still running when the command ends \
                 are killed too: for a server, a watcher or anything long, set `background: true`: \
                 the command runs as a job of this session and the call returns its id at once \
                 with its first output; find_tools loads job_output (reads more, waits for the \
                 end), job_kill and job_list."
            ),
            schema: object_schema(
                json!({
                    "command": {"type": "string", "description": "The command line to run."},
                    "timeout_ms": {"type": "integer", "minimum": 1, "maximum": MAX_TIMEOUT_MS, "description": "Timeout in milliseconds. Default 120000, maximum 600000. A background job has none."},
                    "background": {"type": "boolean", "description": "Run as a background job until it ends, job_kill stops it or the session closes."}
                }),
                &["command"],
            ),
        }
    }

    fn run(&self, args: &Value, ctx: &ToolCtx<'_>) -> ToolOutput {
        match args.get("background") {
            None | Some(Value::Null) | Some(Value::Bool(false)) => {}
            Some(Value::Bool(true)) => {
                return match req_str(args, "command") {
                    Ok(command) => super::job_tools::start(command, ctx),
                    Err(message) => ToolOutput::err(message),
                }
            }
            Some(_) => return ToolOutput::err("`background` must be true or false"),
        }
        let parsed = req_str(args, "command").and_then(|command| {
            let timeout = opt_positive(args, "timeout_ms", DEFAULT_TIMEOUT_MS)?;
            Ok((command, timeout.min(MAX_TIMEOUT_MS)))
        });
        match parsed.and_then(|(command, timeout)| run_shell(command, timeout, ctx)) {
            Ok(output) => output,
            Err(message) => ToolOutput::err(message),
        }
    }
}

pub(super) enum Ending {
    Exited(ExitStatus),
    /// Killed at the deadline after this many ms: no exit code of its own.
    TimedOut(u64),
    /// Killed by the cancel flag: no exit code of its own.
    Cancelled,
}

#[derive(Default)]
pub(super) struct Capture<'r> {
    pub(super) bytes: Vec<u8>,
    pub(super) dropped: usize,
    /// The output as it arrives (`shell` only); changes nothing above.
    live: Live<'r>,
}

impl Capture<'_> {
    fn push(&mut self, chunk: &[u8]) {
        let room = MAX_CAPTURE.saturating_sub(self.bytes.len());
        let kept = room.min(chunk.len());
        self.bytes.extend_from_slice(&chunk[..kept]);
        self.dropped += chunk.len() - kept;
        self.live.push(chunk, Instant::now());
    }
}

fn run_shell(command: &str, timeout_ms: u64, ctx: &ToolCtx<'_>) -> Result<ToolOutput, String> {
    if command.trim().is_empty() {
        return Err("command must not be empty".to_string());
    }
    let mut cmd = shell_command(command);
    cmd.current_dir(ctx.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let (group, mut child) =
        ProcessGroup::spawn(&mut cmd).map_err(|err| format!("cannot start the shell: {err}"))?;
    let (tx, rx) = mpsc::channel();
    if let Some(stdout) = child.stdout.take() {
        pump(stdout, tx.clone());
    }
    if let Some(stderr) = child.stderr.take() {
        pump(stderr, tx);
    }
    let report = |text: &str| ctx.progress(text);
    let live = match ctx.reports_progress() {
        true => Live::new(&report),
        false => Live::off(),
    };
    let mut capture = Capture {
        live,
        ..Capture::default()
    };
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let ending = supervise(&mut child, &group, &rx, &mut capture, deadline, ctx.cancel);
    // Closing the group kills processes the command left running, so the
    // pipes they hold close and the readers finish.
    group.close();
    drain(&rx, &mut capture);
    // The rest when it is due; else the result, which follows at once.
    capture.live.flush(Instant::now());
    Ok(render(capture, ending?, timeout_ms))
}

pub(super) fn shell_command(command: &str) -> Command {
    if cfg!(windows) {
        let mut cmd = Command::new(powershell());
        cmd.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"])
            .arg(format!("{PWSH_PRELUDE}{command}{PWSH_EPILOGUE}"));
        cmd
    } else {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd
    }
}

/// `pwsh.exe` when it is on PATH, else Windows PowerShell.
fn powershell() -> &'static str {
    static PROGRAM: OnceLock<&'static str> = OnceLock::new();
    PROGRAM.get_or_init(|| {
        let on_path = std::env::var_os("PATH").is_some_and(|path| {
            std::env::split_paths(&path).any(|dir| dir.join("pwsh.exe").is_file())
        });
        if on_path {
            "pwsh.exe"
        } else {
            "powershell.exe"
        }
    })
}

/// Forwards everything `source` produces to `tx` from a background thread.
pub(super) fn pump(mut source: impl Read + Send + 'static, tx: Sender<Vec<u8>>) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match source.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
}

/// Collects output until the child exits, the deadline passes or `cancel`
/// is set; in the last two cases the whole group is killed.
pub(super) fn supervise(
    child: &mut Child,
    group: &ProcessGroup,
    rx: &Receiver<Vec<u8>>,
    capture: &mut Capture,
    deadline: Instant,
    cancel: &AtomicBool,
) -> Result<Ending, String> {
    let started = Instant::now();
    // Zero while the pipes are open; once they close, the pause between
    // exit checks, starting at 1 ms and doubling up to POLL.
    let mut idle = Duration::ZERO;
    loop {
        if idle.is_zero() {
            match rx.recv_timeout(POLL) {
                Ok(chunk) => capture.push(&chunk),
                Err(RecvTimeoutError::Timeout) => capture.live.flush(Instant::now()),
                Err(RecvTimeoutError::Disconnected) => idle = Duration::from_millis(1),
            }
        } else {
            std::thread::sleep(idle.min(deadline.saturating_duration_since(Instant::now())));
            idle = (idle * 2).min(POLL);
        }
        match child.try_wait() {
            Ok(Some(status)) => return Ok(Ending::Exited(status)),
            Ok(None) => {}
            Err(err) => {
                stop(child, group);
                return Err(format!("cannot wait for the shell: {err}"));
            }
        }
        if cancel.load(Ordering::Relaxed) {
            stop(child, group);
            return Ok(Ending::Cancelled);
        }
        if Instant::now() >= deadline {
            let elapsed = started.elapsed().as_millis() as u64;
            stop(child, group);
            return Ok(Ending::TimedOut(elapsed));
        }
    }
}

/// Kills the whole group (the direct child alone if that fails) and reaps it.
fn stop(child: &mut Child, group: &ProcessGroup) -> Option<ExitStatus> {
    if group.kill().is_err() {
        let _ = child.kill();
    }
    child.wait().ok()
}

/// Takes output still in flight, until both pipes close or the grace ends.
pub(super) fn drain(rx: &Receiver<Vec<u8>>, capture: &mut Capture) {
    let until = Instant::now() + DRAIN_GRACE;
    loop {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        match rx.recv_timeout(left) {
            Ok(chunk) => capture.push(&chunk),
            Err(_) => return,
        }
    }
}

fn render(capture: Capture, ending: Ending, timeout_ms: u64) -> ToolOutput {
    let mut text = clean_output(&capture.bytes);
    if text.is_empty() {
        text.push_str("(no output)");
    }
    if !text.ends_with('\n') {
        text.push('\n');
    }
    if capture.dropped > 0 {
        text.push_str(&format!(
            "[{} more bytes of output dropped]\n",
            capture.dropped
        ));
    }
    // A killed tree has no exit code of its own: the one the kill gave
    // (0xF4A50001, -190513151) means nothing to the model.
    let (status, killed) = match ending {
        Ending::Exited(status) => (Some(status), None),
        Ending::TimedOut(ms) => (None, Some(timed_out(ms, timeout_ms))),
        Ending::Cancelled => (None, Some("[cancelled; process tree killed]".into())),
    };
    if let Some(note) = &killed {
        text.push_str(note);
        text.push('\n');
    }
    let code = status.and_then(|status| status.code());
    // `none` is the documented marker of a killed or code-less process.
    match code {
        Some(code) => text.push_str(&format!("[exit code: {code}]")),
        None => text.push_str("[exit code: none]"),
    }
    ToolOutput::new(text, killed.is_some() || code != Some(0))
}

/// The note of a command killed at its deadline, in seconds.
fn timed_out(ms: u64, timeout_ms: u64) -> String {
    let seconds = ms as f64 / 1000.0;
    format!("[timed out after {seconds:.1} s (timeout_ms={timeout_ms}); process tree killed]")
}

#[cfg(all(test, windows))]
mod tests {
    /// The prelude turns colour off even where nothing else does (no
    /// `NO_COLOR`): PowerShell 7.2+ would print `Host` and colour WARNING.
    #[test]
    fn the_pwsh_prelude_renders_plain_text() {
        let script =
            "\"rendering $(if ($PSStyle) { $PSStyle.OutputRendering } else { 'none' })\"\n\
                      Write-Warning 'careful'";
        let mut cmd = super::shell_command(script);
        cmd.env_remove("NO_COLOR").env_remove("TERM");
        let out = cmd.output().expect("run PowerShell");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(!text.contains('\u{1b}'), "{text:?}");
        let rendering = text.lines().next().unwrap_or_default();
        assert!(
            matches!(rendering, "rendering PlainText" | "rendering none"),
            "{text:?}"
        );
    }

    /// A three-column table of 60-character values stays on one line, as
    /// in the shell tool's hidden console (`CREATE_NO_WINDOW`) and in Out-String.
    #[test]
    fn the_pwsh_prelude_formats_wide_tables_on_one_line() {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let row = "[pscustomobject]@{A='a'*60; B='b'*60; C='c'*60}";
        let script = format!("{row} | Format-Table\n{row} | Format-Table | Out-String");
        let mut cmd = super::shell_command(&script);
        let out = cmd.creation_flags(CREATE_NO_WINDOW).output().expect("run");
        let text = String::from_utf8_lossy(&out.stdout);
        let (a, c) = ("a".repeat(60), "c".repeat(60));
        let whole = text.lines().filter(|l| l.contains(&a) && l.contains(&c));
        assert_eq!(whole.count(), 2, "{text}");
    }
}
