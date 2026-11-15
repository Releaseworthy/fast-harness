//! grep and glob: a repeated identical result answers `[unchanged since
//! call X]`, an empty result counts what was skipped; find_tools names
//! loaded tools instead of loading them again.

use std::sync::atomic::AtomicBool;

use fast_harness_core::tools::budget::ToolSession;
use fast_harness_core::tools::{builtin, ToolCtx};
use serde_json::json;

#[test]
fn repeats_skips_and_loaded_tools() {
    let dir = std::env::temp_dir().join(format!("fh-search-repeat-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("build")).unwrap();
    std::fs::write(dir.join(".gitignore"), "build/\nlog.txt\n").unwrap();
    std::fs::write(dir.join("a.rs"), "fn needle() {}\n").unwrap();
    std::fs::write(dir.join("log.txt"), "needle\n").unwrap();
    std::fs::write(dir.join("build").join("b.rs"), "needle\n").unwrap();
    std::fs::write(dir.join(".hidden.rs"), "needle\n").unwrap();
    std::fs::write(dir.join("blob.bin"), b"needle\0\0\0").unwrap();
    let (cancel, session, tools) = (AtomicBool::new(false), ToolSession::new(None), builtin());
    let run = |name: &str, id: &str, args: serde_json::Value| {
        let ctx = ToolCtx::new(&dir, &cancel)
            .with_call_id(id)
            .with_session(&session);
        tools.run(name, &args.to_string(), &ctx).text().into_owned()
    };
    let first = run("grep", "c1", json!({"pattern": "needle"}));
    assert!(first.contains("a.rs"), "{first}");
    assert_eq!(
        run("grep", "c2", json!({"pattern": "needle"})),
        "[unchanged since call c1]"
    );
    assert!(run("glob", "c3", json!({"pattern": "*.rs"})).contains("a.rs"));
    assert_eq!(
        run("glob", "c4", json!({"pattern": "*.rs"})),
        "[unchanged since call c3]"
    );
    std::fs::write(dir.join("c.rs"), "x").unwrap();
    assert!(run("glob", "c5", json!({"pattern": "*.rs"})).contains("c.rs"));
    let none = run("grep", "c6", json!({"pattern": "needle", "glob": "*.txt"}));
    assert!(
        none.starts_with(
            "No matches.\n[1 file and 1 folder skipped by .gitignore (include_ignored: true)"
        ),
        "{none}"
    );
    assert!(first.contains("1 binary file not searched"), "{first}");
    let all = run(
        "grep",
        "c7",
        json!({"pattern": "needle", "include_ignored": true, "include_hidden": true}),
    );
    assert!(
        all.contains("log.txt") && all.contains("b.rs") && all.contains(".hidden.rs"),
        "{all}"
    );
    let loaded = vec!["grep".to_string()];
    let ctx = ToolCtx::new(&dir, &cancel).with_loaded(&loaded);
    let found = tools
        .run("find_tools", r#"{"names":["grep","glob"]}"#, &ctx)
        .text()
        .into_owned();
    assert_eq!(
        found,
        "Loaded: glob\nYou can call them now.\ngrep: already loaded"
    );
    let again = tools
        .run("find_tools", r#"{"names":["grep"]}"#, &ctx)
        .text()
        .into_owned();
    assert_eq!(again, "grep: already loaded");
    let _ = std::fs::remove_dir_all(&dir);
}
