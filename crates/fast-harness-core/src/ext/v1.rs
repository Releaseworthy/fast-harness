//! Plugin API version 1 (PLAN.md D52). Everything a plugin needs is
//! re-exported here, so a plugin imports only `fast_harness_core::ext::v1`.

use std::sync::Arc;

pub use crate::events::AgentEvent;
pub use crate::http::Transport;
pub use crate::llm::custom::{ModelFile, ProviderFile};
pub use crate::llm::{Auth, ChatRequest, Completion, Delta, LlmError, Provider, ToolSpec};
pub use crate::panels::{Host, Incoming, NativePanel as Panel, Place};
pub use crate::tools::hooks::{Answer, HookEvent, NativeHook as Hook};
pub use crate::tools::{Effect, Tool, ToolCtx, ToolOutput};

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
