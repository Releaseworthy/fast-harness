//! `fast-harness serve --stdio` (PLAN.md D48): JSON-RPC 2.0 with the Agent
//! Client Protocol v2 names and our `_fh/` methods, driven over channels
//! with `ScriptedTransport`.
//! <https://agentclientprotocol.com/protocol/v2/overview>

mod agent_support;
mod service_support;

use std::sync::Arc;

use agent_support::{env, text_reply, tool_reply, Env};
use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use serde_json::{json, Value};
use service_support::{answer, Client, TestHost};

fn client(env: &Env, replies: Vec<ScriptedResponse>) -> (Client, Arc<ScriptedTransport>) {
    let transport = Arc::new(ScriptedTransport::new(replies));
    let host = TestHost {
        home: env.home.clone(),
        transport: Arc::clone(&transport),
        gate: Arc::default(),
    };
    (Client::start(Arc::new(host)), transport)
}

fn new_session(client: &mut Client, env: &Env) -> String {
    let result = client.call("session/new", json!({"cwd": env.cwd}));
    result["sessionId"].as_str().unwrap().to_string()
}

fn prompt(text: &str, session: &str) -> Value {
    json!({"sessionId": session, "prompt": [{"type": "text", "text": text}]})
}

fn waiting(text: &str, session: &str) -> Value {
    let mut params = prompt(text, session);
    params["_meta"] = json!({"fh/wait": true});
    params
}

fn bodies(transport: &ScriptedTransport) -> Vec<Value> {
    (transport.requests().iter())
        .filter(|r| r.url.contains("/v1/messages"))
        .map(|r| serde_json::from_str(&r.body).unwrap())
        .collect()
}

#[test]
fn initialize_new_and_prompt_stream_updates() {
    let env = env("serve-prompt");
    let (mut client, _) = client(&env, vec![text_reply("hello there")]);
    let init = client.call("initialize", json!({"protocolVersion": 2}));
    assert_eq!(init["protocolVersion"], 2);
    assert!(init["capabilities"]["session"].is_object());
    assert_eq!(init["info"]["name"], "fast-harness");
    let created = client.call("session/new", json!({"cwd": env.cwd}));
    let session = created["sessionId"].as_str().unwrap().to_string();
    let options = created["configOptions"].as_array().unwrap();
    assert_eq!(options[0]["configId"], "model");
    let accepted = client.call("session/prompt", prompt("hi", &session));
    assert_eq!(accepted["messageId"], "msg_u0");
    client.idle(&session, "end_turn");
    client.finish();
    let updates = client.updates(&session);
    let kinds: Vec<&str> = updates
        .iter()
        .filter_map(|u| u["sessionUpdate"].as_str())
        .collect();
    assert_eq!(&kinds[..2], ["user_message", "state_update"]);
    assert_eq!(updates[0]["messageId"], "msg_u0");
    assert_eq!(updates[0]["content"][0]["text"], "hi");
    assert_eq!(updates[1]["state"], "running");
    assert_eq!(answer(&updates), "hello there");
}

#[test]
fn a_waiting_prompt_returns_the_stop_reason_and_usage() {
    let env = env("serve-wait");
    let (mut client, _) = client(&env, vec![text_reply("ok")]);
    let session = new_session(&mut client, &env);
    let result = client.call("session/prompt", waiting("hi", &session));
    assert_eq!(result["stopReason"], "end_turn");
    assert_eq!(result["messageId"], "msg_u0");
    assert_eq!(result["_meta"]["fh/outcome"], "done");
    assert_eq!(result["_meta"]["fh/usage"]["inputTokens"], 10);
    client.finish();
}

#[test]
fn cancel_ends_the_turn_cancelled() {
    let env = env("serve-cancel");
    let slow = r#"{"text":"x","sleepMs":400}"#;
    let replies = vec![
        tool_reply(&[("toolu_1", "echo", slow)]),
        text_reply("never"),
    ];
    let (mut client, transport) = client(&env, replies);
    let session = new_session(&mut client, &env);
    client.call("session/prompt", prompt("go", &session));
    client.wait(|v| v["params"]["update"]["status"] == "in_progress");
    client.notify("session/cancel", json!({"sessionId": session}));
    client.idle(&session, "cancelled");
    client.finish();
    assert_eq!(
        bodies(&transport).len(),
        1,
        "no model call after the cancel"
    );
}

#[test]
fn steer_reaches_the_running_turn() {
    let env = env("serve-steer");
    let slow = r#"{"text":"x","sleepMs":300}"#;
    let replies = vec![tool_reply(&[("toolu_1", "echo", slow)]), text_reply("done")];
    let (mut client, transport) = client(&env, replies);
    let session = new_session(&mut client, &env);
    client.call("session/prompt", prompt("go", &session));
    client.wait(|v| v["params"]["update"]["status"] == "in_progress");
    let steered = client.call(
        "_fh/steer",
        json!({"sessionId": session, "text": "also check"}),
    );
    assert_eq!(steered["queued"], true);
    client.idle(&session, "end_turn");
    client.finish();
    let second = bodies(&transport)[1]["messages"].to_string();
    assert!(second.contains("also check"), "{second}");
    let updates = client.updates(&session);
    let shown = updates
        .iter()
        .any(|u| u["sessionUpdate"] == "user_message" && u["content"][0]["text"] == "also check");
    assert!(shown, "the steer is reported as a user message");
}

