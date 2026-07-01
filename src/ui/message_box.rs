use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use std::ops::Range;
use unicode_width::UnicodeWidthChar;

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
    selection_anchor: Option<usize>,
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
        self.selection_anchor = None;
        self.history_index = None;
    }

    pub(super) fn clear(&mut self) {
        self.buffer.clear();
        self.cursor = 0;
        self.selection_anchor = None;
        self.history_index = None;
    }

    pub(super) fn selection_range(&self) -> Option<Range<usize>> {
        let anchor = self.selection_anchor?.min(self.buffer.len());
        let cursor = self.cursor.min(self.buffer.len());
        if anchor == cursor {
            return None;
        }
        Some(anchor.min(cursor)..anchor.max(cursor))
    }

    pub(super) fn selected_text(&self) -> Option<String> {
        let range = self.selection_range()?;
        Some(self.buffer[range].to_string())
    }

    pub(super) fn handle_mouse_down(&mut self, column: u16, row: u16, area: Rect) {
        let cursor = self.cursor_for_position(column, row, area);
        self.cursor = cursor;
        self.selection_anchor = Some(cursor);
    }

    pub(super) fn handle_mouse_drag(&mut self, column: u16, row: u16, area: Rect) {
        if self.selection_anchor.is_none() {
            self.selection_anchor = Some(self.cursor);
        }
        self.cursor = self.cursor_for_position(column, row, area);
    }

    pub(super) fn handle_mouse_up(&mut self, column: u16, row: u16, area: Rect) {
        if self.selection_anchor.is_some() {
            self.cursor = self.cursor_for_position(column, row, area);
        }
        if self.selection_range().is_none() {
            self.selection_anchor = None;
        }
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent) -> MessageBoxAction {
        match key.code {
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_cursor_to(0, key.modifiers.contains(KeyModifiers::SHIFT));
                MessageBoxAction::Inserted
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_cursor_to(
                    self.buffer.len(),
                    key.modifiers.contains(KeyModifiers::SHIFT),
                );
                MessageBoxAction::Inserted
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.selection_anchor = None;
                self.buffer.drain(..self.cursor);
                self.cursor = 0;
                self.history_index = None;
                MessageBoxAction::Inserted
            }
            KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.selection_anchor = None;
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
                if !key.modifiers.intersects(
                    KeyModifiers::CONTROL
                        | KeyModifiers::ALT
                        | KeyModifiers::SUPER
                        | KeyModifiers::HYPER
                        | KeyModifiers::META,
                ) =>
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
                self.move_cursor_to(
                    self.previous_char_boundary(),
                    key.modifiers.contains(KeyModifiers::SHIFT),
                );
                MessageBoxAction::Inserted
            }
            KeyCode::Right => {
                self.move_cursor_to(
                    self.next_char_boundary(),
                    key.modifiers.contains(KeyModifiers::SHIFT),
                );
                MessageBoxAction::Inserted
            }
            KeyCode::Home => {
                self.move_cursor_to(0, key.modifiers.contains(KeyModifiers::SHIFT));
                MessageBoxAction::Inserted
            }
            KeyCode::End => {
                self.move_cursor_to(
                    self.buffer.len(),
                    key.modifiers.contains(KeyModifiers::SHIFT),
                );
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
                self.selection_anchor = None;
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
                self.selection_anchor = None;
                MessageBoxAction::Inserted
            }
            _ => MessageBoxAction::Noop,
        }
    }

    fn insert_char(&mut self, ch: char) {
        self.delete_selection();
        self.buffer.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
        self.history_index = None;
    }

    pub(super) fn insert_str(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.delete_selection();
        self.buffer.insert_str(self.cursor, text);
        self.cursor += text.len();
        self.history_index = None;
    }

    fn delete_before_cursor(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.cursor == 0 {
            return;
        }
        let previous = self.previous_char_boundary();
        self.buffer.drain(previous..self.cursor);
        self.cursor = previous;
        self.history_index = None;
    }

    fn delete_at_cursor(&mut self) {
        if self.delete_selection() {
            return;
        }
        if self.cursor >= self.buffer.len() {
            return;
        }
        let next = self.next_char_boundary();
        self.buffer.drain(self.cursor..next);
        self.history_index = None;
    }

    fn delete_selection(&mut self) -> bool {
        let Some(range) = self.selection_range() else {
            return false;
        };
        let start = range.start;
        self.buffer.drain(range);
        self.cursor = start;
        self.selection_anchor = None;
        self.history_index = None;
        true
    }

    fn move_cursor_to(&mut self, next: usize, selecting: bool) {
        let next = next.min(self.buffer.len());
        if selecting {
            if self.selection_anchor.is_none() {
                self.selection_anchor = Some(self.cursor);
            }
        } else {
            self.selection_anchor = None;
        }
        self.cursor = next;
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

    fn cursor_for_position(&self, column: u16, row: u16, area: Rect) -> usize {
        let inner_x = area.x.saturating_add(1);
        let inner_y = area.y.saturating_add(1);
        let inner_width = area.width.saturating_sub(2).max(1);
        let inner_height = area.height.saturating_sub(2).max(1);
        let visual_row = row
            .saturating_sub(inner_y)
            .min(inner_height.saturating_sub(1))
            .saturating_add(self.vertical_scroll(area));
        let visual_column = column
            .saturating_sub(inner_x)
            .min(inner_width.saturating_sub(1));
        self.cursor_for_visual_offset(visual_row, visual_column, inner_width)
    }

    fn vertical_scroll(&self, area: Rect) -> u16 {
        let inner_height = area.height.saturating_sub(2).max(1);
        let (row, _) = visual_offset_for_buffer(
            &self.buffer,
            self.cursor,
            area.width.saturating_sub(2).max(1),
        );
        row.saturating_sub(inner_height.saturating_sub(1))
    }

    fn cursor_for_visual_offset(
        &self,
        target_row: u16,
        target_column: u16,
        inner_width: u16,
    ) -> usize {
        let inner_width = inner_width.max(1);
        let mut row = 0u16;
        let mut column = 0u16;

        for (index, ch) in self.buffer.char_indices() {
            if row > target_row || (row == target_row && column >= target_column) {
                return index;
            }
            if ch == '\n' {
                if row == target_row {
                    return index;
                }
                row = row.saturating_add(1);
                column = 0;
                continue;
            }

            let width = ch.width().unwrap_or(0).max(1) as u16;
            let width = width.min(inner_width);
            if column.saturating_add(width) > inner_width {
                row = row.saturating_add(1);
                column = 0;
                if row > target_row || (row == target_row && column >= target_column) {
                    return index;
                }
            }

            let next_column = column.saturating_add(width);
            if row == target_row && target_column < next_column {
                let midpoint = column.saturating_add(width / 2);
                return if target_column <= midpoint {
                    index
                } else {
                    index + ch.len_utf8()
                };
            }
            column = next_column;
            if column >= inner_width {
                row = row.saturating_add(column / inner_width);
                column %= inner_width;
            }
        }

        self.buffer.len()
    }
}

pub(super) fn handle_prompt_input_key(input: Option<&mut MessageBox>, key: KeyEvent) {
    if let Some(input) = input {
        input.handle_key(key);
    }
}

fn visual_offset_for_buffer(buffer: &str, cursor: usize, inner_width: u16) -> (u16, u16) {
    let inner_width = inner_width.max(1);
    let cursor = cursor.min(buffer.len());
    let mut row = 0u16;
    let mut column = 0u16;

    for ch in buffer[..cursor].chars() {
        if ch == '\n' {
            row = row.saturating_add(1);
            column = 0;
            continue;
        }
        let width = ch.width().unwrap_or(0).max(1) as u16;
        let width = width.min(inner_width);
        if column.saturating_add(width) > inner_width {
            row = row.saturating_add(1);
            column = 0;
        }
        column = column.saturating_add(width);
        if column >= inner_width {
            row = row.saturating_add(column / inner_width);
            column %= inner_width;
        }
    }

    (row, column.min(inner_width.saturating_sub(1)))
}
