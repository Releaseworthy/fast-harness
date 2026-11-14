//! A workspace switch loads that workspace's settings (D67): a trusted
//! project's provider files show in the model picker of that workspace only
//! (found in the real window, 2026-11-13: the second workspace listed the
//! first's provider because the switch never reloaded the project layer).

use std::path::Path;
use std::rc::Rc;

use fast_harness_core::app::Runtime;
use fast_harness_core::config::trust;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::ComponentHandle;

use crate::app::{install, with_app, App};
use crate::workspaces;

struct TestPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

const PROVIDER: &str = r#"{"id":"projfake","api":"openai-chat","baseUrl":"http://127.0.0.1:1/v1","keyEnv":"FH_TEST_NO_KEY","models":[{"id":"projmodel","contextWindow":1000,"maxOutput":100}]}"#;

fn lists_project_model() -> bool {
    with_app(|app, _| app.models.iter().any(|id| id.contains("projmodel"))).unwrap_or(false)
}

fn select(path: &Path) {
    let path = path.to_string_lossy().to_string();
    with_app(|app, ui| workspaces::select(app, ui, &path));
}

#[test]
fn the_model_picker_follows_the_workspace_project_providers() {
    let root = std::env::temp_dir().join(format!("fh-ws-switch-{}", std::process::id()));
    let (home, ws1, ws2) = (root.join("home"), root.join("ws1"), root.join("ws2"));
    let providers = ws1.join(".fast-harness").join("providers");
    std::fs::create_dir_all(&providers).unwrap();
    std::fs::create_dir_all(&ws2).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(providers.join("proj.json"), PROVIDER).unwrap();
    trust::trust(&home, &ws1).unwrap();
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let _ = slint::platform::set_platform(Box::new(TestPlatform(window)));
    let ui = crate::chat_list::new_window().expect("window");
    let mut rt = Runtime::load_at(home.clone()).unwrap();
    rt.use_project(&ws1).unwrap();
    install(App::new(&ui, rt, Some(ws1.clone())));
    with_app(crate::settings::fill);
    assert!(lists_project_model(), "ws1 lists its trusted provider");
    select(&ws2);
    assert!(!lists_project_model(), "ws2 must not list ws1's provider");
    select(&ws1);
    assert!(lists_project_model(), "back in ws1 it is listed again");
    let _ = ui.hide();
    let _ = std::fs::remove_dir_all(&root);
}
