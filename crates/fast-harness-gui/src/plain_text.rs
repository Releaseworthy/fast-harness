//! An assistant reply's markdown as the plain text select mode shows
//! (`reply_select`): StyledText cannot be selected in Slint 1.18.1, so a
//! reply swaps to a SelectableText of this. Emphasis marks, backticks and
//! heading marks go, list markers become "• ", links keep their text, and
//! code blocks keep their lines as they are.

/// `markdown` without its syntax.
pub fn plain(markdown: &str) -> String {
    let mut out = Vec::new();
    let mut code = false;
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            code = !code;
            continue;
        }
        out.push(if code { line.to_string() } else { inline(&block(line)) });
    }
    out.join("\n")
}

/// A line without its heading, list or quote mark.
fn block(line: &str) -> String {
    let trimmed = line.trim_start();
    let indent = &line[..line.len() - trimmed.len()];
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
        return trimmed[hashes + 1..].to_string();
    }
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(marker) {
            return format!("{indent}• {rest}");
        }
    }
    match trimmed.strip_prefix("> ") {
        Some(rest) => rest.to_string(),
        None => line.to_string(),
    }
}

/// A line without bold marks and backticks (code spans kept verbatim),
/// its links as their text.
fn inline(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(c) = rest.chars().next() {
        if let Some(after) = rest.strip_prefix("**").or_else(|| rest.strip_prefix("__")) {
            rest = after;
        } else if let Some((span, after)) = enclosed(rest, '`', "`") {
            out.push_str(span);
            rest = after;
        } else if let Some((text, after)) = link(rest) {
            out.push_str(text);
            rest = after;
        } else {
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// `s` opening with `open` and closed by `close`: the inside and the rest.
fn enclosed<'a>(s: &'a str, open: char, close: &str) -> Option<(&'a str, &'a str)> {
    let inner = s.strip_prefix(open)?;
    let end = inner.find(close)?;
    Some((&inner[..end], &inner[end + close.len()..]))
}

/// `[text](url)` at the start of `s`: the text and the rest.
fn link(s: &str) -> Option<(&str, &str)> {
    let (text, after) = enclosed(s, '[', "]")?;
    let (_, rest) = enclosed(after, '(', ")")?;
    Some((text, rest))
}

#[cfg(test)]
mod tests {
    use super::plain;

    #[test]
    fn markdown_syntax_goes_and_the_text_stays() {
        let md = "## Plan\n\nRun **cargo test** on `__init__`, see [the docs](https://x.y/a_b).\n- one\n  * two\n```rust\nlet a = **b**;\n```\n> quoted";
        let want = "Plan\n\nRun cargo test on __init__, see the docs.\n• one\n  • two\nlet a = **b**;\nquoted";
        assert_eq!(plain(md), want);
    }

    #[test]
    fn a_lone_bracket_or_backtick_stays() {
        assert_eq!(plain("a [b c ` d"), "a [b c ` d");
    }
}
