//! MCP, lazy (PLAN.md D47): servers of `mcp.json`, reached by an own client
//! over stdio and Streamable HTTP, protocol 2026-07-28 first with the
//! `initialize` handshake for older servers. No server starts with a
//! session: the tool index comes from `mcp-cache/`; a server starts on the
//! first call of one of its tools (refreshing a stale list first) and stops
//! after [`IDLE`] without calls (a reaper thread, plus a check on every
//! call) and when the last handle drops.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

pub mod cache;
pub mod config;
mod http;
pub mod rpc;
mod stdio;
pub mod wire;

pub use cache::{CacheFile, IndexTool};
pub use config::{Endpoint, ServerSpec};

use crate::http::Transport;
use crate::tools::{ToolCtx, ToolOutput};
use crate::types::now_ms;
use config::process_env;
use rpc::Era;

/// A server stops after this long without a call.
pub const IDLE: Duration = Duration::from_secs(600);
/// Most `tools/list` pages read in one refresh.
const MAX_PAGES: usize = 50;

/// One request's limits and extra headers.
pub struct Call<'a> {
    pub cancel: &'a AtomicBool,
    pub timeout: Duration,
    /// `Mcp-Param-*` headers (HTTP only).
    pub headers: Vec<(String, String)>,
}

/// A live connection to one server.
pub trait Conn: Send {
    fn request(&mut self, method: &str, params: Value, call: &Call<'_>) -> Result<Value, String>;
    fn era(&self) -> Era;
    /// False once a local server process has exited.
    fn alive(&mut self) -> bool;
}

/// The MCP servers of a settings load; clones share the live servers.
#[derive(Clone, Default)]
pub struct McpServers(Arc<Shared>);

#[derive(Default)]
struct Shared {
    home: PathBuf,
    transport: Option<Arc<dyn Transport>>,
    idle: Duration,
    servers: Vec<Server>,
    reaper: AtomicBool,
}

struct Server {
    spec: ServerSpec,
    live: Mutex<Option<Live>>,
    cache: Mutex<Option<CacheFile>>,
}

struct Live {
    conn: Box<dyn Conn>,
    last: Instant,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl McpServers {
    /// The servers of `specs` with their caches read from `home`; nothing
    /// starts. HTTP servers need `transport`.
    pub fn new(home: &Path, specs: Vec<ServerSpec>, transport: Option<Arc<dyn Transport>>) -> Self {
        McpServers::with_idle(home, specs, transport, IDLE)
    }

    /// [`McpServers::new`] whose servers stop after `idle` (tests).
    pub fn with_idle(
        home: &Path,
        specs: Vec<ServerSpec>,
        transport: Option<Arc<dyn Transport>>,
        idle: Duration,
    ) -> Self {
        let servers = (specs.into_iter())
            .map(|spec| Server {
                cache: Mutex::new(cache::read(home, &spec)),
                spec,
                live: Mutex::new(None),
            })
            .collect();
        McpServers(Arc::new(Shared {
            home: home.to_path_buf(),
            transport,
            idle,
            servers,
            reaper: AtomicBool::new(false),
        }))
    }

    pub fn is_empty(&self) -> bool {
        self.0.servers.is_empty()
    }

    pub fn specs(&self) -> Vec<ServerSpec> {
        self.0
            .servers
            .iter()
            .map(|server| server.spec.clone())
            .collect()
    }

    /// The cache of server `name` as it is now.
    pub fn cache_of(&self, name: &str) -> Option<CacheFile> {
        self.server(name)
            .and_then(|server| lock(&server.cache).clone())
    }

    /// The tools of every enabled server's cache, servers in name order.
    pub fn index(&self) -> Vec<IndexTool> {
        (self.0.servers.iter())
            .filter(|server| server.spec.enabled)
            .filter_map(|server| lock(&server.cache).as_ref().map(cache::index_of))
            .flatten()
            .collect()
    }

    /// The index entry of the model-facing `name`.
    pub fn lookup(&self, name: &str) -> Option<IndexTool> {
        self.index().into_iter().find(|tool| tool.name == name)
    }

    /// True while server `name` has a live connection.
    pub fn running(&self, name: &str) -> bool {
        self.server(name)
            .is_some_and(|server| lock(&server.live).is_some())
    }

    fn server(&self, name: &str) -> Option<&Server> {
        self.0
            .servers
            .iter()
            .find(|server| server.spec.name == name)
    }

    /// Calls `tool` of `server` with `args`, starting the server first and
    /// refreshing its list when the cache is past its `ttlMs`.
    pub fn call(&self, server: &str, tool: &str, args: &Value, ctx: &ToolCtx<'_>) -> ToolOutput {
        self.0.reap();
        let Some(entry) = self.server(server).filter(|s| s.spec.enabled) else {
            return ToolOutput::err(format!(
                "MCP server `{server}` is not configured or disabled"
            ));
        };
        let result = self.0.with_live(entry, ctx.cancel, |conn, call, cache| {
            let schema = (cache.and_then(|file| file.tool(tool)))
                .and_then(|raw| raw.get("inputSchema"))
                .ok_or_else(|| format!("the server no longer lists `{tool}`"))?;
            call.headers = http::param_headers(schema, args);
            let params = json!({"name": tool, "arguments": args});
            conn.request("tools/call", params, call)
        });
        self.start_reaper();
        match result {
            Ok(value) => rpc::tool_output(&value),
            Err(err) => ToolOutput::err(format!("MCP server `{server}`: {err}")),
        }
    }

    /// Lists the tools of server `name` now and saves its cache; the number
    /// of tools.
    pub fn refresh(&self, name: &str) -> Result<usize, String> {
        let server = self
            .server(name)
            .ok_or_else(|| format!("no MCP server `{name}`"))?;
        let cancel = AtomicBool::new(false);
        let (shared, spec) = (&self.0, &server.spec);
        let file = shared.with_live_raw(server, &cancel, |conn, call| {
            let file = list(conn, spec, call)?;
            cache::write(&shared.home, &file)?;
            Ok(file)
        })?;
        let count = file.tools.len();
        *lock(&server.cache) = Some(file);
        Ok(count)
    }

    /// Stops every server now (the end of a refresh, tests).
    pub fn stop_all(&self) {
        for server in &self.0.servers {
            lock(&server.live).take();
        }
    }

    fn start_reaper(&self) {
        if self.0.reaper.swap(true, Ordering::SeqCst) {
            return;
        }
        let weak = Arc::downgrade(&self.0);
        let tick = (self.0.idle / 2).clamp(Duration::from_millis(10), Duration::from_secs(30));
        std::thread::spawn(move || loop {
            std::thread::sleep(tick);
            match weak.upgrade() {
                Some(shared) => shared.reap(),
                None => return,
            }
        });
    }
}

impl Shared {
    /// Stops the servers idle for at least `idle`; a busy one is in use.
    fn reap(&self) {
        for server in &self.servers {
            let Ok(mut live) = server.live.try_lock() else {
                continue;
            };
            if live.as_ref().is_some_and(|l| l.last.elapsed() >= self.idle) {
                live.take();
            }
        }
    }

