//! Assistant markdown split into blocks (the owner, 2026-11-15: "support
//! markdown formatting"). Slint 1.18.1's `StyledText` draws inline markup
//! only, so pulldown-cmark splits a reply into headings, paragraphs, list
//! items, code blocks, rules and tables; each block keeps its inline
//! markdown (a slice of the source) for `StyledText`, and `ui/markdown.slint`
//! draws the list (a heading larger, an item after its marker, code in a
//! monospace box, a quote with a bar, a table as a grid).

use std::ops::Range;

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

/// What a block draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Para,
    Heading,
    Item,
    Code,
    Rule,
    Table,
}

/// One block of a reply.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Block {
    pub kind: Kind,
    /// Inline markdown (paragraph, heading, item) or the code's text.
    pub text: String,
    /// A heading's level (1 to 6).
    pub level: usize,
    /// List depth: the item's indent.
    pub indent: usize,
    /// "•", "3." or "[x]" on an item's first block; empty after.
    pub marker: String,
    /// Block quote depth (the bar on the left).
    pub quote: usize,
    /// A table's cells (inline markdown), the header row first.
    pub rows: Vec<Vec<String>>,
}

/// The inline part being collected: its kind and source span.
struct Leaf {
    kind: Kind,
    level: usize,
    span: Option<Range<usize>>,
}

struct Split<'a> {
    src: &'a str,
    out: Vec<Block>,
    quote: usize,
    /// The next number of each open list (`None`: bullets).
    lists: Vec<Option<u64>>,
    marker: Option<String>,
    leaf: Option<Leaf>,
    code: Option<String>,
    table: Option<Vec<Vec<String>>>,
}

/// The blocks of `text`.
pub fn blocks(text: &str) -> Vec<Block> {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut split = Split {
        src: text,
        out: Vec::new(),
        quote: 0,
        lists: Vec::new(),
        marker: None,
        leaf: None,
        code: None,
        table: None,
    };
    for (event, range) in Parser::new_ext(text, options).into_offset_iter() {
        split.event(event, range);
    }
    split.flush();
    split.out
}

impl Split<'_> {
    fn event(&mut self, event: Event, range: Range<usize>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) if self.code.is_some() => {
                if let Some(code) = self.code.as_mut() {
                    code.push_str(&text);
                }
            }
            Event::Rule => {
                self.flush();
                self.push(Kind::Rule, String::new(), 0);
            }
            Event::TaskListMarker(done) => {
                self.marker = Some(if done { "[x]" } else { "[ ]" }.into());
            }
            _ => self.inline(range),
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => self.open(Kind::Para, 0),
            Tag::Heading { level, .. } => self.open(Kind::Heading, level as usize),
            Tag::BlockQuote(_) => {
                self.flush();
                self.quote += 1;
            }
            Tag::CodeBlock(_) => {
                self.flush();
                self.code = Some(String::new());
            }
            Tag::List(first) => {
                self.flush();
                self.lists.push(first);
            }
            Tag::Item => {
                self.flush();
                self.marker = Some(self.next_marker());
            }
            Tag::Table(_) => {
                self.flush();
                self.table = Some(Vec::new());
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    table.push(Vec::new());
                }
            }
            Tag::TableCell => self.leaf = None,
            Tag::HtmlBlock => self.open(Kind::Para, 0),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::HtmlBlock | TagEnd::Item => self.flush(),
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.quote = self.quote.saturating_sub(1);
            }
            TagEnd::CodeBlock => {
                let code = self.code.take().unwrap_or_default();
                self.push(Kind::Code, code.trim_end_matches('\n').to_string(), 0);
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
            }
            TagEnd::TableCell => {
                let text = self.leaf.take().map(|l| self.slice(l.span)).unwrap_or_default();
                if let Some(row) = self.table.as_mut().and_then(|t| t.last_mut()) {
                    row.push(text);
                }
            }
            TagEnd::Table => {
                let rows = self.table.take().unwrap_or_default();
                let mut block = self.block(Kind::Table, String::new(), 0);
                block.rows = rows;
                self.out.push(block);
            }
            _ => {}
        }
    }

    /// An inline event: its span joins the open leaf (a tight list item's
    /// text has no paragraph, so the item opens one).
    fn inline(&mut self, range: Range<usize>) {
        if self.leaf.is_none() {
            self.open(Kind::Para, 0);
        }
        if let Some(leaf) = self.leaf.as_mut() {
            leaf.span = Some(match leaf.span.take() {
                Some(span) => span.start.min(range.start)..span.end.max(range.end),
                None => range,
            });
        }
    }

    fn open(&mut self, kind: Kind, level: usize) {
        if self.table.is_none() {
            self.flush();
        }
        self.leaf = Some(Leaf { kind, level, span: None });
    }

    /// Ends the open leaf as a block (inside a table the cell end takes it).
    fn flush(&mut self) {
        if self.table.is_some() {
            return;
        }
        let Some(leaf) = self.leaf.take() else {
            return;
        };
        let text = self.slice(leaf.span);
        if text.is_empty() {
            return;
        }
        let kind = match leaf.kind {
            Kind::Para if !self.lists.is_empty() => Kind::Item,
            kind => kind,
        };
        self.push(kind, text, leaf.level);
    }

    fn push(&mut self, kind: Kind, text: String, level: usize) {
        let block = self.block(kind, text, level);
        self.out.push(block);
    }

    fn block(&mut self, kind: Kind, text: String, level: usize) -> Block {
        Block {
            kind,
            text,
            level,
            indent: self.lists.len(),
            marker: self.marker.take().unwrap_or_default(),
            quote: self.quote,
            rows: Vec::new(),
        }
    }

    fn next_marker(&mut self) -> String {
        match self.lists.last_mut() {
            Some(Some(n)) => {
                *n += 1;
                format!("{}.", *n - 1)
            }
            _ => "•".into(),
        }
    }

    /// The source of a span, each line without its quote marks and indent.
    fn slice(&self, span: Option<Range<usize>>) -> String {
        let Some(text) = span.and_then(|s| self.src.get(s)) else {
            return String::new();
        };
        let lines: Vec<&str> = text.lines().map(|l| strip_quote(l, self.quote)).collect();
        lines.join("\n").trim().to_string()
    }
}

/// `line` without up to `depth` leading `>` marks and its indent.
fn strip_quote(line: &str, depth: usize) -> &str {
    let mut rest = line.trim_start();
    for _ in 0..depth {
        match rest.strip_prefix('>') {
            Some(after) => rest = after.trim_start(),
            None => break,
        }
    }
    rest
}

/// Each column's share of a table's width: from its longest cell, so a
/// short column stays narrow and a long one wraps.
pub fn shares(rows: &[Vec<String>]) -> Vec<f32> {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<f32> = (0..columns)
        .map(|c| {
            let longest = rows.iter().filter_map(|r| r.get(c)).map(|t| t.chars().count()).max();
            longest.unwrap_or(0).clamp(4, 48) as f32
        })
        .collect();
    let total: f32 = widths.iter().sum();
    widths.iter().map(|w| w / total.max(1.0)).collect()
}

#[cfg(test)]
#[path = "md_blocks_tests.rs"]
mod tests;
