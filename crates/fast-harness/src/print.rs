//! Print mode: one turn, text deltas on stdout, one-line notes on stderr.

use std::io::Write;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};

use fast_harness_core::events::{AgentEvent, TurnOutcome};
use fast_harness_core::{platform, session};

use crate::args::PrintArgs;
use crate::auth_choice::runtime;

static CANCEL: AtomicBool = AtomicBool::new(false);

/// First Ctrl+C cancels the turn; a second one ends the process.
fn on_ctrl_c() {
    if CANCEL.swap(true, Ordering::SeqCst) {
        std::process::exit(130);
    }
}

/// Longest tool argument text shown in a note.
const NOTE_CHARS: usize = 120;

pub fn run(args: PrintArgs) -> Result<ExitCode, String> {
    let mut rt = runtime()?;
    let cwd = crate::work_dir(args.cwd.clone())?;
    if let Some(model) = &args.model {
        rt.config.model = model.clone();
    }
    if let Some(effort) = &args.effort {
        rt.config.set_effort(effort);
    }
    if args.continue_latest && args.resume.is_none() {
        let latest = session::latest(&rt.home, &cwd).map_err(|err| format!("sessions: {err}"))?;
        if latest.is_none() {
            eprintln!("[note] no session to continue; starting a new one");
        }
    }
    let mut conv = rt.open_conversation(&cwd, args.resume.as_deref(), args.continue_latest)?;
    let (agent, notes) = rt.agent(&cwd, &conv)?;
    for note in notes {
        eprintln!("[note] {note}");
    }
    if let Err(err) = platform::on_interrupt(on_ctrl_c) {
        eprintln!("[note] Ctrl+C will not cancel cleanly: {err}");
    }
    let mut printer = Printer {
        session: conv.id.clone(),
        ..Printer::default()
    };
    let outcome = agent.run_turn(&mut conv, &args.prompt, &CANCEL, &mut |event| {
        printer.event(event)
    });
    if outcome == TurnOutcome::Truncated {
        eprintln!("[note] the answer was cut at the output token limit (maxTokens); `--continue` with \"continue\" goes on");
    }
    Ok(exit_code(outcome))
}

fn exit_code(outcome: TurnOutcome) -> ExitCode {
    match outcome {
        TurnOutcome::Done => ExitCode::SUCCESS,
        TurnOutcome::Failed
        | TurnOutcome::StepLimit
        | TurnOutcome::Truncated
        | TurnOutcome::ContextOverflow
        | TurnOutcome::Paused => ExitCode::FAILURE,
        TurnOutcome::Cancelled => ExitCode::from(130),
    }
}

fn clip(text: &str) -> String {
    let flat = text.replace(['\r', '\n'], " ");
    match flat.char_indices().nth(NOTE_CHARS) {
        Some((cut, _)) => format!("{}...", &flat[..cut]),
        None => flat,
    }
}

/// Writes events as they arrive; remembers whether stdout ends mid-line.
#[derive(Default)]
struct Printer {
    mid_line: bool,
    session: String,
    names: Vec<(String, String)>,
}

impl Printer {
    fn event(&mut self, event: AgentEvent) {
        if let AgentEvent::TextDelta(text) = &event {
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(text.as_bytes()).and_then(|_| out.flush());
            self.mid_line = !text.ends_with('\n');
        } else if let Some(note) = self.note(event) {
            if self.mid_line {
                println!();
                self.mid_line = false;
            }
            eprintln!("{note}");
        }
    }

    /// The stderr line for an event, if it gets one.
    fn note(&mut self, event: AgentEvent) -> Option<String> {
        Some(match event {
            AgentEvent::ToolCall {
                id,
                name,
                arguments,
            } => {
                let note = format!("[tool] {name} {}", clip(&arguments));
                self.names.push((id, name));
                note
            }
            AgentEvent::ToolResult {
                id,
                is_error: true,
                content,
                duration_ms,
            } => {
                let name = self
                    .names
                    .iter()
                    .find(|(i, _)| *i == id)
                    .map(|(_, n)| n.as_str());
                let name = name.unwrap_or("?");
                format!(
                    "[tool] {name} failed after {duration_ms} ms: {}",
                    clip(&content)
                )
            }
            AgentEvent::Retry {
                attempt,
                delay_ms,
                error,
            } => {
                format!("[retry] attempt {attempt} failed, retrying in {delay_ms} ms: {error}")
            }
            AgentEvent::AccountSwitch { from, to, reason } => {
                format!("[account] {from} -> {to} ({reason})")
            }
            AgentEvent::Error(message) => format!("[error] {message}"),
            AgentEvent::Compaction {
                summary,
                before,
                after,
                auto,
            } => {
                println!("{summary}");
                let how = if auto { "auto" } else { "manual" };
                format!("[compaction] {how}: context {before} -> about {after} tokens")
            }
            AgentEvent::TurnEnd {
                usage,
                duration_ms,
                outcome,
                ..
            } => format!(
                "[usage] in {} out {} cache read {} write {} | {:.1} s | {outcome:?} | session {}",
                usage.input,
                usage.output,
                usage.cache_read,
                usage.cache_write,
                duration_ms as f64 / 1000.0,
                self.session
            ),
            _ => return None,
        })
    }
}
