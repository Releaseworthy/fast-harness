//! `fast-harness tail` (PLAN.md D49): a session log as text lines, followed
//! while a turn runs (one size check per interval), raw with `--json`.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::process::ExitCode;

use fast_harness_core::session::lock;
use fast_harness_core::trace::kind;
use fast_harness_core::types::Message;
use serde_json::Value;

use crate::session_cmd::EVERY;

/// Longest text a tool line shows.
const TOOL_CHARS: usize = 120;

pub fn follow(log: &Path, json: bool) -> Result<ExitCode, String> {
    let mut offset = 0u64;
    loop {
        let running = lock::live_lock(log).is_some();
        offset = print_new(log, offset, json)?;
        if !running {
            return Ok(ExitCode::SUCCESS);
        }
        std::thread::sleep(EVERY);
    }
}

/// Prints the whole lines after `offset`; returns the new offset.
fn print_new(log: &Path, offset: u64, json: bool) -> Result<u64, String> {
    let len = fs::metadata(log)
        .map_err(|err| format!("{}: {err}", log.display()))?
        .len();
    if len <= offset {
        return Ok(offset);
    }
    let mut file = fs::File::open(log).map_err(|err| err.to_string())?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|err| err.to_string())?;
    let mut bytes = Vec::new();
    file.take(len - offset)
        .read_to_end(&mut bytes)
        .map_err(|err| err.to_string())?;
    let Some(end) = bytes.iter().rposition(|b| *b == b'\n') else {
        return Ok(offset);
    };
    for line in String::from_utf8_lossy(&bytes[..end]).lines() {
        if json {
            println!("{line}");
        } else if let Some(text) = text_line(line) {
            println!("{text}");
        }
    }
    Ok(offset + end as u64 + 1)
}

/// One event as a text line; `None` for events a reader skips.
pub fn text_line(line: &str) -> Option<String> {
    let event: Value = serde_json::from_str(line).ok()?;
    let data = &event["data"];
    let message = || -> Option<String> {
        let message: Message = serde_json::from_value(data["message"].clone()).ok()?;
        Some(message.text())
    };
    match event["type"].as_str()? {
        kind::TURN_START => Some(format!("-- turn {}", data["turn"])),
        kind::USER_MESSAGE => message().map(|text| format!("> {text}")),
        kind::ASSISTANT_MESSAGE => message().filter(|text| !text.trim().is_empty()),
        kind::TOOL_CALL => Some(format!("[tool] {}", data["name"].as_str().unwrap_or("?"))),
        kind::TOOL_RESULT => message().map(|text| {
            let first: String = text
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(TOOL_CHARS)
                .collect();
            format!("[result] {first}")
        }),
        kind::TURN_END => Some(format!(
            "-- turn end {}",
            data["outcome"].as_str().unwrap_or("")
        )),
        _ => None,
    }
}
