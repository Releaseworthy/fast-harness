//! Every session log under `FAST_HARNESS_ROUNDTRIP_DIR` parses to records
//! that write back byte for byte (no key-order normalisation: the writer
//! always wrote `data` keys sorted). Unset: the test does nothing. Point it
//! at a copy of the logs, never at the real home.

use super::*;
use std::collections::BTreeMap;

fn logs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            logs(&path, out);
        } else if path.extension().is_some_and(|e| e == "jsonl") {
            out.push(path);
        }
    }
}

/// `(lines, raw lines by type, differing lines)` of one log; `None` when
/// its first line is not a session header.
fn check(path: &Path, raw: &mut BTreeMap<String, usize>, bad: &mut Vec<String>) -> Option<usize> {
    let text = std::fs::read_to_string(path).ok()?;
    if !text.starts_with("{\"type\":\"session\"") {
        return None;
    }
    let mut lines = 0;
    for (index, line) in text.lines().enumerate().skip(1) {
        let Ok(event) = serde_json::from_str::<TraceEvent>(line) else {
            continue; // a damaged line: the reader skips it too
        };
        lines += 1;
        let record = event.record();
        if let Record::Raw { kind, .. } = &record {
            *raw.entry(kind.clone()).or_default() += 1;
        }
        let back = TraceEvent {
            kind: record.kind().to_string(),
            seq: event.seq,
            time: event.time,
            data: record.data().unwrap(),
        };
        if serde_json::to_string(&back).unwrap() != line && bad.len() < 20 {
            bad.push(format!("{}:{}: {}", path.display(), index + 1, event.kind));
        }
    }
    Some(lines)
}

#[test]
fn real_logs_round_trip_byte_for_byte() {
    let Some(dir) = std::env::var_os("FAST_HARNESS_ROUNDTRIP_DIR") else {
        return;
    };
    let mut paths = Vec::new();
    logs(Path::new(&dir), &mut paths);
    let (mut raw, mut bad) = (BTreeMap::new(), Vec::new());
    let (mut files, mut lines) = (0, 0);
    for path in &paths {
        if let Some(count) = check(path, &mut raw, &mut bad) {
            files += 1;
            lines += count;
        }
    }
    println!("round trip: {files} logs, {lines} lines, raw by type {raw:?}");
    assert!(bad.is_empty(), "lines that changed:\n{}", bad.join("\n"));
}
