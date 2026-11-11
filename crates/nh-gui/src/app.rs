//! The UI-thread state and the pages that read it.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use nh_core::agent::Conversation;
use nh_core::app::Runtime;
use nh_core::trace::now_ms;
use nh_core::view::{Item, State, Transcript};
use nh_core::{session, trace};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel, Weak};

use crate::format::{duration, tokens};
use crate::mirror::Mirror;
use crate::rows::session_row;
use crate::{MainWindow, MsgRow};

pub struct App {
    pub ui: Weak<MainWindow>,
    pub rt: Runtime,
    pub cwd: PathBuf,
    /// The session; `None` before the first message and while a turn runs.
    pub conv: Option<Conversation>,
    /// Id of the current session, also while a turn has the conversation.
    pub conv_id: Option<String>,
    pub view: Transcript,
    pub mirror: Mirror,
    pub rows: Rc<VecModel<MsgRow>>,
    /// First item of the running turn: items before it do not change.
    pub turn_from: usize,
    pub busy: bool,
    pub cancel: Arc<AtomicBool>,
    pub stopping: bool,
    pub login_cancel: Option<Arc<AtomicBool>>,
    /// Model ids in the order of the Settings list.
    pub models: Vec<String>,
    /// A model list fetch runs (model_refresh).
    pub models_fetching: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

pub fn install(app: App) {
    APP.with(|cell| {
        if let Ok(mut slot) = cell.try_borrow_mut() {
            *slot = Some(app);
        }
    });
}

/// Runs `f` with the app and its window; `None` when either is gone (or on
/// a re-entrant call, which the callbacks never make).
pub fn with_app<R>(f: impl FnOnce(&mut App, &MainWindow) -> R) -> Option<R> {
    APP.with(|cell| {
        let mut slot = cell.try_borrow_mut().ok()?;
        let app = slot.as_mut()?;
        let ui = app.ui.upgrade()?;
        Some(f(app, &ui))
    })
}

pub fn show_error(ui: &MainWindow, text: impl Into<SharedString>) {
    ui.set_error_text(text.into());
}

impl App {
    pub fn new(ui: &MainWindow, rt: Runtime, cwd: PathBuf) -> App {
        let rows = Rc::new(VecModel::default());
        ui.set_messages(ModelRc::from(rows.clone()));
        App {
            ui: ui.as_weak(),
            rt,
            cwd,
            conv: None,
            conv_id: None,
            view: Transcript::new(),
            mirror: Mirror::default(),
            rows,
            turn_from: 0,
            busy: false,
            cancel: Arc::new(AtomicBool::new(false)),
            stopping: false,
            login_cancel: None,
            models: Vec::new(),
            models_fetching: false,
        }
    }

    /// Pushes the transcript changes from `turn_from` on into the model.
    pub fn sync_rows(&mut self) {
        self.mirror
            .sync(&self.view.items, &*self.rows, self.turn_from);
    }

    pub fn refresh_status(&self, ui: &MainWindow) {
        ui.set_status_state(state_text(self.view.state, self.busy, self.stopping).into());
        let model = if self.view.model.is_empty() {
            &self.rt.config.model
        } else {
            &self.view.model
        };
        ui.set_status_model(model.as_str().into());
        ui.set_status_account(self.view.account.as_str().into());
        let t = &self.view.totals;
        let used = format!(
            "session: in {} out {} cache {} / {}",
            tokens(t.input),
            tokens(t.output),
            tokens(t.cache_read),
            tokens(t.cache_write)
        );
        ui.set_status_tokens(used.into());
        ui.set_status_timing(last_step_text(&self.view.items).into());
    }

    pub fn refresh_sessions(&self, ui: &MainWindow) {
        let now = now_ms();
        let rows = match session::list(&self.rt.home, &self.cwd) {
            Ok(list) => list
                .iter()
                .map(|s| session_row(s, self.conv_id.as_deref(), now))
                .collect(),
            Err(err) => {
                show_error(ui, format!("Cannot list the sessions: {err}"));
                Vec::new()
            }
        };
        ui.set_sessions(ModelRc::new(VecModel::from(rows)));
    }

    pub fn refresh_trace(&self, ui: &MainWindow) {
        crate::trace_rows::refresh(self, ui);
    }

    /// Shows `view` from its first row.
    pub fn show_view(&mut self, ui: &MainWindow, view: Transcript) {
        self.view = view;
        self.turn_from = 0;
        self.mirror.reset(&self.view.items, &*self.rows);
        self.refresh_status(ui);
        self.refresh_sessions(ui);
        ui.set_page("chat".into());
        ui.invoke_to_bottom();
    }

    pub fn new_session(&mut self, ui: &MainWindow) {
        if self.busy {
            return show_error(ui, "Stop the running turn before starting a new session.");
        }
        self.conv = None;
        self.conv_id = None;
        self.show_view(ui, Transcript::new());
    }

    pub fn open_session(&mut self, ui: &MainWindow, id: &str) {
        if self.busy {
            return show_error(ui, "Stop the running turn before opening another session.");
        }
        match self.rt.open_conversation(&self.cwd, Some(id), false) {
            Ok(conv) => {
                let view = match trace::read(&conv.log_path) {
                    Ok((_, events)) => Transcript::from_trace(&events),
                    Err(_) => Transcript::from_messages(&conv.messages),
                };
                self.conv_id = Some(conv.id.clone());
                self.conv = Some(conv);
                self.show_view(ui, view);
            }
            Err(err) => show_error(ui, err),
        }
    }
}

pub fn state_text(state: State, busy: bool, stopping: bool) -> String {
    if busy && stopping {
        return "Stopping".into();
    }
    match state {
        State::Idle if busy => "Starting".into(),
        State::Idle => "Ready".into(),
        State::Waiting => "Waiting for the model".into(),
        State::Thinking => "Thinking".into(),
        State::Responding => "Responding".into(),
        State::RunningTools { running } => format!("Running {running} tool(s)"),
        State::Retrying { until_ms } => {
            let left = until_ms.saturating_sub(now_ms());
            format!("Retrying in {}", duration(left))
        }
    }
}

/// `last step: TTFB 850 ms, total 3.2 s` from the newest step footer.
pub fn last_step_text(items: &[Item]) -> String {
    items
        .iter()
        .rev()
        .find_map(|item| match item {
            Item::StepFooter { timing, .. } => Some(format!(
                "last step: TTFB {}, total {}",
                duration(timing.ttfb_ms),
                duration(timing.total_ms)
            )),
            _ => None,
        })
        .unwrap_or_default()
}
