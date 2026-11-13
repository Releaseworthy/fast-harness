//! Typed log records (PLAN.md D61): one variant per event type of the trace
//! format, whose serialised form is exactly the `data` of that line. A line
//! whose type or shape this build does not know (a missing required key, an
//! unknown key, another value type) is [`Record::Raw`] and writes back
//! unchanged. Keys a version added or a writer leaves out are `Option`s
//! skipped when `None`; keys written as `null` are [`Nullable`]s, so a read
//! record writes back the same bytes.

use crate::llm::Timing;
use crate::trace::kind;
use crate::types::{Message, StopReason, Usage};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// A key that may be missing (`None`), `null` (`Some(None)`) or a value.
pub type Nullable<T> = Option<Option<T>>;

fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Nullable<T>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// `turn/start`; `resume` (version 8) is written only when true.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnStart {
    pub turn: u32,
    #[serde(default, skip_serializing_if = "is_false")]
    pub resume: bool,
}

/// `user/message`; `kind` (version 8) marks a text added inside a turn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserMessage {
    pub turn: u32,
    pub message: Message,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// `step/start`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepStart {
    pub turn: u32,
    pub step: u32,
}

/// One tool of a `request/header`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeaderTool {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<Value>,
}

/// `request/header`: the frozen system and tools; `context` (version 5) is
/// the context stamp.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestHeader {
    pub system: String,
    pub tools: Vec<HeaderTool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<Value>,
}

/// `request/context`: one model call attempt; `wire` only with capture on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    pub turn: u32,
    pub step: u32,
    pub attempt: u32,
    pub provider: String,
    pub model: String,
    pub account: String,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub effort: Nullable<String>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub wire: Nullable<String>,
}

/// `assistant/attempt`: a failed attempt.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantAttempt {
    pub turn: u32,
    pub step: u32,
    pub attempt: u32,
    pub error: String,
    pub timing: Timing,
}

/// `llm/retry`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LlmRetry {
    pub turn: u32,
    pub step: u32,
    pub attempt: u32,
    pub delay_ms: u64,
    pub error: String,
}

/// `account/switch`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountSwitch {
    pub from: String,
    pub to: String,
    pub reason: String,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub hold_until: Nullable<u64>,
}

/// `assistant/message`; `context` and `window` are version 4. `stop` stays
/// the logged word so a newer reason still reads ([`AssistantMessage::stop_reason`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantMessage {
    pub turn: u32,
    pub step: u32,
    pub message: Message,
    pub usage: Usage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<u64>,
    pub stop: String,
    pub timing: Timing,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

impl AssistantMessage {
    /// The stop word as a [`StopReason`]; `Other` for a word this build does not know.
    pub fn stop_reason(&self) -> StopReason {
        serde_json::from_value(Value::String(self.stop.clone())).unwrap_or(StopReason::Other)
    }
}

/// `request/transformations` (version 4).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestTransformations {
    pub turn: u32,
    pub step: u32,
    pub transformations: Value,
    pub prefix_binding_mismatch: u64,
}

/// `compaction` (version 7, PLAN.md D38). Every key but `summary` may be
/// missing in a log written by hand or by an older build.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Compaction {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<u32>,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facts: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loaded: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kept: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mid: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto: Option<bool>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub model: Nullable<String>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub instructions: Nullable<String>,
}

/// `tool/call`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolCall {
    pub turn: u32,
    pub step: u32,
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

/// `tool/result`; `bytes` and `spilled` are version 6.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolResult {
    pub turn: u32,
    pub step: u32,
    pub message: Message,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spilled: Option<String>,
}

/// `step/end`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StepEnd {
    pub turn: u32,
    pub step: u32,
    pub duration_ms: u64,
}

/// `turn/end`; `outcome` is the logged word (`done`, `failed`, ...).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TurnEnd {
    pub turn: u32,
    pub steps: u32,
    pub usage: Usage,
    pub duration_ms: u64,
    pub outcome: String,
}

