//! Antigravity declarations under the find tool (PLAN.md D30): find_tools
//! alone at the start, then the found tools in first-found order.

use std::sync::atomic::AtomicBool;

use fast_harness_core::llm::antigravity::request::{build, Envelope};
use fast_harness_core::llm::{ChatRequest, ToolSpec};
use fast_harness_core::tools;
use fast_harness_core::types::{Block, Message, Role};
use serde_json::{json, Value};

const MODEL: &str = "antigravity/gemini-3.1-pro-preview";

fn envelope(messages: &[Message], tools: &[ToolSpec]) -> Envelope {
    let cancel = AtomicBool::new(false);
    let req = ChatRequest {
        model: MODEL,
        system: "Be brief.",
        messages,
        tools,
        max_tokens: 8000,
        effort: None,
        cache: Default::default(),
        session: "",
        cancel: &cancel,
    };
    build(&req, "gemini-3.1-pro-preview", "proj-1", "sess-1").expect("envelope")
}

fn declarations(env: &Envelope) -> Vec<Value> {
    env.body["request"]["tools"][0]["functionDeclarations"]
        .as_array()
        .expect("declarations")
        .clone()
}

fn names(list: &[Value]) -> Vec<&str> {
    list.iter().map(|d| d["name"].as_str().unwrap()).collect()
}

fn step(messages: &mut Vec<Message>, id: &str, name: &str, args: &str, output: &str) {
    messages.push(Message {
        role: Role::Assistant,
        content: vec![Block::ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: args.into(),
            signature: None,
        }],
        model: Some(MODEL.into()),
    });
    messages.push(Message {
        role: Role::Tool,
        content: vec![Block::ToolResult {
            call_id: id.into(),
            content: output.into(),
            is_error: false,
        }],
        model: None,
    });
}

#[test]
fn declarations_start_with_find_and_grow_in_found_order() {
    let registry = tools::builtin();
    let specs = registry.specs();
    let mut messages = vec![Message::user("go")];
    let start = declarations(&envelope(&messages, specs));
    assert_eq!(names(&start), ["find_tools"]);
    let description = start[0]["description"].as_str().unwrap();
    assert!(description.contains("read, write, edit, shell, grep, glob"));
    step(
        &mut messages,
        "c1",
        "find_tools",
        r#"{"names":["edit","read"]}"#,
        "Loaded: edit, read",
    );
    let after = declarations(&envelope(&messages, specs));
    assert_eq!(names(&after), ["find_tools", "edit", "read"]);
    assert_eq!(after[0], start[0]);
    step(
        &mut messages,
        "c2",
        "read",
        r#"{"path":"a"}"#,
        "Loaded: shell",
    );
    let later = declarations(&envelope(&messages, specs));
    assert_eq!(later, after, "only find results load tools");
    step(
        &mut messages,
        "c3",
        "find_tools",
        r#"{"query":"x"}"#,
        "Loaded: shell, read",
    );
    let last = declarations(&envelope(&messages, specs));
    assert_eq!(names(&last), ["find_tools", "edit", "read", "shell"]);
    assert_eq!(last[..3], after[..]);
    // The find result goes back as a plain function response.
    let env = envelope(&messages, specs);
    let response = &env.body["request"]["contents"][2]["parts"][0]["functionResponse"];
    assert_eq!(response["name"], "find_tools");
    assert_eq!(
        response["response"],
        json!({"output": "Loaded: edit, read"})
    );
}

#[test]
fn without_find_every_tool_is_declared() {
    let mut registry = tools::Registry::new();
    for spec in tools::builtin().specs().iter().skip(1) {
        registry.add(Box::new(Fixed(spec.clone())));
    }
    let env = envelope(&[Message::user("go")], registry.specs());
    assert_eq!(declarations(&env).len(), 6);
}

struct Fixed(ToolSpec);

impl tools::Tool for Fixed {
    fn spec(&self) -> ToolSpec {
        self.0.clone()
    }

    fn run(&self, _: &Value, _: &tools::ToolCtx<'_>) -> tools::ToolOutput {
        tools::ToolOutput::ok("")
    }
}
