//! Google's finish reasons and usage counts in Fast Harness terms.

use serde_json::Value;

use crate::types::{StopReason, Usage};

/// The stop reason of `finish_reason`; `called` when the answer carried a
/// function call.
///
/// - `STOP`: `End`, or `ToolUse` after a call.
/// - `OTHER`: `ToolUse` after a call (Antigravity ends Claude tool turns with
///   `OTHER`, DSH proxy `createResponseFixer`), else `Other`.
/// - `MAX_TOKENS`: `MaxTokens`.
/// - Content and prompt blocks (`SAFETY`, `RECITATION`, `LANGUAGE`,
///   `BLOCKLIST`, `PROHIBITED_CONTENT`, `SPII`, the `IMAGE_*` blocks and a
///   `promptFeedback.blockReason`, read as `PROMPT_<reason>`): `Refusal`.
/// - Anything else (`MALFORMED_FUNCTION_CALL`, `UNEXPECTED_TOOL_CALL`,
///   `TOO_MANY_TOOL_CALLS`, `NO_IMAGE`, unknown): `Other`.
///
/// Reasons: <https://ai.google.dev/api/generate-content#FinishReason> (read 2026-11-12).
pub fn stop_reason(finish_reason: &str, called: bool) -> StopReason {
    match finish_reason {
        "STOP" | "OTHER" if called => StopReason::ToolUse,
        "STOP" => StopReason::End,
        "MAX_TOKENS" => StopReason::MaxTokens,
        "SAFETY"
        | "RECITATION"
        | "LANGUAGE"
        | "BLOCKLIST"
        | "PROHIBITED_CONTENT"
        | "SPII"
        | "IMAGE_SAFETY"
        | "IMAGE_PROHIBITED_CONTENT"
        | "IMAGE_RECITATION" => StopReason::Refusal,
        reason if reason.starts_with("PROMPT_") => StopReason::Refusal,
        _ => StopReason::Other,
    }
}

fn count(meta: &Value, key: &str) -> u64 {
    meta[key].as_u64().unwrap_or(0)
}

/// Our four disjoint counts from Google's `usageMetadata`.
///
/// Google's `UsageMetadata` (<https://ai.google.dev/api/generate-content#UsageMetadata>,
/// read 2026-11-12): `promptTokenCount` "When `cached_content` is set, this is
/// still the total effective prompt size meaning this includes the number of
/// tokens in the cached content", and `totalTokenCount` is "prompt + tool-use
/// prompt + thoughts + response candidates", so `candidatesTokenCount` does
/// not include `thoughtsTokenCount` and `toolUsePromptTokenCount` is outside
/// the prompt count. Hence:
///
/// - `input` = `promptTokenCount` - `cachedContentTokenCount` + `toolUsePromptTokenCount`
/// - `output` = `candidatesTokenCount` + `thoughtsTokenCount`
/// - `cache_read` = `cachedContentTokenCount`, `cache_write` = 0
///
/// pi-ai maps input, output and cache read the same way (`google-generative-ai.ts`).
pub fn usage_from(meta: &Value) -> Usage {
    let prompt = count(meta, "promptTokenCount");
    let cached = count(meta, "cachedContentTokenCount");
    Usage {
        input: prompt.saturating_sub(cached) + count(meta, "toolUsePromptTokenCount"),
        output: count(meta, "candidatesTokenCount") + count(meta, "thoughtsTokenCount"),
        cache_read: cached,
        cache_write: 0,
        cache_write_1h: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reasons_and_usage() {
        assert_eq!(stop_reason("OTHER", false), StopReason::Other);
        assert_eq!(stop_reason("PROMPT_SAFETY", false), StopReason::Refusal);
        assert_eq!(
            stop_reason("MALFORMED_FUNCTION_CALL", true),
            StopReason::Other
        );
        let usage = usage_from(&json!({"promptTokenCount": 10, "cachedContentTokenCount": 20}));
        assert_eq!((usage.input, usage.cache_read), (0, 20));
    }
}
