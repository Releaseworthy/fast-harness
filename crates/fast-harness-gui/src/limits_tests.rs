use super::*;

#[test]
fn percent_and_factor_clamp_to_50_and_400() {
    assert_eq!(factor(150), 1.5);
    assert_eq!(factor(10), 0.5);
    assert_eq!(factor(900), 4.0);
    assert_eq!(percent(1.25), 125);
    assert_eq!(percent(8.0), 400);
    assert_eq!(percent(f32::NAN), 100);
}

/// The slider's preview scales the open window at once and keeps its
/// physical size: the logical width follows the factor (D80).
#[test]
fn a_scale_change_applies_live_and_keeps_the_window_size() {
    use crate::turn_nav_view_tests::{frames, session, show, window, HEIGHT, WIDTH};
    let (window, ui) = window(WIDTH, HEIGHT);
    connect(&ui);
    show(&ui, &session(3));
    frames(&window, 50);
    let logical = |ui: &MainWindow| ui.window().size().to_logical(ui.window().scale_factor());
    assert_eq!(logical(&ui).width, WIDTH as f32);
    let nav = ui.global::<crate::TurnsUi>();
    let at_100 = nav.get_column_right();
    let g = ui.global::<LimitsUi>();
    for (percent, factor) in [(150, 1.5_f32), (50, 0.5), (400, 4.0), (100, 1.0)] {
        g.invoke_scale_preview(percent);
        frames(&window, 32);
        assert_eq!(ui.window().scale_factor(), factor, "{percent} %");
        assert_eq!(window.size().width, WIDTH, "the window keeps its pixels");
        let width = logical(&ui).width;
        assert!((width - WIDTH as f32 / factor).abs() < 1.0, "{percent} %: {width}");
        // The layout follows: the chat column ends further left when larger.
        let right = nav.get_column_right();
        assert_eq!(right < at_100, factor > 1.0, "{percent} %: column ends at {right}");
    }
}