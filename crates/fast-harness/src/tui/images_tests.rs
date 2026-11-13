//! `/image` in the TUI (PLAN.md D70 "Views"): a file attached, listed above
//! the input and sent with the next prompt; `/image clear`; a refusal in
//! words; the user item equal to a replay.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::Duration;

use fast_harness_core::agent::{Agent, AgentParts, Conversation};
use fast_harness_core::app::{Runtime, RuntimeHost};
use fast_harness_core::auth::StaticAuth;
use fast_harness_core::config::Config;
use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use fast_harness_core::llm::anthropic::Anthropic;
use fast_harness_core::llm::Auth;
use fast_harness_core::media;
use fast_harness_core::service::Host;
use fast_harness_core::view::{Item, NoteKind, Transcript};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use serde_json::{json, Value};

use crate::tui::app::{Msg, Tui};
use crate::tui::draw::{self, View};
use crate::tui::transcript::Cache;

struct ScriptHost {
    home: PathBuf,
    transport: Arc<ScriptedTransport>,
}

impl Host for ScriptHost {
    fn home(&self) -> PathBuf {
        self.home.clone()
    }

    fn config(&self, _cwd: &Path) -> Result<Config, String> {
        Ok(Config::default())
    }

    fn agent(
        &self,
        cwd: &Path,
        _: &Conversation,
        c: &Config,
    ) -> Result<(Agent, Vec<String>), String> {
        let parts = AgentParts {
            catalog: Default::default(),
            extensions: Default::default(),
            provider: Arc::new(Anthropic::new("http://scripted.invalid")),
            transport: self.transport.clone(),
            auth: Arc::new(StaticAuth::new("acct-1", "test", Auth::ApiKey("k".into()))),
            config: c.clone(),
            cwd: cwd.to_path_buf(),
            system: "test system".into(),
        };
        Ok((agent_of(parts), Vec::new()))
    }
}

fn agent_of(parts: AgentParts) -> Agent {
    Agent::new(parts, Arc::new(fast_harness_core::tools::builtin()))
}

/// One streamed answer `seen`, in chunks of 7 bytes.
fn answer() -> ScriptedResponse {
    let events: [Value; 6] = [
        json!({"type": "message_start", "message": {"id": "m1", "type": "message",
            "role": "assistant", "content": [], "model": "claude-x",
            "usage": {"input_tokens": 10, "output_tokens": 1}}}),
        json!({"type": "content_block_start", "index": 0,
            "content_block": {"type": "text", "text": ""}}),
        json!({"type": "content_block_delta", "index": 0,
            "delta": {"type": "text_delta", "text": "seen"}}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"},
            "usage": {"output_tokens": 5}}),
        json!({"type": "message_stop"}),
    ];
    let body: String = (events.iter())
        .map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap_or("")))
        .collect();
    ScriptedResponse::new(200, &body, 7)
}

struct Case {
    tui: Tui,
    rx: Receiver<Msg>,
    transport: Arc<ScriptedTransport>,
    root: PathBuf,
}

fn case(tag: &str) -> Case {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let root =
        std::env::temp_dir().join(format!("fh-tui-img-{tag}-{}-{nanos}", std::process::id()));
    let (home, cwd) = (root.join("home"), root.join("work"));
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::write(cwd.join("dot.png"), media::solid_png(3, 2, [200, 10, 10])).unwrap();
    std::fs::write(cwd.join("notes.txt"), "not an image").unwrap();
    let transport = Arc::new(ScriptedTransport::new(vec![answer()]));
    let rt = Runtime::load_at(home.clone()).unwrap();
    let opened = rt.open_with_events(&cwd, None, false).unwrap();
    let host = Arc::new(RuntimeHost::at_project(rt, &cwd));
    let turns = Arc::new(ScriptHost {
        home,
        transport: transport.clone(),
    });
    let (tx, rx) = mpsc::channel();
    let tui = Tui::with_host(host, turns, cwd, opened, tx).unwrap();
    Case {
        tui,
        rx,
        transport,
        root,
    }
}

