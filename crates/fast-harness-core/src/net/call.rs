//! A call with a cancel flag (PLAN.md D66): the request runs on its own
//! thread while the calling thread hands over the body chunks and rechecks
//! the flag every [`POLL_MS`]. On cancel the calling thread returns at once
//! and shuts down the call's sockets, so a read blocked in the call thread
//! fails and the thread ends. The two threads go in lockstep (each chunk
//! waits for the caller's `Flow`), so `Flow::Stop` ends the read exactly as
//! a call without a flag does.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use ureq::Agent;

use super::watch::{self, CallCtx};
use crate::http::{Flow, HttpError, HttpErrorKind, HttpHead, HttpRequest};

/// How often the calling thread rechecks the cancel flag: 20 wakeups a
/// second during a silent wait, a cancel seen within 50 ms (D66 bound 500).
pub const POLL_MS: u64 = 50;

static LIVE: AtomicUsize = AtomicUsize::new(0);

/// Call threads still running (tests check that none outlives its call).
pub fn live_calls() -> usize {
    LIVE.load(Ordering::SeqCst)
}

/// Counts one call thread from its start to its end, even on a panic.
struct Live;

impl Live {
    fn enter() -> Live {
        LIVE.fetch_add(1, Ordering::SeqCst);
        Live
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The request copied for the call thread.
struct Owned {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    timeout_ms: u32,
}

impl Owned {
    fn new(req: &HttpRequest<'_>) -> Owned {
        Owned {
            method: req.method.to_string(),
            url: req.url.to_string(),
            headers: req
                .headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: req.body.to_vec(),
            timeout_ms: req.timeout_ms,
        }
    }

    /// The same request, without the flag (the caller watches it).
    fn send(&self, agent: &Agent, on_chunk: &mut dyn FnMut(&HttpHead, &[u8]) -> Flow) -> Done {
        let headers: Vec<(&str, &str)> = self
            .headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let req = HttpRequest::new(&self.method, &self.url, &headers, &self.body)
            .with_timeout_ms(self.timeout_ms);
        super::send_inline(agent, &req, on_chunk)
    }
}

type Done = Result<HttpHead, HttpError>;

enum Msg {
    Head(HttpHead),
    Chunk(Vec<u8>),
    End(Done),
}

/// Sends `req` on a call thread; `cancel` ends it in any state.
pub(super) fn send(
    agent: &Agent,
    req: &HttpRequest<'_>,
    cancel: &AtomicBool,
    on_chunk: &mut dyn FnMut(&HttpHead, &[u8]) -> Flow,
) -> Done {
    if cancel.load(Ordering::Relaxed) {
        return Err(cancelled());
    }
    let ctx = Arc::new(CallCtx::default());
    let (to_caller, from_call) = mpsc::channel();
    let (to_call, flows) = mpsc::channel();
    let (agent_copy, owned, call_ctx) = (agent.clone(), Owned::new(req), ctx.clone());
    let spawned = thread::Builder::new()
        .name("fast-harness-http".into())
        .spawn(move || run_call(&agent_copy, &owned, call_ctx, &to_caller, &flows));
    if spawned.is_err() {
        // No thread: the call still runs, cancelled only between chunks.
        return super::send_inline(agent, req, on_chunk);
    }
    let result = pump(&from_call, &to_call, cancel, on_chunk);
    if result.is_none() {
        // The call thread ends once its socket is shut down; it is not
        // joined, so the caller returns now even while it connects.
        ctx.stop();
    }
    result.unwrap_or_else(|| Err(cancelled()))
}

/// The call thread: runs the request and forwards head, chunks and result.
fn run_call(
    agent: &Agent,
    owned: &Owned,
    ctx: Arc<CallCtx>,
    to_caller: &Sender<Msg>,
    flows: &Receiver<Flow>,
) {
    let _live = Live::enter();
    watch::enter(ctx);
    let mut head_sent = false;
    let result = owned.send(agent, &mut |head, chunk| {
        if !head_sent {
            head_sent = to_caller.send(Msg::Head(head.clone())).is_ok();
        }
        if to_caller.send(Msg::Chunk(chunk.to_vec())).is_err() {
            return Flow::Stop;
        }
        flows.recv().unwrap_or(Flow::Stop)
    });
    let _ = to_caller.send(Msg::End(result));
}

/// The caller's side; `None` when `cancel` was set first.
fn pump(
    from_call: &Receiver<Msg>,
    to_call: &Sender<Flow>,
    cancel: &AtomicBool,
    on_chunk: &mut dyn FnMut(&HttpHead, &[u8]) -> Flow,
) -> Option<Done> {
    let mut head = HttpHead::default();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        match from_call.recv_timeout(Duration::from_millis(POLL_MS)) {
            Ok(Msg::Head(got)) => head = got,
            Ok(Msg::Chunk(chunk)) => {
                let flow = on_chunk(&head, &chunk);
                // A closed channel means the call thread already ended.
                let _ = to_call.send(flow);
            }
            Ok(Msg::End(done)) => return Some(done),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                let lost = HttpError::new(HttpErrorKind::Io, "the call thread ended early");
                return Some(Err(lost));
            }
        }
    }
}

fn cancelled() -> HttpError {
    HttpError::new(HttpErrorKind::Cancelled, "cancelled")
}
