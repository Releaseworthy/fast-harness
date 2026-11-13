//! Putting a turn's files back (PLAN.md D69): all or nothing unless forced,
//! and the note the model gets with the next user message.

use std::fmt::Write;
use std::path::{Path, PathBuf};

use super::undo::{self, Manifest, UndoLog, UNTRACKED};

/// What an undo did. `refused` set: nothing changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UndoOutcome {
    /// The turn undone (or refused).
    pub turn: Option<u32>,
    /// Files whose earlier content was put back.
    pub restored: Vec<PathBuf>,
    /// Files the turn created, removed.
    pub removed: Vec<PathBuf>,
    /// Files changed since the turn ended (by the user or a later command).
    pub conflicts: Vec<PathBuf>,
    /// Why nothing was changed, in words.
    pub refused: Option<String>,
    /// Files that could not be put back (the rest were).
    pub errors: Vec<String>,
    /// The conflicting files were overwritten on request.
    pub forced: bool,
    /// [`UNTRACKED`]: what an undo cannot see.
    pub untracked: String,
}

impl UndoOutcome {
    fn refused(turn: Option<u32>, reason: String) -> UndoOutcome {
        UndoOutcome {
            turn,
            refused: Some(reason),
            untracked: UNTRACKED.to_string(),
            ..UndoOutcome::default()
        }
    }

    /// The result in words, for a transcript or a `-p` line.
    pub fn text(&self) -> String {
        let mut out = match (&self.refused, self.turn) {
            (Some(reason), _) => format!("Nothing was undone: {reason}."),
            (None, turn) => format!("Undid the file changes of turn {}.", turn.unwrap_or(0)),
        };
        let list = |paths: &[PathBuf]| -> String {
            let names: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
            names.join(", ")
        };
        for (label, paths) in [
            ("Restored", &self.restored),
            ("Removed (created by that turn)", &self.removed),
            ("Changed since that turn", &self.conflicts),
        ] {
            if !paths.is_empty() {
                let _ = write!(out, "\n{label}: {}", list(paths));
            }
        }
        for error in &self.errors {
            let _ = write!(out, "\nFailed: {error}");
        }
        let _ = write!(out, "\n{}", self.untracked);
        out
    }
}

impl UndoLog {
    /// Restores every file of the newest turn with changes not yet undone;
    /// a file it created is removed. A file changed since that turn stops
    /// the undo (nothing changes) unless `force`.
    pub fn undo_last(&self, force: bool) -> UndoOutcome {
        let Some(dir) = self.dir().map(Path::to_path_buf) else {
            return UndoOutcome::refused(None, "this session keeps no undo data".into());
        };
        let mut inner = self.lock();
        let Some(turn) = undo::last_open(&inner.turns).map(|m| m.turn) else {
            let reason = "no turn of this session changed files with write or edit, or each \
                          such turn was undone already";
            return UndoOutcome::refused(None, reason.into());
        };
        let Some(manifest) = inner.turns.get_mut(&turn) else {
            return UndoOutcome::refused(None, "the undo data changed meanwhile".into());
        };
        match restore(&dir, manifest, force) {
            Ok(outcome) => outcome,
            Err(outcome) => *outcome,
        }
    }

    /// The note for the next user message about undos the model was not
    /// told of yet; marks them told.
    pub(crate) fn take_note(&self) -> Option<String> {
        let dir = self.dir()?.to_path_buf();
        let mut inner = self.lock();
        let mut lines = Vec::new();
        for manifest in inner.turns.values_mut() {
            if manifest.undone && !manifest.noted {
                lines.push(note_line(manifest));
                manifest.noted = true;
                undo::save(&dir, manifest);
            }
        }
        (!lines.is_empty()).then(|| lines.join("\n\n"))
    }
}

