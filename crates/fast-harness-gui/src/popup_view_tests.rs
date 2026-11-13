//! The session popup (PLAN.md D81) in a real (software rendered) window:
//! the chat keeps its frame and a trailing spacer row ends it at the
//! popup's top; the popup owns the keys of a question and of an approval.

use std::cell::RefCell;
use std::rc::Rc;

use fast_harness_core::events::{Question, QuestionAnswer, QuestionOption, QuestionRequest};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model, ModelRc, SharedString, VecModel};

use crate::turn_nav_view_tests::{frames, session, show, window, HEIGHT, WIDTH};
use crate::{ask, spacer, ApprovalView, MainWindow};

fn key(
    window: &slint::platform::software_renderer::MinimalSoftwareWindow,
    text: impl Into<SharedString>,
) {
    let text = text.into();
    window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
}

fn approve(ui: &MainWindow, model: &VecModel<crate::MsgRow>, on: bool) {
    let view = if on {
        crate::shots_approval::view()
    } else {
        ApprovalView::default()
    };
    ui.set_approval(view);
    spacer::show(ui, model);
}

#[test]
fn the_popup_keeps_the_chat_frame_and_a_spacer_ends_the_chat_at_its_top() {
    let (window, ui) = window(WIDTH, HEIGHT);
    let model = show(&ui, &session(8));
    frames(&window, 300);
    ui.invoke_to_bottom();
    frames(&window, 200);
    let (list, end, rows) = (
        ui.get_chat_list_height(),
        ui.get_chat_end(),
        model.row_count(),
    );
    approve(&ui, &model, true);
    frames(&window, 300);
    assert_eq!(ui.get_chat_list_height(), list, "the list keeps its height");
    assert_eq!(model.row_count(), rows + 1);
    assert_eq!(model.row_data(rows).expect("spacer").kind, spacer::KIND);
    let (pad, popup, top) = (ui.get_chat_pad(), ui.get_popup_height(), ui.get_popup_top());
    assert!(
        pad > 0.0 && pad <= popup,
        "the spacer is the popup's overlap: {pad} of {popup}"
    );
    assert!(
        top + popup <= HEIGHT as f32 && top + popup > HEIGHT as f32 - 80.0,
        "{top} {popup}"
    );
    let shown = ui.get_chat_end();
    assert!(
        (shown - top).abs() <= 2.0,
        "the last row ends at the popup's top: {shown} vs {top}"
    );
    approve(&ui, &model, false);
    frames(&window, 300);
    assert_eq!(model.row_count(), rows, "closing removes the spacer");
    assert_eq!(ui.get_chat_list_height(), list);
    assert!((ui.get_chat_end() - end).abs() <= 2.0, "back at the bottom");
    // Open again: at the popup's top, and the wheel still scrolls the chat.
    approve(&ui, &model, true);
    frames(&window, 300);
    assert!((ui.get_chat_end() - top).abs() <= 2.0);
    let position = LogicalPosition::new(WIDTH as f32 / 2.0, 200.0);
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerScrolled {
        position,
        delta_x: 0.0,
        delta_y: 400.0,
    });
    frames(&window, 400);
    assert!(
        ui.get_chat_end() > top + 100.0,
        "scrolled up: {}",
        ui.get_chat_end()
    );
}

fn question(id: &str, multi: bool, labels: &[&str]) -> Question {
    let mut q = Question::default();
    q.id = id.into();
    q.question = format!("Pick {id}");
    q.multi_select = multi;
    q.options = labels
        .iter()
        .enumerate()
        .map(|(i, l)| QuestionOption::new(l, i == 0 && !multi))
        .collect();
    q
}

type Replies = Rc<RefCell<Vec<(String, Option<Vec<QuestionAnswer>>)>>>;

/// The popup's callbacks wired as `ask::connect` wires them, the reply recorded.
fn wire(ui: &MainWindow) -> Replies {
    let replies = Replies::default();
    let weak = ui.as_weak();
    ui.on_ask_toggle(move |q, o| ask::toggle(&weak.unwrap().get_questions(), q as usize, o));
    let weak = ui.as_weak();
    ui.on_ask_other(move |q, t| ask::other(&weak.unwrap().get_questions(), q as usize, &t));
    let (weak, got) = (ui.as_weak(), replies.clone());
    ui.on_ask_reply(move |id, skipped| {
        let answers = (!skipped).then(|| ask::answers(&weak.unwrap().get_questions()));
        got.borrow_mut().push((id.to_string(), answers));
    });
    replies
}

