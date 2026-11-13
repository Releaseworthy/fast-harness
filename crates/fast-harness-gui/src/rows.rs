//! Core types to the Slint row structs.

use fast_harness_core::accounts::model::{Account, AccountKind};
use fast_harness_core::accounts::pool::AccountStatus;
use fast_harness_core::accounts::usage::{current_usage, Window};
use fast_harness_core::session::SessionSummary;
use fast_harness_core::view::{tool_detail, Approvals, Item, NoteKind};
use slint::{SharedString, StyledText};

use crate::format::{ago, duration, fraction, tokens, window_caption};
use crate::tool_rows::tool_view;
use crate::trace_rows::expanded;
use crate::{AccountRow, MsgRow, SessionRow};

/// Assistant text as styled text; plain text when the markdown is refused.
pub fn markdown(text: &str) -> StyledText {
    StyledText::from_markdown(text).unwrap_or_else(|_| StyledText::from_plain_text(text))
}

/// Every drawn text goes through [xpanded]: the software renderer lays a
/// Text out in `i16` pixels, so a longer one draws its lines over each other
/// (`trace_rows::EXPANDED_LINES`). Copy takes the whole `text`.
#[cfg(test)]
pub fn msg_row(item: &Item) -> MsgRow {
    msg_row_with(item, &Approvals::default())
}

/// [`msg_row`] with a tool call's approval word (D64). A call the gate ended
/// (`declined`, `cancelled`, `paused`, `refused`) never ran: its row carries
/// the word, no error edge, and the status pill says `not run`.
pub fn msg_row_with(item: &Item, approvals: &Approvals) -> MsgRow {
    let row = msg_row_plain(item);
    match item {
        Item::Tool { id, .. } => {
            let kept = approvals.kept_from_running(id);
            MsgRow {
                approval: approvals.word(id).unwrap_or_default().into(),
                not_run: kept,
                error: row.error && !kept,
                ..row
            }
        }
        _ => row,
    }
}

fn msg_row_plain(item: &Item) -> MsgRow {
    let row = |kind: &str, text: &str| MsgRow {
        kind: kind.into(),
        text: text.into(),
        shown: expanded(text).into(),
        ..MsgRow::default()
    };
    match item {
        Item::User { text, .. } => row("user", text),
        Item::Assistant { text, thinking, .. } => MsgRow {
            body: markdown(&expanded(text)),
            thinking: expanded(thinking).into(),
            ..row("assistant", text)
        },
        Item::Tool {
            name,
            arguments,
            summary,
            output,
            is_error,
            duration_ms,
            live,
            ..
        } => {
            let detail = tool_detail(name, arguments, output.as_deref(), *is_error, *duration_ms);
            MsgRow {
                title: name.into(),
                summary: summary.into(),
                output: expanded(output.as_deref().unwrap_or("")).into(),
                // The running tool's newest output lines (`view::LiveTail`).
                live: live.as_ref().map(|tail| tail.text()).unwrap_or_default().into(),
                running: output.is_none(),
                error: *is_error,
                tool: tool_view(name, &detail),
                ..row("tool", output.as_deref().unwrap_or(""))
            }
        }
        Item::Note { kind, text } => MsgRow {
            tone: tone(*kind).into(),
            ..row("note", text)
        },
        Item::StepFooter { .. } => row("footer", &footer_text(item)),
    }
}

fn tone(kind: NoteKind) -> &'static str {
    match kind {
        NoteKind::Retry => "retry",
        NoteKind::AccountSwitch => "switch",
        NoteKind::Error => "error",
        NoteKind::Info => "info",
    }
}

/// The dim line under a step; empty for any other item.
pub fn footer_text(item: &Item) -> String {
    let Item::StepFooter {
        turn,
        step,
        model,
        account,
        usage,
        timing,
        ..
    } = item
    else {
        return String::new();
    };
    let parts = [
        format!("turn {turn}"),
        format!("step {step}"),
        model.clone(),
        account.clone(),
        format!("TTFB {}", duration(timing.ttfb_ms)),
        format!("total {}", duration(timing.total_ms)),
        format!("in {} out {}", tokens(usage.input), tokens(usage.output)),
        format!(
            "cache read {} write {}",
            tokens(usage.cache_read),
            tokens(usage.cache_write)
        ),
    ];
    join_parts(&parts)
}

/// The parts with text, joined by ` | `: never an empty part or a doubled separator.
pub fn join_parts(parts: &[String]) -> String {
    let kept: Vec<&str> = parts
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();
    kept.join(" | ")
}

/// Email, else name, else id.
pub fn account_label(account: &Account) -> String {
    let named = account.email.as_deref().or(account.name.as_deref());
    named
        .filter(|s| !s.is_empty())
        .unwrap_or(&account.id)
        .to_string()
}

fn kind_word(account: &Account) -> &'static str {
    match (account.provider, account.kind) {
        (_, AccountKind::Own) => "own",
        (_, AccountKind::ApiKey) => "key",
    }
}

/// Bar fill and caption of one window; `no data` when it is unknown.
pub fn window_cell(window: Option<&Window>, now_ms: i64) -> (f32, SharedString) {
    match window {
        Some(w) => (
            fraction(w.utilization),
            window_caption(w.utilization, w.resets_at, now_ms).into(),
        ),
        None => (0.0, "no data".into()),
    }
}

pub fn account_row(status: &AccountStatus, now_ms: i64) -> AccountRow {
    let usage = status.usage.as_ref().map(|u| current_usage(u, now_ms));
    let five = window_cell(usage.as_ref().and_then(|u| u.five_hour.as_ref()), now_ms);
    let seven = window_cell(usage.as_ref().and_then(|u| u.seven_day.as_ref()), now_ms);
    let hold = match status.held_until {
        Some(until) if until > now_ms => {
            let reason = status.hold_reason.as_deref().unwrap_or("held");
            format!(
                "On hold ({reason}), {} left",
                crate::format::countdown(until, now_ms)
            )
        }
        _ => String::new(),
    };
    AccountRow {
        id: status.account.id.as_str().into(),
        name: account_label(&status.account).into(),
        kind: kind_word(&status.account).into(),
        plan: status.account.plan.clone().unwrap_or_default().into(),
        active: status.active,
        five: five.0,
        five_text: five.1,
        seven: seven.0,
        seven_text: seven.1,
        hold: hold.into(),
        token_ok: status.token_ok,
    }
}

pub fn session_row(summary: &SessionSummary, current: Option<&str>, now_ms: u64) -> SessionRow {
    let title = if summary.title.trim().is_empty() {
        "(no message yet)"
    } else {
        summary.title.as_str()
    };
    SessionRow {
        id: summary.id.as_str().into(),
        title: title.into(),
        time: ago(summary.last_time, now_ms).into(),
        current: current == Some(summary.id.as_str()),
    }
}

#[cfg(test)]
#[path = "rows_tests.rs"]
mod tests;
