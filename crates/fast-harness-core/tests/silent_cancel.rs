//! A cancel ends a model call whose stream is silent (PLAN.md D66): before
//! the first byte and between chunks, on every provider, through
//! `HttpRequest::cancel` and `ScriptedTransport::with_silence`.

mod agent_support;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agent_support::*;
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::events::{AgentEvent, TurnOutcome};
use fast_harness_core::http::{ScriptedResponse, ScriptedTransport, CANCEL_BOUND_MS};
use fast_harness_core::llm::openai::OpenAiChat;
use fast_harness_core::llm::{custom, Auth, ChatRequest, ErrorKind, Provider};

/// Sets `cancel` after `delay`; returns the instant it was set.
fn cancel_after(cancel: &AtomicBool, delay: Duration) -> Instant {
    std::thread::sleep(delay);
    cancel.store(true, Ordering::Relaxed);
    Instant::now()
}

struct Ended {
    outcome: TurnOutcome,
    seen: Vec<AgentEvent>,
    kinds: Vec<String>,
    /// From the cancel to the turn's end.
    latency: Duration,
}

/// One turn over `transport`, cancelled 150 ms after it starts.
fn silent_turn(name: &str, transport: ScriptedTransport) -> Ended {
    let env = env(name);
    let transport = Arc::new(transport);
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let agent = Agent::new(parts(&transport, key_auth(), &env.cwd), echo_registry());
    let cancel = AtomicBool::new(false);
    let mut seen = Vec::new();
    let (outcome, set_at, ended) = std::thread::scope(|scope| {
        let setter = scope.spawn(|| cancel_after(&cancel, Duration::from_millis(150)));
        let outcome = agent.run_turn(&mut conv, "go", &cancel, &mut |e| seen.push(e));
        let ended = Instant::now();
        (outcome, setter.join().unwrap(), ended)
    });
    let kinds = kinds(&conv.log_path);
    drop(env);
    Ended {
        outcome,
        seen,
        kinds,
        latency: ended.saturating_duration_since(set_at),
    }
}

fn assert_cancelled(ended: &Ended, state: &str) {
    eprintln!("cancel latency {state}: {:?}", ended.latency);
    assert_eq!(ended.outcome, TurnOutcome::Cancelled, "{state}");
    assert!(
        ended.latency < Duration::from_millis(CANCEL_BOUND_MS),
        "{state}: {:?}",
        ended.latency
    );
    assert!(!ended.kinds.contains(&"assistant/message".to_string()));
    assert_eq!(ended.kinds.last().map(String::as_str), Some("turn/end"));
    assert!(matches!(
        ended.seen.last(),
        Some(AgentEvent::TurnEnd {
            outcome: TurnOutcome::Cancelled,
            ..
        })
    ));
}

/// The chunk index (7 byte chunks) just after the text delta event.
fn after_text_delta(reply: &ScriptedResponse) -> usize {
    let body: Vec<u8> = reply.chunks.concat();
    let text = String::from_utf8(body).unwrap();
    text.find("content_block_stop").unwrap() / 7
}

/// The trace of a cancel between two chunks, as before D66.
fn cancel_between_chunks_kinds() -> Vec<String> {
    let env = env("cancel-between");
    let transport = Arc::new(ScriptedTransport::new(vec![text_reply("a long answer")]));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let agent = Agent::new(parts(&transport, key_auth(), &env.cwd), echo_registry());
    let cancel = AtomicBool::new(false);
    let outcome = agent.run_turn(&mut conv, "go", &cancel, &mut |e| {
        if matches!(e, AgentEvent::TextDelta(_)) {
            cancel.store(true, Ordering::Relaxed);
        }
    });
    assert_eq!(outcome, TurnOutcome::Cancelled);
    let kinds = kinds(&conv.log_path);
    drop(env);
    kinds
}

#[test]
fn cancel_before_the_first_byte_ends_the_turn_cancelled() {
    let transport = ScriptedTransport::new(vec![text_reply("never")]).with_silence(0, 0);
    let ended = silent_turn("silent-first-byte", transport);
    assert_cancelled(&ended, "before the first byte");
    assert!(!ended
        .seen
        .iter()
        .any(|e| matches!(e, AgentEvent::TextDelta(_))));
    assert_eq!(ended.kinds, cancel_between_chunks_kinds());
}

#[test]
fn cancel_in_a_silent_pause_mid_stream_ends_the_turn_cancelled() {
    let reply = text_reply("half an answer");
    let at = after_text_delta(&reply);
    let transport = ScriptedTransport::new(vec![reply]).with_silence(0, at);
    let ended = silent_turn("silent-mid-stream", transport);
    assert_cancelled(&ended, "between chunks");
    assert!(ended
        .seen
        .contains(&AgentEvent::TextDelta("half an answer".into())));
    assert_eq!(ended.kinds, cancel_between_chunks_kinds());
}

#[test]
fn silence_alone_does_not_end_the_call() {
    // Silence alone does not end the call early: it lasts until the cancel.
    let transport = ScriptedTransport::new(vec![text_reply("x")]).with_silence(0, 0);
    let cancel = AtomicBool::new(false);
    let started = Instant::now();
    let outcome = std::thread::scope(|scope| {
        scope.spawn(|| cancel_after(&cancel, Duration::from_millis(300)));
        let env = env("silent-no-early-end");
        let transport = Arc::new(transport);
        let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
        let agent = Agent::new(parts(&transport, key_auth(), &env.cwd), echo_registry());
        let outcome = agent.run_turn(&mut conv, "go", &cancel, &mut |_| {});
        drop(env);
        outcome
    });
    assert!(started.elapsed() >= Duration::from_millis(300));
    assert_eq!(outcome, TurnOutcome::Cancelled);
}

#[test]
fn openai_chat_cancel_in_silence_is_aborted_within_the_bound() {
    let file = r#"{"id": "sc", "api": "openai-chat", "baseUrl": "http://local/v1",
        "models": [{"id": "chat-1", "maxOutput": 4096}]}"#;
    custom::remember(&[serde_json::from_str(file).unwrap()]);
    let body = "data: {\"choices\": [{\"index\": 0, \"delta\": {\"content\": \"Hel\"}}]}\n\n";
    for at in [0, 1] {
        let reply = ScriptedResponse::new(200, body, 1000);
        let transport = ScriptedTransport::new(vec![reply]).with_silence(0, at);
        let provider = OpenAiChat::new("sc", "http://local/v1/", Vec::new());
        let cancel = AtomicBool::new(false);
        let req = ChatRequest::new("sc/chat-1", "be brief", &[], &[], 1000, &cancel);
        let auth = Auth::ApiKey("test-key".into());
        let (result, set_at, ended) = std::thread::scope(|scope| {
            let setter = scope.spawn(|| cancel_after(&cancel, Duration::from_millis(150)));
            let result = provider.stream(&transport, &auth, &req, &mut |_| {});
            (result, setter.join().unwrap(), Instant::now())
        });
        let latency = ended.saturating_duration_since(set_at);
        eprintln!("openai-chat cancel latency (silence after {at} chunks): {latency:?}");
        assert_eq!(result.unwrap_err().kind, ErrorKind::Aborted);
        assert!(latency < Duration::from_millis(CANCEL_BOUND_MS));
    }
}
