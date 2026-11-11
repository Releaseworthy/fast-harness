//! A [`Transcript`]'s items as wrapped, styled lines, cached per item so a
//! redraw only re-renders the items that changed.

use nh_core::view::{Item, NoteKind};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::markdown;
use super::wrap::{clip, sanitize, wrap_line};

/// Tool output lines shown under a tool call.
pub const TOOL_LINES: usize = 3;
pub const SPINNER: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Opts {
    pub width: usize,
    pub show_thinking: bool,
    /// A turn runs, so tools without output are spinning.
    pub running: bool,
    pub frame: usize,
}

fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn text_lines(text: &str, style: Style, width: usize, indent: usize) -> Vec<Line<'static>> {
    sanitize(text)
        .split('\n')
        .flat_map(|row| wrap_line(&Line::styled(row.to_string(), style), width, indent))
        .collect()
}

fn user_lines(text: &str, width: usize) -> Vec<Line<'static>> {
    let mut lines = text_lines(text, Style::default(), width.saturating_sub(2), 0);
    for (index, line) in lines.iter_mut().enumerate() {
        let lead = if index == 0 { "› " } else { "  " };
        let marker = Span::styled(
            lead,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
        line.spans.insert(0, marker);
    }
    lines
}

fn thinking_lines(thinking: &str, o: &Opts) -> Vec<Line<'static>> {
    let style = dim().add_modifier(Modifier::ITALIC);
    if !o.show_thinking {
        let first = thinking
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("");
        let text = clip(&format!("✻ Thinking: {}", first.trim()), o.width);
        return vec![Line::styled(text, style)];
    }
    let mut lines = vec![Line::styled("✻ Thinking", style)];
    lines.extend(text_lines(thinking.trim(), style, o.width, 0));
    lines
}

fn tool_lines(item: &Item, o: &Opts) -> Vec<Line<'static>> {
    let Item::Tool {
        summary,
        output,
        is_error,
        ..
    } = item
    else {
        return Vec::new();
    };
    let (mark, color) = match (output, is_error) {
        (None, _) if o.running => (SPINNER[o.frame % SPINNER.len()], Color::Yellow),
        (None, _) => ("●", Color::DarkGray),
        (Some(_), true) => ("●", Color::Red),
        (Some(_), false) => ("●", Color::Green),
    };
    let head = clip(summary, o.width.saturating_sub(2));
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{mark} "), Style::default().fg(color)),
        Span::raw(head),
    ])];
    let Some(output) = output else {
        return lines;
    };
    let style = if *is_error {
        Style::default().fg(Color::Red)
    } else {
        dim()
    };
    let rows: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
    for (index, row) in rows.iter().take(TOOL_LINES).enumerate() {
        let lead = if index == 0 { "  ⎿ " } else { "    " };
        let text = clip(row, o.width.saturating_sub(4));
        lines.push(Line::styled(format!("{lead}{text}"), style));
    }
    if rows.len() > TOOL_LINES {
        let more = format!("    … +{} lines", rows.len() - TOOL_LINES);
        lines.push(Line::styled(more, dim()));
    }
    lines
}

fn note_lines(kind: NoteKind, text: &str, width: usize) -> Vec<Line<'static>> {
    let color = match kind {
        NoteKind::Error => Color::Red,
        _ => Color::Yellow,
    };
    text_lines(text, Style::default().fg(color), width, 2)
}

/// Seconds with one decimal.
fn secs(ms: u64) -> String {
    format!("{:.1}s", ms as f64 / 1000.0)
}

fn footer_line(item: &Item, width: usize) -> Vec<Line<'static>> {
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
        return Vec::new();
    };
    let text = format!(
        "turn {turn} step {step} · {model} · {account} · ttfb {} · {} · in {} out {} cache r {} w {}",
        secs(timing.ttfb_ms),
        secs(timing.total_ms),
        usage.input,
        usage.output,
        usage.cache_read,
        usage.cache_write
    );
    text_lines(&text, dim(), width, 2)
}

/// The lines of one item.
pub fn item_lines(item: &Item, o: &Opts) -> Vec<Line<'static>> {
    let width = o.width.max(1);
    match item {
        Item::User { text } => user_lines(text, width),
        Item::Assistant { text, thinking, .. } => {
            let mut lines = Vec::new();
            if !thinking.trim().is_empty() {
                lines.extend(thinking_lines(thinking, o));
            }
            lines.extend(markdown::render(text, width));
            lines
        }
        Item::Tool { .. } => tool_lines(item, o),
        Item::Note { kind, text } => note_lines(*kind, text, width),
        Item::StepFooter { .. } => footer_line(item, width),
    }
}

/// A blank line goes before user messages and answers.
fn spaced(item: &Item) -> bool {
    matches!(item, Item::User { .. } | Item::Assistant { .. })
}

#[derive(Default)]
pub struct Cache {
    opts: Option<Opts>,
    entries: Vec<(Item, Vec<Line<'static>>)>,
}

impl Cache {
    /// Brings the cache up to date with `items` and returns all lines.
    pub fn lines(&mut self, items: &[Item], o: Opts) -> Vec<&Line<'static>> {
        let static_opts = Opts { frame: 0, ..o };
        if self.opts != Some(static_opts) {
            self.entries.clear();
            self.opts = Some(static_opts);
        }
        self.entries.truncate(items.len());
        for (index, item) in items.iter().enumerate() {
            let spinning = matches!(item, Item::Tool { output: None, .. }) && o.running;
            let fresh = self.entries.get(index).is_some_and(|(old, _)| old == item);
            if fresh && !spinning {
                continue;
            }
            let mut lines = Vec::new();
            if index > 0 && spaced(item) {
                lines.push(Line::default());
            }
            lines.extend(item_lines(item, &o));
            let entry = (item.clone(), lines);
            match self.entries.get_mut(index) {
                Some(slot) => *slot = entry,
                None => self.entries.push(entry),
            }
        }
        self.entries.iter().flat_map(|(_, lines)| lines).collect()
    }
}
