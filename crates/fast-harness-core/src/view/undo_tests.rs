//! The notes of an undo (PLAN.md D69 "Views").

use std::path::PathBuf;

use super::*;
use crate::tools::UndoOutcome;

fn done() -> UndoOutcome {
    UndoOutcome {
        turn: Some(2),
        restored: vec![PathBuf::from("a.txt")],
        removed: vec![PathBuf::from("new.txt")],
        untracked: UNTRACKED.to_string(),
        ..UndoOutcome::default()
    }
}

#[test]
fn a_done_undo_shows_what_a_replay_of_its_record_shows() {
    let outcome = done();
    let live = notes(&outcome);
    let replay = record_notes(&record(&outcome).expect("files changed"));
    assert_eq!(live, replay);
    assert_eq!(
        live[0].1,
        "undid the file changes of turn 2: restored a.txt; removed new.txt"
    );
    assert_eq!(live[1], (NoteKind::Info, UNTRACKED.to_string()));
}

#[test]
fn a_conflict_names_the_files_and_the_way_on_in_an_error_note() {
    let outcome = UndoOutcome {
        turn: Some(2),
        conflicts: vec![PathBuf::from("a.txt")],
        refused: Some("1 file changed since turn 2".into()),
        untracked: UNTRACKED.to_string(),
        ..UndoOutcome::default()
    };
    assert!(record(&outcome).is_none(), "a refusal logs nothing");
    let shown = notes(&outcome);
    assert_eq!(shown[0].0, NoteKind::Error);
    let text = &shown[0].1;
    assert!(
        text.starts_with("nothing was undone: 1 file changed"),
        "{text}"
    );
    assert!(text.contains("changed since that turn: a.txt"), "{text}");
    assert!(text.contains("/undo force or Undo anyway"), "{text}");
    assert_eq!(shown[1].1, UNTRACKED);
}

#[test]
fn a_forced_undo_says_so_and_a_failed_file_is_an_error() {
    let mut outcome = done();
    outcome.forced = true;
    outcome.errors = vec!["b.txt: access denied".into()];
    let shown = notes(&outcome);
    assert!(shown[0].1.ends_with("(forced over later changes)"));
    assert_eq!(
        shown[1],
        (NoteKind::Error, "not put back: b.txt: access denied".into())
    );
    let mut view = Transcript::new();
    view.undone(&outcome);
    assert_eq!(view.items.len(), 3);
}
