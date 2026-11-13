use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::Arc;
use std::time::Duration;

use super::*;

const WAIT: Duration = Duration::from_secs(20);

#[test]
fn the_tail_keeps_the_newest_lines_and_says_when_earlier_ones_went() {
    let short = tail("a\nb\n", 0, true);
    assert_eq!((short.text.as_str(), short.dropped), ("a\nb\n", false));
    assert!(tail("a\n", 5, false).dropped, "bytes gone from memory");
    let many: String = (0..1000).map(|i| format!("line {i}\n")).collect();
    let cut = tail(&many, 0, true);
    assert!(cut.dropped);
    assert_eq!(cut.text.lines().count(), TAIL_LINES);
    assert!(cut.text.starts_with("line 600\n") && cut.text.ends_with("line 999\n"));
    let wide = "é".repeat(TAIL_BYTES);
    let cut = tail(&wide, 0, false);
    assert!(cut.text.len() <= TAIL_BYTES && cut.text.chars().all(|c| c == 'é'));
}

/// A view's read leaves the model's `job_output` cursor alone: the model
/// still gets every byte after the view read it.
#[test]
fn a_peek_does_not_consume_the_model_read() {
    let jobs = Jobs::default();
    let cwd = std::env::temp_dir();
    let id = jobs.start("echo one; echo two", &cwd, None).unwrap();
    let ended = jobs.read(&id, WAIT, true, None).unwrap();
    assert!(ended.text.contains("two"), "{ended:?}");
    let view = read(&jobs, &id).unwrap();
    assert!(view.text.contains("one") && view.text.contains("two"));
    assert!(!view.running && !view.dropped);
    let again = read(&jobs, &id).unwrap();
    assert_eq!(view, again, "a peek reads the same again");
    let model = jobs.read(&id, Duration::ZERO, false, None).unwrap();
    assert_eq!(model.text, "", "the model's cursor did not move back");
    assert!(read(&jobs, "job-99").is_none());
}

/// The worker posts the tail at once and again when the job ends, then
/// ends when its watch is dropped (its sender drops).
#[test]
fn the_tail_worker_follows_the_job_and_ends_when_dropped() {
    let jobs = Arc::new(Jobs::default());
    let cwd = std::env::temp_dir();
    let id = jobs
        .start("echo first; sleep 1; echo second", &cwd, None)
        .unwrap();
    let (tx, rx) = channel();
    let watch = TailWatch::start(jobs.clone(), &id, move |t| tx.send(t).is_ok()).unwrap();
    let last = loop {
        let tail = rx.recv_timeout(WAIT).expect("a post");
        if !tail.running {
            break tail;
        }
    };
    assert!(
        last.text.contains("first") && last.text.contains("second"),
        "{last:?}"
    );
    drop(watch);
    loop {
        match rx.recv_timeout(WAIT) {
            Ok(_) => continue,
            Err(err) => break assert_eq!(err, RecvTimeoutError::Disconnected),
        }
    }
}
