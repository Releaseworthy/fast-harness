//! The controller: owns the runtime, the conversation and the transcript,
//! carries out [`Action`]s and applies the messages of the worker threads.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, MutexGuard};

use fast_harness_core::agent::Conversation;
use fast_harness_core::app::{Runtime, RuntimeHost, Slash};
use fast_harness_core::events::AgentEvent;
use fast_harness_core::llm::anthropic::ModelInfo;
use fast_harness_core::service::{Finished, Host, Service, Session, Start};
use fast_harness_core::session::SessionSummary;
use fast_harness_core::trace::TraceEvent;
use fast_harness_core::view::{Item, NoteKind, Transcript};
use ratatui::crossterm::event::Event;

use super::transcript::Cache;
use super::ui::{Action, Screen, Ui};
use super::worker;

/// What the worker threads and the terminal reader send to the UI thread.
pub enum Msg {
    Term(Event),
    TermGone(String),
    Agent(AgentEvent),
    /// The end of a turn the service ran (PLAN.md D59).
    TurnDone(Box<Finished>),
    /// Notes of a turn's agent build (an untrusted project), shown before
    /// its events.
    Notes(Vec<String>),
    /// The label of the signed-in account, or the error.
    Login(Result<String, String>),
    Polled(usize),
    /// The live model list, or why it could not be read.
    Models(Result<Vec<ModelInfo>, String>),
    /// A panel process queued messages (PLAN.md D46).
    Panel,
    /// The text of a finished `/reflect` (PLAN.md D50), or its error.
    Reflected(Result<String, String>),
    /// The Sessions screen's list of read `ticket`, read on a worker (D60).
    Sessions(u64, Result<Vec<SessionSummary>, String>),
    /// The Trace screen's read of `ticket` (D60).
    Trace(u64, super::loads::TraceRead),
    /// The open of `generation` (D60): the session's log and its transcript,
    /// or why it does not read. Boxed: it is far larger than the other
    /// messages on the channel.
    Opened(
        u64,
        Result<Box<fast_harness_core::view::opened::Opened>, String>,
    ),
}

pub struct Tui {
    /// The runtime, shared with the turns' agent builds.
    pub host: Arc<RuntimeHost>,
    pub cwd: PathBuf,
    /// Runs the turns (PLAN.md D59); `current` is the shown session.
    pub(super) service: Service,
    pub(super) current: Arc<Session>,
    pub session: String,
    pub(super) log_path: PathBuf,
    /// The reads of the Sessions and Trace screens on workers (D60).
    pub(super) loads: super::loads::Loads,
    /// The opens of sessions on workers (D60): the newest one wins.
    pub(super) opens: fast_harness_core::view::opened::Opening,
    /// Each session's unsent prompt (D65); `fresh` while the shown session
    /// is a new one, not sent yet, whose draft is the new session's.
    pub(super) drafts: fast_harness_core::app::drafts::Drafts,
    pub(super) fresh: bool,
    pub transcript: Transcript,
    pub ui: Ui,
    pub cache: Cache,
    pub tx: Sender<Msg>,

    login_cancel: Option<Arc<AtomicBool>>,
    /// Makes the job of one live model list fetch (tests swap it).
    pub fetcher: fn(&Runtime) -> worker::FetchJob,
    pub quit: bool,
    /// The panels of `panels/*.json`; a process starts on `/panel <id>`.
    pub panels: fast_harness_core::panels::Panels,
}

/// A session opened with the events of its one parse (D60).
pub type Opened = (Conversation, Vec<TraceEvent>);

/// The transcript of an opened session, with its damaged-line note (once).
pub fn load_transcript((conv, events): &Opened) -> Transcript {
    Transcript::opened(events, &conv.messages, conv.take_notes())
}

impl Tui {
    /// The TUI of `cwd`, whose settings `rt` holds; its turns run in a
    /// [`Service`] over the same runtime.
    pub fn new(rt: Runtime, cwd: PathBuf, opened: Opened, tx: Sender<Msg>) -> Result<Tui, String> {
        let host = Arc::new(RuntimeHost::at_project(rt, &cwd));
        let turns = super::turns::TuiHost::new(host.clone(), tx.clone());
        Tui::with_host(host, Arc::new(turns), cwd, opened, tx)
    }

    /// [`Tui::new`] with `turns` building the turns' agents (tests).
    pub fn with_host(
        host: Arc<RuntimeHost>,
        turns: Arc<dyn Host>,
        cwd: PathBuf,
        opened: Opened,
        tx: Sender<Msg>,
    ) -> Result<Tui, String> {
        let ui = Tui::first_ui(&host.runtime());
        let panels = fast_harness_core::panels::Panels::new(&host.runtime().panels);
        let service = Service::new(turns);
        let transcript = load_transcript(&opened);
        let conv = opened.0;
        let (session, unfinished) = (conv.id.clone(), conv.unfinished);
        // A log with no messages yet is a new session (D65).
        let fresh = conv.messages.is_empty();
        let drafts =
            fast_harness_core::app::drafts::Drafts::new((!fresh).then_some(session.as_str()));
        let log_path = conv.log_path.clone();
        let current = service.open(conv, &cwd).map_err(|err| format!("{err:?}"))?;
        // The TUI shows approval requests and answers them (D64).
        current.set_answering(true);
        let mut tui = Tui {
            panels,
            host,
            cwd,
            service,
            current,
            session,
            log_path,
            loads: Default::default(),
            opens: Default::default(),
            drafts,
            fresh,
            transcript,
            ui,
            cache: Cache::default(),
            tx,
            login_cancel: None,
            fetcher: worker::live_fetch,
            quit: false,
        };
        tui.shown(unfinished);
        tui.refresh_accounts();
        Ok(tui)
    }

