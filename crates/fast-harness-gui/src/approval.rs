//! The approval prompt of the shown session (PLAN.md D64): its oldest
//! request as `MainWindow.approval`, and the answers of its buttons and of
//! the keys.json `approval` keys. The prompt follows the two events
//! (`ApprovalRequest`, `ApprovalDone`): no timer, no polling.

use fast_harness_core::agent::Answer;
use fast_harness_core::events::ApprovalRequest;
use fast_harness_core::keys::Keymap;
use fast_harness_core::tools::Effect;
use fast_harness_core::view::approvals::prompt_text;

use crate::app::with_app;
use crate::runs::Run;
use crate::{ApprovalView, MainWindow};

/// Characters of a command shown whole; a longer one keeps its start and
/// its end (what it does last is often what matters) around a count, so
/// the text stays under the software renderer's i16 pixel limit.
const WHOLE: usize = 4000;
const KEEP: usize = 1500;

/// The text as drawn: whole up to [`WHOLE`] characters.
pub fn shown_text(text: &str) -> String {
    let count = text.chars().count();
    if count <= WHOLE {
        return text.to_string();
    }
    let head: String = text.chars().take(KEEP).collect();
    let tail: String = text.chars().skip(count - KEEP).collect();
    let hidden = count - 2 * KEEP;
    format!("{head}\n... {hidden} more characters (the end follows) ...\n{tail}")
}

fn title(request: &ApprovalRequest) -> String {
    let verb = match request.effect {
        Effect::Write => "change files",
        _ => "run a command",
    };
    format!("Approval needed: {} wants to {verb}", request.tool)
}

fn key(keys: &Keymap, name: &str) -> String {
    keys.label(name).unwrap_or_else(|| "unbound".into())
}

/// The prompt of `run`: its oldest waiting request, or an empty `id`.
pub fn view(run: &Run, keys: &Keymap) -> ApprovalView {
    let waiting = (run.session.as_ref()).map(|s| s.approvals()).unwrap_or_default();
    let Some(request) = waiting.first() else {
        return ApprovalView::default();
    };
    let more = match waiting.len() - 1 {
        0 => String::new(),
        n => format!("{n} more waiting after this one"),
    };
    let agent = (request.agent.as_ref()).map_or_else(String::new, |a| format!("Asked by subagent {a}"));
    ApprovalView {
        id: request.id.as_str().into(),
        title: title(request).into(),
        text: shown_text(&prompt_text(request)).into(),
        agent: agent.into(),
        more: more.into(),
        once_key: key(keys, "approve_once").into(),
        session_key: key(keys, "approve_session").into(),
        decline_key: key(keys, "decline").into(),
    }
}

/// The answer of a keys.json action name.
pub fn answer_of(name: &str) -> Option<Answer> {
    match name {
        "approve_once" => Some(Answer::Once),
        "approve_session" => Some(Answer::Session),
        "decline" => Some(Answer::No(None)),
        _ => None,
    }
}

/// Answers request `id` of the shown session with action `name`; false
/// when it is not an answer or the request no longer waits.
pub fn answer(run: &Run, id: &str, name: &str) -> bool {
    let (Some(session), Some(answer)) = (&run.session, answer_of(name)) else {
        return false;
    };
    !id.is_empty() && session.answer(id, answer)
}

pub fn connect(ui: &MainWindow) {
    ui.on_answer_approval(|id, name| {
        with_app(|app, ui| {
            if answer(&app.runs.shown, &id, &name) {
                // The next request, if any, shows at once.
                app.refresh_run(ui);
            }
        });
    });
}
