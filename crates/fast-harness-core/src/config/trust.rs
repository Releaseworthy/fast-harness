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

/// The trusted list of `home`; a missing or unreadable file is empty.
pub fn read(home: &Path) -> Trusted {
    std::fs::read(home.join(FILE_NAME))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// True when `root` (a project folder) is in the trusted list.
pub fn is_trusted(home: &Path, root: &Path) -> bool {
    let wanted = key(root);
    read(home)
        .paths
        .iter()
        .any(|path| key(Path::new(path)) == wanted)
}

/// Adds `root` to the trusted list (written through a `.tmp` file) and
/// returns its canonical path. Trusting a trusted path changes nothing.
pub fn trust(home: &Path, root: &Path) -> Result<PathBuf, String> {
    let path = canonical(root);
    if !path.is_dir() {
        return Err(format!("{} is not a directory", path.display()));
    }
    let mut list = read(home);
    list.version = VERSION;
    if !is_trusted(home, &path) {
        list.paths.push(path.to_string_lossy().into_owned());
    }
    let file = home.join(FILE_NAME);
    let failed = |err: &dyn std::fmt::Display| format!("{}: {err}", file.display());
    let mut bytes = serde_json::to_vec_pretty(&list).map_err(|err| failed(&err))?;
    bytes.push(b'\n');
    std::fs::create_dir_all(home).map_err(|err| failed(&err))?;
    let tmp = home.join(format!("{FILE_NAME}.tmp"));
    std::fs::write(&tmp, bytes).map_err(|err| failed(&err))?;
    std::fs::rename(&tmp, &file).map_err(|err: io::Error| failed(&err))?;
    Ok(path)
}
