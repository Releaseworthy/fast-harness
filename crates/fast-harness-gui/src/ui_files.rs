//! The built-in `ui/*.slint` sources and `--export-ui <dir>` (PLAN.md D51):
//! a user copies them, edits them in a text editor and runs
//! `fast-harness-gui-live`, which loads `~/.fast-harness/ui`.

use std::path::Path;

/// Every file of `ui/`, by name (a test compares the list with the folder).
pub const FILES: [(&str, &str); 40] = [
    ("accounts.slint", include_str!("../ui/accounts.slint")),
    ("appearance.slint", include_str!("../ui/appearance.slint")),
    ("approval.slint", include_str!("../ui/approval.slint")),
    ("ask.slint", include_str!("../ui/ask.slint")),
    ("attach.slint", include_str!("../ui/attach.slint")),
    ("blocks.slint", include_str!("../ui/blocks.slint")),
    ("codeview.slint", include_str!("../ui/codeview.slint")),
    ("composer.slint", include_str!("../ui/composer.slint")),
    ("config_settings.slint", include_str!("../ui/config_settings.slint")),
    ("dialog.slint", include_str!("../ui/dialog.slint")),
    ("controls.slint", include_str!("../ui/controls.slint")),
    ("icons.slint", include_str!("../ui/icons.slint")),
    ("main.slint", include_str!("../ui/main.slint")),
    ("pages.slint", include_str!("../ui/pages.slint")),
    ("panel_view.slint", include_str!("../ui/panel_view.slint")),
    ("panel.slint", include_str!("../ui/panel.slint")),
    ("parts.slint", include_str!("../ui/parts.slint")),
    ("pill.slint", include_str!("../ui/pill.slint")),
    ("popup.slint", include_str!("../ui/popup.slint")),
    ("prompt_settings.slint", include_str!("../ui/prompt_settings.slint")),
    ("providers.slint", include_str!("../ui/providers.slint")),
    ("queue.slint", include_str!("../ui/queue.slint")),
    ("job_row.slint", include_str!("../ui/job_row.slint")),
    ("jobs.slint", include_str!("../ui/jobs.slint")),
    ("limits.slint", include_str!("../ui/limits.slint")),
    ("scrollbar.slint", include_str!("../ui/scrollbar.slint")),
    ("sidebar.slint", include_str!("../ui/sidebar.slint")),
    ("stepline.slint", include_str!("../ui/stepline.slint")),
    ("steps.slint", include_str!("../ui/steps.slint")),
    ("subagent_row.slint", include_str!("../ui/subagent_row.slint")),
    ("subagents.slint", include_str!("../ui/subagents.slint")),
    ("textarea.slint", include_str!("../ui/textarea.slint")),
    ("titlebar.slint", include_str!("../ui/titlebar.slint")),
    ("toolcard.slint", include_str!("../ui/toolcard.slint")),
    ("trace.slint", include_str!("../ui/trace.slint")),
    ("transcript.slint", include_str!("../ui/transcript.slint")),
    ("turnnav.slint", include_str!("../ui/turnnav.slint")),
    ("types.slint", include_str!("../ui/types.slint")),
    ("undo.slint", include_str!("../ui/undo.slint")),
    ("widgets.slint", include_str!("../ui/widgets.slint")),
];

/// Writes every built-in `.slint` file into `dir` (created when missing);
/// returns how many it wrote.
pub fn export(dir: &Path) -> std::io::Result<usize> {
    std::fs::create_dir_all(dir)?;
    for (name, text) in FILES {
        std::fs::write(dir.join(name), text)?;
    }
    Ok(FILES.len())
}

/// Handles `--export-ui <dir>`: `Some(exit code)` when the arguments asked
/// for it, `None` to start the window.
pub fn from_args() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next()? != "--export-ui" {
        return None;
    }
    let Some(dir) = args.next() else {
        eprintln!("usage: fast-harness-gui --export-ui <dir>");
        return Some(2);
    };
    match export(Path::new(&dir)) {
        Ok(n) => {
            println!("wrote {n} files to {}", Path::new(&dir).display());
            Some(0)
        }
        Err(err) => {
            eprintln!("cannot write {}: {err}", Path::new(&dir).display());
            Some(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_every_file_of_the_ui_folder() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
        let mut on_disk: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|name| name.ends_with(".slint"))
            .collect();
        on_disk.sort();
        let mut listed: Vec<&str> = FILES.iter().map(|(name, _)| *name).collect();
        listed.sort();
        assert_eq!(on_disk, listed);
    }

    #[test]
    fn export_writes_every_file_unchanged() {
        let dir = std::env::temp_dir().join(format!("fh-export-ui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(export(&dir.join("ui")).unwrap(), FILES.len());
        for (name, text) in FILES {
            assert_eq!(std::fs::read_to_string(dir.join("ui").join(name)).unwrap(), text);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
