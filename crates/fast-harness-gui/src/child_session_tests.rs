//! A subagent's session is a full session (D79): Back sits in the tab row
//! before Chat only while a child shows.

use std::rc::Rc;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::{ComponentHandle, PhysicalSize};

use crate::tabbar_tests::{render, sweep};
use crate::turn_nav_view_tests::TestPlatform;
use crate::{MainWindow, SubagentsUi};

const W: u32 = 1000;
const H: u32 = 700;

fn window() -> (Rc<MinimalSoftwareWindow>, MainWindow) {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).expect("platform");
    window.set_size(PhysicalSize::new(W, H));
    let ui = MainWindow::new().expect("window");
    ui.show().expect("show");
    ui.set_sidebar_open(false);
    let weak = ui.as_weak();
    ui.on_open_page(move |page| weak.upgrade().expect("window").set_page(page));
    let weak = ui.as_weak();
    ui.global::<SubagentsUi>().on_back(move || {
        let ui = weak.upgrade().expect("window");
        ui.global::<SubagentsUi>().set_in_child(false);
    });
    (window, ui)
}

/// The first point along the tab row whose click does `done`.
fn first_hit(window: &MinimalSoftwareWindow, done: impl Fn() -> bool) -> Option<f32> {
    let points = (4..90).map(|i| (i as f32 * 6.0, 62.0));
    sweep(window, points, done).map(|(x, _)| x)
}

#[test]
fn back_sits_before_chat_only_in_a_child_session() {
    let (window, ui) = window();
    let subs = ui.global::<SubagentsUi>();
    ui.set_page("trace".into());
    render(&window);
    let chat = first_hit(&window, || ui.get_page() == "chat").expect("the Chat tab");
    assert!(!subs.get_in_child(), "no Back on a top-level session");
    ui.set_page("trace".into());
    subs.set_back_label("Parent session".into());
    subs.set_in_child(true);
    render(&window);
    let hit = first_hit(&window, || !subs.get_in_child() || ui.get_page() == "chat");
    assert!(hit.is_some_and(|x| x <= chat), "{hit:?} vs Chat at {chat}");
    assert_eq!(ui.get_page().as_str(), "trace", "Back came first, not Chat");
    assert!(!subs.get_in_child(), "Back returned to the parent");
    // Back gone: Chat is the first tab again.
    ui.set_page("trace".into());
    render(&window);
    assert_eq!(first_hit(&window, || ui.get_page() == "chat"), Some(chat));
}
