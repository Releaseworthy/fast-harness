//! grep and glob through the built-in registry.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

use fast_harness_core::tools::{builtin, ToolCtx, ToolOutput};
use serde_json::{json, Value};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "fast-harness-tools-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    fn file(&self, rel: &str, content: &[u8]) -> PathBuf {
        let path = self.0.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        path
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

fn tree(tag: &str) -> TempDir {
    let tmp = TempDir::new(tag);
    tmp.file(".gitignore", b"ignored.txt\nbuild/\n");
    tmp.file(
        "src/main.rs",
        b"fn main() {\n    println!(\"Needle\");\n}\n",
    );
    tmp.file("src/lib.rs", b"// needle in lib\n");
    tmp.file("notes.txt", b"a needle here\n");
    tmp.file("ignored.txt", b"needle ignored\n");
    tmp.file("build/out.txt", b"needle built\n");
    tmp.file(".hidden/h.txt", b"needle hidden\n");
    tmp.file("blob.bin", b"needle\0binary");
    tmp
}

#[test]
fn grep_honours_gitignore_and_skips_hidden_and_binary() {
    let tmp = tree("grep");
    let out = run(&tmp.0, "grep", json!({"pattern": "needle"}));
    assert!(!out.is_error(), "{}", out.text());
    assert!(
        out.text().contains("notes.txt\n  1: a needle here\n"),
        "{}",
        out.text()
    );
    assert!(out.text().contains("src/lib.rs\n  1: // needle in lib\n"));
    for absent in ["ignored.txt", "build/", ".hidden", "blob.bin", "main.rs"] {
        assert!(!out.text().contains(absent), "{absent} in {}", out.text());
    }
    let out = run(
        &tmp.0,
        "grep",
        json!({"pattern": "NEEDLE", "ignore_case": true, "glob": "*.rs"}),
    );
    assert!(out.text().contains("src/main.rs\n  2:"), "{}", out.text());
    assert!(!out.text().contains("notes.txt"));
    let out = run(
        &tmp.0,
        "grep",
        json!({"pattern": "needle", "glob": "*.txt"}),
    );
    assert!(!out.text().contains("ignored.txt"), "{}", out.text());
}

#[test]
fn grep_cuts_results_and_long_lines() {
    let tmp = TempDir::new("grep-cut");
    let many: String = (0..10).map(|i| format!("hit {i}\n")).collect();
    tmp.file("many.txt", many.as_bytes());
    tmp.file("long.txt", format!("hit{}\n", "x".repeat(1000)).as_bytes());
    let out = run(&tmp.0, "grep", json!({"pattern": "hit", "max_results": 3}));
    assert_eq!(
        out.text().lines().filter(|l| l.contains(": hit")).count(),
        3
    );
    let more = "[8 more matches in 1 file;";
    assert!(out.text().contains(more), "{}", out.text());
    let out = run(
        &tmp.0,
        "grep",
        json!({"pattern": "hitx", "path": "long.txt"}),
    );
    let text = out.text();
    let line = text.lines().nth(1).unwrap();
    assert!(line.len() < 400 && line.ends_with("[line cut]"), "{line}");
}

#[test]
fn grep_reports_errors_and_no_matches() {
    let tmp = tree("grep-err");
    let bad = run(&tmp.0, "grep", json!({"pattern": "("}));
    assert!(bad.is_error() && bad.text().contains("invalid regex"));
    let missing = run(&tmp.0, "grep", json!({"pattern": "x", "path": "nope"}));
    assert!(missing.is_error() && missing.text().contains("not found"));
    assert_eq!(
        run(&tmp.0, "grep", json!({"pattern": "zzzq"})).text(),
        "No matches."
    );
}

#[test]
fn glob_lists_newest_first_and_respects_gitignore() {
    let tmp = tree("glob");
    let old = tmp.file("src/old.rs", b"");
    let past = SystemTime::now() - Duration::from_secs(3600);
    std::fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(past)
        .unwrap();
    let newest = tmp.file("src/new.rs", b"");
    let future = SystemTime::now() + Duration::from_secs(3600);
    std::fs::File::options()
        .write(true)
        .open(&newest)
        .unwrap()
        .set_modified(future)
        .unwrap();
    let out = run(&tmp.0, "glob", json!({"pattern": "*.rs"}));
    let text = out.text();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.first(), Some(&"src/new.rs"), "{}", out.text());
    assert_eq!(lines.last(), Some(&"src/old.rs"), "{}", out.text());
    assert_eq!(lines.len(), 4);
    let txt = run(&tmp.0, "glob", json!({"pattern": "**/*.txt", "path": "."}));
    assert_eq!(txt.text(), "notes.txt\n");
    let none = run(&tmp.0, "glob", json!({"pattern": "*.zig"}));
    assert_eq!(none.text(), "No files found.");
    let bad = run(
        &tmp.0,
        "glob",
        json!({"pattern": "*.rs", "path": "notes.txt"}),
    );
    assert!(bad.is_error());
}
