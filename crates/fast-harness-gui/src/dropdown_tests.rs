//! The dropdown's list opens with the picked row near its top and the rows
//! after it in view (found in the real window, 2026-11-13: the model picker
//! opened with the picked model on its bottom edge, hiding the next ones).

use std::rc::Rc;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, PhysicalSize, SharedString, VecModel};

slint::slint! {
    import { Dropdown } from "ui/widgets.slint";
    export component DropdownHarness inherits Window {
        width: 300px;
        height: 500px;
        in property <[string]> items;
        in-out property <int> current: 0;
        out property <int> first: dd.first-row;
        dd := Dropdown {
            x: 0; y: 0; width: 300px; height: 32px;
            model: root.items;
            current-index <=> root.current;
        }
    }
}

struct TestPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn open(window: &MinimalSoftwareWindow) {
    let position = LogicalPosition::new(100.0, 16.0);
    let button = PointerEventButton::Left;
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
}

/// The list shows 8 rows.
const SHOWN: i32 = 8;

/// Opens the list of `count` rows with row `current` picked; returns the
/// first row it shows, then closes it with Esc.
fn first_row_when_open(
    (window, ui): (&MinimalSoftwareWindow, &DropdownHarness),
    count: usize,
    current: i32,
) -> i32 {
    let items: Vec<SharedString> = (0..count).map(|i| format!("model {i}").into()).collect();
    ui.set_items(ModelRc::new(VecModel::from(items)));
    ui.set_current(current);
    open(window);
    let first = ui.get_first();
    let text = SharedString::from(slint::platform::Key::Escape);
    window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
    first
}

#[test]
fn the_list_opens_with_the_picked_row_and_the_next_ones_in_view() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).expect("platform");
    window.set_size(PhysicalSize::new(300, 500));
    let ui = DropdownHarness::new().expect("harness");
    ui.show().expect("show");
    let both = (&*window, &ui);
    let first = first_row_when_open(both, 20, 12);
    assert!(
        first <= 12 && 12 - first <= 1,
        "row 12 near the top, first {first}"
    );
    assert!(first + SHOWN > 14, "the rows after it show, first {first}");
    // Near the end the list fills its 8 rows instead of leaving a gap.
    assert_eq!(first_row_when_open(both, 20, 19), 20 - SHOWN);
    assert_eq!(first_row_when_open(both, 20, 0), 0);
    assert_eq!(
        first_row_when_open(both, 5, 3),
        0,
        "a short list never scrolls"
    );
}
