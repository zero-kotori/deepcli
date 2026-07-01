use crate::runtime::{SessionMonitor, SessionObservation};
use ratatui::{
    layout::Rect,
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use super::monitor::{
    append_monitor_quick_actions, deliver_quick_actions, environment_quick_actions,
    format_approvals_tab_lines, format_context_tab_lines, format_deliver_tab_lines,
    format_environment_tab_lines, format_latest_test, format_session_tab_lines,
    format_tests_tab_lines, format_usage_tab_lines, tool_quick_actions, MonitorQuickAction,
    MonitorTab, MonitorTier,
};
use super::monitor_changes::format_changes_tab_lines;
use super::monitor_health::{format_health_tab_lines, health_quick_actions_for_state};
use super::monitor_library::{format_library_tab_lines, library_quick_actions_for_state};
use super::monitor_output::{format_result_tab_lines, format_trace_tab_lines};
use super::monitor_tools::{format_tool_tab_lines, selected_tool_panel_line};
use super::{
    compact_ui_text, latest_action_result_line, rect_contains, rect_content_row_contains,
    session_monitor_for_state, slash_command_suggestions_for_state, TuiState,
};

pub(super) fn visible_panel_line_indices(
    total_lines: usize,
    height: u16,
    focus_line: Option<usize>,
) -> Vec<Option<usize>> {
    let visible = height.saturating_sub(2) as usize;
    if visible == 0 || total_lines <= visible {
        return (0..total_lines).map(Some).collect();
    }
    let Some(focus_line) = focus_line.filter(|line| *line < total_lines) else {
        return (0..visible.saturating_sub(1))
            .map(Some)
            .chain(std::iter::once(None))
            .collect();
    };
    if focus_line < visible.saturating_sub(1) {
        return (0..visible.saturating_sub(1))
            .map(Some)
            .chain(std::iter::once(None))
            .collect();
    }
    if visible <= 2 {
        return (0..visible.saturating_sub(1))
            .map(Some)
            .chain(std::iter::once(None))
            .collect();
    }

    let body_slots = visible.saturating_sub(2);
    let start = focus_line
        .saturating_sub(body_slots.saturating_sub(1))
        .max(1);
    let end = start.saturating_add(body_slots).min(total_lines);
    let mut focused = Vec::with_capacity(visible);
    focused.push(Some(0));
    focused.extend((start..end).map(Some));
    focused.push(None);
    focused.truncate(visible);
    focused
}

pub(super) fn render_task_monitor(frame: &mut Frame<'_>, area: Rect, state: &TuiState) {
    let monitor = session_monitor_for_state(state);
    let text = format_task_monitor_text(state, monitor.as_ref(), area.height);
    frame.render_widget(
        Paragraph::new(text).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Task Monitor (Ctrl-T/click tabs; Up/Down actions; Enter/click run)"),
        ),
        area,
    );
}

pub(super) fn format_task_monitor_text(
    state: &TuiState,
    monitor: Option<&SessionMonitor>,
    height: u16,
) -> String {
    let quick_actions = monitor_quick_actions_for_tab(state, monitor);
    let mut lines = vec![format_monitor_tabs(state.monitor_tab)];
    lines.extend(match state.monitor_tab {
        MonitorTab::Overview => format_task_overview_lines(
            state,
            monitor.map(|monitor| &monitor.observation),
            &quick_actions,
            state.selected_command,
        ),
        MonitorTab::Result => {
            format_result_tab_lines(state, &quick_actions, state.selected_command, height)
        }
        MonitorTab::Changes => {
            format_changes_tab_lines(state, &quick_actions, state.selected_command)
        }
        MonitorTab::Usage => {
            format_usage_tab_lines(monitor, &quick_actions, state.selected_command)
        }
        MonitorTab::Health => {
            format_health_tab_lines(state, &quick_actions, state.selected_command)
        }
        MonitorTab::Library => {
            format_library_tab_lines(state, &quick_actions, state.selected_command)
        }
        MonitorTab::Deliver => {
            format_deliver_tab_lines(monitor, &quick_actions, state.selected_command)
        }
        MonitorTab::Tools => format_tool_tab_lines(state, &quick_actions, state.selected_command),
        MonitorTab::Tests => {
            format_tests_tab_lines(monitor, &quick_actions, state.selected_command)
        }
        MonitorTab::Session => {
            format_session_tab_lines(monitor, &quick_actions, state.selected_command)
        }
        MonitorTab::Environment => {
            format_environment_tab_lines(monitor, &quick_actions, state.selected_command)
        }
        MonitorTab::Approvals => format_approvals_tab_lines(monitor, state.selected_approval),
        MonitorTab::Context => {
            format_context_tab_lines(monitor, &quick_actions, state.selected_command)
        }
        MonitorTab::Trace => {
            format_trace_tab_lines(state, monitor, &quick_actions, state.selected_command)
        }
    });
    let selected_action_line = if state.monitor_tab == MonitorTab::Tools {
        selected_tool_panel_line(state)
    } else if state.selected_command == 0 {
        None
    } else {
        selected_monitor_quick_action_line(&lines)
    };
    truncate_panel_lines_with_focus(lines, height, selected_action_line)
}