#[test]
fn set_config_option_changes_the_model_and_any_key() {
    let env = env("serve-config");
    let (mut client, transport) = client(&env, vec![text_reply("ok")]);
    let session = new_session(&mut client, &env);
    let set = json!({"sessionId": session, "configId": "model", "type": "id",
        "value": "claude-test-b"});
    let result = client.call("session/set_config_option", set);
    assert_eq!(result["configOptions"][0]["currentValue"], "claude-test-b");
    let steps = json!({"sessionId": session, "configId": "maxSteps", "type": "id", "value": "7"});
    client.call("session/set_config_option", steps);
    let bad = json!({"sessionId": session, "configId": "noSuchKey", "type": "id", "value": "1"});
    let id = client.request("session/set_config_option", bad);
    assert_eq!(client.response(id)["error"]["code"], -32602);
    client.call("session/prompt", waiting("hi", &session));
    client.finish();
    assert_eq!(bodies(&transport)[0]["model"], "claude-test-b");
}

#[test]
fn list_and_resume_with_replay() {
    let env = env("serve-resume");
    let (mut client, _) = client(&env, vec![text_reply("hello"), text_reply("again")]);
    let session = new_session(&mut client, &env);
    client.call("session/prompt", waiting("hi", &session));
    assert_eq!(
        client.call("session/close", json!({"sessionId": session})),
        json!({})
    );
    let listed = client.call("session/list", json!({"cwd": env.cwd}));
    let row = &listed["sessions"][0];
    assert_eq!(row["sessionId"], session.as_str());
    assert_eq!(row["title"], "hi");
    assert!(row["updatedAt"].as_str().unwrap().ends_with('Z'));
    let before = client.seen.len();
    let params = json!({"sessionId": session, "cwd": env.cwd, "replayFrom": {"type": "start"}});
    let resumed = client.call("session/resume", params);
    assert!(resumed["configOptions"].is_array());
    let replayed: Vec<Value> = (client.seen[before..].iter())
        .filter(|v| v["method"] == "session/update")
        .map(|v| v["params"]["update"].clone())
        .collect();
    assert_eq!(replayed[0]["sessionUpdate"], "user_message");
    assert_eq!(replayed[0]["messageId"], "msg_u0");
    assert_eq!(replayed[1]["sessionUpdate"], "agent_message");
    assert_eq!(replayed[1]["content"][0]["text"], "hello");
    assert_eq!(replayed.last().unwrap()["state"], "idle");
    let next = client.call("session/prompt", waiting("more", &session));
    assert_eq!(next["messageId"], "msg_u2");
    client.finish();
}

#[test]
fn two_sessions_run_at_once() {
    let env = env("serve-two");
    let replies = vec![
        tool_reply(&[("toolu_1", "gate", "{}")]),
        tool_reply(&[("toolu_1", "gate", "{}")]),
        text_reply("done"),
        text_reply("done"),
    ];
    let (mut client, _) = client(&env, replies);
    let (a, b) = (
        new_session(&mut client, &env),
        new_session(&mut client, &env),
    );
    let first = client.request("session/prompt", waiting("go", &a));
    let second = client.request("session/prompt", waiting("go", &b));
    for id in [first, second] {
        assert_eq!(client.response(id)["result"]["stopReason"], "end_turn");
    }
    client.finish();
    for session in [&a, &b] {
        let updates = client.updates(session);
        let result = updates.iter().find(|u| u["status"] == "completed").unwrap();
        assert_eq!(result["content"][0]["content"]["text"], "together");
    }
}

#[test]
fn errors_are_json_rpc_errors() {
    let env = env("serve-errors");
    let (mut client, _) = client(&env, vec![]);
    client.raw("{not json");
    let parse = client.wait(|v| v["error"]["code"] == -32700);
    assert_eq!(parse["id"], Value::Null);
    let id = client.request("session/nope", json!({}));
    assert_eq!(client.response(id)["error"]["code"], -32601);
    let id = client.request("session/prompt", prompt("hi", "no-such-session"));
    assert_eq!(client.response(id)["error"]["code"], -32002);
    let id = client.request("session/new", json!({"cwd": "relative/dir"}));
    assert_eq!(client.response(id)["error"]["code"], -32602);
    client.raw(r#"{"jsonrpc": "1.0", "id": 9, "method": "initialize"}"#);
    assert_eq!(client.response(9)["error"]["code"], -32600);
    client.finish();
}