    fn open(&self, spec: &ServerSpec, call: &Call<'_>) -> Result<Box<dyn Conn>, String> {
        match &spec.endpoint {
            Endpoint::Stdio { .. } => Ok(Box::new(stdio::StdioConn::open(
                spec,
                &process_env,
                call.cancel,
            )?)),
            Endpoint::Http { .. } => {
                let transport = self.transport.clone().ok_or("no HTTP transport")?;
                Ok(Box::new(http::HttpConn::open(
                    spec,
                    transport,
                    &process_env,
                    call,
                )?))
            }
        }
    }

    /// Runs `work` on the live connection of `server` (started when needed);
    /// a connection whose process died is dropped.
    fn with_live_raw<T>(
        &self,
        server: &Server,
        cancel: &AtomicBool,
        work: impl FnOnce(&mut dyn Conn, &mut Call<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        let timeout = Duration::from_millis(server.spec.timeout_ms);
        let mut call = Call {
            cancel,
            timeout,
            headers: Vec::new(),
        };
        let mut live = lock(&server.live);
        if !live.as_mut().is_some_and(|l| l.conn.alive()) {
            let conn = self.open(&server.spec, &call)?;
            *live = Some(Live {
                conn,
                last: Instant::now(),
            });
        }
        let Some(current) = live.as_mut() else {
            return Err("no connection".into());
        };
        let result = work(current.conn.as_mut(), &mut call);
        current.last = Instant::now();
        if result.is_err() && !current.conn.alive() {
            live.take();
        }
        result
    }

    /// [`Shared::with_live_raw`] after a refresh of a missing or stale
    /// cache; `work` sees the cache.
    fn with_live(
        &self,
        server: &Server,
        cancel: &AtomicBool,
        work: impl FnOnce(&mut dyn Conn, &mut Call<'_>, Option<&CacheFile>) -> Result<Value, String>,
    ) -> Result<Value, String> {
        let spec = &server.spec;
        self.with_live_raw(server, cancel, |conn, call| {
            let stale = !lock(&server.cache)
                .as_ref()
                .is_some_and(|f| f.fresh(now_ms()));
            if stale {
                let file = list(conn, spec, call)?;
                cache::write(&self.home, &file)?;
                *lock(&server.cache) = Some(file);
            }
            // A copy: the lock is not held while the tool runs.
            let snapshot = lock(&server.cache).clone();
            work(conn, call, snapshot.as_ref())
        })
    }
}

/// Every page of `tools/list` as a cache file.
fn list(conn: &mut dyn Conn, spec: &ServerSpec, call: &Call<'_>) -> Result<CacheFile, String> {
    let mut file = CacheFile {
        version: 1,
        server: spec.name.clone(),
        fingerprint: spec.fingerprint(),
        fetched_at: now_ms(),
        era: conn.era().word().into(),
        ..CacheFile::default()
    };
    let mut cursor: Option<String> = None;
    for page in 0..MAX_PAGES {
        let params = match &cursor {
            Some(cursor) => json!({"cursor": cursor}),
            None => json!({}),
        };
        let result = conn.request("tools/list", params, call)?;
        if page == 0 {
            file.ttl_ms = result.get("ttlMs").and_then(Value::as_u64);
            file.cache_scope = result
                .get("cacheScope")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        let tools = result.get("tools").and_then(Value::as_array);
        file.tools.extend(tools.into_iter().flatten().cloned());
        cursor = result
            .get("nextCursor")
            .and_then(Value::as_str)
            .map(str::to_string);
        if cursor.is_none() {
            break;
        }
    }
    Ok(file)
}
