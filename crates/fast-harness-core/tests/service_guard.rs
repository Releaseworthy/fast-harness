//! The panic guard of the service's turn thread (PLAN.md D59): a panic in
//! the agent's build, the sink or the loop ends the turn `Failed` with an
//! error event, `done` runs once and the session works again.

mod agent_support;
mod service_support;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use agent_support::{echo_registry, env, key_auth, parts, text_reply, tool_reply};
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::config::Config;
use fast_harness_core::events::{AgentEvent, TurnOutcome};
use fast_harness_core::http::{
    Flow, HttpError, HttpHead, HttpRequest, ScriptedResponse, ScriptedTransport, Transport,
};
use fast_harness_core::service::{Finished, Host, Service, Sink, Start};
use service_support::watch;

/// The scripted transport, but its second request panics (mid-loop: after
/// the first step's tools). Tool panics do not get this far: the tool
/// runner turns them into error results.
struct Panicky {
    script: Arc<ScriptedTransport>,
    calls: AtomicUsize,
}

impl Transport for Panicky {
    fn send(
        &self,
        req: &HttpRequest<'_>,
        on_chunk: &mut dyn FnMut(&HttpHead, &[u8]) -> Flow,
    ) -> Result<HttpHead, HttpError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 1 {
            panic!("boom transport");
        }
        self.script.send(req, on_chunk)
    }
}

/// Agents with `echo`; the first build panics when asked to.
struct PanicHost {
    home: PathBuf,
    transport: Arc<Panicky>,
    build_panics: AtomicBool,
}

impl Host for PanicHost {
    fn home(&self) -> PathBuf {
        self.home.clone()
    }

    fn config(&self, _cwd: &Path) -> Result<Config, String> {
        Ok(Config::default())
    }

    fn agent(
        &self,
        cwd: &Path,
        _: &Conversation,
        c: &Config,
    ) -> Result<(Agent, Vec<String>), String> {
        if self.build_panics.swap(false, Ordering::SeqCst) {
            panic!("boom agent");
        }
        let mut parts = parts(&self.transport.script, key_auth(), cwd);
        parts.config = c.clone();
        parts.transport = self.transport.clone();
        Ok((Agent::new(parts, echo_registry()), Vec::new()))
    }
}

fn service(dir: &agent_support::Env, script: Vec<ScriptedResponse>, panics: bool) -> Service {
    Service::new(Arc::new(PanicHost {
        home: dir.home.clone(),
        transport: Arc::new(Panicky {
            script: Arc::new(ScriptedTransport::new(script)),
            calls: AtomicUsize::new(0),
        }),
        build_panics: AtomicBool::new(panics),
    }))
}

fn failed_with(end: &Finished, text: &str) {
    assert_eq!(end.outcome, TurnOutcome::Failed);
    let error = end.error.as_deref().unwrap_or_default();
    assert!(error.contains(text), "error: {error}");
    assert!(end.panicked);
}

/// The session runs a second turn to `Done` after the first one panicked.
fn works_again(service: &Service, session: &Arc<fast_harness_core::service::Session>) {
    assert!(!session.running());
    let next = watch(service, session, Start::Text("again".into()));
    let end = next.end();
    assert_eq!(end.outcome, TurnOutcome::Done, "{:?}", end.error);
    assert!(!session.running());
}

#[test]
fn a_panic_in_the_agent_build_ends_failed_and_the_session_works() {
    let dir = env("guard-build");
    let service = service(&dir, vec![text_reply("fine")], true);
    let session = service.create(&dir.cwd).unwrap();
    let turn = watch(&service, &session, Start::Text("hi".into()));
    let error = turn.until(|e| matches!(e, AgentEvent::Error(_)));
    assert!(matches!(&error, AgentEvent::Error(t) if t.contains("boom agent")));
    failed_with(&turn.end(), "boom agent");
    works_again(&service, &session);
}

#[test]
fn a_sink_that_panics_on_the_first_event_ends_failed() {
    let dir = env("guard-sink");
    let service = service(&dir, vec![text_reply("lost"), text_reply("fine")], false);
    let session = service.create(&dir.cwd).unwrap();
    let (tx, done) = std::sync::mpsc::channel();
    let sink: Sink = Box::new(|_| panic!("boom sink"));
    let end = Box::new(move |f: &Finished| tx.send(f.clone()).unwrap());
    service
        .start(&session, Start::Text("hi".into()), sink, end)
        .unwrap();
    let end = done.recv_timeout(std::time::Duration::from_secs(10));
    failed_with(&end.expect("done ran"), "boom sink");
    assert!(done.recv().is_err(), "done ran once");
    works_again(&service, &session);
}

#[test]
fn a_panic_mid_loop_ends_failed() {
    let dir = env("guard-loop");
    let call = (r#"c1"#, "echo", r#"{"text":"one"}"#);
    let service = service(&dir, vec![tool_reply(&[call]), text_reply("fine")], false);
    let session = service.create(&dir.cwd).unwrap();
    let turn = watch(&service, &session, Start::Text("hi".into()));
    turn.until(|e| matches!(e, AgentEvent::ToolResult { .. }));
    failed_with(&turn.end(), "boom transport");
    works_again(&service, &session);
}
