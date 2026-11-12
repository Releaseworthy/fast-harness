//! `~/.fast-harness/trusted.json` (PLAN.md D41): the projects whose
//! `.fast-harness` folder may load. A project folder can name commands and
//! endpoints, so nothing in it is read until the user trusts it once.
//! Format: `{"version": 1, "paths": ["<canonical path>", ...]}`.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const FILE_NAME: &str = "trusted.json";
pub const VERSION: u32 = 1;

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Trusted {
    pub version: u32,
    pub paths: Vec<String>,
}

/// `path` made absolute with links resolved, without the Windows `\\?\`
/// prefix; the path itself when it cannot be resolved.
pub fn canonical(path: &Path) -> PathBuf {
    let full = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = full.to_string_lossy();
    let stripped = text
        .strip_prefix(r"\\?\")
        .filter(|rest| !rest.starts_with("UNC"))
        .map(PathBuf::from);
    stripped.unwrap_or(full)
}

fn key(path: &Path) -> String {
    let text = canonical(path).to_string_lossy().into_owned();
    match cfg!(windows) {
        true => text.to_lowercase(),
        false => text,
    }
}

/// The trusted list of `home`; a missing, unreadable or newer file is
/// empty (nothing trusted).
pub fn read(home: &Path) -> Trusted {
    crate::store::read(home.join(FILE_NAME))
        .ok()
        .and_then(|bytes| parse(&bytes).ok())
        .unwrap_or_default()
}

/// One `trusted.json`; a `version` newer than [`VERSION`] is an error.
pub fn parse(bytes: &[u8]) -> Result<Trusted, String> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    super::ext_file::check_version(value.get("version"))?;
    serde_json::from_value(value).map_err(|e| e.to_string())
}

/// Why `trusted.json` of `home` cannot be read; `None` when it can (or is missing).
pub fn problem(home: &Path) -> Option<String> {
    let bytes = crate::store::read(home.join(FILE_NAME)).ok()?;
    parse(&bytes).err()
}

/// True when `root` (a project folder) is in the trusted list.
pub fn is_trusted(home: &Path, root: &Path) -> bool {
    let wanted = key(root);
    read(home)
        .paths
        .iter()
        .any(|path| key(Path::new(path)) == wanted)
}

/// Adds `root` to the trusted list and returns its canonical path. The list
/// is re-read and written under its lock (`store::update`, PLAN.md D55);
/// an unreadable list starts empty, as [`read`] does. Trusting a trusted
/// path changes nothing.
pub fn trust(home: &Path, root: &Path) -> Result<PathBuf, String> {
    let path = canonical(root);
    if !path.is_dir() {
        return Err(format!("{} is not a directory", path.display()));
    }
    let file = home.join(FILE_NAME);
    let wanted = key(&path);
    let changed = crate::store::update(&file, crate::store::WAIT, |current| {
        let mut list: Trusted = match current.as_deref().map(parse) {
            // A newer file is never rewritten in an older format.
            Some(Err(reason)) if reason.contains("newer") => {
                return Err(io::Error::other(reason));
            }
            Some(Ok(list)) => list,
            _ => Trusted::default(),
        };
        list.version = VERSION;
        if !list
            .paths
            .iter()
            .any(|known| key(Path::new(known)) == wanted)
        {
            list.paths.push(path.to_string_lossy().into_owned());
        }
        let mut bytes = serde_json::to_vec_pretty(&list).map_err(io::Error::other)?;
        bytes.push(b'\n');
        Ok((bytes, ()))
    });
    changed.map_err(|err| format!("{}: {err}", file.display()))?;
    Ok(path)
}
