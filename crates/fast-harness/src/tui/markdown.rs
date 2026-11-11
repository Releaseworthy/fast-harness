//! Assistant markdown to styled lines with pulldown-cmark: headings, bold,
//! italic, inline code, fenced code blocks on their own background, lists,
//! quotes, tables and links (shown as their text).

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::theme::Styles;
use super::wrap::{fill, sanitize, text_width, wrap_line};

/// One unwrapped line: later rows of it are indented by `indent`; code
/// lines are filled with their background.
pub struct MdLine {
    pub line: Line<'static>,
    pub indent: usize,
    pub code: bool,
}

#[derive(Default)]
struct Builder {
    out: Vec<MdLine>,
    spans: Vec<Span<'static>>,
    styles: Vec<Style>,
    /// One entry per open list: the next number of an ordered list.
    lists: Vec<Option<u64>>,
    marker: Option<String>,
    indent: usize,
    quote: usize,
    code: Option<String>,
    gap: bool,
    st: Styles,
}

impl Builder {
    fn style(&self) -> Style {
        self.styles
            .iter()
            .fold(Style::default(), |acc, s| acc.patch(*s))
    }

    fn prefix(&mut self) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        if self.quote > 0 {
            let bars = "│ ".repeat(self.quote);
            spans.push(Span::styled(bars, self.st.quote()));
        }
        match self.marker.take() {
            Some(marker) => spans.push(Span::styled(marker, self.st.marker())),
            None if self.indent > 0 => spans.push(Span::raw(" ".repeat(self.indent))),
            None => {}
        }
        spans
    }

    fn flush(&mut self) {
        if self.spans.is_empty() && self.marker.is_none() {
            return;
        }
        let mut spans = self.prefix();
        let indent = spans.iter().map(|s| s.width()).sum();
        spans.append(&mut self.spans);
        self.out.push(MdLine {
            line: Line::from(spans),
            indent,
            code: false,
        });
    }

    fn block_start(&mut self) {
        self.flush();
        if self.gap && !self.out.is_empty() {
            self.out.push(MdLine {
                line: Line::default(),
                indent: 0,
                code: false,
            });
        }
        self.gap = false;
    }

    fn text(&mut self, text: &str) {
        if let Some(code) = &mut self.code {
            code.push_str(text);
            return;
        }
        let style = self.style();
        self.spans.push(Span::styled(sanitize(text), style));
    }

    fn code_end(&mut self) {
        let Some(code) = self.code.take() else {
            return;
        };
        let body = code.strip_suffix('\n').unwrap_or(&code);
        for row in body.split('\n') {
            let mut spans = self.prefix();
            let indent = spans.iter().map(|s| s.width()).sum();
            spans.push(Span::raw(format!(" {}", sanitize(row))));
            self.out.push(MdLine {
                line: Line::from(spans),
                indent,
                code: true,
            });
        }
        self.gap = true;
    }

    fn item_start(&mut self) {
        self.flush();
        let depth = self.lists.len().saturating_sub(1);
        let bullet = match self.lists.last_mut() {
            Some(Some(n)) => {
                *n += 1;
                format!("{}. ", *n - 1)
            }
            _ => "• ".to_string(),
        };
        let marker = format!("{}{bullet}", "  ".repeat(depth));
        self.indent = text_width(&marker);
        self.marker = Some(marker);
        self.gap = false;
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => self.block_start(),
            Tag::Heading { level, .. } => {
                self.block_start();
                self.styles.push(self.st.heading(level));
            }
            Tag::BlockQuote(_) => {
                self.block_start();
                self.quote += 1;
            }
            Tag::CodeBlock(_) => {
                self.block_start();
                self.code = Some(String::new());
            }
            Tag::List(first) => {
                if self.lists.is_empty() {
                    self.block_start();
                } else {
                    self.flush();
                }
                self.lists.push(first);
            }
            Tag::Item => self.item_start(),
            Tag::Table(_) => self.block_start(),
            Tag::TableHead => self
                .styles
                .push(Style::default().add_modifier(Modifier::BOLD)),
            Tag::TableCell if !self.spans.is_empty() => {
                self.spans.push(Span::styled(" │ ", self.st.quote()));
            }
            Tag::Emphasis => self
                .styles
                .push(Style::default().add_modifier(Modifier::ITALIC)),
            Tag::Strong => self
                .styles
                .push(Style::default().add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self
                .styles
                .push(Style::default().add_modifier(Modifier::CROSSED_OUT)),
            Tag::Link { .. } => self.styles.push(self.st.link()),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Table => {
                self.flush();
                self.gap = true;
            }
            TagEnd::Heading(_) => {
                self.flush();
                self.styles.pop();
                self.gap = true;
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.quote = self.quote.saturating_sub(1);
                self.gap = true;
            }
            TagEnd::CodeBlock => self.code_end(),
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
                self.indent = self.lists.len() * 2;
                self.gap = self.lists.is_empty();
            }
            TagEnd::Item | TagEnd::TableRow => self.flush(),
            TagEnd::TableHead => {
                self.flush();
                self.styles.pop();
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                self.styles.pop();
            }
            _ => {}
        }
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => self.text(&text),
            Event::Code(code) => {
                let style = self.style().patch(self.st.code());
                self.spans.push(Span::styled(sanitize(&code), style));
            }
            Event::SoftBreak => self.text(" "),
            Event::HardBreak => self.flush(),
            Event::Rule => {
                self.block_start();
                let rule = Span::styled("─".repeat(20), self.st.rule());
                self.spans.push(rule);
                self.flush();
                self.gap = true;
            }
            Event::TaskListMarker(done) => self.text(if done { "[x] " } else { "[ ] " }),
            _ => {}
        }
    }
}

