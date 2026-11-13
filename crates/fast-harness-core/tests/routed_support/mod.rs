//! Shared by the subagent tests: a transport routed by the first user
//! message, child logs and outcomes.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use fast_harness_core::http::{
    Flow, HttpError, HttpHead, HttpRequest, ScriptedResponse, ScriptedTransport, Transport,
};
use fast_harness_core::session;
use fast_harness_core::trace;
use serde_json::Value;

use crate::agent_support::Env;

pub const FIND_SUBAGENT: &str = r#"{"names":["subagent"]}"#;

/// Answers each request from the route whose needle the first user
/// message holds, after the route's delay; records every body.
pub struct Routed {
    pub routes: Vec<(&'static str, u64, ScriptedTransport)>,
    pub bodies: Mutex<Vec<String>>,
}

impl Routed {
    pub fn new(routes: Vec<(&'static str, u64, Vec<ScriptedResponse>)>) -> Arc<Routed> {
        let routes = routes
            .into_iter()
            .map(|(needle, delay, list)| (needle, delay, ScriptedTransport::new(list)))
            .collect();
        Arc::new(Routed {
            routes,
            bodies: Mutex::default(),
        })
    }
}

impl Transport for Routed {
    fn send(
        &self,
        req: &HttpRequest<'_>,
        on_chunk: &mut dyn FnMut(&HttpHead, &[u8]) -> Flow,
    ) -> Result<HttpHead, HttpError> {
        let body = String::from_utf8_lossy(req.body).into_owned();
        self.bodies.lock().unwrap().push(body.clone());
        let json: Value = serde_json::from_str(&body).unwrap();
        let first = json["messages"][0]["content"].to_string();
        let (_, delay, inner) = self
            .routes
            .iter()
            .find(|(needle, _, _)| first.contains(needle))
            .expect("a route for the request");
        std::thread::sleep(Duration::from_millis(*delay));
        inner.send(req, on_chunk)
    }
}

pub fn child_logs(env: &Env, root: &str) -> Vec<(trace::SessionHeader, Vec<trace::TraceEvent>)> {
    let sessions = session::list(&env.home, &env.cwd).unwrap();
    sessions
        .iter()
        .filter(|s| s.id != root)
        .map(|s| trace::read(&session::log_path(&env.home, &env.cwd, &s.id)).unwrap())
        .collect()
}

pub fn outcome(events: &[trace::TraceEvent]) -> String {
    let end = events.iter().rev().find(|e| e.kind == "turn/end").unwrap();
    end.data["outcome"].as_str().unwrap().to_string()
}

pub fn json(body: &str) -> Value {
    serde_json::from_str(body).unwrap()
}
