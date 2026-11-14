use super::*;

#[test]
fn percent_and_factor_clamp_to_50_and_400() {
    assert_eq!(factor(150), 1.5);
    assert_eq!(factor(10), 0.5);
    assert_eq!(factor(900), 4.0);
    assert_eq!(percent(1.25), 125);
    assert_eq!(percent(8.0), 400);
    assert_eq!(percent(f32::NAN), 100);
}

struct TestPlatform(std::rc::Rc<slint::platform::software_renderer::MinimalSoftwareWindow>);

impl slint::platform::Platform for TestPlatform {
    fn create_window_adapter(
        &self,
    ) -> Result<std::rc::Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

/// Settings shows the scale the window starts at (2026-11-15: the owner's
/// `uiScale` 1.75 drew the window at 175 % and the slider said 100 %):
/// Settings is filled before the window applies its scale (here: never).
#[test]
fn the_slider_shows_the_saved_scale_before_the_window_applies_it() {
    use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let dir = std::env::temp_dir().join(format!("fh-scale-{}-{nanos}", std::process::id()));
    let (home, cwd) = (dir.join("home"), dir.join("work"));
    std::fs::create_dir_all(home.join("providers")).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    let config = r#"{"model": "local/m", "retries": 0, "uiScale": 1.75}"#;
    std::fs::write(home.join("config.json"), config).unwrap();
    let provider = r#"{"id": "local", "api": "anthropic-messages", "baseUrl": "http://127.0.0.1:9", "models": [{"id": "m"}]}"#;
    std::fs::write(home.join("providers").join("local.json"), provider).unwrap();
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let _ = slint::platform::set_platform(Box::new(TestPlatform(window)));
    let ui = crate::chat_list::new_window().expect("window");
    let rt = fast_harness_core::app::Runtime::load_at(home.clone()).unwrap();
    let app = App::new(&ui, rt, Some(cwd));
    assert_eq!(ui.window().scale_factor(), 1.0, "the window has not applied it");
    show(&app, &ui);
    assert_eq!(ui.global::<LimitsUi>().get_scale(), 175);
    let saved = std::fs::read_to_string(home.join("config.json")).unwrap();
    assert!(saved.contains("1.75"), "showing saves nothing: {saved}");
    drop((app, ui));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The slider's preview scales the open window at once and keeps its
/// physical size: the logical width follows the factor (D80).
#[test]
fn a_scale_change_applies_live_and_keeps_the_window_size() {
    use crate::turn_nav_view_tests::{frames, session, show, window, HEIGHT, WIDTH};
    let (window, ui) = window(WIDTH, HEIGHT);
    connect(&ui);
    show(&ui, &session(3));
    frames(&window, 50);
    let logical = |ui: &MainWindow| ui.window().size().to_logical(ui.window().scale_factor());
    assert_eq!(logical(&ui).width, WIDTH as f32);
    let nav = ui.global::<crate::TurnsUi>();
    let at_100 = nav.get_column_right();
    let g = ui.global::<LimitsUi>();
    for (percent, factor) in [(150, 1.5_f32), (50, 0.5), (400, 4.0), (100, 1.0)] {
        g.invoke_scale_preview(percent);
        frames(&window, 32);
        assert_eq!(ui.window().scale_factor(), factor, "{percent} %");
        assert_eq!(window.size().width, WIDTH, "the window keeps its pixels");
        let width = logical(&ui).width;
        assert!((width - WIDTH as f32 / factor).abs() < 1.0, "{percent} %: {width}");
        // The layout follows: the chat column ends further left when larger.
        let right = nav.get_column_right();
        assert_eq!(right < at_100, factor > 1.0, "{percent} %: column ends at {right}");
    }
}