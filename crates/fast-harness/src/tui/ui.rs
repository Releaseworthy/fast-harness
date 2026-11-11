//! The UI state and its key handling as a plain state machine: keys change
//! the state and return an [`Action`] that the controller carries out.

use fast_harness_core::accounts::google::GoogleStatus;
use fast_harness_core::accounts::pool::AccountStatus;
use fast_harness_core::accounts::providers::{ClientState, ProviderKey};
use fast_harness_core::llm::anthropic::{builtin_models, ModelInfo};
use fast_harness_core::session::SessionSummary;
use fast_harness_core::theme::{self, Theme};
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
    Cancel,
    NewSession,
    /// Load the data of a screen and show it.
    Open(Screen),
    Resume(String),
    UseAccount(String),
    Login,
    /// Start the Google Antigravity browser sign-in.
    LoginGoogle,
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
}

pub struct Ui {
    pub screen: Screen,
    pub input: Input,
    pub running: bool,
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
    /// The Google accounts, listed after ccounts (one selection over both).
    pub google: Vec<GoogleStatus>,
    /// The open provider of the accounts screen; `None` lists the providers (D32).
    pub account_provider: Option<ProviderKey>,
    /// Whether Google sign-in has an OAuth client ("Needs setup" without one).
    pub google_client: ClientState,
    pub sessions: Vec<SessionSummary>,
    pub models: Vec<ModelInfo>,
    /// The model list the pickers show: cached, then live.
    pub catalog: Vec<ModelInfo>,
    /// A live model list fetch is running.
    pub fetching: bool,
    /// The trace screen: timeline entries, steps table and drawn lines.
    pub trace: TraceView,
    /// The open model or effort picker of the chat box.
    pub picker: Option<Picker>,
    pub model: String,
    pub effort: Option<String>,
    pub frame: usize,
    pub theme: &'static Theme,
    /// The mark set; plain until the controller reads the setting.
    pub icons: &'static Icons,
}

impl Ui {
    pub fn new(model: String, effort: Option<String>) -> Ui {
        Ui {
            screen: Screen::Chat,
            input: Input::default(),
            running: false,
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
            google: Vec::new(),
            account_provider: None,
            google_client: ClientState::Missing,
            sessions: Vec::new(),
            models: Vec::new(),
            catalog: builtin_models(),
            fetching: false,
            trace: TraceView::default(),
            picker: None,
            model,
            effort,
            frame: 0,
            theme: theme::by_id(theme::DEFAULT_THEME),
            icons: &icons::PLAIN,
        }
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
        let ctrl_c = is_ctrl(&key, 'c');
        if !ctrl_c {
            self.quit_armed = false;
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

    fn global_key(&mut self, key: &KeyEvent) -> Option<Action> {
        if is_ctrl(key, 'c') {
            return Some(self.ctrl_c());
        }
        if key.code == KeyCode::F(5) {
            return Some(Action::CycleTheme);
        }
        let open = match key.code {
            KeyCode::F(2) => Screen::Accounts,
            KeyCode::F(3) if self.screen == Screen::Trace => Screen::Chat,
            KeyCode::F(3) => Screen::Trace,
            KeyCode::F(4) => Screen::Models,
            _ if self.typing_key() => return None,
            _ if is_ctrl(key, 'a') => Screen::Accounts,
            _ if is_ctrl(key, 'r') => Screen::Trace,
            _ if is_ctrl(key, 'o') => Screen::Sessions,
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

    fn chat_key(&mut self, key: &KeyEvent) -> Action {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc if self.running => return Action::Cancel,
            KeyCode::Enter if alt || ctrl || key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.input.insert("\n")
            }
            KeyCode::Char('j') if ctrl && !alt => self.input.insert("\n"),
            KeyCode::Enter => return self.send(),
            KeyCode::Char('d') if ctrl && !alt && self.input.is_empty() && !self.running => {
                return Action::Quit
            }
            KeyCode::Char('t') if ctrl && !alt => self.show_thinking = !self.show_thinking,
            KeyCode::Char('n') if ctrl && !alt => return self.new_session(),
            KeyCode::Char('p') if ctrl && !alt => {
                self.open_picker(PickerKind::Model);
                return Action::FetchModels;
            }
            KeyCode::Char('e') if ctrl && !alt => {
                self.open_picker(PickerKind::Effort);
                return Action::FetchModels;
            }
            KeyCode::PageUp => self.scroll_up(self.page()),
            KeyCode::PageDown => self.scroll_down(self.page()),
            KeyCode::Home if ctrl || self.input.is_empty() => self.top = Some(0),
            KeyCode::End if ctrl || self.input.is_empty() => self.top = None,
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
        if self.running {
            self.status = "a turn is running: wait or press Esc to cancel it".into();
            return Action::None;
        }
        self.top = None;
        Action::Send(self.input.submit())
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
