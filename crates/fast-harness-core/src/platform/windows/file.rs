//! File permissions: Windows has only the read-only flag, and a read-only
//! target already refuses the rename, so there is nothing to copy.

use std::path::Path;

/// Gives `to` the permissions of `from`: a no-op on Windows.
pub fn copy_permissions(_from: &Path, _to: &Path) -> std::io::Result<()> {
    Ok(())
}
