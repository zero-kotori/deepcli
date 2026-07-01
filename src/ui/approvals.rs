use super::monitor_shell::format_task_monitor_text;
use super::{
    handle_prompt_input_key, rect_contains, rect_content_row_contains, session_monitor_for_state,
    short_id, ChatLine, MessageBox, MonitorTab, TuiState,
};
use crate::session::{ApprovalStatus, SessionStore};
use anyhow::{anyhow, Result};
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

#[derive(Debug)]
pub(super) struct SideQuestionPrompt {
    pub(super) id: String,
    pub(super) question: String,
    pub(super) input: MessageBox,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SelectedBlocker {
    Approval(String),
    SideQuestion(String),
}

pub(super) fn blocker_count(state: &TuiState) -> Option<usize> {
    session_monitor_for_state(state)
        .map(|monitor| monitor.pending_approvals.len() + monitor.open_questions.len())
}

fn selected_blocker(state: &TuiState) -> Option<SelectedBlocker> {
    let monitor = session_monitor_for_state(state)?;
    let approval_count = monitor.pending_approvals.len();
    let total = approval_count + monitor.open_questions.len();
    if total == 0 {
        return None;
    }
    let index = state.selected_approval.min(total - 1);
    if index < approval_count {
        Some(SelectedBlocker::Approval(
            monitor.pending_approvals[index].id.clone(),
        ))
    } else {
        Some(SelectedBlocker::SideQuestion(
            monitor.open_questions[index - approval_count].id.clone(),
        ))
    }
}

pub(super) fn handle_approval_tab_key(key: KeyEvent, state: &mut TuiState) -> bool {
    if state.monitor_tab != MonitorTab::Approvals || !state.input.buffer().trim().is_empty() {
        return false;
    }
    let Some(count) = blocker_count(state) else {
        return false;
    };
    if count == 0 {
        return false;
    }
    match key.code {
        KeyCode::Up | KeyCode::Left => {
            state.selected_approval = state.selected_approval.saturating_sub(1);
            true
        }
        KeyCode::Down | KeyCode::Right => {
            state.selected_approval = (state.selected_approval + 1).min(count - 1);
            true
        }
        KeyCode::Enter => {
            activate_selected_blocker(state);
            true
        }
        KeyCode::Char('d') | KeyCode::Delete => {
            deny_selected_blocker(state);
            true
        }
        _ => false,
    }
}

pub(super) fn handle_approvals_mouse_for_state(
    state: &mut TuiState,
    mouse: MouseEvent,
    tools_area: Rect,
) -> bool {
    if state.monitor_tab != MonitorTab::Approvals
        || !state.input.buffer().trim().is_empty()
        || state.resume_picker.is_some()
        || state.credential_prompt.is_some()
        || state.side_question_prompt.is_some()
        || super::slash_command_suggestions_for_state(state.input.buffer(), state.running).is_some()
        || !rect_contains(tools_area, mouse.column, mouse.row)
    {
        return false;
    }
    match mouse.kind {
        MouseEventKind::ScrollUp => {
            let Some(count) = blocker_count(state) else {
                return false;
            };
            if count == 0 {
                return false;
            }
            state.selected_approval = state.selected_approval.saturating_sub(1);
            state.last_event = selected_blocker_event(state);
            true
        }
        MouseEventKind::ScrollDown => {
            let Some(count) = blocker_count(state) else {
                return false;
            };
            if count == 0 {
                return false;
            }
            state.selected_approval = state.selected_approval.saturating_add(1).min(count - 1);
            state.last_event = selected_blocker_event(state);
            true
        }
        MouseEventKind::Down(MouseButton::Left) => {
            let Some(index) =
                clicked_approvals_tab_index(state, tools_area, mouse.column, mouse.row)
            else {
                return false;
            };
            state.selected_approval = index;
            state.last_event = selected_blocker_event(state);
            true
        }
        _ => false,
    }
}

fn clicked_approvals_tab_index(
    state: &TuiState,
    tools_area: Rect,
    column: u16,
    row: u16,
) -> Option<usize> {
    if !rect_contains(tools_area, column, row) || !rect_content_row_contains(tools_area, row) {
        return None;
    }
    let monitor = session_monitor_for_state(state)?;
    let content_row = row.saturating_sub(tools_area.y + 1) as usize;
    let text = format_task_monitor_text(state, Some(&monitor), tools_area.height);
    let line = text.lines().nth(content_row)?.trim_start();
    if !(line.starts_with("* ") || line.starts_with("- ")) {
        return None;
    }
    for (index, approval) in monitor.pending_approvals.iter().enumerate() {
        if line.contains(short_id(&approval.id)) {
            return Some(index);
        }
    }
    let approval_count = monitor.pending_approvals.len();
    for (index, question) in monitor.open_questions.iter().enumerate() {
        if line.contains(short_id(&question.id)) {
            return Some(approval_count + index);
        }
    }
    None
}

fn selected_blocker_event(state: &TuiState) -> String {
    match selected_blocker(state) {
        Some(SelectedBlocker::Approval(id)) => {
            format!("approval selected: {}", short_id(&id))
        }
        Some(SelectedBlocker::SideQuestion(id)) => {
            format!("btw selected: {}", short_id(&id))
        }
        None => "approval selection unavailable".to_string(),
    }
}

fn activate_selected_blocker(state: &mut TuiState) {
    match selected_blocker(state) {
        Some(SelectedBlocker::Approval(_)) => update_selected_approval(state, true),
        Some(SelectedBlocker::SideQuestion(id)) => open_side_question_answer_prompt(state, &id),
        None => {
            state.selected_approval = 0;
            state.last_event = "no pending blockers".to_string();
        }
    }
}

fn deny_selected_blocker(state: &mut TuiState) {
    match selected_blocker(state) {
        Some(SelectedBlocker::Approval(_)) => update_selected_approval(state, false),
        Some(SelectedBlocker::SideQuestion(_)) => {
            state.last_event = "btw questions cannot be denied; press Enter to answer".to_string();
        }
        None => {
            state.selected_approval = 0;
            state.last_event = "no pending blockers".to_string();
        }
    }
}

fn open_side_question_answer_prompt(state: &mut TuiState, question_id: &str) {
    let question = session_monitor_for_state(state).and_then(|monitor| {
        monitor
            .open_questions
            .into_iter()
            .find(|question| question.id == question_id)
            .map(|question| question.question)
    });
    let Some(question) = question else {
        state.last_event = "btw question not found".to_string();
        return;
    };
    state.side_question_prompt = Some(SideQuestionPrompt {
        id: question_id.to_string(),
        question: question.clone(),
        input: MessageBox::new(),
    });
    state.chat.push(ChatLine {
        role: "deepcli".to_string(),
        content: format!("请回答旁路问题：{question}"),
    });
    state.last_event = "btw answer prompt opened".to_string();
}

pub(super) fn handle_side_question_prompt_key(key: KeyEvent, state: &mut TuiState) {
    match key.code {
        KeyCode::Enter
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::SHIFT) =>
        {
            handle_prompt_input_key(
                state
                    .side_question_prompt
                    .as_mut()
                    .map(|prompt| &mut prompt.input),
                key,
            )
        }
        KeyCode::Enter => confirm_side_question_prompt(state),
        _ => handle_prompt_input_key(
            state
                .side_question_prompt
                .as_mut()
                .map(|prompt| &mut prompt.input),
            key,
        ),
    }
}