    /// The runtime (settings, accounts); hold the guard briefly.
    pub fn rt(&self) -> MutexGuard<'_, Runtime> {
        self.host.runtime()
    }

    fn first_ui(rt: &Runtime) -> Ui {
        let mut ui = Ui::new(rt.config.model.clone(), rt.config.effort.clone());
        ui.themes = fast_harness_core::theme::ThemeSet::load(&rt.home);
        ui.set_greyscale(rt.config.greyscale);
        ui.set_theme(ui.themes.get(&rt.config.theme).clone());
        if let Some(note) = ui.themes.notes().first() {
            let more = ui.themes.notes().len() - 1;
            ui.status = match more {
                0 => format!("themes: {note}"),
                n => format!("themes: {note} (and {n} more)"),
            };
        }
        ui.catalog = rt.models().cached();
        ui.icons = super::icons::by_id(&rt.config.icons);
        ui.keys = rt.keys.tui.clone();
        ui
    }

    /// A session was opened: its transcript is new, the queue its own.
    pub(super) fn shown(&mut self, unfinished: Option<fast_harness_core::session::Unfinished>) {
        self.transcript.revision += 1;
        self.ui.top = None;
        self.ui.current.clone_from(&self.session);
        self.show_queue();
        if let Some(state) = unfinished {
            self.ui.status = format!("the last turn was {}: /resume continues it", state.word());
        }
    }

    pub fn note(&mut self, kind: NoteKind, text: impl Into<String>) {
        let text = text.into();
        self.transcript.items.push(Item::Note { kind, text });
        self.transcript.revision += 1;
    }

    pub fn refresh_accounts(&mut self) {
        let accounts = self.rt().pool.status();
        self.ui.accounts = accounts;
        let len = self.ui.list_len();
        if self.ui.screen == Screen::Accounts && self.ui.selected >= len {
            self.ui.selected = len.saturating_sub(1);
        }
    }

    /// Applies one message; true when the screen changed.
    pub fn handle(&mut self, msg: Msg) -> bool {
        match msg {
            Msg::Term(event) => return self.term_event(event),
            Msg::TermGone(err) => {
                self.ui.status = format!("terminal input failed: {err}");
                self.quit = true;
            }
            Msg::Agent(event) => self.agent_event(event),
            Msg::Panel => self.panel_poll(),
            Msg::Reflected(Ok(text)) => self.note(NoteKind::Info, text),
            Msg::Reflected(Err(err)) => self.note(NoteKind::Error, err),
            Msg::TurnDone(end) => self.turn_done(*end),
            Msg::Sessions(ticket, result) => self.sessions_loaded(ticket, result),
            Msg::Trace(ticket, result) => self.trace_loaded(ticket, result),
            Msg::Opened(generation, result) => self.opened(generation, result),
            Msg::Notes(notes) => {
                for note in notes {
                    self.note(NoteKind::Error, note);
                }
            }
            Msg::Login(result) => self.login_done(result),
            Msg::Models(result) => self.ui.models_fetched(result),
            Msg::Polled(count) => {
                self.refresh_accounts();
                // The 10-minute poller reports silently;  asked for this one.
                if self.ui.status.starts_with("reading usage") {
                    self.ui.status = format!("{count} usage read(s)");
                }
            }
        }
        true
    }

    fn agent_event(&mut self, event: AgentEvent) {
        if matches!(
            event,
            AgentEvent::UsageUpdate { .. } | AgentEvent::AccountSwitch { .. }
        ) {
            self.refresh_accounts();
        }
        if matches!(event, AgentEvent::UserInput { steer: true, .. }) {
            self.show_queue();
        }
        // The prompt follows the gate (D64): an answer from anywhere, a
        // cancel or a pause ends a request and its `ApprovalDone` comes.
        if matches!(
            event,
            AgentEvent::ApprovalRequest(_) | AgentEvent::ApprovalDone { .. }
        ) {
            self.ui.approvals = self.current.approvals();
        }
        self.panels.session(&event);
        self.transcript.apply(&event);
    }

    fn term_event(&mut self, event: Event) -> bool {
        match event {
            Event::Key(key) if self.ui.panel.is_some() => {
                self.panel_key(key);
                true
            }
            Event::Key(key) => {
                let before = self.ui.queued.len();
                let action = self.ui.key(key);
                self.queue_keyed(before);
                self.act(action);
                true
            }
            Event::Mouse(mouse) => self.ui.mouse(mouse),
            Event::Paste(text) => {
                self.ui.paste(&text);
                true
            }
            Event::Resize(..) => true,
            _ => false,
        }
    }

    pub fn act(&mut self, action: Action) {
        match action {
            Action::None => {}
            Action::Quit => self.quit = true,
            Action::Send(text) => self.send(text),
            Action::Steer(text) => self.steer(text),
            Action::Pause => self.pause(),
            Action::ResumeTurn => self.resume_turn(),
            Action::Cancel => {
                self.current.cancel();
                self.ui.status = "cancelling the turn...".into();
            }
            Action::NewSession => self.open_session(None),
            Action::Resume(id) => {
                self.open_session(Some(&id));
                self.ui.screen = Screen::Chat;
            }
            Action::Open(screen) => self.open(screen),
            Action::SetModel(id) => {
                self.ui.set_model(id);
                let mut rt = self.rt();
                rt.config.model = self.ui.model.clone();
                rt.config.effort = self.ui.effort.clone();
            }
            Action::SetEffort(effort) => self.set_effort(&effort),
            Action::FetchModels => self.fetch_models(),
            Action::CycleTheme => {
                let home = self.rt().home.clone();
                let theme = super::theme::cycle(&mut self.ui, &home);
                self.rt().config.theme = theme.id;
            }
            Action::StepGreyscale(delta) => {
                let home = self.rt().home.clone();
                let level = super::theme::step_greyscale(&mut self.ui, &home, delta);
                self.rt().config.greyscale = level;
            }
            Action::Answer(id, answer) => {
                if !self.current.answer(&id, answer) {
                    self.ui.status = "that request has already ended".into();
                }
                // Gone at once; `ApprovalDone` brings the gate's list back.
                self.ui.approvals.retain(|r| r.id != id);
            }
            other => self.account_action(other),
        }
    }

    fn set_effort(&mut self, effort: &str) {
        let now = {
            let mut rt = self.rt();
            rt.config.set_effort(effort);
            rt.config.effort.clone()
        };
        self.ui.effort = now;
        let when = if self.ui.running {
            "after this turn"
        } else {
            "next turn"
        };
        self.ui.status = format!("{when}: effort {effort}");
    }

    pub(super) fn send(&mut self, text: String) {
        // `/trust` trusts the project's .fast-harness folder (PLAN.md D41).
        if text.trim() == "/trust" {
            let trusted = self.rt().trust_project(&self.cwd);
            match trusted {
                Ok(note) => self.note(NoteKind::Info, note),
                Err(err) => self.note(NoteKind::Error, err),
            }
            return;
        }
        // `/panel [id]` (PLAN.md D46).
        if self.panel_command(&text) {
            return;
        }
        // `/reflect [apply]` runs off the UI thread (PLAN.md D50).
        if let Some(args) = fast_harness_core::app::reflect_args(&text) {
            return self.reflect(args);
        }
        // `/help` and the user's commands/*.md (PLAN.md D42).
        let expanded = self.rt().slash(&text);
        let text = match expanded {
            Slash::Send(text) => text,
            Slash::Help(help) => return self.note(NoteKind::Info, help),
            Slash::Error(err) => return self.note(NoteKind::Error, err),
        };
        if self.ui.loading {
            self.ui.status = "a session is loading".into();
            return;
        }
        if self.current.running() {
            self.ui.status = "a turn is running".into();
            return;
        }
        // The new session's prompt now belongs to its id (D65).
        if std::mem::take(&mut self.fresh) {
            self.drafts.created(&self.session);
        }
        self.transcript.push_user(text.clone());
        self.start(Start::Text(text));
    }

    fn open(&mut self, screen: Screen) {
        self.ui.selected = 0;
        match screen {
            Screen::Accounts => {
                // The screen opens on the provider list.
                self.ui.account_provider = None;
                self.refresh_accounts();
            }
            Screen::Trace => self.load_trace(),
            Screen::Models => {
                self.ui.models = self.ui.catalog.clone();
                let current = self.ui.models.iter().position(|m| m.id == self.ui.model);
                self.ui.selected = current.unwrap_or(0);
            }
            Screen::Sessions => self.load_sessions(),
            Screen::Chat => {}
        }
        self.ui.screen = screen;
    }

    fn login_done(&mut self, result: Result<String, String>) {
        self.login_cancel = None;
        self.ui.dialog = super::ui::Dialog::None;
        self.ui.status = match result {
            Ok(name) => format!("signed in: {name}"),
            Err(err) => format!("sign-in failed: {err}"),
        };
        self.refresh_accounts();
    }

    pub fn set_login_cancel(&mut self, cancel: Option<Arc<AtomicBool>>) {
        self.login_cancel = cancel;
    }

    pub fn cancel_login(&mut self) {
        if let Some(cancel) = &self.login_cancel {
            cancel.store(true, Ordering::SeqCst);
            self.ui.status = "cancelling the sign-in...".into();
        }
    }

    /// Something moves on screen without new input: spinners, countdowns.
    pub fn animating(&self) -> bool {
        self.ui.running || self.login_cancel.is_some() || self.ui.screen == Screen::Accounts
    }
}
