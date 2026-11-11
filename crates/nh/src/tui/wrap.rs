//! Display widths and word wrapping of styled lines. Widths come from
//! ratatui's own measure (unicode-width), so Vietnamese, CJK and emoji
//! take the same columns here as in the buffer.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

/// Columns a text takes on screen.
pub fn text_width(text: &str) -> usize {
    Span::raw(text).width()
}

/// Tabs become four spaces; other control characters (escape sequences,
/// carriage returns) are dropped so they cannot reach the terminal.
pub fn sanitize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\t' => out.push_str("    "),
            '\n' => out.push('\n'),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Splits a text into display clusters: a base character with the
/// zero-width characters (combining marks, variation selectors, joiners)
/// after it, and the character after a zero-width joiner.
pub fn clusters(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut prev = '\0';
    for (index, ch) in text.char_indices() {
        let mut buf = [0u8; 4];
        let joins = text_width(ch.encode_utf8(&mut buf)) == 0 || prev == '\u{200d}';
        if index > start && !joins {
            out.push(&text[start..index]);
            start = index;
        }
        prev = ch;
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

#[derive(Clone)]
struct Piece {
    text: String,
    style: Style,
    width: usize,
}

fn pieces(line: &Line<'_>) -> Vec<Piece> {
    let mut out = Vec::new();
    for span in &line.spans {
        let style = line.style.patch(span.style);
        for cluster in clusters(&span.content) {
            out.push(Piece {
                text: cluster.to_string(),
                style,
                width: text_width(cluster),
            });
        }
    }
    out
}

fn row_width(row: &[Piece]) -> usize {
    row.iter().map(|p| p.width).sum()
}

fn trim_end(row: &mut Vec<Piece>) {
    while row.last().is_some_and(|p| p.text == " ") {
        row.pop();
    }
}

/// Breaks pieces into rows of at most `width` columns, at the last space
/// when there is one; `indent` columns are kept free on later rows.
fn break_rows(all: Vec<Piece>, width: usize, indent: usize) -> Vec<Vec<Piece>> {
    let mut rows: Vec<Vec<Piece>> = Vec::new();
    let mut cur: Vec<Piece> = Vec::new();
    let mut space: Option<usize> = None;
    for piece in all {
        let limit = if rows.is_empty() {
            width
        } else {
            width.saturating_sub(indent).max(1)
        };
        while !cur.is_empty() && row_width(&cur) + piece.width > limit {
            if piece.text == " " {
                trim_end(&mut cur);
                rows.push(std::mem::take(&mut cur));
                space = None;
                break;
            }
            let rest = match space.take() {
                Some(at) if at > 0 => cur.split_off(at + 1),
                _ => Vec::new(),
            };
            trim_end(&mut cur);
            rows.push(std::mem::replace(&mut cur, rest));
        }
        if piece.text == " " && cur.is_empty() && !rows.is_empty() {
            continue;
        }
        if piece.text == " " {
            space = Some(cur.len());
        }
        cur.push(piece);
    }
    if !cur.is_empty() || rows.is_empty() {
        rows.push(cur);
    }
    rows
}

fn to_line(row: Vec<Piece>, indent: usize) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    if indent > 0 {
        spans.push(Span::raw(" ".repeat(indent)));
    }
    let mut text = String::new();
    let mut style: Option<Style> = None;
    for piece in row {
        if style.is_some_and(|s| s != piece.style) {
            spans.push(Span::styled(
                std::mem::take(&mut text),
                style.unwrap_or_default(),
            ));
        }
        style = Some(piece.style);
        text.push_str(&piece.text);
    }
    if let Some(style) = style {
        spans.push(Span::styled(text, style));
    }
    Line::from(spans)
}

/// Wraps one line to `width` columns; rows after the first start with
/// `indent` spaces.
pub fn wrap_line(line: &Line<'_>, width: usize, indent: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let indent = if indent >= width { 0 } else { indent };
    break_rows(pieces(line), width, indent)
        .into_iter()
        .enumerate()
        .map(|(index, row)| to_line(row, if index == 0 { 0 } else { indent }))
        .collect()
}

/// Pads a line with `style` spaces up to `width` columns (code blocks).
pub fn fill(mut line: Line<'static>, width: usize, style: Style) -> Line<'static> {
    for span in &mut line.spans {
        span.style = style.patch(span.style);
    }
    let used = line.width();
    if used < width {
        line.spans
            .push(Span::styled(" ".repeat(width - used), style));
    }
    line
}

/// The first row of `text` cut to `width` columns, with `...` when cut.
pub fn clip(text: &str, width: usize) -> String {
    let flat = sanitize(text).replace('\n', " ");
    if text_width(&flat) <= width {
        return flat;
    }
    let mut out = String::new();
    let mut used = 0;
    for cluster in clusters(&flat) {
        let w = text_width(cluster);
        if used + w + 3 > width {
            break;
        }
        used += w;
        out.push_str(cluster);
    }
    out.push_str("...");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(lines: &[Line<'_>]) -> Vec<String> {
        lines.iter().map(|l| l.to_string()).collect()
    }

    #[test]
    fn wraps_at_spaces_and_hard_breaks_long_words() {
        let line = Line::raw("hello world again");
        assert_eq!(texts(&wrap_line(&line, 11, 0)), ["hello world", "again"]);
        let long = Line::raw("abcdefghij");
        assert_eq!(texts(&wrap_line(&long, 4, 0)), ["abcd", "efgh", "ij"]);
        assert_eq!(texts(&wrap_line(&Line::raw(""), 5, 0)), [""]);
        assert_eq!(texts(&wrap_line(&line, 0, 0)).len(), 17 - 2);
    }

    #[test]
    fn wide_characters_take_two_columns() {
        let cjk = Line::raw("漢字漢字漢字");
        let rows = wrap_line(&cjk, 5, 0);
        assert_eq!(texts(&rows), ["漢字", "漢字", "漢字"]);
        assert!(rows.iter().all(|r| r.width() <= 5));
        let emoji = Line::raw("ok 😀😀😀 done");
        for row in wrap_line(&emoji, 6, 0) {
            assert!(row.width() <= 6, "{row}");
        }
    }

    #[test]
    fn vietnamese_combining_marks_stay_with_their_letter() {
        // "Việt" written with a combining dot below and circumflex.
        let text = "Vie\u{323}\u{302}t Nam";
        assert_eq!(clusters(text).len(), 8);
        assert_eq!(text_width(text), 8);
        let rows = wrap_line(&Line::raw(text), 4, 0);
        assert_eq!(texts(&rows), ["Vie\u{323}\u{302}t", "Nam"]);
        assert_eq!(text_width("Tiếng Việt"), 10);
    }

    #[test]
    fn indent_and_fill_and_clip() {
        let rows = wrap_line(&Line::raw("aa bb cc dd"), 5, 2);
        assert_eq!(texts(&rows), ["aa bb", "  cc", "  dd"]);
        let filled = fill(Line::raw("ab"), 5, Style::default());
        assert_eq!(filled.width(), 5);
        assert_eq!(clip("abcdefgh", 6), "abc...");
        assert_eq!(clip("a\tb\u{1b}[0m", 20), "a    b[0m");
    }
}
