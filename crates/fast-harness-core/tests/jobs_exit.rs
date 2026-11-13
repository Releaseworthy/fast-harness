//! A background job dies with the process that started it (PLAN.md D68):
//! on Windows the Job Object of each job has
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, so when the process ends without
//! closing its sessions (a crash, `std::process::exit`) the OS kills the
//! job's whole tree. The test binary runs itself as the process that exits.

#![cfg(windows)]

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use fast_harness_core::tools::Jobs;

const HELPER_ENV: &str = "FH_JOB_EXIT_HELPER";

fn alive(pid: u32) -> bool {
    fast_harness_core::platform::process_start(pid).is_some()
}

/// The pid between `pid=` and the next `=` in `text`.
fn pid_in(text: &str) -> Option<u32> {
    text.split("pid=").nth(1)?.split('=').next()?.parse().ok()
}

/// Run by `a_job_dies_with_the_process` in a child process: starts a job
/// whose shell starts a long-lived `ping`, prints the ping's pid and exits
/// at once, with no destructor run (the jobs are not killed by the code).
#[test]
#[ignore = "a helper process of a_job_dies_with_the_process"]
fn exit_helper_starts_a_job_and_exits() {
    if std::env::var_os(HELPER_ENV).is_none() {
        return;
    }
    let jobs = Jobs::default();
    let command =
        "$p = Start-Process -PassThru -NoNewWindow ping -ArgumentList '-n','120','127.0.0.1'; \
         echo \"pid=$($p.Id)=\"; sleep 120";
    let id = jobs.start(command, &std::env::temp_dir(), None).unwrap();
    let mut out = String::new();
    let deadline = Instant::now() + Duration::from_secs(30);
    while pid_in(&out).is_none() && Instant::now() < deadline {
        let reading = jobs.read(&id, Duration::from_secs(1), false, None).unwrap();
        out.push_str(&reading.text);
    }
    let pid = pid_in(&out).expect("the job printed its grandchild's pid");
    assert!(alive(pid), "the grandchild runs before the exit");
    println!("grandchild pid={pid}=");
    std::process::exit(0);
}

#[test]
fn a_job_dies_with_the_process() {
    let exe = std::env::current_exe().unwrap();
    let mut helper = Command::new(exe)
        .args(["exit_helper_starts_a_job_and_exits", "--exact", "--ignored"])
        .args(["--nocapture", "--test-threads=1"])
        .env(HELPER_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // Only up to the pid line: a pipe end a leftover inherited would never
    // reach its end.
    let stdout = helper.stdout.take().unwrap();
    let pid = BufReader::new(stdout)
        .lines()
        .map_while(Result::ok)
        .find_map(|line| pid_in(&line))
        .expect("the helper printed the grandchild's pid");
    assert!(helper.wait().unwrap().success(), "the helper exits with 0");
    let deadline = Instant::now() + Duration::from_secs(5);
    while alive(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!alive(pid), "the grandchild {pid} outlived its process");
}
