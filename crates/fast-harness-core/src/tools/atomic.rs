//! Replacing a file through a temporary file in the same folder (finding
//! A3, PLAN.md D54): `std::fs::write` truncates in place, so a kill mid-write
//! cut the user's file; a rename leaves either the old or the new file.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::platform;

/// Makes the temporary names of one process unique.
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Writes `bytes` to `path`: a temporary file next to it is written, synced
/// and renamed over it. A symlink stays a link: its target is replaced. When
/// any of that fails (a sharing violation on Windows refuses the rename) the
/// temporary file goes and `path` is written in place, as before.
pub(crate) fn replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let Some(target) = resolve(path) else {
        return std::fs::write(path, bytes);
    };
    let Some(temp) = temp_path(&target) else {
        return std::fs::write(&target, bytes);
    };
    let renamed = write_temp(&temp, &target, bytes).and_then(|()| std::fs::rename(&temp, &target));
    if renamed.is_err() {
        let _ = std::fs::remove_file(&temp);
        return std::fs::write(&target, bytes);
    }
    Ok(())
}

/// The file to replace: `path`, or the resolved target when `path` is a
/// symlink (`None` for a link that does not resolve).
fn resolve(path: &Path) -> Option<PathBuf> {
    let link = std::fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink());
    match link {
        true => std::fs::canonicalize(path).ok(),
        false => Some(path.to_path_buf()),
    }
}

/// `.<name>.<pid>.<n>.tmp` in the folder of `target`.
fn temp_path(target: &Path) -> Option<PathBuf> {
    let name = target.file_name()?.to_string_lossy();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = format!(".{name}.{}.{n}.tmp", std::process::id());
    Some(target.with_file_name(temp))
}

/// Writes and syncs a new `temp`; it gets the permissions of an existing
/// `target` (Unix mode bits, `platform::copy_permissions`).
fn write_temp(temp: &Path, target: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    if target.is_file() {
        platform::copy_permissions(target, temp)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fh-atomic-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn replaces_and_creates_without_leaving_temp_files() {
        let dir = dir("replace");
        let file = dir.join("a.txt");
        replace(&file, b"first").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"first");
        replace(&file, b"second, longer").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"second, longer");
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["a.txt"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn temp_names_are_unique_and_beside_the_target() {
        let target = Path::new("some/dir/a.txt");
        let (one, two) = (temp_path(target).unwrap(), temp_path(target).unwrap());
        assert_ne!(one, two);
        assert_eq!(one.parent(), target.parent());
        assert!(temp_path(Path::new("/")).is_none());
    }
}
