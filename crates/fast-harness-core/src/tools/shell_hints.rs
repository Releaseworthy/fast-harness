//! Hints on a shell result (D89), one short line each and only when they
//! apply; the command is never blocked: a recursive listing that went into
//! folders `.gitignore` leaves out points at `glob`, a command that only
//! prints a file points at `read` (D91: not one that processes the text),
//! and a folder repeated in front of many paths (the working folder, else
//! the one all listed paths share) is left out once, with a note.

use std::path::{Component, Path, PathBuf, MAIN_SEPARATOR};

use super::shell_ignored::{ignored_dirs, listed_path};
use super::shell_read::{plain_read, PlainRead};
use super::ToolOutput;

/// Lines that must hold the folder before it is left out.
const MIN_REPEATS: usize = 3;
/// Output lines looked at for a folder all listed paths share.
const MAX_LINES: usize = 20_000;
const END_MARK: &str = "\n[exit code: ";

/// `output` of `command` run in `cwd` with the hints that apply, placed
/// before the closing `[exit code: N]` line. A hint names `find_tools`
/// only for a tool not in `loaded` (empty: every tool is loaded).
pub(super) fn annotate(
    command: &str,
    cwd: &Path,
    loaded: &[String],
    mut output: ToolOutput,
) -> ToolOutput {
    let text = output.text().into_owned();
    let (body, end) = match text.rfind(END_MARK) {
        Some(at) => text.split_at(at + 1),
        None => (text.as_str(), ""),
    };
    let read = plain_read(command, cwd);
    let sized = read.as_ref().and_then(|read| file_sized(body, read));
    let body = sized.as_deref().unwrap_or(body);
    let mut hints: Vec<String> = glob_hint(command, body, cwd, loaded).into_iter().collect();
    if read.is_some() {
        hints.push(format!(
            "[hint: to read a file use the read tool (line numbers, ranges, images{}) \
             instead of the shell]",
            loads("; find_tools loads it", "read", loaded)
        ));
    }
    let short = shorten_paths(body, cwd);
    if hints.is_empty() && short.is_none() && sized.is_none() {
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

/// The hint for a recursive listing that went into ignored folders.
fn glob_hint(command: &str, body: &str, cwd: &Path, loaded: &[String]) -> Option<String> {
    if !segments(command).iter().any(|words| walks(words)) {
        return None;
    }
    let dirs = ignored_dirs(body, cwd);
    (!dirs.is_empty()).then(|| {
        format!(
            "[hint: this listing went into folders .gitignore leaves out ({}); \
             the glob tool lists files without them{}]",
            dirs.join(", "),
            loads(" (find_tools loads it)", "glob", loaded)
        )
    })
}

/// `words` when `tool` still has to be loaded by a find.
fn loads<'w>(words: &'w str, tool: &str, loaded: &[String]) -> &'w str {
    match loaded.is_empty() || loaded.iter().any(|name| name == tool) {
        true => "",
        false => words,
    }
}

/// The binary-output line of a read of one file, with that file's size:
/// pwsh decoded the bytes and wrote them again, so the stream size is not
/// the file's (D91).
fn file_sized(body: &str, read: &PlainRead) -> Option<String> {
    let (name, path) = read.file.as_ref()?;
    let binary = body.starts_with("[binary output, ") && body.lines().count() == 1;
    let size = std::fs::metadata(path).ok().filter(|_| binary)?.len();
    Some(format!("[binary output of {name}, {size} bytes]\n"))
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

/// `body` without the working folder in front of its paths, else without
/// the folder all its listed absolute paths share, and the note saying
/// so, when at least [`MIN_REPEATS`] lines hold it.
fn shorten_paths(body: &str, cwd: &Path) -> Option<(String, String)> {
    strip_folder(body, cwd).or_else(|| strip_folder(body, &shared_folder(body)?))
}

/// The folder (two names deep at least) every absolute path listed in
/// `body` lies in.
fn shared_folder(body: &str) -> Option<PathBuf> {
    let mut shared: Option<PathBuf> = None;
    for line in body.lines().take(MAX_LINES) {
        let Some(path) = listed_path(line).map(|(path, _)| Path::new(path)) else {
            continue;
        };
        if !path.is_absolute() {
            continue;
        }
        shared = Some(match shared {
            None => path.to_path_buf(),
            Some(shared) => (shared.components().zip(path.components()))
                .take_while(|(a, b)| a == b)
                .map(|(a, _)| a)
                .collect(),
        });
    }
    let shared = shared?;
    let names = (shared.components())
        .filter(|part| matches!(part, Component::Normal(_)))
        .count();
    (names >= 2).then_some(shared)
}

/// `body` without `folder` and a separator in front of its paths, and the
/// note saying so, when at least [`MIN_REPEATS`] lines hold it.
fn strip_folder(body: &str, folder: &Path) -> Option<(String, String)> {
    let mut prefix = folder.display().to_string();
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
