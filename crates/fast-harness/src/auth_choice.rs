//! The runtime the CLI commands share (`fast_harness_core::app`).

use fast_harness_core::app::Runtime;

/// Loads settings and the account pool from `~/.fast-harness` or `FAST_HARNESS_HOME`.
pub fn runtime() -> Result<Runtime, String> {
    let home = crate::home()?;
    Runtime::load_at(home)
}
