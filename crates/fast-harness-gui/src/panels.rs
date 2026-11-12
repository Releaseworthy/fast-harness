//! Panels in the window (PLAN.md D46, docs/panels.md): Settings >
//! Configuration lists them as `panels.<id>` rows with "Open", the one
//! panel view on the "panel" page shows the open panel (tab and sidebar
//! panels alike, to stay in the exe size budget), the rows of
//! `fast_harness_core::panels` become `PanelRow`s, and widget events go back
//! to the process. A panel's reader thread wakes the UI thread through
//! `invoke_from_event_loop`.

use std::sync::Arc;

use fast_harness_core::panels::{self, Cell, Host, Row, Wake};
use serde_json::Value;
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::app::{show_error, with_app, App};
use crate::{ConfigRow, MainWindow, PanelRow, PanelUi};

/// Icon names a panel file may use (named as in `ui/icons.slint`).
const ICONS: [(&str, &str); 12] = [
    ("tools", "\u{eb6d}"),
    ("pulse", "\u{eb31}"),
    ("list-flat", "\u{eb84}"),
    ("eye", "\u{ea70}"),
    ("terminal", "\u{ea85}"),
    ("robot", "\u{ec20}"),
    ("file", "\u{ea7b}"),
    ("search", "\u{ea6d}"),
    ("watch", "\u{eb7c}"),
    ("info", "\u{ea74}"),
    ("settings-gear", "\u{eb51}"),
    ("folder", "\u{ea83}"),
];

/// `symbol-misc`, for an unknown or empty icon name.
const DEFAULT_ICON: &str = "\u{eb63}";

/// The key prefix of a panel row in Settings > Configuration.
pub const KEY_PREFIX: &str = "panels.";

pub fn glyph(name: &str) -> &'static str {
    ICONS
        .iter()
        .find(|(n, _)| *n == name)
        .map_or(DEFAULT_ICON, |(_, g)| *g)
}

/// The Settings > Configuration rows of the panels, each with "Open".
pub fn config_rows(app: &App) -> Vec<ConfigRow> {
    (app.panels.list.iter())
        .map(|panel| {
            let file = &panel.file;
            ConfigRow {
                key: format!("{KEY_PREFIX}{}", file.id).into(),
                value: format!("{} ({})", file.title, file.place.word()).into(),
                source: file.source.word().into(),
                action: "Open".into(),
            }
        })
        .collect()
}

/// Opens panel `id` (starting its command) on the "panel" page.
pub fn open(app: &mut App, ui: &MainWindow, id: &str) -> Result<(), String> {
    let dark = fast_harness_core::theme::ThemeSet::load(&app.rt.home)
        .get(&app.rt.config.theme)
        .dark;
    let session = app.runs.id.clone().unwrap_or_default();
    let Some(panel) = app.panels.get_mut(id) else {
        return Err(format!("no panel {id}"));
    };
    panel.open(Host::new(&app.cwd, &session, dark), wake());
    let g = ui.global::<PanelUi>();
    g.set_id(id.into());
    g.set_title(panel.file.title.as_str().into());
    g.set_icon(glyph(&panel.file.icon).into());
    ui.set_page("panel".into());
    show(app, ui);
    Ok(())
}

fn wake() -> Wake {
    Arc::new(|| {
        let _ = slint::invoke_from_event_loop(|| {
            with_app(|app, ui| {
                if app.panels.poll() {
                    show(app, ui);
                }
            });
        });
    })
}

/// Pushes the open panel's rows and status line into the window.
pub fn show(app: &App, ui: &MainWindow) {
    let g = ui.global::<PanelUi>();
    if let Some(p) = app.panels.get(&g.get_id()) {
        g.set_rows(rows(&p.rows));
        g.set_status(p.status.as_str().into());
        g.set_ended(p.ended);
    }
}

/// The open panel's cells, one per GUI row, with their depth (a `row`
/// node's cells follow each other); a table line is one `cell` with its
/// cells joined by tabs.
pub fn flat(rows: &[Row]) -> Vec<(usize, Cell)> {
    let mut out = Vec::new();
    for row in rows {
        let table = !row.cells.is_empty() && row.cells.iter().all(|c| c.kind == "cell");
        if table {
            let text: Vec<&str> = row.cells.iter().map(|c| c.text.as_str()).collect();
            let line = Cell {
                kind: "cell".into(),
                text: text.join("\t"),
                header: row.cells[0].header,
                ..Cell::default()
            };
            out.push((row.depth, line));
        } else {
            out.extend(row.cells.iter().map(|c| (row.depth, c.clone())));
        }
    }
    out
}

