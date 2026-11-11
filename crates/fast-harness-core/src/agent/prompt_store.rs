//! Where the system prompt template comes from (PLAN.md D31): the
//! workspace's `prompts/<cwd slug>.md`, else the global `system-prompt.md`,
//! else the built-in default. Files are read on every call, so an edit
//! applies from the next turn; an unreadable file is skipped with a note.

use std::fmt;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use super::prompt::DEFAULT_TEMPLATE;
use crate::session;

/// The global template, under the app home.
pub const GLOBAL_FILE: &str = "system-prompt.md";
/// The folder of the per-workspace templates, under the app home.
pub const PROMPTS_DIR: &str = "prompts";

/// Which prompt file an edit goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// `system-prompt.md`, every workspace.
    Global,
    /// `prompts/<cwd slug>.md`, one workspace.
    Workspace,
}

/// Where a template came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Workspace(PathBuf),
    Global(PathBuf),
    Default,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::Workspace(path) => write!(f, "workspace prompt {}", path.display()),
            Source::Global(path) => write!(f, "global prompt {}", path.display()),
            Source::Default => f.write_str("built-in default prompt"),
        }
    }
}

/// A template with its source and the notes of the files skipped on the way.
#[derive(Clone, Debug)]
pub struct Template {
    pub text: String,
    pub source: Source,
    pub notes: Vec<String>,
}

/// The file of `scope` for the workspace `cwd` (the session folder slug).
pub fn path(home: &Path, cwd: &Path, scope: Scope) -> PathBuf {
    match scope {
        Scope::Global => home.join(GLOBAL_FILE),
        Scope::Workspace => home
            .join(PROMPTS_DIR)
            .join(format!("{}.md", session::slug(cwd))),
    }
}

/// The text of `file`; `None` when it does not exist.
pub fn read(file: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(file) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.to_string()),
    }
}

/// The saved text of `scope`; `None` without a file.
pub fn load(home: &Path, cwd: &Path, scope: Scope) -> Result<Option<String>, String> {
    read(&path(home, cwd, scope))
}

/// The template that applies to `cwd`: workspace, else global, else default.
pub fn template(home: &Path, cwd: &Path) -> Template {
    let mut notes = Vec::new();
    for scope in [Scope::Workspace, Scope::Global] {
        let file = path(home, cwd, scope);
        match read(&file) {
            Ok(Some(text)) => {
                let source = match scope {
                    Scope::Workspace => Source::Workspace(file),
                    Scope::Global => Source::Global(file),
                };
                return Template {
                    text,
                    source,
                    notes,
                };
            }
            Ok(None) => {}
            Err(err) => notes.push(format!(
                "system prompt {} is unreadable ({err}); using the next source",
                file.display()
            )),
        }
    }
    Template {
        text: DEFAULT_TEMPLATE.to_string(),
        source: Source::Default,
        notes,
    }
}

/// Writes `text` as the template of `scope` (temp file, then rename).
pub fn save(home: &Path, cwd: &Path, scope: Scope, text: &str) -> Result<(), String> {
    let file = path(home, cwd, scope);
    let fail = |err: std::io::Error| format!("cannot save {}: {err}", file.display());
    let dir = file.parent().unwrap_or(home);
    std::fs::create_dir_all(dir).map_err(fail)?;
    let tmp = file.with_extension("md.tmp");
    let written = std::fs::File::create(&tmp).and_then(|mut out| {
        out.write_all(text.as_bytes())?;
        out.sync_all()
    });
    if let Err(err) = written.and_then(|()| std::fs::rename(&tmp, &file)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(fail(err));
    }
    Ok(())
}

/// Deletes the template of `scope`; nothing to do without one.
pub fn reset(home: &Path, cwd: &Path, scope: Scope) -> Result<(), String> {
    let file = path(home, cwd, scope);
    match std::fs::remove_file(&file) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!("cannot delete {}: {err}", file.display())),
    }
}
