//! A panel's process: its tree in one `platform::ProcessGroup`, a writer
//! thread for stdin, a reader thread that parses stdout lines, one that
//! keeps the last stderr line, and a waiter that reports the end after
//! both readers finished. Every queued message calls the [`Wake`].

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use super::proto::{parse_line, Incoming};
use super::{PanelFile, Wake};
use crate::platform::ProcessGroup;
use crate::tools::command::program;

/// What a panel's threads queue for the UI thread.
#[derive(Debug)]
pub enum Msg {
    Line(Incoming),
    /// The process ended: the exit and its last stderr line.
    Ended(String),
}

/// A running panel process; dropping it kills its tree.
pub struct Process {
    group: ProcessGroup,
    stdin: Sender<String>,
    rx: Receiver<Msg>,
}

impl Process {
    /// Starts `file`'s command in `cwd` and writes `init` first.
    pub fn start(
        file: &PanelFile,
        cwd: &Path,
        init: String,
        wake: Wake,
    ) -> Result<Process, String> {
        let mut cmd = Command::new(program(&file.command, &file.dir));
        cmd.args(&file.args)
            .current_dir(cwd)
            .env("FAST_HARNESS_PANEL_DIR", &file.dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let (group, mut child) = ProcessGroup::spawn(&mut cmd).map_err(|e| e.to_string())?;
        let (stdin, lines) = mpsc::channel::<String>();
        if let Some(mut pipe) = child.stdin.take() {
            std::thread::spawn(move || {
                for line in std::iter::once(init).chain(lines) {
                    let sent = writeln!(pipe, "{line}").and_then(|_| pipe.flush());
                    if sent.is_err() {
                        break;
                    }
                }
            });
        }
        let (tx, rx) = mpsc::channel();
        let reader = child
            .stdout
            .take()
            .map(|out| read_lines(out, tx.clone(), wake.clone()));
        let last = Arc::new(Mutex::new(String::new()));
        let errors = child.stderr.take().map(|err| last_line(err, last.clone()));
        std::thread::spawn(move || wait(child, [reader, errors], last, tx, wake));
        Ok(Process { group, stdin, rx })
    }

    pub fn send(&self, line: String) {
        let _ = self.stdin.send(line);
    }

    pub fn try_recv(&self) -> Option<Msg> {
        self.rx.try_recv().ok()
    }

    /// Kills the tree (also done on drop).
    pub fn kill(self) {}
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.group.kill();
    }
}

fn read_lines(out: impl Read + Send + 'static, tx: Sender<Msg>, wake: Wake) -> JoinHandle<()> {
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines() {
            let Ok(line) = line else { break };
            if let Some(incoming) = parse_line(&line) {
                if tx.send(Msg::Line(incoming)).is_err() {
                    break;
                }
                wake();
            }
        }
    })
}

fn last_line(err: impl Read + Send + 'static, last: Arc<Mutex<String>>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        for line in BufReader::new(err).lines() {
            let Ok(line) = line else { break };
            if !line.trim().is_empty() {
                *last.lock().unwrap_or_else(|p| p.into_inner()) = line.trim().to_string();
            }
        }
    })
}

fn wait(
    mut child: Child,
    readers: [Option<JoinHandle<()>>; 2],
    last: Arc<Mutex<String>>,
    tx: Sender<Msg>,
    wake: Wake,
) {
    let status = child.wait();
    for reader in readers.into_iter().flatten() {
        let _ = reader.join();
    }
    let exit = match status.ok().and_then(|s| s.code()) {
        Some(code) => format!("exit code {code}"),
        None => "no exit code".into(),
    };
    let last = last.lock().unwrap_or_else(|p| p.into_inner()).clone();
    let why = match last.is_empty() {
        true => format!("process ended ({exit})"),
        false => format!("process ended ({exit}): {last}"),
    };
    if tx.send(Msg::Ended(why)).is_ok() {
        wake();
    }
}
