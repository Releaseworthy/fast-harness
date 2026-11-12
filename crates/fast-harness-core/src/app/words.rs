//! Words and numbers both front ends show (PLAN.md D59): the state word of
//! a session, token counts and durations. One home, so the TUI and the GUI
//! cannot drift; where they differ on purpose the difference is a
//! parameter ([`Voice`]).

use crate::session::Unfinished;
use crate::view::State;

/// How a front end words a state: the window writes short sentences with
/// a capital ("Waiting for the model", "Retrying in 3.2 s") and knows the
/// turn's stop and pause marks; the terminal's header uses lower-case
/// words ("waiting", "retrying in 4s") and shows the marks elsewhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Voice {
    Window,
    Terminal,
}

/// What a session does besides its view's state (the window's words).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Marks {
    /// A turn started and its end has not reached the view yet.
    pub busy: bool,
    pub stopping: bool,
    pub pausing: bool,
    /// How the last turn stopped when it can be resumed.
    pub unfinished: Option<Unfinished>,
}

/// The state word of a session at `now_ms` (Unix milliseconds).
pub fn state_word(state: State, marks: Marks, now_ms: u64, voice: Voice) -> String {
    match voice {
        Voice::Window => window_word(state, marks, now_ms),
        Voice::Terminal => terminal_word(state, now_ms),
    }
}

fn window_word(state: State, marks: Marks, now_ms: u64) -> String {
    if marks.busy && marks.stopping {
        return "Stopping".into();
    }
    if marks.busy && marks.pausing {
        return "Pausing after this step's tools".into();
    }
    match state {
        State::Idle if marks.busy => "Starting".into(),
        State::Idle => match marks.unfinished {
            Some(Unfinished::Paused) => "Paused".into(),
            _ => "Ready".into(),
        },
        State::Waiting => "Waiting for the model".into(),
        State::Thinking => "Thinking".into(),
        State::Responding => "Responding".into(),
        State::RunningTools { running } => format!("Running {running} tool(s)"),
        State::Retrying { until_ms } => {
            let left = until_ms.saturating_sub(now_ms);
            format!("Retrying in {}", duration(left))
        }
    }
}

fn terminal_word(state: State, now_ms: u64) -> String {
    match state {
        State::Idle => "idle".into(),
        State::Waiting => "waiting".into(),
        State::Thinking => "thinking".into(),
        State::Responding => "responding".into(),
        State::RunningTools { running } => format!("running {running} tool(s)"),
        State::Retrying { until_ms } => {
            let secs = until_ms.saturating_sub(now_ms).div_ceil(1000);
            format!("retrying in {secs}s")
        }
    }
}

/// Token counts: `950`, `12.3k`, `4.1M`.
pub fn tokens(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

/// `850 ms`, `3.2 s`, `2m 05s`.
pub fn duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms} ms")
    } else if ms < 60_000 {
        format!("{:.1} s", ms as f64 / 1000.0)
    } else {
        let secs = ms / 1000;
        format!("{}m {:02}s", secs / 60, secs % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(state: State, marks: Marks) -> String {
        state_word(state, marks, 1_000, Voice::Window)
    }

    #[test]
    fn window_words_follow_the_marks() {
        let busy = Marks {
            busy: true,
            ..Marks::default()
        };
        assert_eq!(window(State::Idle, busy), "Starting");
        assert_eq!(window(State::Idle, Marks::default()), "Ready");
        let paused = Marks {
            unfinished: Some(Unfinished::Paused),
            ..Marks::default()
        };
        assert_eq!(window(State::Idle, paused), "Paused");
        let stopping = Marks {
            stopping: true,
            ..busy
        };
        assert_eq!(window(State::Thinking, stopping), "Stopping");
        let pausing = Marks {
            pausing: true,
            ..busy
        };
        assert_eq!(
            window(State::Idle, pausing),
            "Pausing after this step's tools"
        );
        // Marks without a turn change nothing.
        assert_eq!(
            window(
                State::Waiting,
                Marks {
                    stopping: true,
                    ..Marks::default()
                }
            ),
            "Waiting for the model"
        );
        let tools = State::RunningTools { running: 2 };
        assert_eq!(window(tools, busy), "Running 2 tool(s)");
        let retry = State::Retrying { until_ms: 4_240 };
        assert_eq!(window(retry, busy), "Retrying in 3.2 s");
    }

    #[test]
    fn terminal_words_are_short() {
        let word = |state| state_word(state, Marks::default(), 1_000, Voice::Terminal);
        assert_eq!(word(State::Idle), "idle");
        assert_eq!(word(State::Waiting), "waiting");
        assert_eq!(
            word(State::RunningTools { running: 3 }),
            "running 3 tool(s)"
        );
        assert_eq!(word(State::Retrying { until_ms: 4_001 }), "retrying in 4s");
        assert_eq!(word(State::Retrying { until_ms: 0 }), "retrying in 0s");
    }

    #[test]
    fn tokens_and_durations() {
        assert_eq!(tokens(950), "950");
        assert_eq!(tokens(12_345), "12.3k");
        assert_eq!(tokens(999_999), "1000.0k");
        assert_eq!(tokens(4_100_000), "4.1M");
        assert_eq!(duration(850), "850 ms");
        assert_eq!(duration(3_240), "3.2 s");
        assert_eq!(duration(125_000), "2m 05s");
    }
}
