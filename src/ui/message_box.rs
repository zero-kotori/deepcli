use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MessageBoxAction {
    Inserted,
    Submitted(String),
    Noop,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct MessageBox {
    buffer: String,
    cursor: usize,
    history: Vec<String>,
    history_index: Option<usize>,
}

impl MessageBox {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn buffer(&self) -> &str {
        &self.buffer
    }

    pub(super) fn cursor(&self) -> usize {
        self.cursor
    }

    pub(super) fn set_buffer(&mut self, value: String) {
        self.buffer = value;
        self.cursor = self.buffer.len();
        self.history_index = None;
    }

    pub(super) fn clear(&mut self) {
        self.buffer.clear();
        self.cursor = 0;
        self.history_index = None;
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent) -> MessageBoxAction {
        match key.code {
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor = 0;
                MessageBoxAction::Inserted
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.cursor = self.buffer.len();
                MessageBoxAction::Inserted
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.buffer.drain(..self.cursor);
                self.cursor = 0;
                self.history_index = None;
                MessageBoxAction::Inserted
            }
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.buffer.drain(self.cursor..);
                self.history_index = None;
                MessageBoxAction::Inserted
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.insert_char('\n');
                MessageBoxAction::Inserted
            }
            KeyCode::Enter => {
                let submitted = self.buffer.trim_end().to_string();
                self.clear();
                if !submitted.is_empty() {
                    self.history.push(submitted.clone());
                }
                MessageBoxAction::Submitted(submitted)
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.insert_char(ch);
                MessageBoxAction::Inserted
            }
            KeyCode::Backspace => {
                self.delete_before_cursor();
                MessageBoxAction::Inserted
            }
            KeyCode::Delete => {
                self.delete_at_cursor();
                MessageBoxAction::Inserted
            }
            KeyCode::Left => {
                self.cursor = self.previous_char_boundary();
                MessageBoxAction::Inserted
            }
            KeyCode::Right => {
                self.cursor = self.next_char_boundary();
                MessageBoxAction::Inserted
            }
            KeyCode::Home => {
                self.cursor = 0;
                MessageBoxAction::Inserted
            }
            KeyCode::End => {
                self.cursor = self.buffer.len();
                MessageBoxAction::Inserted
            }
            KeyCode::Up => {
                if self.history.is_empty() {
                    return MessageBoxAction::Noop;
                }
                let next = self
                    .history_index
                    .map(|index| index.saturating_sub(1))
                    .unwrap_or_else(|| self.history.len() - 1);
                self.history_index = Some(next);
                self.buffer = self.history[next].clone();
                self.cursor = self.buffer.len();
                MessageBoxAction::Inserted
            }
            KeyCode::Down => {
                let Some(index) = self.history_index else {
                    return MessageBoxAction::Noop;
                };
                if index + 1 >= self.history.len() {
                    self.clear();
                } else {
                    self.history_index = Some(index + 1);
                    self.buffer = self.history[index + 1].clone();
                    self.cursor = self.buffer.len();
                }
                MessageBoxAction::Inserted
            }
            _ => MessageBoxAction::Noop,
        }
    }

    fn insert_char(&mut self, ch: char) {
        self.buffer.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
        self.history_index = None;
    }

    pub(super) fn insert_str(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.buffer.insert_str(self.cursor, text);
        self.cursor += text.len();
        self.history_index = None;
    }

    fn delete_before_cursor(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let previous = self.previous_char_boundary();
        self.buffer.drain(previous..self.cursor);
        self.cursor = previous;
        self.history_index = None;
    }

    fn delete_at_cursor(&mut self) {
        if self.cursor >= self.buffer.len() {
            return;
        }
        let next = self.next_char_boundary();
        self.buffer.drain(self.cursor..next);
        self.history_index = None;
    }

    fn previous_char_boundary(&self) -> usize {
        self.buffer[..self.cursor]
            .char_indices()
            .last()
            .map(|(index, _)| index)
            .unwrap_or(0)
    }

    fn next_char_boundary(&self) -> usize {
        self.buffer[self.cursor..]
            .char_indices()
            .nth(1)
            .map(|(offset, _)| self.cursor + offset)
            .unwrap_or_else(|| self.buffer.len())
    }
}

pub(super) fn handle_prompt_input_key(input: Option<&mut MessageBox>, key: KeyEvent) {
    if let Some(input) = input {
        input.handle_key(key);
    }
}
