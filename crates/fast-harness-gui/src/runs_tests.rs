use super::*;
use crate::app::state_text;
use fast_harness_core::view::{Item, State};

fn fresh() -> Result<Run, String> {
    Ok(Run::new(PathBuf::from("."), None, Transcript::new()))
}

fn user(text: &str) -> AgentEvent {
    AgentEvent::UserInput {
        text: text.into(),
        steer: false,
    }
}

fn users(run: &Run) -> Vec<String> {
    (run.view.items.iter())
        .filter_map(|item| match item {
            Item::User { text } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// A conversation in a fresh temporary folder (removed by the caller).
fn conversation(name: &str) -> (PathBuf, Conversation) {
    let dir = std::env::temp_dir().join(format!("fh-gui-runs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let conv = Conversation::create(&dir, &dir, None).unwrap();
    (dir, conv)
}

#[test]
fn start_marks_the_run_busy() {
    let mut runs = Runs::new(PathBuf::from("."));
    runs.show(Some("a".into()), fresh).unwrap();
    assert!(runs.is_shown("a"));
    let (cancel, _inbox) = runs.shown.begin();
    assert!(runs.shown.busy && !cancel.load(Ordering::SeqCst));
    assert_eq!(runs.mark("a", None), "running");
    assert_eq!(state_text(State::Idle, &runs.shown), "Starting");
}

#[test]
fn steer_goes_to_the_inbox_until_the_turn_reads_it() {
    let mut run = fresh().unwrap();
    // Not busy: the text waits in the queue.
    assert!(!run.steer("early".into()));
    assert_eq!(run.queued, ["early"]);
    run.queued.clear();
    let (_, mut inbox) = run.begin();
    assert!(run.steer("go left\nand then up".into()));
    assert_eq!(run.chips(), [(-1, "go left ...".to_string())]);
    run.apply(&AgentEvent::UserInput {
        text: "go left\nand then up".into(),
        steer: true,
    });
    assert!(run.steering.is_empty());
    assert_eq!(users(&run), ["go left\nand then up"]);
    // The turn never read it here: the inbox hands it back.
    assert_eq!(inbox.leftover(), ["go left\nand then up"]);
}

#[test]
fn queue_and_remove_chips() {
    let mut run = fresh().unwrap();
    let _wires = run.begin();
    for text in ["a", "b", "c"] {
        run.queue(text.into());
    }
    assert_eq!(run.remove_queued(1).as_deref(), Some("b"));
    assert_eq!(run.remove_queued(5), None);
    assert_eq!(run.chips(), [(0, "a".to_string()), (1, "c".to_string())]);
}

#[test]
fn done_starts_the_next_text_and_pause_keeps_the_queue() {
    let mut run = fresh().unwrap();
    let _wires = run.begin();
    run.queue("q1".into());
    // Unused steers come back first.
    let next = run.end(None, TurnOutcome::Done, vec!["late".into()]);
    assert_eq!(
        (next.as_deref(), run.queued.clone()),
        (Some("late"), vec!["q1".to_string()])
    );
    let _wires = run.begin();
    assert!(run.pause() && run.pausing);
    assert_eq!(
        state_text(State::Idle, &run),
        "Pausing after this step's tools"
    );
    assert_eq!(run.end(None, TurnOutcome::Paused, Vec::new()), None);
    assert_eq!(
        (run.unfinished, run.queued.len()),
        (Some(Unfinished::Paused), 1)
    );
    assert!(!run.busy && !run.pause());
}

#[test]
fn pause_then_resume() {
    let (dir, conv) = conversation("resume");
    let mut runs = Runs::new(dir.clone());
    runs.show(Some(conv.id.clone()), fresh).unwrap();
    let id = conv.id.clone();
    let _wires = runs.shown.begin();
    assert_eq!(
        runs.shown.end(Some(conv), TurnOutcome::Paused, Vec::new()),
        None
    );
    assert!(runs.shown.can_resume());
    assert_eq!(runs.mark(&id, None), "paused");
    assert_eq!(state_text(State::Idle, &runs.shown), "Paused");
    let _wires = runs.shown.begin();
    assert!(!runs.shown.can_resume() && runs.shown.unfinished.is_none());
    assert_eq!(runs.mark(&id, Some(Unfinished::Paused)), "running");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stop_cancels_and_offers_resume() {
    let mut run = fresh().unwrap();
    assert!(!run.stop());
    let (cancel, _inbox) = run.begin();
    assert!(run.stop() && cancel.load(Ordering::SeqCst));
    assert_eq!(state_text(State::Thinking, &run), "Stopping");
    run.end(None, TurnOutcome::Cancelled, Vec::new());
    assert_eq!(run.mark(), "cancelled");
    // Without its conversation there is nothing to resume.
    assert!(!run.can_resume());
}

#[test]
fn background_session_events_stay_in_their_run() {
    let mut runs = Runs::new(PathBuf::from("."));
    runs.show(Some("a".into()), fresh).unwrap();
    let _wires = runs.shown.begin();
    runs.show(Some("b".into()), fresh).unwrap();
    assert_eq!(runs.parked_ids(), ["a"]);
    runs.get_mut("a").unwrap().apply(&user("for a"));
    assert!(users(&runs.shown).is_empty());
    assert_eq!(runs.mark("a", None), "running");
    assert_eq!(runs.mark("c", Some(Unfinished::Interrupted)), "interrupted");
    // Back to a: its live run returns; idle b is dropped.
    runs.show(Some("a".into()), || Err("no load".into()))
        .unwrap();
    assert_eq!(users(&runs.shown), ["for a"]);
    assert!(runs.parked_ids().is_empty());
}

#[test]
fn parked_run_settles_after_its_turn() {
    let mut runs = Runs::new(PathBuf::from("."));
    runs.show(Some("a".into()), fresh).unwrap();
    let _wires = runs.shown.begin();
    runs.shown.queue("next".into());
    runs.show(None, fresh).unwrap();
    let run = runs.get_mut("a").unwrap();
    assert_eq!(
        run.end(None, TurnOutcome::Done, Vec::new()).as_deref(),
        Some("next")
    );
    // A follow-up started in the background keeps it parked.
    let _wires = runs.get_mut("a").unwrap().begin();
    runs.settle("a");
    assert_eq!(runs.parked_ids(), ["a"]);
    runs.get_mut("a")
        .unwrap()
        .end(None, TurnOutcome::Failed, Vec::new());
    runs.settle("a");
    assert!(runs.parked_ids().is_empty());
    assert!(runs.show(Some("a".into()), || Err("gone".into())).is_err());
    assert_eq!(runs.id, None);
}
