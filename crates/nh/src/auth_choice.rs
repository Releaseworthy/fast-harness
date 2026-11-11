//! The runtime the CLI commands share (`nh_core::app`).

use nh_core::app::Runtime;

/// Loads settings and the account pool from `~/.nh` or `NH_HOME`.
pub fn runtime() -> Result<Runtime, String> {
    let home = crate::home()?;
    Runtime::load_at(home)
}
