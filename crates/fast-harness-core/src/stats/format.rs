//! The text form of [`Stats`] for `fast-harness stats`.

use std::fmt::Write;

use super::Stats;
use crate::view::short_tokens;

/// `1234567` as `1,234,567`.
fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

fn percent(value: Option<u64>) -> String {
    value.map_or_else(|| "-".into(), |v| format!("{v}%"))
}

fn tokens_lines(out: &mut String, s: &Stats) {
    let u = &s.usage;
    let _ = writeln!(
        out,
        "sessions {}, turns {}, steps {}",
        s.sessions, s.turns, s.steps
    );
    let _ = writeln!(
        out,
        "prompt tokens {} (input {}, cache read {}, cache write {}, of it 1h {}), output {}",
        grouped(s.prompt_tokens),
        grouped(u.input),
        grouped(u.cache_read),
        grouped(u.cache_write),
        grouped(u.cache_write_1h),
        grouped(u.output)
    );
    let _ = writeln!(out, "cache hit rate {}", percent(s.cache_hit_percent));
    let _ = writeln!(
        out,
        "cache rewrites after an idle gap {} ({} tokens)",
        s.idle_rewrites,
        grouped(s.idle_rewrite_tokens)
    );
    let window = match s.window {
        0 => "unknown window".to_string(),
        w => format!("{}% of {}", s.peak_context * 100 / w, short_tokens(w)),
    };
    let _ = writeln!(out, "peak context {} ({window})", grouped(s.peak_context));
    if s.priced_steps > 0 {
        let dollars = s.cost_micro_usd as f64 / 1_000_000.0;
        let _ = writeln!(
            out,
            "cost ${dollars:.4} ({} of {} steps priced)",
            s.priced_steps, s.steps
        );
    }
}

fn count_lines(out: &mut String, s: &Stats) {
    let _ = writeln!(out, "find_tools steps {}", s.find_steps);
    let _ = writeln!(out, "clipped or spilled results {}", s.clipped);
    let _ = writeln!(out, "compactions {}", s.compactions);
    let _ = writeln!(
        out,
        "transformations {} (prefix_binding_mismatch {})",
        s.transformations, s.prefix_binding_mismatches
    );
    signal_lines(out, s);
}

/// The free signals of PLAN.md D50.
fn signal_lines(out: &mut String, s: &Stats) {
    let g = &s.signals;
    let per_turn = match s.turns {
        0 => "-".to_string(),
        turns => format!("{:.1}", s.steps as f64 / turns as f64),
    };
    let _ = writeln!(
        out,
        "steps per turn {per_turn} (most {}), cache rewrites {}",
        g.max_turn_steps, g.cache_rewrites
    );
    let _ = writeln!(
        out,
        "repeated reads {}, not loaded yet {}, failed edits {}, failed shell commands {}",
        g.repeated_reads, g.not_loaded, g.failed_edits, g.shell_failures
    );
    let list = |items: &[super::NameCount]| {
        let parts: Vec<String> = items
            .iter()
            .map(|i| format!("{} x{}", i.name, i.count))
            .collect();
        parts.join(", ")
    };
    if !g.repeated_files.is_empty() {
        let _ = writeln!(out, "  re-read: {}", list(&g.repeated_files));
    }
    if !g.failed_commands.is_empty() {
        let _ = writeln!(out, "  non-zero exits: {}", list(&g.failed_commands));
    }
}

fn tool_lines(out: &mut String, s: &Stats) {
    if s.tools.is_empty() {
        return;
    }
    let _ = writeln!(out, "tool result bytes:");
    for tool in &s.tools {
        let _ = writeln!(
            out,
            "  {:<14} {:>5} results {:>12} bytes",
            tool.name,
            tool.results,
            grouped(tool.bytes)
        );
    }
    let _ = writeln!(out, "largest results:");
    for result in &s.largest {
        let _ = writeln!(
            out,
            "  {:<14} turn {} step {} {:>12} bytes  {}",
            result.name,
            result.turn,
            result.step,
            grouped(result.bytes),
            result.session
        );
    }
}

/// The numbers as aligned text lines.
pub fn format_text(stats: &Stats) -> String {
    let mut out = String::new();
    tokens_lines(&mut out, stats);
    count_lines(&mut out, stats);
    tool_lines(&mut out, stats);
    out
}
