//! The tab bar and the dialogs in the whole window (PLAN.md D76): no
//! Subagents or Jobs tab without any, a tab opens its popover under it, a
//! subagent row opens the child read-only and Back returns; Settings and
//! About open as dialogs and Escape or the backdrop closes them.

use std::cell::RefCell;
use std::rc::Rc;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Key, Platform, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, ModelRc, PhysicalSize, Rgb8Pixel, SharedString, VecModel};

use crate::{DialogUi, JobsUi, MainWindow, SubagentRow, SubagentsUi};

const W: u32 = 1000;
const H: u32 = 700;
// The middle of the tab bar (the title bar is 40px, the bar 44px).
const TAB_Y: f32 = 62.0;

struct TestPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

pub fn render(window: &MinimalSoftwareWindow) -> Vec<Rgb8Pixel> {
    let mut buffer = vec![Rgb8Pixel { r: 0, g: 0, b: 0 }; (W * H) as usize];
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut buffer, W as usize);
        });
    }
    buffer
}

pub fn click(window: &MinimalSoftwareWindow, x: f32, y: f32) {
    use slint::platform::PointerEventButton;
    let position = slint::LogicalPosition::new(x, y);
    let button = PointerEventButton::Left;
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
    window.dispatch_event(WindowEvent::PointerExited);
}

pub fn key(window: &MinimalSoftwareWindow, text: SharedString) {
    window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
}

/// Clicks along a line until `done`; the point that did it, if any.
pub fn sweep(
    window: &MinimalSoftwareWindow,
    points: impl Iterator<Item = (f32, f32)>,
    done: impl Fn() -> bool,
) -> Option<(f32, f32)> {
    for (x, y) in points {
        click(window, x, y);
        if done() {
            return Some((x, y));
        }
    }
    None
}

fn along_tabs() -> impl Iterator<Item = (f32, f32)> {
    (10..80).map(|i| (i as f32 * 8.0, TAB_Y))
}

pub fn subagent(id: &str, running: bool) -> SubagentRow {
    let (mark, word) = if running {
        ("running", "running")
    } else {
        ("finished", "finished")
    };
    SubagentRow {
        id: id.into(),
        label: format!("Review {id}").into(),
        mark: mark.into(),
        word: word.into(),
        progress: "step 12 · grep".into(),
        when: if running { "" } else { "2m 05s" }.into(),
    }
}

fn window() -> (Rc<MinimalSoftwareWindow>, MainWindow) {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).expect("platform");
    window.set_size(PhysicalSize::new(W, H));
    let ui = MainWindow::new().expect("window");
    ui.show().expect("show");
    ui.set_sidebar_open(false);
    ui.set_page("chat".into());
    (window, ui)
}

fn differ(a: &[Rgb8Pixel], b: &[Rgb8Pixel], rows: std::ops::Range<u32>) -> usize {
    let range = (rows.start * W) as usize..(rows.end * W) as usize;
    range.filter(|&i| a[i] != b[i]).count()
}

/// Wires open-child and back as the app does for the view: the child's
/// session shows read-only, Back shows the parent again.
fn wire_children(ui: &MainWindow) -> Rc<RefCell<String>> {
    let opened = Rc::new(RefCell::new(String::new()));
    let subs = ui.global::<SubagentsUi>();
    let (sink, weak) = (opened.clone(), ui.as_weak());
    subs.on_open_child(move |id| {
        *sink.borrow_mut() = id.to_string();
        let ui = weak.upgrade().expect("window");
        let subs = ui.global::<SubagentsUi>();
        subs.set_open(false);
        subs.set_back_label("parent session".into());
        subs.set_read_only(true);
    });
    let weak = ui.as_weak();
    subs.on_back(move || {
        let ui = weak.upgrade().expect("window");
        ui.global::<SubagentsUi>().set_read_only(false);
    });
    opened
}

