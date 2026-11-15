//! An assistant reply's markdown as the plain text select mode shows
//! (`reply_select`): StyledText cannot be selected in Slint 1.18.1, so a
//! reply swaps to a SelectableText of this. Emphasis marks, backticks and
//! heading marks go, list markers become "• ", links keep their text, and
//! code blocks keep their lines as they are. plain_links also says where
//! each link's text is, so a drag selects a paragraph with links and a
//! click on one still opens it (ui/selectable.slint `Ghost`).

/// A link in plain text: its text's byte range and its url.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub start: usize,
    pub end: usize,
    pub url: String,
}

/// `markdown` without its syntax.
pub fn plain(markdown: &str) -> String {
    plain_links(markdown).0
}

/// `markdown` without its syntax, and the links in that text.
pub fn plain_links(markdown: &str) -> (String, Vec<Link>) {
    let mut out = String::new();
    let mut links = Vec::new();
    let mut code = false;
    let mut first = true;
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            code = !code;
            continue;
        }
        if !first {
            out.push('\n');
        }
        first = false;
        if code {
            out.push_str(line);
        } else {
            inline(&block(line), &mut out, &mut links);
        }
    }
    (out, links)
}

/// `links` for the UI: one `start\tend\turl` line per link.
pub fn encode(links: &[Link]) -> String {
    let lines: Vec<String> = links
        .iter()
        .map(|l| format!("{}\t{}\t{}", l.start, l.end, l.url))
        .collect();
    lines.join("\n")
}

/// The url of the link in `encode`d `links` whose text holds byte `offset`
/// (either end included), or empty.
pub fn link_at(links: &str, offset: i32) -> String {
    let Ok(offset) = usize::try_from(offset) else {
        return String::new();
    };
    let hit = |line: &str| {
        let mut parts = line.splitn(3, '\t');
        let start: usize = parts.next()?.parse().ok()?;
        let end: usize = parts.next()?.parse().ok()?;
        let url = parts.next()?;
        (start <= offset && offset <= end).then(|| url.to_string())
    };
    links.lines().find_map(hit).unwrap_or_default()
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
/// its links as their text, added to `out`; each link goes to `links`.
fn inline(line: &str, out: &mut String, links: &mut Vec<Link>) {
    let mut rest = line;
    while let Some(c) = rest.chars().next() {
        if let Some(after) = rest.strip_prefix("**").or_else(|| rest.strip_prefix("__")) {
            rest = after;
        } else if let Some((span, after)) = enclosed(rest, '`', "`") {
            out.push_str(span);
            rest = after;
        } else if let Some((text, url, after)) = link(rest) {
            let start = out.len();
            out.push_str(text);
            links.push(Link { start, end: out.len(), url: url.to_string() });
            rest = after;
        } else {
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
}

/// `s` opening with `open` and closed by `close`: the inside and the rest.
fn enclosed<'a>(s: &'a str, open: char, close: &str) -> Option<(&'a str, &'a str)> {
    let inner = s.strip_prefix(open)?;
    let end = inner.find(close)?;
    Some((&inner[..end], &inner[end + close.len()..]))
}

/// `[text](url)` at the start of `s`: the text, the url and the rest.
fn link(s: &str) -> Option<(&str, &str, &str)> {
    let (text, after) = enclosed(s, '[', "]")?;
    let (url, rest) = enclosed(after, '(', ")")?;
    Some((text, url, rest))
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
    fn links_keep_their_text_and_place_outside_code_spans() {
        let (text, links) = super::plain_links("see [the log](https://x.y) and `[a](b)` now");
        assert_eq!(text, "see the log and [a](b) now");
        let want = super::Link { start: 4, end: 11, url: "https://x.y".into() };
        assert_eq!(links, [want]);
        let coded = super::encode(&links);
        assert_eq!(super::link_at(&coded, 6), "https://x.y");
        assert_eq!(super::link_at(&coded, 13), "");
        assert_eq!(super::link_at(&coded, -1), "");
    }

    #[test]
    fn a_lone_bracket_or_backtick_stays() {
        assert_eq!(plain("a [b c ` d"), "a [b c ` d");
    }
}
