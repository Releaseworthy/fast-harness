//! The sidebar's workspaces (PLAN.md D28): folders from `workspaces.json`, each with its
//! sessions nested under it when expanded.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::thread;

use fast_harness_core::platform;
use fast_harness_core::session::{self, slug};
use fast_harness_core::trace::now_ms;
use fast_harness_core::workspaces::Workspaces;
use slint::{ModelRc, VecModel};

use crate::app::{show_error, with_app, App};
use crate::rows::session_row;
use crate::{MainWindow, SideRow};

/// The list, the expanded folders (by session slug) and whether saving is allowed
/// (not after a corrupt file, so it is never overwritten).
pub struct Side {
    pub list: Workspaces,
    pub expanded: HashSet<String>,
    pub can_save: bool,
}

/// Loads the list and picks the start folder: `arg` (added and made current), else the
/// most recently used workspace, else the process's current directory (added).
/// Returns the side state, the folder and a load error to show.
pub fn start(home: &Path, arg: Option<PathBuf>) -> (Side, PathBuf, Option<String>) {
    let (list, error) = match Workspaces::load(home) {
        Ok(list) => (list, None),
        Err(err) => (Workspaces::empty(home), Some(err.to_string())),
    };
    let mut side = Side {
        list,
        expanded: HashSet::new(),
        can_save: error.is_none(),
    };
    let recent = side.list.most_recent().map(|w| w.path.clone());
    let cwd = arg
        .or(recent)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    side.list.add(&cwd, now_ms());
    side.list.touch(&cwd, now_ms());
    side.expanded.insert(slug(&cwd));
    let error = error.or_else(|| save_error(&side));
    (side, cwd, error)
}

fn save_error(side: &Side) -> Option<String> {
    if !side.can_save {
        return None;
    }
    let err = side.list.save().err()?;
    Some(format!("Cannot save the workspaces: {err}"))
}

/// Rebuilds the sidebar rows: every workspace, and the sessions of each expanded one.
pub fn refresh(app: &App, ui: &MainWindow) {
    let now = now_ms();
    let current = slug(&app.cwd);
    let mut rows = Vec::new();
    for ws in app.side.list.list() {
        let key = slug(&ws.path);
        let expanded = app.side.expanded.contains(&key);
        rows.push(workspace_row(&ws.path, key == current, expanded));
        if !expanded {
            continue;
        }
        match session::list(&app.rt.home, &ws.path) {
            Ok(list) => rows.extend(list.iter().map(|s| {
                let open = if key == current {
                    app.runs.id.as_deref()
                } else {
                    None
                };
                let row = session_row(s, open, now);
                let run = app.runs.mark(&s.id, s.unfinished);
                SideRow {
                    kind: "session".into(),
                    id: row.id,
                    workspace: ws.path.to_string_lossy().as_ref().into(),
                    title: row.title,
                    detail: row.time,
                    current: row.current,
                    expanded: false,
                    run: run.into(),
                }
            })),
            Err(err) => show_error(ui, format!("Cannot list the sessions: {err}")),
        }
    }
    ui.set_side_rows(ModelRc::new(VecModel::from(rows)));
}

pub fn workspace_row(path: &Path, current: bool, expanded: bool) -> SideRow {
    let full = path.to_string_lossy().to_string();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| full.clone());
    SideRow {
        kind: "workspace".into(),
        id: full.as_str().into(),
        workspace: full.as_str().into(),
        title: name.into(),
        detail: full.into(),
        current,
        expanded,
        run: "".into(),
    }
}

/// Makes `path` current (a new session there) and expands it; on the current one,
/// toggles it.
pub fn select(app: &mut App, ui: &MainWindow, path: &str) {
    let path = PathBuf::from(path);
    let key = slug(&path);
    if key == slug(&app.cwd) {
        if !app.side.expanded.remove(&key) {
            app.side.expanded.insert(key);
        }
        return refresh(app, ui);
    }
    if switch(app, ui, &path) {
        app.new_session(ui);
    }
}

/// Moves to workspace `path`: cwd, lastUsedAt, saved, expanded. A running
/// turn goes on parked (D39).
fn switch(app: &mut App, ui: &MainWindow, path: &Path) -> bool {
    app.cwd = path.to_path_buf();
    app.side.list.touch(path, now_ms());
    app.side.expanded.insert(slug(path));
    if let Some(err) = save_error(&app.side) {
        show_error(ui, err);
    }
    true
}

/// Opens session `id` of workspace `ws`, switching to that workspace first.
pub fn open_session(app: &mut App, ui: &MainWindow, ws: &str, id: &str) {
    let path = PathBuf::from(ws);
    if slug(&path) != slug(&app.cwd) && !switch(app, ui, &path) {
        return;
    }
    app.open_session(ui, id);
}

/// Removes `path` from the list only; its sessions stay on disk.
pub fn remove(app: &mut App, ui: &MainWindow, path: &str) {
    let path = PathBuf::from(path);
    app.side.list.remove(&path);
    app.side.expanded.remove(&slug(&path));
    if let Some(err) = save_error(&app.side) {
        show_error(ui, err);
    }
    refresh(app, ui);
}

/// Shows the "Select folder" dialog on its own thread, owned by the main window
/// (looked up here, on the UI thread); the pick is added and selected on the UI thread.
pub fn add(_app: &mut App, _ui: &MainWindow) {
    let owner = platform::thread_window().unwrap_or(0);
    thread::spawn(move || {
        let picked = platform::pick_folder(owner, "Add a workspace folder");
        let _ = slint::invoke_from_event_loop(move || {
            with_app(|app, ui| match picked {
                Ok(Some(path)) => added(app, ui, &path),
                Ok(None) => {}
                Err(err) => show_error(ui, format!("Cannot open the folder dialog: {err}")),
            });
        });
    });
}

fn added(app: &mut App, ui: &MainWindow, path: &Path) {
    app.side.list.add(path, now_ms());
    if slug(path) == slug(&app.cwd) {
        app.side.expanded.insert(slug(path));
        if let Some(err) = save_error(&app.side) {
            show_error(ui, err);
        }
        return refresh(app, ui);
    }
    if switch(app, ui, path) {
        app.new_session(ui);
    } else {
        refresh(app, ui);
    }
}
