//! `session::children` (PLAN.md D74): the logs whose header names the parent,
//! their last turn, and the label from the parent's `subagent` call.

use super::*;
use crate::agent::Conversation;
use crate::trace::record::{StepStart, ToolCall, ToolResult, TurnEnd, TurnStart, UserMessage};
use crate::types::{Block, Message, Role};

fn temp() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("fh-children-{}", crate::session::new_id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn write(conv: &mut Conversation, record: Record) {
    conv.trace.write(&record).expect("trace write");
}

/// A child of `parent` whose turn ran `steps` steps, the last calling
/// `tool`, and ended with `outcome` (none: still running).
fn kid(home: &Path, cwd: &Path, parent: &str, outcome: Option<&str>) -> String {
    let mut conv = Conversation::create_at(home, cwd, Some(parent), 1).expect("child");
    write(
        &mut conv,
        Record::TurnStart(TurnStart {
            turn: 1,
            resume: false,
        }),
    );
    let message = Message::user("Review the GUI and its view model");
    write(
        &mut conv,
        Record::UserMessage(UserMessage {
            turn: 1,
            message,
            kind: None,
        }),
    );
    for step in 1..=3 {
        write(&mut conv, Record::StepStart(StepStart { turn: 1, step }));
    }
    let (call_id, name, arguments) = ("c1".into(), "grep".into(), "{}".into());
    write(
        &mut conv,
        Record::ToolCall(ToolCall {
            turn: 1,
            step: 3,
            call_id,
            name,
            arguments,
        }),
    );
    if let Some(outcome) = outcome {
        let outcome = outcome.to_string();
        write(
            &mut conv,
            Record::TurnEnd(TurnEnd {
                turn: 1,
                steps: 3,
                outcome,
                ..TurnEnd::default()
            }),
        );
    }
    conv.id.clone()
}

/// The parent's `subagent` call with `description` and its result naming
/// session `child`.
fn call(conv: &mut Conversation, call_id: &str, description: &str, child: &str) {
    let arguments = serde_json::json!({ "description": description, "prompt": "p" }).to_string();
    let (call_id, name) = (call_id.to_string(), "subagent".to_string());
    write(
        conv,
        Record::ToolCall(ToolCall {
            turn: 1,
            step: 1,
            call_id: call_id.clone(),
            name,
            arguments,
        }),
    );
    let text = serde_json::json!({ "id": "a1", "session": child }).to_string();
    let message = Message {
        role: Role::Tool,
        content: vec![Block::tool_result(call_id, text, false)],
        ..Message::user("")
    };
    let result = ToolResult {
        turn: 1,
        step: 1,
        message,
        duration_ms: 5,
        bytes: None,
        spilled: None,
    };
    write(conv, Record::ToolResult(result));
}

#[test]
fn children_lists_the_logs_naming_the_parent_with_their_last_turn_and_label() {
    let (home, cwd) = (temp(), temp());
    let mut parent = Conversation::create_at(&home, &cwd, None, 0).expect("parent");
    let other = Conversation::create_at(&home, &cwd, None, 0).expect("other");
    let done = kid(&home, &cwd, &parent.id, Some("done"));
    let live = kid(&home, &cwd, &parent.id, None);
    let foreign = kid(&home, &cwd, &other.id, Some("failed"));
    call(&mut parent, "call-1", "Review GUI and view model", &done);
    let found = children(&home, &cwd, &parent.id).expect("children");
    let ids: Vec<&str> = found.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        ids,
        [done.as_str(), live.as_str()],
        "{foreign} is another parent's"
    );
    let first = &found[0];
    assert_eq!(first.label, "Review GUI and view model");
    assert_eq!((first.depth, first.step), (1, 3));
    assert_eq!(first.tool.as_deref(), Some("grep"));
    assert_eq!(first.outcome.as_deref(), Some("done"));
    // No call names the second child: its first prompt is the label.
    assert_eq!(found[1].label, "Review the GUI and its view model");
    assert_eq!(found[1].outcome, None);
    assert!(children(&home, &cwd, "20991231-000000-0000")
        .expect("none")
        .is_empty());
    let empty = temp();
    assert!(children(&home, &empty, &parent.id)
        .expect("no folder")
        .is_empty());
    drop((parent, other));
    for dir in [home, cwd, empty] {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn session_in_finds_the_id_after_the_word() {
    let id = "20261113-195342-6143";
    assert_eq!(
        session_in(&format!(r#"{{"session":"{id}"}}"#)).as_deref(),
        Some(id)
    );
    assert_eq!(
        session_in(&format!("done · session {id} · 3 steps")).as_deref(),
        Some(id)
    );
    assert_eq!(session_in("no session here 12"), None);
}
