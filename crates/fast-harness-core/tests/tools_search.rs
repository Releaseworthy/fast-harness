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
        // An empty result says what was not searched and how to include it.
        "No matches.\n[1 file and 1 folder (1 file inside) skipped by .gitignore (include_ignored: true); \
         2 hidden entries skipped (include_hidden: true); 1 binary file not searched]\n"
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
    // The last line counts what the walk skipped.
    let note = text.lines().last().unwrap_or_default();
    assert!(
        note.contains("skipped by .gitignore (include_ignored: true)"),
        "{text}"
    );
    let lines: Vec<&str> = text.lines().filter(|line| !line.starts_with('[')).collect();
    assert_eq!(lines.first(), Some(&"src/new.rs"), "{}", out.text());
    assert_eq!(lines.last(), Some(&"src/old.rs"), "{}", out.text());
    assert_eq!(lines.len(), 4);
    let txt = run(&tmp.0, "glob", json!({"pattern": "**/*.txt", "path": "."}));
    assert!(txt.text().starts_with("notes.txt\n"), "{}", txt.text());
    let none = run(&tmp.0, "glob", json!({"pattern": "*.zig"}));
    assert!(
        none.text().starts_with("No files found."),
        "{}",
        none.text()
    );
    let bad = run(
        &tmp.0,
        "glob",
        json!({"pattern": "*.rs", "path": "notes.txt"}),
    );
    assert!(bad.is_error());
}

#[test]
fn grep_shows_a_window_around_a_match_deep_in_a_long_line() {
    let tmp = TempDir::new("grep-window");
    // Multibyte padding: the window must cut on character boundaries.
    let line = format!(
        "{}needle{}needle{}\n",
        "é".repeat(4994),
        "y".repeat(1000),
        "z".repeat(50)
    );
    tmp.file("min.js", line.as_bytes());
    let out = run(
        &tmp.0,
        "grep",
        json!({"pattern": "needle", "path": "min.js"}),
    );
    let text = out.text();
    let shown = text.lines().nth(1).unwrap();
    assert!(
        shown.starts_with("  1: ...é") && shown.contains("needle"),
        "{shown}"
    );
    let note = "y... [line cut: columns 4920-5219 of 6056; 1 more matches outside]";
    assert!(shown.ends_with(note), "{shown}");
    assert_eq!(
        shown.chars().filter(|c| matches!(c, 'é' | 'y')).count() + 6,
        300
    );
    // A match at the end of the line: the window ends there, no trailing cut.
    tmp.file("end.js", format!("{}tail\n", "x".repeat(2000)).as_bytes());
    let out = run(&tmp.0, "grep", json!({"pattern": "tail", "path": "end.js"}));
    let text = out.text();
    let shown = text.lines().nth(1).unwrap();
    assert!(
        shown.ends_with("xtail [line cut: columns 1705-2004 of 2004]"),
        "{shown}"
    );
}

/// A max_results far above what the byte budget holds is capped, and the
/// shown part says how many of all the matches it holds (owner, 2026-11-15).
#[test]
fn a_huge_max_results_is_capped_and_says_what_it_shows() {
    let tmp = TempDir::new("grep-huge");
    let line = format!("hit {}\n", "y".repeat(60));
    tmp.file("many.txt", line.repeat(3000).as_bytes());
    let out = run(
        &tmp.0,
        "grep",
        json!({"pattern": "hit", "max_results": 100000}),
    );
    let text = out.text();
    assert!(text.len() <= 8 * 1024, "{} bytes", text.len());
    assert!(
        !text.contains("[output cut") && !text.contains("[full output"),
        "{text}"
    );
    let shown = text.lines().filter(|l| l.contains(": hit")).count();
    assert!(
        text.contains(&format!("showing {shown} of 3000 matches")),
        "{text}"
    );
    // D91: the limit named is the number shown.
    let capped = format!("max_results 100000 capped to {shown} ");
    let held = format!("output budget holds {shown};");
    assert!(text.contains(&capped) || text.contains(&held), "{text}");
    // Long lines fill the budget before max_results: the note still counts.
    let long = format!("hit {}\n", "z".repeat(280));
    tmp.file("many.txt", long.repeat(200).as_bytes());
    let out = run(
        &tmp.0,
        "grep",
        json!({"pattern": "hit", "max_results": 100}),
    );
    let text = out.text();
    let shown = text.lines().filter(|l| l.contains(": hit")).count();
    assert!(
        shown < 100 && text.len() <= 8 * 1024,
        "{shown} {}",
        text.len()
    );
    assert!(
        text.contains(&format!("showing {shown} of 200 matches")),
        "{text}"
    );
}

/// A folder .gitignore leaves out also counts the files in it (matching the
/// glob), bounded so a huge target/ stays cheap (owner, 2026-11-15).
#[test]
fn glob_counts_the_files_inside_ignored_folders() {
    let tmp = tree("glob-inside");
    tmp.file("build/a.txt", b"");
    tmp.file("build/deep/b.txt", b"");
    tmp.file("build/c.rs", b"");
    let out = run(&tmp.0, "glob", json!({"pattern": "*.txt"}));
    let note = "[1 file and 1 folder (3 files inside) skipped by .gitignore";
    assert!(out.text().contains(note), "{}", out.text());
    for i in 0..1001 {
        tmp.file(&format!("build/many/{i}.txt"), b"");
    }
    let out = run(&tmp.0, "glob", json!({"pattern": "*.txt"}));
    let note = "[1 file and 1 folder (1000+ files inside) skipped by .gitignore";
    assert!(out.text().contains(note), "{}", out.text());
}
