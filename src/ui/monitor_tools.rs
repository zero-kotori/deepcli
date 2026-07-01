use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;

use super::monitor::{tool_quick_actions, MonitorQuickAction, MonitorTab};
use super::monitor_shell::visible_panel_line_indices;
use super::{compact_ui_text, normalize_pasted_text, TuiState};

pub(super) const TOOL_KEY_SCROLL_STEP: usize = 5;
pub(super) const TOOL_MOUSE_SCROLL_STEP: usize = 3;
const TOOL_DETAIL_PREVIEW_CHARS: usize = 1_200;
const TOOL_DETAIL_PREVIEW_LINES: usize = 8;
const TOOL_DETAIL_PREVIEW_LINE_CHARS: usize = 180;

#[derive(Debug)]
pub(super) struct ToolLogItem {
    pub(super) title: String,
    pub(super) detail: String,
    pub(super) expanded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ToolTabLine {
    pub(super) text: String,
    pub(super) tool_index: Option<usize>,
}

pub(super) fn handle_tools_tab_key(key: KeyEvent, state: &mut TuiState) -> bool {
    if state.monitor_tab != MonitorTab::Tools || !state.input.buffer().trim().is_empty() {
        return false;
    }
    match key.code {
        KeyCode::Up => {
            move_selected_tool_by(state, false, 1);
            true
        }
        KeyCode::Down => {
            move_selected_tool_by(state, true, 1);
            true
        }
        KeyCode::PageUp => {
            move_selected_tool_by(state, false, TOOL_KEY_SCROLL_STEP);
            true
        }
        KeyCode::PageDown => {
            move_selected_tool_by(state, true, TOOL_KEY_SCROLL_STEP);
            true
        }
        KeyCode::Home => {
            select_tool_at_index(state, 0);
            true
        }
        KeyCode::End => {
            select_tool_at_index(state, state.tool_log.len().saturating_sub(1));
            true
        }
        KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            prefill_tools_session_command(state, false);
            true
        }
        KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            prefill_tools_session_command(state, true);
            true
        }
        KeyCode::Enter
            if key.modifiers.is_empty() || key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            toggle_selected_tool(state);
            true
        }
        _ => false,
    }
}

pub(super) fn prefill_tools_session_command(state: &mut TuiState, failed_only: bool) {
    let command = if failed_only {
        "/session tools --failed --limit 20 --current"
    } else {
        "/session tools --limit 20 --current"
    };
    state.input.set_buffer(command.to_string());
    state.selected_command = 0;
    state.last_event = if failed_only {
        "prefilled failed tool output command".to_string()
    } else {
        "prefilled tool output command".to_string()
    };
}

pub(super) fn toggle_selected_tool(state: &mut TuiState) {
    if state.monitor_tab != MonitorTab::Tools {
        return;
    }
    if let Some(index) = state.selected_tool {
        if let Some(item) = state.tool_log.get_mut(index) {
            item.expanded = !item.expanded;
            let state_label = if item.expanded {
                "expanded"
            } else {
                "collapsed"
            };
            state.last_event = format!("{state_label}: {}", item.title);
        }
    }
}

pub(super) fn toggle_tool_at_row(state: &mut TuiState, tools_area: Rect, row: u16) {
    if state.monitor_tab != MonitorTab::Tools {
        return;
    }
    if row <= tools_area.y + 1 || row >= tools_area.y + tools_area.height.saturating_sub(1) {
        return;
    }
    let line = row.saturating_sub(tools_area.y + 1) as usize;
    let Some(index) = visible_tool_index_at_line(state, tools_area.height, line) else {
        return;
    };
    if let Some(item) = state.tool_log.get_mut(index) {
        item.expanded = !item.expanded;
        state.selected_tool = Some(index);
        let state_label = if item.expanded {
            "expanded"
        } else {
            "collapsed"
        };
        state.last_event = format!("{state_label}: {}", item.title);
    }
}

pub(super) fn move_selected_tool_by(state: &mut TuiState, forward: bool, step: usize) {
    if state.tool_log.is_empty() {
        state.selected_tool = None;
        state.last_event = "no tool calls yet".to_string();
        return;
    }
    let current = state.selected_tool.unwrap_or(if forward {
        0
    } else {
        state.tool_log.len().saturating_sub(1)
    });
    let next = if forward {
        current
            .saturating_add(step.max(1))
            .min(state.tool_log.len().saturating_sub(1))
    } else {
        current.saturating_sub(step.max(1))
    };
    select_tool_at_index(state, next);
}

pub(super) fn select_tool_at_index(state: &mut TuiState, index: usize) {
    if state.tool_log.is_empty() {
        state.selected_tool = None;
        state.last_event = "no tool calls yet".to_string();
        return;
    }
    let index = index.min(state.tool_log.len().saturating_sub(1));
    state.selected_tool = Some(index);
    if let Some(item) = state.tool_log.get(index) {
        state.last_event = format!("tool selected: {}", item.title);
    }
}

pub(super) fn visible_tool_index_at_line(
    state: &TuiState,
    height: u16,
    line: usize,
) -> Option<usize> {
    let tool_lines = tool_tab_lines(state);
    let total_lines = tool_lines.len().saturating_add(1);
    let focus_line = selected_tool_panel_line(state);
    let original_line = visible_panel_line_indices(total_lines, height, focus_line)
        .get(line)
        .and_then(|line| *line)?;
    if original_line == 0 {
        return None;
    }
    tool_lines
        .get(original_line.saturating_sub(1))
        .and_then(|line| line.tool_index)
}

