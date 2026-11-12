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
        let models = vec![ModelFile {
            id: "m1".into(),
            ..ModelFile::default()
        }];
        let file = ProviderFile {
            models,
            ..ProviderFile::default()
        };
        ext.add_provider(Arc::new(Plug), file);
        ext.add_hook(Box::new(Checked));
        ext.add_panel(Box::new(Clicks::default()));
    }

    struct Shout;

    impl Tool for Shout {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: "shout".into(),
                description: "Upper-case a text".into(),
                schema: json!({"type": "object"}),
            }
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
            Ok(Answer {
                output: Some(format!("{output} [checked]")),
                ..Answer::default()
            })
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
    let host = Host {
        cwd: &env.cwd,
        session: "s",
        dark: true,
    };
    panel.open(host, Arc::new(|| {}));
    assert!(panel.poll());
    let text = |panel: &fast_harness_core::panels::Panel| format!("{:?}", panel.rows);
    assert!(text(panel).contains("clicked 0"), "{}", text(panel));
    panel.send("click", "b", Value::Null);
    assert!(panel.poll());
    assert!(text(panel).contains("clicked 1"), "{}", text(panel));
}
