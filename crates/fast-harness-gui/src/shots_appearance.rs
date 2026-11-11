//! Settings > Appearance in the screenshot test: the gallery in each
//! theme, the same at greyscale 50, and (for Dark) a user theme that fails
//! the contrast check, with its Custom badge and warning.

use std::path::Path;

use fast_harness_core::theme::by_id;

use crate::{appearance, shots_png, theme, MainWindow};

/// A user theme with a dim role too faint for its background.
const FAINT: &str = r##"{"version":1,"id":"faint","name":"Faint","kind":"light","extends":"sakura","colors":{"dim":"#c9b8c0"}}"##;

fn write(dir: &Path, name: &str, size: (u32, u32), rgb: &[u8]) {
    let png = shots_png::encode(size.0, size.1, rgb);
    std::fs::write(dir.join(format!("{name}.png")), png).expect("write png");
}

/// `<theme>-appearance.png`, `<theme>-appearance-grey50.png`,
/// `<theme>-chat-grey100.png` and, for Dark, `dark-appearance-custom.png`.
pub fn shot(ui: &MainWindow, dir: &Path, id: &str, size: (u32, u32), render: impl Fn() -> Vec<u8>) {
    let core = by_id(id);
    ui.set_page("settings".into());
    appearance::fill(ui, id, 0);
    write(dir, &format!("{id}-appearance"), size, &render());
    theme::apply(ui, core, 50);
    appearance::fill(ui, id, 50);
    write(dir, &format!("{id}-appearance-grey50"), size, &render());
    theme::apply(ui, core, 0);
    if id == "dark" {
        custom(ui, dir, size, &render);
    }
    appearance::fill(ui, id, 0);
    ui.set_page("chat".into());
    // The chat at greyscale 100: gradients, halos and marks in pure greys.
    theme::apply(ui, core, 100);
    write(dir, &format!("{id}-chat-grey100"), size, &render());
    theme::apply(ui, core, 0);
}

fn custom(ui: &MainWindow, dir: &Path, size: (u32, u32), render: &dyn Fn() -> Vec<u8>) {
    let home = std::env::temp_dir().join(format!("fh-shots-themes-{}", std::process::id()));
    std::fs::create_dir_all(home.join("themes")).expect("themes folder");
    std::fs::write(home.join("themes").join("faint.json"), FAINT).expect("theme file");
    std::fs::write(home.join("themes").join("broken.json"), "{").expect("theme file");
    theme::load(&home);
    appearance::fill(ui, "dark", 0);
    write(dir, "dark-appearance-custom", size, &render());
    theme::load(&home.join("missing"));
    let _ = std::fs::remove_dir_all(&home);
}
