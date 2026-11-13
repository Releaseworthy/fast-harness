use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use super::*;

const WAIT: Duration = Duration::from_secs(20);

fn info(id: &str, command: &str, state: JobState, exit_code: Option<i32>) -> JobInfo {
    let started = SystemTime::now();
    let ended = (state != JobState::Running).then(|| started + Duration::from_secs(125));
    JobInfo {
        id: id.into(),
        command: command.into(),
        state,
        exit_code,
        started,
        ended,
        bytes: 0,
        log: None,
    }
}

type Posted = Receiver<(u64, Vec<JobInfo>)>;

/// A post that sends to a channel; the receiver sees the worker end when
/// the sender, owned by the worker, drops.
fn post() -> (impl Fn(u64, Vec<JobInfo>) -> bool + Send + 'static, Posted) {
    let (tx, rx) = channel();
    (
        move |generation, list| tx.send((generation, list)).is_ok(),
        rx,
    )
}

#[test]
fn rows_say_each_state_in_a_word_and_a_mark() {
    let list = [
        info("job-1", "ping -t localhost", JobState::Running, None),
        info("job-2", "cargo build", JobState::Exited, Some(0)),
        info("job-3", "exit 3", JobState::Exited, Some(3)),
        info("job-4", "sleep 60", JobState::Killed, Some(1)),
        info("job-5", "true", JobState::Exited, None),
    ];
    let rows = rows(&list);
    assert_eq!(rows[0].took, None, "no duration while it runs");
    assert_eq!(rows[1].took, Some(Duration::from_secs(125)));
    let words: Vec<(&str, Mark)> = rows.iter().map(|r| (r.word.as_str(), r.mark)).collect();
    assert_eq!(
        words,
        [
            ("running", Mark::Running),
            ("exited 0", Mark::Exited),
            ("exited 3", Mark::Failed),
            ("killed", Mark::Killed),
            ("exited", Mark::Exited),
        ]
    );
    assert!(rows[0].running() && !rows[1].running());
    assert_eq!(rows[0].text(), "job-1 ping -t localhost running");
}

#[test]
fn a_long_or_multi_line_command_is_shortened() {
    assert_eq!(
        short_command("  ping   -t\tlocalhost "),
        "ping -t localhost"
    );
    assert_eq!(short_command("\nnpm run dev\nnext"), "npm run dev …");
    let long = "x".repeat(100);
    let short = short_command(&long);
    assert_eq!(short.chars().count(), COMMAND_CHARS);
    assert!(short.ends_with('…'));
}

#[test]
fn the_worker_posts_a_change_and_ends_when_stopped() {
    let jobs = Arc::new(Jobs::default());
    let mut follow = Follow::default();
    let (post, rx) = post();
    assert_eq!(follow.target(Some(jobs.clone()), post), Some(Vec::new()));
    let cwd = std::env::temp_dir();
    let id = jobs.start("echo hi", &cwd, None).unwrap();
    let (generation, list) = rx.recv_timeout(WAIT).unwrap();
    let rows = follow.arrived(generation, &list).unwrap();
    assert_eq!(rows[0].id, id);
    follow.stop();
    // The worker's sender drops when it ends; the job's end may post first.
    loop {
        match rx.recv_timeout(WAIT) {
            Ok(_) => continue,
            Err(err) => break assert_eq!(err, RecvTimeoutError::Disconnected),
        }
    }
    drop(jobs.kill(&id));
}

#[test]
fn a_session_switch_retargets_the_worker_and_drops_a_stale_list() {
    let (a, b) = (Arc::new(Jobs::default()), Arc::new(Jobs::default()));
    let mut follow = Follow::default();
    let (post_a, rx_a) = post();
    assert!(follow.target(Some(a.clone()), post_a).is_some());
    let (again, _) = post();
    assert_eq!(follow.target(Some(a.clone()), again), None, "same session");
    let (post_b, _rx_b) = post();
    assert!(follow.target(Some(b.clone()), post_b).is_some());
    // The first worker ended without posting.
    assert_eq!(rx_a.recv_timeout(WAIT), Err(RecvTimeoutError::Disconnected));
    let list = [info("job-1", "sleep 9", JobState::Running, None)];
    assert_eq!(follow.arrived(1, &list), None, "a list of session a");
    assert_eq!(follow.arrived(2, &list).map(|r| r.len()), Some(1));
    assert_eq!(follow.target(None, post().0), Some(Vec::new()));
    assert_eq!(follow.arrived(2, &list), None, "nothing shown");
}
