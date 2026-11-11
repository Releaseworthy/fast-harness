//! The frame: header, transcript or a screen, input box and hint bar.

use fast_harness_core::view::{State, Transcript};
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use super::icons::lead;
use super::panels;
use super::picker::{self, model_name};
use super::theme::Styles;
use super::transcript::{Cache, Opts};
use super::ui::{Screen, Ui};
use super::wrap::clip;
use crate::accounts_cmd::{label, percent};

/// Most rows the input box grows to.
const INPUT_ROWS: usize = 8;

/// What the frame shows besides the UI state.
pub struct View<'a> {
    pub transcript: &'a Transcript,
    pub cache: &'a mut Cache,
    pub session: &'a str,
}

pub fn state_text(state: State, now_ms: u64) -> String {
    match state {
        State::Idle => "idle".into(),
        State::Waiting => "waiting".into(),
        State::Thinking => "thinking".into(),
        State::Responding => "responding".into(),
        State::RunningTools { running } => format!("running {running} tool(s)"),
        State::Retrying { until_ms } => {
            format!(
                "retrying in {}s",
                until_ms.saturating_sub(now_ms).div_ceil(1000)
            )
        }
    }
}

/// Token counts as `1.2k` and `3.4M`.
pub fn tokens(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

fn account_text(ui: &Ui) -> String {
    let Some(active) = ui.accounts.iter().find(|s| s.active) else {
        return "no account".into();
    };
    let usage = active.usage.as_ref();
    let five = percent(usage.and_then(|u| u.five_hour.as_ref()));
    let seven = percent(usage.and_then(|u| u.seven_day.as_ref()));
    let name = label(&active.account.email, &active.account.name);
    format!("{name} 5h {five} 7d {seven}")
}

/// The app name with its mark, first in the header.
pub const APP_NAME: &str = "Fast Harness";

pub fn header_text(ui: &Ui, view: &View<'_>, now_ms: u64) -> String {
    let t = view.transcript;
    format!(
        "{}{APP_NAME} │ {} {} │ {} │ {} │ {} │ in {} out {} cache r {} w {}",
        lead(ui.icons.app),
        ui.model,
        ui.effort.as_deref().unwrap_or("default"),
        account_text(ui),
        view.session,
        state_text(t.state, now_ms),
        tokens(t.totals.input),
        tokens(t.totals.output),
        tokens(t.totals.cache_read),
        tokens(t.totals.cache_write)
    )
}

fn hint_text(ui: &Ui) -> String {
    if !ui.status.is_empty() {
        return ui.status.clone();
    }
    match ui.screen {
        Screen::Chat if ui.picker.is_some() => "↑↓ PgUp PgDn choose · Enter pick · Esc close".into(),
        Screen::Chat if ui.running => "Esc cancel · Ctrl+P model · Ctrl+E effort · PgUp/PgDn scroll · Ctrl+T thinking · F3 trace · F5 theme · F6 grey".into(),
        Screen::Chat => "Enter send · Alt+Enter newline · Ctrl+P model · Ctrl+E effort · Ctrl+T thinking · Ctrl+N new · Ctrl+O sessions · F2 accounts · F3 trace · F5 theme · F6 grey".into(),
        Screen::Accounts => "Enter use · l Claude sign-in · g Google sign-in · k add key · d remove · r read usage · Esc back".into(),
        Screen::Trace if ui.trace.show_steps => "s timeline · ↑↓ PgUp PgDn Home End scroll · Esc back".into(),
        Screen::Trace if ui.trace.full => "f cut bodies · s steps table · ↑↓ PgUp PgDn Home End scroll · Esc back".into(),
        Screen::Trace => "f full bodies · s steps table · ↑↓ PgUp PgDn Home End scroll · Esc back".into(),
        Screen::Models => "↑↓ choose · Enter use model · e/Tab cycle effort · Esc back".into(),
        Screen::Sessions => "↑↓ choose · Enter resume · Esc back".into(),
    }
}

/// The tabs of a session: the current one is bold and reversed, so it
/// stands out without colour; F3 switches between them.
pub fn tab_line(ui: &Ui, st: &Styles) -> Line<'static> {
    let tabs = [
        (Screen::Chat, ui.icons.chat, "Chat"),
        (Screen::Trace, ui.icons.trace, "Trace"),
    ];
    let mut spans = Vec::new();
    for (index, (screen, icon, name)) in tabs.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled("|", st.dim()));
        }
        let style = if ui.screen == screen {
            st.base().add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            st.dim()
        };
        spans.push(Span::styled(format!(" {}{name} ", lead(icon)), style));
    }
    spans.push(Span::styled("  F3 switches", st.dim()));
    Line::from(spans)
}

