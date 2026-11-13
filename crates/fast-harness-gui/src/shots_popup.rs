//! D81 in the whole window, for a visual check: `FAST_HARNESS_SHOTS_DIR=<dir>
//! cargo test -p fast-harness-gui popup_shots -- --ignored` writes, per
//! theme (dark, light), `popup-approval` (an approval over a long chat) and
//! `popup-ask` (three questions: a single choice with the recommended option
//! first, a multi choice, and "Other" with typed text), each with the end
//! of the chat right above the popup.

use std::path::PathBuf;

use fast_harness_core::events::{Question, QuestionOption, QuestionRequest};
use fast_harness_core::theme::builtins;
use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::{ModelRc, Rgb8Pixel, VecModel};

use crate::turn_nav_view_tests::{frames, session, show, window, WIDTH};

/// Tall enough for the three questions without the popup's own scrolling.
const HEIGHT: u32 = 1000;
use crate::{ask, shots_png, spacer, theme, ApprovalView};

fn render(window: &MinimalSoftwareWindow) -> Vec<u8> {
    let mut buffer = vec![Rgb8Pixel { r: 0, g: 0, b: 0 }; (WIDTH * HEIGHT) as usize];
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut buffer, WIDTH as usize);
        });
    }
    buffer.iter().flat_map(|p| [p.r, p.g, p.b]).collect()
}

fn out_dir() -> PathBuf {
    std::env::var_os("FAST_HARNESS_SHOTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("fast-harness-gui-shots"))
}

fn option(label: &str, description: &str, recommended: bool) -> QuestionOption {
    let mut o = QuestionOption::new(label, recommended);
    o.description = (!description.is_empty()).then(|| description.into());
    o
}

fn question(
    id: &str,
    header: &str,
    text: &str,
    multi: bool,
    options: Vec<QuestionOption>,
) -> Question {
    let mut q = Question::default();
    (q.id, q.header, q.question) = (id.into(), Some(header.into()), text.into());
    (q.multi_select, q.options) = (multi, options);
    q
}

fn request() -> QuestionRequest {
    let mut r = QuestionRequest::default();
    r.questions = vec![
        question(
            "db",
            "Database",
            "Which database should the service use?",
            false,
            vec![
                option(
                    "Postgres",
                    "Already runs in staging; best JSON support",
                    true,
                ),
                option("SQLite", "One file, no server", false),
            ],
        ),
        question(
            "os",
            "Targets",
            "Which platforms must the release build for?",
            true,
            vec![
                option("Windows", "", false),
                option("Linux", "", false),
                option("macOS", "", false),
            ],
        ),
        question(
            "name",
            "Name",
            "What should the binary be called?",
            false,
            vec![
                option("fh", "Short, like the crate", false),
                option("fast-harness", "", false),
            ],
        ),
    ];
    r
}

#[test]
#[ignore = "writes PNG screenshots for a visual check"]
fn popup_shots() {
    let (window, ui) = window(WIDTH, HEIGHT);
    let dir = out_dir();
    std::fs::create_dir_all(&dir).expect("shots directory");
    let model = show(&ui, &session(8));
    for name in ["Dark", "Light"] {
        let core = builtins().iter().find(|t| t.name == name).expect("theme");
        theme::apply(&ui, core, 0);
        let save = |shot: &str| {
            let png = shots_png::encode(WIDTH, HEIGHT, &render(&window));
            let file = format!("{}-{shot}.png", name.to_lowercase());
            std::fs::write(dir.join(file), png).expect("write png");
        };
        ui.set_approval(crate::shots_approval::view());
        spacer::show(&ui, &model);
        frames(&window, 400);
        save("popup-approval");
        ui.set_approval(ApprovalView::default());
        let questions = ModelRc::new(VecModel::from(ask::rows(&request())));
        ask::toggle(&questions, 0, 0);
        ask::toggle(&questions, 1, 0);
        ask::toggle(&questions, 1, 1);
        ask::other(&questions, 2, "fast-harness-gui");
        ui.set_questions(questions);
        ui.set_ask_id("q1".into());
        spacer::show(&ui, &model);
        frames(&window, 400);
        save("popup-ask");
        ui.set_ask_id("".into());
        spacer::show(&ui, &model);
        frames(&window, 200);
    }
}
