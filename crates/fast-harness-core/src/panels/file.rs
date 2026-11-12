//! `panels/*.json` (docs/panels.md): `{"id", "title", "icon", "where",
//! "command", "args"}`. A later layer's panel with the same id replaces
//! the earlier one; a bad file is skipped with its reason.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::native::{NativePanel, NativeRef};
use crate::config::ext_file;
use crate::config::layers::{kind_files, LayerFile, Layers, Skipped, Source};

/// Where a panel opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Place {
    /// A closable tab, like Settings.
    Tab,
    /// A section of the sidebar.
    Sidebar,
}

impl Place {
    pub fn word(self) -> &'static str {
        match self {
            Place::Tab => "tab",
            Place::Sidebar => "sidebar",
        }
    }
}

/// One panel file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelFile {
    pub id: String,
    pub title: String,
    /// An icon name (`ui/icons.slint` names in the GUI); may be empty.
    pub icon: String,
    pub place: Place,
    pub command: String,
    pub args: Vec<String>,
    /// The file's folder: a relative `command` with a separator resolves
    /// against it, and the process sees it as `FAST_HARNESS_PANEL_DIR`.
    pub dir: PathBuf,
    pub source: Source,
    /// A native plugin's panel (D52), drawn in process instead of `command`.
    pub native: Option<NativeRef>,
}

#[derive(Deserialize)]
struct Raw {
    id: String,
    title: Option<String>,
    icon: Option<String>,
    #[serde(rename = "where")]
    place: Option<String>,
    command: String,
    #[serde(default)]
    args: Vec<String>,
}

/// The panels of every layer folder, the files that loaded and the skipped ones.
pub fn load(layers: &Layers) -> (Vec<PanelFile>, Vec<LayerFile>, Vec<Skipped>) {
    let (mut panels, mut loaded, mut skipped) = (Vec::<PanelFile>::new(), Vec::new(), Vec::new());
    for file in kind_files(layers, "panels", "json") {
        match read(&file.path, file.source) {
            Ok(panel) => {
                panels.retain(|p| p.id != panel.id);
                panels.push(panel);
                loaded.push(file);
            }
            Err(reason) => skipped.push(Skipped {
                path: file.path,
                reason,
            }),
        }
    }
    (panels, loaded, skipped)
}

fn read(path: &Path, source: Source) -> Result<PanelFile, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    ext_file::on(ext_file::json_header(&value)?)?;
    let raw: Raw = serde_json::from_value(value).map_err(|e| e.to_string())?;
    if ext_file::check_name(&raw.id).is_err() {
        return Err("id must be letters, digits, - or _".into());
    }
    if raw.command.trim().is_empty() {
        return Err("command is empty".into());
    }
    let place = match raw.place.as_deref() {
        None | Some("tab") => Place::Tab,
        Some("sidebar") => Place::Sidebar,
        Some(other) => return Err(format!("where must be tab or sidebar, not {other}")),
    };
    Ok(PanelFile {
        title: raw.title.unwrap_or_else(|| raw.id.clone()),
        id: raw.id,
        icon: raw.icon.unwrap_or_default(),
        place,
        command: raw.command,
        args: raw.args,
        dir: path.parent().map(Path::to_path_buf).unwrap_or_default(),
        source,
        native: None,
    })
}

impl PanelFile {
    /// The entry of a native plugin's panel (D52): no command, no file.
    pub fn native(panel: std::sync::Arc<dyn NativePanel>) -> PanelFile {
        PanelFile {
            id: panel.id().to_string(),
            title: panel.title().to_string(),
            icon: panel.icon().to_string(),
            place: panel.place(),
            command: String::new(),
            args: Vec::new(),
            dir: PathBuf::new(),
            source: Source::Default,
            native: Some(NativeRef(panel)),
        }
    }
}
