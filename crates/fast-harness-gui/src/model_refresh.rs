//! The live model list: each time a model or effort picker opens, one
//! thread fetches every provider's model list (and its effort levels) and posts the
//! result back to the window, which updates the open list in place. The
//! fetch, its rows and its note are `app::pickers` (shared with the TUI,
//! D59); this file hands the result to the Slint event loop.

use fast_harness_core::app::pickers::{merge, spawn_fetch, Fetched};
use fast_harness_core::app::words::Voice;

use crate::app::{with_app, App};
use crate::{settings, MainWindow};
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
    let effort = app.rt.config.effort.as_deref();
    let merged = merge(fetched, &model, effort, Voice::Window);
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
    use crate::pickers::model_index;
    use fast_harness_core::llm::anthropic::builtin_models;

    /// The window's words and index of the shared merge (the fetch itself is
    /// tested in `app::pickers`).
    #[test]
    fn a_merge_in_window_words_keeps_the_dropdown_selection() {
        let mut old = builtin_models().remove(0);
        old.id = "old-b".into();
        old.display_name = "OLD-B".into();
        old.deprecated = true;
        let merged = merge(Ok(vec![old]), "old-b", None, Voice::Window);
        let list = merged.list.unwrap();
        assert_eq!(list[1], ("old-b".into(), "OLD-B (deprecated)".into()));
        assert_eq!(model_index(&list, "old-b"), 1);
        let failed: Fetched = Err("no account".into());
        let note = merge(failed, "m", None, Voice::Window).note;
        assert_eq!(note, "Could not fetch the model list: no account");
    }
}