/// `tool/approval` (version 11, PLAN.md D64): how an approval request ended,
/// so a replay shows the same word a live transcript showed (`once`,
/// `session`, `declined`, `cancelled`, `paused`, `refused`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolApproval {
    pub turn: u32,
    pub step: u32,
    pub call_id: String,
    pub decision: String,
}

/// `hook` (version 10): `decision` is `none` or the keys that acted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hook {
    pub turn: u32,
    pub event: String,
    pub command: String,
    pub ms: u64,
    pub decision: String,
}

/// `hook/error` (version 10): a hook that failed and was ignored.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookError {
    pub turn: u32,
    pub event: String,
    pub command: String,
    pub ms: u64,
    pub error: String,
}

/// `undo` (version 12, PLAN.md D69): the user undid the `write` and
/// `edit` changes of `turn`; `restored` got their earlier content back,
/// `removed` had been created by that turn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Undo {
    pub turn: u32,
    pub restored: Vec<String>,
    pub removed: Vec<String>,
    #[serde(default)]
    pub forced: bool,
}

macro_rules! records {
    ($($variant:ident($ty:ident) = $kind:path,)*) => {
        /// The `data` of one log line, typed by its `type`.
        #[derive(Clone, Debug, PartialEq)]
        #[non_exhaustive]
        pub enum Record {
            $($variant($ty),)*
            /// A line whose type or shape this build does not know, kept as read.
            Raw { kind: String, data: Value },
        }

        impl Record {
            /// The event type written in the line's `type`.
            pub fn kind(&self) -> &str {
                match self {
                    $(Record::$variant(_) => $kind,)*
                    Record::Raw { kind, .. } => kind,
                }
            }

            /// The line's `data`: keys in the order the log has them.
            pub fn data(&self) -> serde_json::Result<Value> {
                match self {
                    $(Record::$variant(data) => serde_json::to_value(data),)*
                    Record::Raw { data, .. } => Ok(data.clone()),
                }
            }

            /// The record of a line of type `kind` with `data`; `Raw` when
            /// the type is unknown or the data does not have its shape.
            pub fn parse(kind: &str, data: &Value) -> Record {
                let typed = match kind {
                    $($kind => $ty::deserialize(data).map(Record::$variant).ok(),)*
                    _ => None,
                };
                typed.unwrap_or_else(|| Record::Raw {
                    kind: kind.to_string(),
                    data: data.clone(),
                })
            }
        }
    };
}

records! {
    TurnStart(TurnStart) = kind::TURN_START,
    UserMessage(UserMessage) = kind::USER_MESSAGE,
    StepStart(StepStart) = kind::STEP_START,
    RequestHeader(RequestHeader) = kind::REQUEST_HEADER,
    RequestContext(Request) = kind::REQUEST_CONTEXT,
    AssistantAttempt(AssistantAttempt) = kind::ASSISTANT_ATTEMPT,
    LlmRetry(LlmRetry) = kind::LLM_RETRY,
    AccountSwitch(AccountSwitch) = kind::ACCOUNT_SWITCH,
    AssistantMessage(AssistantMessage) = kind::ASSISTANT_MESSAGE,
    RequestTransformations(RequestTransformations) = kind::REQUEST_TRANSFORMATIONS,
    Compaction(Compaction) = kind::COMPACTION,
    ToolCall(ToolCall) = kind::TOOL_CALL,
    ToolResult(ToolResult) = kind::TOOL_RESULT,
    StepEnd(StepEnd) = kind::STEP_END,
    TurnEnd(TurnEnd) = kind::TURN_END,
    ToolApproval(ToolApproval) = kind::TOOL_APPROVAL,
    Hook(Hook) = kind::HOOK,
    HookError(HookError) = kind::HOOK_ERROR,
    Undo(Undo) = kind::UNDO,
}
