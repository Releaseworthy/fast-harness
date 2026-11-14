//! `context` (PLAN.md D83): what keeps tool results small before they enter
//! the history. Nothing here edits a message already sent, so the cached
//! prefix and Claude's preserved thinking stay valid.

use serde::{Deserialize, Serialize};

/// Default `context.readHeadBytes`: half the 8 KB result budget. A read
/// without `offset` and `limit` returns whole lines up to this size, then a
/// note with the file's line count (D83 has the numbers).
pub const DEFAULT_READ_HEAD_BYTES: usize = 4 * 1024;

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
}

impl Default for ContextConfig {
    fn default() -> ContextConfig {
        ContextConfig {
            read_head_bytes: DEFAULT_READ_HEAD_BYTES,
            dedupe_reads: true,
        }
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
    }
}
