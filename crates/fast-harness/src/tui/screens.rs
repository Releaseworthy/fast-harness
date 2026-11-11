//! Keys of the accounts, trace, model and session screens.

use fast_harness_core::accounts::providers::ProviderKey;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::input::Input;
use super::picker::next_effort;
use super::ui::{Action, Dialog, Screen, Ui};

impl Ui {
    pub(super) fn list_len(&self) -> usize {
        match (self.screen, self.account_provider) {
            (Screen::Accounts, None) => ProviderKey::ALL.len(),
            (Screen::Accounts, Some(ProviderKey::Claude)) => self.accounts.len(),
            (Screen::Accounts, Some(ProviderKey::Google)) => self.google.len(),
            (Screen::Models, _) => self.models.len(),
            (Screen::Sessions, _) => self.sessions.len(),
            _ => 0,
        }
    }

    fn move_selection(&mut self, key: &KeyEvent) -> bool {
        let len = self.list_len();
        match key.code {
            KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down if self.selected + 1 < len => self.selected += 1,
            KeyCode::Down => {}
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.selected = len.saturating_sub(1),
            _ => return false,
        }
        true
    }

    pub(super) fn screen_key(&mut self, key: &KeyEvent) -> Action {
        if self.screen == Screen::Accounts && !matches!(self.dialog, Dialog::None) {
            return self.dialog_key(key);
        }
        if let (Screen::Accounts, Some(open), KeyCode::Esc) =
            (self.screen, self.account_provider, key.code)
        {
            // Back to the provider list, on the provider just left.
            self.account_provider = None;
            self.selected = ProviderKey::ALL
                .iter()
                .position(|p| *p == open)
                .unwrap_or(0);
            return Action::None;
        }
        if key.code == KeyCode::Esc || key.code == KeyCode::Char('q') {
            self.screen = Screen::Chat;
            return Action::None;
        }
        if self.screen == Screen::Trace {
            self.trace_key(key);
            return Action::None;
        }
        if self.move_selection(key) {
            return Action::None;
        }
        match self.screen {
            Screen::Accounts => self.accounts_key(key),
            Screen::Models => self.models_key(key),
            Screen::Sessions => match (key.code, self.sessions.get(self.selected)) {
                (KeyCode::Enter, Some(session)) => Action::Resume(session.id.clone()),
                _ => Action::None,
            },
            Screen::Chat | Screen::Trace => Action::None,
        }
    }

    fn trace_key(&mut self, key: &KeyEvent) {
        let page = self.view_height.saturating_sub(1).max(1) as isize;
        match key.code {
            KeyCode::Up => self.trace_scroll(-1),
            KeyCode::Down => self.trace_scroll(1),
            KeyCode::PageUp => self.trace_scroll(-page),
            KeyCode::PageDown => self.trace_scroll(page),
            KeyCode::Home => self.trace_top = 0,
            KeyCode::End => self.trace_scroll(isize::MAX / 2),
            KeyCode::Char('f') => {
                self.trace.full = !self.trace.full;
                self.trace_top = 0;
            }
            KeyCode::Char('s') => {
                self.trace.show_steps = !self.trace.show_steps;
                self.trace_top = 0;
            }
            _ => {}
        }
    }

    /// The id of the selected row of the open provider.
    fn selected_account(&self) -> Option<String> {
        let account = match self.account_provider? {
            ProviderKey::Claude => &self.accounts.get(self.selected)?.account,
            ProviderKey::Google => &self.google.get(self.selected)?.account,
        };
        Some(account.id.clone())
    }

    /// Enter on the provider list opens one, on its active account.
    fn open_provider(&mut self) {
        let Some(&provider) = ProviderKey::ALL.get(self.selected) else {
            return;
        };
        self.account_provider = Some(provider);
        let active = match provider {
            ProviderKey::Claude => self.accounts.iter().position(|s| s.active),
            ProviderKey::Google => self.google.iter().position(|s| s.active),
        };
        self.selected = active.unwrap_or(0);
    }

    fn accounts_key(&mut self, key: &KeyEvent) -> Action {
        let id = self.selected_account();
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return Action::None;
        }
        if self.account_provider.is_none() && key.code == KeyCode::Enter {
            self.open_provider();
            return Action::None;
        }
        match (key.code, id) {
            (KeyCode::Enter, Some(id)) => Action::UseAccount(id),
            (KeyCode::Char('l'), _) => Action::Login,
            (KeyCode::Char('g'), _) => Action::LoginGoogle,
            (KeyCode::Char('k'), _) => {
                self.dialog = Dialog::AddKey(Input::masked());
                Action::None
            }
            (KeyCode::Char('d'), Some(id)) => {
                self.dialog = Dialog::ConfirmRemove(id);
                Action::None
            }
            (KeyCode::Char('r'), _) => Action::PollUsage,
            _ => Action::None,
        }
    }

    fn dialog_key(&mut self, key: &KeyEvent) -> Action {
        match std::mem::take(&mut self.dialog) {
            Dialog::None => Action::None,
            Dialog::Login(url) if key.code == KeyCode::Esc => {
                self.dialog = Dialog::Login(url);
                Action::CancelLogin
            }
            Dialog::Login(url) => {
                self.dialog = Dialog::Login(url);
                Action::None
            }
            Dialog::ConfirmRemove(id) => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => Action::Remove(id),
                _ => Action::None,
            },
            Dialog::AddKey(mut input) => match key.code {
                KeyCode::Esc => Action::None,
                KeyCode::Enter if !input.text().trim().is_empty() => {
                    Action::AddKey(input.text().trim().to_string())
                }
                code => {
                    match code {
                        KeyCode::Char(c) => input.insert(c.encode_utf8(&mut [0; 4])),
                        KeyCode::Backspace => input.backspace(),
                        _ => {}
                    }
                    self.dialog = Dialog::AddKey(input);
                    Action::None
                }
            },
        }
    }

    fn models_key(&mut self, key: &KeyEvent) -> Action {
        match key.code {
            KeyCode::Enter => match self.models.get(self.selected) {
                Some(model) => Action::SetModel(model.id.clone()),
                None => Action::None,
            },
            KeyCode::Char('e') | KeyCode::Tab | KeyCode::Right | KeyCode::Left => {
                match next_effort(&self.model, self.effort.as_deref()) {
                    Some(effort) => Action::SetEffort(effort),
                    None => Action::None,
                }
            }
            _ => Action::None,
        }
    }
}
