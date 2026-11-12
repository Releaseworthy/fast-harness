//! Phase 10 experiment E5 (docs/perf.md): UI-thread time per text delta at
//! 500 transcript items, through `Transcript::apply` and `Mirror::sync` into a
//! `VecModel` (the path `turn::deliver` runs; no window needed). Ignored: run
//! `cargo test --release -p fast-harness-gui delta_time -- --ignored --nocapture`.

use super::*;
use fast_harness_core::events::AgentEvent;
use fast_harness_core::view::Transcript;
use std::time::{Duration, Instant};

const ITEMS: usize = 500;
const DELTAS: usize = 200;

/// A transcript of at least `ITEMS` items: user, answer, tool call and result.
fn history() -> Transcript {
    let mut view = Transcript::new();
    let mut n = 0;
    while view.items.len() < ITEMS {
        n += 1;
        view.push_user(format!("question {n}: please look at file {n}.rs"));
        view.apply(&AgentEvent::TextDelta("Let me check. ".repeat(20)));
        let id = format!("t{n}");
        view.apply(&AgentEvent::ToolCall {
            id: id.clone(),
            name: "read".into(),
            arguments: format!("{{\"path\":\"src/{n}.rs\"}}"),
        });
        view.apply(&AgentEvent::ToolResult {
            id,
            content: "fn main() {}\n".repeat(40),
            is_error: false,
            duration_ms: 2,
        });
    }
    view
}

/// Times `DELTAS` deltas of 24 bytes; `from` is the run's `turn_from`.
fn run(view: &mut Transcript, from: usize) -> Vec<Duration> {
    let model = VecModel::<MsgRow>::default();
    let mut mirror = Mirror::default();
    mirror.sync(&view.items, &model, 0);
    let chunk = "word word word word wor ".to_string();
    (0..DELTAS)
        .map(|_| {
            let start = Instant::now();
            view.apply(&AgentEvent::TextDelta(chunk.clone()));
            mirror.sync(&view.items, &model, from);
            start.elapsed()
        })
        .collect()
}

fn report(label: &str, mut times: Vec<Duration>) {
    let total: Duration = times.iter().sum();
    times.sort();
    let us = |d: Duration| d.as_secs_f64() * 1e6;
    println!(
        "{label}: mean {:.1} us, median {:.1} us, p95 {:.1} us, max {:.1} us",
        us(total) / DELTAS as f64,
        us(times[DELTAS / 2]),
        us(times[DELTAS * 95 / 100]),
        us(times[DELTAS - 1])
    );
}

#[test]
#[ignore = "measurement, run by hand"]
fn delta_time_at_500_items() {
    let mut view = history();
    let items = view.items.len();
    view.push_user("new turn");
    let from = view.items.len() - 1;
    report(&format!("{items} items, new turn"), run(&mut view, from));
    let mut one_turn = history();
    report(&format!("{items} items, one turn (from 0)"), run(&mut one_turn, 0));
}