/// Markdown to unwrapped styled lines.
pub fn parse(text: &str, st: &Styles) -> Vec<MdLine> {
    let options =
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS;
    let mut builder = Builder {
        st: *st,
        ..Builder::default()
    };
    for event in Parser::new_ext(text, options) {
        builder.event(event);
    }
    builder.code_end();
    builder.flush();
    builder.out
}

/// Markdown wrapped to `width` columns.
pub fn render(text: &str, width: usize, st: &Styles) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for md in parse(text, st) {
        let rows = wrap_line(&md.line, width, md.indent);
        if md.code {
            out.extend(rows.into_iter().map(|r| fill(r, width, st.code())));
        } else {
            out.extend(rows);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::theme::color;
    use super::*;

    /// The default theme's render; a local item shadows the glob import.
    fn render(text: &str, width: usize) -> Vec<Line<'static>> {
        super::render(text, width, &Styles::default())
    }

    fn texts(lines: &[Line<'_>]) -> Vec<String> {
        lines.iter().map(|l| l.to_string()).collect()
    }

    fn style_of(lines: &[Line<'_>], needle: &str) -> Style {
        lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains(needle))
            .map(|s| s.style)
            .unwrap_or_else(|| panic!("no span with {needle}"))
    }

    #[test]
    fn inline_styles() {
        let lines = render(
            "# Title\n\nplain **bold** *ital* `code` [link](http://x)",
            80,
        );
        assert_eq!(texts(&lines), ["Title", "", "plain bold ital code link"]);
        assert!(style_of(&lines, "Title")
            .add_modifier
            .contains(Modifier::BOLD));
        assert!(style_of(&lines, "bold")
            .add_modifier
            .contains(Modifier::BOLD));
        assert!(style_of(&lines, "ital")
            .add_modifier
            .contains(Modifier::ITALIC));
        let p = Styles::default().palette();
        let title = style_of(&lines, "Title");
        assert_eq!(title.fg, Some(color(p.accent)));
        assert!(title.add_modifier.contains(Modifier::UNDERLINED));
        let code = style_of(&lines, "code");
        assert_eq!(
            (code.fg, code.bg),
            (Some(color(p.text)), Some(color(p.code)))
        );
        let link = style_of(&lines, "link");
        assert_eq!(link.fg, Some(color(p.accent)));
        assert!(link.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn code_blocks_fill_the_width_on_their_background() {
        let lines = render("text\n\n```rust\nfn main() {}\n```\nafter", 20);
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[2].to_string().trim_end(), " fn main() {}");
        assert_eq!(lines[2].width(), 20);
        let code_bg = Some(color(Styles::default().palette().code));
        assert!(lines[2].spans.iter().all(|s| s.style.bg == code_bg));
        assert_eq!(lines[4].to_string(), "after");
    }

    #[test]
    fn lists_nest_and_wrap_under_their_text() {
        let lines = render("- one two three\n- b\n  1. x\n  2. y", 10);
        assert_eq!(
            texts(&lines),
            ["• one two", "  three", "• b", "  1. x", "  2. y"]
        );
    }

    #[test]
    fn quotes_rules_tables_and_wide_text() {
        let lines = render("> quoted\n\n---\n\n|a|b|\n|-|-|\n|1|2|", 40);
        let all = texts(&lines);
        assert!(all.contains(&"│ quoted".to_string()), "{all:?}");
        assert!(all.iter().any(|l| l.starts_with('─')));
        assert!(all.contains(&"1 │ 2".to_string()), "{all:?}");
        for line in render("Xin chào thế giới 世界你好 😀 emoji", 9) {
            assert!(line.width() <= 9);
        }
        assert!(render("", 10).is_empty());
        assert!(!render("```\nunclosed", 10).is_empty());
    }
}
