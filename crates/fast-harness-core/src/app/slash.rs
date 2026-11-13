//! Slash commands of every front end (PLAN.md D42): `/help` and the user's
//! `commands/*.md` (`/name args`: the body with `$ARGUMENTS` replaced, or
//! the arguments appended). `/compact` and `/trust` stay with their owners.

use std::fmt::Write;

use super::Runtime;
use crate::config::index::read_md;
use crate::tools::Extensions;

/// Slash commands the harness itself handles; a file cannot replace them.
const BUILT_IN: [(&str, &str); 7] = [
    (
        "undo [force]",
        "restore the files the last turn changed with write or edit (shell changes are not tracked)",
    ),
    (
        "image <path> | clear",
        "attach a png, jpeg, gif or webp file to the next prompt, or remove the attached images",
    ),
    (
        "compact [instructions]",
        "summarise the conversation so far",
    ),
    (
        "reflect [apply]",
        "propose memory, AGENTS.md, skill and tool edits from this session's signals (selfImprove)",
    ),
    ("trust", "trust this project's .fast-harness folder"),
    (
        "panel [id]",
        "open a panel of panels/*.json (docs/panels.md), or list them",
    ),
    ("help", "this list"),
];

/// What to do with a composer or `-p` text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Slash {
    /// Send this text (the input itself, or a command expanded).
    Send(String),
    /// Show this text; send nothing.
    Help(String),
    /// Show this error; send nothing.
    Error(String),
}

/// `text` resolved against `ext`'s commands.
pub fn slash(ext: &Extensions, text: &str) -> Slash {
    let Some(rest) = text.trim_start().strip_prefix('/') else {
        return Slash::Send(text.to_string());
    };
    let (name, args) = match rest.split_once(char::is_whitespace) {
        Some((name, args)) => (name, args.trim()),
        None => (rest.trim_end(), ""),
    };
    if name == "help" {
        return Slash::Help(help(ext));
    }
    let built_in = BUILT_IN
        .iter()
        .any(|(usage, _)| usage.split(' ').next() == Some(name));
    let item = ext.command(name).filter(|_| !built_in);
    let Some(item) = item else {
        return Slash::Send(text.to_string());
    };
    match read_md(&item.path) {
        Ok(file) => Slash::Send(expand_body(&file.body, args)),
        Err(err) => Slash::Error(format!("/{name}: {err}")),
    }
}

/// `Some(force)` for `/undo` and `/undo force` (PLAN.md D69); front ends
/// call `Session::undo_last_turn` with it instead of sending the text.
pub use crate::service::undo::command as undo_args;

/// The argument of `/image <path>` or `/image clear` (PLAN.md D70), `Some("")`
/// for `/image` alone, `None` for any other text: never sent to the model.
pub fn image_args(text: &str) -> Option<&str> {
    let rest = text.trim().strip_prefix("/image")?;
    (rest.is_empty() || rest.starts_with(char::is_whitespace)).then(|| rest.trim())
}

/// The arguments of a `/reflect` text (PLAN.md D50), `None` for any other.
pub fn reflect_args(text: &str) -> Option<&str> {
    let rest = text.trim().strip_prefix("/reflect")?;
    (rest.is_empty() || rest.starts_with(char::is_whitespace)).then(|| rest.trim())
}

/// `body` with `$ARGUMENTS` replaced by `args`; without the placeholder the
/// arguments follow the body after a blank line.
pub fn expand_body(body: &str, args: &str) -> String {
    let body = body.trim();
    if body.contains("$ARGUMENTS") {
        body.replace("$ARGUMENTS", args)
    } else if args.is_empty() {
        body.to_string()
    } else {
        format!("{body}\n\n{args}")
    }
}

/// The `/help` text: the built-in commands, then the user's.
pub fn help(ext: &Extensions) -> String {
    let mut out = String::from("Commands:");
    for (usage, what) in BUILT_IN {
        let _ = write!(out, "\n  /{usage}: {what}");
    }
    for item in &ext.index.commands {
        let entry = &item.entry;
        let _ = write!(
            out,
            "\n  /{}: {} ({})",
            entry.name, entry.description, item.source
        );
    }
    if ext.index.commands.is_empty() {
        out.push_str("\nAdd your own as commands/<name>.md (docs/extending.md).");
    }
    out
}

impl Runtime {
    /// [`slash`] with this runtime's command files.
    pub fn slash(&self, text: &str) -> Slash {
        slash(&self.extensions, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_replaces_or_appends() {
        assert_eq!(expand_body("Fix $ARGUMENTS now", "bug 1"), "Fix bug 1 now");
        assert_eq!(expand_body("Review\n", "src"), "Review\n\nsrc");
        assert_eq!(expand_body("Review", ""), "Review");
        let ext = Extensions::default();
        assert_eq!(slash(&ext, "/nope x"), Slash::Send("/nope x".into()));
        assert!(matches!(slash(&ext, "/help"), Slash::Help(text) if text.contains("/trust")));
    }
}