fn ask_for(ui: &MainWindow, id: &str) {
    let mut request = QuestionRequest::default();
    request.questions = vec![
        question("db", false, &["Postgres", "MySQL"]),
        question("os", true, &["Linux", "Mac", "Windows"]),
        question("name", false, &["Alpha", "Beta"]),
    ];
    ui.set_questions(ModelRc::new(VecModel::from(ask::rows(&request))));
    ui.set_ask_id(id.into());
}

#[test]
fn question_keys_choose_single_multi_and_other_and_escape_skips() {
    let (window, ui) = window(WIDTH, HEIGHT);
    let replies = wire(&ui);
    ask_for(&ui, "q1");
    frames(&window, 200);
    // db: the recommended first option; os: Linux and Windows; name: Other.
    let (down, tab): (SharedString, SharedString) = (Key::DownArrow.into(), Key::Tab.into());
    let space = SharedString::from(" ");
    for k in [
        &space, &tab, &space, &down, &down, &space, &tab, &down, &down, &space,
    ] {
        key(&window, k.clone());
    }
    ui.invoke_ask_other(2, "Gamma".into());
    key(&window, Key::Return);
    let got = replies.borrow_mut().pop().expect("submitted");
    let answers = got.1.expect("answered, not skipped");
    assert_eq!(got.0, "q1");
    assert_eq!(
        answers[0],
        QuestionAnswer::new("db", vec!["Postgres".into()], None)
    );
    assert_eq!(
        answers[1],
        QuestionAnswer::new("os", vec!["Linux".into(), "Windows".into()], None)
    );
    assert_eq!(
        answers[2],
        QuestionAnswer::new("name", Vec::new(), Some("Gamma".into()))
    );
    ui.set_ask_id("".into());
    frames(&window, 100);
    ask_for(&ui, "q2");
    frames(&window, 200);
    key(&window, Key::Escape);
    assert_eq!(
        replies.borrow().last().cloned(),
        Some(("q2".to_string(), None)),
        "Esc skips"
    );
}

#[test]
fn approval_keys_answer_inside_the_popup_and_enter_space_tab_never_allow() {
    let (window, ui) = window(WIDTH, HEIGHT);
    let answers = Rc::new(RefCell::new(Vec::<String>::new()));
    let got = answers.clone();
    ui.on_answer_approval(move |_, name| got.borrow_mut().push(name.to_string()));
    ui.on_key_action(|scope, text, _, alt, _, _, _| {
        let hit = scope == "approval" && alt && text == "y";
        SharedString::from(if hit { "approve_once" } else { "" })
    });
    let stops = Rc::new(RefCell::new(0));
    let got = stops.clone();
    ui.on_stop(move || *got.borrow_mut() += 1);
    ui.set_approval(crate::shots_approval::view());
    frames(&window, 200);
    for _ in 0..4 {
        for k in [
            Key::Return.into(),
            SharedString::from(" "),
            Key::Tab.into(),
            Key::Backtab.into(),
        ] {
            key(&window, k);
        }
    }
    assert!(
        !answers.borrow().iter().any(|a| a.starts_with("approve")),
        "{:?}",
        answers.borrow()
    );
    answers.borrow_mut().clear();
    window.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Alt.into(),
    });
    key(&window, "y");
    window.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Alt.into(),
    });
    assert_eq!(answers.borrow().as_slice(), ["approve_once"]);
    // Shown again, the popup takes the focus back.
    ui.set_approval(ApprovalView::default());
    frames(&window, 100);
    ui.set_approval(crate::shots_approval::view());
    frames(&window, 100);
    key(&window, Key::Escape);
    assert_eq!(*stops.borrow(), 1, "Esc in the approval popup stops");
}

#[test]
fn the_mirror_writes_above_the_spacer() {
    use fast_harness_core::view::Approvals;
    let (one, two) = (session(1), session(2));
    let plain = VecModel::<crate::MsgRow>::default();
    crate::mirror::Mirror::new(false).reset_with(&two.items, &Approvals::default(), &plain);
    let model = VecModel::<crate::MsgRow>::default();
    spacer::set(&model, true);
    let mut mirror = crate::mirror::Mirror::new(false);
    mirror.reset_with(&one.items, &Approvals::default(), &model);
    mirror.sync(&two.items, &model, 0);
    let n = model.row_count();
    assert_eq!(
        (n, spacer::count(&model)),
        (plain.row_count() + 1, plain.row_count())
    );
    assert_eq!(model.row_data(n - 1).expect("last").kind, spacer::KIND);
    spacer::set(&model, false);
    assert_eq!(model.row_count(), plain.row_count());
}
