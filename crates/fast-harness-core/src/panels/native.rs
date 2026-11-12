//! In-process panels of native plugins (PLAN.md D52, docs/plugins.md):
//! the same messages and view tree as a panel process (D46), returned by a
//! Rust object instead of written as JSON lines. Nothing runs before the
//! panel opens.

use std::fmt;
use std::sync::Arc;

use serde_json::Value;

use super::{Host, Incoming, Place};
use crate::events::AgentEvent;

/// A panel compiled into the binary. Every method answers with the
/// messages a panel process would print (`view`, `patch`, `subscribe`,
/// `notify`), so the GUI and the TUI draw it like any other panel.
pub trait NativePanel: Send + Sync {
    /// Unique among panels; a `panels/<id>.json` file with the same id wins.
    fn id(&self) -> &str;
    fn title(&self) -> &str;
    /// An icon name (`ui/icons.slint` names in the GUI); may be empty.
    fn icon(&self) -> &str {
        ""
    }
    fn place(&self) -> Place {
        Place::Tab
    }
    /// The panel opens: usually one `Incoming::View` with the whole tree.
    fn open(&self, host: Host<'_>) -> Vec<Incoming>;
    /// A `click`, `submit` or `change` on the node `id`.
    fn event(&self, kind: &str, id: &str, value: &Value) -> Vec<Incoming>;
    /// A session event of a kind the panel subscribed to (`turn`, `tool`,
    /// `usage`); the default ignores it.
    fn session(&self, _event: &AgentEvent) -> Vec<Incoming> {
        Vec::new()
    }
}

/// A shared native panel inside a [`super::PanelFile`]; equal only to
/// itself.
#[derive(Clone)]
pub struct NativeRef(pub Arc<dyn NativePanel>);

impl fmt::Debug for NativeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NativePanel({})", self.0.id())
    }
}

impl PartialEq for NativeRef {
    fn eq(&self, other: &NativeRef) -> bool {
        std::ptr::addr_eq(Arc::as_ptr(&self.0), Arc::as_ptr(&other.0))
    }
}

impl Eq for NativeRef {}
