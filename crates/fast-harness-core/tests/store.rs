//! `store` (PLAN.md D55): whole-file writes, the lock, read-change-write
//! across threads and processes, and the session lock taken by two
//! processes at once. Cross-process tests re-run this binary as children:
//! a child test returns at once unless [`ROLE`] names it.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

use fast_harness_core::session::lock::Held;
use fast_harness_core::{config, platform, store};

const ROLE: &str = "FH_STORE_TEST_ROLE";
const TARGET: &str = "FH_STORE_TEST_TARGET";
const INDEX: &str = "FH_STORE_TEST_INDEX";
const GO: &str = "FH_STORE_TEST_GO";
const LONG: Duration = Duration::from_secs(120);

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let path = std::env::temp_dir().join(format!(
            "fast-harness-test-store-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = (fs::read_dir(dir).unwrap())
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn counter(bytes: Option<&[u8]>) -> u64 {
    let text = bytes.map(String::from_utf8_lossy).unwrap_or_default();
    text.trim().parse().unwrap_or(0)
}

fn increment(path: &Path) {
    store::update(path, LONG, |current| {
        let next = counter(current.as_deref()) + 1;
        Ok((next.to_string().into_bytes(), ()))
    })
    .unwrap();
}

/// This binary again, running only `test` with `role` and `vars`.
fn child(test: &str, vars: &[(&str, String)]) -> std::process::Child {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args(["--exact", test, "--nocapture", "--test-threads=1"]);
    cmd.env(ROLE, test);
    for (key, value) in vars {
        cmd.env(key, value);
    }
    (cmd.stdout(Stdio::piped()).stderr(Stdio::piped()))
        .spawn()
        .unwrap()
}

fn finished(children: Vec<std::process::Child>) -> Vec<Output> {
    let outputs: Vec<Output> = (children.into_iter())
        .map(|child| child.wait_with_output().unwrap())
        .collect();
    for output in &outputs {
        let err = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "child failed: {err}");
    }
    outputs
}

fn is_child(test: &str) -> bool {
    std::env::var(ROLE).is_ok_and(|role| role == test)
}

#[test]
fn write_atomic_leaves_one_whole_content_and_no_temp_file() {
    let dir = TempDir::new("atomic");
    let path = dir.0.join("file.json");
    let contents: Vec<Vec<u8>> = (0..8)
        .map(|i| {
            format!(
                "{{\"writer\":{i},\"pad\":\"{}\"}}",
                "x".repeat(2000 * (i + 1))
            )
            .into()
        })
        .collect();
    std::thread::scope(|scope| {
        for content in &contents {
            let (path, contents) = (&path, &contents);
            scope.spawn(move || {
                for _ in 0..50 {
                    store::write_atomic(path, content).unwrap();
                    let now = fs::read(path).unwrap();
                    assert!(contents.contains(&now), "a torn file");
                }
            });
        }
    });
    assert!(contents.contains(&fs::read(&path).unwrap()));
    assert_eq!(names(&dir.0), ["file.json"]);
}

/// Fails with the lock removed (`f` called directly): the threads lose counts.
#[test]
fn locked_read_change_write_loses_no_count() {
    let dir = TempDir::new("locked");
    let path = dir.0.join("counter");
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                for _ in 0..200 {
                    store::locked(&path, LONG, || {
                        let n = counter(fs::read(&path).ok().as_deref());
                        fs::write(&path, (n + 1).to_string()).unwrap();
                    })
                    .unwrap();
                }
            });
        }
    });
    assert_eq!(counter(Some(&fs::read(&path).unwrap())), 1600);
    assert_eq!(names(&dir.0), ["counter"], "the lock is gone");
}

#[test]
fn update_across_processes_loses_no_count() {
    let dir = TempDir::new("processes");
    let path = dir.0.join("counter");
    let target = path.to_string_lossy().into_owned();
    let children = (0..4)
        .map(|_| child("child_update", &[(TARGET, target.clone())]))
        .collect();
    finished(children);
    assert_eq!(counter(Some(&fs::read(&path).unwrap())), 400);
}

#[test]
fn child_update() {
    if !is_child("child_update") {
        return;
    }
    let path = PathBuf::from(std::env::var(TARGET).unwrap());
    for _ in 0..100 {
        increment(&path);
    }
}

#[test]
fn save_setting_across_processes_keeps_every_key() {
    let home = TempDir::new("settings");
    let target = home.0.to_string_lossy().into_owned();
    let children = (0..4)
        .map(|i| {
            child(
                "child_settings",
                &[(TARGET, target.clone()), (INDEX, i.to_string())],
            )
        })
        .collect();
    finished(children);
    let text = fs::read_to_string(home.0.join(config::FILE_NAME)).unwrap();
    let root: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(root.as_object().unwrap().len(), 80);
    assert_eq!(root["k3_19"], 19);
}

#[test]
fn child_settings() {
    if !is_child("child_settings") {
        return;
    }
    let home = PathBuf::from(std::env::var(TARGET).unwrap());
    let index = std::env::var(INDEX).unwrap();
    for n in 0..20 {
        config::save_setting(&home, &format!("k{index}_{n}"), n.into()).unwrap();
    }
}

fn age(path: &Path, by: Duration) {
    let file = fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() - by).unwrap();
}

