//! Does a shell command only print a file (D91)? `Get-Content`, `gc`,
//! `cat` or `type` as a whole statement whose output goes straight out,
//! at most through `Select-Object -First/-Last/-Skip` or `Out-Host`. A
//! read whose lines feed `ConvertFrom-Json`, `Select-String`, `%`, a
//! variable or an expression is data processing: no hint to use `read`.

use std::path::{Path, PathBuf};

const READERS: &[&str] = &["get-content", "gc", "cat", "type"];
const CHANGE_DIR: &[&str] = &[
    "cd",
    "chdir",
    "set-location",
    "sl",
    "pushd",
    "push-location",
];
/// Stages after a read that still print it as it is.
const PRINTERS: &[&str] = &["out-host", "oh", "out-default", "more"];
const SELECT: &[&str] = &["select-object", "select"];
const SELECT_FLAGS: &[&str] = &["-first", "-last", "-skip", "-skiplast", "-index"];
/// `Get-Content` flags followed by a value that is not a path.
const VALUED: &[&str] = &[
    "-encoding",
    "-totalcount",
    "-head",
    "-first",
    "-tail",
    "-last",
    "-readcount",
    "-delimiter",
    "-stream",
    "-filter",
    "-include",
    "-exclude",
];

/// A command that prints a file and does nothing else with its text.
pub(super) struct PlainRead {
    /// The one file it reads, as written and resolved, when that is known:
    /// one read of one existing file, from a folder `cd` did not hide.
    pub file: Option<(String, PathBuf)>,
}

/// The plain read in `command` run in `cwd`, if it has one.
pub(super) fn plain_read(command: &str, cwd: &Path) -> Option<PlainRead> {
    let (mut dir, mut reads, mut file) = (Some(cwd.to_path_buf()), 0, None);
    let command = command.replace("&&", "\n").replace("||", "\n");
    for statement in command.split([';', '\n']) {
        let stages: Vec<Vec<String>> = statement.split('|').map(words).collect();
        let Some((name, args)) = stages[0].split_first() else {
            continue;
        };
        let name = name.to_ascii_lowercase();
        if CHANGE_DIR.contains(&name.as_str()) {
            dir = dir.and_then(|dir| changed(&dir, args));
            continue;
        }
        let read = READERS.contains(&name.as_str())
            && !args.is_empty()
            && !args.iter().any(|arg| arg.eq_ignore_ascii_case("-wait"))
            && stages[1..].iter().all(|stage| prints(stage));
        if read {
            reads += 1;
            file = dir.as_deref().and_then(|dir| single_file(args, dir));
        }
    }
    (reads > 0).then(|| PlainRead {
        file: file.filter(|_| reads == 1),
    })
}

/// The words of one pipeline stage without quotes; `cmd /c` goes.
fn words(stage: &str) -> Vec<String> {
    let words: Vec<String> = (stage.split_whitespace())
        .map(|word| word.trim_matches(['\'', '"']).to_string())
        .collect();
    match words.as_slice() {
        [cmd, c, ..]
            if matches!(cmd.to_ascii_lowercase().as_str(), "cmd" | "cmd.exe")
                && c.eq_ignore_ascii_case("/c") =>
        {
            words[2..].to_vec()
        }
        _ => words,
    }
}

/// Does this stage after a read print what it gets, at most cut short?
fn prints(stage: &[String]) -> bool {
    let Some((name, args)) = stage.split_first() else {
        return false;
    };
    let name = name.to_ascii_lowercase();
    if PRINTERS.contains(&name.as_str()) {
        return args.is_empty();
    }
    SELECT.contains(&name.as_str())
        && args.iter().all(|arg| {
            let flag = arg.to_ascii_lowercase();
            SELECT_FLAGS.contains(&flag.as_str())
                || arg.split(',').all(|n| n.parse::<u64>().is_ok())
        })
}

/// The folder `cd <args>` from `dir` goes to; `None` when it cannot be
/// told (a variable other than a leading `$env:NAME`, `~`, `-`).
fn changed(dir: &Path, args: &[String]) -> Option<PathBuf> {
    let target = args.iter().find(|arg| !arg.starts_with('-'))?;
    let target = match target.strip_prefix("$env:") {
        Some(rest) => {
            let at = rest.find(['\\', '/']).unwrap_or(rest.len());
            std::env::var(&rest[..at]).ok()? + &rest[at..]
        }
        None => target.clone(),
    };
    let plain = !target.contains('$') && !target.starts_with('~');
    plain.then(|| dir.join(target))
}

/// The one existing file the read `args` name, resolved against `dir`.
fn single_file(args: &[String], dir: &Path) -> Option<(String, PathBuf)> {
    let mut paths = Vec::new();
    let mut skip = false;
    for arg in args {
        let flag = arg.to_ascii_lowercase();
        if std::mem::take(&mut skip) || arg.starts_with('-') {
            skip = VALUED.contains(&flag.as_str());
            continue;
        }
        paths.push(arg);
    }
    let [name] = paths.as_slice() else {
        return None;
    };
    let path = dir.join(name);
    let plain = !name.contains(['$', '*', '?', '(', ')']) && path.is_file();
    plain.then(|| (name.to_string(), path))
}