/// Checks, then restores `manifest`'s files and marks it undone.
fn restore(
    dir: &Path,
    manifest: &mut Manifest,
    force: bool,
) -> Result<UndoOutcome, Box<UndoOutcome>> {
    let turn = Some(manifest.turn);
    let refuse = |reason: String| Box::new(UndoOutcome::refused(turn, reason));
    if let Some(reason) = &manifest.not_undoable {
        return Err(refuse(format!(
            "turn {} is not undoable: {reason}",
            manifest.turn
        )));
    }
    let conflicts = conflicts(manifest);
    if !conflicts.is_empty() && !force {
        let mut outcome = refuse(format!(
            "{} file(s) changed since turn {} ended; undo with force to overwrite them",
            conflicts.len(),
            manifest.turn
        ));
        outcome.conflicts = conflicts;
        return Err(outcome);
    }
    let priors = priors(dir, manifest).map_err(refuse)?;
    let mut outcome = UndoOutcome {
        turn,
        forced: force && !conflicts.is_empty(),
        conflicts,
        untracked: UNTRACKED.to_string(),
        ..UndoOutcome::default()
    };
    for (path, prior) in priors {
        put_back(&path, prior, &mut outcome);
    }
    manifest.undone = true;
    manifest.noted = false;
    undo::drop_blobs(dir, manifest);
    undo::save(dir, manifest);
    Ok(outcome)
}

/// Every kept prior state of `manifest` read before anything is written
/// (all or nothing): `None` for a file the turn created.
type Priors = Vec<(PathBuf, Option<Vec<u8>>)>;

fn priors(dir: &Path, manifest: &Manifest) -> Result<Priors, String> {
    let folder = undo::turn_dir(dir, manifest.turn);
    let mut priors = Vec::new();
    for file in &manifest.files {
        let path = file.path.display();
        let prior = match (&file.before, file.existed) {
            (Some(blob), _) => Some(
                crate::store::read(folder.join(blob))
                    .map_err(|err| format!("the kept copy of {path} cannot be read ({err})"))?,
            ),
            (None, true) => return Err(format!("no copy of {path} was kept")),
            (None, false) => None,
        };
        priors.push((file.path.clone(), prior));
    }
    Ok(priors)
}

/// The tracked files whose content is not what the turn left.
fn conflicts(manifest: &Manifest) -> Vec<PathBuf> {
    let current = |path: &Path| match std::fs::read(path) {
        Ok(bytes) => undo::hash(&bytes),
        Err(_) => String::new(),
    };
    (manifest.files.iter())
        .filter(|file| current(&file.path) != file.after)
        .map(|file| file.path.clone())
        .collect()
}

/// Writes `prior` back to `path`, or removes `path` when it did not exist.
fn put_back(path: &Path, prior: Option<Vec<u8>>, outcome: &mut UndoOutcome) {
    let done = match &prior {
        Some(bytes) => {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            super::atomic::replace(path, bytes)
        }
        None => match std::fs::remove_file(path) {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
            _ => Ok(()),
        },
    };
    match (done, prior.is_some()) {
        (Ok(()), true) => outcome.restored.push(path.to_path_buf()),
        (Ok(()), false) => outcome.removed.push(path.to_path_buf()),
        (Err(err), _) => outcome.errors.push(format!("{}: {err}", path.display())),
    }
}

/// `[undo] ...`: the model's view of one undone turn.
fn note_line(manifest: &Manifest) -> String {
    let names = |existed: bool| -> Vec<String> {
        (manifest.files.iter())
            .filter(|file| file.existed == existed)
            .map(|file| file.path.display().to_string())
            .collect()
    };
    let mut text = format!(
        "[undo] The user undid the file changes made with write and edit in turn {}.",
        manifest.turn
    );
    let (restored, removed) = (names(true), names(false));
    if !restored.is_empty() {
        let _ = write!(
            text,
            " Restored to their content before that turn: {}.",
            restored.join(", ")
        );
    }
    if !removed.is_empty() {
        let _ = write!(
            text,
            " Removed (that turn created them): {}.",
            removed.join(", ")
        );
    }
    text.push_str(" Read a file again before editing it.");
    text
}