#[test]
fn the_subagents_tab_opens_its_popover_and_a_row_opens_the_child() {
    let (window, ui) = window();
    let subs = ui.global::<SubagentsUi>();
    let opened = wire_children(&ui);
    let empty = render(&window);
    let none = sweep(&window, along_tabs(), || subs.get_open());
    assert_eq!(none, None, "no Subagents tab without subagents");
    let rows = vec![
        subagent("a", true),
        subagent("b", true),
        subagent("c", false),
    ];
    subs.set_rows(ModelRc::new(VecModel::from(rows)));
    subs.set_running(2);
    let tabs = render(&window);
    assert!(differ(&empty, &tabs, 40..84) > 0, "the tab draws");
    let (x, _) = sweep(&window, along_tabs(), || subs.get_open()).expect("the tab opens");
    let open = render(&window);
    assert!(
        differ(&tabs, &open, 90..250) > 0,
        "the popover draws under the tab"
    );
    let down = (90..300).step_by(6).map(|y| (x + 60.0, y as f32));
    sweep(&window, down, || subs.get_read_only()).expect("a row opens the child");
    assert_eq!((opened.borrow().as_str(), subs.get_open()), ("a", false));
    let child = render(&window);
    let back =
        (0..120).flat_map(|i| [620.0, 632.0, 644.0].map(|y| (W as f32 - 30.0 - i as f32 * 4.0, y)));
    sweep(&window, back, || !subs.get_read_only()).expect("Back returns");
    assert!(
        differ(&child, &render(&window), 600..672) > 0,
        "the composer is back"
    );
    subs.set_rows(ModelRc::new(VecModel::from(Vec::<SubagentRow>::new())));
    subs.set_running(0);
    assert_eq!(
        differ(&empty, &render(&window), 40..84),
        0,
        "the tab goes with them"
    );
}

#[test]
fn the_jobs_tab_shows_only_with_jobs_and_opens_its_popover() {
    let (window, ui) = window();
    let jobs = ui.global::<JobsUi>();
    let empty = render(&window);
    assert_eq!(
        sweep(&window, along_tabs(), || jobs.get_open()),
        None,
        "no Jobs tab without jobs"
    );
    jobs.set_running(ModelRc::new(VecModel::from(crate::shots_tabbar::jobs())));
    let tabs = render(&window);
    assert!(differ(&empty, &tabs, 40..84) > 0, "the tab draws");
    sweep(&window, along_tabs(), || jobs.get_open()).expect("the tab opens");
    let open = render(&window);
    assert!(
        differ(&tabs, &open, 90..200) > 0,
        "the popover draws under the tab"
    );
    key(&window, Key::Escape.into());
    assert!(!jobs.get_open(), "Escape closes it");
    assert_eq!(
        differ(&tabs, &render(&window), 90..400),
        0,
        "the popover is gone"
    );
}

#[test]
fn settings_and_about_open_as_dialogs_and_escape_closes_them() {
    let (window, ui) = window();
    let weak = ui.as_weak();
    ui.on_open_page(move |page| {
        let ui = weak.upgrade().expect("window");
        if !crate::dialogs::open(&ui, &page) {
            ui.set_page(page);
        }
    });
    let dialog = ui.global::<DialogUi>();
    let chat = render(&window);
    let bar = |name: &'static str| {
        let points = (2..60).map(|i| (i as f32 * 5.0, 20.0));
        sweep(&window, points, || dialog.get_open() == name)
    };
    bar("settings").expect("the Settings button opens the dialog");
    assert_eq!(ui.get_page().as_str(), "chat", "no Settings page or tab");
    assert!(
        differ(&chat, &render(&window), 100..600) > 0,
        "the dialog draws"
    );
    key(&window, Key::Escape.into());
    assert_eq!(dialog.get_open().as_str(), "", "Escape closes Settings");
    bar("about").expect("the About button opens the dialog");
    key(&window, Key::Escape.into());
    assert_eq!(dialog.get_open().as_str(), "", "Escape closes About");
    crate::dialogs::open(&ui, "about");
    render(&window);
    click(&window, 8.0, H as f32 - 40.0);
    assert_eq!(dialog.get_open().as_str(), "", "a backdrop click closes it");
    // The transcript as before; the composer took the focus back.
    assert_eq!(
        differ(&chat, &render(&window), 84..480),
        0,
        "the chat shows again"
    );
}