impl Case {
    fn turn(&mut self, text: &str) {
        self.tui.send(text.into());
        while self.tui.ui.running || self.tui.current.running() {
            let msg = self.rx.recv_timeout(Duration::from_secs(20));
            self.tui.handle(msg.expect("a message within 20 s"));
        }
    }

    fn errors(&self) -> Vec<String> {
        (self.tui.transcript.items.iter())
            .filter_map(|item| match item {
                Item::Note {
                    kind: NoteKind::Error,
                    text,
                } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// The frame's text with the transcript left out.
    fn screen(&mut self) -> String {
        let (transcript, mut cache) = (Transcript::new(), Cache::default());
        let mut view = View {
            transcript: &transcript,
            cache: &mut cache,
            session: "s-1",
        };
        let mut term = Terminal::new(TestBackend::new(120, 20)).unwrap();
        (term.draw(|frame| draw::draw(frame, &mut self.tui.ui, &mut view, 0))).unwrap();
        let buffer = term.backend().buffer().clone();
        buffer.content().iter().map(|cell| cell.symbol()).collect()
    }
}

impl Drop for Case {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn an_attached_image_is_listed_then_sent_with_the_next_prompt() {
    let mut c = case("send");
    c.tui.send("/image dot.png".into());
    let label = "png 3x2, ".to_string();
    assert_eq!(c.tui.ui.images.len(), 1, "{:?}", c.errors());
    assert!(
        c.tui.ui.images[0].starts_with(&label),
        "{:?}",
        c.tui.ui.images
    );
    assert!(c.screen().contains("images  \u{EAEA} 1 png 3x2"));
    assert!(c.transport.requests().is_empty(), "/image is never sent");
    c.turn("what is this?");
    let body: Value = serde_json::from_str(&c.transport.requests()[0].body).unwrap();
    let messages = body["messages"].as_array().unwrap();
    let user = messages.iter().rfind(|m| m["role"] == "user").unwrap();
    let last = &user["content"];
    let image = (last.as_array().unwrap().iter()).find(|b| b["type"] == "image");
    assert_eq!(image.unwrap()["source"]["media_type"], "image/png");
    assert!(c.tui.ui.images.is_empty());
    assert!(!c.screen().contains("images  \u{EAEA}"));
    let user = (c.tui.transcript.items.iter()).find_map(|item| match item {
        Item::User { text, images } => Some((text.clone(), images.clone())),
        _ => None,
    });
    let (text, images) = user.unwrap();
    assert_eq!(text, media::user_text("what is this?", &images));
    assert!(text.starts_with("[image 1: png 3x2, "), "{text}");
    // The live item equals the replay of the session log.
    let opened = c
        .tui
        .rt()
        .open_with_events(&c.tui.cwd, Some(c.tui.session.as_str()), false);
    let replayed = crate::tui::app::load_transcript(&opened.unwrap());
    let first = replayed
        .items
        .iter()
        .find(|i| matches!(i, Item::User { .. }));
    assert_eq!(first, Some(&Item::User { text, images }));
}

#[test]
fn image_clear_drops_the_queue_and_a_refusal_says_why() {
    let mut c = case("clear");
    c.tui.send("/image \"dot.png\"".into());
    c.tui.send("/image dot.png".into());
    assert_eq!(c.tui.ui.images.len(), 2);
    c.tui.send("/image clear".into());
    assert!(c.tui.ui.images.is_empty());
    assert!(c.tui.current.attached_images().is_empty());
    c.tui.send("/image notes.txt".into());
    let errors = c.errors();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].starts_with("image not attached: "), "{errors:?}");
    assert!(c.tui.ui.images.is_empty());
    c.tui.send("/image missing.png".into());
    assert_eq!(c.errors().len(), 2);
    assert!(fast_harness_core::app::image_args("/imagex").is_none());
    assert!(c.transport.requests().is_empty());
}
