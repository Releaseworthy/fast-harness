//! shell through the built-in registry: exit codes, merged output, timeout
//! killing a whole process tree, cancel.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use fast_harness_core::tools::{builtin, ToolCtx, ToolOutput};
use serde_json::{json, Value};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "fast-harness-tools-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run_with(dir: &Path, args: Value, cancel: &AtomicBool) -> ToolOutput {
    let ctx = ToolCtx::new(dir, cancel);
    builtin().run("shell", &args.to_string(), &ctx)
}

fn run(dir: &Path, args: Value) -> ToolOutput {
    run_with(dir, args, &AtomicBool::new(false))
}

#[test]
fn shell_reports_output_cwd_and_exit_code() {
    let tmp = TempDir::new("sh-ok");
    std::fs::write(tmp.0.join("marker.txt"), "x").unwrap();
    let list = if cfg!(windows) {
        "Get-ChildItem -Name; Write-Output 'h\u{e9}llo \u{2713}'"
    } else {
        "ls; echo 'h\u{e9}llo \u{2713}'"
    };
    let out = run(&tmp.0, json!({"command": list}));
    assert!(!out.is_error(), "{}", out.text());
    assert!(out.text().contains("marker.txt"), "{}", out.text());
    assert!(out.text().contains("h\u{e9}llo \u{2713}"), "{}", out.text());
    assert!(out.text().ends_with("[exit code: 0]"), "{}", out.text());
    let out = run(&tmp.0, json!({"command": "exit 3"}));
    assert!(
        out.is_error() && out.text().ends_with("[exit code: 3]"),
        "{}",
        out.text()
    );
}

#[test]
fn shell_merges_stderr() {
    let tmp = TempDir::new("sh-err");
    let command = if cfg!(windows) {
        "Write-Output out1; [Console]::Error.WriteLine('err1'); Write-Output out2"
    } else {
        "echo out1; echo err1 >&2; echo out2"
    };
    let out = run(&tmp.0, json!({"command": command}));
    for text in ["out1", "err1", "out2", "[exit code: 0]"] {
        assert!(out.text().contains(text), "{text} missing: {}", out.text());
    }
    assert!(run(&tmp.0, json!({})).text().contains("missing required"));
    assert!(run(&tmp.0, json!({"command": "x", "timeout_ms": "5"})).is_error());
}

#[test]
fn shell_cancel_kills_the_command() {
    let tmp = TempDir::new("sh-cancel");
    let cancel = AtomicBool::new(false);
    let started = Instant::now();
    let out = std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(Duration::from_millis(1500));
            cancel.store(true, Ordering::Relaxed);
        });
        let sleep = if cfg!(windows) {
            "Start-Sleep 30"
        } else {
            "sleep 30"
        };
        run_with(&tmp.0, json!({"command": sleep}), &cancel)
    });
    assert!(
        out.is_error() && out.text().contains("cancelled"),
        "{}",
        out.text()
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
}

/// Pid printed by the command as `<label>=<pid>`.
#[cfg(windows)]
fn pid_of(output: &str, label: &str) -> u32 {
    let prefix = format!("{label}=");
    let line = output.lines().find(|line| line.starts_with(&prefix));
    let line = line.unwrap_or_else(|| panic!("no {label} pid in {output}"));
    line[prefix.len()..].trim().parse().unwrap()
}

#[cfg(windows)]
fn alive(pid: u32) -> bool {
    let out = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).contains(&format!("\"{pid}\""))
}

#[cfg(windows)]
#[test]
fn shell_timeout_kills_the_process_tree() {
    assert!(
        alive(std::process::id()),
        "the liveness probe must see a live pid"
    );
    let tmp = TempDir::new("sh-tree");
    let command = "Write-Output \"PARENT=$PID\"; \
        $p = Start-Process pwsh -ArgumentList '-NoProfile','-Command','Start-Sleep 30' \
        -NoNewWindow -PassThru; Write-Output \"CHILD=$($p.Id)\"; Start-Sleep 30";
    let started = Instant::now();
    let out = run(&tmp.0, json!({"command": command, "timeout_ms": 6000}));
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "{:?}",
        started.elapsed()
    );
    assert!(
        out.is_error() && out.text().contains("timed out"),
        "{}",
        out.text()
    );
    let parent = pid_of(&out.text(), "PARENT");
    let child = pid_of(&out.text(), "CHILD");
    let deadline = Instant::now() + Duration::from_secs(10);
    while (alive(parent) || alive(child)) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(!alive(parent), "parent {parent} still alive");
    assert!(!alive(child), "child {child} still alive");
}
