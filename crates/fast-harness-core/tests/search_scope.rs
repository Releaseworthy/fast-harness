//! What grep and glob say about the matches they show and the entries they
//! left out, on the owner's test repository of 2026-11-15 (D91).

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use fast_harness_core::tools::{builtin, ToolCtx};
use serde_json::{json, Value};

struct Repo(PathBuf);

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The owner's repo: `node_modules/` ignored, a hidden `.secret/`, 150
/// small files under `src/m*/s*`, a 2000-line `app.log` of 95-byte lines.
fn repo(tag: &str) -> Repo {
    let root = std::env::temp_dir().join(format!("fh-scope-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let write = |rel: &str, text: &str| {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    std::fs::create_dir_all(root.join(".git/objects")).unwrap();
    write(".git/HEAD", "ref: refs/heads/main\n");
    write(".gitignore", "node_modules/\n");
    for i in 1..=150 {
        let rel = format!("src/m{}/s{}/f{i}.txt", i % 5, i % 3);
        write(&rel, &format!("line one {i}\nvalue = {i}\n"));
    }
    for i in 1..=100 {
        write(&format!("node_modules/pkg{}/i{i}.js", i % 4), "mod\n");
    }
    write(".secret/notes.txt", "TODO secret\n");
    let mut log = format!("ERROR at top {}\n", "x".repeat(77));
    for i in 2..=2000 {
        log.push_str(&format!("log line {i:05} {}\n", "y".repeat(74)));
    }
    write("app.log", &log);
    Repo(root)
}

fn run(repo: &Repo, name: &str, args: Value, budget: usize) -> String {
    let cancel = AtomicBool::new(false);
    let ctx = ToolCtx::new(&repo.0, &cancel).with_budget(budget);
    let out = builtin().run(name, &args.to_string(), &ctx);
    out.text().into_owned()
}

/// The number written right after `word` in `text`.
fn number_after(text: &str, word: &str) -> Option<usize> {
    let rest = &text[text.find(word)? + word.len()..];
    rest.split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

/// Owner: "capped to 85" while 50 matches were shown (budget 5461 bytes).
#[test]
fn the_stated_cap_is_the_number_shown() {
    let repo = repo("cap");
    for args in [
        json!({"pattern": ".", "max_results": 10000}),
        json!({"pattern": "."}),
    ] {
        let text = run(&repo, "grep", args, 5461);
        let shown = text.lines().filter(|l| l.starts_with("  ")).count();
        assert_eq!(number_after(&text, "showing "), Some(shown), "{text}");
        assert_eq!(
            number_after(&text, &format!("showing {shown} of ")),
            Some(2300)
        );
        if let Some(cap) = number_after(&text, "capped to ") {
            assert_eq!(cap, shown, "{text}");
        }
    }
}

/// Owner: glob `src/m1/**` noted node_modules and .secret as skipped,
/// though both lie outside the folder the pattern can match in.
#[test]
fn only_entries_the_pattern_could_match_are_counted() {
    let repo = repo("glob");
    let text = run(&repo, "glob", json!({"pattern": "src/m1/**"}), 8192);
    assert!(
        text.contains("src/m1/s1/f1.txt") && !text.contains("skipped"),
        "{text}"
    );
    let grep = json!({"pattern": "value", "glob": "src/m1/**"});
    let text = run(&repo, "grep", grep, 8192);
    assert!(!text.contains("skipped"), "{text}");
    let text = run(&repo, "glob", json!({"pattern": "node_modules/**"}), 8192);
    assert_eq!(
        text,
        "No files found.\n[1 folder (100 files inside) skipped by .gitignore \
         (include_ignored: true)]\n"
    );
}

/// Owner: grep said 2 hidden entries and glob 1 in the same repo (grep
/// counted `.gitignore`, which glob `node_modules/**` cannot match). A
/// hidden entry counts only when the pattern could match it or a file in
/// it, so grep and glob agree on one pattern; `.git` never counts.
#[test]
fn grep_and_glob_count_the_same_hidden_entries() {
    let repo = repo("hidden");
    let hidden = |text: &str| number_after(text, "; ").filter(|_| text.contains("hidden"));
    let cases = [("*", Some(2)), ("**/*.txt", Some(1)), ("**/*.zz", None)];
    for (pattern, count) in cases {
        let glob = run(&repo, "glob", json!({"pattern": pattern}), 8192);
        let grep = json!({"pattern": "TODO|value", "glob": pattern});
        let grep = run(&repo, "grep", grep, 8192);
        for text in [&glob, &grep] {
            let note = text.lines().last().unwrap_or_default();
            assert_eq!(hidden(note), count, "{pattern}: {text}");
        }
    }
    let grep = run(&repo, "grep", json!({"pattern": "TODO"}), 8192);
    assert!(grep.contains("; 2 hidden entries skipped"), "{grep}");
}
