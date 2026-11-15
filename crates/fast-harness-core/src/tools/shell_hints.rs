//! Hints on a shell result (D89), one short line each and only when they
//! apply; the command is never blocked: a recursive listing that went into
//! folders `.gitignore` leaves out points at `glob`, reading a file with
//! `Get-Content`, `cat` or `type` points at `read`, and the working folder
//! repeated in front of many paths is left out once, with a note.

use std::path::{Path, MAIN_SEPARATOR};

use super::shell_ignored::ignored_dirs;
use super::ToolOutput;

/// Lines that must hold the working folder before it is left out.
const MIN_REPEATS: usize = 3;
const END_MARK: &str = "\n[exit code: ";
const READ_HINT: &str = "[hint: to read a file use the read tool (line numbers, ranges, \
                         images; find_tools loads it) instead of the shell]";

/// `output` of `command` run in `cwd` with the hints that apply, placed
/// before the closing `[exit code: N]` line.
pub(super) fn annotate(command: &str, cwd: &Path, mut output: ToolOutput) -> ToolOutput {
    let text = output.text().into_owned();
    let (body, end) = match text.rfind(END_MARK) {
        Some(at) => text.split_at(at + 1),
        None => (text.as_str(), ""),
    };
    let segments = segments(command);
    let mut hints = Vec::new();
    if segments.iter().any(|words| walks(words)) {
        let dirs = ignored_dirs(body, cwd);
        if !dirs.is_empty() {
            hints.push(format!(
                "[hint: this listing went into folders .gitignore leaves out ({}); \
                 the glob tool lists files without them (find_tools loads it)]",
                dirs.join(", ")
            ));
        }
    }
    if segments.iter().any(|words| reads_file(words)) {
        hints.push(READ_HINT.to_string());
    }
    let short = shorten_paths(body, cwd);
    if hints.is_empty() && short.is_none() {
        return output;
    }
    let mut out = match short {
        Some((short, note)) => {
            hints.insert(0, note);
            short
        }
        None => body.to_string(),
    };
    for hint in hints {
        out.push_str(&hint);
        out.push('\n');
    }
    out.push_str(end);
    output.set_text(out);
    output
}

/// The simple commands of `command`, each as its lower-case words without
/// quotes; a leading `cmd /c` is dropped.
fn segments(command: &str) -> Vec<Vec<String>> {
    let split = |c: char| matches!(c, ';' | '|' | '&' | '\n' | '(' | ')' | '{' | '}');
    command
        .split(split)
        .map(|segment| {
            let words: Vec<String> = segment
                .split_whitespace()
                .map(|word| word.trim_matches(['\'', '"']).to_ascii_lowercase())
                .collect();
            match words.as_slice() {
                [cmd, c, ..] if matches!(cmd.as_str(), "cmd" | "cmd.exe") && c == "/c" => {
                    words[2..].to_vec()
                }
                _ => words,
            }
        })
        .collect()
}

/// A recursive listing: `Get-ChildItem`, `gci`, `ls` or `dir` with
/// `-Recurse` (or a prefix of it, `-Depth`) or `dir /s`.
fn walks(words: &[String]) -> bool {
    let Some((first, rest)) = words.split_first() else {
        return false;
    };
    if !matches!(first.as_str(), "get-childitem" | "gci" | "ls" | "dir") {
        return false;
    }
    rest.iter().any(|word| {
        let flag = word.split(':').next().unwrap_or(word);
        let recurse = flag.len() >= 2 && "-recurse".starts_with(flag);
        recurse || (flag.len() >= 3 && "-depth".starts_with(flag)) || flag == "/s"
    })
}

/// Reading a file: `Get-Content`, `gc`, `cat` or `type` with a path, not
/// following a growing file (`-Wait`).
fn reads_file(words: &[String]) -> bool {
    let Some((first, rest)) = words.split_first() else {
        return false;
    };
    let reader = matches!(first.as_str(), "get-content" | "gc" | "cat" | "type");
    reader && !rest.is_empty() && !rest.iter().any(|word| word == "-wait")
}

/// `body` without the working folder in front of its paths, and the note
/// saying so, when at least [`MIN_REPEATS`] lines hold it.
fn shorten_paths(body: &str, cwd: &Path) -> Option<(String, String)> {
    let mut prefix = cwd.display().to_string();
    if !prefix.ends_with(MAIN_SEPARATOR) {
        prefix.push(MAIN_SEPARATOR);
    }
    let fold = |text: &str| match cfg!(windows) {
        true => text.to_ascii_lowercase(),
        false => text.to_string(),
    };
    // ASCII case folding keeps every byte offset.
    let (haystack, needle) = (fold(body), fold(&prefix));
    let lines = haystack.lines().filter(|l| l.contains(&needle)).count();
    if lines < MIN_REPEATS {
        return None;
    }
    let mut out = String::with_capacity(body.len());
    let mut at = 0;
    for (start, _) in haystack.match_indices(&needle) {
        out.push_str(&body[at..start]);
        at = start + needle.len();
    }
    out.push_str(&body[at..]);
    let note = format!("[paths under {prefix} are shown relative to it]");
    Some((out, note))
}

#[cfg(test)]
#[path = "shell_hints_tests.rs"]
mod tests;
