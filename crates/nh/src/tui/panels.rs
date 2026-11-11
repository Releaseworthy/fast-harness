//! The accounts, trace, model and session screens.

use nh_core::accounts::model::AccountKind;
use nh_core::accounts::pool::AccountStatus;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use super::ui::{Dialog, Screen, Ui};
use super::wrap::clip;
use crate::accounts_cmd::{label, percent, relative, state};

fn selected_style() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}

/// Rows of a list with the selection kept in view.
fn list(rows: Vec<String>, selected: usize, height: usize, width: usize) -> Vec<Line<'static>> {
    let skip = (selected + 1).saturating_sub(height.max(1));
    rows.into_iter()
        .enumerate()
        .skip(skip)
        .take(height)
        .map(|(index, row)| {
            let row = clip(&row, width);
            if index == selected {
                Line::styled(row, selected_style())
            } else {
                Line::raw(row)
            }
        })
        .collect()
}

fn framed(title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(format!(" {title} "))
}

/// One accounts row: active marker, kind, name, plan, 5h and 7d, reset, state.
pub fn account_row(status: &AccountStatus, now: i64) -> String {
    let account = &status.account;
    let kind = match account.kind {
        AccountKind::Own => "own",
        AccountKind::Linked => "linked",
        AccountKind::ApiKey => "key",
    };
    let usage = status.usage.as_ref();
    let five = usage.and_then(|u| u.five_hour.as_ref());
    let seven = usage.and_then(|u| u.seven_day.as_ref());
    let five_reset = five.and_then(|w| w.resets_at).map(|at| relative(at - now));
    let seven_reset = seven.and_then(|w| w.resets_at).map(|at| relative(at - now));
    format!(
        "{} {:<6} {:<28} {:<6} 5h {:>4} {:<10} 7d {:>4} {:<10} {}",
        if status.active { "*" } else { " " },
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

fn dialog_lines(dialog: &Dialog) -> Option<Vec<Line<'static>>> {
    let yellow = Style::default().fg(Color::Yellow);
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
                format!("Remove {id} from nh? y removes, any other key keeps it"),
                yellow,
            )]
        }
        Dialog::Login(url) => vec![
            Line::styled(
                "Sign in to Claude in the browser (Esc cancels). If it did not open, visit:",
                yellow,
            ),
            Line::raw(url.clone()),
        ],
    })
}

fn draw_accounts(frame: &mut Frame, ui: &mut Ui, area: Rect, now_ms: u64) {
    let now = i64::try_from(now_ms).unwrap_or(i64::MAX);
    let dialog = dialog_lines(&ui.dialog);
    let dialog_height = if dialog.is_some() { 6 } else { 0 };
    let [top, bottom] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(dialog_height)]).areas(area);
    let inner = framed("Accounts").inner(top);
    let height = usize::from(inner.height);
    let rows: Vec<String> = ui.accounts.iter().map(|s| account_row(s, now)).collect();
    let lines = if rows.is_empty() {
        vec![Line::raw(
            "No accounts: l signs in, i imports from claude-switch, k adds an API key.",
        )]
    } else {
        list(rows, ui.selected, height, usize::from(inner.width))
    };
    frame.render_widget(Paragraph::new(lines).block(framed("Accounts")), top);
    if let Some(lines) = dialog {
        let widget = Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(framed("Account"));
        frame.render_widget(widget, bottom);
    }
}

fn draw_trace(frame: &mut Frame, ui: &mut Ui, area: Rect) {
    let block = framed("Trace: one row per step");
    let inner = block.inner(area);
    ui.view_height = usize::from(inner.height).max(1);
    let lines: Vec<Line<'static>> = if ui.trace.is_empty() {
        vec![Line::raw("No steps in this session yet.")]
    } else {
        ui.trace
            .iter()
            .skip(ui.trace_top)
            .take(ui.view_height)
            .map(|row| Line::raw(clip(row, usize::from(inner.width))))
            .collect()
    };
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_models(frame: &mut Frame, ui: &mut Ui, area: Rect) {
    let title = format!(
        "Model (next turn): {} · effort {}",
        ui.model,
        ui.effort.as_deref().unwrap_or("default")
    );
    let block = framed(&title);
    let inner = block.inner(area);
    let rows = ui
        .models
        .iter()
        .map(|m| {
            let mark = if m.id == ui.model { "*" } else { " " };
            let ctx = m.context_window / 1000;
            format!("{mark} {:<28} {:<24} {ctx}k context", m.id, m.display_name)
        })
        .collect();
    let lines = list(
        rows,
        ui.selected,
        usize::from(inner.height),
        usize::from(inner.width),
    );
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_sessions(frame: &mut Frame, ui: &mut Ui, area: Rect) {
    let block = framed("Sessions of this directory, newest first");
    let inner = block.inner(area);
    let rows: Vec<String> = ui
        .sessions
        .iter()
        .map(|s| format!("{:<28} {:>3} turns  {}", s.id, s.turns, s.title))
        .collect();
    let lines = if rows.is_empty() {
        vec![Line::raw("No sessions yet.")]
    } else {
        list(
            rows,
            ui.selected,
            usize::from(inner.height),
            usize::from(inner.width),
        )
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
