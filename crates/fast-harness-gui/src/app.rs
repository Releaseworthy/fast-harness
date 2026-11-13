//! The UI-thread state and the pages that read it.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use fast_harness_core::app::words::{state_word, Marks, Voice};
use fast_harness_core::app::Runtime;
use fast_harness_core::service::Service;
use fast_harness_core::trace::now_ms;
use fast_harness_core::view::{Item, State, Transcript};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel, Weak};

use crate::format::{duration, tokens};
use crate::host::GuiHost;
use crate::mirror::Mirror;
use crate::runs::{Run, Runs};
use crate::{MainWindow, MsgRow, QueueChip};

pub struct App {
    pub ui: Weak<MainWindow>,
    pub rt: Runtime,
    pub cwd: PathBuf,
    /// The sessions the window opened, their turns and queues (D59).
    pub service: Service,
    /// Hands the agents built here to the service's turn threads.
    pub host: Arc<GuiHost>,
    /// The view of the shown session and of the parked live ones (D39).
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
    /// The Trace page's read on a worker (D60).
    pub trace_load: crate::trace_load::TraceLoad,
    /// The session open on a worker (D60, gate G10).
    pub opens: crate::open_load::Opens,
    /// Each session's unsent composer text (D65).
    pub drafts: fast_harness_core::app::drafts::Drafts,
    /// The worker watching the shown session's background jobs (D68).
    pub jobs: fast_harness_core::view::jobs::Follow,
    /// The undo running and the last conflict (D69 "Views").
    pub undo: crate::undo_view::UndoState,
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
        let host = Arc::new(GuiHost::new(rt.home.clone()));
        App {
            ui: ui.as_weak(),
            panels: fast_harness_core::panels::Panels::new(&rt.panels),
            service: Service::new(host.clone()),
            host,
            rt,
            runs: Runs::new(cwd.clone()),
            cwd,
            mirror: Mirror::default(),
            rows,
            login_cancel: None,
            models: Vec::new(),
            models_fetching: false,
            side,
            trace_load: Default::default(),
            opens: Default::default(),
            // The window starts with a new session.
            drafts: Default::default(),
            jobs: Default::default(),
            undo: Default::default(),
        }
    }

    /// Pushes the shown transcript's changes from its turn start into the model.
    pub fn sync_rows(&mut self) {
        let run = &self.runs.shown;
        let (items, approvals) = (&run.view.items, &run.view.approvals);
        (self.mirror).sync_with(items, approvals, &*self.rows, run.turn_from);
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
        let word = match run.status().unfinished {
            Some(state) if run.can_resume() => state.word(),
            _ => "",
        };
        ui.set_resume_word(word.into());
        ui.set_approval(crate::approval::view(run, &self.rt.keys.gui));
        // The undo row follows turn ends, switches and undos (D69).
        crate::undo_view::show(self, ui);
    }

    /// Asks the workers for the sidebar's lists and redraws it (D60).
    pub fn refresh_sessions(&mut self, ui: &MainWindow) {
        crate::workspaces::refresh(self, ui);
    }

    /// Asks a worker for the Trace page of the shown session (D60).
    pub fn refresh_trace(&mut self, ui: &MainWindow) {
        crate::trace_load::refresh(self, ui);
    }

    /// Shows the shown run from its first row.
    pub fn show_run(&mut self, ui: &MainWindow) {
        let view = &self.runs.shown.view;
        (self.mirror).reset_with(&view.items, &view.approvals, &*self.rows);
        self.refresh_status(ui);
        self.refresh_run(ui);
        self.refresh_sessions(ui);
        crate::jobs_view::follow(self, ui);
        ui.set_page("chat".into());
        ui.invoke_to_bottom();
    }

    /// A new session in the current folder; a running turn goes on parked.
    pub fn new_session(&mut self, ui: &MainWindow) {
        // The new session's own draft (D65), kept while other sessions show.
        crate::drafts::switch(self, ui, None);
        let cwd = self.cwd.clone();
        let fresh = || Ok(Run::new(cwd, None, Transcript::new()));
        let shown = self.runs.show(None, fresh);
        // An open still running for another session is dropped when it comes.
        crate::open_load::clear(self, ui);
        self.shown(ui, shown);
    }

    /// Shows session `id`: its live run when it has one, else its log, which
    /// a worker opens (D60): the UI thread never parses a log (gate G10).
    pub fn open_session(&mut self, ui: &MainWindow, id: &str) {
        // The composer is `id`'s from now on, also while its log loads (D65).
        crate::drafts::switch(self, ui, Some(id));
        // The shown session needs no read (a running turn may be appending
        // to its log), and a parked run shows at once: either way an open
        // asked before is left, so its result is dropped.
        if self.runs.id.as_deref() == Some(id) {
            return crate::open_load::clear(self, ui);
        }
        if let Some(run) = self.runs.unpark(id) {
            crate::open_load::clear(self, ui);
            let shown = self.runs.take(id.to_string(), run);
            return self.shown(ui, Ok(shown));
        }
        crate::open_load::open(self, ui, id);
    }

    /// After `Runs::show`: closes the session of a dropped idle run, then
    /// shows the new run.
    fn shown(&mut self, ui: &MainWindow, shown: Result<Option<String>, String>) {
        match shown {
            Ok(dropped) => {
                if let Some(id) = dropped {
                    self.service.close(&id);
                }
                self.show_run(ui);
            }
            Err(err) => show_error(ui, err),
        }
    }
}

/// The status bar's word for `run` (the window's voice, PLAN.md D59).
pub fn state_text(state: State, run: &Run) -> String {
    let status = run.status();
    let marks = Marks {
        busy: run.busy,
        stopping: status.stopping,
        pausing: status.pausing,
        unfinished: status.unfinished,
    };
    state_word(state, marks, now_ms(), Voice::Window)
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
