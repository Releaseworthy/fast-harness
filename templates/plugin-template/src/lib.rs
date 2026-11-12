//! An example Fast Harness native plugin (docs/plugins.md in Fast Harness):
//! one tool the model can find and call, and one panel.

use std::sync::atomic::{AtomicU64, Ordering};

use fast_harness_core::ext::v1::{Ext, Host, Incoming, Panel, Tool, ToolCtx, ToolOutput, ToolSpec};
use serde_json::{json, Value};

// Fails to compile against another major version of the plugin API.
fast_harness_core::ext_api!(1);

/// Called once at start by the generated `register_all`.
pub fn register(ext: &mut Ext) {
    ext.add_tool(Box::new(WordCount));
    ext.add_panel(Box::new(Counter::default()));
}

/// `word_count`: counts the words of a text.
struct WordCount;

// A tool that only reads can add `fn effect(&self) -> Effect { Effect::Read }`
// (`ext::v1::Effect`) so its calls run in parallel with other reads. A tool
// that runs long can call `ctx.progress("new text")` while it works: front
// ends show it live; the returned `ToolOutput` is what the model reads.
impl Tool for WordCount {
    fn spec(&self) -> ToolSpec {
        // v1 structs are `#[non_exhaustive]`: build them with constructors.
        let schema = json!({"type": "object", "properties": {"text": {"type": "string"}},
            "required": ["text"]});
        ToolSpec::new("word_count", "Count the words of a text.", schema)
    }

    fn run(&self, args: &Value, _ctx: &ToolCtx<'_>) -> ToolOutput {
        match args["text"].as_str() {
            Some(text) => ToolOutput::ok(format!("{} words", text.split_whitespace().count())),
            None => ToolOutput::err("`text` (a string) is required"),
        }
    }
}

/// A panel with a button that counts its clicks (`/panel counter`).
#[derive(Default)]
struct Counter {
    clicks: AtomicU64,
}

impl Counter {
    fn view(&self) -> Incoming {
        let clicks = self.clicks.load(Ordering::Relaxed);
        Incoming::View(json!({"type": "column", "children": [
            {"type": "text", "text": format!("Clicked {clicks} times")},
            {"type": "button", "id": "add", "label": "Click"}
        ]}))
    }
}

impl Panel for Counter {
    fn id(&self) -> &str {
        "counter"
    }

    fn title(&self) -> &str {
        "Counter"
    }

    fn open(&self, _host: Host<'_>) -> Vec<Incoming> {
        vec![self.view()]
    }

    fn event(&self, kind: &str, id: &str, _value: &Value) -> Vec<Incoming> {
        if kind == "click" && id == "add" {
            self.clicks.fetch_add(1, Ordering::Relaxed);
        }
        vec![self.view()]
    }
}
