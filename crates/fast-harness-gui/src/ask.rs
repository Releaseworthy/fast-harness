//! The questions of `ask_user_question` in the session popup (PLAN.md
//! D81): the oldest question request of the shown session becomes
//! `MainWindow.questions` (rebuilt only when its id changes, so choices
//! survive a refresh), the popup's toggles and the custom text edit that
//! model, and Submit or Skip reply through `Session::reply`.

use fast_harness_core::events::{QuestionAnswer, QuestionRequest};
use slint::{Model, ModelRc, SharedString, VecModel};

use crate::app::with_app;
use crate::runs::Run;
use crate::{AskOption, AskQuestion, MainWindow};

/// The popup rows of `request`.
pub fn rows(request: &QuestionRequest) -> Vec<AskQuestion> {
    let row = |q: &fast_harness_core::events::Question| {
        let options: Vec<AskOption> = (q.options.iter())
            .map(|o| AskOption {
                label: o.label.as_str().into(),
                description: o.description.clone().unwrap_or_default().into(),
                recommended: o.recommended,
                chosen: false,
            })
            .collect();
        AskQuestion {
            id: q.id.as_str().into(),
            header: q.header.clone().unwrap_or_default().into(),
            question: q.question.as_str().into(),
            multi: q.multi_select,
            options: ModelRc::new(VecModel::from(options)),
            other_chosen: false,
            other_text: SharedString::new(),
        }
    };
    request.questions.iter().map(row).collect()
}

/// Shows the oldest question request of `run` (or none).
pub fn show(run: &Run, ui: &MainWindow) {
    let waiting = (run.session.as_ref()).map(|s| s.questions()).unwrap_or_default();
    let id = waiting.first().map(|r| r.id.clone()).unwrap_or_default();
    if ui.get_ask_id() == id.as_str() {
        return;
    }
    let rows = waiting.first().map(rows).unwrap_or_default();
    ui.set_questions(ModelRc::new(VecModel::from(rows)));
    ui.set_ask_id(id.into());
}

/// Option `option` of question `index` chosen (`-1`: "Other"): a single
/// choice clears the others, a multi choice flips it.
pub fn toggle(questions: &ModelRc<AskQuestion>, index: usize, option: i32) {
    let Some(mut q) = questions.row_data(index) else {
        return;
    };
    let other = option < 0;
    for (i, mut o) in q.options.iter().enumerate() {
        let hit = !other && i as i32 == option;
        o.chosen = match q.multi {
            true => o.chosen != hit,
            false => hit,
        };
        q.options.set_row_data(i, o);
    }
    q.other_chosen = match q.multi {
        true => q.other_chosen != other,
        false => other,
    };
    questions.set_row_data(index, q);
}

/// The custom text of question `index` changed: typing chooses "Other".
pub fn other(questions: &ModelRc<AskQuestion>, index: usize, text: &str) {
    let Some(mut q) = questions.row_data(index) else {
        return;
    };
    q.other_text = text.into();
    if !text.trim().is_empty() && !q.other_chosen {
        questions.set_row_data(index, q);
        return toggle(questions, index, -1);
    }
    questions.set_row_data(index, q);
}

/// The answers as the popup holds them.
pub fn answers(questions: &ModelRc<AskQuestion>) -> Vec<QuestionAnswer> {
    let answer = |q: AskQuestion| {
        let chosen = (q.options.iter())
            .filter(|o| o.chosen)
            .map(|o| o.label.to_string())
            .collect();
        let text = q.other_text.trim().to_string();
        let other = (q.other_chosen && !text.is_empty()).then_some(text);
        QuestionAnswer::new(&q.id, chosen, other)
    };
    questions.iter().map(answer).collect()
}

pub fn connect(ui: &MainWindow) {
    ui.on_ask_toggle(|q, o| { with_app(|_, ui| toggle(&ui.get_questions(), q as usize, o)); });
    ui.on_ask_other(|q, t| { with_app(|_, ui| other(&ui.get_questions(), q as usize, &t)); });
    ui.on_ask_reply(|id, skipped| {
        with_app(|app, ui| {
            let reply = (!skipped).then(|| answers(&ui.get_questions()));
            let run = &app.runs.shown;
            if run.session.as_ref().is_some_and(|s| s.reply(&id, reply)) {
                app.refresh_run(ui);
            }
        });
    });
}

#[cfg(test)]
#[path = "ask_tests.rs"]
mod tests;
