//! What a tool call returns (PLAN.md D58): parts (text now, another kind
//! later) behind constructors and accessors, so a new part kind changes no
//! plugin and no caller.

use std::borrow::Cow;

/// One piece of a tool's output. A later kind (an image) comes as a new
/// variant; the fields of a variant never change.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Part {
    /// Text the model reads.
    Text(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolOutput {
    parts: Vec<Part>,
    is_error: bool,
    /// Size of the full text before the budget cut it.
    bytes: usize,
    /// The spill file holding the full output, when it was cut.
    spilled: Option<String>,
}

impl ToolOutput {
    /// One text part.
    pub fn new(text: impl Into<String>, is_error: bool) -> ToolOutput {
        ToolOutput::from_parts(vec![Part::Text(text.into())], is_error)
    }

    pub fn ok(text: impl Into<String>) -> ToolOutput {
        ToolOutput::new(text, false)
    }

    pub fn err(text: impl Into<String>) -> ToolOutput {
        ToolOutput::new(text, true)
    }

    /// Several parts, in order.
    pub fn from_parts(parts: Vec<Part>, is_error: bool) -> ToolOutput {
        let mut output = ToolOutput {
            parts,
            is_error,
            bytes: 0,
            spilled: None,
        };
        output.bytes = output.text().len();
        output
    }

    pub fn parts(&self) -> &[Part] {
        &self.parts
    }

    /// The text parts joined in order without a separator (so splitting a
    /// text into parts does not change it); other kinds are skipped.
    pub fn text(&self) -> Cow<'_, str> {
        match self.parts.as_slice() {
            [Part::Text(text)] => Cow::Borrowed(text),
            parts => Cow::Owned(parts.iter().filter_map(Part::text).collect()),
        }
    }

    /// [`ToolOutput::text`] without a copy when there is one part.
    pub fn into_text(self) -> String {
        match <[Part; 1]>::try_from(self.parts) {
            Ok([Part::Text(text)]) => text,
            Err(parts) => parts.iter().filter_map(Part::text).collect(),
        }
    }

    pub fn is_error(&self) -> bool {
        self.is_error
    }

    /// Size of the full text before the budget cut it.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// The spill file holding the full output, when the budget cut it.
    pub fn spilled(&self) -> Option<&str> {
        self.spilled.as_deref()
    }

    /// Replaces the text parts with one text part first; other kinds stay.
    pub(crate) fn set_text(&mut self, text: String) {
        let others = self.parts.drain(..).filter(|part| part.text().is_none());
        self.parts = std::iter::once(Part::Text(text)).chain(others).collect();
    }

    /// Appends `text` to the last text part (a new one when there is none).
    /// `bytes` stays: it is the size the tool returned.
    pub(crate) fn push_text(&mut self, text: &str) {
        match self.parts.last_mut() {
            Some(Part::Text(last)) => last.push_str(text),
            _ => self.parts.push(Part::Text(text.to_string())),
        }
    }

    /// Records the full size and the spill file (the budget cut).
    pub(crate) fn set_size(&mut self, bytes: usize, spilled: Option<String>) {
        self.bytes = bytes;
        self.spilled = spilled;
    }
}

impl Part {
    /// The text of a text part.
    pub fn text(&self) -> Option<&str> {
        match self {
            Part::Text(text) => Some(text),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_join_and_accessors_read() {
        let one = ToolOutput::ok("hi");
        assert_eq!(
            (one.text(), one.is_error(), one.bytes()),
            ("hi".into(), false, 2)
        );
        assert!(matches!(one.text(), Cow::Borrowed(_)));
        assert_eq!(one.spilled(), None);
        let parts = vec![Part::Text("a".into()), Part::Text("bc".into())];
        let two = ToolOutput::from_parts(parts, true);
        assert_eq!(
            (two.text(), two.bytes(), two.is_error()),
            ("abc".into(), 3, true)
        );
        assert_eq!(two.clone().into_text(), "abc");
        let mut cut = two;
        cut.set_text("x".into());
        cut.push_text("y");
        cut.set_size(3, Some("f.txt".into()));
        assert_eq!(cut.parts(), [Part::Text("xy".into())]);
        assert_eq!((cut.bytes(), cut.spilled()), (3, Some("f.txt")));
    }
}
