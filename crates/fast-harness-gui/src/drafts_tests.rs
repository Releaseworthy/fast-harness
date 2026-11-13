//! Each session keeps its own composer draft (D65): A to B to A, the new
//! session's draft after switching away and pressing New session again, a
//! draft cleared by its send, and the drafts while a session opens on a
//! worker (D60). No event loop runs here, so the workers' results are handed
//! in by hand, with the generation each ask or cancel took.

use std::path::PathBuf;
use std::rc::Rc;

use fast_harness_core::agent::Conversation;
use fast_harness_core::app::Runtime;
use fast_harness_core::ui_thread;
use fast_harness_core::view::opened::{self, Opened};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::ComponentHandle;

use crate::app::{install, with_app, App};
use crate::{open_load, turn, MainWindow};

struct TestPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

struct Env {
    ui: MainWindow,
    dir: PathBuf,
    home: PathBuf,
    cwd: PathBuf,
    a: String,
    b: String,
}

impl Drop for Env {
    fn drop(&mut self) {
        ui_thread::mark(false);
        let _ = self.ui.hide();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A window showing a new session, on a home with sessions `a` and `b` and a
/// local provider that refuses connections (a send starts a turn).
fn setup(tag: &str) -> Env {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let dir = std::env::temp_dir().join(format!("fh-drafts-{tag}-{}-{nanos}", std::process::id()));
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let _ = slint::platform::set_platform(Box::new(TestPlatform(window)));
    let ui = MainWindow::new().expect("window");
    let (home, cwd) = (dir.join("home"), dir.join("work"));
    std::fs::create_dir_all(home.join("providers")).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::write(
        home.join("config.json"),
        r#"{"model": "local/m", "retries": 0}"#,
    )
    .unwrap();
    let provider = r#"{"id": "local", "api": "anthropic-messages", "baseUrl": "http://127.0.0.1:9", "models": [{"id": "m"}]}"#;
    std::fs::write(home.join("providers").join("local.json"), provider).unwrap();
    let a = Conversation::create(&home, &cwd, None).unwrap().id;
    let b = Conversation::create(&home, &cwd, None).unwrap().id;
    let rt = Runtime::load_at(home.clone()).unwrap();
    install(App::new(&ui, rt, Some(cwd.clone())));
    ui_thread::mark(true);
    Env {
        ui,
        dir,
        home,
        cwd,
        a,
        b,
    }
}

/// A worker's open of `id`, read with the mark lifted (as its own thread
/// would), handed in as generation `generation`.
fn arrive(env: &Env, id: &str, generation: u64) {
    ui_thread::mark(false);
    let result: Result<Opened, String> = opened::load(&env.home, &env.cwd, id);
    ui_thread::mark(true);
    with_app(|app, ui| open_load::arrived(app, ui, generation, &env.cwd, result));
}

fn open(id: &str) {
    with_app(|app, ui| app.open_session(ui, id));
}

fn new_session() {
    with_app(|app, ui| app.new_session(ui));
}

fn draft(env: &Env) -> String {
    env.ui.get_draft().to_string()
}

fn shown() -> Option<String> {
    with_app(|app, _| app.runs.id.clone()).flatten()
}

#[test]
fn a_to_b_to_a_shows_each_sessions_own_draft() {
    let env = setup("aba");
    open(&env.a);
    arrive(&env, &env.a, 1);
    env.ui.set_draft("for A".into());
    open(&env.b);
    assert_eq!(draft(&env), "", "B loading: never A's text");
    arrive(&env, &env.b, 2);
    assert_eq!((shown(), draft(&env)), (Some(env.b.clone()), String::new()));
    env.ui.set_draft("for B".into());
    open(&env.a);
    assert_eq!(draft(&env), "for A", "A's draft while A loads");
    arrive(&env, &env.a, 3);
    assert_eq!(
        (shown(), draft(&env)),
        (Some(env.a.clone()), "for A".into())
    );
    open(&env.b);
    arrive(&env, &env.b, 4);
    assert_eq!(draft(&env), "for B");
}

#[test]
fn the_new_sessions_draft_comes_back_with_new_session() {
    let env = setup("new");
    env.ui.set_draft("new text".into());
    open(&env.a);
    assert_eq!(draft(&env), "");
    arrive(&env, &env.a, 1);
    env.ui.set_draft("for A".into());
    new_session();
    assert_eq!((shown(), draft(&env)), (None, "new text".into()));
    open(&env.a);
    arrive(&env, &env.a, 3);
    assert_eq!(draft(&env), "for A");
    new_session();
    assert_eq!(draft(&env), "new text", "pressed again: still there");
}

#[test]
fn a_sent_draft_is_gone_from_its_session_and_from_new_session() {
    let env = setup("sent");
    env.ui.set_draft("hello".into());
    // What the window's submit does: send, and clear the composer if taken.
    let sent = with_app(|app, ui| turn::send(app, ui, "hello"));
    assert_eq!(sent, Some(true), "{}", env.ui.get_error_text());
    env.ui.set_draft("".into());
    let created = shown().expect("the first prompt creates the session");
    // The next text typed belongs to the created session, not to New session.
    env.ui.set_draft("next for it".into());
    open(&env.a);
    arrive(&env, &env.a, 1);
    env.ui.set_draft("for A".into());
    new_session();
    assert_eq!(
        draft(&env),
        "",
        "the sent session's text does not come back"
    );
    open(&created);
    assert_eq!(draft(&env), "next for it");
    open(&env.a);
    assert_eq!(draft(&env), "for A");
}

#[test]
fn a_dropped_or_failed_open_never_swaps_the_draft_shown() {
    let env = setup("stale");
    open(&env.a);
    arrive(&env, &env.a, 1);
    env.ui.set_draft("for A".into());
    open(&env.b);
    env.ui.set_draft("typed while B loads".into());
    // Back to the shown A before B came: A's draft, and B's result drops.
    open(&env.a);
    assert_eq!(draft(&env), "for A");
    arrive(&env, &env.b, 2);
    assert_eq!(
        (shown(), draft(&env)),
        (Some(env.a.clone()), "for A".into())
    );
    open(&env.b);
    assert_eq!(draft(&env), "typed while B loads");
    arrive(&env, &env.b, 4);
    // An open that fails keeps B shown, with B's draft.
    open("20990101-000000-0000");
    assert_eq!(draft(&env), "");
    arrive(&env, "20990101-000000-0000", 5);
    assert_eq!(shown(), Some(env.b.clone()));
    assert_eq!(draft(&env), "typed while B loads");
}
