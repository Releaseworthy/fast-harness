//! The controller: owns the runtime, the conversation and the transcript,
//! carries out [`Action`]s and applies the messages of the worker threads.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use nh_core::agent::{Agent, Conversation};
use nh_core::app::Runtime;
use nh_core::events::{AgentEvent, TurnOutcome};
use nh_core::llm::anthropic::ModelInfo;
use nh_core::session;
use nh_core::trace::{self, detail, report};
use nh_core::view::{Item, NoteKind, Transcript};
use ratatui::crossterm::event::Event;

use super::transcript::Cache;
use super::ui::{Action, Screen, Ui};
use super::worker;

/// What the worker threads and the terminal reader send to the UI thread.
pub enum Msg {
    Term(Event),
    TermGone(String),
    Agent(AgentEvent),
    TurnDone(Box<Conversation>, TurnOutcome),
    /// The label of the signed-in account, or the error.
    Login(Result<String, String>),
    Polled(usize),
    /// The live model list, or why it could not be read.
    Models(Result<Vec<ModelInfo>, String>),
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
    login_cancel: Option<Arc<AtomicBool>>,
    /// Makes the job of one live model list fetch (tests swap it).
    pub fetcher: fn(&Runtime) -> worker::FetchJob,
    pub quit: bool,
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
        ui.theme = nh_core::theme::by_id(&rt.config.theme);
        ui.catalog = rt.models().cached();
        let mut tui = Tui {
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
        self.conv = Some(conv);
        self.ui.top = None;
    }

    pub fn note(&mut self, kind: NoteKind, text: impl Into<String>) {
        let text = text.into();
        self.transcript.items.push(Item::Note { kind, text });
        self.transcript.revision += 1;
    }

    pub fn refresh_accounts(&mut self) {
        self.ui.accounts = self.rt.pool.status();
        let len = self.ui.accounts.len();
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
                self.transcript.apply(&event);
            }
            Msg::TurnDone(conv, outcome) => self.turn_done(*conv, outcome),
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
                self.rt.config.theme = theme.id.to_string();
            }
            other => self.account_action(other),
        }
    }

    fn send(&mut self, text: String) {
        let Some(conv) = self.conv.take() else {
            self.ui.status = "a turn is running".into();
            return;
        };
        let agent: Result<Agent, String> = self.rt.agent(&self.cwd, &conv);
        self.transcript.push_user(text.clone());
        let agent = match agent {
            Ok(agent) => agent,
            Err(err) => {
                self.conv = Some(conv);
                self.note(NoteKind::Error, format!("{err} (F2 opens the accounts)"));
                return;
            }
        };
        self.cancel.store(false, Ordering::SeqCst);
        self.ui.running = true;
        let id = conv.id.clone();
        let started = worker::start_turn(agent, conv, text, self.cancel.clone(), self.tx.clone());
        if let Err(err) = started {
            self.ui.running = false;
            self.note(NoteKind::Error, format!("cannot start the turn: {err}"));
            self.open_session(Some(&id));
        }
    }

    fn turn_done(&mut self, conv: Conversation, outcome: TurnOutcome) {
        self.ui.running = false;
        self.conv = Some(conv);
        self.refresh_accounts();
        if outcome == TurnOutcome::Cancelled {
            self.ui.status = "turn cancelled".into();
        } else if self.ui.status.starts_with("cancelling") {
            self.ui.status.clear();
        }
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
                self.refresh_accounts();
                self.ui.selected = self.ui.accounts.iter().position(|s| s.active).unwrap_or(0);
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
