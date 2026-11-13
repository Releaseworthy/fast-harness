//! Each session's unsent composer text (PLAN.md D65). One composer serves
//! every session of a window or a terminal, so its text belongs to one
//! session at a time, the owner: the session shown, or the session being
//! opened while its log loads (D60). When the composer moves to another
//! session, the owner's text is saved and the other session's comes back.
//! `None` is the new session, not sent yet: it has its own slot, so a draft
//! typed there is still there when New session is pressed again. Drafts
//! live in memory only, for the life of the window or the terminal.

use std::collections::HashMap;

/// A composer's text and its cursor (a byte offset into `text`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    pub text: String,
    pub cursor: usize,
}

impl Draft {
    /// `text` with the cursor at its end (a front end that cannot read its
    /// cursor).
    pub fn at_end(text: impl Into<String>) -> Draft {
        let text = text.into();
        Draft {
            cursor: text.len(),
            text,
        }
    }
}

/// The saved drafts of the sessions the composer does not show, and its
/// owner. The owner's own draft is the composer's text, never saved here.
#[derive(Debug, Default)]
pub struct Drafts {
    owner: Option<String>,
    saved: HashMap<Option<String>, Draft>,
}

impl Drafts {
    /// The composer starts with session `owner` (`None`: a new session).
    pub fn new(owner: Option<&str>) -> Drafts {
        Drafts {
            owner: owner.map(str::to_string),
            saved: HashMap::new(),
        }
    }

    /// The session the composer's text belongs to (`None`: the new one).
    pub fn owner(&self) -> Option<&str> {
        self.owner.as_deref()
    }

    /// The composer moves to session `to` (`None`: the new session): the
    /// composer's `current` text is saved for the owner (an empty one
    /// clears its slot), and `to`'s draft comes back, taken out of the
    /// store (empty when it has none). `None` when `to` already owns it:
    /// the composer keeps its text and its cursor.
    pub fn switch(&mut self, current: Draft, to: Option<&str>) -> Option<Draft> {
        if self.owner.as_deref() == to {
            return None;
        }
        let from = self.owner.take();
        if current.text.is_empty() {
            self.saved.remove(&from);
        } else {
            self.saved.insert(from, current);
        }
        let to = to.map(str::to_string);
        let next = self.saved.remove(&to).unwrap_or_default();
        self.owner = to;
        Some(next)
    }

    /// The first prompt of the new session created session `id`: the
    /// composer, emptied by the send, now belongs to `id`, and the new
    /// session's slot stays empty. Another owner is left as it is.
    pub fn created(&mut self, id: &str) {
        if self.owner.is_none() {
            self.saved.remove(&None);
            self.owner = Some(id.to_string());
        }
    }

    /// The saved draft of `session` (`None`: the new session); the owner's
    /// text is the composer's, so it is never here.
    pub fn saved(&self, session: Option<&str>) -> Option<&Draft> {
        self.saved.get(&session.map(str::to_string))
    }
}

#[cfg(test)]
#[path = "drafts_tests.rs"]
mod tests;
