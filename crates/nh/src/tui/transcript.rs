//! A [`Transcript`]'s items as wrapped, styled lines, cached per item so a
//! redraw only re-renders the items that changed.

use nh_core::view::{Item, NoteKind};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::icons::lead;
use super::markdown;
use super::theme::Styles;
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
    /// The theme: a change re-renders every cached line.
    pub styles: Styles,
}

fn text_lines(text: &str, style: Style, width: usize, indent: usize) -> Vec<Line<'static>> {
    sanitize(text)
        .split('\n')
        .flat_map(|row| wrap_line(&Line::styled(row.to_string(), style), width, indent))
        .collect()
}

fn user_lines(text: &str, width: usize, st: &Styles) -> Vec<Line<'static>> {
    let mut lines = text_lines(text, st.text(), width.saturating_sub(2), 0);
    for (index, line) in lines.iter_mut().enumerate() {
        let lead = if index == 0 {
            lead(st.icons.pointer)
        } else {
            "  ".into()
        };
        let marker = Span::styled(lead, st.user_marker());
        line.spans.insert(0, marker);
    }
    lines
}

fn thinking_lines(thinking: &str, o: &Opts) -> Vec<Line<'static>> {
    let style = o.styles.thinking();
    if !o.show_thinking {
        let first = thinking
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("");
        let mark = lead(o.styles.icons.thinking);
        let text = clip(&format!("{mark}Thinking: {}", first.trim()), o.width);
        return vec![Line::styled(text, style)];
    }
    let head = format!("{}Thinking", lead(o.styles.icons.thinking));
    let mut lines = vec![Line::styled(head, style)];
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
    let st = &o.styles;
    // A mark per state, not only a colour: greyscale themes read the same.
    let (mark, mark_style) = match (output, is_error) {
        (None, _) if o.running => (SPINNER[o.frame % SPINNER.len()], st.accent()),
        (None, _) => (st.icons.pending, st.dim()),
        (Some(_), true) => (st.icons.error, st.error()),
        (Some(_), false) => (st.icons.ok, st.success()),
    };
    let head = clip(summary, o.width.saturating_sub(2));
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("{mark} "), mark_style),
        Span::styled(head, st.text()),
    ])];
    let Some(output) = output else {
        return lines;
    };
    let style = if *is_error { st.error() } else { st.dim() };
    let rows: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
    for (index, row) in rows.iter().take(TOOL_LINES).enumerate() {
        let lead = if index == 0 { "  ⎿ " } else { "    " };
        let text = clip(row, o.width.saturating_sub(4));
        lines.push(Line::styled(format!("{lead}{text}"), style));
    }
    if rows.len() > TOOL_LINES {
        let more = format!("    … +{} lines", rows.len() - TOOL_LINES);
        lines.push(Line::styled(more, st.dim()));
    }
    lines
}

/// A note with a mark for its kind, so greyscale themes still tell them apart.
fn note_lines(kind: NoteKind, text: &str, width: usize, st: &Styles) -> Vec<Line<'static>> {
    let (mark, style) = match kind {
        NoteKind::Retry => (st.icons.retry, st.warning()),
        NoteKind::AccountSwitch => (st.icons.switch, st.warning()),
        NoteKind::Error => (st.icons.error, st.error()),
        NoteKind::Info => (st.icons.info, st.dim()),
    };
    text_lines(&format!("{}{text}", lead(mark)), style, width, 2)
}

/// Seconds with one decimal.
fn secs(ms: u64) -> String {
    format!("{:.1}s", ms as f64 / 1000.0)
}

fn footer_line(item: &Item, width: usize, st: &Styles) -> Vec<Line<'static>> {
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
    text_lines(&text, st.dim(), width, 2)
}

/// The lines of one item.
pub fn item_lines(item: &Item, o: &Opts) -> Vec<Line<'static>> {
    let width = o.width.max(1);
    let st = &o.styles;
    match item {
        Item::User { text } => user_lines(text, width, st),
        Item::Assistant { text, thinking, .. } => {
            let mut lines = Vec::new();
            if !thinking.trim().is_empty() {
                lines.extend(thinking_lines(thinking, o));
            }
            lines.extend(markdown::render(text, width, st));
            lines
        }
        Item::Tool { .. } => tool_lines(item, o),
        Item::Note { kind, text } => note_lines(*kind, text, width, st),
        Item::StepFooter { .. } => footer_line(item, width, st),
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