/// Draws the whole frame.
pub fn draw(frame: &mut Frame, ui: &mut Ui, view: &mut View<'_>, now_ms: u64) {
    let area = frame.area();
    let st = Styles::of(ui);
    frame.render_widget(Block::default().style(st.base()), area);
    let width = usize::from(area.width);
    let (rows, cursor) = ui.input.layout(width.saturating_sub(2));
    let input_height = if ui.screen == Screen::Chat {
        rows.len().min(INPUT_ROWS) as u16 + 2
    } else {
        0
    };
    let [head, tabs, body, input, hint] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(input_height),
        Constraint::Length(1),
    ])
    .areas(area);
    let header = clip(&header_text(ui, view, now_ms), width);
    frame.render_widget(Paragraph::new(header).style(st.bar()), head);
    frame.render_widget(Paragraph::new(tab_line(ui, &st)).style(st.base()), tabs);
    match ui.screen {
        Screen::Chat => {
            draw_transcript(frame, ui, view, body);
            draw_input(frame, ui, &rows, cursor, input);
            picker::draw(frame, ui, input, body);
        }
        _ => panels::draw(frame, ui, body, now_ms),
    }
    let hint_style = if ui.status.is_empty() {
        st.hint()
    } else {
        st.status()
    };
    let text = clip(&hint_text(ui), width);
    frame.render_widget(Paragraph::new(text).style(hint_style), hint);
}

fn draw_transcript(frame: &mut Frame, ui: &mut Ui, view: &mut View<'_>, area: Rect) {
    let opts = Opts {
        width: usize::from(area.width),
        show_thinking: ui.show_thinking,
        running: ui.running,
        frame: ui.frame,
        styles: Styles::of(ui),
    };
    let lines = view.cache.lines(&view.transcript.items, opts);
    let height = usize::from(area.height);
    let first = ui.first_line(lines.len(), height);
    let visible: Vec<Line<'static>> = lines
        .iter()
        .skip(first)
        .take(height)
        .map(|l| (*l).clone())
        .collect();
    if visible.is_empty() {
        let hello = Line::styled(
            "Fast Harness: type a prompt and press Enter (F2 for accounts)",
            opts.styles.dim(),
        );
        frame.render_widget(Paragraph::new(hello), area);
        return;
    }
    frame.render_widget(Paragraph::new(visible), area);
    if ui.top.is_some() {
        let text = format!(" {} more (End) ", opts.styles.icons.more);
        let more = Span::styled(text, opts.styles.badge());
        let at = Rect::new(
            area.right().saturating_sub(15),
            area.bottom().saturating_sub(1),
            15,
            1,
        );
        frame.render_widget(Paragraph::new(Line::from(more)), at.intersection(area));
    }
}

/// The input box title: the model and effort of the next turn.
fn settings_title(ui: &Ui, st: &Styles) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!(" {}", lead(st.icons.settings)), st.dim()),
        Span::styled(model_name(&ui.model), st.accent()),
        Span::styled(" · effort ", st.dim()),
        Span::styled(
            ui.effort.clone().unwrap_or_else(|| "default".into()),
            st.accent(),
        ),
        Span::styled(" ", st.dim()),
    ])
}

fn draw_input(frame: &mut Frame, ui: &Ui, rows: &[String], cursor: (usize, usize), area: Rect) {
    let st = Styles::of(ui);
    let inner_rows = usize::from(area.height.saturating_sub(2)).max(1);
    let skip = (cursor.0 + 1).saturating_sub(inner_rows);
    let text: Vec<Line<'static>> = rows
        .iter()
        .skip(skip)
        .take(inner_rows)
        .map(|r| Line::raw(r.clone()))
        .collect();
    // The chat input has the focus: its frame is drawn in the accent colour.
    let block = st.focus_block().title(settings_title(ui, &st));
    frame.render_widget(Paragraph::new(text).block(block), area);
    let x = area.x + 1 + cursor.1.min(u16::MAX as usize) as u16;
    let y = area.y + 1 + (cursor.0 - skip).min(u16::MAX as usize) as u16;
    if x < area.right() && y < area.bottom() {
        frame.set_cursor_position(Position::new(x, y));
    }
}
