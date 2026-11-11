//! The live model list: each time a model or effort picker opens, one
//! thread fetches every provider's model list (and its effort levels) and posts the
//! result back to the window, which updates the open list in place.

use std::thread::{self, JoinHandle};

use fast_harness_core::llm::anthropic::ModelInfo;

use crate::app::{with_app, App};
use crate::pickers::{model_list, refit};
use crate::{settings, MainWindow};

/// The result of one fetch: the list, or the user-facing reason it failed.
pub type Fetched = Result<Vec<ModelInfo>, String>;

/// What the pickers show after a fetch.
#[derive(Debug, PartialEq)]
pub struct Merged {
    /// `(id, label)` rows; `None` keeps the current list (the fetch failed).
    pub list: Option<Vec<(String, String)>>,
    /// The effort refitted to the current model's live levels.
    pub effort: Option<String>,
    /// The picker note: why the effort changed, or why the fetch failed.
    pub note: String,
}

/// Merges a fetch into the pickers' state for the current `model` and
/// `effort`. A failure keeps the list and the effort and explains itself.
pub fn merge(fetched: Fetched, model: &str, effort: Option<&str>) -> Merged {
    match fetched {
        Ok(models) => {
            let list = model_list(&models, model);
            let (effort, note) = refit(model, effort);
            Merged {
                list: Some(list),
                effort,
                note,
            }
        }
        Err(err) => Merged {
            list: None,
            effort: effort.map(str::to_string),
            note: format!("Could not fetch the model list: {err}"),
        },
    }
}

/// Runs `fetch` on its own thread and hands the result to `deliver` there.
pub fn spawn_fetch<F, D>(fetch: F, deliver: D) -> std::io::Result<JoinHandle<()>>
where
    F: FnOnce() -> Fetched + Send + 'static,
    D: FnOnce(Fetched) + Send + 'static,
{
    thread::Builder::new()
        .name("fast-harness-models".into())
        .spawn(move || deliver(fetch()))
}

/// A picker opened: starts one fetch unless one is running. The result
/// reaches `finish` on the UI thread through the Slint event loop.
pub fn start(app: &mut App, ui: &MainWindow) {
    if app.models_fetching {
        return;
    }
    let source = app.rt.models();
    let started = spawn_fetch(
        move || source.fetch(),
        |fetched| {
            let _ = slint::invoke_from_event_loop(move || {
                with_app(|app, ui| finish(app, ui, fetched));
            });
        },
    );
    match started {
        Ok(_) => {
            app.models_fetching = true;
            ui.set_models_refreshing(true);
        }
        Err(err) => ui.set_picker_note(format!("Could not fetch the model list: {err}").into()),
    }
}

/// The fetch came back: the lists change in place, the selection stays.
fn finish(app: &mut App, ui: &MainWindow, fetched: Fetched) {
    app.models_fetching = false;
    ui.set_models_refreshing(false);
    let model = app.rt.config.model.clone();
    let merged = merge(fetched, &model, app.rt.config.effort.as_deref());
    if let Some(list) = merged.list {
        app.rt.config.effort = merged.effort;
        settings::show_models(app, ui, list);
    }
    ui.set_picker_note(merged.note.into());
    app.refresh_status(ui);
}

#[cfg(test)]
mod tests {
    use super::*;
    use fast_harness_core::llm::anthropic::builtin_models;
    use std::sync::mpsc;

    fn fake(ids: &[(&str, bool)]) -> Vec<ModelInfo> {
        let base = builtin_models().remove(0);
        let one = |(id, deprecated): &(&str, bool)| ModelInfo {
            id: id.to_string(),
            display_name: id.to_uppercase(),
            deprecated: *deprecated,
            ..base.clone()
        };
        ids.iter().map(one).collect()
    }

    #[test]
    fn success_keeps_the_selection_and_labels_deprecated_models() {
        let models = fake(&[("new-a", false), ("old-b", true)]);
        let merged = merge(Ok(models), "old-b", None);
        let list = merged.list.unwrap();
        assert_eq!(list[0], (String::new(), "Claude".into()));
        assert_eq!(list[2], ("old-b".into(), "OLD-B (deprecated)".into()));
        assert_eq!(crate::pickers::model_index(&list, "old-b"), 2);
        assert_eq!(merged.note, "");
        let gone = merge(Ok(fake(&[("new-a", false)])), "mine", None);
        assert_eq!(gone.list.unwrap()[1], ("mine".into(), "mine".into()));
        let ag = merge(Ok(fake(&[("antigravity/gemini-r", false)])), "new-a", None);
        let labels: Vec<String> = ag.list.unwrap().into_iter().map(|(_, l)| l).collect();
        assert_eq!(
            labels,
            [
                "Claude",
                "new-a",
                "Google Antigravity",
                "ANTIGRAVITY/GEMINI-R"
            ]
        );
    }

    #[test]
    fn success_refits_the_effort() {
        let merged = merge(Ok(fake(&[("x", false)])), "claude-opus-4-5", Some("max"));
        assert_eq!(merged.effort.as_deref(), Some("high"));
        assert!(merged.note.contains("does not take max"), "{}", merged.note);
    }

    #[test]
    fn failure_keeps_the_list_and_notes_the_error() {
        let merged = merge(Err("no account".into()), "m", Some("low"));
        assert_eq!(merged.list, None);
        assert_eq!(merged.effort.as_deref(), Some("low"));
        assert_eq!(merged.note, "Could not fetch the model list: no account");
    }

    #[test]
    fn the_fetch_thread_delivers_a_fake_result() {
        let (tx, rx) = mpsc::channel();
        let fetch = || Ok(fake(&[("live-1", false)]));
        let handle = spawn_fetch(fetch, move |got| tx.send(got).unwrap()).unwrap();
        handle.join().unwrap();
        let merged = merge(rx.recv().unwrap(), "live-1", None);
        let header = (String::new(), "Claude".to_string());
        assert_eq!(
            merged.list.unwrap(),
            [header, ("live-1".into(), "LIVE-1".into())]
        );
    }
}