pub(super) const MONITOR_ADVANCED_TOGGLE_LABEL: &str = "+advanced";

enum MonitorTabSegmentKind {
    Tab(MonitorTab),
    Separator,
    EnterAdvanced,
}

struct MonitorTabSegment {
    kind: MonitorTabSegmentKind,
    text: String,
}

pub(super) fn first_advanced_monitor_tab() -> MonitorTab {
    MonitorTab::all()
        .iter()
        .copied()
        .find(|tab| tab.tier() == MonitorTier::Advanced)
        .unwrap_or(MonitorTab::Overview)
}

fn monitor_tab_strip(active: MonitorTab) -> Vec<MonitorTabSegment> {
    let mut segments = Vec::new();
    for tab in MonitorTab::all()
        .iter()
        .filter(|tab| tab.tier() == MonitorTier::Core)
    {
        segments.push(MonitorTabSegment {
            kind: MonitorTabSegmentKind::Tab(*tab),
            text: monitor_tab_label(*tab, active),
        });
    }
    if active.tier() == MonitorTier::Advanced {
        segments.push(MonitorTabSegment {
            kind: MonitorTabSegmentKind::Separator,
            text: "|".to_string(),
        });
        for tab in MonitorTab::all()
            .iter()
            .filter(|tab| tab.tier() == MonitorTier::Advanced)
        {
            segments.push(MonitorTabSegment {
                kind: MonitorTabSegmentKind::Tab(*tab),
                text: monitor_tab_label(*tab, active),
            });
        }
    } else {
        segments.push(MonitorTabSegment {
            kind: MonitorTabSegmentKind::EnterAdvanced,
            text: MONITOR_ADVANCED_TOGGLE_LABEL.to_string(),
        });
    }
    segments
}

fn monitor_tab_label(tab: MonitorTab, active: MonitorTab) -> String {
    if tab == active {
        format!("[{}]", tab.label())
    } else {
        tab.label().to_string()
    }
}

