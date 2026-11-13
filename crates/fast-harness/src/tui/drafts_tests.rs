//! Each session keeps its own prompt draft (D65): A to B to A with the
//! cursor kept, the new session's draft after switching away and pressing
//! New session again, a draft gone with its send, and the drafts while a
//! session opens on a worker (D60). The workers' results are handed in by
//! hand, as the channel would deliver them.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use fast_harness_core::agent::Conversation;
use fast_harness_core::app::Runtime;
use fast_harness_core::view::opened::{self, Opened};

use super::app::{Msg, Tui};
use super::ui::Action;

struct Env {
    tui: Tui,
    rx: Receiver<Msg>,
    root: PathBuf,
    home: PathBuf,
    cwd: PathBuf,
    a: String,
    b: String,
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A TUI on a new session (an empty log), with sessions `a` and `b` and a
/// local provider that refuses connections (a send runs a turn that fails).
fn setup(tag: &str) -> Env {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let root = std::env::temp_dir().join(format!("fh-drafts-{tag}-{}-{nanos}", std::process::id()));
    let (home, cwd) = (root.join("home"), root.join("work"));
    std::fs::create_dir_all(home.join("providers")).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::write(
        home.join("config.json"),
        r#"{"model": "local/m", "retries": 0}"#,
    )
    .unwrap();
    let provider = r#"{"id": "local", "api": "anthropic-messages", "baseUrl": "http://127.0.0.1:9", "models": [{"id": "m"}]}"#;
    std::fs::write(home.join("providers").join("local.json"), provider).unwrap();
    let id = || Conversation::create(&home, &cwd, None).unwrap().id;
    let (a, b) = (id(), id());
    let rt = Runtime::load_at(home.clone()).unwrap();
    let opened = rt.open_with_events(&cwd, None, false).unwrap();
    let (tx, rx) = mpsc::channel();
    let tui = Tui::new(rt, cwd.clone(), opened, tx).unwrap();
    Env {
        tui,
        rx,
        root,
        home,
        cwd,
        a,
        b,
    }
}

/// Asks session `id` and hands in its worker's result as `generation`.
fn open(env: &mut Env, id: &str, generation: u64) {
    env.tui.act(Action::Resume(id.to_string()));
    arrive(env, id, generation);
}

fn arrive(env: &mut Env, id: &str, generation: u64) {
    let result = opened::load(&env.home, &env.cwd, id).map(Box::new);
    let result: Result<Box<Opened>, String> = result;
    env.tui.handle(Msg::Opened(generation, result));
}

fn text(env: &Env) -> &str {
    env.tui.ui.input.text()
}

#[test]
fn a_to_b_to_a_shows_each_sessions_own_draft_with_its_cursor() {
    let mut env = setup("aba");
    let (a, b) = (env.a.clone(), env.b.clone());
    open(&mut env, &a, 1);
    env.tui.ui.input.insert("for A");
    env.tui.ui.input.left();
    env.tui.act(Action::Resume(env.b.clone()));
    assert_eq!(text(&env), "", "B loading: never A's text");
    arrive(&mut env, &b, 2);
    assert_eq!(env.tui.session, env.b);
    env.tui.ui.input.insert("for B");
    env.tui.act(Action::Resume(env.a.clone()));
    assert_eq!(text(&env), "for A", "A's draft while A loads");
    arrive(&mut env, &a, 3);
    // The cursor came back where it was: before the last character.
    env.tui.ui.input.insert("!");
    assert_eq!(text(&env), "for !A");
    open(&mut env, &b, 4);
    assert_eq!(text(&env), "for B");
}

#[test]
fn the_new_sessions_draft_comes_back_with_new_session() {
    let mut env = setup("new");
    let a = env.a.clone();
    env.tui.ui.input.insert("new text");
    open(&mut env, &a, 1);
    assert_eq!(text(&env), "");
    env.tui.ui.input.insert("for A");
    env.tui.act(Action::NewSession);
    assert_eq!(text(&env), "new text");
    let fresh = env.tui.session.clone();
    assert!(fresh != env.a && fresh != env.b, "a new session");
    open(&mut env, &a, 3);
    assert_eq!(text(&env), "for A");
    env.tui.act(Action::NewSession);
    assert_eq!(text(&env), "new text", "pressed again: still there");
}

#[test]
fn a_sent_draft_is_gone_from_its_session_and_from_new_session() {
    let mut env = setup("sent");
    let a = env.a.clone();
    let created = env.tui.session.clone();
    env.tui.ui.input.insert("hello");
    let action = env.tui.ui.key(enter());
    env.tui.act(action);
    assert_eq!(text(&env), "");
    // The turn fails on the refused connection; wait for its end.
    while env.tui.ui.running {
        let msg = env
            .rx
            .recv_timeout(Duration::from_secs(20))
            .expect("turn end");
        env.tui.handle(msg);
    }
    env.tui.ui.input.insert("next for it");
    open(&mut env, &a, 1);
    env.tui.act(Action::NewSession);
    assert_eq!(text(&env), "", "the sent session's text does not come back");
    open(&mut env, &created, 3);
    assert_eq!(text(&env), "next for it");
}

#[test]
fn a_dropped_or_failed_open_never_swaps_the_draft_shown() {
    let mut env = setup("stale");
    let (a, b) = (env.a.clone(), env.b.clone());
    open(&mut env, &a, 1);
    env.tui.ui.input.insert("for A");
    env.tui.act(Action::Resume(env.b.clone()));
    env.tui.ui.input.insert("typed while B loads");
    // Back to the shown A before B came: A's draft, and B's result drops.
    env.tui.act(Action::Resume(env.a.clone()));
    assert_eq!(text(&env), "for A");
    arrive(&mut env, &b, 2);
    assert_eq!(
        (env.tui.session.as_str(), text(&env)),
        (env.a.as_str(), "for A")
    );
    open(&mut env, &b, 4);
    assert_eq!(text(&env), "typed while B loads");
    // An open that fails keeps B shown, with B's draft.
    open(&mut env, "20990101-000000-0000", 5);
    assert_eq!(env.tui.session, env.b);
    assert_eq!(text(&env), "typed while B loads");
}

fn enter() -> ratatui::crossterm::event::KeyEvent {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
}
