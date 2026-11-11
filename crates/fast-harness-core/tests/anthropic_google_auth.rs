//! The Anthropic provider refuses a Google credential without sending it.

use fast_harness_core::http::ScriptedTransport;
use fast_harness_core::llm::anthropic::Anthropic;
use fast_harness_core::llm::{Auth, ChatRequest, ErrorKind, Provider};
use fast_harness_core::types::Message;
use std::sync::atomic::AtomicBool;

#[test]
fn google_credential_is_an_auth_error_and_no_request() {
    let transport = ScriptedTransport::new(vec![]);
    let cancel = AtomicBool::new(false);
    let messages = [Message::user("hi")];
    let req = ChatRequest {
        model: "claude-opus-4-7",
        system: "sys",
        messages: &messages,
        tools: &[],
        max_tokens: 1000,
        effort: None,
        cache_long: false,
        cancel: &cancel,
    };
    let auth = Auth::Google {
        access: "fake-access".into(),
        project: "fake-project".into(),
    };
    let err = Anthropic::default()
        .stream(&transport, &auth, &req, &mut |_| {})
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Auth);
    assert_eq!(err.message, "a Google credential cannot call Anthropic");
    assert!(transport.requests().is_empty());
}
