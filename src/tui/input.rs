use anyhow::Result;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::widgets::ListState;

use super::{
    App, Modal,
    actions::Picker,
    screen::{Prompt, Screen, Tab},
};
use crate::model::{Priority, Status};

impl App {
    pub(super) fn command(&mut self, text: &str) -> Result<()> {
        let text = text.trim();
        match text {
            "projects" => self.go(Screen::Projects),
            "demands" => self.go(Screen::Demands(None)),
            "active" => self.go(Screen::Active),
            "questions" => self.go(Screen::Questions),
            "debts" => self.go(Screen::Debts),
            "refresh" => self.refresh()?,
            "help" => self.show_help(),
            "quit" | "q" => self.quit = true,
            "" => (),
            _ => match text.strip_prefix("search ") {
                Some(term) => {
                    self.query = term.trim().into();
                    self.go(Screen::Search);
                }
                None => self.message = "Unknown command. Press ? for help.".into(),
            },
        }
        Ok(())
    }

    pub(super) fn key(&mut self, key: KeyCode, modifiers: KeyModifiers) -> Result<()> {
        match std::mem::replace(&mut self.modal, Modal::None) {
            Modal::Input { prompt, text } => self.input_key(prompt, text, key, modifiers),
            Modal::Picker(picker) => {
                self.picker_key(picker, key);
                Ok(())
            }
            Modal::Help => {
                self.help_key(key);
                Ok(())
            }
            Modal::None => self.navigate(key),
        }
    }

    fn input_key(
        &mut self,
        prompt: Prompt,
        mut text: String,
        key: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<()> {
        let control = modifiers.contains(KeyModifiers::CONTROL);
        match key {
            KeyCode::Esc => return Ok(()),
            KeyCode::Enter => {
                let result = self.submit(&prompt, &text);
                if result.is_err() && prompt.is_draft() {
                    self.modal = Modal::Input { prompt, text };
                }
                return result;
            }
            KeyCode::Backspace => {
                text.pop();
            }
            KeyCode::Char('u') if control => text.clear(),
            KeyCode::Char(c) if !control => text.push(c),
            _ => (),
        }
        self.modal = Modal::Input { prompt, text };
        Ok(())
    }

    fn picker_key(&mut self, mut picker: Picker, key: KeyCode) {
        match key {
            KeyCode::Esc => return,
            KeyCode::Enter => {
                self.external = Some(picker.choices.remove(picker.selected).1);
                return;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                picker.selected = (picker.selected + 1).min(picker.choices.len() - 1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                picker.selected = picker.selected.saturating_sub(1);
            }
            _ => (),
        }
        self.modal = Modal::Picker(picker);
    }

    fn help_key(&mut self, key: KeyCode) {
        if matches!(key, KeyCode::Esc | KeyCode::Char('?' | 'q')) {
            self.scroll = 0;
            return;
        }
        self.modal = Modal::Help;
        self.move_selection(key, true);
    }

    fn navigate(&mut self, key: KeyCode) -> Result<()> {
        match key {
            KeyCode::Char('n') => self.begin_new()?,
            KeyCode::Char('s') => self.begin_status()?,
            KeyCode::Char('u') => self.use_selected()?,
            KeyCode::Char('c') => self.copy_context()?,
            KeyCode::Char('e') => self.edit_selected()?,
            KeyCode::Char('r') => self.resume_selected()?,
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('?') => self.show_help(),
            KeyCode::Char('/') => self.begin_prompt(Prompt::Search),
            KeyCode::Char(':') => self.begin_prompt(Prompt::Command),
            KeyCode::F(5) => self.refresh()?,
            KeyCode::Esc => self.back(),
            KeyCode::Char(digit @ '1'..='6') => self.digit(digit),
            KeyCode::Tab | KeyCode::Right if self.is_detail() => {
                self.tab = self.tab.next();
                self.scroll = 0;
                self.invalidate();
            }
            KeyCode::BackTab | KeyCode::Left if self.is_detail() => {
                self.tab = self.tab.previous();
                self.scroll = 0;
                self.invalidate();
            }
            KeyCode::Enter if !self.is_detail() => {
                if let Some(row) = self.selected_row() {
                    let (target, tab) = (row.target.clone(), row.tab);
                    self.tab = tab;
                    self.go(target);
                }
            }
            KeyCode::Char('f') => {
                self.status = Status::next_filter(self.status);
                self.filters_changed();
            }
            KeyCode::Char('p') => {
                self.priority = Priority::next_filter(self.priority);
                self.filters_changed();
            }
            KeyCode::Char('x') => {
                self.status = None;
                self.priority = None;
                self.filters_changed();
            }
            _ => self.move_selection(key, self.is_detail()),
        }
        Ok(())
    }

    fn digit(&mut self, digit: char) {
        if self.is_detail() {
            if let Some(tab) = Tab::from_digit(digit) {
                self.tab = tab;
                self.scroll = 0;
                self.invalidate();
            }
            return;
        }
        match digit {
            '1' => self.go(Screen::Projects),
            '2' => self.go(Screen::Demands(None)),
            '3' => self.go(Screen::Active),
            '4' => self.go(Screen::Questions),
            '5' => self.go(Screen::Debts),
            _ => (),
        }
    }

    fn filters_changed(&mut self) {
        self.selection = ListState::default().with_selected(Some(0));
        self.invalidate();
    }

    fn move_selection(&mut self, key: KeyCode, detail: bool) {
        let position = if detail {
            self.scroll as usize
        } else {
            self.selection.selected().unwrap_or(0)
        };
        let max = if detail {
            self.max_scroll as usize
        } else {
            self.rows.len().saturating_sub(1)
        };
        let next = match key {
            KeyCode::Down | KeyCode::Char('j') => position.saturating_add(1),
            KeyCode::Up | KeyCode::Char('k') => position.saturating_sub(1),
            KeyCode::PageDown => position.saturating_add(10),
            KeyCode::PageUp => position.saturating_sub(10),
            KeyCode::Home => 0,
            KeyCode::End => max,
            _ => position,
        }
        .min(max);
        if detail {
            self.scroll = u16::try_from(next).unwrap_or(u16::MAX);
        } else {
            self.selection.select(Some(next));
        }
    }
}
