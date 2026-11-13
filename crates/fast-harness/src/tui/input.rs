//! The multi-line prompt editor with a history of earlier prompts.

use fast_harness_core::app::drafts::Draft;

use super::wrap::{clusters, text_width};

#[derive(Default)]
pub struct Input {
    text: String,
    /// Byte offset, always on a character boundary.
    cursor: usize,
    history: Vec<String>,
    /// Position in `history` while browsing; the unsent text is kept aside.
    browsing: Option<usize>,
    draft: String,
    /// Shows bullets instead of the text (API keys).
    pub masked: bool,
}

impl Input {
    pub fn masked() -> Input {
        Input {
            masked: true,
            ..Input::default()
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.browsing = None;
    }

    /// The text and its cursor, taken out: another session's draft shows (D65).
    pub fn take_draft(&mut self) -> Draft {
        let draft = Draft {
            text: std::mem::take(&mut self.text),
            cursor: self.cursor,
        };
        self.clear();
        draft
    }

    /// Shows a session's draft; a cursor off a character boundary goes to
    /// the end.
    pub fn show_draft(&mut self, draft: Draft) {
        self.clear();
        let fits = draft.text.is_char_boundary(draft.cursor);
        self.cursor = if fits { draft.cursor } else { draft.text.len() };
        self.text = draft.text;
    }

    pub fn insert(&mut self, text: &str) {
        let clean = text.replace("\r\n", "\n").replace('\r', "\n");
        let clean: String = clean
            .chars()
            .filter(|c| *c == '\n' || !c.is_control())
            .collect();
        self.text.insert_str(self.cursor, &clean);
        self.cursor += clean.len();
    }

    fn prev_boundary(&self) -> usize {
        self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn next_boundary(&self) -> usize {
        self.text[self.cursor..]
            .chars()
            .next()
            .map_or(self.cursor, |c| self.cursor + c.len_utf8())
    }

    pub fn backspace(&mut self) {
        let start = self.prev_boundary();
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    pub fn delete(&mut self) {
        let end = self.next_boundary();
        self.text.replace_range(self.cursor..end, "");
    }

    pub fn left(&mut self) {
        self.cursor = self.prev_boundary();
    }

    pub fn right(&mut self) {
        self.cursor = self.next_boundary();
    }

    fn line_start(&self) -> usize {
        self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1)
    }

    fn line_end(&self) -> usize {
        self.text[self.cursor..]
            .find('\n')
            .map_or(self.text.len(), |i| self.cursor + i)
    }

    pub fn home(&mut self) {
        self.cursor = self.line_start();
    }

    pub fn end(&mut self) {
        self.cursor = self.line_end();
    }

    /// Moves to the same column of the line above; false on the first line.
    pub fn up(&mut self) -> bool {
        let start = self.line_start();
        if start == 0 {
            return false;
        }
        let column = self.text[start..self.cursor].chars().count();
        let above = self.text[..start - 1].rfind('\n').map_or(0, |i| i + 1);
        self.cursor = column_offset(&self.text, above, start - 1, column);
        true
    }

    /// Moves to the same column of the line below; false on the last line.
    pub fn down(&mut self) -> bool {
        let end = self.line_end();
        if end == self.text.len() {
            return false;
        }
        let column = self.text[self.line_start()..self.cursor].chars().count();
        let below_end = self.text[end + 1..]
            .find('\n')
            .map_or(self.text.len(), |i| end + 1 + i);
        self.cursor = column_offset(&self.text, end + 1, below_end, column);
        true
    }

    /// Up: the line above, else the previous prompt.
    pub fn up_or_history(&mut self) {
        if self.up() || self.history.is_empty() {
            return;
        }
        let index = match self.browsing {
            None => {
                self.draft = self.text.clone();
                self.history.len() - 1
            }
            Some(i) => i.saturating_sub(1),
        };
        self.show(index);
    }

    /// Down: the line below, else the next prompt or the unsent draft.
    pub fn down_or_history(&mut self) {
        if self.down() {
            return;
        }
        match self.browsing {
            Some(i) if i + 1 < self.history.len() => self.show(i + 1),
            Some(_) => {
                self.text = std::mem::take(&mut self.draft);
                self.cursor = self.text.len();
                self.browsing = None;
            }
            None => {}
        }
    }

    fn show(&mut self, index: usize) {
        self.browsing = Some(index);
        self.text = self.history[index].clone();
        self.cursor = self.text.len();
    }

    /// Takes the text out for sending and remembers it.
    pub fn submit(&mut self) -> String {
        let text = std::mem::take(&mut self.text);
        if self.history.last() != Some(&text) && !text.trim().is_empty() {
            self.history.push(text.clone());
        }
        self.clear();
        text
    }

    /// The rows to show at `width` columns and the cursor (row, column).
    pub fn layout(&self, width: usize) -> (Vec<String>, (usize, usize)) {
        let width = width.max(2);
        let mut rows = vec![String::new()];
        let mut used = 0;
        let mut cursor = (0, 0);
        let mut offset = 0;
        for cluster in clusters(&self.text) {
            if offset == self.cursor {
                cursor = (rows.len() - 1, used);
            }
            offset += cluster.len();
            if cluster == "\n" {
                rows.push(String::new());
                used = 0;
                continue;
            }
            let shown = if self.masked { "•" } else { cluster };
            let w = text_width(shown);
            if used + w > width - 1 {
                rows.push(String::new());
                used = 0;
            }
            if let Some(row) = rows.last_mut() {
                row.push_str(shown);
            }
            used += w;
        }
        if offset <= self.cursor {
            cursor = (rows.len() - 1, used);
        }
        (rows, cursor)
    }
}

/// Byte offset of `column` characters into the line `start..end`.
fn column_offset(text: &str, start: usize, end: usize, column: usize) -> usize {
    text[start..end]
        .char_indices()
        .nth(column)
        .map_or(end, |(i, _)| start + i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> Input {
        let mut input = Input::default();
        input.insert(text);
        input
    }

    #[test]
    fn edits_with_multibyte_characters() {
        let mut input = typed("Việt 漢");
        input.backspace();
        assert_eq!(input.text(), "Việt ");
        input.left();
        input.left();
        input.backspace();
        assert_eq!(input.text(), "Vit ");
        input.delete();
        assert_eq!(input.text(), "Vi ");
        input.home();
        input.insert("a\r\nb\u{1b}");
        assert_eq!(input.text(), "a\nbVi ");
        input.end();
        input.right();
        input.delete();
        assert_eq!(input.text(), "a\nbVi ");
    }

    #[test]
    fn moves_between_lines_and_history() {
        let mut input = typed("first");
        assert_eq!(input.submit(), "first");
        input.insert("second");
        input.submit();
        input.insert("one\ntwo");
        assert!(input.up());
        assert!(!input.up());
        assert!(input.down());
        input.up();
        input.up_or_history();
        assert_eq!(input.text(), "second");
        input.up_or_history();
        assert_eq!(input.text(), "first");
        input.up_or_history();
        assert_eq!(input.text(), "first");
        input.down_or_history();
        assert_eq!(input.text(), "second");
        input.down_or_history();
        assert_eq!(input.text(), "one\ntwo");
    }

    #[test]
    fn layout_wraps_and_masks() {
        let input = typed("abcd\nxy");
        let (rows, cursor) = input.layout(4);
        assert_eq!(rows, ["abc", "d", "xy"]);
        assert_eq!(cursor, (2, 2));
        let mut key = Input::masked();
        key.insert("sk-secret");
        let (rows, _) = key.layout(40);
        assert_eq!(rows, ["•••••••••"]);
        assert!(!rows[0].contains("secret"));
        let (rows, cursor) = typed("漢字").layout(4);
        assert_eq!(rows, ["漢", "字"]);
        assert_eq!(cursor, (1, 2));
    }
}
