//! Worker threads: the agent turn, the terminal reader, sign-in, usage reads
//! and the 10-minute usage poller. Each one reports through the UI channel.

use std::io;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use fast_harness_core::accounts::google::GoogleAuth;
use fast_harness_core::accounts::oauth;
use fast_harness_core::accounts::pool::{AccountPool, USAGE_EVERY_MS};
use fast_harness_core::agent::{Agent, Conversation, Inbox};
use fast_harness_core::app::Runtime;
use fast_harness_core::llm::anthropic::ModelInfo;
use fast_harness_core::net::UreqTransport;
use fast_harness_core::platform;
use fast_harness_core::trace::now_ms;
use ratatui::crossterm::event;

use super::app::{Msg, Tui};
use super::ui::{Action, Dialog};
use crate::accounts_cmd::label;

/// How long a sign-in waits for the browser.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);

/// What a turn thread does: a new user text, or resume the last turn.
pub enum Job {
    Text(String),
    Resume,
}

/// Runs one turn on a thread, reading `inbox` between steps (PLAN.md D39);
/// the conversation and the texts no turn used come back in `TurnDone`.
pub fn start_turn(
    agent: Agent,
    mut conv: Conversation,
    job: Job,
    cancel: Arc<AtomicBool>,
    mut inbox: Inbox,
    tx: Sender<Msg>,
) -> io::Result<()> {
    thread::Builder::new()
        .name("fast-harness-turn".into())
        .spawn(move || {
            let sink_tx = tx.clone();
            let mut sink = move |event| {
                let _ = sink_tx.send(Msg::Agent(event));
            };
            let outcome = match job {
                Job::Text(prompt) => {
                    agent.run_turn_with(&mut conv, &prompt, &cancel, &mut inbox, &mut sink)
                }
                Job::Resume => agent.resume_turn(&mut conv, &cancel, &mut inbox, &mut sink),
            };
            let leftover = inbox.leftover();
            let _ = tx.send(Msg::TurnDone(Box::new(conv), outcome, leftover));
        })
        .map(|_| ())
}

/// One live model list read, run off the UI thread.
pub type FetchJob = Box<dyn FnOnce() -> Result<Vec<ModelInfo>, String> + Send>;

/// The real fetch: GET /v1/models with the credential of this moment.
pub fn live_fetch(rt: &Runtime) -> FetchJob {
    let source = rt.models();
    Box::new(move || source.fetch())
}

/// Runs job on a thread; the result comes back in Msg::Models.
pub fn start_fetch(job: FetchJob, tx: Sender<Msg>) -> io::Result<()> {
    thread::Builder::new()
        .name("fast-harness-models".into())
        .spawn(move || {
            let _ = tx.send(Msg::Models(job()));
        })
        .map(|_| ())
}

/// Forwards terminal events until the UI is gone.
pub fn start_reader(tx: Sender<Msg>) -> io::Result<()> {
    thread::Builder::new()
        .name("fast-harness-input".into())
        .spawn(move || loop {
            let msg = match event::read() {
                Ok(event) => Msg::Term(event),
                Err(err) => {
                    let _ = tx.send(Msg::TermGone(err.to_string()));
                    return;
                }
            };
            if tx.send(msg).is_err() {
                return;
            }
        })
        .map(|_| ())
}

/// How often the poller offers the Google pool a quota read; the pool paces
/// each account itself (at most every 10 minutes, 30 s after a turn).
const GOOGLE_TICK_MS: u64 = 30_000;

