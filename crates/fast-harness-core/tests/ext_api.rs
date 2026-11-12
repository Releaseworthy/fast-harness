//! The native plugin API (PLAN.md D52): a test plugin (a module here)
//! registers a tool, a provider, a hook and a panel; each is visible to a
//! session and the front ends after `ext::install`.

mod agent_support;
mod hooks_support;

use std::sync::{Arc, Once};

use agent_support::{env, text_reply, tool_reply};
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::app::Runtime;
use fast_harness_core::config::Config;
use fast_harness_core::http::ScriptedTransport;
use fast_harness_core::panels::{Host, Panels};
use fast_harness_core::tools::Registry;
use hooks_support::{bodies, turn};
use serde_json::Value;

/// The test plugin, written as a plugin crate would be.
mod plugin {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    use fast_harness_core::ext::v1::*;
    use serde_json::{json, Value};

    fast_harness_core::ext_api!(1);

    pub fn register(ext: &mut Ext) {
        ext.add_tool(Box::new(Shout));
        let file = ProviderFile::new(vec![ModelFile::new("m1")]);
        ext.add_provider(Arc::new(Plug), file);
        ext.add_hook(Box::new(Checked));
        ext.add_panel(Box::new(Clicks::default()));
    }

    struct Shout;

    impl Tool for Shout {
        fn spec(&self) -> ToolSpec {
            ToolSpec::new("shout", "Upper-case a text", json!({"type": "object"}))
        }

        fn run(&self, args: &Value, _ctx: &ToolCtx<'_>) -> ToolOutput {
            ToolOutput::ok(args["text"].as_str().unwrap_or("").to_uppercase())
        }
    }

    struct Plug;

    impl Provider for Plug {
        fn id(&self) -> &str {
            "plug"
        }

        fn stream(
            &self,
            _http: &dyn Transport,
            _auth: &Auth,
            _req: &ChatRequest<'_>,
            _on_delta: &mut dyn FnMut(Delta<'_>),
        ) -> Result<Completion, LlmError> {
            unreachable!("only routed in this test")
        }
    }

    /// PostToolUse of `shout`: appends ` [checked]` to the output.
    struct Checked;

    impl Hook for Checked {
        fn name(&self) -> &str {
            "checked"
        }

        fn matches(&self, event: HookEvent, tool: Option<&str>) -> bool {
            event == HookEvent::PostToolUse && tool == Some("shout")
        }

        fn run(&self, _event: HookEvent, input: &Value) -> Result<Answer, String> {
            let output = input["output"].as_str().unwrap_or("");
            Ok(Answer::default().with_output(format!("{output} [checked]")))
        }
    }

    #[derive(Default)]
    struct Clicks(AtomicU64);

    impl Clicks {
        fn view(&self) -> Vec<Incoming> {
            let text = format!("clicked {}", self.0.load(Ordering::Relaxed));
            vec![Incoming::View(json!({"type": "column", "children": [
                {"type": "text", "id": "t", "text": text},
                {"type": "button", "id": "b", "label": "Click"}]}))]
        }
    }

    impl Panel for Clicks {
        fn id(&self) -> &str {
            "clicks"
        }

        fn title(&self) -> &str {
            "Clicks"
        }

        fn open(&self, _host: Host<'_>) -> Vec<Incoming> {
            self.view()
        }

        fn event(&self, kind: &str, id: &str, _value: &Value) -> Vec<Incoming> {
            if (kind, id) == ("click", "b") {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
            self.view()
        }
    }
}

fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let mut ext = fast_harness_core::ext::Ext::new();
        plugin::register(&mut ext);
        assert!(fast_harness_core::ext::install(ext));
    });
}

#[test]
fn api_version_is_one() {
    assert_eq!(fast_harness_core::ext::API_VERSION, 1);
    assert!(fast_harness_core::ext::Ext::new().is_empty());
}

/// Every constructible v1 type is built through `ext::v1` alone (PLAN.md
/// D57): no struct literal, so a later field does not break this code.
mod constructible {
    use std::path::Path;
    use std::sync::atomic::AtomicBool;

    use fast_harness_core::ext::v1::*;
    use serde_json::json;

