//! Antigravity request envelope: contents, provenance of thinking and
//! signatures, tool names, merging, Claude's VALIDATED mode.

use std::sync::atomic::AtomicBool;

use fast_harness_core::llm::antigravity::request::{build, Envelope, IDENTITY};
use fast_harness_core::llm::{ChatRequest, ToolSpec};
use fast_harness_core::types::{Block, Message, Role};
use serde_json::{json, Value};

const SIG: &str = "c2lnbmF0dXJlLW9mLWEtcmVhbC10aG91Z2h0LWZyb20tZ2VtaW5pLTMtcHJvLXByZXZpZXc=";
const GEMINI: &str = "antigravity/gemini-3.1-pro-preview";
const CLAUDE: &str = "antigravity/claude-opus-4-6-thinking";

fn assistant(model: &str, content: Vec<Block>) -> Message {
    Message {
        role: Role::Assistant,
        content,
        model: Some(model.to_string()),
    }
}

fn call(id: &str, name: &str, signature: Option<&str>) -> Block {
    Block::ToolCall {
        id: id.into(),
        name: name.into(),
        arguments: r#"{"path":"a.txt"}"#.into(),
        signature: signature.map(str::to_string),
    }
}

fn result(id: &str, content: &str, is_error: bool) -> Message {
    Message {
        role: Role::Tool,
        content: vec![Block::ToolResult {
            call_id: id.into(),
            content: content.into(),
            is_error,
        }],
        model: None,
    }
}

fn thinking(text: &str, signature: &str) -> Block {
    Block::Thinking {
        text: text.into(),
        signature: Some(signature.into()),
    }
}

fn tool(name: &str) -> ToolSpec {
    ToolSpec {
        name: name.into(),
        description: format!("{name} tool"),
        schema: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
    }
}

fn envelope(model: &str, messages: &[Message], tools: &[ToolSpec]) -> Envelope {
    let cancel = AtomicBool::new(false);
    let req = ChatRequest {
        model,
        system: "Be brief.",
        messages,
        tools,
        max_tokens: 8000,
        effort: None,
        cache: Default::default(),
        session: "",
        cancel: &cancel,
    };
    let bare = model.trim_start_matches("antigravity/");
    build(&req, bare, "proj-1", "sess-1").expect("envelope")
}

fn contents(env: &Envelope) -> &Vec<Value> {
    env.body["request"]["contents"]
        .as_array()
        .expect("contents")
}

#[test]
fn text_only() {
    let env = envelope(GEMINI, &[Message::user("hello")], &[]);
    let body = &env.body;
    assert_eq!(body["project"], "proj-1");
    assert_eq!(body["model"], "gemini-3.1-pro-preview");
    assert_eq!(body["requestType"], "agent");
    assert_eq!(body["userAgent"], "antigravity");
    let id = body["requestId"].as_str().unwrap();
    assert!(id.starts_with("agent-") && id.len() == 42, "{id}");
    assert_eq!(id.as_bytes()[20], b'4');
    let request = &body["request"];
    assert_eq!(request["sessionId"], "sess-1");
    assert_eq!(
        request["contents"],
        json!([{"role": "user", "parts": [{"text": "hello"}]}])
    );
    let system = &request["systemInstruction"];
    assert_eq!(system["role"], "user");
    assert_eq!(
        system["parts"][0]["text"],
        format!("{IDENTITY}\n\nBe brief.")
    );
    assert!(request.get("tools").is_none() && request.get("toolConfig").is_none());
    assert_eq!(request["generationConfig"]["maxOutputTokens"], 8000);
    let other = envelope(GEMINI, &[Message::user("hello")], &[]);
    assert_ne!(other.body["requestId"], body["requestId"]);
}

#[test]
fn tool_round_trip_keeps_same_model_signature() {
    let messages = vec![
        Message::user("read it"),
        assistant(
            GEMINI,
            vec![thinking("plan", SIG), call("c1", "read", Some(SIG))],
        ),
        result("c1", "file text", false),
    ];
    let env = envelope(GEMINI, &messages, &[tool("read")]);
    assert_eq!(
        contents(&env)[1],
        json!({"role": "model", "parts": [
            {"thought": true, "text": "plan", "thoughtSignature": SIG},
            {"functionCall": {"name": "read", "args": {"path": "a.txt"}, "id": "c1"}, "thoughtSignature": SIG}
        ]})
    );
    assert_eq!(
        contents(&env)[2],
        json!({"role": "user", "parts": [
            {"functionResponse": {"name": "read", "response": {"output": "file text"}, "id": "c1"}}
        ]})
    );
    let decl = &env.body["request"]["tools"][0]["functionDeclarations"][0];
    assert_eq!(decl["name"], "read");
    assert_eq!(decl["parameters"]["properties"]["path"]["type"], "string");
}

