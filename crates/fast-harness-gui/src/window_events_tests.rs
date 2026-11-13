use slint::winit_030::winit::dpi::PhysicalSize;
use slint::winit_030::winit::event::{Ime, WindowEvent};

use super::{kind, Kind, Rest};

#[test]
fn a_focused_idle_composer_rests_once_and_input_wakes_it() {
    let mut rest = Rest::default();
    assert!(rest.due(true), "an idle focused composer rests");
    assert!(!rest.due(true), "a resting window is not told twice");
    assert!(rest.input(), "input makes it active again");
    assert!(!rest.input(), "only once");
    assert!(rest.due(true));
}

#[test]
fn no_rest_without_composer_focus_or_during_a_composition() {
    let mut rest = Rest::default();
    assert!(!rest.due(false), "no focused composer, no blinking caret");
    rest.ime(&Ime::Preedit("ka".into(), Some((2, 2))));
    assert!(!rest.due(true), "an open composition keeps its preedit");
    rest.ime(&Ime::Commit("か".into()));
    assert!(rest.due(true), "committed: rests again");
}

#[test]
fn a_real_activation_change_clears_a_rest() {
    let mut rest = Rest::default();
    assert!(rest.due(true));
    rest.activation();
    assert!(!rest.input(), "Slint heard the real activation itself");
}

#[test]
fn a_resize_re_reads_the_window_state_and_input_wakes_the_caret() {
    let resized = WindowEvent::Resized(PhysicalSize::new(800, 600));
    assert_eq!(kind(&resized), Kind::Resize, "Win+Up, snap, restore");
    assert_eq!(kind(&WindowEvent::Focused(false)), Kind::Activation(false));
    assert_eq!(kind(&WindowEvent::Ime(Ime::Enabled)), Kind::Ime);
    let dropped = WindowEvent::DroppedFile("a.png".into());
    assert_eq!(kind(&dropped), Kind::Drop("a.png".into()));
    assert_eq!(kind(&WindowEvent::RedrawRequested), Kind::Other);
}
