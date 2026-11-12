use super::*;
use fast_harness_core::view::NoteKind;

#[test]
fn start_marks_the_run_busy() {
    let mut w = Window::new("busy");
    let id = w.show_new();
    assert!(w.runs.is_shown(&id));
    let (model, open) = gated(&["hi"]);
    w.start(&id, Start::Text("go".into()), model).unwrap();
    assert!(w.runs.shown.busy && !w.runs.shown.status().stopping);
    assert_eq!(w.runs.mark(&id, None), "running");
    assert_eq!(state_text(State::Idle, &w.runs.shown), "Starting");
    open.send(()).unwrap();
    let (end, next) = w.pump(&id);
    assert_eq!((end.outcome, next), (TurnOutcome::Done, None));
    assert_eq!(w.runs.mark(&id, None), "");
    assert_eq!(state_text(w.runs.shown.view.state, &w.runs.shown), "Ready");
}

#[test]
fn steer_goes_to_the_inbox_until_the_turn_reads_it() {
    let mut w = Window::new("steer");
    let id = w.show_new();
    let session = w.session(&id);
    // No turn runs: the text waits in the queue.
    assert!(!session.steer("early".into()));
    assert_eq!(session.status().queued, ["early"]);
    session.drop_queue();
    let (model, open) = gated(&["one", "two"]);
    w.start(&id, Start::Text("go".into()), model).unwrap();
    assert!(session.steer("go left\nand then up".into()));
    assert_eq!(w.runs.shown.chips(), [(-1, "go left ...".to_string())]);
    for _ in 0..2 {
        open.send(()).unwrap();
    }
    let (end, _) = w.pump(&id);
    assert_eq!(end.outcome, TurnOutcome::Done, "{:?}", end.error);
    assert!(session.status().steering.is_empty());
    assert_eq!(users(&w.runs.shown), ["go", "go left\nand then up"]);
}

#[test]
fn queue_and_remove_chips() {
    let mut w = Window::new("chips");
    let id = w.show_new();
    let session = w.session(&id);
    let (model, open) = gated(&["hi"]);
    w.start(&id, Start::Text("go".into()), model).unwrap();
    for text in ["a", "b", "c"] {
        session.queue(text.into());
    }
    assert_eq!(session.remove_queued(1).as_deref(), Some("b"));
    assert_eq!(session.remove_queued(5), None);
    assert_eq!(
        w.runs.shown.chips(),
        [(0, "a".to_string()), (1, "c".to_string())]
    );
    session.drop_queue();
    open.send(()).unwrap();
    w.pump(&id);
}

#[test]
fn done_starts_the_next_text_and_pause_keeps_the_queue() {
    let mut w = Window::new("pause");
    let id = w.show_new();
    let session = w.session(&id);
    w.start(&id, Start::Text("go".into()), model(&["hi"]))
        .unwrap();
    session.queue("q1".into());
    let (_, next) = w.pump(&id);
    assert_eq!(next.as_deref(), Some("q1"));
    assert!(session.status().queued.is_empty());
    let (model, open) = gated(&["hi"]);
    w.start(&id, Start::Text("go on".into()), model).unwrap();
    session.queue("q2".into());
    assert!(session.pause() && session.status().pausing);
    assert_eq!(
        state_text(State::Idle, &w.runs.shown),
        "Pausing after this step's tools"
    );
    open.send(()).unwrap();
    let (end, next) = w.pump(&id);
    assert_eq!((end.outcome, next), (TurnOutcome::Paused, None));
    let status = session.status();
    assert_eq!(
        (status.unfinished, status.queued),
        (Some(Unfinished::Paused), vec!["q2".to_string()])
    );
    assert!(!w.runs.shown.busy && !session.pause());
}

#[test]
fn pause_then_resume() {
    let mut w = Window::new("resume");
    let id = w.show_new();
    let session = w.session(&id);
    let (model, open) = gated(&["hi"]);
    w.start(&id, Start::Text("go".into()), model).unwrap();
    assert!(session.pause());
    open.send(()).unwrap();
    w.pump(&id);
    assert!(w.runs.shown.can_resume());
    assert_eq!(w.runs.mark(&id, None), "paused");
    assert_eq!(state_text(State::Idle, &w.runs.shown), "Paused");
    let (model, open) = gated(&["more"]);
    w.start(&id, Start::Resume, model).unwrap();
    let status = w.runs.shown.status();
    assert!(!w.runs.shown.can_resume() && status.unfinished.is_none());
    assert_eq!(w.runs.mark(&id, Some(Unfinished::Paused)), "running");
    open.send(()).unwrap();
    w.pump(&id);
}

