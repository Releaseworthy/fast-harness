//! The view tree (docs/panels.md) flattened into rows for list models: a
//! `column` stacks its children one row each (indented by depth), a `row`
//! puts the cells of its children side by side, a `table` gives a header
//! row and a row of cells per line, a `list` a row per item. An unknown
//! widget type becomes a `note` cell "unknown widget <type>".

use serde_json::Value;

/// Rows a view gives at most; the rest is cut with a note.
const MAX_ROWS: usize = 2000;
/// Characters of one cell's text at most (the software renderer lays text
/// out in i16 pixels, docs/pitfalls-gui.md).
const MAX_TEXT: usize = 12_000;
/// Deepest nesting followed.
const MAX_DEPTH: usize = 32;

/// One widget as drawn: `kind` is the widget type, or `item` (a list
/// entry), `cell` (a table cell, `header` for the first row) or `note`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cell {
    pub kind: String,
    pub id: String,
    /// The text, or the label of a button, input, select, checkbox or progress.
    pub text: String,
    /// An input's text, a select's chosen option, a list item's index.
    pub value: String,
    /// An input's placeholder.
    pub hint: String,
    pub options: Vec<String>,
    pub checked: bool,
    /// 0 to 1.
    pub progress: f32,
    pub header: bool,
}

/// One drawn line: its indentation depth and its cells, left to right.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub depth: usize,
    pub cells: Vec<Cell>,
}

/// The rows of `root`; a root `column` does not indent its children.
pub fn flatten(root: &Value) -> Vec<Row> {
    let mut out = Vec::new();
    match kind(root) {
        Some("column") => children(root).for_each(|c| walk(c, 0, &mut out)),
        _ => walk(root, 0, &mut out),
    }
    if out.len() > MAX_ROWS {
        out.truncate(MAX_ROWS);
        out.push(single(0, note(format!("view cut at {MAX_ROWS} rows"))));
    }
    out
}

/// Replaces the node with `id` in `root` by `node`; false when none has it.
pub fn patch(root: &mut Value, id: &str, node: Value) -> bool {
    if root.get("id").and_then(Value::as_str) == Some(id) {
        *root = node;
        return true;
    }
    let Some(kids) = root.get_mut("children").and_then(Value::as_array_mut) else {
        return false;
    };
    for kid in kids {
        if patch(kid, id, node.clone()) {
            return true;
        }
    }
    false
}

fn kind(node: &Value) -> Option<&str> {
    node.get("type").and_then(Value::as_str)
}

fn children(node: &Value) -> impl Iterator<Item = &Value> {
    node.get("children")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn single(depth: usize, cell: Cell) -> Row {
    Row {
        depth,
        cells: vec![cell],
    }
}

fn walk(node: &Value, depth: usize, out: &mut Vec<Row>) {
    if out.len() > MAX_ROWS {
        return;
    }
    if depth > MAX_DEPTH {
        return out.push(single(depth, note("view nested too deep".into())));
    }
    match kind(node) {
        Some("column") => children(node).for_each(|c| walk(c, depth + 1, out)),
        Some("row") => {
            let mut cells = Vec::new();
            inline(node, 0, &mut cells);
            out.push(Row { depth, cells });
        }
        Some("list") => list(node, depth, out),
        Some("table") => table(node, depth, out),
        _ => out.push(single(depth, cell(node))),
    }
}

/// The cells of `node` and its descendants on one line.
fn inline(node: &Value, level: usize, cells: &mut Vec<Cell>) {
    match kind(node) {
        Some("row" | "column") if level < MAX_DEPTH => {
            children(node).for_each(|c| inline(c, level + 1, cells))
        }
        Some("list" | "table") => cells.push(note("a list or table cannot sit in a row".into())),
        _ => cells.push(cell(node)),
    }
}

fn list(node: &Value, depth: usize, out: &mut Vec<Row>) {
    let id = text(node, "id");
    let items = node.get("items").and_then(Value::as_array);
    for (index, item) in items.into_iter().flatten().enumerate() {
        let item = Cell {
            kind: "item".into(),
            id: id.clone(),
            text: scalar(item),
            value: index.to_string(),
            ..Cell::default()
        };
        out.push(single(depth, item));
    }
}

fn table(node: &Value, depth: usize, out: &mut Vec<Row>) {
    let line = |values: &Value, header: bool| -> Vec<Cell> {
        (values.as_array().into_iter().flatten())
            .map(|v| Cell {
                kind: "cell".into(),
                text: scalar(v),
                header,
                ..Cell::default()
            })
            .collect()
    };
    if let Some(columns) = node.get("columns") {
        let cells = line(columns, true);
        out.push(Row { depth, cells });
    }
    let rows = node.get("rows").and_then(Value::as_array);
    for row in rows.into_iter().flatten() {
        let cells = line(row, false);
        out.push(Row { depth, cells });
    }
}

fn cell(node: &Value) -> Cell {
    let Some(kind) = kind(node) else {
        return note("unknown widget (no type)".into());
    };
    let label = match kind {
        "text" | "markdown" => text(node, "text"),
        _ => text(node, "label").or_else_text(|| text(node, "text")),
    };
    let known = [
        "text", "markdown", "button", "input", "select", "checkbox", "progress",
    ];
    if !known.contains(&kind) {
        return note(format!("unknown widget {kind}"));
    }
    let options: Vec<String> = (node.get("options").and_then(Value::as_array))
        .into_iter()
        .flatten()
        .map(scalar)
        .collect();
    Cell {
        kind: kind.into(),
        id: text(node, "id"),
        text: label,
        value: node.get("value").map(scalar).unwrap_or_default(),
        hint: text(node, "placeholder"),
        options,
        checked: node.get("checked").and_then(Value::as_bool) == Some(true),
        progress: progress(node),
        header: false,
    }
}

fn progress(node: &Value) -> f32 {
    let value = node.get("value").and_then(Value::as_f64).unwrap_or(0.0);
    value.clamp(0.0, 1.0) as f32
}

fn note(text: String) -> Cell {
    Cell {
        kind: "note".into(),
        text,
        ..Cell::default()
    }
}

fn text(node: &Value, key: &str) -> String {
    node.get(key).map(scalar).unwrap_or_default()
}

/// A string as is, a number or bool as JSON, null as "", cut at [`MAX_TEXT`].
fn scalar(value: &Value) -> String {
    let text = match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    };
    match text.char_indices().nth(MAX_TEXT) {
        Some((cut, _)) => format!("{}...", &text[..cut]),
        None => text,
    }
}

trait OrElseText {
    fn or_else_text(self, f: impl FnOnce() -> String) -> String;
}

impl OrElseText for String {
    fn or_else_text(self, f: impl FnOnce() -> String) -> String {
        match self.is_empty() {
            true => f(),
            false => self,
        }
    }
}
