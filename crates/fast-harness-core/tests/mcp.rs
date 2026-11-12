//! MCP, lazy (PLAN.md D47). The stdio server is this test binary itself:
//! `fake_mcp_server` acts as an MCP server when `FH_FAKE_MCP` names an era
//! (`modern` or `legacy`) and passes as a no-op otherwise.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use fast_harness_core::config::layers::Layers;
use fast_harness_core::http::{ScriptedResponse, ScriptedTransport, Transport};
use fast_harness_core::tools::mcp::config::load;
use fast_harness_core::tools::mcp::{cache, McpServers, ServerSpec};
use fast_harness_core::tools::{Extensions, ToolCtx};
use serde_json::{json, Value};

#[test]
fn fake_mcp_server() {
    let Ok(era) = std::env::var("FH_FAKE_MCP") else {
        return;
    };
    let log = std::env::var("FH_FAKE_LOG").unwrap_or_default();
    let ttl: u64 = (std::env::var("FH_FAKE_TTL").ok())
        .and_then(|t| t.parse().ok())
        .unwrap_or(3_600_000);
    let mut out = std::io::stdout();
    for line in std::io::stdin().lock().lines() {
        let Ok(msg) = serde_json::from_str::<Value>(&line.unwrap_or_default()) else {
            continue;
        };
        let method = msg["method"].as_str().unwrap_or("").to_string();
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
            .unwrap();
        writeln!(file, "{method}").unwrap();
        let Some(id) = msg.get("id").cloned() else {
            continue;
        };
        let body = fake_answer(&era, &method, &msg, ttl);
        let note = json!({"jsonrpc": "2.0", "method": "notifications/message", "params": {}});
        writeln!(out, "not json\n{note}").unwrap();
        let mut reply = json!({"jsonrpc": "2.0", "id": id});
        reply[body.0] = body.1;
        writeln!(out, "{reply}").unwrap();
        out.flush().unwrap();
    }
    std::process::exit(0);
}