pub(super) fn format_monitor_tabs(active: MonitorTab) -> String {
    monitor_tab_strip(active)
        .iter()
        .map(|segment| segment.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn format_task_overview_lines(
    state: &TuiState,
    observation: Option<&SessionObservation>,
    quick_actions: &[MonitorQuickAction],
    selected_quick_action: usize,
) -> Vec<String> {
    let ui_state = if state.running { "running" } else { "ready" };
    let last = compact_ui_text(&state.last_event, 80);
    let mut lines = if let Some(observation) = observation {
        let plan = if observation.plan_total == 0 {
            "plan=none".to_string()
        } else {
            let mut plan = format!(
                "plan={}/{}",
                observation.plan_completed, observation.plan_total
            );
            if observation.plan_in_progress > 0 {
                plan.push_str(&format!(" running={}", observation.plan_in_progress));
            }
            if observation.plan_failed > 0 {
                plan.push_str(&format!(" failed={}", observation.plan_failed));
            }
            plan
        };
        let test = observation
            .latest_test
            .as_ref()
            .map(format_latest_test)
            .unwrap_or_else(|| "test=none".to_string());
        let current = observation
            .current_step
            .as_deref()
            .map(|step| format!(" current={}", compact_ui_text(step, 48)))
            .unwrap_or_default();
        vec![
            format!(
                "state={} ui={} {plan} approvals={} btw={}{}",
                observation.state,
                ui_state,
                observation.pending_approvals,
                observation.open_questions,
                current
            ),
            format!(
                "{} tools={} failed_tools={} last={}",
                test,
                observation.tool_calls.max(state.tool_log.len()),
                observation.failed_tools,
                last
            ),
        ]
    } else {
        vec![
            format!("state={ui_state} plan=unknown approvals=unknown btw=unknown"),
            format!("test=unknown tools={} last={last}", state.tool_log.len()),
        ]
    };
    if let Some(result) = latest_action_result_line(state) {
        lines.push(result);
    }
    append_monitor_quick_actions(
        &mut lines,
        "quick actions",
        quick_actions,
        selected_quick_action,
    );
    lines
}

pub(super) fn monitor_quick_actions_for_tab(
    state: &TuiState,
    monitor: Option<&SessionMonitor>,
) -> Vec<MonitorQuickAction> {
    if let Some(projection) = state.monitor_tab.static_quick_actions() {
        return projection.actions();
    }

    match state.monitor_tab {
        MonitorTab::Health => health_quick_actions_for_state(state),
        MonitorTab::Library => library_quick_actions_for_state(state),
        MonitorTab::Deliver => deliver_quick_actions(monitor),
        MonitorTab::Tools => tool_quick_actions(),
        MonitorTab::Environment => environment_quick_actions(monitor),
        MonitorTab::Overview
        | MonitorTab::Result
        | MonitorTab::Changes
        | MonitorTab::Usage
        | MonitorTab::Tests
        | MonitorTab::Session
        | MonitorTab::Approvals
        | MonitorTab::Context
        | MonitorTab::Trace => Vec::new(),
    }
}

pub(super) fn select_monitor_tab_at_position(
    state: &mut TuiState,
    tools_area: Rect,
    column: u16,
    row: u16,
) -> bool {
    if state.resume_picker.is_some()
        || slash_command_suggestions_for_state(state.input.buffer(), state.running).is_some()
        || !rect_contains(tools_area, column, row)
        || row != tools_area.y.saturating_add(1)
        || column <= tools_area.x
    {
        return false;
    }

    let relative_column = column.saturating_sub(tools_area.x + 1) as usize;
    let mut offset = 0usize;
    for segment in monitor_tab_strip(state.monitor_tab) {
        let end = offset + segment.text.len();
        let hit = (offset..end).contains(&relative_column);
        match segment.kind {
            MonitorTabSegmentKind::Tab(tab) if hit => {
                state.monitor_tab = tab;
                state.selected_command = 0;
                state.last_event = format!("monitor tab: {}", state.monitor_tab.label());
                return true;
            }
            MonitorTabSegmentKind::EnterAdvanced if hit => {
                state.monitor_tab = first_advanced_monitor_tab();
                state.selected_command = 0;
                state.last_event = format!("monitor tab: {}", state.monitor_tab.label());
                return true;
            }
            _ => {}
        }
        offset = end + 1;
    }
    false
}

pub(super) fn clicked_monitor_quick_action_index(
    state: &TuiState,
    monitor: Option<&SessionMonitor>,
    tools_area: Rect,
    row: u16,
    actions: &[MonitorQuickAction],
) -> Option<usize> {
    if actions.is_empty() || !rect_content_row_contains(tools_area, row) {
        return None;
    }
    let content_index = row.saturating_sub(tools_area.y + 1) as usize;
    let text = format_task_monitor_text(state, monitor, tools_area.height);
    let line = text.lines().nth(content_index)?;
    let trimmed = line.trim_start();
    let command = trimmed.strip_prefix("> ").unwrap_or(trimmed);
    let command = command.strip_suffix(" (edit)").unwrap_or(command);
    if !command.starts_with('/') {
        return None;
    }
    actions.iter().position(|action| action.command == command)
}

pub(super) fn truncate_panel_lines(mut lines: Vec<String>, height: u16) -> String {
    let visible = height.saturating_sub(2) as usize;
    if visible > 0 && lines.len() > visible {
        lines.truncate(visible.saturating_sub(1));
        lines.push(more_panel_lines_marker());
    }
    lines.join("\n")
}

pub(super) fn truncate_panel_lines_with_focus(
    lines: Vec<String>,
    height: u16,
    focus_line: Option<usize>,
) -> String {
    let visible = height.saturating_sub(2) as usize;
    if visible == 0 || lines.len() <= visible {
        return lines.join("\n");
    }
    let Some(focus_line) = focus_line.filter(|line| *line < lines.len()) else {
        return truncate_panel_lines(lines, height);
    };
    if focus_line < visible.saturating_sub(1) {
        return truncate_panel_lines(lines, height);
    }
    if visible <= 2 {
        return lines
            .into_iter()
            .take(visible.saturating_sub(1))
            .chain(std::iter::once(more_panel_lines_marker()))
            .collect::<Vec<_>>()
            .join("\n");
    }

    let body_slots = visible.saturating_sub(2);
    let start = focus_line
        .saturating_sub(body_slots.saturating_sub(1))
        .max(1);
    let end = start.saturating_add(body_slots).min(lines.len());
    let mut focused = Vec::with_capacity(visible);
    focused.push(lines[0].clone());
    focused.extend(lines[start..end].iter().cloned());
    focused.push(more_panel_lines_marker());
    focused.truncate(visible);
    focused.join("\n")
}

pub(super) fn selected_monitor_quick_action_line(lines: &[String]) -> Option<usize> {
    lines
        .iter()
        .position(|line| line.trim_start().starts_with("> /"))
}

fn more_panel_lines_marker() -> String {
    "[more: use /session, /approval, /btw, /doctor, or /trace for full detail]".to_string()
}
