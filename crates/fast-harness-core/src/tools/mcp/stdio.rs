//! An MCP server as a local process (PLAN.md D47): one JSON-RPC message per
//! line on stdin and stdout, stderr dropped, the process tree in one
//! `ProcessGroup` killed when the connection drops. Lines that are not JSON
//! are skipped. The first request probes with `server/discover`; any error
//! that is not a modern one, or a silence, falls back to `initialize`
//! (<https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio>).

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::config::{expand, Endpoint, ServerSpec};
use super::rpc::{self, Era, Reply};
use super::{Call, Conn};
use crate::platform::ProcessGroup;
use crate::tools::command::program;

/// Longest wait for the `server/discover` probe before falling back.
const PROBE_MS: u64 = 10_000;
const TICK: Duration = Duration::from_millis(50);

pub struct StdioConn {
    group: ProcessGroup,
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Value>,
    next: u64,
    era: Era,
}

enum WaitErr {
    Timeout,
    Fail(String),
}

impl StdioConn {
    /// Starts the server of `spec` and finds its protocol era.
    pub fn open(
        spec: &ServerSpec,
        env: &dyn Fn(&str) -> Option<String>,
        cancel: &AtomicBool,
    ) -> Result<StdioConn, String> {
        let Endpoint::Stdio {
            command,
            args,
            env: vars,
            dir,
        } = &spec.endpoint
        else {
            return Err("not a stdio server".into());
        };
        let mut cmd = Command::new(program(command, dir));
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for (name, value) in vars {
            cmd.env(name, expand(value, env)?);
        }
        let (group, mut child) = ProcessGroup::spawn(&mut cmd)
            .map_err(|err| format!("cannot start `{command}`: {err}"))?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = group.kill();
            return Err("no stdin or stdout pipe".into());
        };
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || read_lines(stdout, tx));
        let mut conn = StdioConn {
            group,
            child,
            stdin,
            rx,
            next: 1,
            era: Era::Modern,
        };
        conn.era = conn.probe(cancel, spec.timeout_ms)?;
        Ok(conn)
    }

    fn probe(&mut self, cancel: &AtomicBool, timeout_ms: u64) -> Result<Era, String> {
        let id = self.id();
        self.send(&rpc::request(id, "server/discover", json!({}), Era::Modern))?;
        let probe = Duration::from_millis(PROBE_MS.min(timeout_ms));
        let reply = match self.wait(id, cancel, probe) {
            Ok(reply) => Some(reply),
            Err(WaitErr::Timeout) => None,
            Err(WaitErr::Fail(err)) => return Err(err),
        };
        let era = rpc::probe_era(reply.as_ref());
        if era == Era::Legacy {
            let id = self.id();
            self.send(&rpc::initialize(id))?;
            let timeout = Duration::from_millis(timeout_ms);
            let reply = self.wait(id, cancel, timeout).map_err(wait_text)?;
            rpc::result(reply).map_err(|err| format!("initialize: {err}"))?;
            self.send(&rpc::notification("notifications/initialized", json!({})))?;
        }
        Ok(era)
    }

    fn id(&mut self) -> u64 {
        self.next += 1;
        self.next - 1
    }

    fn send(&mut self, msg: &Value) -> Result<(), String> {
        let mut line = msg.to_string();
        line.push('\n');
        (self.stdin.write_all(line.as_bytes()))
            .and_then(|()| self.stdin.flush())
            .map_err(|err| format!("the server's stdin closed: {err}"))
    }

    /// The reply to `id`; the server's own requests are refused and its
    /// notifications skipped meanwhile.
    fn wait(&mut self, id: u64, cancel: &AtomicBool, limit: Duration) -> Result<Reply, WaitErr> {
        let deadline = Instant::now() + limit;
        loop {
            if cancel.load(Ordering::Relaxed) {
                let params = json!({"requestId": id, "reason": "cancelled"});
                let _ = self.send(&rpc::notification("notifications/cancelled", params));
                return Err(WaitErr::Fail("cancelled".into()));
            }
            if Instant::now() >= deadline {
                return Err(WaitErr::Timeout);
            }
            let msg = match self.rx.recv_timeout(TICK) {
                Ok(msg) => msg,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(WaitErr::Fail("the server exited".into()))
                }
            };
            if let Some(reply) = rpc::reply_to(&msg, id) {
                return Ok(reply);
            }
            if let Some(answer) = rpc::refuse(&msg) {
                let _ = self.send(&answer);
            }
        }
    }
}

fn wait_text(err: WaitErr) -> String {
    match err {
        WaitErr::Timeout => "the server did not answer in time".into(),
        WaitErr::Fail(err) => err,
    }
}

fn read_lines(stdout: ChildStdout, tx: Sender<Value>) {
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else {
            return;
        };
        if let Ok(msg) = serde_json::from_str::<Value>(line.trim()) {
            if msg.is_object() && tx.send(msg).is_err() {
                return;
            }
        }
    }
}

impl Conn for StdioConn {
    fn request(&mut self, method: &str, params: Value, call: &Call<'_>) -> Result<Value, String> {
        let id = self.id();
        self.send(&rpc::request(id, method, params, self.era))?;
        let reply = self
            .wait(id, call.cancel, call.timeout)
            .map_err(wait_text)?;
        rpc::result(reply)
    }

    fn era(&self) -> Era {
        self.era
    }

    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for StdioConn {
    fn drop(&mut self) {
        let _ = self.group.kill();
        let _ = self.child.wait();
    }
}
