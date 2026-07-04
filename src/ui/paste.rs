use super::command_palette::clamp_selected_command;
use super::{TuiDialog, TuiState};

pub(super) fn handle_tui_paste(state: &mut TuiState, text: &str) {
    let text = normalize_pasted_text(text);
    if text.is_empty() {
        return;
    }
    let char_count = text.chars().count();
    if let Some(prompt) = &mut state.credential_prompt {
        prompt.input.insert_str(&text);
        state.last_event = format!("pasted {char_count} hidden char(s)");
        return;
    }
    if let Some(prompt) = &mut state.side_question_prompt {
        prompt.input.insert_str(&text);
        state.last_event = format!("pasted {char_count} char(s) into btw answer");
        return;
    }
    if let Some(TuiDialog::Interview(dialog)) = &mut state.dialog {
        dialog.input.insert_str(&text);
        state.last_event = format!("pasted {char_count} char(s) into btw answer");
        return;
    }
    if let Some(picker) = &mut state.resume_picker {
        picker.push_query_str(&text);
        state.last_event = format!("pasted {char_count} char(s) into resume filter");
        return;
    }

    state.input.insert_str(&text);
    clamp_selected_command(state);
    state.last_event = format!("pasted {char_count} char(s)");
}

pub(super) fn normalize_pasted_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}
