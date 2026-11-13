//! The runtime the CLI commands share (`fast_harness_core::app`).

use fast_harness_core::app::Runtime;

/// Loads settings and the account pool from `~/.fast-harness` or
/// `FAST_HARNESS_HOME`, and starts the background check of the newest Claude
/// Code release (PLAN.md D71).
pub fn runtime() -> Result<Runtime, String> {
    let home = crate::home()?;
    let runtime = Runtime::load_at(home)?;
    runtime.update_claude_code();
    Ok(runtime)
}
