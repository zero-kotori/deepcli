use crate::runtime::SessionMonitor;

use super::{
    compact_ui_text, latest_action_result, latest_action_result_line,
    monitor::{append_monitor_quick_actions, MonitorQuickAction},
    non_empty_output_lines, TuiState,
};

pub(super) fn format_result_tab_lines(
    state: &TuiState,
    quick_actions: &[MonitorQuickAction],
    selected_quick_action: usize,
    height: u16,
) -> Vec<String> {
    let mut lines = Vec::new();
    match latest_action_result(state) {
        Some(result) => {
            lines.push(format!("status: {}", result.status));
            lines.push(format!("summary: {}", compact_ui_text(result.summary, 100)));
            let output_lines = non_empty_output_lines(result.content);
            let window_size = result_output_window_size(height, quick_actions);
            let scroll = state
                .result_scroll
                .min(output_lines.len().saturating_sub(1));
            let end = output_lines.len().saturating_sub(scroll).max(1);
            let start = end.saturating_sub(window_size);
            let below = output_lines.len().saturating_sub(end);
            if start == 0 && below == 0 {
                lines.push("output:".to_string());
            } else {
                lines.push(format!(
                    "output (PageUp older; PageDown latest; above={start} below={below}):"
                ));
            }
            let mut emitted = false;
            for line in output_lines.into_iter().skip(start).take(end - start) {
                lines.push(format!("  {}", compact_ui_text(line, 118)));
                emitted = true;
            }
            if !emitted {
                lines.push("  <empty>".to_string());
            }
        }
        None => {
            lines.push("no command output yet".to_string());
            lines.push("run a quick action or slash command to populate this view".to_string());
        }
    }
    append_monitor_quick_actions(
        &mut lines,
        "quick actions",
        quick_actions,
        selected_quick_action,
    );
    lines
}

pub(super) fn result_output_window_size(
    height: u16,
    quick_actions: &[MonitorQuickAction],
) -> usize {
    let visible = height.saturating_sub(2) as usize;
    let tab_lines = 1;
    let result_fixed_lines = 3;
    let quick_action_lines = if quick_actions.is_empty() {
        0
    } else {
        1 + quick_actions.len()
    };
    visible
        .saturating_sub(tab_lines + result_fixed_lines + quick_action_lines)
        .max(1)
}

pub(super) fn format_trace_tab_lines(
    state: &TuiState,
    monitor: Option<&SessionMonitor>,
    quick_actions: &[MonitorQuickAction],
    selected_quick_action: usize,
) -> Vec<String> {
    let mut lines = vec![format!("last: {}", compact_ui_text(&state.last_event, 100))];
    if let Some(result) = latest_action_result_line(state) {
        lines.push(result);
    }
    if let Some(monitor) = monitor {
        if monitor.recent_events.is_empty() {
            lines.push("no audit events recorded".to_string());
        } else {
            lines.extend(
                monitor
                    .recent_events
                    .iter()
                    .rev()
                    .map(|event| format!("{} {}", event.created_at, event.event_type)),
            );
        }
    } else {
        lines.push("audit events unavailable for running handoff".to_string());
    }
    append_monitor_quick_actions(
        &mut lines,
        "quick actions",
        quick_actions,
        selected_quick_action,
    );
    lines
}
