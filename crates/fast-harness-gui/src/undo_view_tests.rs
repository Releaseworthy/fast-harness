//! The undo row and `/undo` in the window (PLAN.md D69 "Views"): hidden
//! without a turn to undo, refused during a turn, never sent to the model,
//! a conflict refused until the explicit "Undo anyway" (force).

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::time::Duration;

use fast_harness_core::agent::Conversation;
use fast_harness_core::app::Runtime;
use fast_harness_core::service::Session;
use fast_harness_core::tools::{self, ToolCtx, UndoInfo};
use fast_harness_core::view::{opened, Item, NoteKind};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::ComponentHandle;

use super::{bar, finished, typed, Conflict, RUNNING};
use crate::app::{install, with_app, App};
use crate::{open_load, turn, MainWindow};

struct TestPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

#[test]
fn the_row_model_says_the_turn_the_files_and_a_conflict_in_words() {
    assert_eq!(bar(None, None).turn, 0, "no turn: the row is hidden");
    let info = UndoInfo {
        turn: 3,
        files: 2,
        not_undoable: None,
    };
    let row = bar(Some(&info), None);
    assert_eq!((row.turn, row.summary.as_str()), (3, "Turn 3 changed 2 files with write or edit"));
    assert!(row.blocked.is_empty() && row.conflict.is_empty());
    let conflict = |turn| Conflict {
        session: "s".into(),
        turn,
        files: "a.txt".into(),
    };
    let row = bar(Some(&info), Some(&conflict(3)));
    assert_eq!(row.conflict.as_str(), "Conflict: changed since turn 3: a.txt");
    assert!(bar(Some(&info), Some(&conflict(2))).conflict.is_empty(), "another turn's");
    let capped = UndoInfo {
        not_undoable: Some("over the cap".into()),
        ..info
    };
    assert_eq!(
        bar(Some(&capped), None).blocked.as_str(),
        "Turn 3 (2 files) cannot be undone: over the cap"
    );
}

/// `write` of `content` to `u.txt` as turn 1 of the session, tracked.
fn tracked_write(session: &Session, cwd: &Path) {
    session
        .with_conv(|conv| {
            conv.tool_session.undo().begin_turn(1);
            let cancel = AtomicBool::new(false);
            let mut ctx = ToolCtx::new(cwd, &cancel);
            ctx.session = Some(&conv.tool_session);
            let args = r#"{"path": "u.txt", "content": "after"}"#;
            tools::builtin().run("write", args, &ctx);
        })
        .unwrap();
}

fn setup(dir: &Path) -> (MainWindow, PathBuf) {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let _ = slint::platform::set_platform(Box::new(TestPlatform(window)));
    let ui = crate::chat_list::new_window().expect("window");
    let (home, cwd) = (dir.join("home"), dir.join("work"));
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::write(cwd.join("u.txt"), "before").unwrap();
    let id = Conversation::create(&home, &cwd, None).unwrap().id;
    let rt = Runtime::load_at(home.clone()).unwrap();
    install(App::new(&ui, rt, Some(cwd.clone())));
    super::connect(&ui);
    with_app(|app, ui| app.open_session(ui, &id));
    let result = opened::load(&home, &cwd, &id);
    with_app(|app, ui| open_load::arrived(app, ui, 1, &cwd, result));
    (ui, cwd)
}

fn notes(app: &App) -> Vec<(NoteKind, String)> {
    (app.runs.shown.view.items.iter())
        .filter_map(|item| match item {
            Item::Note { kind, text } => Some((*kind, text.clone())),
            Item::User { .. } => panic!("a user message: /undo reached the turn"),
            _ => None,
        })
        .collect()
}

#[test]
fn undo_runs_off_the_model_refuses_during_a_turn_and_forces_only_on_request() {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    let dir = std::env::temp_dir().join(format!("fh-gui-undo-{}-{nanos}", std::process::id()));
    let (ui, cwd) = setup(&dir);
    let (tx, rx) = mpsc::channel();
    let session = with_app(|app, ui| {
        app.undo.results = Some(tx);
        app.refresh_run(ui);
        app.runs.shown.session.clone().unwrap()
    })
    .unwrap();
    assert_eq!(ui.get_undo().turn, 0, "hidden without a turn to undo");
    tracked_write(&session, &cwd);
    with_app(|app, ui| app.refresh_run(ui));
    assert_eq!(ui.get_undo().summary.as_str(), "Turn 1 changed 1 file with write or edit");
    // During a turn: refused, neither steered nor queued.
    with_app(|app, ui| {
        app.runs.shown.busy = true;
        assert!(!turn::send(app, ui, "/undo"));
        typed(app, ui, false);
        app.runs.shown.busy = false;
    });
    assert_eq!(ui.get_error_text().as_str(), RUNNING);
    assert!(session.status().queued.is_empty() && rx.try_recv().is_err());
    // A conflict: nothing changes, the row offers Undo anyway.
    std::fs::write(cwd.join("u.txt"), "by hand").unwrap();
    assert_eq!(with_app(|app, ui| turn::send(app, ui, "/undo")), Some(true));
    let (id, result, queued) = rx.recv_timeout(Duration::from_secs(20)).unwrap();
    with_app(|app, ui| finished(app, ui, &id, result, queued));
    assert_eq!(std::fs::read_to_string(cwd.join("u.txt")).unwrap(), "by hand");
    assert!(ui.get_undo().conflict.as_str().ends_with("u.txt"), "{:?}", ui.get_undo().conflict);
    let refused = with_app(|app, _| notes(app)).unwrap();
    assert_eq!(refused[refused.len() - 2].0, NoteKind::Error);
    // Undo anyway: the second, explicit click.
    ui.invoke_undo_turn(true);
    let (id, result, queued) = rx.recv_timeout(Duration::from_secs(20)).unwrap();
    with_app(|app, ui| finished(app, ui, &id, result, queued));
    assert_eq!(std::fs::read_to_string(cwd.join("u.txt")).unwrap(), "before");
    let done = with_app(|app, _| notes(app)).unwrap();
    assert!(done[done.len() - 2].1.ends_with("(forced over later changes)"), "{done:?}");
    assert_eq!(ui.get_undo().turn, 0, "nothing left to undo");
    let _ = ui.hide();
    let _ = std::fs::remove_dir_all(&dir);
}
