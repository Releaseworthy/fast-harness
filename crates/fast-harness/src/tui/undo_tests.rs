//! `/undo` in the TUI (PLAN.md D69 "Views"): never sent to the model, the
//! outcome shown as notes, a conflict refused until `/undo force`, a steer
//! or queued `/undo` intercepted, and the hint bar.

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
use fast_harness_core::service::Host;
use fast_harness_core::view::{Item, NoteKind};
use serde_json::{json, Value};

use super::app::{Msg, Tui};

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
        let mut agent = Agent::new(parts, Arc::new(fast_harness_core::tools::builtin()));
        agent.preload = vec!["write".into()];
        Ok((agent, Vec::new()))
    }
}

fn reply(block: Value, delta: Value, stop: &str) -> ScriptedResponse {
    let events: [Value; 6] = [
        json!({"type": "message_start", "message": {"id": "m1", "type": "message",
            "role": "assistant", "content": [], "model": "claude-x",
            "usage": {"input_tokens": 10, "output_tokens": 1}}}),
        json!({"type": "content_block_start", "index": 0, "content_block": block}),
        json!({"type": "content_block_delta", "index": 0, "delta": delta}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "message_delta", "delta": {"stop_reason": stop},
            "usage": {"output_tokens": 5}}),
        json!({"type": "message_stop"}),
    ];
    let body: String = (events.iter())
        .map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap_or("")))
        .collect();
    ScriptedResponse::new(200, &body, 7)
}

/// A turn that writes `content` to `u.txt`, then says done.
fn write_turn(content: &str) -> Vec<ScriptedResponse> {
    let args = json!({"path": "u.txt", "content": content}).to_string();
    let call = json!({"type": "tool_use", "id": "c1", "name": "write", "input": {}});
    let text = json!({"type": "text", "text": ""});
    vec![
        reply(
            call,
            json!({"type": "input_json_delta", "partial_json": args}),
            "tool_use",
        ),
        reply(
            text,
            json!({"type": "text_delta", "text": "done"}),
            "end_turn",
        ),
    ]
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
        std::env::temp_dir().join(format!("fh-tui-undo-{tag}-{}-{nanos}", std::process::id()));
    let (home, cwd) = (root.join("home"), root.join("work"));
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::write(cwd.join("u.txt"), "before").unwrap();
    let transport = Arc::new(ScriptedTransport::new(write_turn("after")));
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
    fn file(&self) -> String {
        std::fs::read_to_string(self.tui.cwd.join("u.txt")).unwrap()
    }

    /// Applies messages until `done` holds.
    fn until(&mut self, done: impl Fn(&Tui) -> bool) {
        while !done(&self.tui) {
            let msg = self.rx.recv_timeout(Duration::from_secs(20));
            self.tui.handle(msg.expect("a message within 20 s"));
        }
    }

    /// Sends `text` and applies messages until the undo it started ended.
    fn undo(&mut self, text: &str) {
        self.tui.send(text.into());
        self.until(|tui| !tui.ui.status.starts_with("undoing"));
    }

    fn turn(&mut self) {
        self.tui.send("change u".into());
        self.until(|tui| !tui.ui.running);
        assert_eq!(self.file(), "after");
    }

    fn notes(&self) -> Vec<(NoteKind, String)> {
        (self.tui.transcript.items.iter())
            .filter_map(|item| match item {
                Item::Note { kind, text } => Some((*kind, text.clone())),
                _ => None,
            })
            .collect()
    }

    fn users(&self) -> usize {
        let user = |item: &&Item| matches!(item, Item::User { .. });
        self.tui.transcript.items.iter().filter(user).count()
    }
}

impl Drop for Case {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn undo_is_not_sent_to_the_model_and_its_outcome_is_shown() {
    let mut case = case("plain");
    case.turn();
    assert_eq!(
        case.tui.ui.undo.as_ref().map(|i| (i.turn, i.files)),
        Some((1, 1))
    );
    assert!(super::keys::hint_text(&case.tui.ui).starts_with("/undo turn 1 (1 file) · "));
    let (requests, users) = (case.transport.requests().len(), case.users());
    case.undo("/undo");
    assert_eq!(case.file(), "before");
    assert_eq!(case.transport.requests().len(), requests, "no model call");
    assert_eq!(case.users(), users, "no user message");
    let notes = case.notes();
    let last = &notes[notes.len() - 2..];
    assert_eq!(last[0].0, NoteKind::Info);
    assert!(
        last[0]
            .1
            .starts_with("undid the file changes of turn 1: restored "),
        "{last:?}"
    );
    assert!(last[1].1.contains("not tracked"), "{last:?}");
    assert!(case.tui.ui.undo.is_none(), "nothing left to undo");
    assert!(!super::keys::hint_text(&case.tui.ui).contains("/undo"));
}

#[test]
fn a_conflict_is_refused_until_undo_force() {
    let mut case = case("conflict");
    case.turn();
    std::fs::write(case.tui.cwd.join("u.txt"), "by hand").unwrap();
    case.undo("/undo");
    assert_eq!(case.file(), "by hand", "a conflict changes nothing");
    let refused = case.notes().into_iter().rev().nth(1).unwrap();
    assert_eq!(refused.0, NoteKind::Error);
    assert!(
        refused.1.contains("changed since that turn: "),
        "{refused:?}"
    );
    assert!(refused.1.contains("u.txt"), "{refused:?}");
    assert!(case.tui.ui.undo.is_some(), "the turn can still be undone");
    case.undo("/undo force");
    assert_eq!(case.file(), "before");
    let done = case.notes().into_iter().rev().nth(1).unwrap();
    assert!(done.1.ends_with("(forced over later changes)"), "{done:?}");
}

#[test]
fn a_steered_or_queued_undo_never_reaches_the_model() {
    let mut case = case("queued");
    case.turn();
    let requests = case.transport.requests().len();
    case.tui.steer("/undo".into());
    assert_eq!(case.notes().last().unwrap().1, super::undo::RUNNING);
    assert!(case.tui.current.status().queued.is_empty(), "not queued");
    case.tui.current.queue("/undo".into());
    case.tui.next_queued();
    case.until(|tui| !tui.ui.status.starts_with("undoing"));
    assert_eq!(case.file(), "before");
    assert_eq!(case.transport.requests().len(), requests, "no model call");
}
