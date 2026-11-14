//! Settings > Subagent models (D82): a check box writes `subagent.models`
//! (only the models turned off), Save and Reset write `subagent.guide`.

use std::path::PathBuf;
use std::rc::Rc;

use fast_harness_core::app::Runtime;
use fast_harness_core::ui_thread;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::Model;

use super::*;
use crate::app::install;

struct TestPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

struct Env {
    ui: MainWindow,
    dir: PathBuf,
}

impl Drop for Env {
    fn drop(&mut self) {
        ui_thread::mark(false);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn setup() -> Env {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let dir = std::env::temp_dir().join(format!("fh-spawn-{}-{nanos}", std::process::id()));
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let _ = slint::platform::set_platform(Box::new(TestPlatform(window)));
    let ui = crate::chat_list::new_window().expect("window");
    let (home, cwd) = (dir.join("home"), dir.join("work"));
    std::fs::create_dir_all(home.join("providers")).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::write(home.join("config.json"), r#"{"model": "local/m", "retries": 0}"#).unwrap();
    let provider = r#"{"id": "local", "api": "anthropic-messages", "baseUrl": "http://127.0.0.1:9", "models": [{"id": "m"}]}"#;
    std::fs::write(home.join("providers").join("local.json"), provider).unwrap();
    let rt = Runtime::load_at(home).unwrap();
    install(App::new(&ui, rt, Some(cwd)));
    ui_thread::mark(true);
    connect(&ui);
    Env { ui, dir }
}

fn saved(env: &Env) -> serde_json::Value {
    let text = std::fs::read_to_string(env.dir.join("home").join("config.json")).unwrap();
    serde_json::from_str::<serde_json::Value>(&text).unwrap()["subagent"].clone()
}

#[test]
fn the_check_boxes_and_the_guide_write_both_keys() {
    let env = setup();
    with_app(|app, ui| show(app, ui));
    let g = env.ui.global::<SpawnUi>();
    let rows: Vec<SpawnModel> = g.get_models().iter().collect();
    assert!(rows.iter().all(|r| r.on), "every model starts enabled");
    assert!(rows.iter().any(|r| r.id == "local/m"), "custom models are listed");
    assert_eq!(g.get_guide().as_str(), DEFAULT_SUBAGENT_GUIDE);
    let opus = rows.iter().position(|r| r.id == "claude-opus-5-5").unwrap();
    g.invoke_toggled(i32::try_from(opus).unwrap(), false);
    assert_eq!(saved(&env)["models"], serde_json::json!({"claude-opus-5-5": false}));
    assert!(!g.get_models().row_data(opus).unwrap().on);
    let off = with_app(|app, _| app.rt.config.subagent.allows("claude-opus-5-5"));
    assert_eq!(off, Some(false), "applies from the next call");
    g.invoke_toggled(i32::try_from(opus).unwrap(), true);
    assert_eq!(saved(&env)["models"], serde_json::json!({}));
    g.set_guide("Use Haiku for reading.".into());
    assert!(g.get_dirty());
    g.invoke_save();
    assert_eq!(saved(&env)["guide"], "Use Haiku for reading.");
    assert!(!g.get_dirty());
    g.invoke_reset();
    assert_eq!(saved(&env)["guide"], DEFAULT_SUBAGENT_GUIDE);
    assert_eq!(g.get_guide().as_str(), DEFAULT_SUBAGENT_GUIDE);
}