/// Reads every Claude account's usage now and then every 10 minutes, and
/// offers the Google pool a read every 30 s, until the returned sender is
/// dropped. Google rounds that read nothing stay silent.
pub fn start_poller(
    pool: Arc<AccountPool>,
    google: Arc<GoogleAuth>,
    http: Arc<UreqTransport>,
    tx: Sender<Msg>,
) -> io::Result<Sender<()>> {
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    thread::Builder::new()
        .name("fast-harness-usage".into())
        .spawn(move || {
            let mut waited = USAGE_EVERY_MS;
            loop {
                let mut count = google.poll(http.as_ref(), now_ms());
                let claude = waited >= USAGE_EVERY_MS;
                if claude {
                    count += pool.poll_usage(http.as_ref());
                    waited = 0;
                }
                if (claude || count > 0) && tx.send(Msg::Polled(count)).is_err() {
                    return;
                }
                match stop_rx.recv_timeout(Duration::from_millis(GOOGLE_TICK_MS)) {
                    Err(RecvTimeoutError::Timeout) => waited += GOOGLE_TICK_MS,
                    _ => return,
                }
            }
        })?;
    Ok(stop_tx)
}

impl Tui {
    /// The account actions of the accounts screen.
    pub fn account_action(&mut self, action: Action) {
        let (pool, google) = (self.rt.pool.clone(), self.rt.google.clone());
        let is_google = |id: &str| self.ui.google.iter().any(|s| s.account.id == id);
        let result = match action {
            Action::UseAccount(id) if is_google(&id) => {
                google.use_account(&id).map(|_| format!("using {id}"))
            }
            Action::Remove(id) if is_google(&id) => {
                google.remove(&id).map(|_| format!("removed {id}"))
            }
            Action::UseAccount(id) => pool.set_active(&id).map(|_| format!("using {id}")),
            Action::Remove(id) => pool.remove(&id).map(|_| format!("removed {id}")),
            Action::AddKey(key) => pool
                .add_api_key(&key, "API key")
                .map(|account| format!("added {}", account.id)),
            Action::Login => {
                self.login();
                return;
            }
            Action::LoginGoogle => {
                self.login_google();
                return;
            }
            Action::CancelLogin => {
                self.cancel_login();
                return;
            }
            Action::PollUsage => {
                self.poll_now();
                return;
            }
            _ => return,
        };
        self.ui.status = result.unwrap_or_else(|err| err.to_string());
        self.refresh_accounts();
    }

    /// Starts one live model list fetch unless one is running.
    pub fn fetch_models(&mut self) {
        if self.ui.fetching {
            return;
        }
        let job = (self.fetcher)(&self.rt);
        match start_fetch(job, self.tx.clone()) {
            Ok(()) => self.ui.fetch_started(),
            Err(err) => self
                .ui
                .models_fetched(Err(format!("model list thread: {err}"))),
        }
    }

    fn poll_now(&mut self) {
        let (pool, http, tx) = (
            self.rt.pool.clone(),
            self.rt.transport.clone(),
            self.tx.clone(),
        );
        let spawned = thread::Builder::new()
            .name("fast-harness-usage-now".into())
            .spawn(move || {
                let count = pool.poll_usage(http.as_ref());
                let _ = tx.send(Msg::Polled(count));
            });
        self.ui.status = match spawned {
            Ok(_) => "reading usage...".into(),
            Err(err) => format!("cannot read usage: {err}"),
        };
    }

    fn login(&mut self) {
        let flow = match oauth::begin_login() {
            Ok(flow) => flow,
            Err(err) => {
                self.ui.status = format!("sign-in failed: {err}");
                return;
            }
        };
        let url = flow.url().to_string();
        if let Err(err) = platform::open_url(&url) {
            self.ui.status = format!("could not open the browser: {err}");
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let (pool, http, tx) = (
            self.rt.pool.clone(),
            self.rt.transport.clone(),
            self.tx.clone(),
        );
        let flag = cancel.clone();
        let spawned = thread::Builder::new()
            .name("fast-harness-login".into())
            .spawn(move || {
                let result = pool
                    .finish_login(http.as_ref(), &flow, LOGIN_TIMEOUT, &flag)
                    .map(|account| label(&account.email, &account.name))
                    .map_err(|err| err.to_string());
                let _ = tx.send(Msg::Login(result));
            });
        match spawned {
            Ok(_) => {
                self.ui.dialog = Dialog::Login(url);
                self.set_login_cancel(Some(cancel));
            }
            Err(err) => self.ui.status = format!("sign-in failed: {err}"),
        }
    }
}
