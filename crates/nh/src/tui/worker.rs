//! Worker threads: the agent turn, the terminal reader, sign-in, usage reads
//! and the 10-minute usage poller. Each one reports through the UI channel.

use std::io;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use nh_core::accounts::oauth;
use nh_core::accounts::pool::{AccountPool, USAGE_EVERY_MS};
use nh_core::agent::{Agent, Conversation};
use nh_core::net::UreqTransport;
use nh_core::platform;
use ratatui::crossterm::event;

use super::app::{Msg, Tui};
use super::ui::{Action, Dialog};
use crate::accounts_cmd::label;

/// How long a sign-in waits for the browser.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);

/// Runs one turn on a thread; the conversation comes back in `TurnDone`.
pub fn start_turn(
    agent: Agent,
    mut conv: Conversation,
    prompt: String,
    cancel: Arc<AtomicBool>,
    tx: Sender<Msg>,
) -> io::Result<()> {
    thread::Builder::new()
        .name("nh-turn".into())
        .spawn(move || {
            let sink_tx = tx.clone();
            let mut sink = move |event| {
                let _ = sink_tx.send(Msg::Agent(event));
            };
            let outcome = agent.run_turn(&mut conv, &prompt, &cancel, &mut sink);
            let _ = tx.send(Msg::TurnDone(Box::new(conv), outcome));
        })
        .map(|_| ())
}

/// Forwards terminal events until the UI is gone.
pub fn start_reader(tx: Sender<Msg>) -> io::Result<()> {
    thread::Builder::new()
        .name("nh-input".into())
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

/// Reads every account's usage now and then every 10 minutes, until the
/// returned sender is dropped.
pub fn start_poller(
    pool: Arc<AccountPool>,
    http: Arc<UreqTransport>,
    tx: Sender<Msg>,
) -> io::Result<Sender<()>> {
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    thread::Builder::new()
        .name("nh-usage".into())
        .spawn(move || loop {
            let count = pool.poll_usage(http.as_ref());
            if tx.send(Msg::Polled(count)).is_err() {
                return;
            }
            match stop_rx.recv_timeout(Duration::from_millis(USAGE_EVERY_MS)) {
                Err(RecvTimeoutError::Timeout) => {}
                _ => return,
            }
        })?;
    Ok(stop_tx)
}

impl Tui {
    /// The account actions of the accounts screen.
    pub fn account_action(&mut self, action: Action) {
        let pool = self.rt.pool.clone();
        let result = match action {
            Action::UseAccount(id) => pool.set_active(&id).map(|_| format!("using {id}")),
            Action::Remove(id) => pool.remove(&id).map(|_| format!("removed {id}")),
            Action::AddKey(key) => pool
                .add_api_key(&key, "API key")
                .map(|account| format!("added {}", account.id)),
            Action::Import => pool
                .import_linked()
                .map(|n| format!("{n} claude-switch account(s) added or updated")),
            Action::Login => {
                self.login();
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

    fn poll_now(&mut self) {
        let (pool, http, tx) = (
            self.rt.pool.clone(),
            self.rt.transport.clone(),
            self.tx.clone(),
        );
        let spawned = thread::Builder::new()
            .name("nh-usage-now".into())
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
            .name("nh-login".into())
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
