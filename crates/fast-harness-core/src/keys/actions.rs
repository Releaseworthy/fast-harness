//! Every named action of each front end with its default keys. List
//! navigation (arrows, PgUp/PgDn, Home/End, Enter to pick, Esc to close a
//! picker) and text editing keys are not actions: they stay fixed.

/// The front end of a keymap (`tui` or `gui` in `keys.json`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Front {
    Tui,
    Gui,
}

impl Front {
    pub fn word(self) -> &'static str {
        match self {
            Front::Tui => "tui",
            Front::Gui => "gui",
        }
    }

    pub fn actions(self) -> &'static [ActionDef] {
        match self {
            Front::Tui => TUI_ACTIONS,
            Front::Gui => GUI_ACTIONS,
        }
    }
}

/// When an action applies. An action for one state wins over an `Always`
/// action on the same key (Enter sends when idle, steers while running).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    Always,
    Idle,
    Running,
}

impl When {
    pub fn word(self) -> &'static str {
        match self {
            When::Always => "always",
            When::Idle => "idle",
            When::Running => "while a turn runs",
        }
    }

    pub fn applies(self, running: bool) -> bool {
        match self {
            When::Always => true,
            When::Idle => !running,
            When::Running => running,
        }
    }
}

/// One action: its name in `keys.json`, where it works and its defaults.
#[derive(Clone, Copy, Debug)]
pub struct ActionDef {
    pub name: &'static str,
    /// TUI: `global` (every screen), `chat`, `screens` (every screen but
    /// the chat), `accounts`, `trace`, `models`. GUI: `window`, `composer`.
    pub scope: &'static str,
    pub when: When,
    pub keys: &'static [&'static str],
    pub about: &'static str,
}

const fn act(
    name: &'static str,
    scope: &'static str,
    when: When,
    keys: &'static [&'static str],
    about: &'static str,
) -> ActionDef {
    ActionDef {
        name,
        scope,
        when,
        keys,
        about,
    }
}

use When::{Always, Idle, Running};

/// The TUI actions in the order the help lists them.
pub const TUI_ACTIONS: &[ActionDef] = &[
    act(
        "interrupt",
        "global",
        Always,
        &["ctrl+c"],
        "cancel the turn; twice when idle quits",
    ),
    act("cycle_theme", "global", Always, &["f5"], "next theme"),
    act("greyscale_up", "global", Always, &["f6"], "greyscale +10"),
    act(
        "greyscale_down",
        "global",
        Always,
        &["shift+f6"],
        "greyscale -10",
    ),
    act(
        "open_accounts",
        "global",
        Always,
        &["f2", "ctrl+a"],
        "accounts screen",
    ),
    act(
        "toggle_trace",
        "global",
        Always,
        &["f3", "ctrl+r"],
        "trace screen and back",
    ),
    act("open_models", "global", Always, &["f4"], "models screen"),
    act(
        "open_sessions",
        "global",
        Always,
        &["ctrl+o"],
        "sessions screen",
    ),
    act("send", "chat", Idle, &["enter"], "send the input"),
    act(
        "steer",
        "chat",
        Running,
        &["enter"],
        "steer the running turn",
    ),
    act(
        "queue",
        "chat",
        Running,
        &["alt+enter"],
        "queue a follow-up",
    ),
    act(
        "newline",
        "chat",
        Always,
        &["alt+enter", "ctrl+enter", "shift+enter", "ctrl+j"],
        "new line in the input",
    ),
    act("stop", "chat", Running, &["esc"], "cancel the turn"),
    act(
        "pause",
        "chat",
        Running,
        &["ctrl+g"],
        "pause after this step's tools",
    ),
    act(
        "remove_queued",
        "chat",
        Always,
        &["ctrl+backspace"],
        "drop the last queued follow-up",
    ),
    act("quit", "chat", Idle, &["ctrl+d"], "quit (empty input)"),
    act(
        "toggle_thinking",
        "chat",
        Always,
        &["ctrl+t"],
        "show or hide thinking",
    ),
    act("new_session", "chat", Always, &["ctrl+n"], "new session"),
    act("pick_model", "chat", Always, &["ctrl+p"], "model picker"),
    act("pick_effort", "chat", Always, &["ctrl+e"], "effort picker"),
    act(
        "scroll_page_up",
        "chat",
        Always,
        &["pageup"],
        "scroll up a page",
    ),
    act(
        "scroll_page_down",
        "chat",
        Always,
        &["pagedown"],
        "scroll down a page",
    ),
    act(
        "scroll_top",
        "chat",
        Always,
        &["ctrl+home"],
        "scroll to the top",
    ),
    act(
        "scroll_bottom",
        "chat",
        Always,
        &["ctrl+end"],
        "follow the bottom",
    ),
    act(
        "back",
        "screens",
        Always,
        &["esc", "q"],
        "back (to the chat)",
    ),
    act("trace_full", "trace", Always, &["f"], "full or cut bodies"),
    act(
        "trace_steps",
        "trace",
        Always,
        &["s"],
        "steps table or timeline",
    ),
    act(
        "account_sign_in",
        "accounts",
        Always,
        &["l"],
        "Claude sign-in",
    ),
    act(
        "account_google_sign_in",
        "accounts",
        Always,
        &["g"],
        "Google sign-in",
    ),
    act(
        "account_add_key",
        "accounts",
        Always,
        &["k"],
        "add an API key",
    ),
    act(
        "account_remove",
        "accounts",
        Always,
        &["d"],
        "remove the account",
    ),
    act("account_usage", "accounts", Always, &["r"], "read usage"),
    act(
        "cycle_effort",
        "models",
        Always,
        &["e", "tab", "right", "left"],
        "next effort",
    ),
];

/// The GUI actions. `composer` actions work while the message box has the
/// focus; `window` actions anywhere in the window (Ctrl, Alt or Meta keys
/// and function keys only, so typing is never taken).
pub const GUI_ACTIONS: &[ActionDef] = &[
    act("send", "composer", Idle, &["enter"], "send the message"),
    act(
        "steer",
        "composer",
        Running,
        &["enter"],
        "steer the running turn",
    ),
    act(
        "queue",
        "composer",
        Running,
        &["alt+enter"],
        "queue a follow-up",
    ),
    act("stop", "composer", Running, &["esc"], "stop the turn"),
    act(
        "pause",
        "window",
        Running,
        &["ctrl+g"],
        "pause after this step's tools",
    ),
    act(
        "resume",
        "window",
        Idle,
        &["ctrl+shift+r"],
        "resume the last turn",
    ),
    act("new_session", "window", Always, &["ctrl+n"], "new session"),
    act(
        "toggle_sidebar",
        "window",
        Always,
        &["ctrl+b"],
        "show or hide the sidebar",
    ),
    act("open_chat", "window", Always, &["ctrl+1"], "chat page"),
    act("open_accounts", "window", Always, &["f2"], "accounts page"),
    act("open_trace", "window", Always, &["f3"], "trace page"),
    act(
        "open_settings",
        "window",
        Always,
        &["ctrl+,"],
        "settings page",
    ),
    act(
        "focus_composer",
        "window",
        Always,
        &["ctrl+l"],
        "focus the message box",
    ),
];
