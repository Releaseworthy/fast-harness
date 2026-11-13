//! The model and effort picker lists and the live model list fetch, shared
//! by the TUI and the window (PLAN.md D59): the rows built from the live
//! catalog, the fetch thread, its result or error note, and the row that
//! keeps the highlight across a refresh. Each front end keeps its drawing
//! and hands the fetch result to its own UI thread.

use std::thread::{self, JoinHandle};

use super::words::Voice;
use crate::llm::anthropic::ModelInfo;
use crate::llm::catalog::{group_title, label as provider_label, provider_of, providers};
use crate::llm::{efforts, fit_effort};

/// One picker row: `(value, label)`; an empty value is a provider title row.
pub type Row = (String, String);

/// The result of one fetch: the list, or the reason it failed.
pub type Fetched = Result<Vec<ModelInfo>, String>;

/// True for a provider title row: never highlighted or picked.
pub fn is_header(row: &Row) -> bool {
    row.0.is_empty()
}

/// The model rows of `catalog` (the cached or fetched list) grouped by
/// provider (PLAN.md D24): a title row ("Claude" or a providers/*.json
/// title) before each non-empty group, and `configured` first in its group
/// (labelled by its id) when the catalog lacks it. The window labels a row
/// by its display name (see [`labelled`]) with " (deprecated)" after; the
/// terminal by the padded display name, the id and "  deprecated".
pub fn model_list(catalog: &[ModelInfo], configured: &str, voice: Voice) -> Vec<Row> {
    let listed = catalog.iter().any(|m| m.id == configured);
    let mut list = Vec::new();
    for provider in providers() {
        let group: Vec<&ModelInfo> = (catalog.iter())
            .filter(|m| provider_of(&m.id) == provider)
            .collect();
        let missing = !listed && !configured.is_empty() && provider_of(configured) == provider;
        let mine = if missing { configured } else { "" };
        let rows = match voice {
            Voice::Window => window_rows(&group, mine),
            Voice::Terminal => terminal_rows(&group, mine),
        };
        if !rows.is_empty() {
            list.push((String::new(), group_title(provider).to_string()));
            list.extend(rows);
        }
    }
    list
}

fn window_rows(group: &[&ModelInfo], mine: &str) -> Vec<Row> {
    let table: Vec<Row> = (group.iter())
        .map(|m| (m.id.clone(), m.display_name.clone()))
        .collect();
    let mut rows = labelled(&table, mine);
    for (id, label) in &mut rows {
        *label = provider_label(id, label);
        if group.iter().any(|m| &m.id == id && m.deprecated) {
            label.push_str(" (deprecated)");
        }
    }
    rows
}

fn terminal_rows(group: &[&ModelInfo], mine: &str) -> Vec<Row> {
    let mut rows: Vec<Row> = (group.iter())
        .map(|m| {
            let tag = if m.deprecated { "  deprecated" } else { "" };
            (
                m.id.clone(),
                format!("{:<18} {}{tag}", m.display_name, m.id),
            )
        })
        .collect();
    if !mine.is_empty() {
        rows.insert(0, (mine.to_string(), mine.to_string()));
    }
    rows
}

/// `(id, display name)` pairs to `(id, label)`: the display name, plus the
/// id in parentheses when another model shares the name; `configured` first
/// (labelled by its id) when the table lacks it.
pub fn labelled(table: &[Row], configured: &str) -> Vec<Row> {
    let mut list: Vec<Row> = table
        .iter()
        .map(|(id, name)| {
            let shared = table.iter().filter(|(_, other)| other == name).count() > 1;
            let label = if shared || name.is_empty() {
                format!("{name} ({id})").trim().to_string()
            } else {
                name.clone()
            };
            (id.clone(), label)
        })
        .collect();
    if !configured.is_empty() && !list.iter().any(|(id, _)| id == configured) {
        list.insert(0, (configured.to_string(), configured.to_string()));
    }
    list
}

/// The efforts `catalog` lists for `model`, else the provider's
/// (`llm::efforts`: the remembered live list, then the built-in table).
pub fn effort_list(catalog: &[ModelInfo], model: &str) -> Vec<String> {
    let listed = (catalog.iter())
        .find(|m| m.id == model)
        .map(|m| m.efforts.clone())
        .filter(|list| !list.is_empty());
    listed.unwrap_or_else(|| efforts(model))
}

/// The effort to keep when `model` is picked, and a note for the user when
/// it had to change (empty otherwise).
pub fn refit(model: &str, effort: Option<&str>) -> (Option<String>, String) {
    let fitted = fit_effort(model, effort);
    let note = match (effort, fitted.as_deref()) {
        (Some(old), Some(new)) if old != new => {
            format!("Effort set to {new}: {model} does not take {old}.")
        }
        _ => String::new(),
    };
    (fitted, note)
}

/// The row of `rows` with the value highlighted before a refresh
/// (`old[selected]`), so the highlight stays; `None` when it is gone.
pub fn kept_row(old: &[Row], selected: usize, rows: &[Row]) -> Option<usize> {
    let keep = &old.get(selected)?.0;
    (rows.iter())
        .position(|row| &row.0 == keep)
        .filter(|i| !is_header(&rows[*i]))
}

/// The note for a fetch that failed, worded for `voice`.
pub fn fetch_error(err: &str, voice: Voice) -> String {
    match voice {
        Voice::Window => format!("Could not fetch the model list: {err}"),
        Voice::Terminal => format!("error: {err}"),
    }
}

/// A fetch came back: the new catalog, or the note that says why it failed
/// (the pickers then keep their list).
pub fn fetched(result: Fetched, voice: Voice) -> Result<Vec<ModelInfo>, String> {
    result.map_err(|err| fetch_error(&err, voice))
}

/// Runs `fetch` on its own thread and hands the result to `deliver` there;
/// `deliver` posts it to the front end's UI thread.
pub fn spawn_fetch<F, D>(fetch: F, deliver: D) -> std::io::Result<JoinHandle<()>>
where
    F: FnOnce() -> Fetched + Send + 'static,
    D: FnOnce(Fetched) + Send + 'static,
{
    thread::Builder::new()
        .name("fast-harness-models".into())
        .spawn(move || deliver(fetch()))
}

/// What the window's pickers show after a fetch.
#[derive(Debug, PartialEq)]
pub struct Merged {
    /// The rows; `None` keeps the current list (the fetch failed).
    pub list: Option<Vec<Row>>,
    /// The effort refitted to the current model's live levels.
    pub effort: Option<String>,
    /// The picker note: why the effort changed, or why the fetch failed.
    pub note: String,
}

/// Merges a fetch into the pickers' state for the current `model` and
/// `effort`. A failure keeps the list and the effort and explains itself.
pub fn merge(result: Fetched, model: &str, effort: Option<&str>, voice: Voice) -> Merged {
    match fetched(result, voice) {
        Ok(models) => {
            let (effort, note) = refit(model, effort);
            Merged {
                list: Some(model_list(&models, model, voice)),
                effort,
                note,
            }
        }
        Err(note) => Merged {
            list: None,
            effort: effort.map(str::to_string),
            note,
        },
    }
}

#[cfg(test)]
#[path = "pickers_tests.rs"]
mod tests;
