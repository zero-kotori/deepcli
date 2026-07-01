use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{latest_action_result, non_empty_output_lines, MonitorTab, TuiState};

pub(super) const TRANSCRIPT_SCROLL_STEP: usize = 6;
pub(super) const TRANSCRIPT_MOUSE_SCROLL_STEP: usize = 3;
pub(super) const RESULT_SCROLL_STEP: usize = 4;
pub(super) const RESULT_MOUSE_SCROLL_STEP: usize = 3;

pub(super) fn handle_transcript_scroll_key(key: KeyEvent, state: &mut TuiState) -> bool {
    match key.code {
        KeyCode::PageUp => {
            scroll_transcript(state, TRANSCRIPT_SCROLL_STEP);
            true
        }
        KeyCode::PageDown => {
            state.transcript_scroll = state
                .transcript_scroll
                .saturating_sub(TRANSCRIPT_SCROLL_STEP);
            state.last_event = transcript_scroll_event(state);
            true
        }
        KeyCode::Home if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.transcript_scroll = state.chat.len().saturating_sub(1);
            state.last_event = transcript_scroll_event(state);
            true
        }
        KeyCode::End if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.transcript_scroll = 0;
            state.last_event = transcript_scroll_event(state);
            true
        }
        _ => false,
    }
}

pub(super) fn handle_result_scroll_key(key: KeyEvent, state: &mut TuiState) -> bool {
    if state.monitor_tab != MonitorTab::Result || !state.input.buffer().trim().is_empty() {
        return false;
    }
    match key.code {
        KeyCode::PageUp => {
            scroll_result(state, RESULT_SCROLL_STEP);
            true
        }
        KeyCode::PageDown => {
            scroll_result_down(state, RESULT_SCROLL_STEP);
            true
        }
        KeyCode::Home if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.result_scroll = result_output_line_count(state).saturating_sub(1);
            state.last_event = result_scroll_event(state);
            true
        }
        KeyCode::End if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.result_scroll = 0;
            state.last_event = result_scroll_event(state);
            true
        }
        _ => false,
    }
}

pub(super) fn scroll_result_from_mouse(state: &mut TuiState, amount: usize) {
    if state.monitor_tab == MonitorTab::Result && state.input.buffer().trim().is_empty() {
        scroll_result(state, amount);
    }
}

pub(super) fn scroll_result(state: &mut TuiState, amount: usize) {
    let max_scroll = result_output_line_count(state).saturating_sub(1);
    state.result_scroll = state.result_scroll.saturating_add(amount).min(max_scroll);
    state.last_event = result_scroll_event(state);
}

pub(super) fn scroll_result_down(state: &mut TuiState, amount: usize) {
    if state.monitor_tab != MonitorTab::Result || !state.input.buffer().trim().is_empty() {
        return;
    }
    state.result_scroll = state.result_scroll.saturating_sub(amount);
    state.last_event = result_scroll_event(state);
}

pub(super) fn result_scroll_event(state: &TuiState) -> String {
    if latest_action_result(state).is_none() {
        return "result output unavailable".to_string();
    }
    if state.result_scroll == 0 {
        "result output at latest".to_string()
    } else {
        format!("result output scrolled back {}", state.result_scroll)
    }
}

pub(super) fn result_output_line_count(state: &TuiState) -> usize {
    latest_action_result(state)
        .map(|result| non_empty_output_lines(result.content).len())
        .unwrap_or_default()
}

pub(super) fn scroll_transcript(state: &mut TuiState, amount: usize) {
    let max_scroll = state.chat.len().saturating_sub(1);
    state.transcript_scroll = state
        .transcript_scroll
        .saturating_add(amount)
        .min(max_scroll);
    state.last_event = transcript_scroll_event(state);
}

pub(super) fn transcript_scroll_event(state: &TuiState) -> String {
    if state.transcript_scroll == 0 {
        "messages at latest".to_string()
    } else {
        format!("messages scrolled back {}", state.transcript_scroll)
    }
}
