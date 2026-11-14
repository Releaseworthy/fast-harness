//! The sidebar's session rows in a real (software rendered) window
//! (2026-11-15, the owner: sessions sat too far right under their
//! workspace, with a vertical guide line): a session's text starts a short
//! step right of the workspace's chevron and no guide line remains.
//! `FH_SIDE_SHOTS=<dir>` also saves the window as `sidebar.png` there.

use std::path::Path;
use std::rc::Rc;

use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::{Model, Rgb8Pixel, VecModel};

use crate::turn_nav_view_tests::{frames, window, HEIGHT, WIDTH};
use crate::workspaces::workspace_row;
use crate::SideRow;

/// The sidebar's padding, the workspace row's padding and the guide line's
/// old column (under the folder icon).
const PAD: usize = 12;
const ROW_PAD: usize = 8;
const OLD_GUIDE: usize = PAD + ROW_PAD + 12 + 8 + 9;

fn session(id: &str, title: &str, run: &str) -> SideRow {
    SideRow {
        kind: "session".into(),
        id: id.into(),
        workspace: "F:\\work\\fast-harness".into(),
        title: title.into(),
        detail: "2 min ago".into(),
        current: false,
        expanded: false,
        run: run.into(),
    }
}

fn render(window: &MinimalSoftwareWindow) -> Vec<Rgb8Pixel> {
    let size = window.size();
    let mut pixels = vec![Rgb8Pixel { r: 0, g: 0, b: 0 }; (size.width * size.height) as usize];
    window.request_redraw();
    window.draw_if_needed(|r| {
        r.render(&mut pixels, size.width as usize);
    });
    pixels
}

fn save(pixels: &[Rgb8Pixel], name: &str) {
    let Some(dir) = std::env::var_os("FH_SIDE_SHOTS") else {
        return;
    };
    let rgb: Vec<u8> = pixels.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
    let png = crate::shots_png::encode(WIDTH, HEIGHT, &rgb);
    std::fs::create_dir_all(&dir).expect("shots dir");
    std::fs::write(Path::new(&dir).join(format!("{name}.png")), png).expect("write png");
}

/// The bands (first y, last y, leftmost marked x) of rows that draw in the
/// sidebar, above its bottom separator.
fn bands(pixels: &[Rgb8Pixel]) -> Vec<(usize, usize, usize)> {
    let at = |x: usize, y: usize| pixels[y * WIDTH as usize + x];
    let first = |y: usize| (PAD..240).find(|&x| at(x, y) != at(2, y));
    let mut bands: Vec<(usize, usize, usize)> = Vec::new();
    for y in 0..HEIGHT as usize - 80 {
        match (first(y), bands.last_mut()) {
            (Some(x), Some(b)) if b.1 + 1 == y => (b.1, b.2) = (y, b.2.min(x)),
            (Some(x), _) => bands.push((y, y, x)),
            (None, _) => {}
        }
    }
    bands
}

#[test]
fn sessions_sit_a_short_step_in_without_a_guide_line() {
    let (window, ui) = window(WIDTH, HEIGHT);
    ui.set_sidebar_open(true);
    let rows = vec![
        workspace_row(Path::new("F:\\work\\fast-harness"), false, true),
        session("1", "Theme picker for the GUI", ""),
        session("2", "Fix the retry backoff", "running"),
    ];
    ui.set_side_rows(Rc::new(VecModel::from(rows)).into());
    frames(&window, 100);
    let pixels = render(&window);
    save(&pixels, "sidebar");
    let bands = bands(&pixels);
    // Two lines per session: its title and its time with the run mark.
    let sessions = &bands[bands.len() - 4..];
    for band in sessions {
        assert!(
            (PAD + ROW_PAD + 12..=PAD + ROW_PAD + 20).contains(&band.2),
            "session text at x {}, bands {bands:?}",
            band.2
        );
    }
    // Text crosses the old column; a line ran down both rows unbroken.
    let at = |x: usize, y: usize| pixels[y * WIDTH as usize + x];
    let (mut run, mut longest) = (0, 0);
    for y in sessions[0].0..=sessions[3].1 {
        run = if at(OLD_GUIDE, y) != at(2, y) {
            run + 1
        } else {
            0
        };
        longest = longest.max(run);
    }
    assert!(longest < 20, "a {longest} px guide line at x {OLD_GUIDE}");
    // The selected look, for the eye only (`sidebar-selected.png`).
    let mut current = session("1", "Theme picker for the GUI", "");
    current.current = true;
    ui.get_side_rows().set_row_data(1, current);
    frames(&window, 300);
    save(&render(&window), "sidebar-selected");
}
