use crate::runtime::RuntimeProgress;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use std::sync::mpsc::Sender;

use super::command_palette::slash_command_suggestions_for_state;
use super::input_submission::submit_tui_input;
use super::monitor::MonitorQuickAction;
use super::monitor_shell::{clicked_monitor_quick_action_index, monitor_quick_actions_for_tab};
use super::session_projection::session_monitor_for_state;
use super::text::compact_ui_text;
use super::worker::WorkerDone;
use super::{rect_content_row_contains, TuiState};

pub(super) fn handle_monitor_quick_action_key(
    key: KeyEvent,
    state: &mut TuiState,
    progress_tx: &Sender<RuntimeProgress>,
    done_tx: &Sender<WorkerDone>,
) -> bool {
    if !key.modifiers.is_empty() || !state.input.buffer().trim().is_empty() {
        return false;
    }
    let monitor = session_monitor_for_state(state);
    let actions = monitor_quick_actions_for_tab(state, monitor.as_ref());
    if actions.is_empty() {
        return false;
    }
    match key.code {
        KeyCode::Up | KeyCode::Left => {
            state.selected_command = state.selected_command.saturating_sub(1);
            state.last_event = selected_quick_action_event(state.selected_command, &actions);
            true
        }
        KeyCode::Down | KeyCode::Right => {
            state.selected_command = (state.selected_command + 1).min(actions.len() - 1);
            state.last_event = selected_quick_action_event(state.selected_command, &actions);
            true
        }
        KeyCode::Enter => {
            activate_selected_monitor_quick_action(state, &actions, progress_tx, done_tx);
            true
        }
        _ => false,
    }
}

pub(super) fn selected_quick_action_event(
    selected: usize,
    actions: &[MonitorQuickAction],
) -> String {
    let selected = selected.min(actions.len().saturating_sub(1));
    let command = actions
        .get(selected)
        .map(|action| compact_ui_text(&action.command, 70))
        .unwrap_or_else(|| "<none>".to_string());
    format!("quick action selected: {command}")
}

pub(super) fn activate_selected_monitor_quick_action(
    state: &mut TuiState,
    actions: &[MonitorQuickAction],
    progress_tx: &Sender<RuntimeProgress>,
    done_tx: &Sender<WorkerDone>,
) {
    let Some(action) = actions
        .get(state.selected_command.min(actions.len().saturating_sub(1)))
        .cloned()
    else {
        return;
    };
    state.selected_command = 0;
    if action.edit_before_run {
        state.input.set_buffer(action.command.clone());
        state.last_event = format!(
            "quick action ready for edit: {}",
            compact_ui_text(&action.command, 70)
        );
        return;
    }
    state.last_event = format!(
        "quick action submitted: {}",
        compact_ui_text(&action.command, 70)
    );
    submit_tui_input(state, action.command, progress_tx.clone(), done_tx.clone());
}

pub(super) fn activate_monitor_quick_action_at_row(
    state: &mut TuiState,
    tools_area: Rect,
    row: u16,
    progress_tx: &Sender<RuntimeProgress>,
    done_tx: &Sender<WorkerDone>,
) -> bool {
    if !state.input.buffer().trim().is_empty()
        || state.resume_picker.is_some()
        || slash_command_suggestions_for_state(state.input.buffer(), state.running).is_some()
        || !rect_content_row_contains(tools_area, row)
    {
        return false;
    }
    let monitor = session_monitor_for_state(state);
    let actions = monitor_quick_actions_for_tab(state, monitor.as_ref());
    let Some(index) =
        clicked_monitor_quick_action_index(state, monitor.as_ref(), tools_area, row, &actions)
    else {
        return false;
    };
    state.selected_command = index;
    activate_selected_monitor_quick_action(state, &actions, progress_tx, done_tx);
    true
}
