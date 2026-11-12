//! The UI files loader of fast-harness-gui-live (PLAN.md D51).

use std::path::{Path, PathBuf};

use crate::{live, live_check, ui_contract, ui_files};

/// A fresh folder under the temporary directory.
fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fh-live-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A home whose `ui` folder holds the built-in files, `edit` applied to `file`.
fn home_with(tag: &str, file: &str, edit: impl Fn(&str) -> String) -> PathBuf {
    let home = temp(tag);
    ui_files::export(&home.join("ui")).unwrap();
    let path = home.join("ui").join(file);
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, edit(&text)).unwrap();
    home
}

fn check_home(home: &Path) -> Vec<String> {
    let root = home.join("staged");
    live::stage(&root, live::user_dir(Some(home)).as_deref(), true).unwrap();
    live_check::check(&root.join("ui").join("main.slint"))
}

#[test]
fn the_built_in_ui_passes_its_own_contract() {
    let home = home_with("builtin", "main.slint", str::to_string);
    assert_eq!(check_home(&home), Vec::<String>::new());
    assert!(ui_contract::parse(ui_contract::DOC).len() > 100);
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn a_file_that_does_not_compile_is_reported() {
    let home = home_with("broken", "pages.slint", |t| {
        format!("{t}\nthis is not slint")
    });
    let errors = check_home(&home);
    assert!(!errors.is_empty());
    assert!(errors[0].starts_with("pages.slint:"), "{errors:?}");
    let report = live_check::report("Reload kept the current UI", &home, &errors);
    assert!(report.contains("pages.slint:"), "{report}");
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn a_missing_contract_entry_is_reported() {
    let home = home_with("contract", "main.slint", |t| {
        t.replace("    callback reload-ui();\n", "")
            .replace("root.reload-ui();", "")
    });
    let errors = check_home(&home);
    assert_eq!(
        errors,
        vec!["MainWindow.reload-ui: missing callback".to_string()]
    );
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn a_changed_property_type_is_reported() {
    let home = home_with("type", "main.slint", |t| {
        t.replace(
            "in property <bool> ui-files;",
            "in property <string> ui-files;",
        )
        .replace("ui-files: root.ui-files;", "")
    });
    let errors = check_home(&home);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].starts_with("MainWindow.ui-files: is String"),
        "{errors:?}"
    );
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn a_broken_folder_falls_back_to_the_built_in_ui_with_the_errors() {
    let home = home_with("fallback", "main.slint", |t| {
        t.replace("MainWindow", "Window2")
    });
    let note = live::prepare(Some(&home)).unwrap();
    assert!(note.starts_with("The built-in UI is in use"), "{note}");
    assert!(note.contains("does not export MainWindow"), "{note}");
    let staged = std::fs::read_to_string(live::main_file()).unwrap();
    assert!(staged.contains("export component MainWindow"));
    // Reload reports the same folder and keeps the current UI.
    assert!(live::reload(Some(&home)).starts_with("Reload kept the current UI"));
    let ui = crate::MainWindow::new().unwrap();
    ui.set_error_text(note.into());
    assert!(ui.get_error_text().contains("MainWindow"));
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn no_user_folder_uses_the_built_in_ui() {
    let home = temp("none");
    assert!(live::user_dir(Some(&home)).is_none());
    assert!(live::reload(Some(&home)).starts_with("No ~/.fast-harness/ui"));
    std::fs::remove_dir_all(home).unwrap();
}
