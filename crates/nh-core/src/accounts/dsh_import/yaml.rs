//! A reader for the YAML subset DSH writes: nested block maps by indentation,
//! plain, single-quoted and double-quoted scalars, comments and blank lines.
//!
//! [`walk`] reports every scalar with its key path and raw text and keeps
//! nothing; the caller unquotes only the values it uses with [`scalar`].
//! Content outside the subset (sequences, block scalars, multi-line values) is
//! skipped without error.

/// Calls `visit(path, raw)` for every `key: value` line, where `path` holds the
/// keys of the enclosing maps and the key itself, and `raw` the value text.
pub(crate) fn walk(text: &str, visit: &mut dyn FnMut(&[&str], &str)) {
    let mut stack: Vec<(usize, &str)> = Vec::new();
    let mut skip_above: Option<usize> = None;
    let mut path: Vec<&str> = Vec::new();
    for line in text.lines() {
        let Some((indent, body)) = content(line) else {
            continue;
        };
        if let Some(limit) = skip_above {
            if indent > limit {
                continue;
            }
            skip_above = None;
        }
        if body == "---" || body == "..." || body.starts_with('%') {
            stack.clear();
            continue;
        }
        while stack.last().is_some_and(|(i, _)| *i >= indent) {
            stack.pop();
        }
        let Some((key, rest)) = split_key(body) else {
            // A sequence item or a line this reader does not know: skip it and
            // everything nested under it.
            skip_above = Some(indent);
            continue;
        };
        let rest = rest.trim();
        if rest.is_empty() || rest.starts_with('#') {
            stack.push((indent, key));
            continue;
        }
        skip_above = Some(indent);
        path.clear();
        path.extend(stack.iter().map(|(_, k)| *k));
        path.push(key);
        visit(&path, rest);
    }
}

/// The indentation and the text of a line, or `None` for blank and comment lines.
fn content(line: &str) -> Option<(usize, &str)> {
    let body = line.trim_start_matches(' ');
    let indent = line.len() - body.len();
    let body = body.trim_end();
    if body.is_empty() || body.starts_with('#') {
        return None;
    }
    Some((indent, body))
}

/// Splits `key: rest`. Keys may be plain or quoted; quoted keys keep only text
/// without escapes (enough for record names). `None` when the line is no pair.
fn split_key(body: &str) -> Option<(&str, &str)> {
    if body.starts_with("- ") || body == "-" {
        return None;
    }
    let (key, after) = match body.as_bytes().first()? {
        q @ (b'"' | b'\'') => {
            let end = body[1..].find(char::from(*q))? + 1;
            (&body[1..end], &body[end + 1..])
        }
        _ => {
            let colon = plain_colon(body)?;
            (body[..colon].trim_end(), &body[colon..])
        }
    };
    let rest = after.strip_prefix(':')?;
    if !(rest.is_empty() || rest.starts_with([' ', '\t'])) {
        return None;
    }
    Some((key, rest))
}

/// The byte offset of the `:` that ends a plain key.
fn plain_colon(body: &str) -> Option<usize> {
    let bytes = body.as_bytes();
    (0..bytes.len())
        .find(|&i| bytes[i] == b':' && bytes.get(i + 1).is_none_or(|b| *b == b' ' || *b == b'\t'))
}

/// The value of a scalar's raw text: quotes removed, escapes resolved, a
/// trailing comment dropped. `None` for anything that is not a single-line
/// scalar (block scalars, unterminated quotes, flow collections).
pub(crate) fn scalar(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let (value, rest) = match raw.as_bytes().first()? {
        b'"' => double_quoted(&raw[1..])?,
        b'\'' => single_quoted(&raw[1..])?,
        b'|' | b'>' | b'{' | b'[' | b'&' | b'*' | b'!' => return None,
        _ => return Some(plain(raw)),
    };
    let rest = rest.trim_start();
    (rest.is_empty() || rest.starts_with('#')).then_some(value)
}

fn plain(raw: &str) -> String {
    let end = raw
        .char_indices()
        .find(|&(i, c)| c == '#' && raw[..i].ends_with([' ', '\t']))
        .map_or(raw.len(), |(i, _)| i);
    raw[..end].trim_end().to_string()
}

/// Text after the opening `'`: `''` is a quote. Returns the value and the rest.
fn single_quoted(text: &str) -> Option<(String, &str)> {
    let mut out = String::new();
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c != '\'' {
            out.push(c);
        } else if chars.peek().is_some_and(|(_, n)| *n == '\'') {
            chars.next();
            out.push('\'');
        } else {
            return Some((out, &text[i + 1..]));
        }
    }
    None
}

/// Text after the opening `"`, with backslash escapes.
fn double_quoted(text: &str) -> Option<(String, &str)> {
    let mut out = String::new();
    let mut chars = text.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Some((out, &text[i + 1..])),
            '\\' => out.push(escape(&mut chars)?),
            _ => out.push(c),
        }
    }
    None
}

fn escape(chars: &mut std::str::CharIndices<'_>) -> Option<char> {
    let (_, c) = chars.next()?;
    let digits = match c {
        '0' => return Some('\0'),
        'a' => return Some('\u{7}'),
        'b' => return Some('\u{8}'),
        't' | '\t' => return Some('\t'),
        'n' => return Some('\n'),
        'v' => return Some('\u{b}'),
        'f' => return Some('\u{c}'),
        'r' => return Some('\r'),
        'e' => return Some('\u{1b}'),
        ' ' | '"' | '/' | '\\' => return Some(c),
        'N' => return Some('\u{85}'),
        '_' => return Some('\u{a0}'),
        'x' => 2,
        'u' => 4,
        'U' => 8,
        _ => return None,
    };
    let mut code = 0u32;
    for _ in 0..digits {
        code = code * 16 + chars.next()?.1.to_digit(16)?;
    }
    char::from_u32(code)
}
