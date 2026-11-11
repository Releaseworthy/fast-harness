//! System prompt templates (PLAN.md D31): precedence, placeholders, save and
//! reset per scope, unreadable files, the preview.

use std::path::{Path, PathBuf};

use fast_harness_core::agent::prompt::{self, DEFAULT_TEMPLATE};
use fast_harness_core::agent::prompt_store::{self as store, Scope, Source};

/// A fresh temp folder with `home` and `work` (a git root) inside; removed on drop.
struct Temp(PathBuf);

impl Temp {
    fn new(name: &str) -> Temp {
        let root = std::env::temp_dir().join(format!(
            "fh-prompt-{name}-{}-{}",
            std::process::id(),
            fast_harness_core::trace::now_ms()
        ));
        std::fs::create_dir_all(root.join("home")).unwrap();
        std::fs::create_dir_all(root.join("work").join(".git")).unwrap();
        Temp(root)
    }
    fn home(&self) -> PathBuf {
        self.0.join("home")
    }
    fn work(&self) -> PathBuf {
        self.0.join("work")
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn source(home: &Path, cwd: &Path) -> Source {
    store::template(home, cwd).source
}

#[test]
fn workspace_wins_over_global_over_default() {
    let temp = Temp::new("order");
    let (home, work) = (temp.home(), temp.work());
    assert_eq!(source(&home, &work), Source::Default);
    assert_eq!(store::template(&home, &work).text, DEFAULT_TEMPLATE);
    store::save(&home, &work, Scope::Global, "GLOBAL").unwrap();
    let global = store::path(&home, &work, Scope::Global);
    assert_eq!(source(&home, &work), Source::Global(global.clone()));
    store::save(&home, &work, Scope::Workspace, "WORKSPACE").unwrap();
    let file = store::path(&home, &work, Scope::Workspace);
    assert!(file.starts_with(home.join("prompts")));
    assert_eq!(source(&home, &work), Source::Workspace(file));
    assert_eq!(store::template(&home, &work).text, "WORKSPACE");
    // Another workspace still gets the global one.
    let other = temp.0.join("other");
    assert_eq!(source(&home, &other), Source::Global(global));
}

#[test]
fn save_and_reset_each_scope() {
    let temp = Temp::new("save");
    let (home, work) = (temp.home(), temp.work());
    for scope in [Scope::Global, Scope::Workspace] {
        assert_eq!(store::load(&home, &work, scope).unwrap(), None);
        store::save(&home, &work, scope, "one").unwrap();
        store::save(&home, &work, scope, "two").unwrap();
        assert_eq!(
            store::load(&home, &work, scope).unwrap().as_deref(),
            Some("two")
        );
        let tmp = store::path(&home, &work, scope).with_extension("md.tmp");
        assert!(!tmp.exists());
    }
    store::reset(&home, &work, Scope::Workspace).unwrap();
    assert_eq!(store::load(&home, &work, Scope::Workspace).unwrap(), None);
    assert_eq!(store::template(&home, &work).text, "two");
    store::reset(&home, &work, Scope::Global).unwrap();
    store::reset(&home, &work, Scope::Global).unwrap();
    assert_eq!(source(&home, &work), Source::Default);
}

#[test]
fn unreadable_file_falls_back_with_a_note() {
    let temp = Temp::new("unreadable");
    let (home, work) = (temp.home(), temp.work());
    store::save(&home, &work, Scope::Global, "GLOBAL {model}").unwrap();
    let file = store::path(&home, &work, Scope::Workspace);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, [0xff, 0xfe, 0x00, 0xc3]).unwrap();
    let rendered = prompt::render(&home, &work, "m-1");
    assert!(rendered.text.starts_with("GLOBAL m-1"));
    assert!(matches!(rendered.source, Source::Global(_)));
    assert_eq!(rendered.notes.len(), 1);
    assert!(
        rendered.notes[0].contains("unreadable"),
        "{:?}",
        rendered.notes
    );
    // A folder in place of the global file: the default, two notes.
    std::fs::remove_file(store::path(&home, &work, Scope::Global)).unwrap();
    std::fs::create_dir_all(store::path(&home, &work, Scope::Global)).unwrap();
    let rendered = prompt::render(&home, &work, "m-1");
    assert_eq!(rendered.source, Source::Default);
    assert_eq!(rendered.notes.len(), 2);
}

#[test]
fn placeholders_are_filled_once_and_others_stay() {
    let values = [
        "C:\\{date}".to_string(),
        "Windows".into(),
        "2026-11-12".into(),
        "opus".into(),
    ];
    let text = prompt::fill_with("{cwd} {os} {date} {model} {other} {cwd", &values);
    assert_eq!(text, "C:\\{date} Windows 2026-11-12 opus {other} {cwd");
    let filled = prompt::fill(DEFAULT_TEMPLATE, Path::new("/w"), "m");
    assert!(filled.contains("working in /w on "), "{filled}");
    assert!(!filled.contains('{'), "{filled}");
    assert_eq!(
        prompt::fill("{MODEL}{ model}", Path::new("/w"), "m"),
        "{MODEL}{ model}"
    );
}

#[test]
fn preview_appends_agents_md() {
    let temp = Temp::new("preview");
    let (home, work) = (temp.home(), temp.work());
    std::fs::write(work.join("AGENTS.md"), "RULE-ONE").unwrap();
    store::save(&home, &work, Scope::Workspace, "Model {model}.").unwrap();
    let text = prompt::render(&home, &work, "opus").text;
    assert!(text.starts_with("Model opus.\n"), "{text}");
    let agents = text.find("# Project instructions (AGENTS.md)").unwrap();
    assert!(text[agents..].contains("RULE-ONE"));
    let preview = prompt::preview("X", &work, "opus");
    assert!(preview.starts_with("X\n") && preview.contains("RULE-ONE"));
}
