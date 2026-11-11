//! The System prompt section's view model (PLAN.md D31).

use std::path::PathBuf;

use fast_harness_core::agent::prompt::DEFAULT_TEMPLATE;
use fast_harness_core::agent::prompt_store::{self as store, Scope};

use crate::prompt_settings::{view, PLACEHOLDERS};

/// A fresh temp folder with `home` and `work`; removed on drop.
struct Temp(PathBuf);

impl Temp {
    fn new(name: &str) -> Temp {
        let root = std::env::temp_dir().join(format!(
            "fh-gui-prompt-{name}-{}-{}",
            std::process::id(),
            fast_harness_core::trace::now_ms()
        ));
        std::fs::create_dir_all(root.join("home")).unwrap();
        std::fs::create_dir_all(root.join("work")).unwrap();
        Temp(root)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn global_starts_from_the_default_then_the_saved_text() {
    let temp = Temp::new("global");
    let (home, work) = (temp.0.join("home"), temp.0.join("work"));
    let start = view(&home, &work, Scope::Global);
    assert_eq!(start.text, DEFAULT_TEMPLATE);
    assert!(!start.override_on);
    assert!(
        start.source.contains("built-in default"),
        "{}",
        start.source
    );
    assert!(
        start.source.contains("system-prompt.md"),
        "{}",
        start.source
    );
    store::save(&home, &work, Scope::Global, "G {model}").unwrap();
    let saved = view(&home, &work, Scope::Global);
    assert_eq!(saved.text, "G {model}");
    assert!(saved.source.contains("global prompt"), "{}", saved.source);
    assert_eq!(saved.status, "");
}

#[test]
fn workspace_shows_the_global_text_until_it_overrides() {
    let temp = Temp::new("workspace");
    let (home, work) = (temp.0.join("home"), temp.0.join("work"));
    store::save(&home, &work, Scope::Global, "GLOBAL").unwrap();
    let off = view(&home, &work, Scope::Workspace);
    assert_eq!(off.text, "GLOBAL");
    assert!(!off.override_on);
    assert!(off.source.contains("Override writes"), "{}", off.source);
    store::save(&home, &work, Scope::Workspace, "OWN").unwrap();
    let on = view(&home, &work, Scope::Workspace);
    assert_eq!(on.text, "OWN");
    assert!(on.override_on);
    assert!(on.source.contains("workspace prompt"), "{}", on.source);
}

#[test]
fn an_unreadable_file_is_reported_in_words() {
    let temp = Temp::new("unreadable");
    let (home, work) = (temp.0.join("home"), temp.0.join("work"));
    std::fs::write(store::path(&home, &work, Scope::Global), [0xff, 0xfe]).unwrap();
    let shown = view(&home, &work, Scope::Global);
    assert_eq!(shown.text, DEFAULT_TEMPLATE);
    assert!(shown.status.starts_with("Cannot read"), "{}", shown.status);
    for token in ["{cwd}", "{os}", "{date}", "{model}"] {
        assert!(PLACEHOLDERS.contains(token));
    }
}
