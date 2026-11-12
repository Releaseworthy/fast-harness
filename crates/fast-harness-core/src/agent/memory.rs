//! Workspace memory (PLAN.md D50): `<app home>/memory/<cwd slug>/MEMORY.md`,
//! an index of at most [`MAX_INDEX_BYTES`] with one line per note, and the
//! notes in `notes/*.md`. A new session freezes the index into a second
//! system block after the stable prompt ([`crate::llm::split_memory`]); a
//! later change of `MEMORY.md` reaches a running session as a context
//! update. There is no memory tool: the block names the folder and the model
//! uses `read`, `write`, `edit` and `grep` on it (absolute paths are allowed).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::llm::{MEMORY_CLOSE, MEMORY_OPEN};
use crate::session;
use crate::store;

/// The byte cap of `MEMORY.md`; older lines roll into [`ARCHIVE_NOTE`].
pub const MAX_INDEX_BYTES: usize = 2048;
/// The index file name.
pub const INDEX: &str = "MEMORY.md";
/// The note compaction appends its durable facts to.
pub const FACTS_NOTE: &str = "notes/compaction-facts.md";
/// The note that keeps the index lines rolled out of `MEMORY.md`.
pub const ARCHIVE_NOTE: &str = "notes/index-archive.md";

/// The memory of one workspace, read now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Memory {
    pub dir: PathBuf,
    /// The text of `MEMORY.md` (empty when missing).
    pub index: String,
}

/// `<app_home>/memory/<cwd slug>`.
pub fn dir(app_home: &Path, cwd: &Path) -> PathBuf {
    app_home.join("memory").join(session::slug(cwd))
}

impl Memory {
    /// Reads the index of `cwd`'s memory folder; a missing file is empty.
    pub fn read(app_home: &Path, cwd: &Path) -> Memory {
        let dir = dir(app_home, cwd);
        let index = fs::read_to_string(dir.join(INDEX)).unwrap_or_default();
        Memory { dir, index }
    }

    /// The memory block that ends a new session's system prompt.
    pub fn block(&self) -> String {
        let index = self.index.trim();
        // The tags close the block: the index may not repeat them.
        let index = index
            .replace(MEMORY_OPEN, "memory_index")
            .replace(MEMORY_CLOSE, "/memory_index");
        let index = if index.is_empty() {
            "(no notes yet)".into()
        } else {
            index
        };
        format!(
            "{MEMORY_OPEN}\nMemory folder of this workspace: {}\n{INDEX} there lists the notes, \
             one line each (at most {MAX_INDEX_BYTES} bytes); notes/*.md hold the detail. Read a \
             note with read when its line matters, search them with grep, and keep what later \
             sessions need with write or edit (a note in notes/, then its line in {INDEX}).\n\n\
             {index}\n{MEMORY_CLOSE}",
            self.dir.display()
        )
    }
}

/// Appends `facts` under a `## date` heading to [`FACTS_NOTE`] and keeps its
/// line in the index up to date. Empty facts and `none` write nothing.
pub fn append_facts(dir: &Path, facts: &str, date: &str) -> io::Result<bool> {
    let facts = facts.trim();
    if facts.is_empty() || facts.eq_ignore_ascii_case("none") {
        return Ok(false);
    }
    let note = dir.join(FACTS_NOTE);
    store::update(&note, store::WAIT, |old| {
        let mut text = String::from_utf8_lossy(&old.unwrap_or_default()).into_owned();
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&format!("\n## {date}\n\n{facts}\n"));
        Ok((text.into_bytes(), ()))
    })?;
    let line = format!("- {FACTS_NOTE}: durable facts from compactions (last {date})");
    set_line(dir, FACTS_NOTE, &line)?;
    Ok(true)
}

/// Replaces the index line naming `note` (or appends it), then holds the cap;
/// the index is re-read and written under its lock (`store::update`).
pub fn set_line(dir: &Path, note: &str, line: &str) -> io::Result<()> {
    store::update(&dir.join(INDEX), store::WAIT, |old| {
        let old = String::from_utf8_lossy(&old.unwrap_or_default()).into_owned();
        let mut lines: Vec<String> = old.lines().map(str::to_string).collect();
        match lines.iter().position(|l| names(l, note)) {
            Some(at) => lines[at] = line.to_string(),
            None => lines.push(line.to_string()),
        }
        let (kept, rolled) = cap(lines, MAX_INDEX_BYTES);
        if !rolled.is_empty() {
            archive(dir, &rolled)?;
        }
        Ok((joined(&kept).into_bytes(), ()))
    })
}

fn names(line: &str, note: &str) -> bool {
    line.trim_start_matches(['-', '*', ' ']).starts_with(note)
}

fn joined(lines: &[String]) -> String {
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// Keeps the newest lines within `max` bytes: the oldest (first) lines roll
/// out, and the archive line takes the first place once anything rolled.
pub fn cap(mut lines: Vec<String>, max: usize) -> (Vec<String>, Vec<String>) {
    let archive_line = format!("- {ARCHIVE_NOTE}: older index lines, rolled out of {INDEX}");
    let mut rolled = Vec::new();
    let has_archive = |lines: &[String]| lines.iter().any(|l| names(l, ARCHIVE_NOTE));
    while joined(&lines).len() > max {
        if !has_archive(&lines) {
            lines.insert(0, archive_line.clone());
        }
        let oldest = lines.iter().position(|l| !names(l, ARCHIVE_NOTE));
        match oldest {
            // The newest line alone stays even when it is over the cap.
            Some(at) if lines.len() > 2 => rolled.push(lines.remove(at)),
            _ => break,
        }
    }
    (lines, rolled)
}

fn archive(dir: &Path, rolled: &[String]) -> io::Result<()> {
    store::update(&dir.join(ARCHIVE_NOTE), store::WAIT, |old| {
        let mut text = String::from_utf8_lossy(&old.unwrap_or_default()).into_owned();
        for line in rolled {
            text.push_str(line);
            text.push('\n');
        }
        Ok((text.into_bytes(), ()))
    })
}
