//! The Trace tab opens at its top (2026-11-14: the virtual list held the
//! end while no rows were placed yet, so the timeline opened at its end and
//! then kept a middle entry at the top as heights came in).

use slint::{ComponentHandle, ModelRc, VecModel};

use crate::real_child_tests::{pump, window};
use crate::TraceProbe;

#[test]
fn the_trace_timeline_opens_at_its_top() {
    let (window, ui) = window();
    let mut rows = crate::shots_trace::entries();
    for _ in 0..3 {
        rows.extend(crate::shots_trace::entries());
    }
    ui.set_trace_entries(ModelRc::new(VecModel::from(rows)));
    pump(&window, 100);
    ui.set_page("trace".into());
    pump(&window, 800);
    let probe = ui.global::<TraceProbe>();
    let (content, view) = (probe.get_content_height(), probe.get_visible_height());
    assert!(content > 2.0 * view, "too short to scroll: {content} px in {view} px");
    let y = probe.get_view_y();
    assert!(y.abs() < 0.5, "the trace opens scrolled to y {y}, not at its top");
}