#[test]
fn call_from_another_model_gets_the_sentinel() {
    let messages = vec![
        Message::user("go"),
        assistant(
            "antigravity/gemini-3-flash",
            vec![call("c1", "read", Some(SIG)), call("c2", "read", Some(SIG))],
        ),
        result("c1", "a", false),
        result("c2", "b", false),
    ];
    let env = envelope(GEMINI, &messages, &[tool("read")]);
    let parts = &contents(&env)[1]["parts"];
    assert_eq!(
        parts[0]["thoughtSignature"],
        "skip_thought_signature_validator"
    );
    assert!(parts[1].get("thoughtSignature").is_none());
    // Both results in one user turn.
    assert_eq!(contents(&env).len(), 3);
    assert_eq!(contents(&env)[2]["parts"].as_array().unwrap().len(), 2);
}

#[test]
fn short_signature_is_not_real() {
    let messages = vec![
        Message::user("go"),
        assistant(GEMINI, vec![call("c1", "read", Some("short"))]),
    ];
    let env = envelope(GEMINI, &messages, &[]);
    let part = &contents(&env)[1]["parts"][0];
    assert_eq!(part["thoughtSignature"], "skip_thought_signature_validator");
}

#[test]
fn anthropic_thinking_is_dropped() {
    let messages = vec![
        Message::user("hi"),
        assistant(
            "claude-opus-5-5",
            vec![
                thinking("secret plan", SIG),
                Block::Text {
                    text: "answer".into(),
                },
            ],
        ),
        Message::user("more"),
    ];
    let env = envelope(GEMINI, &messages, &[]);
    assert_eq!(
        contents(&env)[1],
        json!({"role": "model", "parts": [{"text": "answer"}]})
    );
}

#[test]
fn tool_results_use_the_sanitised_call_name() {
    let messages = vec![
        Message::user("go"),
        assistant(GEMINI, vec![call("c1", "read.file", None)]),
        result("c1", "denied", true),
    ];
    let env = envelope(GEMINI, &messages, &[tool("read.file")]);
    let call = &contents(&env)[1]["parts"][0]["functionCall"];
    assert_eq!(call["name"], "read_file");
    let response = &contents(&env)[2]["parts"][0]["functionResponse"];
    assert_eq!(response["name"], "read_file");
    assert_eq!(response["response"], json!({"error": "denied"}));
    assert_eq!(env.names.original("read_file"), "read.file");
}

#[test]
fn same_roles_merge_and_history_starts_with_user() {
    let messages = vec![
        assistant(
            GEMINI,
            vec![Block::Text {
                text: "earlier".into(),
            }],
        ),
        Message::user("one"),
        Message::user("two"),
    ];
    let env = envelope(GEMINI, &messages, &[]);
    let list = contents(&env);
    assert_eq!(list.len(), 3);
    assert_eq!(list[0]["role"], "user");
    assert_eq!(
        list[1],
        json!({"role": "model", "parts": [{"text": "earlier"}]})
    );
    assert_eq!(
        list[2],
        json!({"role": "user", "parts": [{"text": "one"}, {"text": "two"}]})
    );
}

#[test]
fn gemini_2_calls_carry_no_id() {
    let model = "antigravity/gemini-2.5-flash";
    let messages = vec![
        Message::user("go"),
        assistant(model, vec![call("c1", "read", None)]),
        result("c1", "ok", false),
    ];
    let env = envelope(model, &messages, &[tool("read")]);
    assert!(contents(&env)[1]["parts"][0]["functionCall"]
        .get("id")
        .is_none());
    assert!(contents(&env)[2]["parts"][0]["functionResponse"]
        .get("id")
        .is_none());
}

#[test]
fn claude_uses_validated_and_keeps_signed_thinking() {
    let messages = vec![
        Message::user("go"),
        assistant(
            CLAUDE,
            vec![
                thinking("signed", SIG),
                thinking("unsigned", "x"),
                call("c1", "read", None),
            ],
        ),
        result("c1", "ok", false),
    ];
    let env = envelope(CLAUDE, &messages, &[tool("read")]);
    let request = &env.body["request"];
    assert_eq!(
        request["toolConfig"],
        json!({"functionCallingConfig": {"mode": "VALIDATED"}})
    );
    let parts = contents(&env)[1]["parts"].as_array().unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(
        parts[0],
        json!({"thought": true, "text": "signed", "thoughtSignature": SIG})
    );
    assert_eq!(parts[1]["functionCall"]["id"], "c1");
    let no_tools = envelope(CLAUDE, &[Message::user("x")], &[]);
    assert!(no_tools.body["request"].get("toolConfig").is_none());
}

#[test]
fn empty_history_is_an_error() {
    let cancel = AtomicBool::new(false);
    let req = ChatRequest {
        model: GEMINI,
        system: "",
        messages: &[],
        tools: &[],
        max_tokens: 100,
        effort: None,
        cache: Default::default(),
        session: "",
        cancel: &cancel,
    };
    assert!(build(&req, "gemini-3.1-pro-preview", "", "s").is_err());
    let env = envelope(GEMINI, &[Message::user("x")], &[]);
    assert_eq!(env.body["project"], "proj-1");
}
