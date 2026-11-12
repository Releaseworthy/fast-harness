use fast_harness_core::session;
use fast_harness_core::trace::{self, SessionHeader, TraceWriter};
use fast_harness_core::workspaces::{Workspaces, WorkspacesError};
use std::path::{Path, PathBuf};

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let path = std::env::temp_dir().join(format!(
            "fast-harness-test-ws-{name}-{}-{}",
            std::process::id(),
            trace::now_ms()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fake_session(home: &Path, cwd: &str, id: &str, created_at: u64) {
    let header = SessionHeader {
        version: trace::VERSION,
        id: id.to_string(),
        created_at,
        cwd: cwd.to_string(),
        harness: "fast-harness test".to_string(),
        parent: None,
        depth: 0,
    };
    let path = session::log_path(home, Path::new(cwd), id);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    TraceWriter::create(&path, &header).unwrap();
}

#[test]
fn a_missing_file_is_seeded_from_session_folders() {
    let home = TempDir::new("seed");
    fake_session(&home.0, "F:\\work\\a", "a1", 100);
    fake_session(&home.0, "F:\\work\\a", "a2", 300);
    fake_session(&home.0, "F:\\work\\b", "b1", 50);
    std::fs::create_dir_all(home.0.join("sessions").join("--empty--")).unwrap();
    let list = Workspaces::load(&home.0).unwrap().list();
    let paths: Vec<_> = list.iter().map(|w| w.path.clone()).collect();
    assert_eq!(
        paths,
        [PathBuf::from("F:\\work\\b"), PathBuf::from("F:\\work\\a")]
    );
    assert_eq!((list[1].added_at, list[1].last_used_at), (100, 300));
    assert!(
        !home.0.join("workspaces.json").exists(),
        "seeding does not save"
    );
}

#[test]
fn add_dedupes_by_slug_and_keeps_the_existing_entry() {
    let home = TempDir::new("dedupe");
    let mut list = Workspaces::load(&home.0).unwrap();
    assert!(list.add(Path::new("F:\\work\\x"), 10));
    assert!(!list.add(Path::new("F:/work/x"), 20));
    assert_eq!(list.list().len(), 1);
    assert_eq!(list.list()[0].added_at, 10);
    assert!(list.touch(Path::new("F:\\work\\x"), 30));
    assert_eq!(list.most_recent().unwrap().last_used_at, 30);
}

#[test]
fn remove_drops_the_entry_and_keeps_the_files() {
    let home = TempDir::new("remove");
    fake_session(&home.0, "F:\\work\\a", "a1", 100);
    let mut list = Workspaces::load(&home.0).unwrap();
    assert!(list.remove(Path::new("F:\\work\\a")));
    assert!(!list.remove(Path::new("F:\\work\\a")));
    assert!(list.list().is_empty());
    assert!(session::log_path(&home.0, Path::new("F:\\work\\a"), "a1").is_file());
}

#[test]
fn save_and_load_round_trip() {
    let home = TempDir::new("round");
    let mut list = Workspaces::load(&home.0).unwrap();
    list.add(Path::new("F:\\work\\b"), 20);
    list.add(Path::new("F:\\work\\a"), 10);
    list.save().unwrap();
    let text = std::fs::read_to_string(home.0.join("workspaces.json")).unwrap();
    assert!(text.contains("\"version\": 1") && text.contains("\"lastUsedAt\": 10"));
    let again = Workspaces::load(&home.0).unwrap();
    assert_eq!(again.list(), list.list());
    assert_eq!(again.list()[0].path, PathBuf::from("F:\\work\\a"));
}

#[test]
fn a_corrupt_file_or_another_version_is_an_error() {
    let home = TempDir::new("corrupt");
    let file = home.0.join("workspaces.json");
    std::fs::write(&file, "{not json").unwrap();
    assert!(matches!(
        Workspaces::load(&home.0),
        Err(WorkspacesError::Json(_))
    ));
    std::fs::write(&file, r#"{"version":2,"workspaces":[]}"#).unwrap();
    assert!(matches!(
        Workspaces::load(&home.0),
        Err(WorkspacesError::Version(2))
    ));
}