    #[test]
    fn every_v1_type_builds_without_a_literal() {
        let spec = ToolSpec::new("t", "a tool", json!({"type": "object"}));
        let messages = [
            Message::user("hi"),
            Message::new(Role::Tool, vec![Block::tool_result("c1", "ok", false)]),
            Message::assistant("done").with_model("p/m"),
        ];
        let cancel = AtomicBool::new(false);
        let tools = [spec.clone()];
        let req = ChatRequest::new("p/m", "system", &messages, &tools, 1000, &cancel)
            .with_effort(Some("low"))
            .with_cache(Cache::new(true, false))
            .with_session("s1");
        assert_eq!((req.max_tokens, req.session), (1000, "s1"));
        let usage = Usage::new(1, 2, 3, 4).with_cache_write_1h(1);
        let mut done = Completion::new(messages[2].clone(), usage, StopReason::End);
        done.timing = Timing::new(1, 2, 3);
        done.head = HttpHead::new(200, vec![("retry-after".into(), "1".into())]);
        let error = LlmError::new(ErrorKind::RateLimit, "slow down")
            .with_status(429)
            .with_head(done.head.clone());
        assert_eq!(error.retry_after_ms(), Some(1000));
        let _ = HttpRequest::new("GET", "http://x", &[], b"").with_timeout_ms(5);
        let _ = HttpError::new(HttpErrorKind::Timeout, "late");
        let mut model = ModelFile::new("m1");
        (model.context_window, model.price) = (Some(1000), Some(Price::new(1.0, 2.0)));
        model.compaction = CompactionPatch::default();
        let mut file = ProviderFile::new(vec![model]);
        file.key_env = Some("KEY".into());
        let answer = Answer::default()
            .with_block("no")
            .with_arguments(json!({}))
            .with_output("out")
            .with_context("ctx")
            .with_cont("more");
        assert_eq!(answer.cont.as_deref(), Some("more"));
        let session = ToolSession::new(None);
        let ctx = ToolCtx::new(Path::new("."), &cancel)
            .with_budget(100)
            .with_call_id("c1")
            .with_session(&session);
        assert_eq!((ctx.budget, ctx.call_id), (100, "c1"));
        let _ = (
            ToolOutput::ok("a"),
            ToolOutput::err("b"),
            ToolOutput::new("c", true),
        );
        let _ = (
            Host::new(Path::new("."), "s1", true),
            Place::Tab,
            Effect::Read,
        );
        let _ = (Auth::ApiKey("k".into()), Delta::Text("x"), Flow::Continue);
        let _ = (
            Incoming::Notify("n".into()),
            HookEvent::Stop,
            TurnOutcome::Done,
        );
        let _ = AgentEvent::TurnStart { turn: 1 };
    }
}

#[test]
fn tool_and_hook_reach_a_session() {
    install();
    let env = env("ext-session");
    let runtime = Runtime::load_at(env.home.clone()).unwrap();
    let names: Vec<String> = (runtime.extensions.registry().specs().iter())
        .map(|spec| spec.name.clone())
        .collect();
    assert!(names.contains(&"shout".to_string()), "{names:?}");
    let transport = Arc::new(ScriptedTransport::new(vec![
        tool_reply(&[("c1", "shout", r#"{"text":"hi"}"#)]),
        text_reply("done"),
    ]));
    let mut parts = agent_support::parts(&transport, agent_support::key_auth(), &env.cwd);
    parts.config = Config::default();
    parts.extensions = Arc::clone(&runtime.extensions);
    let mut registry = Registry::new();
    runtime.extensions.add_commands(&mut registry);
    let agent = Agent::new(parts, Arc::new(registry));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    turn(&agent, &mut conv, "go");
    let message = bodies(&transport)[1]["messages"][2].to_string();
    assert!(message.contains("HI [checked]"), "{message}");
}

#[test]
fn provider_and_panel_are_listed() {
    install();
    let env = env("ext-listed");
    let runtime = Runtime::load_at(env.home.clone()).unwrap();
    let Ok((provider, _)) = runtime.route("plug/m1") else {
        panic!("plug/m1 does not route");
    };
    assert_eq!(provider.id(), "plug");
    let mut panels = Panels::new(&runtime.panels);
    let panel = panels
        .get_mut("clicks")
        .expect("the plugin panel is listed");
    assert!(!panel.started());
    panel.open(Host::new(&env.cwd, "s", true), Arc::new(|| {}));
    assert!(panel.poll());
    let text = |panel: &fast_harness_core::panels::Panel| format!("{:?}", panel.rows);
    assert!(text(panel).contains("clicked 0"), "{}", text(panel));
    panel.send("click", "b", Value::Null);
    assert!(panel.poll());
    assert!(text(panel).contains("clicked 1"), "{}", text(panel));
}
