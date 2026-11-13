//! The UI state and its key handling as a plain state machine: keys change
//! the state and return an [`Action`] that the controller carries out.

use fast_harness_core::accounts::pool::AccountStatus;
use fast_harness_core::accounts::providers::ProviderKey;
use fast_harness_core::keys::{Front, Keymap};
use fast_harness_core::llm::anthropic::{builtin_models, ModelInfo};
use fast_harness_core::session::SessionSummary;
use fast_harness_core::theme::{self, Palette, Theme, ThemeSet};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};

use super::icons::{self, Icons};
use super::input::Input;
use super::picker::{Picker, PickerKind};
use super::timeline::TraceView;

/// Lines one mouse wheel notch scrolls.
pub const WHEEL_LINES: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Chat,
    Accounts,
    Trace,
    Models,
    Sessions,
}

/// A dialog on the accounts screen.
#[derive(Default)]
pub enum Dialog {
    #[default]
    None,
    AddKey(Input),
    ConfirmRemove(String),
    /// Waiting for the browser sign-in at this URL.
    Login(String),
}

/// What the controller has to do after a key.
#[derive(Debug, PartialEq)]
pub enum Action {
    None,
    Quit,
    Send(String),
    /// Enter while a turn runs: steer it at the next step (PLAN.md D39).
    Steer(String),
    /// Ctrl+G while a turn runs: stop after this step's tools.
    Pause,
    /// `/resume`: continue a paused, cancelled or interrupted turn.
    ResumeTurn,
    Cancel,
    NewSession,
    /// Load the data of a screen and show it.
    Open(Screen),
    Resume(String),
    UseAccount(String),
    Login,
    CancelLogin,
    AddKey(String),
    Remove(String),
    PollUsage,
    SetModel(String),
    SetEffort(String),
    /// Fetch the live model list unless a fetch is running.
    FetchModels,
    /// Switch to the next theme and save it.
    CycleTheme,
    /// Step the greyscale level by this much (wrapping) and save it.
    StepGreyscale(i16),
    /// Answer approval request `id` (D64).
    Answer(String, fast_harness_core::agent::Answer),
}

pub struct Ui {
    pub screen: Screen,
    pub input: Input,
    pub running: bool,
    /// Follow-ups queued with Alt+Enter, started in order after a turn.
    pub queued: Vec<String>,
    /// Steers sent to the running turn and not read yet.
    pub steering: Vec<String>,
    pub quit_armed: bool,
    pub show_thinking: bool,
    /// First transcript line shown; `None` follows the bottom.
    pub top: Option<usize>,
    /// Transcript lines and visible rows at the last draw.
    pub total_lines: usize,
    pub view_height: usize,
    pub selected: usize,
    pub trace_top: usize,
    pub dialog: Dialog,
    /// One-line message in the hint bar.
    pub status: String,
    pub accounts: Vec<AccountStatus>,
    /// The open provider of the accounts screen; `None` lists the providers (D32).
    pub account_provider: Option<ProviderKey>,
    pub sessions: Vec<SessionSummary>,
    /// The Sessions screen's list is being read on a worker (D60).
    pub sessions_loading: bool,
    /// A session's log is being opened on a worker (D60): the transcript says
    /// so and no prompt starts.
    pub loading: bool,
    pub models: Vec<ModelInfo>,
    /// The model list the pickers show: cached, then live.
    pub catalog: Vec<ModelInfo>,
    /// The runtime's model facts (PLAN.md D67): providers, names, efforts.
    pub facts: fast_harness_core::llm::Catalog,
    /// A live model list fetch is running.
    pub fetching: bool,
    /// The trace screen: timeline entries, steps table and drawn lines.
    pub trace: TraceView,
    /// The open model or effort picker of the chat box.
    pub picker: Option<Picker>,
    pub model: String,
    pub effort: Option<String>,
    pub frame: usize,
    /// The picked theme; draw with [`Ui::palette`], which has the greyscale
    /// level applied.
    pub theme: Theme,
    /// The themes F5 cycles through: built-in, then user themes.
    pub themes: ThemeSet,
    /// Greyscale level, 0 to 100.
    pub greyscale: u8,
    /// `theme` at `greyscale`: the colours every style uses.
    pub palette: Palette,
    /// The mark set; plain until the controller reads the setting.
    pub icons: &'static Icons,
    /// The TUI bindings of `keys.json` (PLAN.md D45); defaults until the
    /// controller sets them.
    pub keys: Keymap,
    /// The open panel screen (`/panel <id>`, PLAN.md D46): drawn in place of
    /// the transcript, and it takes the keys.
    pub panel: Option<super::panel_screen::PanelScreen>,
    /// The shown session's approval requests, oldest first (D64): the
    /// first one is the prompt above the input box.
    pub approvals: Vec<fast_harness_core::events::ApprovalRequest>,
    /// The shown session's id (the Sessions screen marks it when it waits).
    pub current: String,
}

