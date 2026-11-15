//! `context` (PLAN.md D83): what keeps tool results small before they enter
//! the history. Nothing here edits a message already sent, so the cached
//! prefix and Claude's preserved thinking stay valid.

use serde::{Deserialize, Serialize};

/// Default `context.readHeadBytes`: three quarters of the 8 KB result
/// budget. A read without `offset` and `limit` returns whole lines up to
/// this size, then a note with the file's line count (D83 chose 4 KB from
/// a cost model; D87 raised it: ~64 lines a read cost extra calls).
pub const DEFAULT_READ_HEAD_BYTES: usize = 6 * 1024;

/// Default `context.stepOutputBytes`: the results of one step together, so
/// a step of many broad calls (read + grep + glob) adds at most about 4k
/// tokens; one or two calls keep their own budget.
pub const DEFAULT_STEP_OUTPUT_BYTES: usize = 16 * 1024;

/// The smallest share a call of a crowded step gets.
pub const MIN_STEP_SHARE: usize = 2 * 1024;

/// Does a call of tool `name` share the step budget? Not `find_tools` (not
/// budgeted) and not a subagent's answer (its own `subagent.outputBytes`).
pub fn shares_step(name: &str) -> bool {
    !matches!(name, "find_tools" | "subagent" | "agent_output")
}

/// The `context` settings.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ContextConfig {
    /// Byte cap of a `read` without `offset` and `limit`; 0 uses the plain
    /// result budget (`toolOutput.read` or `toolOutputBytes`).
    pub read_head_bytes: usize,
    /// A `read` of lines an earlier read of this session already returned,
    /// the file unchanged (size and modified time), answers
    /// `[unchanged since call <id>]` instead of the lines again.
    pub dedupe_reads: bool,
    /// Byte budget of all the results of one step: each of the `n` calls
    /// that share it gets at most `stepOutputBytes / n` (never below
    /// [`MIN_STEP_SHARE`], never above its own budget); 0 turns it off.
    pub step_output_bytes: usize,
}

impl Default for ContextConfig {
    fn default() -> ContextConfig {
        ContextConfig {
            read_head_bytes: DEFAULT_READ_HEAD_BYTES,
            dedupe_reads: true,
            step_output_bytes: DEFAULT_STEP_OUTPUT_BYTES,
        }
    }
}

impl ContextConfig {
    /// The budget of one call whose own budget is `own` in a step where
    /// `sharing` calls share the step budget.
    pub fn step_share(&self, own: usize, sharing: usize) -> usize {
        if self.step_output_bytes == 0 || sharing <= 1 {
            return own;
        }
        let share = (self.step_output_bytes / sharing).max(MIN_STEP_SHARE);
        own.min(share)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_camel_case_and_missing_ones_take_the_default() {
        let parsed: ContextConfig = serde_json::from_str(r#"{"readHeadBytes": 2048}"#).unwrap();
        assert_eq!(parsed.read_head_bytes, 2048);
        assert!(parsed.dedupe_reads);
        let off: ContextConfig = serde_json::from_str(r#"{"dedupeReads": false}"#).unwrap();
        assert_eq!(off.read_head_bytes, DEFAULT_READ_HEAD_BYTES);
        assert!(!off.dedupe_reads);
        assert_eq!(off.step_output_bytes, DEFAULT_STEP_OUTPUT_BYTES);
    }

    #[test]
    fn a_crowded_step_splits_its_budget_and_a_lone_call_keeps_its_own() {
        let context = ContextConfig::default();
        assert_eq!(context.step_share(8192, 1), 8192);
        assert_eq!(context.step_share(8192, 2), 8192);
        assert_eq!(context.step_share(8192, 4), 4096);
        assert_eq!(context.step_share(8192, 20), MIN_STEP_SHARE);
        assert_eq!(context.step_share(1024, 20), 1024, "never above its own");
        let off: ContextConfig = serde_json::from_str(r#"{"stepOutputBytes": 0}"#).unwrap();
        assert_eq!(off.step_share(8192, 20), 8192);
    }
}
