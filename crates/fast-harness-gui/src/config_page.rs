//! Settings > Configuration (PLAN.md D41, D42): every resolved key with its
//! value and source, the extension files, and "Open config folder".

use fast_harness_core::app::Runtime;
use fast_harness_core::platform;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::app::{show_error, with_app, App};
use crate::{ConfigRow, ConfigUi, MainWindow};

/// Longest value shown in a row; the rest is elided.
const VALUE_CHARS: usize = 120;

/// `(key, value, source)` of every resolved key, sorted by key, then the
/// key bindings as `keys.<front>.<action>` (PLAN.md D45).
pub fn rows(rt: &Runtime) -> Vec<(String, String, String)> {
    let layered = &rt.layered;
    (layered.sources.keys())
        .filter_map(|key| {
            let (value, source) = layered.get(key)?;
            let value = value.to_string();
            let value = match value.char_indices().nth(VALUE_CHARS) {
                Some((cut, _)) => format!("{}...", &value[..cut]),
                None => value,
            };
            Some((key.clone(), value, source.word().to_string()))
        })
        .chain(key_rows(rt))
        .collect()
}

fn key_rows(rt: &Runtime) -> Vec<(String, String, String)> {
    let maps = [&rt.keys.tui, &rt.keys.gui];
    (maps.into_iter())
        .flat_map(|map| {
            let front = map.front.word();
            (map.rows().into_iter()).map(move |(name, keys, source)| {
                (
                    format!("keys.{front}.{name}"),
                    keys,
                    source.word().to_string(),
                )
            })
        })
        .collect()
}

/// `tools: a, b; agents: c; commands: none` of the index.
pub fn files(rt: &Runtime) -> String {
    let index = &rt.extensions.index;
    let names = |items: &[fast_harness_core::config::index::Item]| match items.is_empty() {
        true => "none".to_string(),
        false => (items.iter().map(|i| i.entry.name.as_str()))
            .collect::<Vec<_>>()
            .join(", "),
    };
    format!(
        "Tool files: {}. Agent types: {}. Commands: {}. Skills: {}.",
        names(&index.tools),
        names(&index.agents),
        names(&index.commands),
        names(&index.skills)
    )
}

pub fn show(app: &App, ui: &MainWindow) {
    let rows: Vec<ConfigRow> = rows(&app.rt)
        .into_iter()
        .map(|(key, value, source)| ConfigRow {
            key: key.into(),
            value: value.into(),
            source: source.into(),
            action: SharedString::new(),
        })
        .chain(crate::panels::config_rows(app))
        .collect();
    let g = ui.global::<ConfigUi>();
    g.set_rows(ModelRc::new(VecModel::from(rows)));
    let folder = format!("Config folder: {}", app.rt.home.display());
    g.set_folder(SharedString::from(folder));
    g.set_files(files(&app.rt).into());
}

pub fn connect(ui: &MainWindow) {
    // "Open" on a panels.<id> row (PLAN.md D46).
    ui.global::<ConfigUi>().on_action(|key| {
        with_app(|app, ui| crate::panels::config_action(app, ui, &key));
    });
    ui.global::<ConfigUi>().on_open_folder(|| {
        with_app(|app, ui| {
            if let Err(err) = platform::open_folder(&app.rt.home) {
                show_error(ui, format!("Cannot open {}: {err}", app.rt.home.display()));
            }
        });
    });
}
