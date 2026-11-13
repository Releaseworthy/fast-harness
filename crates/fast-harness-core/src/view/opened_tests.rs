//! A session opened on a worker (D60, gate G10): the open itself reads the
//! log off the UI thread, a result that is not the newest is dropped, and a
//! failed open is an error the view shows (it keeps the session it shows).

use std::path::{Path, PathBuf};

use crate::agent::Conversation;
use crate::session;
use crate::ui_thread;
use crate::view::opened::{load, Next, Opening};

fn root(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    std::env::temp_dir().join(format!("fh-opened-{tag}-{}-{nanos}", std::process::id()))
}

/// A home with one session in `work`.
fn setup(root: &Path) -> (PathBuf, PathBuf, String) {
    let (home, cwd) = (root.join("home"), root.join("work"));
    std::fs::create_dir_all(&cwd).unwrap();
    let id = Conversation::create(&home, &cwd, None).unwrap().id;
    (home, cwd, id)
}

#[test]
fn an_opening_keeps_one_worker_and_starts_the_newest_after_it() {
    let mut opening = Opening::default();
    assert!(!opening.loading());
    let first = opening.ask(Some("a")).expect("the first open starts");
    assert_eq!(first.generation, 1);
    assert_eq!(first.id.as_deref(), Some("a"));
    assert!(opening.loading());
    // A second open while one runs: remembered, not started.
    assert_eq!(opening.ask(Some("b")), None);
    // The first ends: the newest starts, so its own result is not applied.
    let second = match opening.arrive(first.generation) {
        Next::Start(ask) => ask,
        other => panic!("the newest should start, not {other:?}"),
    };
    assert_eq!((second.generation, second.id.as_deref()), (2, Some("b")));
    assert!(opening.loading());
    // The newest ends: applied, and nothing waits.
    assert_eq!(opening.arrive(second.generation), Next::Apply);
    assert!(!opening.loading());
}

#[test]
fn a_result_that_is_not_the_newest_is_dropped() {
    let mut opening = Opening::default();
    let first = opening.ask(Some("a")).expect("starts");
    assert_eq!(opening.ask(Some("b")), None);
    // The open of `a` ended after `b` was asked: dropped, `b` starts instead.
    assert!(matches!(opening.arrive(first.generation), Next::Start(_)));
    // Its duplicate arrival changes nothing at all.
    assert_eq!(opening.arrive(first.generation), Next::Drop);
}

#[test]
fn a_cancelled_open_drops_its_result_and_starts_nothing() {
    let mut opening = Opening::default();
    let first = opening.ask(Some("a")).expect("starts");
    opening.cancel();
    assert_eq!(opening.arrive(first.generation), Next::Drop);
    // The next ask starts on its own, since the worker slot is free.
    let next = opening.ask(Some("b")).expect("starts");
    assert_eq!(opening.arrive(next.generation), Next::Apply);
}

#[test]
fn a_failed_open_is_an_error_and_leaves_nothing_loading() {
    let root = root("error");
    let (home, cwd, _id) = setup(&root);
    let mut opening = Opening::default();
    let ask = opening.ask(Some("no-such-session")).expect("starts");
    let result = load(&home, &cwd, &ask.id.unwrap_or_default());
    let err = result.err().expect("an unknown session does not open");
    assert!(err.contains("no-such-session"), "{err}");
    assert_eq!(opening.arrive(ask.generation), Next::Apply);
    assert!(!opening.loading());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn opening_a_session_reads_its_log_off_the_ui_thread() {
    let root = root("worker");
    let (home, cwd, id) = setup(&root);
    assert!(session::log_path(&home, &cwd, &id).is_file());
    // The test thread is the front end's UI thread: a read here panics in a
    // debug build, so the worker thread is the only place it can happen.
    ui_thread::mark(true);
    let asked = id.clone();
    let opened = std::thread::spawn(move || load(&home, &cwd, &asked))
        .join()
        .expect("the worker does not panic");
    ui_thread::mark(false);
    let opened = opened.expect("the session opens");
    assert_eq!(opened.conv.id, id);
    let _ = std::fs::remove_dir_all(&root);
}
