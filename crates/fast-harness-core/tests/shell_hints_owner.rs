//! The shell hints on the owner's own commands of 2026-11-15 (D91), run
//! through real pwsh from a working folder other than the one listed.
#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use fast_harness_core::tools::{builtin, ToolCtx};
use serde_json::json;

struct Dir(PathBuf);

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn dir(tag: &str) -> Dir {
    let root = std::env::temp_dir().join(format!("fh-owner-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    Dir(root)
}

/// A repository with `node_modules/` ignored and four packages in it.
fn repo(tag: &str) -> Dir {
    let repo = dir(tag);
    std::fs::create_dir_all(repo.0.join(".git")).unwrap();
    std::fs::write(repo.0.join(".gitignore"), "node_modules/\n").unwrap();
    for i in 0..12 {
        let pkg = repo.0.join(format!("node_modules/pkg{}", i % 4));
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join(format!("i{i}.js")), "mod\n").unwrap();
    }
    std::fs::create_dir_all(repo.0.join("src")).unwrap();
    std::fs::write(repo.0.join("src/a.txt"), "a\n").unwrap();
    repo
}

fn shell(command: &str, cwd: &Path, loaded: &[String]) -> String {
    let cancel = AtomicBool::new(false);
    let ctx = ToolCtx::new(cwd, &cancel).with_loaded(loaded);
    let args = json!({ "command": command }).to_string();
    builtin().run("shell", &args, &ctx).text().into_owned()
}

/// Owner: `cd <repo>; Get-ChildItem -Recurse node_modules` (default
/// table format), `% FullName` and `Select FullName` from another working
/// folder gave no hint and repeated the absolute path on every line.
#[test]
fn a_listing_of_another_folder_still_points_at_glob() {
    let (repo, cwd) = (repo("walk"), dir("walk-cwd"));
    let root = repo.0.display().to_string();
    let hint = format!("folders .gitignore leaves out ({root}\\node_modules)");
    for command in [
        format!("cd '{root}'; Get-ChildItem -Recurse node_modules"),
        format!("Get-ChildItem -Recurse -File -Path '{root}' | % FullName"),
        format!("gci -Recurse '{root}\\node_modules' | Select FullName"),
    ] {
        let text = shell(&command, &cwd.0, &[]);
        assert!(text.contains(&hint), "{command}\n{text}");
        assert!(text.contains("the glob tool lists"), "{text}");
        let repeats = text.lines().filter(|l| l.contains(&root)).count();
        assert!(repeats <= 3, "{command}\n{text}");
    }
    let text = shell(
        &format!("gci -Recurse -File '{root}' | % FullName"),
        &cwd.0,
        &[],
    );
    let note = format!("[paths under {root}\\ are shown relative to it]");
    assert!(
        text.contains("\nnode_modules\\pkg1\\i1.js\n") && text.contains(&note),
        "{text}"
    );
}

/// Owner: `Get-Content … | ConvertFrom-Json` got the hint to use read,
/// which also said "find_tools loads it" though read was loaded.
#[test]
fn only_a_plain_read_gets_the_read_hint() {
    let cwd = dir("read");
    std::fs::write(cwd.0.join("a.json"), "{\"a\": 1}\n").unwrap();
    for command in [
        "Get-Content a.json | ConvertFrom-Json",
        "Get-Content -Raw a.json | ConvertFrom-Json",
        "Get-Content a.json | % { $j = $_ | ConvertFrom-Json; $j.a }",
        "gc a.json | Select-String a",
        "(Get-Content a.json).Count",
        "$x = Get-Content a.json; $x.Length",
    ] {
        let text = shell(command, &cwd.0, &[]);
        assert!(!text.contains("[hint"), "{command}\n{text}");
    }
    let loaded = ["read".to_string(), "shell".to_string()];
    let text = shell(
        "Get-Content a.json | Select-Object -First 1",
        &cwd.0,
        &loaded,
    );
    let hint = "[hint: to read a file use the read tool (line numbers, ranges, images) \
                instead of the shell]";
    assert!(text.contains(hint), "{text}");
    let text = shell("cat a.json", &cwd.0, &["shell".to_string()]);
    assert!(text.contains("images; find_tools loads it)"), "{text}");
}

/// Owner: a 102400-byte file printed with `Get-Content -Raw` was reported
/// as 100786 bytes: pwsh decoded and re-encoded it, so the stream size is
/// not the file's; a single file named by the command gives its real size.
#[test]
fn a_binary_file_read_reports_its_real_size() {
    let (data, cwd) = (dir("bin"), dir("bin-cwd"));
    let mut seed = 1u64;
    let bytes: Vec<u8> = (0..102_400)
        .map(|_| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as u8
        })
        .collect();
    std::fs::write(data.0.join("rand.bin"), bytes).unwrap();
    let root = data.0.display();
    let text = shell(
        &format!("cd '{root}'; Get-Content -Raw rand.bin"),
        &cwd.0,
        &[],
    );
    assert!(
        text.starts_with("[binary output of rand.bin, 102400 bytes]\n"),
        "{text}"
    );
    let piped = format!("Get-Content -Raw '{root}\\rand.bin' | Out-String");
    let text = shell(&piped, &cwd.0, &[]);
    assert!(text.starts_with("[binary output, about "), "{text}");
}
