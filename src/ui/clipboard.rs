use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use std::io::{self, Write};

use super::{rect_contains, MessageBox, TuiDialog, TuiState};

const OSC52_CLIPBOARD_TARGET: &str = "c";
const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub(super) fn handle_clipboard_key<W: Write>(
    key: KeyEvent,
    state: &mut TuiState,
    writer: &mut W,
) -> io::Result<bool> {
    if !is_copy_key(key) {
        return Ok(false);
    }

    let Some(text) = selected_editable_text(state) else {
        if key
            .modifiers
            .intersects(KeyModifiers::SUPER | KeyModifiers::META)
        {
            state.last_event = "no selected text to copy".to_string();
            return Ok(true);
        }
        return Ok(false);
    };

    writer.write_all(osc52_clipboard_sequence(&text).as_bytes())?;
    writer.flush()?;
    state.last_event = format!(
        "copied selected text to clipboard ({} chars)",
        text.chars().count()
    );
    Ok(true)
}

pub(super) fn handle_input_selection_mouse(
    state: &mut TuiState,
    mouse: MouseEvent,
    input_area: Rect,
) -> bool {
    let Some(input) = active_editable_input_mut(state) else {
        return false;
    };
    if !rect_contains(input_area, mouse.column, mouse.row) {
        return false;
    }

    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            input.handle_mouse_down(mouse.column, mouse.row, input_area);
            true
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            input.handle_mouse_drag(mouse.column, mouse.row, input_area);
            true
        }
        MouseEventKind::Up(MouseButton::Left) => {
            input.handle_mouse_up(mouse.column, mouse.row, input_area);
            true
        }
        _ => false,
    }
}

fn is_copy_key(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('c' | 'C'))
        && key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER | KeyModifiers::META)
}

fn selected_editable_text(state: &TuiState) -> Option<String> {
    if state.credential_prompt.is_some() {
        return None;
    }
    if let Some(prompt) = &state.side_question_prompt {
        return prompt.input.selected_text();
    }
    if let Some(TuiDialog::Interview(dialog)) = &state.dialog {
        return dialog.input.selected_text();
    }
    state.input.selected_text()
}

fn active_editable_input_mut(state: &mut TuiState) -> Option<&mut MessageBox> {
    if state.credential_prompt.is_some() {
        return None;
    }
    if let Some(prompt) = &mut state.side_question_prompt {
        return Some(&mut prompt.input);
    }
    if let Some(TuiDialog::Interview(dialog)) = &mut state.dialog {
        return Some(&mut dialog.input);
    }
    Some(&mut state.input)
}

fn osc52_clipboard_sequence(text: &str) -> String {
    format!(
        "\u{1b}]52;{};{}\u{7}",
        OSC52_CLIPBOARD_TARGET,
        base64_encode(text.as_bytes())
    )
}

fn base64_encode(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut index = 0usize;

    while index < bytes.len() {
        let b0 = bytes[index];
        let b1 = bytes.get(index + 1).copied().unwrap_or(0);
        let b2 = bytes.get(index + 2).copied().unwrap_or(0);

        output.push(BASE64_ALPHABET[(b0 >> 2) as usize] as char);
        output.push(BASE64_ALPHABET[(((b0 & 0b0000_0011) << 4) | (b1 >> 4)) as usize] as char);
        if index + 1 < bytes.len() {
            output.push(BASE64_ALPHABET[(((b1 & 0b0000_1111) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            output.push('=');
        }
        if index + 2 < bytes.len() {
            output.push(BASE64_ALPHABET[(b2 & 0b0011_1111) as usize] as char);
        } else {
            output.push('=');
        }

        index += 3;
    }

    output
}
