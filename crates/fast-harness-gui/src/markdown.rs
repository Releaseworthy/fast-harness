//! Assistant markdown for Slint 1.18.1's `StyledText::from_markdown`, which
//! refuses the whole text on any construct it does not draw
//! (`i-slint-common` 1.18.1 `styled_text.rs`): headings, code blocks, block
//! quotes, images, rules, HTML and setext underlines are errors, so a normal
//! reply with "## Plan" showed its raw syntax. The text is rewritten line by
//! line into what it draws: a heading is a bold line, a code line an inline
//! code span, a quote its plain line, a rule nothing, and `<` an escaped
//! character outside code spans. What still fails falls back to plain text.

use slint::StyledText;

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
