//! Blanks out what is not code, then reads functions and imports out of the
//! text: the counter behind the size rule and the layering rule.

use regex::Regex;
use std::sync::OnceLock;

/// One function: from its `fn` line to the line of its closing brace.
pub(crate) struct Func {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) lines: usize,
}

/// The functions of one file, without its test module (see [`without_tests`]).
pub(crate) fn functions(path: &str, text: &str) -> Vec<Func> {
    let chars: Vec<char> = mask(&without_tests(text)).chars().collect();
    let lines = line_numbers(&chars);
    let mut out = Vec::new();
    let mut i = 0;
    while i + 2 < chars.len() {
        let starts = chars[i] == 'f'
            && chars[i + 1] == 'n'
            && !ident_char(chars[i + 2])
            && (i == 0 || !ident_char(chars[i - 1]));
        if starts {
            if let Some((name, open)) = signature(&chars, i + 2) {
                if let Some(close) = closing_brace(&chars, open) {
                    out.push(Func {
                        path: path.to_string(),
                        name,
                        lines: lines[close] - lines[i] + 1,
                    });
                }
            }
        }
        i += 1;
    }
    out
}

/// The `crate::<module>` imports of `text` with their line numbers.
pub(crate) fn imports(text: &str) -> Vec<(usize, String)> {
    let masked = mask(&without_tests(text));
    import_re()
        .captures_iter(&masked)
        .map(|hit| {
            let at = hit.get(0).map(|found| found.start()).unwrap_or(0);
            (masked[..at].matches('\n').count() + 1, hit[1].to_string())
        })
        .collect()
}

fn import_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\bcrate::([a-z_][a-z0-9_]*)").expect("import regex"))
}

/// The text without its test module: everything from a `#[cfg(test)]` line
/// that is followed by a `mod` line to the end of the file.
pub(crate) fn without_tests(text: &str) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut cut = lines.len();
    for i in 0..lines.len() {
        if lines[i].trim() != "#[cfg(test)]" {
            continue;
        }
        let after = &lines[i + 1..(i + 4).min(lines.len())];
        let opens_mod = after.iter().any(|line| {
            let text = line.trim_start();
            text.starts_with("mod ") || text.starts_with("pub mod ")
        });
        if opens_mod {
            cut = i;
            break;
        }
    }
    lines[..cut].concat()
}

/// `text` with comments, strings, char literals and raw strings as spaces, so
/// a brace or a word inside them is read as nothing. Newlines stay.
fn mask(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            i = skip_line(&chars, i, &mut out);
        } else if c == '/' && next == Some('*') {
            i = skip_block(&chars, i, &mut out);
        } else if c == '"' {
            i = skip_quoted(&chars, i, '"', &mut out);
        } else if c == '\'' && is_char_literal(&chars, i) {
            i = skip_quoted(&chars, i, '\'', &mut out);
        } else if let Some(hashes) = raw_open(&chars, i) {
            i = skip_raw(&chars, i, hashes, &mut out);
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

fn masked_char(c: char) -> char {
    if c == '\n' {
        '\n'
    } else {
        ' '
    }
}

fn ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn skip_line(chars: &[char], mut i: usize, out: &mut String) -> usize {
    while i < chars.len() && chars[i] != '\n' {
        out.push(' ');
        i += 1;
    }
    i
}

fn skip_block(chars: &[char], mut i: usize, out: &mut String) -> usize {
    let mut depth = 0usize;
    while i < chars.len() {
        if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
            depth += 1;
            out.push_str("  ");
            i += 2;
        } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
            depth -= 1;
            out.push_str("  ");
            i += 2;
            if depth == 0 {
                break;
            }
        } else {
            out.push(masked_char(chars[i]));
            i += 1;
        }
    }
    i
}

fn skip_quoted(chars: &[char], mut i: usize, quote: char, out: &mut String) -> usize {
    out.push(' ');
    i += 1;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' {
            out.push(' ');
            if let Some(escaped) = chars.get(i + 1) {
                out.push(masked_char(*escaped));
            }
            i += 2;
            continue;
        }
        out.push(masked_char(c));
        i += 1;
        if c == quote {
            break;
        }
    }
    i
}

/// A char literal such as `'{'`, `'\n'` or `'\''`; a lifetime is not one.
fn is_char_literal(chars: &[char], i: usize) -> bool {
    match chars.get(i + 1) {
        Some('\\') => chars.get(i + 3).is_some(),
        Some(_) => chars.get(i + 2) == Some(&'\''),
        None => false,
    }
}

/// The number of `#` of the raw string that starts at `i` (`0` for `r"`),
/// `None` when no raw string starts there.
fn raw_open(chars: &[char], i: usize) -> Option<usize> {
    if chars[i] != 'r' {
        return None;
    }
    if i > 0 && ident_char(chars[i - 1]) && chars[i - 1] != 'b' {
        return None;
    }
    if i > 1 && chars[i - 1] == 'b' && ident_char(chars[i - 2]) {
        return None;
    }
    let mut j = i + 1;
    let mut hashes = 0;
    while chars.get(j) == Some(&'#') {
        hashes += 1;
        j += 1;
    }
    if chars.get(j) == Some(&'"') {
        Some(hashes)
    } else {
        None
    }
}

fn skip_raw(chars: &[char], mut i: usize, hashes: usize, out: &mut String) -> usize {
    for _ in 0..hashes + 2 {
        if i < chars.len() {
            out.push(' ');
            i += 1;
        }
    }
    while i < chars.len() {
        let closes = chars[i] == '"' && (0..hashes).all(|k| chars.get(i + 1 + k) == Some(&'#'));
        if closes {
            for _ in 0..=hashes {
                if i < chars.len() {
                    out.push(' ');
                    i += 1;
                }
            }
            return i;
        }
        out.push(masked_char(chars[i]));
        i += 1;
    }
    i
}

fn line_numbers(chars: &[char]) -> Vec<usize> {
    let mut out = Vec::with_capacity(chars.len());
    let mut line = 1;
    for &c in chars {
        out.push(line);
        if c == '\n' {
            line += 1;
        }
    }
    out
}

/// The name and the index of the opening brace of the function whose `fn` token
/// ends at `i`: `None` for a `fn` type and for a declaration that ends in `;`.
fn signature(chars: &[char], mut i: usize) -> Option<(String, usize)> {
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    let start = i;
    while i < chars.len() && ident_char(chars[i]) {
        i += 1;
    }
    if i == start {
        return None;
    }
    let name: String = chars[start..i].iter().collect();
    let mut depth = 0i32;
    while i < chars.len() {
        match chars[i] {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ';' if depth == 0 => return None,
            '{' if depth == 0 => return Some((name, i)),
            _ => {}
        }
        i += 1;
    }
    None
}

fn closing_brace(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open;
    while i < chars.len() {
        match chars[i] {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}