pub(super) fn selected_tool_panel_line(state: &TuiState) -> Option<usize> {
    let selected = state.selected_tool?;
    if state
        .tool_log
        .get(selected)
        .is_some_and(|item| item.expanded)
    {
        return tool_tab_lines(state)
            .iter()
            .position(|line| line.text.starts_with("selected detail: "))
            .map(|line| line + 1);
    }
    tool_tab_lines(state)
        .iter()
        .position(|line| line.tool_index == Some(selected))
        .map(|line| line + 1)
}

pub(super) fn format_tool_tab_lines(
    state: &TuiState,
    quick_actions: &[MonitorQuickAction],
    selected_quick_action: usize,
) -> Vec<String> {
    tool_tab_lines_with_actions(state, quick_actions, selected_quick_action)
        .into_iter()
        .map(|line| line.text)
        .collect()
}

pub(super) fn tool_tab_lines(state: &TuiState) -> Vec<ToolTabLine> {
    tool_tab_lines_with_actions(state, &tool_quick_actions(), state.selected_command)
}

fn tool_tab_lines_with_actions(
    state: &TuiState,
    quick_actions: &[MonitorQuickAction],
    selected_quick_action: usize,
) -> Vec<ToolTabLine> {
    if state.tool_log.is_empty() {
        let mut lines = vec![ToolTabLine {
            text: "no tool calls yet".to_string(),
            tool_index: None,
        }];
        append_tool_quick_action_lines(&mut lines, quick_actions, selected_quick_action);
        return lines;
    }
    let mut lines = Vec::new();
    if let Some((index, item)) = selected_tool_item(state).filter(|(_, item)| item.expanded) {
        lines.push(ToolTabLine {
            text: format!("selected detail: {}", item.title),
            tool_index: None,
        });
        for line in tool_detail_preview_lines(&item.detail) {
            lines.push(ToolTabLine {
                text: format!("  {line}"),
                tool_index: None,
            });
        }
        if tool_detail_is_truncated(&item.detail) {
            lines.push(ToolTabLine {
                text: "  [detail truncated; Ctrl-O prefill full output, Ctrl-F failed tools]"
                    .to_string(),
                tool_index: None,
            });
        }
        lines.push(ToolTabLine {
            text: format!(
                "tool calls: selected {}/{}",
                index + 1,
                state.tool_log.len()
            ),
            tool_index: None,
        });
    } else {
        lines.push(ToolTabLine {
            text: "tool calls: Up/Down select, Enter/click expand, Ctrl-O full, Ctrl-F failed"
                .to_string(),
            tool_index: None,
        });
        append_tool_quick_action_lines(&mut lines, quick_actions, selected_quick_action);
    }
    lines.extend(state.tool_log.iter().enumerate().map(|(index, item)| {
        let marker = if item.expanded { "v" } else { ">" };
        let selected = if state.selected_tool == Some(index) {
            "*"
        } else {
            " "
        };
        ToolTabLine {
            text: format!("{selected} {marker} {}", item.title),
            tool_index: Some(index),
        }
    }));
    lines
}

pub(super) fn append_tool_quick_action_lines(
    lines: &mut Vec<ToolTabLine>,
    actions: &[MonitorQuickAction],
    selected: usize,
) {
    if actions.is_empty() {
        return;
    }
    lines.push(ToolTabLine {
        text: "tool actions (click or Ctrl-O/Ctrl-F; edit before run):".to_string(),
        tool_index: None,
    });
    let selected = selected.min(actions.len() - 1);
    for (index, action) in actions.iter().enumerate() {
        let marker = if index == selected { ">" } else { " " };
        let suffix = if action.edit_before_run {
            " (edit)"
        } else {
            ""
        };
        lines.push(ToolTabLine {
            text: format!(" {marker} {}{suffix}", action.command),
            tool_index: None,
        });
    }
}

fn selected_tool_item(state: &TuiState) -> Option<(usize, &ToolLogItem)> {
    let index = state.selected_tool?;
    state.tool_log.get(index).map(|item| (index, item))
}

pub(super) fn tool_detail_preview_lines(detail: &str) -> Vec<String> {
    let normalized = normalize_pasted_text(detail);
    let preview = normalized
        .chars()
        .take(TOOL_DETAIL_PREVIEW_CHARS)
        .collect::<String>();
    let mut lines = preview
        .lines()
        .take(TOOL_DETAIL_PREVIEW_LINES)
        .map(|line| compact_ui_text(line, TOOL_DETAIL_PREVIEW_LINE_CHARS))
        .collect::<Vec<_>>();
    if lines.is_empty() {
        lines.push("<empty detail>".to_string());
    }
    lines
}

pub(super) fn tool_detail_is_truncated(detail: &str) -> bool {
    let normalized = normalize_pasted_text(detail);
    normalized.chars().count() > TOOL_DETAIL_PREVIEW_CHARS
        || normalized.lines().count() > TOOL_DETAIL_PREVIEW_LINES
        || normalized
            .lines()
            .any(|line| line.chars().count() > TOOL_DETAIL_PREVIEW_LINE_CHARS)
}
