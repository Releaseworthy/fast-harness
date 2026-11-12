//! The controller: owns the runtime, the conversation and the transcript,
//! carries out [`Action`]s and applies the messages of the worker threads.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use fast_harness_core::accounts::model::Provider;
use fast_harness_core::accounts::providers::ClientState;
use fast_harness_core::agent::{Agent, Conversation, Inbox, Input};
use fast_harness_core::app::{Runtime, Slash};
use fast_harness_core::events::{AgentEvent, TurnOutcome};
use fast_harness_core::llm::anthropic::ModelInfo;
use fast_harness_core::session;
use fast_harness_core::trace::{self, detail, report};
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
    /// The conversation, the outcome and the inbox texts no turn used.
    TurnDone(Box<Conversation>, TurnOutcome, Vec<String>),
    /// The label of the signed-in account, or the error.
    Login(Result<String, String>),
    Polled(usize),
    /// The live model list, or why it could not be read.
    Models(Result<Vec<ModelInfo>, String>),
    /// A panel process queued messages (PLAN.md D46).
    Panel,
}

pub struct Tui {
    pub rt: Runtime,
    pub cwd: PathBuf,
    conv: Option<Conversation>,
    pub session: String,
    log_path: PathBuf,
    pub transcript: Transcript,
    pub ui: Ui,
    pub cache: Cache,
    pub tx: Sender<Msg>,
    cancel: Arc<AtomicBool>,
    /// The sender of the running turn's inbox (PLAN.md D39).
    pub(super) inbox: Option<Sender<Input>>,
    login_cancel: Option<Arc<AtomicBool>>,
    /// Makes the job of one live model list fetch (tests swap it).
    pub fetcher: fn(&Runtime) -> worker::FetchJob,
    pub quit: bool,
    /// The panels of `panels/*.json`; a process starts on `/panel <id>`.
    pub panels: fast_harness_core::panels::Panels,
}

/// The transcript of a saved session: its trace, else its messages.
pub fn load_transcript(conv: &Conversation) -> Transcript {
    match trace::read(&conv.log_path) {
        Ok((_, events)) if !events.is_empty() => Transcript::from_trace(&events),
        _ => Transcript::from_messages(&conv.messages),
    }
}

impl Tui {
    pub fn new(rt: Runtime, cwd: PathBuf, conv: Conversation, tx: Sender<Msg>) -> Tui {
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
        let panels = fast_harness_core::panels::Panels::new(&rt.panels);
        let mut tui = Tui {
            panels,
            rt,
            cwd,
            session: String::new(),
            log_path: PathBuf::new(),
            conv: None,
            transcript: Transcript::new(),
            ui,
            cache: Cache::default(),
            tx,
            cancel: Arc::new(AtomicBool::new(false)),
            inbox: None,
            login_cancel: None,
            fetcher: worker::live_fetch,
            quit: false,
        };
        tui.set_conversation(conv);
        tui.refresh_accounts();
        tui
    }

