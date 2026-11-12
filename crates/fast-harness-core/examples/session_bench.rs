//! Time of the session list (gate G10, PLAN.md D60): seeds N logs of S bytes
//! in a temporary home, then times `session::list` with no index (every log
//! parsed), with a warm index (no log read) and `session::latest`, median of
//! 3 each. The home is removed at the end.
//!
//! `cargo run --release -p fast-harness-core --example session_bench -- [logs] [kb]`

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fast_harness_core::session::{self, index::INDEX_FILE};
use fast_harness_core::trace::{self, SessionHeader, TraceWriter};
use fast_harness_core::types::Message;

/// One log of at least `bytes`: turns of a user message of about 1 KB.
fn template(home: &Path, bytes: usize) -> Vec<u8> {
    let path = home.join("template.jsonl");
    let header = SessionHeader::new("template", "bench", None);
    let mut writer = TraceWriter::create(&path, &header).expect("template");
    let text = "word ".repeat(200);
    let mut turn = 1;
    while std::fs::metadata(&path).map_or(0, |m| m.len() as usize) < bytes {
        writer.turn_start(turn).expect("write");
        writer
            .user_message(turn, &Message::user(text.as_str()))
            .expect("write");
        turn += 1;
    }
    drop(writer);
    let bytes = std::fs::read(&path).expect("read");
    std::fs::remove_file(&path).expect("remove");
    bytes
}

/// `logs` copies of the template, each with its own id in the header.
fn seed(home: &Path, cwd: &Path, logs: usize, bytes: usize) {
    let body = template(home, bytes);
    let first_line = body.iter().position(|&b| b == b'\n').expect("header") + 1;
    for n in 0..logs {
        let id = format!("20261113-{:06}-{:04x}", n / 60 * 100 + n % 60, n);
        let header = SessionHeader::new(&id, &cwd.to_string_lossy(), None);
        let mut line = serde_json::to_vec(&serde_json::json!({
            "type": "session", "version": header.version, "id": id,
            "createdAt": header.created_at + n as u64, "cwd": header.cwd,
            "harness": header.harness,
        }))
        .expect("header");
        line.push(b'\n');
        line.extend_from_slice(&body[first_line..]);
        std::fs::write(session::log_path(home, cwd, &id), line).expect("seed");
    }
}

fn median(mut f: impl FnMut()) -> Duration {
    let mut times: Vec<Duration> = (0..3)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed()
        })
        .collect();
    times.sort();
    times[1]
}

fn main() {
    let mut args = std::env::args().skip(1);
    let logs: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(200);
    let kb: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(1024);
    let home: PathBuf = std::env::temp_dir().join(format!(
        "fast-harness-session-bench-{}-{}",
        std::process::id(),
        trace::now_ms()
    ));
    let cwd = home.join("work");
    std::fs::create_dir_all(session::dir(&home, &cwd)).expect("home");
    seed(&home, &cwd, logs, kb * 1024);
    let index = session::dir(&home, &cwd).join(INDEX_FILE);
    let cold = median(|| {
        let _ = std::fs::remove_file(&index);
        let listing = session::list_counted(&home, &cwd).expect("list");
        assert_eq!(listing.parsed, logs);
    });
    let warm = median(|| {
        let listing = session::list_counted(&home, &cwd).expect("list");
        assert_eq!((listing.parsed, listing.sessions.len()), (0, logs));
    });
    let latest = median(|| {
        session::latest(&home, &cwd).expect("latest").expect("one");
    });
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    println!(
        "{logs} logs of {kb} KB: cold index {:.1} ms, warm index {:.1} ms, latest {:.2} ms",
        ms(cold),
        ms(warm),
        ms(latest)
    );
    let _ = std::fs::remove_dir_all(&home);
}
