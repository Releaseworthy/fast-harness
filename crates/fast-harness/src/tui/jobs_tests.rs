//! The jobs line above the input (PLAN.md D68 "Views"): drawn only while the
//! session has jobs, a mark and a word per state, fed by the watch worker,
//! a stale list dropped, and `/kill`.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use fast_harness_core::app::Runtime;
use fast_harness_core::tools::JobState;
use fast_harness_core::view::jobs::{JobRow, Mark};
use fast_harness_core::view::Transcript;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use super::app::{Msg, Tui};
use super::draw::{self, View};
use super::icons::PLAIN;
use super::theme::Styles;
use super::transcript::Cache;
use super::ui::Ui;

const WIDTH: u16 = 100;

fn row(id: &str, command: &str, word: &str, mark: Mark) -> JobRow {
    let (id, command, word) = (id.into(), command.into(), word.into());
    JobRow {
        id,
        command,
        word,
        mark,
        took: None,
    }
}

/// The frame's rows of text, and the buffer.
fn draw_ui(ui: &mut Ui) -> (Vec<String>, ratatui::buffer::Buffer) {
    let transcript = Transcript::new();
    let mut cache = Cache::default();
    let mut view = View {
        transcript: &transcript,
        cache: &mut cache,
        session: "s-1",
    };
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, 12)).unwrap();
    terminal
        .draw(|frame| draw::draw(frame, ui, &mut view, 0))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let cells: Vec<&str> = buffer.content().iter().map(|c| c.symbol()).collect();
    let rows = cells
        .chunks(usize::from(WIDTH))
        .map(|r| r.concat())
        .collect();
    (rows, buffer)
}

#[test]
fn the_line_above_the_input_shows_running_and_exited_jobs() {
    let mut ui = Ui::new("m".into(), None);
    ui.icons = &PLAIN;
    ui.jobs = vec![
        row("job-1", "ping -t localhost", "running", Mark::Running),
        row("job-2", "cargo build", "exited 0", Mark::Exited),
        row("job-3", "exit 3", "exited 3", Mark::Failed),
    ];
    let (rows, buffer) = draw_ui(&mut ui);
    let at = rows
        .iter()
        .position(|r| r.starts_with("jobs "))
        .expect("a jobs line");
    let line = rows[at].trim_end();
    let expected = "jobs  ○ job-1 ping -t localhost running  ✓ job-2 cargo build exited 0  \
                    ✗ job-3 exit 3 exited 3";
    assert_eq!(line, expected);
    // The input box's top border is the next row.
    assert!(
        rows[at + 1].starts_with('╭') || rows[at + 1].starts_with('┌'),
        "{rows:#?}"
    );
    let st = Styles::of(&ui);
    // Every mark is one cell wide, so a character index is a column.
    let x = |needle: &str| line[..line.find(needle).unwrap()].chars().count();
    let cell = |needle: &str| &buffer[(x(needle) as u16, at as u16)];
    assert_eq!(cell("○").fg, st.accent().fg.unwrap());
    assert_eq!(cell("exited 0").fg, st.success().fg.unwrap());
    assert_eq!(cell("exited 3").fg, st.error().fg.unwrap());
}

#[test]
fn no_line_when_the_session_has_no_jobs() {
    let mut ui = Ui::new("m".into(), None);
    let (rows, _) = draw_ui(&mut ui);
    assert!(!rows.iter().any(|r| r.starts_with("jobs ")), "{rows:#?}");
}

fn root(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    std::env::temp_dir().join(format!("fh-jobs-{tag}-{}-{nanos}", std::process::id()))
}

fn tui(root: &Path) -> (Tui, Receiver<Msg>) {
    let (home, cwd) = (root.join("home"), root.join("work"));
    std::fs::create_dir_all(&cwd).unwrap();
    let rt = Runtime::load_at(home).unwrap();
    let opened = rt.open_with_events(&cwd, None, false).unwrap();
    let (tx, rx) = mpsc::channel();
    (Tui::new(rt, cwd, opened, tx).unwrap(), rx)
}

/// Applies messages until the shown jobs satisfy `done`.
fn until(tui: &mut Tui, rx: &Receiver<Msg>, done: impl Fn(&[JobRow]) -> bool) {
    while !done(&tui.ui.jobs) {
        let msg = rx.recv_timeout(Duration::from_secs(20)).expect("a message");
        tui.handle(msg);
    }
}

#[test]
fn the_worker_feeds_the_line_drops_a_stale_list_and_kill_ends_the_job() {
    let root = root("feed");
    let (mut tui, rx) = tui(&root);
    assert!(tui.ui.jobs.is_empty());
    let jobs = tui.current.jobs();
    let id = jobs.start("sleep 30", &tui.cwd, None).unwrap();
    until(&mut tui, &rx, |rows| {
        rows.first().is_some_and(JobRow::running)
    });
    // A list of an earlier generation (another session's) is dropped.
    tui.handle(Msg::Jobs(0, Vec::new()));
    assert_eq!(tui.ui.jobs.len(), 1);
    tui.send(format!("/kill {id}"));
    until(&mut tui, &rx, |rows| rows[0].mark == Mark::Killed);
    assert_eq!(tui.ui.jobs[0].word, "killed");
    assert_eq!(jobs.list()[0].state, JobState::Killed);
    drop(tui);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn only_kill_as_a_word_is_the_kill_command() {
    let root = root("word");
    let (mut tui, _rx) = tui(&root);
    assert!(
        !tui.kill_command("/killport 8080"),
        "a user command keeps its name"
    );
    assert!(tui.kill_command("/kill job-9"));
    drop(tui);
    let _ = std::fs::remove_dir_all(&root);
}