fn confirm_side_question_prompt(state: &mut TuiState) {
    let Some(prompt) = state.side_question_prompt.as_ref() else {
        return;
    };
    let answer = prompt.input.buffer().trim().to_string();
    if answer.is_empty() {
        state.chat.push(ChatLine {
            role: "error".to_string(),
            content: "btw answer 不能为空。".to_string(),
        });
        state.last_event = "btw answer rejected".to_string();
        return;
    }
    let id = prompt.id.clone();
    match answer_side_question_for_state(state, &id, &answer) {
        Ok(message) => {
            state.side_question_prompt = None;
            state.chat.push(ChatLine {
                role: "deepcli".to_string(),
                content: message,
            });
            state.last_event = "btw answer saved".to_string();
            clamp_selected_blocker_to_monitor(state);
        }
        Err(error) => {
            state.chat.push(ChatLine {
                role: "error".to_string(),
                content: error.to_string(),
            });
            state.last_event = "btw answer failed".to_string();
        }
    }
}

fn answer_side_question_for_state(state: &mut TuiState, id: &str, answer: &str) -> Result<String> {
    if let Some(runtime) = state.runtime.as_mut() {
        return runtime.answer_current_side_question(id, answer);
    }
    let active = state
        .active_session
        .as_ref()
        .ok_or_else(|| anyhow!("当前运行会话不可用"))?;
    let session = SessionStore::new(&active.workspace).load(&active.session_id)?;
    let item = session.answer_side_question(id, answer)?;
    Ok(format!("answered btw question {}", item.id))
}

fn update_selected_approval(state: &mut TuiState, approved: bool) {
    let selected = {
        let Some(monitor) = session_monitor_for_state(state) else {
            state.last_event = "failed to read approval requests".to_string();
            return;
        };
        if monitor.pending_approvals.is_empty() {
            state.selected_approval = 0;
            state.last_event = "no pending approval requests".to_string();
            return;
        }
        let index = state
            .selected_approval
            .min(monitor.pending_approvals.len() - 1);
        monitor.pending_approvals[index].id.clone()
    };

    match update_approval_for_state(state, &selected, approved) {
        Ok(message) => {
            let action = if approved {
                "approval approved"
            } else {
                "approval denied"
            };
            state.chat.push(ChatLine {
                role: "deepcli".to_string(),
                content: message.clone(),
            });
            state.last_event = action.to_string();
            clamp_selected_blocker_to_monitor(state);
        }
        Err(error) => {
            state.chat.push(ChatLine {
                role: "error".to_string(),
                content: error.to_string(),
            });
            state.last_event = "approval action failed".to_string();
        }
    }
}

fn update_approval_for_state(state: &mut TuiState, id: &str, approved: bool) -> Result<String> {
    if let Some(runtime) = state.runtime.as_mut() {
        return runtime.update_current_approval(id, approved);
    }
    let active = state
        .active_session
        .as_ref()
        .ok_or_else(|| anyhow!("当前运行会话不可用"))?;
    let session = SessionStore::new(&active.workspace).load(&active.session_id)?;
    let status = if approved {
        ApprovalStatus::Approved
    } else {
        ApprovalStatus::Denied
    };
    let item = session.update_approval_request(id, status)?;
    let action = if approved { "approved" } else { "denied" };
    Ok(format!(
        "{action} approval request {} for tool {}",
        item.id, item.tool
    ))
}

fn clamp_selected_blocker_to_monitor(state: &mut TuiState) {
    let remaining = session_monitor_for_state(state)
        .map(|monitor| monitor.pending_approvals.len() + monitor.open_questions.len())
        .unwrap_or_default();
    if remaining == 0 {
        state.selected_approval = 0;
    } else {
        state.selected_approval = state.selected_approval.min(remaining - 1);
    }
}
