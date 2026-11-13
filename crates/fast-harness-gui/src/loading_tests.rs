//! The "Loading the session…" line (D60) stays readable over the rows of the
//! session still shown: in the real window (2026-11-13) a tool card at the
//! top of the shown session drew over it, so only the composer said so.

use std::rc::Rc;

use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, ModelRc, PhysicalSize, VecModel};

slint::slint! {
    import { TranscriptView } from "ui/transcript.slint";
    import { MsgRow } from "ui/types.slint";
    export { MsgRow }
    export component LoadingHarness inherits Window {
        width: 600px;
        height: 300px;
        in property <[MsgRow]> rows;
        in property <bool> loading;
        TranscriptView {
            x: 0; y: 0; width: 600px; height: 300px;
            rows: root.rows;
            loading: root.loading;
        }
    }
}

struct TestPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

const W: usize = 600;
const H: usize = 300;
/// The rows checked: the top of the view, inside the loading line.
const STRIP: usize = 20;

fn render(window: &MinimalSoftwareWindow) -> Vec<PremultipliedRgbaColor> {
    let mut buffer = vec![PremultipliedRgbaColor::default(); W * H];
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut buffer, W);
    });
    buffer
}

fn rows(n: usize) -> ModelRc<MsgRow> {
    let text = "word ".repeat(120);
    let row = MsgRow {
        kind: "user".into(),
        text: text.as_str().into(),
        shown: text.as_str().into(),
        ..MsgRow::default()
    };
    ModelRc::new(VecModel::from(vec![row; n]))
}

#[test]
fn the_loading_line_shows_over_the_rows_still_shown() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).expect("platform");
    window.set_size(PhysicalSize::new(W as u32, H as u32));
    let ui = LoadingHarness::new().expect("harness");
    ui.show().expect("show");
    ui.set_loading(true);
    let empty = render(&window);
    ui.set_rows(rows(30));
    let full = render(&window);
    ui.set_loading(false);
    let shown = render(&window);
    let strip = |b: &[PremultipliedRgbaColor]| -> Vec<(u8, u8, u8, u8)> {
        (b[..W * STRIP].iter())
            .map(|p| (p.red, p.green, p.blue, p.alpha))
            .collect()
    };
    assert_ne!(
        strip(&full),
        strip(&shown),
        "the rows reach the top of the view"
    );
    assert_eq!(
        strip(&empty),
        strip(&full),
        "the loading line is drawn over the rows, not under them"
    );
}
