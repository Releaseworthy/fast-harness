//! Assistant markdown for Slint 1.18.1's `StyledText::from_markdown`, which
//! refuses the whole text on any construct it does not draw
//! (`i-slint-common` 1.18.1 `styled_text.rs`): headings, code blocks, block
//! quotes, images, rules, HTML and setext underlines are errors, so a normal
//! reply with "## Plan" showed its raw syntax. The text is rewritten line by
//! line into what it draws: a heading is a bold line, a code line an inline
//! code span, a quote its plain line, a rule nothing, and `<` an escaped
//! character outside code spans. What still fails falls back to plain text.

use slint::{ModelRc, StyledText, VecModel};

use crate::md_blocks::{blocks, shares, Block, Kind};
use crate::plain_text::{encode, plain_links};
use crate::{MdBlock, MdCell, MdRow};

/// A reply as the blocks `ui/markdown.slint` draws (src/md_blocks.rs).
pub fn reply_blocks(text: &str) -> ModelRc<MdBlock> {
    let list: Vec<MdBlock> = blocks(text).into_iter().map(md_block).collect();
    ModelRc::new(VecModel::from(list))
}

fn md_block(block: Block) -> MdBlock {
    let kind = match block.kind {
        Kind::Para => "p",
        Kind::Heading => "h",
        Kind::Item => "li",
        Kind::Code => "code",
        Kind::Rule => "rule",
        Kind::Table => "table",
    };
    let (plain, links) = selectable(&block);
    let (text, code) = match block.kind {
        Kind::Heading => (markdown(&bold(&block.text)), String::new()),
        Kind::Code => (StyledText::default(), block.text),
        Kind::Rule | Kind::Table => (StyledText::default(), String::new()),
        _ => (markdown(&block.text), String::new()),
    };
    let int = |n: usize| i32::try_from(n).unwrap_or(i32::MAX);
    MdBlock {
        kind: kind.into(),
        text,
        plain: plain.into(),
        links: links.into(),
        code: code.into(),
        level: int(block.level),
        indent: int(block.indent),
        marker: block.marker.into(),
        quote: int(block.quote),
        rows: table_rows(&block.rows),
    }
}

/// The plain text a drag selects in `block` (`MdText`) and its links
/// (`plain_text::encode`, for a click on one): paragraphs, headings and
/// list items; empty for the rest. Soft line breaks draw as spaces.
fn selectable(block: &Block) -> (String, String) {
    if !matches!(block.kind, Kind::Para | Kind::Heading | Kind::Item) {
        return Default::default();
    }
    inline_plain(&block.text)
}

/// Inline markdown's plain text on one line and its encoded links.
fn inline_plain(text: &str) -> (String, String) {
    let (plain, links) = plain_links(text);
    (plain.replace('\n', " "), encode(&links))
}

/// A table's rows, every row padded to the widest one's columns.
fn table_rows(rows: &[Vec<String>]) -> ModelRc<MdRow> {
    let shares = shares(rows);
    let list: Vec<MdRow> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let cells: Vec<MdCell> = shares
                .iter()
                .enumerate()
                .map(|(column, share)| {
                    let raw = row.get(column).map(String::as_str).unwrap_or("");
                    let text = if index == 0 { bold(raw) } else { raw.to_string() };
                    let (plain, links) = inline_plain(raw);
                    let (plain, links) = (plain.into(), links.into());
                    MdCell { text: markdown(&text), plain, links, share: *share }
                })
                .collect();
            MdRow {
                cells: ModelRc::new(VecModel::from(cells)),
                head: index == 0,
                odd: index % 2 == 0 && index > 0,
            }
        })
        .collect();
    ModelRc::new(VecModel::from(list))
}

/// `text` bold (headings, header cells); empty stays empty.
fn bold(text: &str) -> String {
    let text = text.replace("**", "");
    if text.trim().is_empty() {
        String::new()
    } else {
        format!("**{}**", text.trim())
    }
}

/// Assistant text as styled text; plain text when the markdown is refused.
pub fn markdown(text: &str) -> StyledText {
    StyledText::from_markdown(&prepare(text))
        .or_else(|_| StyledText::from_markdown(text))
        .unwrap_or_else(|_| StyledText::from_plain_text(text))
}

/// `text` with the constructs Slint refuses rewritten into ones it draws.
pub fn prepare(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let mut fence: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(mark) = &fence {
            if trimmed.starts_with(mark.as_str()) {
                fence = None;
            } else if !line.trim().is_empty() {
                out.push_str(&code_span(line));
                out.push('\n');
            }
            continue;
        }
        if let Some(mark) = fence_mark(trimmed) {
            fence = Some(mark);
            continue;
        }
        if let Some(kept) = block_line(trimmed) {
            out.push_str(&escape_html(&kept));
            out.push('\n');
        } else if !line.trim().is_empty() {
            out.push_str(&escape_html(line));
            out.push('\n');
        } else {
            out.push('\n');
        }
    }
    out
}

/// The opening fence of a code block ("```" or "~~~", three or more).
fn fence_mark(trimmed: &str) -> Option<String> {
    for c in ['`', '~'] {
        let n = trimmed.chars().take_while(|&x| x == c).count();
        if n >= 3 {
            return Some(c.to_string().repeat(n));
        }
    }
    None
}

/// A heading, quote, rule or image line rewritten; `None` keeps the line.
fn block_line(trimmed: &str) -> Option<String> {
    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&hashes) {
        let rest = &trimmed[hashes..];
        if rest.is_empty() || rest.starts_with(' ') {
            let title = rest.trim().trim_end_matches('#').trim().replace("**", "");
            return Some(if title.is_empty() {
                String::new()
            } else {
                format!("**{title}**")
            });
        }
    }
    if let Some(quoted) = trimmed.strip_prefix('>') {
        return Some(quoted.trim_start().to_string());
    }
    if is_rule(trimmed) {
        return Some(String::new());
    }
    trimmed.contains("![").then(|| trimmed.replace("![", "["))
}

/// A thematic break or setext underline: three or more of one of `-*_=`
/// with spaces between.
fn is_rule(trimmed: &str) -> bool {
    let marks: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    let Some(first) = marks.chars().next() else {
        return false;
    };
    "-*_=".contains(first) && marks.len() >= 3 && marks.chars().all(|c| c == first)
}

/// One code line as an inline code span with a fence longer than any
/// backtick run inside it (blank-padded so edge backticks stay code).
fn code_span(line: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in line.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat(longest + 1);
    format!("{fence} {} {fence}", line.trim_end())
}

/// `<` escaped outside inline code spans: Slint draws only `<u>` and
/// `<font>` and refuses any other HTML (`Vec<T>`, `<br>`).
fn escape_html(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 4);
    let mut open: Option<usize> = None;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '`' {
            let n = chars[i..].iter().take_while(|&&x| x == '`').count();
            open = match open {
                Some(m) if m == n => None,
                None => Some(n),
                keep => keep,
            };
            out.push_str(&"`".repeat(n));
            i += n;
            continue;
        }
        if c == '<' && open.is_none() {
            out.push('\\');
        }
        out.push(c);
        i += 1;
    }
    out
}

#[cfg(test)]
#[path = "markdown_tests.rs"]
mod tests;
