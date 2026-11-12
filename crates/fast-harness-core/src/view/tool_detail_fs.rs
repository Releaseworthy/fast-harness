//! Cards of the file tools: read, write, edit, grep and glob.

use super::diff::line_diff;
use super::tool_detail::{count, size, Call, LineKind, Tone, ToolDetail};

const MINUS: char = '\u{2212}';

/// "N: text" (a read line) as its number and text.
fn numbered(line: &str) -> Option<(u64, &str)> {
    let (number, text) = line.split_once(": ")?;
    let number = number.parse().ok()?;
    Some((number, text))
}

pub(crate) fn read(detail: &mut ToolDetail, call: &Call<'_>) {
    // No offset or limit fields: the "lines A–B" chip says it.
    detail.target = call.str("path").unwrap_or_default().to_string();
    let Some(out) = call.result() else { return };
    if out.trim() == "(empty file)" {
        return detail.chip("empty file", Tone::Plain);
    }
    let mut note = None;
    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("[lines ") {
            let range = rest.split(';').next().unwrap_or_default();
            note = Some(range.replacen(" to ", "-", 1));
        } else if let Some(rest) = line.strip_prefix("[showing lines ") {
            note = rest.split(';').next().map(str::to_string);
        } else if let Some((number, text)) = numbered(line) {
            detail.line(LineKind::Context, None, Some(number), text);
        }
    }
    let first = detail.lines.first().and_then(|l| l.new).unwrap_or(1);
    let last = detail.lines.last().and_then(|l| l.new).unwrap_or(first);
    let chip = match note.as_deref().and_then(|n| n.split_once(" of ")) {
        Some((range, total)) => format!("lines {} of {total}", range.replace('-', "\u{2013}")),
        None if first == 1 => format!("all {}", count(detail.lines.len(), "line", "lines")),
        None => format!("lines {first}\u{2013}{last}"),
    };
    detail.chip(chip, Tone::Info);
}

pub(crate) fn write(detail: &mut ToolDetail, call: &Call<'_>) {
    detail.target = call.str("path").unwrap_or_default().to_string();
    let content = call.str("content").unwrap_or_default();
    for (index, text) in content.lines().enumerate() {
        detail.line(LineKind::Added, None, Some(index as u64 + 1), text);
    }
    if let Some(out) = call.result() {
        if out.trim_end().ends_with("(new file)") {
            detail.chip("new file", Tone::Info);
        } else if let Some(old) = out.split_once("(replaced ").map(|(_, rest)| rest) {
            let lines = old.split(", ").nth(1).unwrap_or_default();
            let lines = lines.trim_end().trim_end_matches(')');
            detail.chip(format!("replaced {lines}"), Tone::Warning);
        }
    }
    let added = content.lines().count();
    detail.chip(format!("+{}", count(added, "line", "lines")), Tone::Success);
    detail.chip(size(content.len()), Tone::Plain);
}

pub(crate) fn edit(detail: &mut ToolDetail, call: &Call<'_>) {
    detail.target = call.str("path").unwrap_or_default().to_string();
    if call.args.get("replace_all").and_then(|v| v.as_bool()) == Some(true) {
        detail.field("replace all", "yes");
    }
    let out = call.result().unwrap_or_default();
    let at = out
        .rsplit_once(" at line")
        .map(|(_, rest)| rest.trim_start_matches('s').trim());
    let start = at.and_then(|at| at.split(',').next()?.trim().parse::<u64>().ok());
    let diff = line_diff(
        call.str("old_string").unwrap_or_default(),
        call.str("new_string").unwrap_or_default(),
    );
    let (mut old, mut new) = (start, start);
    let (mut added, mut removed) = (0, 0);
    for (kind, text) in diff {
        match kind {
            LineKind::Removed => removed += 1,
            LineKind::Added => added += 1,
            _ => {}
        }
        let shown_old = (kind != LineKind::Added).then_some(old).flatten();
        let shown_new = (kind != LineKind::Removed).then_some(new).flatten();
        detail.line(kind, shown_old, shown_new, &text);
        old = old.map(|n| n + u64::from(kind != LineKind::Added));
        new = new.map(|n| n + u64::from(kind != LineKind::Removed));
    }
    // A count of zero says nothing: a pure insert shows only "+n".
    if added > 0 {
        detail.chip(format!("+{added}"), Tone::Success);
    }
    if removed > 0 {
        detail.chip(format!("{MINUS}{removed}"), Tone::Error);
    }
    edit_result(detail, out, at);
}

