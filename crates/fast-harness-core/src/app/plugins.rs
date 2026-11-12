//! The registrations of native plugins (PLAN.md D52) merged into the
//! settings in `Runtime::apply`: files win over plugins on the same id.

use std::sync::Arc;

use crate::ext::Ext;
use crate::llm::custom::ProviderFile;
use crate::llm::Provider;
use crate::panels::PanelFile;

/// `panels` plus the plugins' panels whose id no file uses.
pub(super) fn with_panels(mut panels: Vec<PanelFile>, plugins: &Ext) -> Vec<PanelFile> {
    for panel in plugins.panels() {
        if !panels.iter().any(|file| file.id == panel.id()) {
            panels.push(PanelFile::native(Arc::clone(panel)));
        }
    }
    panels
}

/// `native plugins: tools a, b; providers p; hooks h; panels x` for
/// `doctor`; `None` without registrations (the official builds).
pub(super) fn summary(plugins: &Ext) -> Option<String> {
    if plugins.is_empty() {
        return None;
    }
    let tools: Vec<String> = plugins.tools().iter().map(|t| t.spec().name).collect();
    let providers: Vec<&str> = plugins.providers().iter().map(|p| p.0.id()).collect();
    let hooks: Vec<&str> = plugins.hooks().iter().map(|h| h.name()).collect();
    let panels: Vec<&str> = plugins.panels().iter().map(|p| p.id()).collect();
    Some(format!(
        "native plugins: tools {}; providers {}; hooks {}; panels {}",
        list(&tools),
        list(&providers),
        list(&hooks),
        list(&panels)
    ))
}

fn list<S: AsRef<str>>(items: &[S]) -> String {
    match items.is_empty() {
        true => "none".into(),
        false => (items.iter().map(AsRef::as_ref).collect::<Vec<_>>()).join(", "),
    }
}

/// The plugins' providers whose id is neither built in nor used by a
/// `providers/*.json` file.
pub(super) fn providers(
    files: &[ProviderFile],
    plugins: &Ext,
) -> Vec<(Arc<dyn Provider>, ProviderFile)> {
    (plugins.providers().iter())
        .filter(|(_, file)| file.is_custom() && !files.iter().any(|f| f.id == file.id))
        .cloned()
        .collect()
}
