//! fast-harness-gui-live (cargo feature `ui-files`, PLAN.md D51): the window
//! is built at run time from `~/.fast-harness/ui` by Slint's live preview,
//! which keeps the typed `MainWindow` API the rest of the crate uses.
//!
//! The binary stages the user folder (with its own fonts and images) into a
//! folder of its own and points the live preview there (`main_file`); the
//! preview reloads when that folder changes. Only checked files reach it: at
//! start and on Reload the user folder is first staged into a scratch folder
//! and checked (`live_check`); a failure is reported in the banner and the
//! built-in UI (at start) or the current UI (on Reload) stays.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use slint::ComponentHandle;

use crate::{live_check, ui_files, MainWindow};

/// The staged `ui/main.slint` the live preview loads.
static MAIN: OnceLock<PathBuf> = OnceLock::new();

/// The files `ui/*.slint` refer to with `../fonts` and `../assets`.
const ASSETS: [(&str, &[u8]); 4] = [
    (
        "fonts/FastHarnessIcons.ttf",
        include_bytes!("../fonts/FastHarnessIcons.ttf"),
    ),
    (
        "assets/fast-harness-256.png",
        include_bytes!("../assets/fast-harness-256.png"),
    ),
    (
        "assets/fast-harness-64.png",
        include_bytes!("../assets/fast-harness-64.png"),
    ),
    (
        "assets/fast-harness-grey-64.png",
        include_bytes!("../assets/fast-harness-grey-64.png"),
    ),
];

/// The file the generated `MainWindow::new` loads (build/live.rs). Without
/// `prepare` (tests) it is the built-in UI, staged once per process.
pub fn main_file() -> PathBuf {
    MAIN.get_or_init(|| {
        let root = stage_root("builtin");
        let _ = stage(&root, None, true);
        root.join("ui").join("main.slint")
    })
    .clone()
}

/// The folder of the staged UIs of every running fast-harness-gui-live.
fn stage_parent() -> PathBuf {
    std::env::temp_dir().join("fast-harness-ui")
}

/// A folder of this process under the temporary directory.
pub fn stage_root(tag: &str) -> PathBuf {
    stage_parent().join(format!("{}-{tag}", std::process::id()))
}

/// Removes the folders of processes that no longer run (a killed process
/// cannot clean up after itself).
fn sweep() {
    let Ok(entries) = std::fs::read_dir(stage_parent()) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let pid = name.split('-').next().and_then(|p| p.parse::<u32>().ok());
        let gone = pid.is_some_and(|pid| fast_harness_core::platform::process_start(pid).is_none());
        if gone {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// `~/.fast-harness/ui` when it has a `main.slint`.
pub fn user_dir(home: Option<&Path>) -> Option<PathBuf> {
    let dir = home?.join("ui");
    dir.join("main.slint").is_file().then_some(dir)
}

/// Writes the assets and the UI files (the user folder, or the built-in
/// files) under `root`. `clean` first removes `root/ui`; the folder the
/// preview watches is only overwritten, `main.slint` last.
pub fn stage(root: &Path, user: Option<&Path>, clean: bool) -> io::Result<()> {
    for (name, bytes) in ASSETS {
        let path = root.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, bytes)?;
    }
    let ui = root.join("ui");
    if clean && ui.exists() {
        std::fs::remove_dir_all(&ui)?;
    }
    match user {
        Some(dir) => copy_dir(dir, &ui),
        None => ui_files::export(&ui).map(|_| ()),
    }
}

fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::create_dir_all(to)?;
    let mut main = None;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if entry.file_name() == "main.slint" {
            main = Some((entry.path(), target));
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    if let Some((from, to)) = main {
        std::fs::copy(from, to)?;
    }
    Ok(())
}

/// Stages the UI before the window exists; returns the banner note.
pub fn prepare(home: Option<&Path>) -> Option<String> {
    sweep();
    let root = stage_root("live");
    let main = root.join("ui").join("main.slint");
    let note = match user_dir(home) {
        None => {
            let staged = stage(&root, None, true);
            Some(staged.err().map_or_else(no_folder_note, |e| e.to_string()))
        }
        Some(user) => {
            let errors = match stage(&root, Some(&user), true) {
                Ok(()) => live_check::check(&main),
                Err(err) => vec![format!("cannot read {}: {err}", user.display())],
            };
            (!errors.is_empty()).then(|| {
                let _ = stage(&root, None, true);
                live_check::report("The built-in UI is in use", &user, &errors)
            })
        }
    };
    let _ = MAIN.set(main);
    note
}

/// The banner text when there is no user folder.
fn no_folder_note() -> String {
    "No ~/.fast-harness/ui/main.slint: the built-in UI is in use. \
     Run fast-harness-gui --export-ui %USERPROFILE%\\.fast-harness\\ui, edit, then Reload (F5)."
        .into()
}

/// Checks the user folder and, when it passes, stages it where the preview
/// reloads it. Returns the banner text.
pub fn reload(home: Option<&Path>) -> String {
    let Some(user) = user_dir(home) else {
        return no_folder_note();
    };
    let scratch = stage_root("check");
    let errors = match stage(&scratch, Some(&user), true) {
        Ok(()) => live_check::check(&scratch.join("ui").join("main.slint")),
        Err(err) => vec![format!("cannot read {}: {err}", user.display())],
    };
    let _ = std::fs::remove_dir_all(&scratch);
    if !errors.is_empty() {
        return live_check::report("Reload kept the current UI", &user, &errors);
    }
    let live = main_file();
    let Some(root) = live.parent().and_then(Path::parent) else {
        return "The staged UI folder is gone.".into();
    };
    match stage(root, Some(&user), false) {
        Ok(()) => format!("UI files reloaded from {}.", user.display()),
        Err(err) => format!("Reload failed: {err}"),
    }
}

/// Shows the UI files section and binds Reload.
pub fn connect(ui: &MainWindow) {
    ui.set_ui_files(true);
    let weak = ui.as_weak();
    ui.on_reload_ui(move || {
        let home = fast_harness_core::platform::app_home();
        let note = reload(home.as_deref());
        if let Some(ui) = weak.upgrade() {
            ui.set_error_text(note.into());
        }
    });
}

/// Removes the staged folders of this process.
pub fn cleanup() {
    for tag in ["live", "builtin", "check"] {
        let _ = std::fs::remove_dir_all(stage_root(tag));
    }
}