pub fn rows(rows: &[Row]) -> ModelRc<PanelRow> {
    let out: Vec<PanelRow> = (flat(rows).iter())
        .map(|(depth, c)| panel_row(*depth, c))
        .collect();
    ModelRc::new(VecModel::from(out))
}

fn panel_row(depth: usize, c: &Cell) -> PanelRow {
    let look = match c.kind.as_str() {
        "button" | "select" => 1,
        "input" => 2,
        "progress" => 3,
        "note" => 4,
        _ if c.header => 5,
        _ => 0,
    };
    let hint = if c.hint.is_empty() { &c.text } else { &c.hint };
    PanelRow {
        depth: i32::try_from(depth).unwrap_or(i32::MAX),
        look,
        text: display(c).into(),
        value: c.value.as_str().into(),
        hint: hint.as_str().into(),
        click: activate(c).is_some(),
        progress: c.progress,
    }
}

/// The text a row shows, with its marks: `[x]`, `•`, `!`, the percent.
pub fn display(c: &Cell) -> String {
    let label = |text: &str| match text.is_empty() {
        true => String::new(),
        false => format!("{text}: "),
    };
    match c.kind.as_str() {
        "checkbox" => format!("[{}] {}", if c.checked { "x" } else { " " }, c.text),
        "item" => format!("• {}", c.text),
        "note" => format!("! {}", c.text),
        "select" if c.value.is_empty() => format!("{}choose  >", label(&c.text)),
        "select" => format!("{}{}  >", label(&c.text), c.value),
        "progress" => format!("{}{:.0}%", label(&c.text), c.progress * 100.0),
        _ => c.text.clone(),
    }
}

/// The event a click on `c` sends; `None` when it is not clickable. A
/// select moves to its next option.
pub fn activate(c: &Cell) -> Option<(&'static str, Value)> {
    match c.kind.as_str() {
        "button" => Some(("click", Value::Null)),
        "item" => Some(("click", Value::String(c.text.clone()))),
        "checkbox" => Some(("change", Value::Bool(!c.checked))),
        "select" if !c.options.is_empty() => {
            let at = c.options.iter().position(|o| *o == c.value);
            let next = at.map_or(0, |i| (i + 1) % c.options.len());
            Some(("change", Value::String(c.options[next].clone())))
        }
        _ => None,
    }
}
/// `/panel [id]` typed in the composer: true when it was one.
pub fn slash(app: &mut App, ui: &MainWindow, text: &str) -> bool {
    let Some(id) = panels::slash_id(text) else {
        return false;
    };
    if id.is_empty() {
        ui.set_error_text(panels::listing(&app.panels).into());
    } else if let Err(err) = open(app, ui, id) {
        show_error(ui, format!("{err}\n{}", panels::listing(&app.panels)));
    }
    true
}

/// "Open" on a `panels.<id>` row of Settings > Configuration.
pub fn config_action(app: &mut App, ui: &MainWindow, key: &str) {
    if let Some(id) = key.strip_prefix(KEY_PREFIX) {
        if let Err(err) = open(app, ui, id) {
            show_error(ui, err);
        }
    }
}

pub fn connect(ui: &MainWindow) {
    let g = ui.global::<PanelUi>();
    g.on_activate(|index| {
        with_app(|app, ui| {
            let Some(panel) = app.panels.get(&ui.global::<PanelUi>().get_id()) else {
                return;
            };
            let cells = flat(&panel.rows);
            let cell = usize::try_from(index).ok().and_then(|i| cells.get(i));
            if let Some((_, cell)) = cell {
                if let Some((kind, value)) = activate(cell) {
                    panel.send(kind, &cell.id, value);
                }
            }
        });
    });
    g.on_submit(|index, text| {
        with_app(|app, ui| {
            let Some(panel) = app.panels.get(&ui.global::<PanelUi>().get_id()) else {
                return;
            };
            let cells = flat(&panel.rows);
            if let Some((_, cell)) = usize::try_from(index).ok().and_then(|i| cells.get(i)) {
                panel.send("submit", &cell.id, Value::String(text.into()));
            }
        });
    });
}