#[test]
fn stop_with_a_queue_cancels_and_offers_resume() {
    let mut w = Window::new("stop");
    let id = w.show_new();
    let session = w.session(&id);
    assert!(!session.cancel());
    let (model, open) = gated(&["hi"]);
    w.start(&id, Start::Text("go".into()), model).unwrap();
    session.queue("after".into());
    assert!(session.cancel() && session.status().stopping);
    assert_eq!(state_text(State::Thinking, &w.runs.shown), "Stopping");
    open.send(()).unwrap();
    let (end, next) = w.pump(&id);
    assert_eq!((end.outcome, next), (TurnOutcome::Cancelled, None));
    assert_eq!(w.runs.shown.mark(), "cancelled");
    // The queue waits for the user; the turn can go on.
    assert_eq!(session.status().queued, ["after"]);
    assert!(w.runs.shown.can_resume());
}

#[test]
fn background_session_events_stay_in_their_run() {
    let mut w = Window::new("parked");
    let a = w.show_new();
    let (model, open) = gated(&["for a"]);
    w.start(&a, Start::Text("ask a".into()), model).unwrap();
    let b = w.show_new();
    assert_eq!(w.runs.parked_ids(), std::slice::from_ref(&a));
    // The window applies a parked session's events, and is told not to
    // touch the rows, the status or the panels with them.
    let event = AgentEvent::UserInput {
        text: "late".into(),
        steer: false,
    };
    assert_eq!(w.runs.apply(&a, &event), Some(false));
    assert_eq!(w.runs.apply("gone", &event), None);
    open.send(()).unwrap();
    let (end, _) = w.pump(&a);
    assert_eq!(end.outcome, TurnOutcome::Done);
    assert!(users(&w.runs.shown).is_empty() && w.runs.is_shown(&b));
    assert!(w.runs.shown.view.items.is_empty());
    assert_eq!(w.runs.mark(&a, None), "");
    assert_eq!(w.runs.mark("c", Some(Unfinished::Interrupted)), "interrupted");
    // Back to a: its run returns with its result; idle b is dropped.
    w.runs.show(Some(a.clone()), || Err("no load".into())).unwrap();
    assert_eq!(users(&w.runs.shown), ["ask a", "late"]);
    let answered = (w.runs.shown.view.items.iter())
        .any(|item| matches!(item, Item::Assistant { text, .. } if text == "for a"));
    assert!(answered, "{:?}", w.runs.shown.view.items.len());
    assert!(w.runs.parked_ids().is_empty());
}

#[test]
fn parked_run_settles_after_its_turn() {
    let mut w = Window::new("settle");
    let a = w.show_new();
    let session = w.session(&a);
    let (model, open) = gated(&["hi"]);
    w.start(&a, Start::Text("go".into()), model).unwrap();
    session.queue("next".into());
    let blank = Run::new(w.dir.clone(), None, Transcript::new());
    w.runs.show(None, || Ok(blank)).unwrap();
    open.send(()).unwrap();
    let (_, next) = w.pump(&a);
    assert_eq!(next.as_deref(), Some("next"));
    // A follow-up started in the background keeps it parked.
    let (model, open) = gated(&["hi again"]);
    w.start(&a, Start::Text("next".into()), model).unwrap();
    assert!(!w.runs.settle(&a));
    assert_eq!(w.runs.parked_ids(), std::slice::from_ref(&a));
    open.send(()).unwrap();
    w.pump(&a);
    assert!(w.runs.settle(&a));
    assert!(w.runs.parked_ids().is_empty());
    assert!(w.runs.show(Some(a), || Err("gone".into())).is_err());
    assert_eq!(w.runs.id, None);
}

#[test]
fn a_turn_that_panics_ends_failed_and_the_next_prompt_runs() {
    let mut w = Window::new("panic");
    let id = w.show_new();
    let boom = model(&[]);
    boom.panics.store(true, Ordering::SeqCst);
    w.start(&id, Start::Text("go".into()), boom).unwrap();
    let (end, next) = w.pump(&id);
    assert_eq!((end.outcome, end.panicked, next), (TurnOutcome::Failed, true, None));
    let run = &w.runs.shown;
    assert!(!run.busy && run.mark() != "running");
    assert_eq!(state_text(run.view.state, run), "Ready");
    let shown = (run.view.items.iter()).any(|item| {
        matches!(item, Item::Note { kind: NoteKind::Error, text } if text.contains("boom transport"))
    });
    assert!(shown, "the error shows in the transcript");
    assert!(crate::turn::crash_text().contains(crate::crash::FILE_NAME));
    w.start(&id, Start::Text("again".into()), model(&["fine"]))
        .unwrap();
    let (end, _) = w.pump(&id);
    assert_eq!(end.outcome, TurnOutcome::Done, "{:?}", end.error);
    assert_eq!(users(&w.runs.shown), ["go", "again"]);
}
