//! The composer's action button (D60): while a session opens, Send waits,
//! but Stop still stops the shown session's running turn.

use std::rc::Rc;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, PhysicalSize};

slint::slint! {
    import { Composer } from "ui/composer.slint";
    export component ComposerHarness inherits Window {
        width: 600px;
        height: 200px;
        in property <bool> busy;
        in property <bool> loading;
        in-out property <int> submits;
        in-out property <int> stops;
        Composer {
            x: 0; y: 0; width: 600px; height: 200px;
            busy: root.busy;
            loading: root.loading;
            submit => { root.submits += 1; }
            stop => { root.stops += 1; }
        }
    }
}

struct TestPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

/// A click on the action button: the last item of the toolbar, bottom
/// right inside the 12 px padding.
fn click_action(window: &MinimalSoftwareWindow) {
    let position = LogicalPosition::new(600.0 - 12.0 - 40.0, 200.0 - 12.0 - 12.0);
    let button = PointerEventButton::Left;
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
}

#[test]
fn stop_stays_enabled_while_a_session_opens_and_send_waits() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).expect("platform");
    window.set_size(PhysicalSize::new(600, 200));
    let ui = ComposerHarness::new().expect("harness");
    ui.show().expect("show");
    click_action(&window);
    assert_eq!(ui.get_submits(), 1, "an idle, loaded composer sends");
    ui.set_loading(true);
    click_action(&window);
    assert_eq!(ui.get_submits(), 1, "Send waits while a session opens");
    ui.set_busy(true);
    click_action(&window);
    assert_eq!(ui.get_stops(), 1, "Stop works while another session opens");
    ui.set_loading(false);
    click_action(&window);
    assert_eq!(ui.get_stops(), 2);
    assert_eq!(ui.get_submits(), 1);
}
