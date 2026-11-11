//! Google Antigravity (Cloud Code Assist): the Hub client identity shared with the
//! accounts, the chat provider and the model catalog (PLAN.md D23, D24).
//!
//! - `hub`: Hub version, User-Agent, hosts and JSON control calls.
//! - `request`: a [`ChatRequest`](crate::llm::ChatRequest) as Antigravity's envelope.
//! - `stream`: the `streamGenerateContent?alt=sse` answer assembled into a completion.
//! - `models`: `fetchAvailableModels` as model entries named `antigravity/<id>`, efforts.
//! - `provider`: [`Antigravity`], the [`Provider`](crate::llm::Provider).

pub mod hub;
pub mod models;
pub mod provider;
pub mod request;
pub mod stream;

use std::collections::HashMap;

pub use provider::Antigravity;

/// Prefix of Antigravity model ids in config, pickers and traces (D24).
pub const PREFIX: &str = "antigravity/";

/// The Antigravity id inside a Fast Harness model id: `antigravity/gemini-x` gives `gemini-x`.
pub fn bare_id(model: &str) -> Option<&str> {
    model.strip_prefix(PREFIX).filter(|id| !id.is_empty())
}

/// True for the model ids Antigravity serves (`antigravity/<id>`).
pub fn is_antigravity(model: &str) -> bool {
    bare_id(model).is_some()
}

/// Tool names as sent to Google (sanitised) mapped back to the agent's names:
/// `request` fills it, `stream` reads it to name the calls Google answers with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolNames {
    sent_to_original: HashMap<String, String>,
}

impl ToolNames {
    pub fn insert(&mut self, sent: &str, original: &str) {
        self.sent_to_original
            .insert(sent.to_string(), original.to_string());
    }

    /// The agent's name for `sent`; `sent` itself when unknown.
    pub fn original<'a>(&'a self, sent: &'a str) -> &'a str {
        self.sent_to_original
            .get(sent)
            .map(String::as_str)
            .unwrap_or(sent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_names() {
        assert_eq!(bare_id("antigravity/gemini-x"), Some("gemini-x"));
        assert_eq!(bare_id("antigravity/"), None);
        assert_eq!(bare_id("claude-opus-5-5"), None);
        assert!(is_antigravity("antigravity/claude-sonnet-5-5"));
        let mut names = ToolNames::default();
        names.insert("read_file_", "read-file.");
        assert_eq!(names.original("read_file_"), "read-file.");
        assert_eq!(names.original("other"), "other");
    }
}
