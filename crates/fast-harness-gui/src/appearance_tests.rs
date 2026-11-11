//! Settings > Appearance (PLAN.md D33): the theme pick and the greyscale
//! level are saved, Reload finds a new user theme, a failing user theme
//! lists its pairs, and the slider and number box stay in sync.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use fast_harness_core::config::Config;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Key, Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, PhysicalSize, SharedString};

use crate::{appearance, theme};

/// A fresh temp home; removed on drop.
struct Temp(PathBuf);

impl Temp {
    fn new(name: &str) -> Temp {
        let root = std::env::temp_dir().join(format!(
            "fh-gui-appearance-{name}-{}-{}",
            std::process::id(),
            fast_harness_core::trace::now_ms()
        ));
        std::fs::create_dir_all(root.join("themes")).unwrap();
        Temp(root)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn saved(home: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(home.join("config.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn a_theme_pick_is_saved() {
    let temp = Temp::new("pick");
    let mut config = Config::default();
    let (picked, result) = theme::choose(&mut config, &temp.0, 3).unwrap();
    result.unwrap();
    assert_eq!(picked.id, "synthwave");
    assert_eq!(config.theme, "synthwave");
    assert_eq!(saved(&temp.0)["theme"], "synthwave");
    assert!(theme::choose(&mut config, &temp.0, 99).is_none());
    assert!(theme::choose(&mut config, &temp.0, -1).is_none());
    assert_eq!(config.theme, "synthwave");
}

#[test]
fn the_greyscale_level_clamps_and_is_saved() {
    let temp = Temp::new("grey");
    let mut config = Config::default();
    for (asked, kept) in [(42, 42), (150, 100), (-5, 0), (100, 100)] {
        theme::save_level(&mut config, &temp.0, asked).unwrap();
        assert_eq!(config.greyscale, kept);
        assert_eq!(saved(&temp.0)["greyscale"], kept);
    }
    // Both settings live side by side in config.json.
    theme::choose(&mut config, &temp.0, 6).unwrap().1.unwrap();
    assert_eq!(saved(&temp.0)["greyscale"], 100);
    assert_eq!(saved(&temp.0)["theme"], "sakura");
}

#[test]
fn reload_picks_up_a_new_user_theme() {
    let temp = Temp::new("reload");
    theme::load(&temp.0);
    assert_eq!(appearance::cards("dark", 0).len(), 8);
    let mine =
        r#"{"version":1,"id":"mine","name":"Mine","kind":"dark","extends":"aurora","colors":{}}"#;
    std::fs::write(temp.0.join("themes").join("mine.json"), mine).unwrap();
    theme::load(&temp.0);
    let cards = appearance::cards("mine", 0);
    assert_eq!(cards.len(), 9);
    let last = &cards[8];
    assert_eq!(
        (last.name.as_str(), last.custom, last.current),
        ("Mine", true, true)
    );
    assert!(last.dark);
    assert_eq!(last.failures, "");
    assert!(cards[..8].iter().all(|c| !c.custom && !c.current));
    theme::load(&temp.0.join("missing"));
}

#[test]
fn a_failing_user_theme_lists_its_pairs() {
    let temp = Temp::new("faint");
    let faint = r##"{"version":1,"id":"faint","name":"Faint","kind":"light","extends":"sakura","colors":{"dim":"#c9b8c0"}}"##;
    std::fs::write(temp.0.join("themes").join("faint.json"), faint).unwrap();
    std::fs::write(temp.0.join("themes").join("broken.json"), "{").unwrap();
    theme::load(&temp.0);
    let cards = appearance::cards("dark", 0);
    let card = cards.iter().find(|c| c.id == "faint").expect("still loads");
    assert!(card.failures.contains("dim on "), "{}", card.failures);
    assert!(card.failures.contains("(needs 4.5:1)"), "{}", card.failures);
    let others = cards.iter().filter(|c| c.id != "faint");
    assert!(others.clone().all(|c| c.failures.is_empty()));
    // The notes name the broken file; the contrast pairs stay on the card.
    let notes = appearance::notes();
    assert!(notes.contains("broken.json"), "{notes}");
    assert!(!notes.contains("Faint"), "{notes}");
    theme::load(&temp.0.join("missing"));
}

slint::slint! {
    import { Slider, NumberBox } from "ui/controls.slint";
    export component ControlsHarness inherits Window {
        width: 400px;
        height: 40px;
        in-out property <int> value: 40;
        out property <string> box-text: number.text;
        Slider { x: 0; y: 4px; width: 300px; height: 32px; value: root.value; changed(v) => { root.value = v; } }
        number := NumberBox { x: 310px; y: 4px; value: root.value; changed(v) => { root.value = v; } }
    }
}

struct TestPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn click(window: &MinimalSoftwareWindow, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    let button = PointerEventButton::Left;
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
}

fn key(window: &MinimalSoftwareWindow, text: impl Into<SharedString>) {
    let text = text.into();
    window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
}

fn harness() -> (Rc<MinimalSoftwareWindow>, ControlsHarness) {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).expect("platform");
    window.set_size(PhysicalSize::new(400, 40));
    let ui = ControlsHarness::new().expect("harness");
    ui.show().expect("show");
    (window, ui)
}

fn shown(ui: &ControlsHarness) -> (i32, String) {
    (ui.get_value(), ui.get_box_text().to_string())
}

#[test]
fn the_slider_steps_by_keyboard_and_the_box_follows() {
    let (window, ui) = harness();
    // A click on the track: (150 - 10) / 280 of the way.
    click(&window, 150.0, 20.0);
    assert_eq!(shown(&ui), (50, "50".into()));
    let steps = [
        (Key::RightArrow, 51),
        (Key::PageUp, 61),
        (Key::PageDown, 51),
        (Key::LeftArrow, 50),
        (Key::End, 100),
        (Key::UpArrow, 100),
        (Key::Home, 0),
        (Key::DownArrow, 0),
        (Key::PageUp, 10),
    ];
    for (i, (k, want)) in steps.into_iter().enumerate() {
        key(&window, k);
        assert_eq!(shown(&ui), (want, want.to_string()), "step {i}");
    }
}

#[test]
fn the_number_box_clamps_and_the_slider_follows() {
    let (window, ui) = harness();
    // The box: digits only, out of range clamps, Enter applies.
    click(&window, 350.0, 20.0);
    key(&window, Key::End);
    key(&window, Key::Backspace);
    key(&window, Key::Backspace);
    for typed in ["1", "5", "x", "-", "0"] {
        key(&window, typed);
    }
    key(&window, Key::Return);
    assert_eq!(shown(&ui), (100, "100".into()));
    for _ in 0..3 {
        key(&window, Key::Backspace);
    }
    key(&window, "7");
    key(&window, Key::Return);
    assert_eq!(shown(&ui), (7, "7".into()));
    key(&window, Key::UpArrow);
    assert_eq!(shown(&ui), (8, "8".into()));
    key(&window, Key::DownArrow);
    key(&window, Key::DownArrow);
    assert_eq!(shown(&ui), (6, "6".into()));
    // An emptied box restores the value when it loses focus.
    key(&window, Key::Backspace);
    click(&window, 10.0, 20.0);
    assert_eq!(shown(&ui), (0, "0".into()), "the slider's left end");
    // The slider now has focus and its keys move the box too.
    key(&window, Key::PageUp);
    assert_eq!(shown(&ui), (10, "10".into()));
}
