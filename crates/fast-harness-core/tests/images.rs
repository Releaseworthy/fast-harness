//! Image input (PLAN.md D70): a user image and a `read` image in Claude
//! bodies, data URLs and placeholders on openai-chat, the v13 trace round
//! trip, v12 logs, and a missing media file after a reopen.

mod agent_support;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_support::{env, key_auth, parts, text_reply, tool_reply, Env};
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::crypto::base64_encode;
use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use fast_harness_core::llm::openai::OpenAiChat;
use fast_harness_core::llm::{custom, ChatRequest};
use fast_harness_core::media::{self, solid_png};
use fast_harness_core::tools;
use fast_harness_core::types::{Block, ImageRef, Message, Role};
use serde_json::{json, Value};

fn claude(env: &Env, script: Vec<ScriptedResponse>) -> (Agent, Arc<ScriptedTransport>) {
    let transport = Arc::new(ScriptedTransport::new(script));
    let parts = parts(&transport, key_auth(), &env.cwd);
    let mut agent = Agent::new(parts, Arc::new(tools::builtin()));
    agent.preload = vec!["read".into()];
    (agent, transport)
}

fn bodies(transport: &ScriptedTransport) -> Vec<Value> {
    let requests = transport.requests();
    requests
        .iter()
        .map(|r| serde_json::from_str(&r.body).unwrap())
        .collect()
}

fn turn(agent: &Agent, conv: &mut Conversation, text: &str) {
    agent.run_turn(conv, text, &AtomicBool::new(false), &mut |_| {});
}

/// A red 8x8 PNG attached to the conversation's next user message.
fn attach_red(conv: &Conversation) -> (ImageRef, Vec<u8>) {
    let png = solid_png(8, 8, [255, 0, 0]);
    let dir = conv.tool_session.media_dir().unwrap();
    let image = media::attach_bytes(dir, &png, Some("image/png")).unwrap();
    conv.tool_session.attach(image.clone());
    (image, png)
}

fn last_user(body: &Value) -> &Value {
    let messages = body["messages"].as_array().unwrap();
    messages.iter().rev().find(|m| m["role"] == "user").unwrap()
}

