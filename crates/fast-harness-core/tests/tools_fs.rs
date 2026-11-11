//! read, write and edit through the built-in registry.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

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

fn run(dir: &Path, name: &str, args: Value) -> ToolOutput {
    let cancel = AtomicBool::new(false);
    let ctx = ToolCtx {
        cwd: dir,
        cancel: &cancel,
    };
    builtin().run(name, &args.to_string(), &ctx)
}

#[test]
fn builtin_lists_six_tools_with_schemas() {
    let registry = builtin();
    let names: Vec<&str> = registry.specs().iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["read", "write", "edit", "shell", "grep", "glob"]);
    for spec in registry.specs() {
        assert!(!spec.description.is_empty());
        assert_eq!(spec.schema["type"], "object");
        assert!(spec.schema["required"]
            .as_array()
            .is_some_and(|r| !r.is_empty()));
    }
}

#[test]
fn write_creates_parents_and_read_numbers_lines() {
    let tmp = TempDir::new("rw");
    let out = run(
        &tmp.0,
        "write",
        json!({"path": "a/b/c.txt", "content": "one\ntwo\nthree\n"}),
    );
    assert!(!out.is_error, "{}", out.content);
    assert!(out.content.contains("14 bytes"), "{}", out.content);
    let out = run(&tmp.0, "read", json!({"path": "a/b/c.txt"}));
    assert_eq!(out, ToolOutput::ok("1: one\n2: two\n3: three\n"));
    let abs = tmp.0.join("a/b/c.txt");
    let out = run(
        &tmp.0,
        "read",
        json!({"path": abs.to_string_lossy(), "offset": 2, "limit": 1}),
    );
    assert!(out.content.starts_with("2: two\n"), "{}", out.content);
    assert!(out.content.contains("1 more lines, continue with offset=3"));
}

#[test]
fn read_reports_errors() {
    let tmp = TempDir::new("read-err");
    std::fs::write(tmp.0.join("bin.dat"), b"abc\0def").unwrap();
    std::fs::write(tmp.0.join("short.txt"), "x\n").unwrap();
    std::fs::write(tmp.0.join("empty.txt"), "").unwrap();
    let dir = run(&tmp.0, "read", json!({"path": "."}));
    assert!(
        dir.is_error && dir.content.contains("directory"),
        "{}",
        dir.content
    );
    let bin = run(&tmp.0, "read", json!({"path": "bin.dat"}));
    assert!(
        bin.is_error && bin.content.contains("binary"),
        "{}",
        bin.content
    );
    let missing = run(&tmp.0, "read", json!({"path": "nope.txt"}));
    assert!(missing.is_error && missing.content.contains("nope.txt"));
    let past = run(&tmp.0, "read", json!({"path": "short.txt", "offset": 5}));
    assert!(past.is_error && past.content.contains("past the end"));
    let bad = run(&tmp.0, "read", json!({"path": "short.txt", "offset": 0}));
    assert!(bad.is_error && bad.content.contains("positive integer"));
    assert!(run(&tmp.0, "read", json!({}))
        .content
        .contains("missing required"));
    assert_eq!(
        run(&tmp.0, "read", json!({"path": "empty.txt"})).content,
        "(empty file)"
    );
}

#[test]
fn edit_requires_a_unique_match_unless_replace_all() {
    let tmp = TempDir::new("edit");
    let file = tmp.0.join("f.txt");
    std::fs::write(&file, "foo bar foo\n").unwrap();
    let dup = run(
        &tmp.0,
        "edit",
        json!({"path": "f.txt", "old_string": "foo", "new_string": "baz"}),
    );
    assert!(
        dup.is_error && dup.content.contains("matches 2 times"),
        "{}",
        dup.content
    );
    let all = run(
        &tmp.0,
        "edit",
        json!({"path": "f.txt", "old_string": "foo", "new_string": "baz", "replace_all": true}),
    );
    assert!(
        !all.is_error && all.content.contains("Replaced 2 occurrences"),
        "{}",
        all.content
    );
    let one = run(
        &tmp.0,
        "edit",
        json!({"path": "f.txt", "old_string": "bar", "new_string": "qux"}),
    );
    assert!(
        one.content.contains("Replaced 1 occurrence"),
        "{}",
        one.content
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "baz qux baz\n");
    let none = run(
        &tmp.0,
        "edit",
        json!({"path": "f.txt", "old_string": "zzz", "new_string": "y"}),
    );
    assert!(none.is_error && none.content.contains("not found"));
    let same = run(
        &tmp.0,
        "edit",
        json!({"path": "f.txt", "old_string": "baz", "new_string": "baz"}),
    );
    assert!(same.is_error);
    let gone = run(
        &tmp.0,
        "edit",
        json!({"path": "x.txt", "old_string": "a", "new_string": "b"}),
    );
    assert!(gone.is_error && gone.content.contains("x.txt"));
}

#[test]
fn edit_retries_with_crlf() {
    let tmp = TempDir::new("crlf");
    let file = tmp.0.join("win.txt");
    std::fs::write(&file, "alpha\r\nbeta\r\ngamma\r\n").unwrap();
    let out = run(
        &tmp.0,
        "edit",
        json!({"path": "win.txt", "old_string": "alpha\nbeta", "new_string": "one\ntwo"}),
    );
    assert!(!out.is_error, "{}", out.content);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "one\r\ntwo\r\ngamma\r\n"
    );
}
