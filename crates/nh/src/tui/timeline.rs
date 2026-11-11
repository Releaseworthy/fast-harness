//! The trace screen as a readable timeline of the session log: what the
//! user sent, every model call, what the model thought and returned, the
//! tool calls and results. `f` shows full bodies, `s` the steps table.

use nh_core::trace::detail::{Entry, EntryKind};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::theme::Styles;
use super::wrap::{clip, sanitize, text_width, wrap_line};

/// Body rows shown per entry before `f` (full mode).
pub const BODY_ROWS: usize = 8;
const INDENT: &str = "    ";

/// What the trace screen shows and the lines drawn from it.
#[derive(Default)]
pub struct TraceView {
    pub entries: Vec<Entry>,
    /// The steps table, one row per step.
    pub steps: Vec<String>,
    pub error: Option<String>,
    pub full: bool,
    pub show_steps: bool,
    pub lines: Vec<Line<'static>>,
    /// Width, theme, full and steps flags of `lines`; `None` after a change.
    key: Option<(usize, &'static str, bool, bool)>,
}

impl TraceView {
    pub fn set(&mut self, entries: Vec<Entry>, steps: Vec<String>, error: Option<String>) {
        self.entries = entries;
        self.steps = steps;
        self.error = error;
        self.key = None;
    }

    /// Re-renders `lines` when the width, the theme or a toggle changed.
    pub fn render(&mut self, width: usize, st: &Styles) {
        let key = (width, st.theme.id, self.full, self.show_steps);
        if self.key == Some(key) {
            return;
        }
        self.lines = match &self.error {
            Some(err) => vec![Line::styled(clip(err, width), st.error())],
            None if self.show_steps => steps_lines(&self.steps, width, st),
            None => lines(&self.entries, self.full, width, st),
        };
        self.key = Some(key);
    }

    pub fn title(&self) -> &'static str {
        match (self.show_steps, self.full) {
            (true, _) => "Trace: steps table, one row per step",
            (false, true) => "Trace: timeline, full bodies",
            (false, false) => "Trace: timeline, bodies cut to 8 lines",
        }
    }
}

fn steps_lines(steps: &[String], width: usize, st: &Styles) -> Vec<Line<'static>> {
    if steps.is_empty() {
        return vec![Line::styled("No steps in this session yet.", st.dim())];
    }
    steps
        .iter()
        .map(|row| Line::styled(clip(row, width), st.text()))
        .collect()
}

/// The timeline lines of `entries` at `width` columns.
pub fn lines(entries: &[Entry], full: bool, width: usize, st: &Styles) -> Vec<Line<'static>> {
    if entries.is_empty() {
        return vec![Line::styled("Nothing sent in this session yet.", st.dim())];
    }
    let mut out = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        if entry.kind == EntryKind::Turn && index > 0 {
            out.push(Line::default());
        }
        out.push(title_line(entry, width, st));
        out.extend(body_lines(entry, full, width, st));
    }
    out
}

/// The mark and the styles of an entry title: (mark, mark style, title style).
fn look(entry: &Entry, st: &Styles) -> (&'static str, Style, Style) {
    let bold = st.text().add_modifier(Modifier::BOLD);
    match entry.kind {
        EntryKind::Turn => ("", st.accent(), st.accent().add_modifier(Modifier::BOLD)),
        EntryKind::User => ("› ", st.user_marker(), bold),
        EntryKind::Header => ("≡ ", st.dim(), st.dim()),
        EntryKind::Request => ("→ ", st.dim(), st.dim()),
        EntryKind::Thinking => ("✻ ", st.thinking(), st.thinking()),
        EntryKind::Response => ("◆ ", st.accent(), st.text()),
        EntryKind::ToolCall => ("▸ ", st.marker(), st.code()),
        EntryKind::ToolResult if entry.is_error => ("✗ ", st.error(), st.code()),
        EntryKind::ToolResult => ("✓ ", st.success(), st.code()),
        EntryKind::Answer => ("← ", st.dim(), st.dim()),
        EntryKind::Retry => ("↻ ", st.warning(), st.warning()),
        EntryKind::Failure => ("✗ ", st.error(), st.error()),
        EntryKind::Switch => ("⇄ ", st.warning(), st.warning()),
        EntryKind::TurnEnd => ("■ ", st.dim(), st.dim()),
    }
}

fn title_line(entry: &Entry, width: usize, st: &Styles) -> Line<'static> {
    let (mark, mark_style, style) = look(entry, st);
    if entry.kind == EntryKind::Turn {
        let title = clip(&entry.title, width.saturating_sub(4));
        let rule = "─".repeat(width.saturating_sub(text_width(&title) + 4));
        return Line::from(vec![
            Span::styled("── ", st.quote()),
            Span::styled(title, style),
            Span::styled(format!(" {rule}"), st.quote()),
        ]);
    }
    let title = clip(&entry.title, width.saturating_sub(4));
    Line::from(vec![
        Span::raw("  "),
        Span::styled(mark, mark_style),
        Span::styled(title, style),
    ])
}

fn body_style(entry: &Entry, st: &Styles) -> Style {
    match entry.kind {
        _ if entry.is_error => st.error(),
        EntryKind::User | EntryKind::Response | EntryKind::Answer => st.text(),
        EntryKind::Thinking => st.thinking(),
        EntryKind::ToolCall | EntryKind::ToolResult => st.code(),
        EntryKind::Failure => st.error(),
        _ => st.dim(),
    }
}

/// The body wrapped below the title with a 4-space indent; cut to
/// [`BODY_ROWS`] rows unless `full`. The system prompt shows only in full.
fn body_lines(entry: &Entry, full: bool, width: usize, st: &Styles) -> Vec<Line<'static>> {
    let body = entry.body.trim_end();
    if body.is_empty() || (entry.kind == EntryKind::Header && !full) {
        return Vec::new();
    }
    let style = body_style(entry, st);
    let inner = width.saturating_sub(INDENT.len()).max(1);
    let mut rows = Vec::new();
    for source in body.lines() {
        let wrapped = wrap_line(&Line::styled(sanitize(source), style), inner, 0);
        if wrapped.is_empty() {
            rows.push(Line::raw(INDENT));
        }
        for mut line in wrapped {
            line.spans.insert(0, Span::raw(INDENT));
            rows.push(line);
        }
    }
    if !full && rows.len() > BODY_ROWS {
        let more = rows.len() - BODY_ROWS;
        rows.truncate(BODY_ROWS);
        let text = format!("{INDENT}… {more} more lines (f shows all)");
        rows.push(Line::styled(clip(&text, width), st.dim()));
    }
    rows
}
