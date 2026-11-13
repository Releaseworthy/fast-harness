//! Background shell jobs (PLAN.md D68): `shell` with `background: true`,
//! `job_output`, `job_kill`, `job_list`, the caps, the close of the session,
//! approvals and the find tool.

mod agent_support;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use agent_support::{env, Env};
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::config::approvals::{covered, subject, Allow};
use fast_harness_core::config::Config;
use fast_harness_core::service::{Host, Service};
use fast_harness_core::tools::{self, find as finder, Effect, JobState, ToolCtx, ToolSession};
use serde_json::json;

const SECOND: Duration = Duration::from_secs(1);

/// Runs tool `name` of the built-in registry in a session at `env`.
fn call(env: &Env, session: &ToolSession, name: &str, args: serde_json::Value) -> String {
    let cancel = AtomicBool::new(false);
    let ctx = ToolCtx::new(&env.cwd, &cancel).with_session(session);
    let out = tools::builtin().run(name, &args.to_string(), &ctx);
    out.text().into_owned()
}

fn tool_session(env: &Env) -> ToolSession {
    ToolSession::new(Some(env.home.join("s1.out")))
}

#[test]
fn a_job_that_prints_and_keeps_running_returns_its_id_at_once() {
    let env = env("jobs-start");
    let session = tool_session(&env);
    let started = Instant::now();
    let args = json!({"command": "echo ready; sleep 30", "background": true});
    let out = call(&env, &session, "shell", args);
    assert!(started.elapsed() < 10 * SECOND, "{:?}", started.elapsed());
    assert!(
        out.starts_with("started job-1 in the background\n"),
        "{out}"
    );
    assert!(out.contains("[job-1 is running"), "{out}");
    let list = call(&env, &session, "job_list", json!({}));
    assert!(list.starts_with("job-1  running  "), "{list}");
    assert!(list.ends_with("echo ready; sleep 30"), "{list}");
    let killed = call(&env, &session, "job_kill", json!({"id": "job-1"}));
    assert!(
        killed.contains("[job-1 was killed with its process tree]"),
        "{killed}"
    );
    let list = call(&env, &session, "job_list", json!({}));
    assert!(list.starts_with("job-1  killed  "), "{list}");
}

/// Reads until `needle` shows, at most 20 s; returns everything read.
fn read_until(jobs: &tools::Jobs, id: &str, needle: &str) -> String {
    let mut all = String::new();
    let deadline = Instant::now() + 20 * SECOND;
    while !all.contains(needle) && Instant::now() < deadline {
        all.push_str(&jobs.read(id, 5 * SECOND, false, None).unwrap().text);
    }
    all
}

#[test]
fn reads_return_only_the_output_since_the_last_read() {
    let env = env("jobs-incremental");
    let jobs = tools::Jobs::default();
    let id = jobs
        .start("echo one; sleep 2; echo two; sleep 30", &env.cwd, None)
        .unwrap();
    let first = read_until(&jobs, &id, "one");
    assert!(first.contains("one") && !first.contains("two"), "{first}");
    let second = read_until(&jobs, &id, "two");
    assert!(
        second.contains("two") && !second.contains("one"),
        "{second}"
    );
    let empty = jobs.read(&id, Duration::ZERO, false, None).unwrap();
    assert!(
        !empty.text.contains("one") && !empty.text.contains("two"),
        "{}",
        empty.text
    );
    assert_eq!(empty.info.state, JobState::Running);
    jobs.kill_all();
}

#[test]
fn waiting_for_the_end_gives_the_output_and_exit_code() {
    let env = env("jobs-end");
    let session = tool_session(&env);
    let args = json!({"command": "sleep 2; echo done; exit 3", "background": true});
    let out = call(&env, &session, "shell", args);
    assert!(out.contains("is running"), "{out}");
    let args = json!({"id": "job-1", "wait_ms": 30000, "until": "end"});
    let out = call(&env, &session, "job_output", args);
    assert!(out.contains("done"), "{out}");
    assert!(out.ends_with("[job-1 ended]\n[exit code: 3]"), "{out}");
    let info = &session.jobs().list()[0];
    assert_eq!((info.state, info.exit_code), (JobState::Exited, Some(3)));
    let list = call(&env, &session, "job_list", json!({}));
    assert!(list.starts_with("job-1  exited 3  "), "{list}");
    let bad = call(&env, &session, "job_output", json!({"id": "job-9"}));
    assert!(bad.contains("no background job `job-9`"), "{bad}");
}

#[cfg(windows)]
fn alive(pid: u32) -> bool {
    fast_harness_core::platform::process_start(pid).is_some()
}

