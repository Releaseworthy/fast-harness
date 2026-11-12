//! The accounts, trace, model and session screens.

use fast_harness_core::accounts::model::AccountKind;
use fast_harness_core::accounts::pool::AccountStatus;
use fast_harness_core::accounts::providers::{summaries, ProviderKey, ProviderSummary};
use fast_harness_core::llm::catalog::{group_title, provider_of};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use super::theme::Styles;
use super::ui::{Dialog, Screen, Ui};
use super::wrap::clip;
use crate::accounts_cmd::{label, percent, relative, state};

/// Rows of a list with the selection kept in view: the selected row has a
/// `›` marker and the selection background, never reverse video.
fn list(rows: Vec<String>, selected: usize, size: Rect, st: &Styles) -> Vec<Line<'static>> {
    let (height, width) = (usize::from(size.height), usize::from(size.width));
    let skip = (selected + 1).saturating_sub(height.max(1));
    rows.into_iter()
        .enumerate()
        .skip(skip)
        .take(height)
        .map(|(index, row)| {
            let row = clip(&row, width.saturating_sub(2));
            if index == selected {
                let fill = " ".repeat(width.saturating_sub(2 + super::wrap::text_width(&row)));
                let mark = Span::raw(format!("{} ", st.icons.pointer));
                Line::from(vec![mark, Span::raw(row), Span::raw(fill)]).style(st.selected())
            } else {
                Line::from(format!("  {row}"))
            }
        })
        .collect()
}

/// One accounts row: active marker, kind, name, plan, 5h and 7d, reset, state.
pub fn account_row(status: &AccountStatus, now: i64, mark: &str) -> String {
    let account = &status.account;
    let kind = match account.kind {
        AccountKind::Own => "own",
        AccountKind::ApiKey => "key",
    };
    let usage = status.usage.as_ref();
    let five = usage.and_then(|u| u.five_hour.as_ref());
    let seven = usage.and_then(|u| u.seven_day.as_ref());
    let five_reset = five.and_then(|w| w.resets_at).map(|at| relative(at - now));
    let seven_reset = seven.and_then(|w| w.resets_at).map(|at| relative(at - now));
    format!(
        "{} {:<6} {:<28} {:<6} 5h {:>4} {:<10} 7d {:>4} {:<10} {}",
        if status.active { mark } else { " " },
        kind,
        clip(&label(&account.email, &account.name), 28),
        clip(account.plan.as_deref().unwrap_or("-"), 6),
        percent(five),
        five_reset.unwrap_or_else(|| "-".into()),
        percent(seven),
        seven_reset.unwrap_or_else(|| "-".into()),
        state(status, now)
    )
}

/// One provider row: name, account count, status and the lowest quota left (D32).
pub fn provider_row(summary: &ProviderSummary) -> String {
    let left = summary
        .left
        .map(|l| format!(" · {l:.0}% left"))
        .unwrap_or_default();
    format!(
        "{:<20} {:<12} {}{left}",
        summary.key.name(),
        summary.count_text,
        summary.status
    )
}

/// The title, the rows and the empty-list words of the accounts screen:
/// the providers, or the accounts of the open one.
fn account_rows(ui: &Ui, now_ms: u64, mark: &str) -> (String, Vec<String>, &'static str) {
    let now = i64::try_from(now_ms).unwrap_or(i64::MAX);
    let Some(provider) = ui.account_provider else {
        let rows = summaries(&ui.accounts, now_ms)
            .iter()
            .map(provider_row)
            .collect();
        return ("Accounts: Enter opens a provider".into(), rows, "");
    };
    let title = format!("Accounts · {}", provider.name());
    match provider {
        ProviderKey::Claude => {
            let rows = ui.accounts.iter().map(|s| account_row(s, now, mark));
            let empty = "No Claude account: l signs in, k adds an API key.";
            (title, rows.collect(), empty)
        }
    }
}

/// Who a sign-in URL belongs to (only Claude signs in on this branch).
fn login_target(_url: &str) -> &'static str {
    "Claude"
}