#[test]
fn a_lock_of_a_gone_process_or_unreadable_and_old_is_taken_over() {
    let dir = TempDir::new("stale");
    let path = dir.0.join("file.json");
    let lock = store::lock_path(&path);
    let dead = (1..u32::MAX / 4)
        .map(|n| u32::MAX - 4 * n)
        .find(|pid| platform::process_start(*pid).is_none())
        .unwrap();
    fs::write(&lock, format!("{{\"pid\":{dead},\"started\":1}}")).unwrap();
    // Old enough for platforms without a liveness backend too.
    age(&lock, Duration::from_secs(1200));
    assert_eq!(store::locked(&path, LONG, || 7).unwrap(), 7);
    assert!(!lock.exists());
    fs::write(&lock, "half").unwrap();
    age(&lock, Duration::from_secs(30));
    assert_eq!(store::locked(&path, LONG, || 8).unwrap(), 8);
    assert!(!lock.exists());
}

#[test]
fn a_lock_held_by_a_live_thread_times_out_and_a_panic_frees_it() {
    let dir = TempDir::new("held");
    let path = dir.0.join("file.json");
    let (taken, wait_taken) = mpsc::channel();
    let (release, wait_release) = mpsc::channel::<()>();
    let holder = std::thread::spawn({
        let path = path.clone();
        move || {
            store::locked(&path, LONG, || {
                taken.send(()).unwrap();
                wait_release.recv().unwrap();
            })
        }
    });
    wait_taken.recv().unwrap();
    let err = store::locked(&path, Duration::from_millis(100), || ()).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::TimedOut);
    let text = err.to_string();
    assert!(
        text.contains(&format!("pid {}", std::process::id())),
        "{text}"
    );
    assert!(text.contains("file.json.lock"), "{text}");
    release.send(()).unwrap();
    holder.join().unwrap().unwrap();
    let panicked = std::panic::catch_unwind(|| store::locked(&path, LONG, || panic!("inside")));
    assert!(panicked.is_err());
    assert!(
        !store::lock_path(&path).exists(),
        "the guard removed the lock"
    );
    assert_eq!(store::locked(&path, Duration::ZERO, || 1).unwrap(), 1);
}

#[test]
fn two_processes_taking_one_session_lock_one_wins() {
    let dir = TempDir::new("session");
    let log = dir.0.join("s1.jsonl");
    let go = dir.0.join("go");
    let vars = [
        (TARGET, log.to_string_lossy().into_owned()),
        (GO, go.to_string_lossy().into_owned()),
    ];
    let children = vec![child("child_take", &vars), child("child_take", &vars)];
    fs::write(&go, "").unwrap();
    let outputs = finished(children);
    // libtest prints `test child_take ... ` before the child's own line.
    let results: Vec<String> = (outputs.iter())
        .filter_map(|out| {
            let text = String::from_utf8_lossy(&out.stdout).into_owned();
            let at = text.find("TAKE=")?;
            text[at..].split_whitespace().next().map(str::to_string)
        })
        .collect();
    let won = results.iter().filter(|r| *r == "TAKE=ok").count();
    let lost = results
        .iter()
        .filter(|r| *r == "TAKE=AlreadyExists")
        .count();
    assert_eq!((won, lost), (1, 1), "{results:?}");
    let held = Held::take(&log, "s1", "x", 1).unwrap();
    drop(held);
}

#[test]
fn child_take() {
    if !is_child("child_take") {
        return;
    }
    let log = PathBuf::from(std::env::var(TARGET).unwrap());
    let go = PathBuf::from(std::env::var(GO).unwrap());
    let start = std::time::Instant::now();
    while !go.exists() && start.elapsed() < LONG {
        std::thread::sleep(Duration::from_millis(2));
    }
    let taken = Held::take(&log, "s1", "x", 1);
    let dir = log.parent().unwrap();
    fs::write(dir.join(format!("tried-{}", std::process::id())), "").unwrap();
    match taken {
        Ok(held) => {
            println!("TAKE=ok");
            // Held until the other child tried too.
            let tried = || {
                names(dir)
                    .iter()
                    .filter(|n| n.starts_with("tried-"))
                    .count()
            };
            while tried() < 2 && start.elapsed() < LONG {
                std::thread::sleep(Duration::from_millis(5));
            }
            drop(held);
        }
        Err(err) => println!("TAKE={:?}", err.kind()),
    }
}

/// Before the fix the error cost the whole wait and read "held by an unknown
/// process": `create_dir_all` failing (the parent is a file) counted as busy.
#[test]
fn a_lock_whose_parent_is_a_file_fails_at_once_with_its_cause() {
    let dir = TempDir::new("parent-file");
    let file = dir.0.join("config.json");
    fs::write(&file, "{}").unwrap();
    let path = file.join("accounts.json");
    let started = std::time::Instant::now();
    let err = store::locked(&path, LONG, || ()).unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(1), "{err}");
    assert!(!err.to_string().contains("held by"), "{err}");
    let started = std::time::Instant::now();
    let err = store::update(&path, LONG, |_| Ok((b"x".to_vec(), ()))).unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(1), "{err}");
    assert!(!err.to_string().contains("held by"), "{err}");
}

/// A lock without a start time (`started` 0, a platform that cannot tell)
/// is held for 10 minutes, then stale whoever has the pid now: before, a
/// reused live pid kept it for ever (Linux) and Windows took it at once.
#[test]
fn a_lock_without_a_start_time_is_stale_after_ten_minutes() {
    let me = std::process::id();
    assert!(!store::stale(Some((me, 0)), Duration::from_secs(5)));
    assert!(!store::stale(Some((me, 0)), Duration::from_secs(590)));
    assert!(store::stale(Some((me, 0)), Duration::from_secs(610)));
}