impl Ui {
    pub fn new(model: String, effort: Option<String>) -> Ui {
        Ui {
            screen: Screen::Chat,
            input: Input::default(),
            running: false,
            queued: Vec::new(),
            steering: Vec::new(),
            quit_armed: false,
            show_thinking: false,
            top: None,
            total_lines: 0,
            view_height: 1,
            selected: 0,
            trace_top: 0,
            dialog: Dialog::None,
            status: String::new(),
            accounts: Vec::new(),
            account_provider: None,
            sessions: Vec::new(),
            sessions_loading: false,
            loading: false,
            models: Vec::new(),
            catalog: builtin_models(),
            facts: Default::default(),
            fetching: false,
            trace: TraceView::default(),
            picker: None,
            model,
            effort,
            frame: 0,
            theme: theme::by_id(theme::DEFAULT_THEME).clone(),
            themes: ThemeSet::builtin(),
            greyscale: 0,
            palette: theme::by_id(theme::DEFAULT_THEME).palette,
            icons: &icons::PLAIN,
            keys: Keymap::defaults(Front::Tui),
            panel: None,
            approvals: Vec::new(),
            current: String::new(),
        }
    }

    /// Shows `theme` at the current greyscale level.
    pub fn set_theme(&mut self, theme: Theme) {
        self.palette = theme.shown(self.greyscale);
        self.theme = theme;
    }

    /// Shows the theme at greyscale `level` (clamped to 0..=100).
    pub fn set_greyscale(&mut self, level: u8) {
        self.greyscale = level.min(theme::GREYSCALE_MAX);
        self.palette = self.theme.shown(self.greyscale);
    }

    fn max_top(&self) -> usize {
        self.total_lines.saturating_sub(self.view_height)
    }

    /// Scrolls the transcript up by `lines`; stops following the bottom.
    pub fn scroll_up(&mut self, lines: usize) {
        let top = self.top.unwrap_or(self.max_top());
        self.top = Some(top.saturating_sub(lines));
    }

    /// Scrolls down; reaching the bottom follows it again.
    pub fn scroll_down(&mut self, lines: usize) {
        if let Some(top) = self.top {
            let next = top + lines;
            self.top = (next < self.max_top()).then_some(next);
        }
    }

    /// The first line to show for `total` lines in `height` rows.
    pub fn first_line(&mut self, total: usize, height: usize) -> usize {
        self.total_lines = total;
        self.view_height = height.max(1);
        let max = self.max_top();
        match self.top {
            Some(top) if top < max => top,
            _ => {
                self.top = None;
                max
            }
        }
    }

    fn page(&self) -> usize {
        self.view_height.saturating_sub(1).max(1)
    }

    pub fn mouse(&mut self, event: MouseEvent) -> bool {
        match (event.kind, self.screen) {
            (MouseEventKind::ScrollUp, Screen::Chat) => self.scroll_up(WHEEL_LINES),
            (MouseEventKind::ScrollDown, Screen::Chat) => self.scroll_down(WHEEL_LINES),
            (MouseEventKind::ScrollUp, Screen::Trace) => {
                self.trace_top = self.trace_top.saturating_sub(WHEEL_LINES)
            }
            (MouseEventKind::ScrollDown, Screen::Trace) => self.trace_scroll(WHEEL_LINES as isize),
            _ => return false,
        }
        true
    }

    pub fn paste(&mut self, text: &str) {
        match (&mut self.dialog, self.screen) {
            (Dialog::AddKey(input), Screen::Accounts) => input.insert(text.trim()),
            (_, Screen::Chat) => self.input.insert(text),
            _ => {}
        }
    }

    pub fn trace_scroll(&mut self, delta: isize) {
        let max = self.trace.lines.len().saturating_sub(self.view_height);
        let next = self.trace_top.saturating_add_signed(delta);
        self.trace_top = next.min(max);
    }

    /// Handles one key press.
    pub fn key(&mut self, key: KeyEvent) -> Action {
        if key.kind == KeyEventKind::Release {
            return Action::None;
        }
        self.status.clear();
        if self.action_of("global", &key) != Some("interrupt") {
            self.quit_armed = false;
        }
        // The approval keys first while the prompt shows (D64).
        if let Some(action) = super::approval::key(self, &key) {
            return action;
        }
        if self.picker.is_some() && self.screen == Screen::Chat {
            return self.picker_key(&key);
        }
        if let Some(action) = self.global_key(&key) {
            return action;
        }
        match self.screen {
            Screen::Chat => self.chat_key(&key),
            _ => self.screen_key(&key),
        }
    }