fn dialog_lines(dialog: &Dialog, st: &Styles) -> Option<Vec<Line<'static>>> {
    let yellow = st.warning();
    Some(match dialog {
        Dialog::None => return None,
        Dialog::AddKey(input) => {
            let (rows, _) = input.layout(usize::MAX / 2);
            vec![
                Line::styled(
                    "Paste the Anthropic API key, Enter adds it, Esc cancels:",
                    yellow,
                ),
                Line::raw(format!("key: {}", rows.concat())),
            ]
        }
        Dialog::ConfirmRemove(id) => {
            vec![Line::styled(
                format!("Remove {id} from fast-harness? y removes, any other key keeps it"),
                yellow,
            )]
        }
        Dialog::Login(url) => vec![
            Line::styled(
                format!(
                    "Sign in to {} in the browser (Esc cancels). If it did not open, visit:",
                    login_target(url)
                ),
                yellow,
            ),
            Line::raw(url.clone()),
        ],
    })
}

fn draw_accounts(frame: &mut Frame, ui: &mut Ui, area: Rect, now_ms: u64) {
    let st = Styles::of(ui);
    let dialog = dialog_lines(&ui.dialog, &st);
    let dialog_height = if dialog.is_some() { 6 } else { 0 };
    let [top, bottom] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(dialog_height)]).areas(area);
    let (title, rows, empty) = account_rows(ui, now_ms, st.icons.account);
    let inner = st.block(&title).inner(top);
    let lines = if rows.is_empty() {
        vec![Line::raw(empty)]
    } else {
        list(rows, ui.selected, inner, &st)
    };
    let widget = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(widget.block(st.block(&title)), top);
    if let Some(lines) = dialog {
        let widget = Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(st.block("Account"));
        frame.render_widget(widget, bottom);
    }
}

fn draw_trace(frame: &mut Frame, ui: &mut Ui, area: Rect) {
    let st = Styles::of(ui);
    let block = st.block(ui.trace.title());
    let inner = block.inner(area);
    ui.view_height = usize::from(inner.height).max(1);
    ui.trace.render(usize::from(inner.width), &st);
    ui.trace_scroll(0);
    let lines: Vec<Line<'static>> = ui
        .trace
        .lines
        .iter()
        .skip(ui.trace_top)
        .take(ui.view_height)
        .cloned()
        .collect();
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_models(frame: &mut Frame, ui: &mut Ui, area: Rect) {
    let title = format!(
        "Model (next turn): {} · effort {}",
        ui.model,
        ui.effort.as_deref().unwrap_or("default")
    );
    let st = Styles::of(ui);
    let block = st.block(&title);
    let inner = block.inner(area);
    let (rows, selected) = model_rows(ui);
    let lines = list(rows, selected, inner, &st);
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// The Models screen rows grouped by provider (a title row before each
/// group, PLAN.md D24) and the row of the selected model.
fn model_rows(ui: &Ui) -> (Vec<String>, usize) {
    let mut rows = Vec::new();
    let mut selected = 0;
    let mut group = "";
    for (index, m) in ui.models.iter().enumerate() {
        let provider = provider_of(&m.id);
        if provider != group {
            group = provider;
            rows.push(group_title(provider).to_string());
        }
        if index == ui.selected {
            selected = rows.len();
        }
        let mark = if m.id == ui.model { "*" } else { " " };
        let ctx = m.context_window / 1000;
        rows.push(format!(
            "{mark} {:<28} {:<24} {ctx}k context",
            m.id, m.display_name
        ));
    }
    (rows, selected)
}

fn draw_sessions(frame: &mut Frame, ui: &mut Ui, area: Rect) {
    let st = Styles::of(ui);
    let block = st.block("Sessions of this directory, newest first");
    let inner = block.inner(area);
    let rows: Vec<String> = ui
        .sessions
        .iter()
        .map(|s| format!("{:<28} {:>3} turns  {}", s.id, s.turns, s.title))
        .collect();
    let lines = if rows.is_empty() && ui.sessions_loading {
        vec![Line::styled("Loading sessions...", st.dim())]
    } else if rows.is_empty() {
        vec![Line::raw("No sessions yet.")]
    } else {
        list(rows, ui.selected, inner, &st)
    };
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

pub fn draw(frame: &mut Frame, ui: &mut Ui, area: Rect, now_ms: u64) {
    match ui.screen {
        Screen::Accounts => draw_accounts(frame, ui, area, now_ms),
        Screen::Trace => draw_trace(frame, ui, area),
        Screen::Models => draw_models(frame, ui, area),
        Screen::Sessions => draw_sessions(frame, ui, area),
        Screen::Chat => {}
    }
}
