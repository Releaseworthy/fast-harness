//! Attaching in the window (PLAN.md D70 "Views"): a path becomes a chip
//! with its label, the x removes it, a refusal says why, `/image` typed in
//! the composer is never sent, and thumbnails are small and cached.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use fast_harness_core::agent::Conversation;
use fast_harness_core::app::Runtime;
use fast_harness_core::media;
use fast_harness_core::view::{opened, Item};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::Model;

use super::{attach, chips, remove};
use crate::app::{install, with_app};
use crate::{open_load, thumbs, turn, MainWindow};

struct TestPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fh-gui-img-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn setup(dir: &Path) -> (MainWindow, PathBuf) {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let _ = slint::platform::set_platform(Box::new(TestPlatform(window)));
    let ui = MainWindow::new().expect("window");
    let (home, cwd) = (dir.join("home"), dir.join("work"));
    std::fs::create_dir_all(&cwd).unwrap();
    std::fs::write(cwd.join("dot.png"), media::solid_png(3, 2, [200, 10, 10])).unwrap();
    std::fs::write(cwd.join("notes.txt"), "not an image").unwrap();
    let id = Conversation::create(&home, &cwd, None).unwrap().id;
    let rt = Runtime::load_at(home.clone()).unwrap();
    install(App::new(&ui, rt, Some(cwd.clone())));
    super::connect(&ui);
    with_app(|app, ui| app.open_session(ui, &id));
    let result = opened::load(&home, &cwd, &id);
    with_app(|app, ui| open_load::arrived(app, ui, 1, &cwd, result));
    (ui, cwd)
}

use crate::app::App;

fn labels(ui: &MainWindow) -> Vec<String> {
    let chips = ui.get_attached();
    (0..chips.row_count())
        .filter_map(|i| chips.row_data(i))
        .map(|chip| chip.label.to_string())
        .collect()
}

#[test]
fn a_path_becomes_a_chip_the_x_removes_it_and_a_refusal_says_why() {
    let dir = temp("chips");
    let (ui, cwd) = setup(&dir);
    with_app(|app, ui| attach(app, ui, &cwd.join("dot.png")));
    with_app(|app, ui| attach(app, ui, &cwd.join("dot.png")));
    let shown = labels(&ui);
    assert_eq!(shown.len(), 2, "{}", ui.get_error_text());
    assert!(shown[0].starts_with("png 3x2, "), "{shown:?}");
    assert!(ui.get_error_text().is_empty(), "no banner for a success");
    with_app(|app, ui| remove(app, ui, 0));
    assert_eq!(labels(&ui).len(), 1);
    with_app(|app, ui| attach(app, ui, &cwd.join("notes.txt")));
    assert!(ui.get_error_text().starts_with("Image not attached: "));
    assert_eq!(labels(&ui).len(), 1);
    // Typed in the composer: attached or cleared, never a message.
    let sent = with_app(|app, ui| turn::send(app, ui, "/image clear")).unwrap();
    assert!(sent && labels(&ui).is_empty());
    with_app(|app, ui| turn::send(app, ui, "/image \"dot.png\"")).unwrap();
    assert_eq!(labels(&ui).len(), 1);
    let users = with_app(|app, _| {
        let items = &app.runs.shown.view.items;
        items.iter().filter(|i| matches!(i, Item::User { .. })).count()
    });
    assert_eq!(users, Some(0), "/image never reached the model");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_thumbnail_is_at_most_128_pixels_and_cached_by_its_path() {
    let dir = temp("thumbs");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("wide.png");
    std::fs::write(&path, media::solid_png(300, 200, [10, 120, 200])).unwrap();
    let pixels = thumbs::decode(&path).expect("decoded");
    assert_eq!((pixels.width(), pixels.height()), (128, 85));
    assert_eq!(pixels.as_slice()[0].b, 200, "the box filter keeps a solid colour");
    assert_eq!(thumbs::get(&path).size().width, 0, "not decoded yet");
    thumbs::insert(vec![(path.clone(), Some(pixels))]);
    assert_eq!(thumbs::get(&path).size().width, 128);
    let image = fast_harness_core::types::ImageRef::new(
        path.to_string_lossy().into_owned(),
        "image/png",
        10,
        (300, 200),
    );
    let chip = chips(&[image]).row_data(0).unwrap();
    assert_eq!(chip.thumb.size().width, 128);
    assert!(chip.label.starts_with("png 300x200"));
    let _ = std::fs::remove_dir_all(&dir);
}