    fn typing_key(&self) -> bool {
        matches!(self.dialog, Dialog::AddKey(_)) && self.screen == Screen::Accounts
    }

    /// The action of `key` in `scope` of the keymap.
    pub(super) fn action_of(&self, scope: &str, key: &KeyEvent) -> Option<&'static str> {
        let pressed = super::keys::key_of(key)?;
        self.keys.lookup(scope, &pressed, self.running)
    }

    fn global_key(&mut self, key: &KeyEvent) -> Option<Action> {
        let name = self.action_of("global", key)?;
        // The key dialog types: only keys that cannot type reach it.
        let types = matches!(key.code, KeyCode::Char(_));
        if self.typing_key() && types && name != "interrupt" {
            return None;
        }
        let open = match name {
            "interrupt" => return Some(self.ctrl_c()),
            "cycle_theme" => return Some(Action::CycleTheme),
            "greyscale_up" => return Some(Action::StepGreyscale(10)),
            "greyscale_down" => return Some(Action::StepGreyscale(-10)),
            "open_accounts" => Screen::Accounts,
            "toggle_trace" if self.screen == Screen::Trace => Screen::Chat,
            "toggle_trace" => Screen::Trace,
            "open_models" => Screen::Models,
            "open_sessions" => Screen::Sessions,
            _ => return None,
        };
        if open == Screen::Sessions && self.running {
            self.status = "wait for the turn to end (Esc cancels it)".into();
            return Some(Action::None);
        }
        Some(Action::Open(open))
    }

    fn ctrl_c(&mut self) -> Action {
        if self.running {
            return Action::Cancel;
        }
        if self.typing_key() {
            self.dialog = Dialog::None;
            return Action::None;
        }
        if self.quit_armed {
            return Action::Quit;
        }
        self.quit_armed = true;
        self.input.clear();
        self.status = "press Ctrl+C again to quit".into();
        Action::None
    }

    /// A chat key: its keymap action, else editing (PLAN.md D45).
    fn chat_key(&mut self, key: &KeyEvent) -> Action {
        let Some(name) = self.action_of("chat", key) else {
            match key.code {
                KeyCode::Home if self.input.is_empty() => self.top = Some(0),
                KeyCode::End if self.input.is_empty() => self.top = None,
                _ => self.edit_key(key),
            }
            return Action::None;
        };
        match name {
            "stop" => return Action::Cancel,
            "queue" => return self.queue(),
            "pause" => return Action::Pause,
            "send" | "steer" => return self.send(),
            "quit" if self.input.is_empty() => return Action::Quit,
            "new_session" => return self.new_session(),
            "pick_model" | "pick_effort" => {
                let kind = match name {
                    "pick_model" => PickerKind::Model,
                    _ => PickerKind::Effort,
                };
                self.open_picker(kind);
                return Action::FetchModels;
            }
            "remove_queued" if !self.queued.is_empty() => self.drop_queued(),
            "newline" => self.input.insert("\n"),
            "toggle_thinking" => self.show_thinking = !self.show_thinking,
            "scroll_page_up" => self.scroll_up(self.page()),
            "scroll_page_down" => self.scroll_down(self.page()),
            "scroll_top" => self.top = Some(0),
            "scroll_bottom" => self.top = None,
            _ => self.edit_key(key),
        }
        Action::None
    }

    fn edit_key(&mut self, key: &KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            // AltGr arrives as Ctrl+Alt on Windows.
            KeyCode::Char(c) if !ctrl || alt => self.input.insert(c.encode_utf8(&mut [0; 4])),
            KeyCode::Backspace => self.input.backspace(),
            KeyCode::Delete => self.input.delete(),
            KeyCode::Left => self.input.left(),
            KeyCode::Right => self.input.right(),
            KeyCode::Home => self.input.home(),
            KeyCode::End => self.input.end(),
            KeyCode::Up => self.input.up_or_history(),
            KeyCode::Down => self.input.down_or_history(),
            _ => {}
        }
    }

    fn send(&mut self) -> Action {
        if self.input.text().trim().is_empty() {
            return Action::None;
        }
        self.top = None;
        if self.running {
            return Action::Steer(self.input.submit());
        }
        let text = self.input.submit();
        if text.trim() == super::steer::RESUME_COMMAND {
            return Action::ResumeTurn;
        }
        Action::Send(text)
    }

    fn new_session(&mut self) -> Action {
        if self.running {
            self.status = "wait for the turn to end (Esc cancels it)".into();
            return Action::None;
        }
        Action::NewSession
    }
}

pub fn is_ctrl(key: &KeyEvent, letter: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::ALT)
        && matches!(key.code, KeyCode::Char(c) if c.eq_ignore_ascii_case(&letter))
}
