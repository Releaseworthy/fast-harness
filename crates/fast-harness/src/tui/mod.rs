//! `fast-harness tui` (and `fast-harness` alone on a terminal): the full-screen front end.
//! The UI thread owns the state; the agent turn, the terminal reader,
//! sign-in and usage reads run on threads and report through one channel.

mod app;
mod draw;
mod google;
mod icons;
mod input;
mod markdown;
mod panels;
mod picker;
#[cfg(test)]
mod picker_fetch_tests;
#[cfg(test)]
mod picker_tests;
mod screens;
#[cfg(test)]
mod scroll_tests;
mod steer;
#[cfg(test)]
mod steer_tests;
#[cfg(test)]
mod tabs_tests;
#[cfg(test)]
mod tests;
mod theme;
#[cfg(test)]
mod theme_tests;
mod timeline;
mod transcript;
mod ui;
mod worker;
mod wrap;

use std::io::{self, stdout};
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use fast_harness_core::trace::now_ms;
use ratatui::crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event,
    KeyCode, KeyEventKind, KeyModifiers,
};
use ratatui::crossterm::execute;
use ratatui::DefaultTerminal;

use crate::args::TuiArgs;
use crate::auth_choice::runtime;
use app::{Msg, Tui};
use draw::View;

/// Shortest time between two redraws (about 30 a second).
const FRAME: Duration = Duration::from_millis(33);
/// Wake-up while spinners or countdowns move.
const TICK: Duration = Duration::from_millis(100);
/// Wake-up when nothing moves.
const IDLE: Duration = Duration::from_secs(5);

pub fn run(args: TuiArgs) -> Result<ExitCode, String> {
    let mut rt = runtime()?;
    rt.interactive = true;
    let cwd = crate::work_dir(args.cwd.clone())?;
    rt.use_project(&cwd)?;
    let conv = rt.open_conversation(&cwd, args.resume.as_deref(), args.continue_latest)?;
    let (tx, rx) = mpsc::channel();
    let mut tui = Tui::new(rt, cwd, conv, tx.clone());
    let _poller = worker::start_poller(
        tui.rt.pool.clone(),
        tui.rt.google.clone(),
        tui.rt.transport.clone(),
        tx.clone(),
    )
    .map_err(|err| format!("usage thread: {err}"))?;
    let mut terminal = enter().map_err(|err| format!("terminal: {err}"))?;
    let _guard = Guard;
    worker::start_reader(tx).map_err(|err| format!("input thread: {err}"))?;
    event_loop(&mut terminal, &mut tui, &rx).map_err(|err| format!("terminal: {err}"))?;
    drop(_guard);
    println!(
        "session {} (resume with `fast-harness tui --resume {}`)",
        tui.session, tui.session
    );
    Ok(ExitCode::SUCCESS)
}

/// Raw mode, alternate screen, mouse and paste; a panic hook restores them.
fn enter() -> io::Result<DefaultTerminal> {
    let terminal = ratatui::try_init()?;
    // ratatui's hook restores raw mode and the screen; this one adds mouse and paste.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(stdout(), DisableMouseCapture, DisableBracketedPaste);
        previous(info);
    }));
    // Both are optional: without them the wheel and multi-line pastes degrade.
    let _ = execute!(stdout(), EnableMouseCapture);
    let _ = execute!(stdout(), EnableBracketedPaste);
    Ok(terminal)
}

/// Without bracketed paste (the Windows console) a paste arrives as keys: an
/// Enter followed by more key presses in the same batch becomes a newline.
pub(crate) fn paste_newlines(batch: &mut [Msg]) {
    let press =
        |msg: &Msg| matches!(msg, Msg::Term(Event::Key(k)) if k.kind != KeyEventKind::Release);
    let last = batch.iter().rposition(press);
    for (index, msg) in batch.iter_mut().enumerate() {
        if let Msg::Term(Event::Key(key)) = msg {
            let plain = key.code == KeyCode::Enter && key.modifiers.is_empty();
            if plain && key.kind != KeyEventKind::Release && Some(index) != last {
                key.modifiers = KeyModifiers::ALT;
            }
        }
    }
}

fn restore() {
    let _ = execute!(stdout(), DisableMouseCapture, DisableBracketedPaste);
    ratatui::restore();
}

/// Restores the terminal on every way out of [`run`], errors included.
struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        restore();
    }
}

fn redraw(terminal: &mut DefaultTerminal, tui: &mut Tui, started: Instant) -> io::Result<()> {
    let now = now_ms();
    tui.ui.frame = (started.elapsed().as_millis() / TICK.as_millis()) as usize;
    let Tui {
        ui,
        transcript,
        cache,
        session,
        ..
    } = tui;
    let mut view = View {
        transcript,
        cache,
        session,
    };
    terminal.draw(|frame| draw::draw(frame, ui, &mut view, now))?;
    Ok(())
}

/// Waits for messages, applies them in batches and redraws at most once a
/// frame, only when something changed.
fn event_loop(terminal: &mut DefaultTerminal, tui: &mut Tui, rx: &Receiver<Msg>) -> io::Result<()> {
    let started = Instant::now();
    let mut dirty = true;
    let mut drawn = Instant::now() - FRAME;
    let mut revision = u64::MAX;
    while !tui.quit {
        let changed = dirty || tui.transcript.revision != revision;
        let wait = if changed {
            FRAME.saturating_sub(drawn.elapsed())
        } else if tui.animating() {
            TICK
        } else {
            IDLE
        };
        if changed && wait.is_zero() {
            redraw(terminal, tui, started)?;
            (dirty, drawn, revision) = (false, Instant::now(), tui.transcript.revision);
            continue;
        }
        match rx.recv_timeout(wait) {
            Ok(msg) => {
                let mut batch = vec![msg];
                batch.extend(rx.try_iter());
                paste_newlines(&mut batch);
                for msg in batch {
                    dirty |= tui.handle(msg);
                }
            }
            Err(RecvTimeoutError::Timeout) => dirty |= !changed && tui.animating(),
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}
