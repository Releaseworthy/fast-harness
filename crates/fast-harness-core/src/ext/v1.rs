//! Plugin API version 1 (PLAN.md D52, D57). Everything a plugin needs is
//! re-exported here, so a plugin imports only `fast_harness_core::ext::v1`.
//!
//! Unstable for every 0.x release (docs/plugins.md, "Stability"). Rules
//! that let a later commit grow this module without breaking a plugin:
//!
//! - Every struct with public fields and every enum here is
//!   `#[non_exhaustive]`: a plugin builds a struct through its constructor
//!   (`ToolSpec::new`, `Completion::new`, `Answer::default().with_output`)
//!   and matches an enum with a wildcard arm.
//! - A method added to `Tool`, `Provider`, `Hook`, `Panel` or `Transport`
//!   always has a default body.
//!
//! A struct literal does not compile outside the crate:
//!
//! ```compile_fail
//! use fast_harness_core::ext::v1::ToolSpec;
//! let _ = ToolSpec { name: "a".into(), description: "b".into(), schema: serde_json::Value::Null };
//! ```
//!
//! ```compile_fail
//! use fast_harness_core::ext::v1::{Completion, HttpHead, Message, StopReason, Timing, Usage};
//! let _ = Completion {
//!     message: Message::assistant("hi"),
//!     usage: Usage::default(),
//!     stop: StopReason::End,
//!     timing: Timing::default(),
//!     head: HttpHead::default(),
//!     transformations: None,
//! };
//! ```
//!
//! ```compile_fail
//! use fast_harness_core::ext::v1::Answer;
//! let _ = Answer { block: None, arguments: None, output: None, context: None, cont: None };
//! ```
//!
//! nor does a match without a wildcard arm:
//!
//! ```compile_fail
//! use fast_harness_core::ext::v1::AgentEvent;
//! fn name(event: &AgentEvent) -> &'static str {
//!     match event {
//!         AgentEvent::UserInput { .. } => "input",
//!         AgentEvent::TurnStart { .. } => "turn",
//!         AgentEvent::StepStart { .. } => "step",
//!         AgentEvent::Request { .. } => "request",
//!         AgentEvent::TextDelta(_) => "text",
//!         AgentEvent::ThinkingDelta(_) => "thinking",
//!         AgentEvent::ToolCallStart { .. } => "tool start",
//!         AgentEvent::ToolArgsDelta { .. } => "tool args",
//!         AgentEvent::ToolCall { .. } => "tool call",
//!         AgentEvent::ToolResult { .. } => "tool result",
//!         AgentEvent::Retry { .. } => "retry",
//!         AgentEvent::AccountSwitch { .. } => "switch",
//!         AgentEvent::UsageUpdate { .. } => "usage",
//!         AgentEvent::Subagent { .. } => "subagent",
//!         AgentEvent::SubagentProgress { .. } => "progress",
//!         AgentEvent::StepEnd { .. } => "step end",
//!         AgentEvent::Compaction { .. } => "compaction",
//!         AgentEvent::TurnEnd { .. } => "turn end",
//!         AgentEvent::Error(_) => "error",
//!     }
//! }
//! ```
//!
//! ```compile_fail
//! use fast_harness_core::ext::v1::HookEvent;
//! fn name(event: HookEvent) -> u8 {
//!     match event {
//!         HookEvent::SessionStart => 0,
//!         HookEvent::UserPromptSubmit => 1,
//!         HookEvent::PreToolUse => 2,
//!         HookEvent::PostToolUse => 3,
//!         HookEvent::PreCompact => 4,
//!         HookEvent::Stop => 5,
//!         HookEvent::SubagentStop => 6,
//!     }
//! }
//! ```
//!
//! The same code with constructors and wildcard arms compiles:
//!
//! ```
//! use fast_harness_core::ext::v1::*;
//! let _ = ToolSpec::new("a", "b", serde_json::Value::Null);
//! let _ = Completion::new(Message::assistant("hi"), Usage::default(), StopReason::End);
//! let _ = Answer::default().with_output("x");
//! let named = |event: HookEvent| match event {
//!     HookEvent::Stop => 5,
//!     _ => 0,
//! };
//! assert_eq!(named(HookEvent::Stop), 5);
//! ```

use std::sync::Arc;

pub use crate::config::compaction::CompactionPatch;
pub use crate::events::{AgentEvent, TurnOutcome};
pub use crate::http::{Flow, HttpError, HttpErrorKind, HttpHead, HttpRequest, Transport};
pub use crate::llm::custom::{ModelFile, Price, ProviderFile};
pub use crate::llm::{
    Auth, Cache, ChatRequest, Completion, Delta, ErrorKind, LlmError, Provider, Timing, ToolSpec,
};
pub use crate::panels::{Host, Incoming, NativePanel as Panel, Place};
pub use crate::tools::hooks::{Answer, HookEvent, NativeHook as Hook};
pub use crate::tools::{Effect, Part, Progress, Tool, ToolCtx, ToolOutput, ToolSession};
pub use crate::types::{Block, Message, Role, StopReason, Usage};

/// What the plugins of a build registered. Filled once at start by
/// `fast_harness_plugins::register_all`, then read by every settings load
/// (`Runtime::apply`): tools join every session's registry (deferred behind
/// `find_tools` like the built-ins), providers join the custom providers
/// (`<id>/<model>`), hooks run after the `hooks.json` entries, panels list
/// next to `panels/*.json` (a file with the same id wins).
#[derive(Clone, Default)]
pub struct Ext {
    tools: Vec<Arc<dyn Tool>>,
    providers: Vec<(Arc<dyn Provider>, ProviderFile)>,
    hooks: Vec<Arc<dyn Hook>>,
    panels: Vec<Arc<dyn Panel>>,
}

impl Ext {
    pub fn new() -> Ext {
        Ext::default()
    }

    /// A tool; a `tools/<name>.json` override file applies to it like to a
    /// built-in (description, `disabled`), and a built-in of the same name
    /// wins.
    pub fn add_tool(&mut self, tool: Box<dyn Tool>) {
        self.tools.push(Arc::from(tool));
    }

    /// A provider with its models, named `<provider id>/<model id>` in
    /// pickers and config. `file` carries the model entries (window, max
    /// output, efforts, price) and the credential (`key_env`, `account`);
    /// its `id` is set to the provider's. A `providers/<id>.json` file with
    /// the same id wins.
    pub fn add_provider(&mut self, provider: Arc<dyn Provider>, mut file: ProviderFile) {
        file.id = provider.id().to_string();
        self.providers.push((provider, file));
    }

    /// An in-process hook, run after the `hooks.json` entries of its event.
    pub fn add_hook(&mut self, hook: Box<dyn Hook>) {
        self.hooks.push(Arc::from(hook));
    }

    /// An in-process panel drawn like a `panels/*.json` panel.
    pub fn add_panel(&mut self, panel: Box<dyn Panel>) {
        self.panels.push(Arc::from(panel));
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
            && self.providers.is_empty()
            && self.hooks.is_empty()
            && self.panels.is_empty()
    }

    pub fn tools(&self) -> &[Arc<dyn Tool>] {
        &self.tools
    }

    pub fn providers(&self) -> &[(Arc<dyn Provider>, ProviderFile)] {
        &self.providers
    }

    pub fn hooks(&self) -> &[Arc<dyn Hook>] {
        &self.hooks
    }

    pub fn panels(&self) -> &[Arc<dyn Panel>] {
        &self.panels
    }
}