#[test]
fn a_user_image_goes_to_claude_as_base64_before_the_text_and_not_into_the_trace() {
    let env = env("img-user");
    let (agent, transport) = claude(&env, vec![text_reply("red")]);
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let (image, png) = attach_red(&conv);
    assert!(image.path.contains(".media"), "{}", image.path);
    turn(&agent, &mut conv, "what colour?");
    let body = &bodies(&transport)[0];
    let content = body["messages"][0]["content"].as_array().unwrap().clone();
    assert_eq!(
        content[0]["text"],
        format!("[image 1: {}]\n", media::label(&image))
    );
    assert!(media::label(&image).starts_with("png 8x8, 268 bytes, about 1 tokens"));
    let source = json!({"type": "base64", "media_type": "image/png", "data": base64_encode(&png)});
    assert_eq!(content[1], json!({"type": "image", "source": source}));
    assert_eq!(content[2]["text"], "what colour?");
    let log = std::fs::read_to_string(&conv.log_path).unwrap();
    assert!(
        log.starts_with(r#"{"type":"session","version":13"#),
        "{}",
        &log[..60]
    );
    assert!(log.contains(r#""mediaType":"image/png"#) && log.contains(r#""type":"image""#));
    assert!(
        !log.contains(&base64_encode(&png)),
        "never inline in the trace"
    );
    let id = conv.id.clone();
    drop(conv);
    let (conv, _) = Conversation::reopen(&env.home, &env.cwd, &id).unwrap();
    let images: Vec<&ImageRef> = conv.messages.iter().flat_map(Message::images).collect();
    assert_eq!(images, [&image], "the reference comes back on reopen");
    assert!(
        conv.tool_session.attached().is_empty(),
        "the message took the queue"
    );
}

#[test]
fn a_read_png_goes_inside_its_tool_result_and_a_bad_one_is_an_error() {
    let env = env("img-read");
    std::fs::write(env.cwd.join("shot.png"), solid_png(4, 2, [0, 0, 255])).unwrap();
    std::fs::write(env.cwd.join("fake.png"), b"\x89PNG\r\n\x1a\nbroken").unwrap();
    let read = |id: &str, path: &str| (id.to_string(), json!({"path": path}).to_string());
    let (a, b) = (read("c1", "shot.png"), read("c2", "fake.png"));
    let calls = [
        (a.0.as_str(), "read", a.1.as_str()),
        (b.0.as_str(), "read", b.1.as_str()),
    ];
    let (agent, transport) = claude(&env, vec![tool_reply(&calls), text_reply("blue")]);
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    turn(&agent, &mut conv, "look");
    let body = &bodies(&transport)[1];
    let content = last_user(body)["content"].as_array().unwrap().clone();
    let result = &content[0];
    assert_eq!(result["type"], "tool_result");
    let inner = result["content"].as_array().unwrap();
    assert_eq!(
        inner[0]["text"],
        "shot.png: image, png 4x2, 94 bytes, about 1 tokens"
    );
    assert_eq!(inner[1]["type"], "image");
    assert_eq!(inner[1]["source"]["media_type"], "image/png");
    assert_eq!(content[1]["is_error"], true);
    let error = content[1]["content"].as_str().unwrap();
    assert!(
        error.contains("fake.png is an image; not shown: the image header holds no readable size"),
        "{error}"
    );
    assert!(content[1].get("cache_control").is_some());
}

fn openai(images_model: bool) -> OpenAiChat {
    let file = format!(
        r#"{{"id": "oc", "api": "openai-chat", "baseUrl": "http://local/v1",
        "models": [{{"id": "m", "images": {images_model}}}]}}"#
    );
    let models = serde_json::from_str::<custom::ProviderFile>(&file)
        .unwrap()
        .models;
    OpenAiChat::new("oc", "http://local/v1", Vec::new()).with_models(models)
}

#[test]
fn openai_chat_sends_data_urls_or_a_placeholder_never_an_image_to_a_text_model() {
    let root = std::env::temp_dir().join(format!("fh-img-openai-{}", std::process::id()));
    let png = solid_png(2, 2, [0, 255, 0]);
    let image = media::attach_bytes(&root, &png, None).unwrap();
    let call = Block::ToolCall {
        id: "c1".into(),
        name: "read".into(),
        arguments: "{}".into(),
        signature: None,
    };
    let history = vec![
        media::user_message("which colour?", std::slice::from_ref(&image)),
        Message::new(Role::Assistant, vec![call]),
        Message::new(
            Role::Tool,
            vec![
                Block::tool_result("c1", "g.png: image", false),
                Block::Image(image.clone()),
            ],
        ),
    ];
    let cancel = AtomicBool::new(false);
    let req = ChatRequest::new("oc/m", "", &history, &[], 100, &cancel);
    let body: Value = serde_json::from_str(&openai(true).request_body(&req)).unwrap();
    let url = format!("data:image/png;base64,{}", base64_encode(&png));
    let part = json!({"type": "image_url", "image_url": {"url": url}});
    assert_eq!(body["messages"][0]["content"][1], part);
    assert_eq!(
        body["messages"][0]["content"][2],
        json!({"type": "text", "text": "which colour?"})
    );
    assert_eq!(
        body["messages"][2],
        json!({"role": "tool", "tool_call_id": "c1", "content": "g.png: image"})
    );
    assert_eq!(
        body["messages"][3]["content"][1], part,
        "a tool image follows as a user message"
    );
    let text_only = openai(false).request_body(&req);
    assert!(!text_only.contains("image_url") && !text_only.contains(&base64_encode(&png)));
    let body: Value = serde_json::from_str(&text_only).unwrap();
    let label = media::label(&image);
    let placeholder = format!("[image not sent: this model does not take images; {label}]");
    let expected = format!("[image 1: {label}]\n\n\n{placeholder}\n\nwhich colour?");
    assert_eq!(body["messages"][0]["content"], expected);
    assert_eq!(
        body["messages"][2]["content"],
        format!("g.png: image\n{placeholder}")
    );
    assert_eq!(body["messages"].as_array().unwrap().len(), 3);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_missing_media_file_after_reopen_is_a_placeholder_and_a_v12_log_still_opens() {
    let env = env("img-missing");
    let (agent, _) = claude(&env, vec![text_reply("red")]);
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let (image, _) = attach_red(&conv);
    turn(&agent, &mut conv, "colour?");
    let id = conv.id.clone();
    drop(conv);
    std::fs::remove_file(&image.path).unwrap();
    let (mut conv, _) = Conversation::reopen(&env.home, &env.cwd, &id).unwrap();
    let (agent, transport) = claude(&env, vec![text_reply("ok")]);
    turn(&agent, &mut conv, "again");
    let body = &bodies(&transport)[0];
    let first = body["messages"][0]["content"].as_array().unwrap();
    assert!(first.iter().all(|b| b["type"] != "image"));
    let text = first[1]["text"].as_str().unwrap();
    assert!(
        text.starts_with("[image not sent: its file ") && text.contains("cannot be read"),
        "{text}"
    );
}

#[test]
fn a_version_12_log_without_images_still_opens() {
    let env = env("img-v12");
    let (agent, _) = claude(&env, vec![text_reply("hi")]);
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    turn(&agent, &mut conv, "hello");
    let (id, path, count) = (conv.id.clone(), conv.log_path.clone(), conv.messages.len());
    drop(conv);
    let log = std::fs::read_to_string(&path).unwrap();
    let old = log.replacen(r#""version":13"#, r#""version":12"#, 1);
    assert_ne!(log, old);
    std::fs::write(&path, old).unwrap();
    let (conv, _) = Conversation::reopen(&env.home, &env.cwd, &id).unwrap();
    assert_eq!(conv.messages.len(), count);
}
