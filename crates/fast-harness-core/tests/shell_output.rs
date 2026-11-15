//! What shell gives the model: progress frames, binary output, the exit
//! code of a killed command and of a failed prompt.

use std::sync::atomic::AtomicBool;

use fast_harness_core::tools::{builtin, ToolCtx, ToolOutput};
use serde_json::json;

fn run(command: &str, timeout_ms: u64) -> ToolOutput {
    let cwd = std::env::temp_dir();
    let cancel = AtomicBool::new(false);
    let ctx = ToolCtx::new(&cwd, &cancel);
    let args = json!({"command": command, "timeout_ms": timeout_ms});
    builtin().run("shell", &args.to_string(), &ctx)
}

fn pick<'a>(windows: &'a str, unix: &'a str) -> &'a str {
    if cfg!(windows) {
        windows
    } else {
        unix
    }
}

#[test]
fn progress_bars_keep_only_their_last_frame() {
    let command = pick(
        "[Console]::Out.Write(\"get 10%`rget 50%`rget 100%`r`nnext`n\")",
        "printf 'get 10%%\\rget 50%%\\rget 100%%\\r\\nnext\\n'",
    );
    let text = run(command, 60_000).text().into_owned();
    assert!(text.starts_with("get 100%\nnext\n"), "{text}");
    assert!(!text.contains("10%"), "{text}");
}

#[test]
fn binary_output_becomes_a_note() {
    let command = pick(
        "$b = [byte[]](0..255) * 20; [Console]::OpenStandardOutput().Write($b, 0, $b.Length)",
        "head -c 5120 /dev/urandom",
    );
    let text = run(command, 60_000).text().into_owned();
    assert!(text.contains("[binary output, "), "{text}");
    assert!(text.contains(" bytes omitted]"), "{text}");
    assert!(text.len() < 400, "{text}");
}

#[test]
fn a_timeout_reports_no_fake_exit_code() {
    let out = run(pick("Start-Sleep 30", "sleep 30"), 1500);
    let text = out.text();
    assert!(out.is_error(), "{text}");
    assert!(text.contains("[timed out after "), "{text}");
    assert!(
        text.contains(" s (timeout_ms=1500); process tree killed]"),
        "{text}"
    );
    assert!(text.ends_with("[exit code: none]"), "{text}");
}

#[cfg(windows)]
#[test]
fn a_prompt_without_a_console_fails_the_command() {
    let out = run(
        "$name = Read-Host 'name'; Write-Output \"after $name\"",
        60_000,
    );
    let text = out.text();
    assert!(text.contains("after"), "the script goes on: {text}");
    assert!(out.is_error() && text.ends_with("[exit code: 1]"), "{text}");
    // A non-terminating error alone keeps pwsh's exit 0.
    let out = run("Write-Error soft; Write-Output fine", 60_000);
    assert!(out.text().ends_with("[exit code: 0]"), "{}", out.text());
    // The last statement's failure still fails the command.
    let out = run("Get-Item nope-not-here -ErrorAction Stop", 60_000);
    assert!(out.text().ends_with("[exit code: 1]"), "{}", out.text());
}