    fn set_conversation(&mut self, conv: Conversation) {
        self.transcript = load_transcript(&conv);
        self.transcript.revision += 1;
        self.session = conv.id.clone();
        self.log_path = conv.log_path.clone();
        let unfinished = conv.unfinished;
        self.conv = Some(conv);
        self.ui.top = None;
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
        let mut all = self.rt.pool.status();
        all.retain(|s| s.account.provider != Provider::Google);
        self.ui.accounts = all;
        self.ui.google = self.rt.google.status();
        self.ui.google_client = ClientState::of(&self.rt.home);
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
            Msg::Agent(event) => {
                if matches!(
                    event,
                    AgentEvent::UsageUpdate { .. } | AgentEvent::AccountSwitch { .. }
                ) {
                    self.refresh_accounts();
                }
                if let AgentEvent::UserInput { text, steer: true } = &event {
                    self.ui
                        .steering
                        .retain(|sent| !text.contains(sent.as_str()));
                }
                self.panels.session(&event);
                self.transcript.apply(&event);
            }
            Msg::Panel => self.panel_poll(),
            Msg::TurnDone(conv, outcome, leftover) => self.turn_done(*conv, outcome, leftover),
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

    fn term_event(&mut self, event: Event) -> bool {
        match event {
            Event::Key(key) if self.ui.panel.is_some() => {
                self.panel_key(key);
                true
            }
            Event::Key(key) => {
                let action = self.ui.key(key);
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
                self.cancel.store(true, Ordering::SeqCst);
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
                self.rt.config.model = self.ui.model.clone();
                self.rt.config.effort = self.ui.effort.clone();
            }
            Action::SetEffort(effort) => {
                self.rt.config.set_effort(&effort);
                self.ui.effort = self.rt.config.effort.clone();
                let when = if self.ui.running {
                    "after this turn"
                } else {
                    "next turn"
                };
                self.ui.status = format!("{when}: effort {effort}");
            }
            Action::FetchModels => self.fetch_models(),
            Action::CycleTheme => {
                let theme = super::theme::cycle(&mut self.ui, &self.rt.home);
                self.rt.config.theme = theme.id;
            }
            Action::StepGreyscale(delta) => {
                let level = super::theme::step_greyscale(&mut self.ui, &self.rt.home, delta);
                self.rt.config.greyscale = level;
            }
            other => self.account_action(other),
        }
    }

    pub(super) fn send(&mut self, text: String) {
        // `/trust` trusts the project's .fast-harness folder (PLAN.md D41).
        if text.trim() == "/trust" {
            match self.rt.trust_project(&self.cwd) {
                Ok(note) => self.note(NoteKind::Info, note),
                Err(err) => self.note(NoteKind::Error, err),
            }
            return;
        }
        // `/panel [id]` (PLAN.md D46).
        if self.panel_command(&text) {
            return;
        }
        // `/help` and the user's commands/*.md (PLAN.md D42).
        let text = match self.rt.slash(&text) {
            Slash::Send(text) => text,
            Slash::Help(help) => return self.note(NoteKind::Info, help),
            Slash::Error(err) => return self.note(NoteKind::Error, err),
        };
        if self.conv.is_none() {
            self.ui.status = "a turn is running".into();
            return;
        }
        self.transcript.push_user(text.clone());
        self.start(worker::Job::Text(text));
    }

    /// Starts `job` on the idle conversation with a fresh inbox.
    pub(super) fn start(&mut self, job: worker::Job) {
        let Some(conv) = self.conv.take() else {
            self.ui.status = "a turn is running".into();
            return;
        };
        let agent: Result<(Agent, Vec<String>), String> = self.rt.agent(&self.cwd, &conv);
        let agent = match agent {
            Ok((agent, notes)) => {
                for note in notes {
                    self.note(NoteKind::Error, note);
                }
                agent
            }
            Err(err) => {
                self.conv = Some(conv);
                self.note(NoteKind::Error, format!("{err} (F2 opens the accounts)"));
                return;
            }
        };
        self.cancel.store(false, Ordering::SeqCst);
        self.ui.running = true;
        let id = conv.id.clone();
        let (inbox_tx, inbox) = Inbox::channel();
        self.inbox = Some(inbox_tx);
        let (cancel, tx) = (self.cancel.clone(), self.tx.clone());
        let started = worker::start_turn(agent, conv, job, cancel, inbox, tx);
        if let Err(err) = started {
            self.ui.running = false;
            self.inbox = None;
            self.note(NoteKind::Error, format!("cannot start the turn: {err}"));
            self.open_session(Some(&id));
        }
    }

    /// The conversation when no turn runs.
    pub(super) fn conv_mut(&mut self) -> Option<&mut Conversation> {
        self.conv.as_mut()
    }

    fn turn_done(&mut self, conv: Conversation, outcome: TurnOutcome, leftover: Vec<String>) {
        self.ui.running = false;
        self.inbox = None;
        self.conv = Some(conv);
        // Schedules the Google quota read of the account the turn used (D23).
        self.rt.google.after_turn(trace::now_ms());
        self.refresh_accounts();
        if outcome == TurnOutcome::Cancelled {
            self.ui.status = "turn cancelled: /resume continues it".into();
        } else if self.ui.status.starts_with("cancelling") {
            self.ui.status.clear();
        }
        self.after_turn(outcome, leftover);
    }

    fn open_session(&mut self, id: Option<&str>) {
        if self.ui.running {
            return;
        }
        match self.rt.open_conversation(&self.cwd, id, false) {
            Ok(conv) => {
                self.set_conversation(conv);
                self.ui.status = format!("session {}", self.session);
            }
            Err(err) => self.ui.status = err,
        }
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
            Screen::Sessions => match session::list(&self.rt.home, &self.cwd) {
                Ok(list) => self.ui.sessions = list,
                Err(err) => self.ui.status = format!("sessions: {err}"),
            },
            Screen::Chat => {}
        }
        self.ui.screen = screen;
    }

    fn load_trace(&mut self) {
        self.ui.trace_top = 0;
        match trace::read(&self.log_path) {
            Ok((_, events)) => {
                let rows = report::step_rows(&events);
                let steps = if rows.is_empty() {
                    Vec::new()
                } else {
                    report::format_table(&rows)
                        .lines()
                        .map(String::from)
                        .collect()
                };
                self.ui.trace.set(detail::entries(&events), steps, None);
            }
            Err(err) => {
                let text = format!("cannot read {}: {err}", self.log_path.display());
                self.ui.trace.set(Vec::new(), Vec::new(), Some(text));
            }
        }
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
