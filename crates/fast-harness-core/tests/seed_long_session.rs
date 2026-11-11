//! Seeds a session with a long chat for a real-window scroll check (ignored):
//! `FAST_HARNESS_SEED_HOME=<fresh dir> FAST_HARNESS_SEED_CWD=<dir> cargo test
//! -p fast-harness-core --test seed_long_session -- --ignored`.
//! It writes 40 turns, a 20 000 character tool output and a 3 000 line one.

use std::path::PathBuf;

use fast_harness_core::llm::Timing;
use fast_harness_core::session;
use fast_harness_core::trace::writer::TraceWriter;
use fast_harness_core::trace::SessionHeader;
use fast_harness_core::types::{Block, Message, Role, StopReason, Usage};

fn assistant(content: Vec<Block>) -> Message {
    Message {
        role: Role::Assistant,
        content,
        model: Some("claude-opus-4-5".into()),
    }
}

fn tool_step(log: &mut TraceWriter, turn: u32, output: &str) -> std::io::Result<()> {
    let call = Block::ToolCall {
        id: format!("c{turn}"),
        name: "bash".into(),
        arguments: r#"{"command":"cat big.txt"}"#.into(),
        signature: None,
    };
    let (usage, timing) = (Usage::default(), Timing::default());
    log.step_start(turn, 1)?;
    log.assistant_message(
        turn,
        1,
        &assistant(vec![call]),
        &usage,
        StopReason::ToolUse,
        &timing,
        "seed",
    )?;
    log.tool_call(
        turn,
        1,
        &format!("c{turn}"),
        "bash",
        r#"{"command":"cat big.txt"}"#,
    )?;
    let result = Message {
        role: Role::Tool,
        content: vec![Block::ToolResult {
            call_id: format!("c{turn}"),
            content: output.into(),
            is_error: false,
        }],
        model: None,
    };
    log.tool_result(turn, 1, &result, 5)?;
    log.step_end(turn, 1, 5)?;
    Ok(())
}

#[test]
#[ignore]
fn seed_long_session() -> std::io::Result<()> {
    let home = PathBuf::from(std::env::var("FAST_HARNESS_SEED_HOME").expect("seed home"));
    let cwd = PathBuf::from(std::env::var("FAST_HARNESS_SEED_CWD").expect("seed cwd"));
    let id = session::new_id();
    let path = session::log_path(&home, &cwd, &id);
    std::fs::create_dir_all(path.parent().expect("log folder"))?;
    let header = SessionHeader::new(&id, &cwd.to_string_lossy(), None);
    let mut log = TraceWriter::create(&path, &header)?;
    let (usage, timing) = (Usage::default(), Timing::default());
    for turn in 1..=40u32 {
        log.turn_start(turn)?;
        log.user_message(turn, &Message::user(format!("Question {turn}: show more")))?;
        if turn == 20 {
            let big: String = (0..2000).map(|i| format!("big line {i:04}\n")).collect();
            tool_step(&mut log, turn, &big[..20_000])?;
        }
        if turn == 30 {
            let huge: String = (0..3000).map(|i| format!("trace line {i:04}\n")).collect();
            tool_step(&mut log, turn, &huge)?;
        }
        let text = format!("Answer {turn}.\n\n").repeat(1 + (turn as usize * 7) % 23);
        let answer = assistant(vec![Block::Text { text }]);
        log.step_start(turn, 2)?;
        log.assistant_message(turn, 2, &answer, &usage, StopReason::End, &timing, "seed")?;
        log.step_end(turn, 2, 5)?;
        log.turn_end(turn, 2, &usage, 10, "done")?;
    }
    println!("{}", path.display());
    Ok(())
}