fn fake_answer(era: &str, method: &str, msg: &Value, ttl: u64) -> (&'static str, Value) {
    let missing = json!({"code": -32601, "message": "Method not found"});
    let meta = msg["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"].as_str();
    if era == "modern" && meta != Some("2026-07-28") {
        return (
            "error",
            json!({"code": -32022, "message": "no version", "data": {"supported": ["2026-07-28"]}}),
        );
    }
    let echo = json!({"name": "echo", "description": "Echo the text back", "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}}});
    match (era, method) {
        ("modern", "server/discover") => (
            "result",
            json!({"resultType": "complete", "supportedVersions": ["2026-07-28"], "capabilities": {"tools": {}}}),
        ),
        ("legacy", "initialize") => (
            "result",
            json!({"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "fake", "version": "1"}}),
        ),
        ("modern", "tools/list") => (
            "result",
            json!({"tools": [echo], "ttlMs": ttl, "cacheScope": "private"}),
        ),
        ("legacy", "tools/list") => ("result", json!({"tools": [echo]})),
        (_, "tools/call") => {
            let text = msg["params"]["arguments"]["text"].as_str().unwrap_or("");
            (
                "result",
                json!({"content": [{"type": "text", "text": format!("echo: {text}")}]}),
            )
        }
        _ => ("error", missing),
    }
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fh-mcp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Writes `mcp.json` naming the fake server and loads it.
fn fake_spec(home: &Path, era: &str, ttl: u64) -> Vec<ServerSpec> {
    let exe = std::env::current_exe().unwrap();
    let file = json!({"servers": {"fake": {
        "command": exe.to_string_lossy(),
        "args": ["fake_mcp_server", "--exact", "--nocapture", "--test-threads=1"],
        "env": {"FH_FAKE_MCP": era, "FH_FAKE_LOG": home.join("log.txt").to_string_lossy(), "FH_FAKE_TTL": ttl.to_string()}
    }}});
    std::fs::write(home.join("mcp.json"), file.to_string()).unwrap();
    load(&Layers::user_only(home)).0
}

fn log(home: &Path) -> String {
    std::fs::read_to_string(home.join("log.txt")).unwrap_or_default()
}

fn call(servers: &McpServers, name: &str, args: &str, cwd: &Path) -> String {
    let ext = Extensions::default().with_mcp(servers.clone());
    let cancel = AtomicBool::new(false);
    let output = ext.registry().run(name, args, &ToolCtx::new(cwd, &cancel));
    assert!(!output.is_error(), "{}", output.text());
    output.into_text()
}

#[test]
fn modern_stdio_server_starts_only_on_the_first_call() {
    let home = temp("modern");
    let specs = fake_spec(&home, "modern", 3_600_000);
    let servers = McpServers::new(&home, specs.clone(), None);
    assert!(servers.index().is_empty() && !servers.running("fake"));
    assert_eq!(servers.refresh("fake"), Ok(1));
    assert_eq!(log(&home), "server/discover\ntools/list\n");
    let file = cache::read(&home, &specs[0]).expect("cache written");
    assert_eq!(
        (file.ttl_ms, file.era.as_str()),
        (Some(3_600_000), "modern")
    );
    drop(servers);

    let servers = McpServers::new(&home, specs.clone(), None);
    let ext = Extensions::default().with_mcp(servers.clone());
    let registry = ext.registry();
    assert!(registry.specs().iter().any(|s| s.name == "mcp__fake__echo"));
    assert!(!servers.running("fake"), "the index comes from the cache");
    assert_eq!(log(&home), "server/discover\ntools/list\n");
    assert_eq!(
        call(&servers, "mcp__fake__echo", r#"{"text":"hi"}"#, &home),
        "echo: hi"
    );
    assert!(servers.running("fake"));
    assert_eq!(
        log(&home),
        "server/discover\ntools/list\nserver/discover\ntools/call\n"
    );

    let mut always = specs.clone();
    always[0].always_load = true;
    let ext = Extensions::default().with_mcp(McpServers::new(&home, always, None));
    assert_eq!(ext.mcp_preload(), vec!["mcp__fake__echo".to_string()]);
    let settings = fast_harness_core::app::load_settings(&home, None, &|_| None).unwrap();
    let doctor = fast_harness_core::app::doctor(&settings);
    assert!(doctor.contains("mcp:\n  fake (stdio, user)"), "{doctor}");
    assert!(doctor.contains("1 tools cached"), "{doctor}");
    drop(servers);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn legacy_stdio_server_gets_the_initialize_handshake() {
    let home = temp("legacy");
    let servers = McpServers::new(&home, fake_spec(&home, "legacy", 0), None);
    assert_eq!(servers.refresh("fake"), Ok(1));
    assert_eq!(
        call(&servers, "mcp__fake__echo", r#"{"text":"old"}"#, &home),
        "echo: old"
    );
    assert_eq!(
        log(&home),
        "server/discover\ninitialize\nnotifications/initialized\ntools/list\ntools/call\n"
    );
    assert_eq!(servers.cache_of("fake").unwrap().era, "legacy");
    drop(servers);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn an_expired_list_is_refreshed_on_use_and_idle_servers_stop() {
    let home = temp("ttl");
    let specs = fake_spec(&home, "modern", 1);
    McpServers::new(&home, specs.clone(), None)
        .refresh("fake")
        .unwrap();
    std::thread::sleep(Duration::from_millis(20));
    let idle = Duration::from_millis(200);
    let servers = McpServers::with_idle(&home, specs, None, idle);
    assert_eq!(servers.index().len(), 1, "an expired cache still indexes");
    call(&servers, "mcp__fake__echo", r#"{"text":"x"}"#, &home);
    assert!(
        log(&home).ends_with("server/discover\ntools/list\ntools/call\n"),
        "{}",
        log(&home)
    );
    assert!(servers.running("fake"));
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!servers.running("fake"), "stopped after the idle time");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn past_the_cap_tools_go_through_describe_and_call() {
    let home = temp("cap");
    let specs = fake_spec(&home, "modern", 3_600_000);
    let mut tools: Vec<Value> = (0..69)
        .map(|i| json!({"name": format!("t{i}"), "description": format!("Tool number {i}"), "inputSchema": {"type": "object"}}))
        .collect();
    tools.push(json!({"name": "echo", "description": "Echo the text back", "inputSchema": {"type": "object"}}));
    let file = cache::CacheFile {
        version: 1,
        server: "fake".into(),
        fingerprint: specs[0].fingerprint(),
        fetched_at: fast_harness_core::trace::now_ms(),
        ttl_ms: Some(3_600_000),
        tools,
        ..Default::default()
    };
    cache::write(&home, &file).unwrap();
    let servers = McpServers::new(&home, specs, None);
    let ext = Extensions::default().with_mcp(servers.clone());
    let registry = ext.registry();
    let names: Vec<&str> = registry.specs().iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"mcp_describe") && names.contains(&"mcp_call"));
    assert!(!names.iter().any(|n| n.starts_with("mcp__")), "{names:?}");
    assert!(registry.specs()[0].description.contains("70 MCP tools"));
    let found = call(
        &servers,
        "find_tools",
        r#"{"query":"echo text back"}"#,
        &home,
    );
    assert!(
        found.contains("mcp_describe") && found.contains("- mcp__fake__echo: Echo"),
        "{found}"
    );
    let described = call(
        &servers,
        "mcp_describe",
        r#"{"name":"mcp__fake__t7"}"#,
        &home,
    );
    assert!(described.contains("Tool number 7"));
    assert!(!servers.running("fake"));
    let args = r#"{"name":"mcp__fake__echo","arguments":{"text":"far"}}"#;
    assert_eq!(call(&servers, "mcp_call", args, &home), "echo: far");
    drop(servers);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn an_untrusted_project_mcp_json_loads_nothing() {
    let home = temp("trust-home");
    let project = temp("trust-project");
    std::fs::create_dir_all(project.join(".fast-harness")).unwrap();
    let file = r#"{"servers": {"p": {"command": "p.exe"}, "bad name": {"command": "x"}}}"#;
    std::fs::write(project.join(".fast-harness").join("mcp.json"), file).unwrap();
    let (specs, files, skipped) = load(&Layers::find(&home, &project));
    assert!(specs.is_empty() && files.is_empty() && skipped.is_empty());
    fast_harness_core::config::trust::trust(&home, &project).unwrap();
    let (specs, _, skipped) = load(&Layers::find(&home, &project));
    assert_eq!(specs.len(), 1);
    assert_eq!(skipped.len(), 1, "the bad name is reported");
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&project);
}

fn http_spec(home: &Path) -> Vec<ServerSpec> {
    let file = r#"{"servers": {"web": {"url": "http://mcp.test/mcp", "headers": {"Authorization": "Bearer ${PATH}"}}}}"#;
    std::fs::write(home.join("mcp.json"), file).unwrap();
    load(&Layers::user_only(home)).0
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

#[test]
fn modern_http_server_with_sse_and_request_headers() {
    let home = temp("http-modern");
    let list = "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{}}\n\n\
        data: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":[{\"name\":\"echo\",\"inputSchema\":{\"type\":\"object\",\"properties\":{\"r\":{\"type\":\"string\",\"x-mcp-header\":\"Region\"}}}}],\"ttlMs\":60000,\"cacheScope\":\"public\"}}\n\n";
    let transport = Arc::new(ScriptedTransport::new(vec![
        ScriptedResponse::new(
            200,
            r#"{"jsonrpc":"2.0","id":1,"result":{"supportedVersions":["2026-07-28"]}}"#,
            7,
        ),
        ScriptedResponse::new(200, list, 5).with_header("content-type", "text/event-stream"),
        ScriptedResponse::new(
            200,
            r#"{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"ok"}]}}"#,
            3,
        ),
    ]));
    let shared: Arc<dyn Transport> = transport.clone();
    let servers = McpServers::new(&home, http_spec(&home), Some(shared));
    let cancel = AtomicBool::new(false);
    let ctx = ToolCtx::new(&home, &cancel);
    let output = servers.call("web", "echo", &json!({"r": "eu"}), &ctx);
    assert_eq!(output.text(), "ok");
    let requests = transport.requests();
    assert_eq!(requests.len(), 3);
    let list = &requests[1].headers;
    assert_eq!(header(list, "mcp-method"), Some("tools/list"));
    assert_eq!(header(list, "mcp-protocol-version"), Some("2026-07-28"));
    assert_eq!(header(&requests[2].headers, "mcp-name"), Some("echo"));
    assert_eq!(header(&requests[2].headers, "mcp-param-region"), Some("eu"));
    let path = std::env::var("PATH").unwrap_or_default();
    assert_eq!(
        header(&requests[2].headers, "authorization"),
        Some(format!("Bearer {path}").as_str())
    );
    assert!(requests[2]
        .body
        .contains("io.modelcontextprotocol/clientInfo"));
    let listing = fast_harness_core::app::mcp_lines(&home, &servers.specs());
    assert!(listing.contains("(headers: Authorization)"), "{listing}");
    assert!(
        path.is_empty() || !listing.contains(&path),
        "no header value in listings"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn legacy_http_server_gets_initialize_and_its_session() {
    let home = temp("http-legacy");
    let transport = Arc::new(ScriptedTransport::new(vec![
        ScriptedResponse::new(400, "Bad Request", 4),
        ScriptedResponse::new(
            200,
            r#"{"jsonrpc":"2.0","id":2,"result":{"protocolVersion":"2025-06-18"}}"#,
            9,
        )
        .with_header("mcp-session-id", "abc"),
        ScriptedResponse::new(202, "", 1),
        ScriptedResponse::new(
            200,
            r#"{"jsonrpc":"2.0","id":3,"result":{"tools":[{"name":"echo"}]}}"#,
            9,
        ),
    ]));
    let shared: Arc<dyn Transport> = transport.clone();
    let servers = McpServers::new(&home, http_spec(&home), Some(shared));
    assert_eq!(servers.refresh("web"), Ok(1));
    let requests = transport.requests();
    assert!(requests[1].body.contains("\"initialize\""));
    assert_eq!(header(&requests[3].headers, "mcp-session-id"), Some("abc"));
    assert_eq!(
        header(&requests[3].headers, "mcp-protocol-version"),
        Some("2025-06-18")
    );
    assert_eq!(header(&requests[3].headers, "mcp-method"), None);
    assert_eq!(servers.cache_of("web").unwrap().era, "legacy");
    let _ = std::fs::remove_dir_all(&home);
}

/// The legacy handshake with session `session`: probe 400, initialize, 202.
fn legacy_handshake(session: &str, out: &mut Vec<ScriptedResponse>) {
    out.push(ScriptedResponse::new(400, "Bad Request", 4));
    out.extend(new_session(session));
}

fn new_session(session: &str) -> Vec<ScriptedResponse> {
    let init = r#"{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":"2025-06-18"}}"#;
    vec![
        ScriptedResponse::new(200, init, 9).with_header("mcp-session-id", session),
        ScriptedResponse::new(202, "", 1),
    ]
}

fn list_reply() -> ScriptedResponse {
    ScriptedResponse::new(
        200,
        r#"{"jsonrpc":"2.0","id":0,"result":{"tools":[{"name":"echo"}]}}"#,
        9,
    )
}

/// D63: HTTP 404 to a request with the session id: one new initialize, one
/// retry with the new id, success.
#[test]
fn an_expired_http_session_is_initialized_again_once() {
    let home = temp("http-expired");
    let mut script = Vec::new();
    legacy_handshake("old", &mut script);
    script.push(ScriptedResponse::new(404, "Not Found", 4));
    script.extend(new_session("new"));
    script.push(list_reply());
    let transport = Arc::new(ScriptedTransport::new(script));
    let shared: Arc<dyn Transport> = transport.clone();
    let servers = McpServers::new(&home, http_spec(&home), Some(shared));
    assert_eq!(servers.refresh("web"), Ok(1));
    let requests = transport.requests();
    assert_eq!(requests.len(), 7);
    assert_eq!(header(&requests[3].headers, "mcp-session-id"), Some("old"));
    assert!(requests[4].body.contains("\"initialize\""));
    assert_eq!(header(&requests[4].headers, "mcp-session-id"), None);
    assert!(requests[6].body.contains("tools/list"));
    assert_eq!(header(&requests[6].headers, "mcp-session-id"), Some("new"));
    let _ = std::fs::remove_dir_all(&home);
}

/// D63: a second 404 after the new session is an error, not a loop, and the
/// connection is dead: the next use connects again from the probe.
#[test]
fn a_second_404_is_an_error_and_the_connection_is_dropped() {
    let home = temp("http-expired-twice");
    let mut script = Vec::new();
    legacy_handshake("old", &mut script);
    script.push(ScriptedResponse::new(404, "Not Found", 4));
    script.extend(new_session("new"));
    script.push(ScriptedResponse::new(404, "Not Found", 4));
    legacy_handshake("third", &mut script);
    script.push(list_reply());
    let transport = Arc::new(ScriptedTransport::new(script));
    let shared: Arc<dyn Transport> = transport.clone();
    let servers = McpServers::new(&home, http_spec(&home), Some(shared));
    let error = servers.refresh("web").unwrap_err();
    assert!(error.contains("404 again"), "{error}");
    assert_eq!(transport.requests().len(), 7, "one retry only");
    assert_eq!(servers.refresh("web"), Ok(1), "a new connection");
    let requests = transport.requests();
    assert!(
        requests[7].body.contains("server/discover"),
        "not alive: probed again"
    );
    assert_eq!(
        header(&requests[10].headers, "mcp-session-id"),
        Some("third")
    );
    let _ = std::fs::remove_dir_all(&home);
}
