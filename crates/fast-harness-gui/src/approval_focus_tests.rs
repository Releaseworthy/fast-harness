//! The approval prompt's focus chain (PLAN.md D64): Decline is the only Tab
//! stop, so no sequence of Tab, Shift+Tab, Enter and Space can approve.

use std::rc::Rc;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Key, Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, PhysicalSize, SharedString};

slint::slint! {
    import { ApprovalPrompt } from "ui/approval.slint";
    export component ApprovalHarness inherits Window {
        width: 600px;
        height: 300px;
        in-out property <string> answers;
        prompt := ApprovalPrompt {
            x: 0; y: 0; width: 600px; height: 200px;
            answer(a) => { root.answers += a + ";"; }
        }
        Rectangle {
            x: 0; y: 220px; width: 600px; height: 40px;
            TextInput { width: 100%; height: 100%; }
        }
    }
}

struct TestPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn key(window: &MinimalSoftwareWindow, text: impl Into<SharedString>) {
    let text = text.into();
    window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
}

fn answers_after(step: Key) -> String {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).expect("platform");
    window.set_size(PhysicalSize::new(600, 300));
    let ui = ApprovalHarness::new().expect("harness");
    ui.show().expect("show");
    // Focus the stand-in composer, as in the window.
    let position = LogicalPosition::new(50.0, 240.0);
    let button = PointerEventButton::Left;
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
    for _ in 0..8 {
        key(&window, step);
        key(&window, " ");
        key(&window, Key::Return);
    }
    ui.get_answers().to_string()
}

// One test per key: the Slint platform is set once per test thread.
#[test]
fn shift_tab_reaches_decline_but_never_an_allow_button() {
    let answers = answers_after(Key::Backtab);
    assert!(!answers.contains("approve"), "{answers}");
    assert!(answers.contains("decline"), "{answers}");
}

#[test]
fn tab_reaches_decline_but_never_an_allow_button() {
    let answers = answers_after(Key::Tab);
    assert!(!answers.contains("approve"), "{answers}");
    assert!(answers.contains("decline"), "{answers}");
}
