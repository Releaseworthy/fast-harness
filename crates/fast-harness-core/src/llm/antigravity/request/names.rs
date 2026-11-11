//! Function names Antigravity accepts: `[a-zA-Z0-9_-]`, at most 64
//! characters, first character a letter or `_` (DSH antigravity-switch
//! `sanitizeToolName`). Two originals never share a sent name.

use std::collections::HashMap;

use crate::llm::antigravity::ToolNames;

/// Longest function name Antigravity takes.
pub const MAX_NAME: usize = 64;

/// Sent names given so far in one request, both ways.
#[derive(Clone, Debug, Default)]
pub struct NameTable {
    sent_to_original: HashMap<String, String>,
    original_to_sent: HashMap<String, String>,
}

impl NameTable {
    /// The sent name of `original`: the one it already has, else its
    /// sanitised form, with `_2`, `_3`, ... when another original holds it.
    pub fn sent(&mut self, original: &str) -> String {
        if let Some(sent) = self.original_to_sent.get(original) {
            return sent.clone();
        }
        let clean = sanitize(original);
        let mut candidate = clean.clone();
        let mut n = 2u32;
        while self.sent_to_original.contains_key(&candidate) {
            let suffix = format!("_{n}");
            let keep = MAX_NAME.saturating_sub(suffix.len());
            candidate = format!("{}{suffix}", truncate(&clean, keep));
            n += 1;
        }
        self.sent_to_original
            .insert(candidate.clone(), original.to_string());
        self.original_to_sent
            .insert(original.to_string(), candidate.clone());
        candidate
    }

    /// The table as the stream reads it.
    pub fn into_tool_names(self) -> ToolNames {
        let mut names = ToolNames::default();
        for (sent, original) in &self.sent_to_original {
            names.insert(sent, original);
        }
        names
    }
}

/// `original` with every other character as `_`, a leading `_` when it does
/// not start with a letter or `_`, cut to 64 characters.
pub fn sanitize(original: &str) -> String {
    let mut clean: String = original
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let starts_well = clean
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    if !starts_well {
        clean.insert(0, '_');
    }
    truncate(&clean, MAX_NAME).to_string()
}

/// The first `max` characters (all ASCII after sanitising).
fn truncate(text: &str, max: usize) -> &str {
    match text.char_indices().nth(max) {
        Some((at, _)) => &text[..at],
        None => text,
    }
}
