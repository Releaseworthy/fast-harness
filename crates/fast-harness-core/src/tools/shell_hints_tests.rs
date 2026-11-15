use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use serde_json::json;

use super::{annotate, reads_file, segments, walks};
use crate::tools::{Tool, ToolCtx, ToolOutput};

/// A fresh folder under the temp dir, removed on drop.
struct Dir(PathBuf);

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A repository with `target/` ignored: `src/a.rs`, `target/debug/x.rlib`.
fn repo(name: &str) -> Dir {
    let root = std::env::temp_dir().join(format!("fh-hints-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for dir in [".git", "src", "target/debug"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
    std::fs::write(root.join("src/a.rs"), "fn a() {}\n").unwrap();
    std::fs::write(root.join("target/debug/x.rlib"), "x").unwrap();
    Dir(root)
}

fn shell(command: &str, cwd: &std::path::Path) -> String {
    let cancel = AtomicBool::new(false);
    let ctx = ToolCtx::new(cwd, &cancel);
    let out = crate::tools::shell::ShellTool.run(&json!({ "command": command }), &ctx);
    out.into_text()
}

#[test]
fn commands_that_walk_or_read_are_told_apart() {
    let walk = |c: &str| segments(c).iter().any(|w| walks(w));
    let read = |c: &str| segments(c).iter().any(|w| reads_file(w));
    assert!(walk("gci -Recurse src") && walk("Get-ChildItem . -r -Filter *.rs"));
    assert!(walk("cmd /c dir /s /b") && walk("$x = (gci -Depth 2).FullName"));
    assert!(!walk("gci src") && !walk("Get-ChildItem -ReadOnly"));
    assert!(read("Get-Content -Raw a.txt") && read("cmd /c type a.png"));
    assert!(read("gc a.txt | Select-String x") && !read("Get-Content -Wait log.txt"));
    assert!(!read("cargo test") && !read("type"));
}

#[test]
fn absolute_paths_lose_the_working_folder_once_noted() {
    let cwd = std::env::temp_dir().join("fh-hints-paths");
    let sep = std::path::MAIN_SEPARATOR;
    let lines: String = (1..=4)
        .map(|i| format!("{}{sep}src{sep}f{i}.rs\n", cwd.display()))
        .collect();
    let out = annotate(
        "(gci src).FullName",
        &cwd,
        ToolOutput::ok(lines + "[exit code: 0]"),
    );
    let text = out.into_text();
    assert!(text.starts_with(&format!("src{sep}f1.rs\n")), "{text}");
    let note = format!(
        "[paths under {}{sep} are shown relative to it]\n",
        cwd.display()
    );
    assert!(text.ends_with(&format!("{note}[exit code: 0]")), "{text}");
    let once = annotate(
        "echo",
        &cwd,
        ToolOutput::ok(format!("{}\n[exit code: 0]", cwd.display())),
    );
    assert_eq!(
        once.into_text(),
        format!("{}\n[exit code: 0]", cwd.display())
    );
}

/// Owner, 2026-11-15: `gci -Recurse` walked into ignored folders and
/// `Get-Content` read files with no word about `glob` and `read`.
#[cfg(windows)]
#[test]
fn a_recursive_listing_into_an_ignored_folder_points_at_glob() {
    let dir = repo("walk");
    let text = shell(
        "gci -Recurse | Select-Object -ExpandProperty FullName",
        &dir.0,
    );
    assert!(
        text.contains("folders .gitignore leaves out (target)"),
        "{text}"
    );
    assert!(text.contains("the glob tool"), "{text}");
    assert!(text.contains("\ntarget\\debug\\x.rlib\n"), "{text}");
    assert!(text.ends_with("[exit code: 0]"), "{text}");
    let clean = shell("gci -Recurse src | % FullName", &dir.0);
    assert!(!clean.contains("[hint"), "{clean}");
    let read = shell("Get-Content src/a.rs", &dir.0);
    assert!(
        read.starts_with("fn a() {}\n[hint: to read a file use the read tool"),
        "{read}"
    );
}