/// The chips that come from edit's output.
fn edit_result(detail: &mut ToolDetail, out: &str, at: Option<&str>) {
    if let Some(at) = at.filter(|at| !at.is_empty()) {
        let word = if at.contains(',') { "lines" } else { "line" };
        detail.chip(format!("{word} {at}"), Tone::Plain);
    }
    let replaced = out
        .strip_prefix("Replaced ")
        .and_then(|rest| rest.split(' ').next());
    if let Some(k) = replaced
        .and_then(|k| k.parse::<usize>().ok())
        .filter(|&k| k > 1)
    {
        detail.chip(format!("{k} replacements"), Tone::Info);
    }
}

/// "PATH:N: text": the first ':' followed by digits and ": ".
fn match_line(line: &str) -> Option<(&str, u64, &str)> {
    for (index, _) in line.match_indices(':') {
        let rest = &line[index + 1..];
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 && rest[digits..].starts_with(": ") {
            let number = rest[..digits].parse().ok()?;
            return Some((&line[..index], number, &rest[digits + 2..]));
        }
    }
    None
}

pub(crate) fn grep(detail: &mut ToolDetail, call: &Call<'_>) {
    detail.target = call.str("pattern").unwrap_or_default().to_string();
    detail.field("path", call.str("path").unwrap_or("."));
    if let Some(glob) = call.text("glob") {
        detail.field("glob", glob);
    }
    if call.args.get("ignore_case").and_then(|v| v.as_bool()) == Some(true) {
        detail.field("ignore case", "yes");
    }
    if let Some(max) = call.text("max_results") {
        detail.field("max results", max);
    }
    let Some(out) = call.result() else { return };
    if out.trim() == "No matches." {
        return detail.chip("no matches", Tone::Plain);
    }
    let (groups, cut) = grep_groups(out);
    let matches: usize = groups.iter().map(|(_, lines)| lines.len()).sum();
    for (path, lines) in &groups {
        detail.line(LineKind::Header, None, None, path);
        for (number, text) in lines {
            detail.line(LineKind::Context, None, *number, text);
        }
    }
    let files = count(groups.len(), "file", "files");
    let found = count(matches, "match", "matches");
    detail.chip(format!("{found} in {files}"), Tone::Info);
    match cut {
        Some(more) if more.contains(' ') => detail.chip(more, Tone::Warning),
        Some(cut) => detail.chip(format!("cut at {cut}"), Tone::Warning),
        None => {}
    }
}

type Group<'a> = (&'a str, Vec<(Option<u64>, &'a str)>);

/// Matches grouped by path in first-seen order, and the cut note's number.
/// Reads both shapes: `path` lines with indented `N: text` lines (trace
/// version 6) and `path:N: text` lines (older logs).
fn grep_groups(out: &str) -> (Vec<Group<'_>>, Option<String>) {
    let mut groups: Vec<Group<'_>> = Vec::new();
    let mut cut = None;
    let mut more = None;
    let mut current = "";
    for line in out.lines().filter(|l| !l.is_empty()) {
        if let Some(rest) = line.strip_prefix("[results cut at ") {
            cut = rest.split(' ').next().map(str::to_string);
            continue;
        }
        if let Some(rest) = line
            .strip_prefix('[')
            .filter(|r| r.contains(" more matches in "))
        {
            more = rest.split(';').next().map(str::to_string);
            continue;
        }
        let (path, number, text) = match (match_line(line), line.strip_prefix("  ")) {
            (_, Some(entry)) => match numbered(entry) {
                Some((number, text)) => (current, Some(number), text),
                None => (current, None, entry),
            },
            (Some((path, number, text)), None) => (path, Some(number), text),
            (None, None) => {
                current = line;
                continue;
            }
        };
        match groups.iter_mut().find(|(p, _)| *p == path) {
            Some((_, lines)) => lines.push((number, text)),
            None => groups.push((path, vec![(number, text)])),
        }
    }
    (groups, more.or(cut))
}

pub(crate) fn glob(detail: &mut ToolDetail, call: &Call<'_>) {
    detail.target = call.str("pattern").unwrap_or_default().to_string();
    detail.field("path", call.str("path").unwrap_or("."));
    let Some(out) = call.result() else { return };
    if out.trim() == "No files found." {
        return detail.chip("no files", Tone::Plain);
    }
    let mut more = 0;
    for line in out.lines().filter(|l| !l.is_empty()) {
        let hidden = line
            .strip_prefix('[')
            .and_then(|r| r.split_once(" more files not shown"));
        match hidden.and_then(|(k, _)| k.parse::<usize>().ok()) {
            Some(k) => more = k,
            None => detail.line(LineKind::Context, None, None, line),
        }
    }
    let shown = detail.lines.len();
    if more > 0 {
        detail.lines_total = shown + more;
        detail.chip(
            format!("showing {shown} of {}", shown + more),
            Tone::Warning,
        );
    } else {
        detail.chip(count(shown, "file", "files"), Tone::Info);
    }
}
