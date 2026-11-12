//! What write, edit and subagent report about their work.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use fast_harness_core::agent::ChildStats;
use fast_harness_core::events::AgentEvent;
use fast_harness_core::tools::{builtin, ToolCtx, ToolOutput};
use fast_harness_core::types::Usage;
use serde_json::{json, Value};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "fast-harness-report-{tag}-{}-{nanos}",
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

fn run(dir: &Path, name: &str, args: Value) -> ToolOutput {
    let cancel = AtomicBool::new(false);
    let ctx = ToolCtx::new(dir, &cancel);
    builtin().run(name, &args.to_string(), &ctx)
}

#[test]
fn write_reports_new_and_replaced_files() {
    let tmp = TempDir::new("write");
    let out = run(
        &tmp.0,
        "write",
        json!({"path": "a/c.txt", "content": "one\ntwo\nthree\n"}),
    );
    assert_eq!(
        out,
        ToolOutput::ok("Wrote 14 bytes (3 lines) to a/c.txt (new file)")
    );
    let out = run(&tmp.0, "write", json!({"path": "a/c.txt", "content": "x"}));
    assert_eq!(
        out,
        ToolOutput::ok("Wrote 1 bytes (1 line) to a/c.txt (replaced 14 bytes, 3 lines)")
    );
    let registry = builtin();
    let spec = registry.specs().iter().find(|s| s.name == "write");
    let description = spec.map(|s| s.description.clone()).unwrap_or_default();
    assert!(description.starts_with("Create or fully replace a UTF-8 text file"));
    assert!(description.contains("Missing parent folders are created."));
    assert!(description.ends_with("whether the file was new."));
}

#[test]
fn edit_reports_the_line_of_a_match_inside_a_line() {
    let tmp = TempDir::new("edit-one");
    std::fs::write(tmp.0.join("f.txt"), "alpha\nbeta gamma\ndelta\n").unwrap();
    let args = json!({"path": "f.txt", "old_string": "gamma", "new_string": "G"});
    let out = run(&tmp.0, "edit", args);
    assert_eq!(
        out,
        ToolOutput::ok("Replaced 1 occurrence in f.txt at line 2")
    );
}

#[test]
fn edit_reports_the_lines_of_every_match() {
    let tmp = TempDir::new("edit-many");
    std::fs::write(tmp.0.join("f.txt"), "x a\nb\nx x\n\nx\n").unwrap();
    let args = json!({"path": "f.txt", "old_string": "x", "new_string": "y", "replace_all": true});
    let out = run(&tmp.0, "edit", args);
    assert_eq!(
        out,
        ToolOutput::ok("Replaced 4 occurrences in f.txt at lines 1, 3, 3, 5")
    );
}

#[test]
fn edit_lists_at_most_ten_lines() {
    let tmp = TempDir::new("edit-cut");
    let text: String = (0..12).map(|_| "k\n").collect();
    std::fs::write(tmp.0.join("f.txt"), text).unwrap();
    let args = json!({"path": "f.txt", "old_string": "k", "new_string": "m", "replace_all": true});
    let out = run(&tmp.0, "edit", args);
    let want = "Replaced 12 occurrences in f.txt at lines 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, ...";
    assert_eq!(out, ToolOutput::ok(want));
}

#[test]
fn subagent_line_names_model_steps_tools_and_usage() {
    let mut stats = ChildStats::default();
    let request = |model: &str| AgentEvent::Request {
        model: model.into(),
        account: "a".into(),
        attempt: 1,
    };
    let call = |name: &str| AgentEvent::ToolCall {
        id: "1".into(),
        name: name.into(),
        arguments: "{}".into(),
    };
    for event in [
        request("m1"),
        call("read"),
        call("grep"),
        call("read"),
        request("m2"),
    ] {
        stats.record(event);
    }
    stats.steps = 3;
    stats.usage = Usage {
        input: 120,
        output: 45,
        ..Usage::default()
    };
    assert_eq!(
        stats.line("s1"),
        "[subagent session s1 · model m2 · 3 steps · tools read, grep · in 120 out 45]"
    );
    let empty = ChildStats::default();
    assert_eq!(empty.line("s2"), "[subagent session s2]");
    let one = ChildStats {
        steps: 1,
        ..ChildStats::default()
    };
    assert_eq!(one.line("s3"), "[subagent session s3 · 1 step]");
}
