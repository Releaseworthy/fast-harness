//! Phase 10 experiment E4 (docs/perf.md): how long the `shell` tool takes for
//! `echo 1`, next to a bare PowerShell start. Ignored: it measures, it does
//! not check. Run with
//! `cargo test --release -p fast-harness-core --test shell_echo_timing -- --ignored --nocapture`.

use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use fast_harness_core::tools::{builtin, ToolCtx};
use serde_json::json;

const RUNS: usize = 10;

fn summary(label: &str, mut times: Vec<Duration>) {
    times.sort();
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    println!(
        "{label}: min {:.0} ms, median {:.0} ms, max {:.0} ms",
        ms(times[0]),
        ms(times[times.len() / 2]),
        ms(times[times.len() - 1])
    );
}

fn timed(mut f: impl FnMut()) -> Vec<Duration> {
    (0..RUNS)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed()
        })
        .collect()
}

#[test]
#[ignore = "measurement, run by hand"]
fn shell_echo_one_timing() {
    let cwd = std::env::temp_dir().join(format!("fh-e4-{}", std::process::id()));
    std::fs::create_dir_all(&cwd).unwrap();
    let cancel = AtomicBool::new(false);
    let ctx = ToolCtx::new(&cwd, &cancel);
    let registry = builtin();
    let args = json!({"command": "echo 1"}).to_string();
    let tool = timed(|| {
        let out = registry.run("shell", &args, &ctx);
        assert!(out.text().contains('1'), "{}", out.text());
    });
    let program = if cfg!(windows) { "pwsh.exe" } else { "sh" };
    let flags: &[&str] = if cfg!(windows) {
        &[
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "echo 1",
        ]
    } else {
        &["-c", "echo 1"]
    };
    let bare = timed(|| {
        let status = Command::new(program)
            .args(flags)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    });
    // The tool's UTF-8 prelude, without the tool's supervision loop.
    let prelude = "[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)\n\
        $OutputEncoding = [Console]::OutputEncoding\n$ProgressPreference = 'SilentlyContinue'\necho 1";
    let with_prelude = timed(|| {
        let mut cmd = Command::new(program);
        if cfg!(windows) {
            cmd.args(&flags[..4]).arg(prelude);
        } else {
            cmd.args(flags);
        }
        let status = cmd.stdout(Stdio::null()).stderr(Stdio::null()).status();
        assert!(status.unwrap().success());
    });
    summary("shell tool `echo 1`", tool);
    summary("bare shell start `echo 1`", bare);
    summary("bare shell start with the tool prelude", with_prelude);
    std::fs::remove_dir_all(&cwd).unwrap();
}
