//! Core types to the Slint row structs.

use fast_harness_core::accounts::model::{Account, AccountKind};
use fast_harness_core::accounts::pool::AccountStatus;
use fast_harness_core::accounts::usage::{current_usage, Window};
use fast_harness_core::session::SessionSummary;
use fast_harness_core::view::{Item, NoteKind};
use slint::{SharedString, StyledText};

use crate::format::{ago, duration, fraction, shorten, tokens, window_caption};
use crate::{AccountRow, MsgRow, SessionRow};

/// Tool output shown in a card; Copy still takes all of it.
pub const OUTPUT_CHARS: usize = 20_000;

/// Assistant text as styled text; plain text when the markdown is refused.
pub fn markdown(text: &str) -> StyledText {
    StyledText::from_markdown(text).unwrap_or_else(|_| StyledText::from_plain_text(text))
}

pub fn msg_row(item: &Item) -> MsgRow {
    let row = |kind: &str, text: &str| MsgRow {
        kind: kind.into(),
        text: text.into(),
        ..MsgRow::default()
    };
    match item {
        Item::User { text } => row("user", text),
        Item::Assistant { text, thinking, .. } => MsgRow {
            body: markdown(text),
            thinking: thinking.into(),
            ..row("assistant", text)
        },
        Item::Tool {
            name,
            summary,
            output,
            is_error,
            ..
        } => MsgRow {
            title: name.into(),
            summary: summary.into(),
            output: shorten(output.as_deref().unwrap_or(""), OUTPUT_CHARS).into(),
            running: output.is_none(),
            error: *is_error,
            ..row("tool", output.as_deref().unwrap_or(""))
        },
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
    format!(
        "turn {turn} | step {step} | {model} | {account} | TTFB {} | total {} | in {} out {} | cache read {} write {}",
        duration(timing.ttfb_ms),
        duration(timing.total_ms),
        tokens(usage.input),
        tokens(usage.output),
        tokens(usage.cache_read),
        tokens(usage.cache_write),
    )
}

/// Email, else name, else id.
pub fn account_label(account: &Account) -> String {
    let named = account.email.as_deref().or(account.name.as_deref());
    named
        .filter(|s| !s.is_empty())
        .unwrap_or(&account.id)
        .to_string()
}

fn kind_word(kind: AccountKind) -> &'static str {
    match kind {
        AccountKind::Own => "own",
        AccountKind::ApiKey => "key",
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
        kind: kind_word(status.account.kind).into(),
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
