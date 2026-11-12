//! The UI-thread state and the pages that read it.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use fast_harness_core::app::Runtime;
use fast_harness_core::session::Unfinished;
use fast_harness_core::trace::{self, now_ms};
use fast_harness_core::view::{Item, State, Transcript};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel, Weak};

use crate::format::{duration, tokens};
use crate::mirror::Mirror;
use crate::runs::{Run, Runs};
use crate::{MainWindow, MsgRow, QueueChip};

pub struct App {
    pub ui: Weak<MainWindow>,
    pub rt: Runtime,
    pub cwd: PathBuf,
    /// The shown session and the parked ones with running turns (D39).
    pub runs: Runs,
    pub mirror: Mirror,
    pub rows: Rc<VecModel<MsgRow>>,
    pub login_cancel: Option<Arc<AtomicBool>>,
    /// Model ids in the order of the Settings list.
    pub models: Vec<String>,
    /// A model list fetch runs (model_refresh).
    pub models_fetching: bool,
    /// The sidebar's workspaces.
    pub side: crate::workspaces::Side,
    /// The panels of `panels/*.json` (PLAN.md D46); a process starts on open.
    pub panels: fast_harness_core::panels::Panels,
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
    /// `arg`: the folder from the command line, if any (see `workspaces::start`).
    pub fn new(ui: &MainWindow, rt: Runtime, arg: Option<PathBuf>) -> App {
        let rows = Rc::new(VecModel::default());
        ui.set_messages(ModelRc::from(rows.clone()));
        let (side, cwd, error) = crate::workspaces::start(&rt.home, arg);
        if let Some(error) = error {
            show_error(ui, error);
        }
        App {
            ui: ui.as_weak(),
            panels: fast_harness_core::panels::Panels::new(&rt.panels),
            rt,
            runs: Runs::new(cwd.clone()),
            cwd,
            mirror: Mirror::default(),
            rows,
            login_cancel: None,
            models: Vec::new(),
            models_fetching: false,
            side,
        }
    }

    /// Pushes the shown transcript's changes from its turn start into the model.
    pub fn sync_rows(&mut self) {
        let run = &self.runs.shown;
        self.mirror
            .sync(&run.view.items, &*self.rows, run.turn_from);
    }

    pub fn refresh_status(&self, ui: &MainWindow) {
        let view = &self.runs.shown.view;
        ui.set_status_state(state_text(view.state, &self.runs.shown).into());
        let model = if view.model.is_empty() {
            &self.rt.config.model
        } else {
            &view.model
        };
        ui.set_status_model(model.as_str().into());
        ui.set_status_account(view.account.as_str().into());
        let t = &view.totals;
        let context = view
            .context_status()
            .map(|text| format!("{text} · "))
            .unwrap_or_default();
        let used = format!(
            "{context}session: in {} out {} cache {} / {}",
            tokens(t.input),
            tokens(t.output),
            tokens(t.cache_read),
            tokens(t.cache_write)
        );
        ui.set_status_tokens(used.into());
        ui.set_status_timing(last_step_text(&view.items).into());
    }

    /// The composer's part of the shown run: busy, the chips and Resume.
    pub fn refresh_run(&self, ui: &MainWindow) {
        let run = &self.runs.shown;
        ui.set_busy(run.busy);
        let chips: Vec<QueueChip> = (run.chips().into_iter())
            .map(|(index, text)| QueueChip {
                index,
                text: text.into(),
            })
            .collect();
        ui.set_queue_chips(ModelRc::new(VecModel::from(chips)));
        let word = match run.unfinished {
            Some(state) if run.can_resume() => state.word(),
            _ => "",
        };
        ui.set_resume_word(word.into());
    }

    pub fn refresh_sessions(&self, ui: &MainWindow) {
        crate::workspaces::refresh(self, ui);
    }

    pub fn refresh_trace(&self, ui: &MainWindow) {
        crate::trace_rows::refresh(self, ui);
    }

    /// Shows the shown run from its first row.
    pub fn show_run(&mut self, ui: &MainWindow) {
        self.mirror.reset(&self.runs.shown.view.items, &*self.rows);
        self.refresh_status(ui);
        self.refresh_run(ui);
        self.refresh_sessions(ui);
        ui.set_page("chat".into());
        ui.invoke_to_bottom();
    }

    /// A new session in the current folder; a running turn goes on parked.
    pub fn new_session(&mut self, ui: &MainWindow) {
        let cwd = self.cwd.clone();
        let fresh = || Ok(Run::new(cwd, None, Transcript::new()));
        if let Err(err) = self.runs.show(None, fresh) {
            return show_error(ui, err);
        }
        self.show_run(ui);
    }

    /// Shows session `id`: its live run when it has one, else its log.
    pub fn open_session(&mut self, ui: &MainWindow, id: &str) {
        let (rt, cwd) = (&self.rt, self.cwd.clone());
        let load = || {
            let conv = rt.open_conversation(&cwd, Some(id), false)?;
            let view = match trace::read(&conv.log_path) {
                Ok((_, events)) => Transcript::from_trace(&events),
                Err(_) => Transcript::from_messages(&conv.messages),
            };
            Ok(Run::new(cwd.clone(), Some(conv), view))
        };
        match self.runs.show(Some(id.to_string()), load) {
            Ok(()) => self.show_run(ui),
            Err(err) => show_error(ui, err),
        }
    }
}

pub fn state_text(state: State, run: &Run) -> String {
    if run.busy && run.stopping {
        return "Stopping".into();
    }
    if run.busy && run.pausing {
        return "Pausing after this step's tools".into();
    }
    match state {
        State::Idle if run.busy => "Starting".into(),
        State::Idle => match run.unfinished {
            Some(Unfinished::Paused) => "Paused".into(),
            _ => "Ready".into(),
        },
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