#[cfg(windows)]
#[test]
fn a_kill_ends_the_child_of_the_child() {
    let env = env("jobs-tree");
    let jobs = tools::Jobs::default();
    let command =
        "$p = Start-Process -PassThru -NoNewWindow ping -ArgumentList '-n','60','127.0.0.1'; \
                   echo \"pid=$($p.Id)=\"; sleep 60";
    let id = jobs.start(command, &env.cwd, None).unwrap();
    let out = read_until(&jobs, &id, "=\n");
    let pid: u32 = out
        .split("pid=")
        .nth(1)
        .and_then(|r| r.split('=').next())
        .unwrap()
        .parse()
        .unwrap();
    assert!(alive(pid), "the grandchild runs");
    let info = jobs.kill(&id).unwrap();
    assert_eq!(info.state, JobState::Killed);
    let deadline = Instant::now() + 5 * SECOND;
    while alive(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!alive(pid), "the grandchild {pid} outlived the kill");
}

/// A host whose sessions never run a turn.
struct Idle(PathBuf);

impl Host for Idle {
    fn home(&self) -> PathBuf {
        self.0.clone()
    }

    fn config(&self, _: &Path) -> Result<Config, String> {
        Ok(Config::default())
    }

    fn agent(
        &self,
        _: &Path,
        _: &Conversation,
        _: &Config,
    ) -> Result<(Agent, Vec<String>), String> {
        Err("no turns here".into())
    }
}

#[test]
fn closing_the_session_kills_its_jobs() {
    let env = env("jobs-close");
    let service = Service::new(Arc::new(Idle(env.home.clone())));
    let session = service.create(&env.cwd).unwrap();
    let jobs = session.jobs();
    let shared = session.with_conv(|conv| Arc::ptr_eq(conv.tool_session.jobs(), &jobs));
    assert!(shared.unwrap(), "the view and the tools see the same jobs");
    let before = jobs.revision();
    jobs.start("sleep 30", &env.cwd, None).unwrap();
    assert!(jobs.wait_change(before, SECOND) > before);
    let running = jobs.revision();
    service.close(&session.id);
    assert_eq!(jobs.list()[0].state, JobState::Killed);
    assert!(
        jobs.wait_change(running, Duration::ZERO) > running,
        "the end is a change"
    );
}

#[test]
fn the_running_cap_and_the_output_cap_hold() {
    let env = env("jobs-caps");
    let jobs = tools::Jobs::default();
    for _ in 0..tools::jobs::MAX_RUNNING {
        jobs.start("sleep 30", &env.cwd, None).unwrap();
    }
    let over = jobs.start("sleep 30", &env.cwd, None).unwrap_err();
    assert!(over.contains("the cap"), "{over}");
    jobs.kill("job-1").unwrap();
    let id = jobs
        .start("'x' * 100000", &env.cwd, Some(&env.home))
        .unwrap();
    let reading = jobs.read(&id, 30 * SECOND, true, None).unwrap();
    assert!(reading.skipped > 0, "{}", reading.skipped);
    assert_eq!(reading.text.len(), tools::jobs::RING);
    let log = reading.info.log.clone().unwrap();
    assert!(log.starts_with(&env.home), "{}", log.display());
    let logged = std::fs::metadata(&log).unwrap().len();
    assert_eq!(logged, reading.info.bytes);
    assert!(tools::job_tools::render(&reading).contains("bytes left the buffer"));
    jobs.kill_all();
    assert!(jobs.list().iter().all(|job| job.state != JobState::Running));
}

#[test]
fn a_background_start_asks_like_shell_and_reads_never_ask() {
    let cwd = std::env::temp_dir();
    let raw = r#"{"command":"npm run dev","background":true}"#;
    let none = Allow::default();
    assert!(!covered(&none, "shell", Some(Effect::Run), raw, &cwd));
    let npm = Allow {
        commands: vec!["npm run".into()],
        ..Allow::default()
    };
    assert!(covered(&npm, "shell", Some(Effect::Run), raw, &cwd));
    let asked = subject("shell", raw, &cwd);
    assert_eq!(asked.summary, "run in the background: npm run dev");
    assert_ne!(
        asked.key,
        subject("shell", r#"{"command":"npm run dev"}"#, &cwd).key
    );
    let registry = tools::builtin();
    assert_eq!(registry.effect("job_output"), Some(Effect::Read));
    assert_eq!(registry.effect("job_list"), Some(Effect::Read));
    assert_eq!(registry.effect("job_kill"), Some(Effect::Run));
    assert!(!covered(
        &none,
        "job_kill",
        Some(Effect::Run),
        r#"{"id":"job-1"}"#,
        &cwd
    ));
}

#[test]
fn find_tools_finds_the_job_tools_and_they_wait_to_be_loaded() {
    let registry = tools::builtin();
    assert!(registry.finds());
    let others = &registry.specs()[1..];
    let ranked: Vec<&str> = finder::search("background job output", others)
        .iter()
        .map(|&i| others[i].name.as_str())
        .collect();
    assert_eq!(ranked[0], "job_output", "{ranked:?}");
    let out = finder::run(
        &json!({"names": ["job_output", "job_kill", "job_list"]}),
        others,
    );
    assert!(out
        .text()
        .starts_with("Loaded: job_output, job_kill, job_list\n"));
    let shell = others.iter().find(|spec| spec.name == "shell").unwrap();
    assert!(shell.description.contains("background: true"));
    assert!(!shell
        .description
        .contains("do not start background servers"));
}
