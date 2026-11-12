//! File permissions: on Unix the mode bits of the old file go to its
//! replacement (an executable script stays executable); elsewhere nothing.

use std::path::Path;

/// Gives `to` the permissions (mode bits) of `from`.
#[cfg(unix)]
pub fn copy_permissions(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::set_permissions(to, std::fs::metadata(from)?.permissions())
}

/// Gives `to` the permissions of `from`: a no-op without Unix modes.
#[cfg(not(unix))]
pub fn copy_permissions(_from: &Path, _to: &Path) -> std::io::Result<()> {
    Ok(())
}
