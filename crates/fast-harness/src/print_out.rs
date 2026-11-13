//! What print mode writes: text deltas on stdout, one-line notes on
//! stderr, or with `--json` one line per event (PLAN.md D48). The writers
//! are swappable so tests read the exact bytes.

use std::io::Write;
use std::sync::{Arc, Mutex};

use fast_harness_core::events::AgentEvent;
use fast_harness_core::service::{json as events, Sink};
use serde_json::Value;

/// Longest tool argument text shown in a note.
const NOTE_CHARS: usize = 120;

type Shared = Arc<Mutex<Box<dyn Write + Send>>>;

/// Stdout and stderr of print mode; every write is flushed at once.
#[derive(Clone)]
pub struct Out {
    out: Shared,
    err: Shared,
}

impl Out {
    pub fn new(out: impl Write + Send + 'static, err: impl Write + Send + 'static) -> Out {
        Out {
            out: Arc::new(Mutex::new(Box::new(out))),
            err: Arc::new(Mutex::new(Box::new(err))),
        }
    }

    /// The process's stdout and stderr.
    pub fn std() -> Out {
        Out::new(std::io::stdout(), std::io::stderr())
    }

    /// `text` on stdout as is.
    pub fn write(&self, text: &str) {
        let mut out = self.out.lock().unwrap_or_else(|p| p.into_inner());
        let _ = out.write_all(text.as_bytes()).and_then(|_| out.flush());
    }

    /// One JSON line on stdout.
    pub fn line(&self, value: &Value) {
        self.write(&format!("{value}\n"));
    }

    /// One note line on stderr.
    pub fn note(&self, text: &str) {
        let mut err = self.err.lock().unwrap_or_else(|p| p.into_inner());
        let _ = err
            .write_all(format!("{text}\n").as_bytes())
            .and_then(|_| err.flush());
    }
}

/// Print mode cannot answer approvals (PLAN.md D64): the first refused call
/// says once on stderr how to allow it.
fn refused_note(out: &Out, event: &AgentEvent, told: &mut bool) {
    if *told || !matches!(event, AgentEvent::ApprovalDone { answer, .. } if answer == "refused") {
        return;
    }
    *told = true;
    out.note("[approvals] a call was refused: \"approvals\": \"ask\" and print mode cannot answer; allow it in the \"allow\" list of ~/.fast-harness/config.json (\"commands\": command prefixes, \"paths\": path prefixes, \"tools\": tool names) or set \"approvals\": \"off\"");
}

/// The `--json` sink: one line per event (schema in docs/service.md).
pub fn json_sink(out: Out) -> Sink {
    let mut told = false;
    Box::new(move |event| {
        refused_note(&out, &event, &mut told);
        out.line(&events::event(&event));
    })
}

/// The text sink: deltas on stdout, notes on stderr.
pub fn text_sink(out: Out, session: String) -> Sink {
    let mut printer = Printer {
        out,
        mid_line: false,
        session,
        names: Vec::new(),
        told: false,
    };
    Box::new(move |event| printer.event(event))
}

fn clip(text: &str) -> String {
    let flat = text.replace(['\r', '\n'], " ");
    match flat.char_indices().nth(NOTE_CHARS) {
        Some((cut, _)) => format!("{}...", &flat[..cut]),
        None => flat,
    }
}

/// Writes events as they arrive; remembers whether stdout ends mid-line.
struct Printer {
    out: Out,
    mid_line: bool,
    session: String,
    names: Vec<(String, String)>,
    told: bool,
}

impl Printer {
    fn event(&mut self, event: AgentEvent) {
        refused_note(&self.out, &event, &mut self.told);
        if let AgentEvent::TextDelta(text) = &event {
            self.out.write(text);
            self.mid_line = !text.ends_with('\n');
        } else if let Some(note) = self.note(event) {
            if self.mid_line {
                self.out.write("\n");
                self.mid_line = false;
            }
            self.out.note(&note);
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
                let name = (self.names.iter().find(|(i, _)| *i == id)).map(|(_, n)| n.as_str());
                let name = name.unwrap_or("?");
                format!(
                    "[tool] {name} failed after {duration_ms} ms: {}",
                    clip(&content)
                )
            }
            AgentEvent::Compaction {
                summary,
                before,
                after,
                auto,
            } => {
                self.out.write(&format!("{summary}\n"));
                let how = if auto { "auto" } else { "manual" };
                format!("[compaction] {how}: context {before} -> about {after} tokens")
            }
            event => return self.plain_note(event),
        })
    }

    /// The notes that need no state of the printer.
    fn plain_note(&self, event: AgentEvent) -> Option<String> {
        Some(match event {
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
